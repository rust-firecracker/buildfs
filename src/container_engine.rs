use async_trait::async_trait;
use bollard::{ClientVersion, Docker};
use podman_rest_client::{
    v5::apis::{System, SystemCompat},
    PodmanRestClient,
};

#[async_trait]
pub trait ContainerEngine {
    async fn ping(&self);
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
}
