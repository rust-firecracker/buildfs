use std::{collections::HashMap, path::PathBuf, pin::Pin};

use async_trait::async_trait;
use bollard::{
    container::{Config, CreateContainerOptions, LogOutput, RemoveContainerOptions, StopContainerOptions},
    exec::{CreateExecOptions, StartExecResults},
    secret::HostConfig,
    ClientVersion, Docker,
};
use futures_util::{Stream, StreamExt, TryStreamExt};
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

use crate::schema::{BuildScriptContainer, BuildScriptContainerImage};

use super::{format_uid_gid_string, ContainerEngine, ExecParams, ExecReader, StreamType};

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
        let config = Config {
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
            host_config: Some(HostConfig {
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

        let stream = match self
            .client
            .start_exec(&response.id, None)
            .await
            .expect("Could not start exec via Docker daemon")
        {
            StartExecResults::Attached { output, input: _ } => output,
            StartExecResults::Detached => panic!("Attaching to Docker daemon exec failed"),
        };

        Box::new(DockerExecReader { stream })
    }

    async fn export_container(&self, container_name: &str, tar_path: &PathBuf) {
        let mut stream = self.client.export_container(container_name);
        let mut file = tokio::fs::File::options()
            .write(true)
            .append(true)
            .create(true)
            .open(tar_path)
            .await
            .expect("Could not open tarball file");

        while let Some(result) = stream.next().await {
            let bytes = result.expect("Could not stream contents of tarball while exporting Docker container");
            file.write_all(&bytes)
                .await
                .expect("Could not write streamed-in content to tarball");
        }
    }

    async fn remove_container(&self, container_name: &str, timeout: Option<u64>) {
        self.client
            .stop_container(container_name, timeout.map(|t| StopContainerOptions { t: t as i64 }))
            .await
            .expect("Could not stop container via Docker daemon");

        self.client
            .remove_container(
                container_name,
                Some(RemoveContainerOptions {
                    force: true,
                    ..Default::default()
                }),
            )
            .await
            .expect("Could not remove container via Docker daemon");
    }
}

struct DockerExecReader {
    stream: Pin<Box<dyn Stream<Item = Result<LogOutput, bollard::errors::Error>> + Send>>,
}

#[async_trait]
impl ExecReader for DockerExecReader {
    async fn read(&mut self) -> Option<(String, StreamType)> {
        let (bytes, stream_type) = match self.stream.try_next().await.ok()?? {
            LogOutput::StdErr { message } => (message, StreamType::Stderr),
            LogOutput::StdOut { message } => (message, StreamType::Stdout),
            LogOutput::StdIn { message } => (message, StreamType::Stdin),
            LogOutput::Console { message } => (message, StreamType::Unknown),
        };

        Some((String::from_utf8_lossy(&bytes).into_owned(), stream_type))
    }
}
