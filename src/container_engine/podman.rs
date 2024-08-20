use std::{collections::HashMap, path::PathBuf, process::Stdio};

use async_trait::async_trait;
use podman_rest_client::{
    v5::{
        apis::{Containers, Images, System},
        models::{BindOptions, Mount, SpecGenerator},
        params::{ContainerDeleteLibpod, ContainerStopLibpod, ImagePullLibpod},
    },
    PodmanRestClient,
};
use tokio::{
    io::{AsyncBufReadExt, BufReader, Lines},
    process::{Child, ChildStdout, Command},
};
use uuid::Uuid;

use crate::{
    container_engine::format_uid_gid_string,
    schema::{BuildScriptContainer, BuildScriptContainerImage},
};

use super::{ContainerEngine, ExecParams, ExecReader};

pub struct PodmanContainerEngine {
    client: PodmanRestClient,
}

impl PodmanContainerEngine {
    pub fn new(connection_uri: Option<String>) -> Self {
        let connection_uri = match connection_uri {
            Some(uri) => uri,
            None => {
                let uid = unsafe { libc::geteuid() };
                let resolved_uri = match uid {
                    0 => "unix:///run/podman/podman.sock".to_string(),
                    other => format!("unix:///run/user/{other}/podman/podman.sock"),
                };
                log::debug!("Podman connection URI was resolved to {resolved_uri}");
                resolved_uri
            }
        };

        if !connection_uri.starts_with("unix://") {
            panic!("A Podman connection can only use a Unix socket and must be unix://M where M is the socket path");
        }

        let socket_path = connection_uri.trim_start_matches("unix://");
        Self {
            client: PodmanRestClient::new_unix(socket_path),
        }
    }

    fn get_podman_path() -> PathBuf {
        let podman_path = which::which("podman").expect("Could not locate \"podman\" binary in PATH");
        log::debug!("Located \"podman\" binary at {podman_path:?}");
        podman_path
    }
}

#[async_trait]
impl ContainerEngine for PodmanContainerEngine {
    async fn ping(&self) {
        self.client
            .system_version_libpod()
            .await
            .expect("Pinging libpod failed");
    }

    async fn pull_image(&self, image: &BuildScriptContainerImage) {
        self.client
            .image_pull_libpod(Some(ImagePullLibpod {
                reference: Some(image.full_name().as_str()),
                ..Default::default()
            }))
            .await
            .expect("Could not pull image via libpod");
    }

    async fn start_container(
        &self,
        container: BuildScriptContainer,
        mut extra_volumes: HashMap<PathBuf, PathBuf>,
    ) -> (String, String) {
        let container_name = Uuid::new_v4().to_string();
        extra_volumes.extend(container.volumes);

        let spec_generator = SpecGenerator {
            image: Some(container.image.full_name()),
            privileged: Some(container.rootful),
            terminal: Some(true),
            remove: Some(true),
            env: Some(container.env),
            hostname: container.hostname,
            oci_runtime: container.oci_runtime,
            timeout: container.timeout,
            cap_add: container.cap_add,
            cap_drop: container.cap_drop,
            name: Some(container_name.clone()),
            mounts: Some(
                extra_volumes
                    .into_iter()
                    .map(|(src, dst)| Mount {
                        bind_options: Some(BindOptions {
                            create_mountpoint: Some(true),
                            ..Default::default()
                        }),
                        source: Some(src.to_string_lossy().to_string()),
                        destination: Some(dst.to_string_lossy().to_string()),
                        r#type: Some("bind".to_string()),
                        ..Default::default()
                    })
                    .collect(),
            ),
            ..Default::default()
        };

        let response = self
            .client
            .container_create_libpod(spec_generator)
            .await
            .expect("Could not create container via libpod");

        self.client
            .container_start_libpod(&container_name, None)
            .await
            .expect("Could not start container via libpod");

        (response.id, container_name)
    }

    async fn exec_in_container(&self, exec_params: ExecParams<'_>) -> Box<dyn ExecReader> {
        let mut command = Command::new(Self::get_podman_path());
        command.arg("exec");

        if let Some(uid_gid_string) = format_uid_gid_string(exec_params.uid, exec_params.gid) {
            command.arg("--user");
            command.arg(uid_gid_string);
        }

        command.arg("--tty");

        if let Some(working_dir) = exec_params.working_dir {
            command.arg("--workdir");
            command.arg(working_dir);
        }

        if let Some(true) = exec_params.privileged {
            command.arg("--privileged");
        }

        for (env_key, env_value) in exec_params.env {
            command.arg(format!("--env={env_key}={env_value}"));
        }

        command.arg(exec_params.container_id);

        for segment in exec_params.cmd.split_whitespace() {
            command.arg(segment);
        }

        command.stdout(Stdio::piped());

        let mut child = command
            .spawn()
            .expect("Could not fork \"podman\" binary for running \"podman exec\"");
        let stdout = child
            .stdout
            .take()
            .expect("Could not pipe stdout of \"podman\" binary for reading exec output");
        Box::new(PodmanExecReader {
            child,
            stdout_reader: BufReader::new(stdout).lines(),
        })
    }

    async fn export_container(&self, container_name: &str, tar_path: &PathBuf) {
        let mut command = Command::new(Self::get_podman_path());
        command.arg("export");
        command.arg("-o");
        command.arg(tar_path);
        command.arg(container_name);

        let exit_status = command
            .status()
            .await
            .expect("Could not fork \"podman\" binary for running \"podman export\"");
        if !exit_status.success() {
            panic!("Running \"podman export\" failed with exit status: {exit_status}");
        }
    }

    async fn remove_container(&self, container_name: &str, timeout: Option<u64>) {
        self.client
            .container_stop_libpod(
                container_name,
                Some(ContainerStopLibpod {
                    timeout: timeout.map(|t| t as i64),
                    ..Default::default()
                }),
            )
            .await
            .expect("Could not stop container via libpod");

        self.client
            .container_delete_libpod(
                container_name,
                Some(ContainerDeleteLibpod {
                    force: Some(true),
                    timeout: timeout.map(|t| t as i64),
                    ..Default::default()
                }),
            )
            .await
            .expect("Could not remove container via libpod");
    }
}

struct PodmanExecReader {
    child: Child,
    stdout_reader: Lines<BufReader<ChildStdout>>,
}

#[async_trait]
impl ExecReader for PodmanExecReader {
    async fn read(&mut self) -> Option<String> {
        if let Ok(Some(exit_status)) = self.child.try_wait() {
            if !exit_status.success() {
                log::error!("Podman exec CLI command exited with non-zero status: {exit_status}");
            }

            return None;
        }

        self.stdout_reader.next_line().await.ok()?.map(|s| s + "\n")
    }
}
