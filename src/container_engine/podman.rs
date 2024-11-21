use std::{collections::HashMap, path::PathBuf};

use async_trait::async_trait;
use futures_util::StreamExt;
use hyper::upgrade::Upgraded;
use hyper_util::rt::TokioIo;
use podman_rest_client::{
    v5::{
        apis::{Containers, Exec, Images, System},
        models::{BindOptions, ContainerExecLibpodBody, ExecStartLibpodBody, Mount, SpecGenerator},
        params::{ContainerStopLibpod, ImagePullLibpod},
    },
    AttachFrame, AttachFrameStream, PodmanRestClient,
};
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

use crate::{
    container_engine::format_uid_gid_string,
    schema::{BuildScriptContainer, BuildScriptContainerImage},
};

use super::{ContainerEngine, ExecParams, ExecReader, StreamType};

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
        let cmd_parts = exec_params
            .cmd
            .split_whitespace()
            .map(|slice| slice.to_owned())
            .collect::<Vec<_>>();

        let exec_id = self
            .client
            .container_exec_libpod(
                &exec_params.container_id,
                ContainerExecLibpodBody {
                    attach_stdout: Some(true),
                    attach_stdin: Some(false),
                    attach_stderr: Some(true),
                    cmd: Some(cmd_parts),
                    user: format_uid_gid_string(exec_params.uid, exec_params.gid),
                    working_dir: exec_params
                        .working_dir
                        .map(|path_buf| path_buf.to_string_lossy().into_owned()),
                    privileged: exec_params.privileged,
                    env: Some(exec_params.env.into_iter().map(|(k, v)| format!("{k}={v}")).collect()),
                    ..Default::default()
                },
            )
            .await
            .expect("Could not create exec via libpod")
            .id;

        let exec_io = self
            .client
            .exec_start_libpod(
                &exec_id,
                ExecStartLibpodBody {
                    detach: Some(false),
                    ..Default::default()
                },
            )
            .await
            .expect("Could not start exec via libpod");
        let stream = AttachFrameStream::new(exec_io);

        Box::new(PodmanExecReader { stream })
    }

    async fn export_container(&self, container_name: &str, tar_path: &PathBuf) {
        let mut file = tokio::fs::File::options()
            .write(true)
            .create(true)
            .append(true)
            .open(tar_path)
            .await
            .expect("Could not open export tarball file");
        let mut stream = self.client.container_export_libpod(container_name);

        while let Some(bytes_result) = stream.next().await {
            let bytes = bytes_result.expect("Could not receive bytes streamed-in from libpod");
            file.write_all(&bytes)
                .await
                .expect("Could not write streamed-in tar contents to file");
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
    }
}

struct PodmanExecReader {
    stream: AttachFrameStream<TokioIo<Upgraded>>,
}

#[async_trait]
impl ExecReader for PodmanExecReader {
    async fn read(&mut self) -> Option<(String, StreamType)> {
        let attach_frame = self.stream.next().await?.ok()?;
        let (bytes, stream_type) = match attach_frame {
            AttachFrame::Stdin(bytes) => (bytes, StreamType::Stdin),
            AttachFrame::Stdout(bytes) => (bytes, StreamType::Stdout),
            AttachFrame::Stderr(bytes) => (bytes, StreamType::Stderr),
        };

        Some((String::from_utf8_lossy(&bytes).into_owned(), stream_type))
    }
}
