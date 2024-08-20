use std::{collections::HashMap, path::PathBuf};

use async_trait::async_trait;

use crate::schema::{BuildScriptContainer, BuildScriptContainerImage};

pub mod docker;
pub mod podman;

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

    async fn export_container(&self, container_name: &str, tar_path: &PathBuf);

    async fn remove_container(&self, container_name: &str, timeout: Option<u64>);
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

pub(super) fn format_uid_gid_string(uid: Option<u32>, gid: Option<u32>) -> Option<String> {
    match uid {
        Some(uid) => match gid {
            Some(gid) => Some(format!("{}:{}", uid.to_string(), gid.to_string())),
            None => Some(uid.to_string()),
        },
        None => None,
    }
}
