use std::{collections::HashMap, fmt::Display, path::PathBuf};

use serde::Deserialize;

#[derive(Deserialize, Debug)]
pub struct BuildScript {
    pub filesystem: BuildScriptFilesystem,
    pub container: BuildScriptContainer,
    #[serde(default)]
    pub commands: Vec<BuildScriptCommand>,
    #[serde(default)]
    pub overlays: Vec<BuildScriptOverlay>,
    #[serde(default)]
    pub export: BuildScriptExport,
}

#[derive(Deserialize, Debug)]
pub struct BuildScriptFilesystem {
    #[serde(default)]
    pub preferred_name: Option<String>,
    #[serde(default, rename = "type")]
    pub filesystem_type: FilesystemType,
    pub size_mib: u32,
}

#[derive(Deserialize, Debug)]
pub struct BuildScriptContainer {
    #[serde(default)]
    pub engine: ContainerEngineType,
    pub image: BuildScriptContainerImage,
    #[serde(default)]
    pub rootful: bool,
    #[serde(default)]
    pub connection_uri: Option<String>,
    #[serde(default)]
    pub volumes: HashMap<PathBuf, PathBuf>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default)]
    pub hostname: Option<String>,
    #[serde(default)]
    pub oci_runtime: Option<String>,
    #[serde(default)]
    pub timeout: Option<u64>,
    #[serde(default)]
    pub cap_add: Option<Vec<String>>,
    #[serde(default)]
    pub cap_drop: Option<Vec<String>>,
}

#[derive(Deserialize, Debug)]
pub struct BuildScriptContainerImage {
    pub name: String,
    pub tag: String,
}

impl BuildScriptContainerImage {
    pub fn full_name(&self) -> String {
        format!("{}:{}", self.name, self.tag)
    }
}

#[derive(Deserialize, Debug)]
pub struct BuildScriptCommand {
    // only one of these can be specified
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub script: Option<String>,
    #[serde(default)]
    pub script_path: Option<PathBuf>,
    // options addable to any
    #[serde(default)]
    pub uid: Option<u32>,
    #[serde(default)]
    pub gid: Option<u32>,
    #[serde(default)]
    pub working_dir: Option<PathBuf>,
    #[serde(default)]
    pub privileged: Option<bool>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default)]
    pub attach_stdout: Option<bool>,
    #[serde(default)]
    pub attach_stdin: Option<bool>,
    #[serde(default)]
    pub attach_stderr: Option<bool>,
}

#[derive(Deserialize, Debug)]
pub struct BuildScriptOverlay {
    pub source: PathBuf,
    pub destination: PathBuf,
}

#[derive(Deserialize, Debug, Default)]
pub struct BuildScriptExport {
    #[serde(default)]
    pub files: Export,
    #[serde(default)]
    pub directories: Export,
}

#[derive(Deserialize, Debug, Default)]
pub struct Export {
    #[serde(default)]
    pub include: Vec<String>,
    #[serde(default)]
    pub exclude: Vec<String>,
    #[serde(default)]
    pub create: Vec<String>,
}

#[derive(Deserialize, Debug, Default)]
pub enum ContainerEngineType {
    #[default]
    Docker,
    Podman,
}

impl Display for ContainerEngineType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ContainerEngineType::Docker => write!(f, "Docker"),
            ContainerEngineType::Podman => write!(f, "Podman"),
        }
    }
}

#[derive(Deserialize, Debug, Default)]
pub enum FilesystemType {
    #[default]
    Ext4,
    Btrfs,
    Squashfs,
    Vfat,
    Xfs,
}
