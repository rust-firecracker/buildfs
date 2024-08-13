use std::path::PathBuf;

use serde::Deserialize;

#[derive(Deserialize, Debug)]
pub struct Config {
    pub filesystem: FilesystemConfig,
    pub container: ContainerConfig,
    #[serde(default)]
    pub scripts: Vec<ScriptConfig>,
    #[serde(default)]
    pub overlays: Vec<OverlayConfig>,
    #[serde(default)]
    pub export: ExportConfig,
}

#[derive(Deserialize, Debug)]
pub struct FilesystemConfig {
    #[serde(default)]
    pub preferred_name: Option<String>,
    #[serde(default, rename = "type")]
    pub filesystem_type: FsFilesystemType,
    pub size_mib: u32,
}

#[derive(Deserialize, Debug)]
pub struct ContainerConfig {
    #[serde(default)]
    pub engine: FsContainerEngine,
    pub image: String,
    #[serde(default)]
    pub rootful: bool,
}

#[derive(Deserialize, Debug)]
pub struct ScriptConfig {
    #[serde(default)]
    pub shell: Option<String>,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub path: Option<PathBuf>,
}

#[derive(Deserialize, Debug)]
pub struct OverlayConfig {
    pub source: PathBuf,
    pub destination: PathBuf,
}

#[derive(Deserialize, Debug, Default)]
pub struct ExportConfig {
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
pub enum FsContainerEngine {
    #[default]
    Docker,
    Podman,
}

#[derive(Deserialize, Debug, Default)]
pub enum FsFilesystemType {
    #[default]
    Ext4,
    Btrfs,
    Squashfs,
    Vfat,
    Xfs,
}
