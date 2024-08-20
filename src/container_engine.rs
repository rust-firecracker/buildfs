use std::{collections::HashMap, path::PathBuf};

use async_trait::async_trait;
use bollard::{container::CreateContainerOptions, ClientVersion, Docker};
use podman_rest_client::{
    v5::{
        apis::{Containers, System},
        models::{BindOptions, SpecGenerator},
    },
    PodmanRestClient,
};
use uuid::Uuid;

use crate::schema::BuildScriptContainer;

#[async_trait]
pub trait ContainerEngine {
    async fn ping(&self);

    async fn start_container(&self, container: BuildScriptContainer, extra_volumes: HashMap<PathBuf, PathBuf>);
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

    async fn start_container(&self, container: BuildScriptContainer, mut extra_volumes: HashMap<PathBuf, PathBuf>) {
        let name = Uuid::new_v4().to_string();
        extra_volumes.extend(container.volumes);

        let spec_generator = SpecGenerator {
            image: Some(container.image),
            privileged: Some(container.rootful),
            terminal: Some(true),
            remove: Some(true),
            env: Some(container.env),
            hostname: container.hostname,
            oci_runtime: container.oci_runtime,
            timeout: container.timeout,
            cap_add: container.cap_add,
            cap_drop: container.cap_drop,
            name: Some(name.clone()),
            mounts: Some(
                extra_volumes
                    .into_iter()
                    .map(|(src, dst)| podman_rest_client::v5::models::Mount {
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
        dbg!(&spec_generator);

        let _ = self
            .client
            .container_create_libpod(spec_generator)
            .await
            .expect("Could not create container via libpod");

        self.client
            .container_start_libpod(&name, None)
            .await
            .expect("Could not start container via libpod");
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

    async fn start_container(&self, container: BuildScriptContainer, mut extra_volumes: HashMap<PathBuf, PathBuf>) {
        extra_volumes.extend(container.volumes);

        let name = Uuid::new_v4().to_string();
        let config = bollard::container::Config {
            image: Some(container.image),
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

        self.client
            .create_container(
                Some(CreateContainerOptions {
                    name: &name,
                    platform: None,
                }),
                config,
            )
            .await
            .expect("Could not create container via Docker daemon");

        self.client
            .start_container::<String>(&name, None)
            .await
            .expect("Could not start container via Docker daemon");
    }
}
