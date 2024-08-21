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
    #[serde(default, rename = "type")]
    pub filesystem_type: FilesystemType,
    pub size_mib: u32,
    pub block_size_mib: Option<u32>,
    #[serde(default)]
    pub dd_args: Vec<String>,
    #[serde(default)]
    pub mkfs_args: Vec<String>,
}

#[derive(Deserialize, Debug, Clone)]
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

#[derive(Deserialize, Debug, Clone)]
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
    pub script_inline: Option<String>,
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
}

#[derive(Deserialize, Debug)]
pub struct BuildScriptOverlay {
    #[serde(default)]
    pub source: Option<PathBuf>,
    #[serde(default)]
    pub source_inline: Option<String>,
    pub destination: PathBuf,
    #[serde(default)]
    pub is_directory: bool,
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
    pub include: Vec<PathBuf>,
    #[serde(default)]
    pub create: Vec<PathBuf>,
}

#[derive(Deserialize, Debug, Default, Clone)]
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
