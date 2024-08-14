use std::{collections::HashMap, path::PathBuf};

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
    pub engine: ContainerEngine,
    pub image: String,
    #[serde(default)]
    pub rootful: bool,
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
    pub env: Option<HashMap<String, String>>,
    #[serde(default)]
    pub tty: Option<bool>,
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
pub enum ContainerEngine {
    #[default]
    Docker,
    Podman,
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
