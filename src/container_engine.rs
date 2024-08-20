use std::{collections::HashMap, path::PathBuf, pin::Pin, process::Stdio};

use async_trait::async_trait;
use bollard::{
    container::{CreateContainerOptions, LogOutput},
    exec::{CreateExecOptions, StartExecResults},
    ClientVersion, Docker,
};
use futures::{Stream, StreamExt, TryStreamExt};
use podman_rest_client::{
    v5::{
        apis::{Containers, Images, System},
        models::{BindOptions, Mount, SpecGenerator},
        params::ImagePullLibpod,
    },
    PodmanRestClient,
};
use tokio::{
    io::{AsyncBufReadExt, BufReader, Lines},
    process::{Child, ChildStdout, Command},
};
use uuid::Uuid;

use crate::schema::{BuildScriptContainer, BuildScriptContainerImage};

#[async_trait]
pub trait ContainerEngine {
    async fn ping(&self);

    async fn pull_image(&self, image: &BuildScriptContainerImage);

    async fn start_container(
        &self,
        container: BuildScriptContainer,
        extra_volumes: HashMap<PathBuf, PathBuf>,
    ) -> (String, String);

    async fn exec_in_container(&self, exec_params: ExecParams<'_>) -> Box<dyn ExecReader>;
}

#[async_trait]
pub trait ExecReader {
    async fn read(&mut self) -> Option<String>;
}

pub struct ExecParams<'a> {
    pub container_name: &'a str,
    pub container_id: &'a str,
    pub cmd: String,
    pub uid: Option<u32>,
    pub gid: Option<u32>,
    pub working_dir: Option<PathBuf>,
    pub privileged: Option<bool>,
    pub env: HashMap<String, String>,
}

pub struct PodmanContainerEngine {
    client: PodmanRestClient,
}

impl PodmanContainerEngine {
    pub fn new(connection_uri: Option<String>) -> Self {
        let connection_uri = match connection_uri {
            Some(uri) => uri,
            None => {
                let uid = unsafe { libc::geteuid() };
                match uid {
                    0 => "unix:///run/podman/podman.sock".to_string(),
                    other => format!("unix:///run/user/{other}/podman/podman.sock"),
                }
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
        let podman_path = which::which("podman").expect("Could not locate \"podman\" binary to perform exec via CLI");
        log::info!("Located \"podman\" binary at {podman_path:?}");

        let mut command = Command::new(podman_path);
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

        let mut child = command.spawn().expect("Could not spawn \"podman\" binary for exec");
        let stdout = child
            .stdout
            .take()
            .expect("Could not pipe stdout of \"podman\" binary for reading exec output");
        Box::new(PodmanExecReader {
            child,
            stdout_reader: BufReader::new(stdout).lines(),
        })
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

        self.stdout_reader.next_line().await.ok()?
    }
}

pub struct DockerContainerEngine {
    client: Docker,
}

impl DockerContainerEngine {
    pub fn new(connection_uri: Option<String>) -> Self {
        let client = match connection_uri {
            Some(connection_uri) => {
                let client_version = ClientVersion {
                    major_version: 1,
                    minor_version: 46,
                };

                if connection_uri.starts_with("http://") {
                    Docker::connect_with_http(&connection_uri, 5, &client_version)
                } else {
                    Docker::connect_with_local(&connection_uri, 5, &client_version)
                }
            }
            None => Docker::connect_with_defaults(),
        }
        .expect("Could not connect to Docker daemon");

        Self { client }
    }
}

#[async_trait]
impl ContainerEngine for DockerContainerEngine {
    async fn ping(&self) {
        let response = self.client.ping().await.expect("Pinging Docker daemon failed");

        if !response.contains("OK") {
            panic!("Ping response from Docker daemon is not OK: {response}");
        }
    }

    async fn pull_image(&self, image: &BuildScriptContainerImage) {
        let mut stream = self.client.create_image(
            Some(bollard::image::CreateImageOptions {
                from_image: image.full_name(),
                tag: image.tag.clone(),
                ..Default::default()
            }),
            None,
            None,
        );

        while let Some(result) = stream.next().await {
            result.expect("Could not pull image via Docker daemon");
        }
    }

    async fn start_container(
        &self,
        container: BuildScriptContainer,
        mut extra_volumes: HashMap<PathBuf, PathBuf>,
    ) -> (String, String) {
        extra_volumes.extend(container.volumes);

        let container_name = Uuid::new_v4().to_string();
        let config = bollard::container::Config {
            image: Some(container.image.full_name()),
            tty: Some(true),
            hostname: container.hostname,
            env: Some(
                container
                    .env
                    .into_iter()
                    .map(|(key, value)| format!("{key}={value}"))
                    .collect::<Vec<_>>(),
            ),
            host_config: Some(bollard::models::HostConfig {
                binds: Some(
                    extra_volumes
                        .into_iter()
                        .map(|(src, dst)| format!("{}:{}", src.to_string_lossy(), dst.to_string_lossy()))
                        .collect(),
                ),
                runtime: container.oci_runtime,
                cap_add: container.cap_add,
                cap_drop: container.cap_drop,
                privileged: Some(container.rootful),
                ..Default::default()
            }),
            ..Default::default()
        };

        let response = self
            .client
            .create_container(
                Some(CreateContainerOptions {
                    name: &container_name,
                    platform: None,
                }),
                config,
            )
            .await
            .expect("Could not create container via Docker daemon");

        self.client
            .start_container::<String>(&container_name, None)
            .await
            .expect("Could not start container via Docker daemon");

        (response.id, container_name)
    }

    async fn exec_in_container(&self, exec_params: ExecParams<'_>) -> Box<dyn ExecReader> {
        let response = self
            .client
            .create_exec(
                exec_params.container_name,
                CreateExecOptions::<String> {
                    attach_stdin: Some(false),
                    attach_stdout: Some(true),
                    attach_stderr: Some(true),
                    tty: Some(true),
                    env: Some(
                        exec_params
                            .env
                            .into_iter()
                            .map(|(key, value)| format!("{key}={value}"))
                            .collect(),
                    ),
                    cmd: Some(exec_params.cmd.split_whitespace().map(|s| s.to_owned()).collect()),
                    privileged: exec_params.privileged,
                    user: format_uid_gid_string(exec_params.uid, exec_params.gid),
                    working_dir: exec_params
                        .working_dir
                        .map(|path_buf| path_buf.to_string_lossy().to_string()),
                    ..Default::default()
                },
            )
            .await
            .expect("Could not create exec via Docker daemon");

        let output = match self
            .client
            .start_exec(&response.id, None)
            .await
            .expect("Could not start exec via Docker daemon")
        {
            StartExecResults::Attached { output, input: _ } => output,
            StartExecResults::Detached => panic!("Attaching to Docker daemon exec failed"),
        };

        Box::new(DockerExecReader { output })
    }
}

struct DockerExecReader {
    output: Pin<Box<dyn Stream<Item = Result<LogOutput, bollard::errors::Error>> + Send>>,
}

#[async_trait]
impl ExecReader for DockerExecReader {
    async fn read(&mut self) -> Option<String> {
        let bytes = match self.output.try_next().await.ok()?? {
            LogOutput::StdErr { message } => message,
            LogOutput::StdOut { message } => message,
            LogOutput::StdIn { message } => message,
            LogOutput::Console { message } => message,
        };
        String::from_utf8(bytes.to_vec()).ok()
    }
}

fn format_uid_gid_string(uid: Option<u32>, gid: Option<u32>) -> Option<String> {
    match uid {
        Some(uid) => match gid {
            Some(gid) => Some(format!("{}:{}", uid.to_string(), gid.to_string())),
            None => Some(uid.to_string()),
        },
        None => None,
    }
}
