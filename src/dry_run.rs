use std::path::{Path, PathBuf};

use uuid::Uuid;

use crate::{
    container_engine::{ContainerEngine, DockerContainerEngine, PodmanContainerEngine},
    package::{unpack_command, BUILD_SCRIPT_FILENAME},
    schema::{BuildScript, ContainerEngineType},
    DryRunArgs, PackArgs, PackageType,
};

pub async fn dry_run_command(dry_run_args: DryRunArgs) {
    let (build_script, container_engine, build_script_path) = prepare_for_run(dry_run_args).await;
}

pub async fn prepare_for_run(dry_run_args: DryRunArgs) -> (BuildScript, Box<dyn ContainerEngine>, PathBuf) {
    let unpack_path = PathBuf::from(format!("/tmp/{}", Uuid::new_v4()));

    unpack_command(PackArgs {
        source_path: dry_run_args.package,
        destination_path: unpack_path.clone(),
        package_type: dry_run_args.package_type,
    })
    .await;

    let build_script_path = match dry_run_args.package_type {
        PackageType::BuildScript => &unpack_path,
        _ => &unpack_path.join(BUILD_SCRIPT_FILENAME),
    };
    let build_script_json = tokio::fs::read_to_string(build_script_path)
        .await
        .expect("Could not read build script from temporary location");
    let build_script =
        toml::from_str::<BuildScript>(&build_script_json).expect("Could not decode build script from TOML");

    let container_engine: Box<dyn ContainerEngine> = match build_script.container.engine {
        ContainerEngineType::Docker => Box::new(DockerContainerEngine::new(
            build_script.container.connection_uri.clone(),
        )),
        ContainerEngineType::Podman => Box::new(PodmanContainerEngine::new(
            build_script.container.connection_uri.clone(),
        )),
    };

    let references = build_script
        .commands
        .iter()
        .filter(|command| command.script_path.is_some())
        .map(|command| command.script_path.as_ref().unwrap())
        .chain(build_script.overlays.iter().map(|overlay| &overlay.destination))
        .collect::<Vec<_>>();

    if let PackageType::BuildScript = dry_run_args.package_type {
        if !references.is_empty() {
            panic!(
                "Build script validation failed: A non-packaged script contains {} reference(s) to outside resources",
                references.len()
            )
        }
    } else {
        for reference_path in references {
            if !reference_path.is_absolute() {
                panic!(
                    "Build script validation failed: {} reference isn't absolute (relative to package root)",
                    reference_path.to_string_lossy()
                );
            }

            let full_path = unpack_path.adjoin_absolute(&reference_path);
            if !tokio::fs::metadata(&full_path).await.is_ok() {
                panic!(
                    "Build script validation failed: {} reference doesn't exist",
                    reference_path.to_string_lossy()
                );
            }
        }
    }

    (build_script, container_engine, unpack_path)
}

pub trait AdjoinAbsolute {
    fn adjoin_absolute(&self, other: &Path) -> PathBuf;
}

impl AdjoinAbsolute for Path {
    fn adjoin_absolute(&self, other: &Path) -> PathBuf {
        let other = other.to_string_lossy();
        self.join(other.trim_start_matches("/"))
    }
}
