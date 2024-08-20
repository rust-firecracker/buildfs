use std::path::{Path, PathBuf};

use uuid::Uuid;

use crate::{
    container_engine::{ContainerEngine, DockerContainerEngine, PodmanContainerEngine},
    package::{get_package_type, unpack_command, BUILD_SCRIPT_FILENAME},
    schema::{BuildScript, ContainerEngineType},
    DryRunArgs, PackageType, UnpackArgs,
};

pub async fn dry_run_command(dry_run_args: DryRunArgs) {
    let (_, container_engine, _) = prepare_for_run(dry_run_args).await;
    container_engine.ping().await;
}

pub async fn prepare_for_run(dry_run_args: DryRunArgs) -> (BuildScript, Box<dyn ContainerEngine>, PathBuf) {
    let package_type = get_package_type(&dry_run_args.package).await;

    let (unpack_path, build_script_path) = match package_type {
        PackageType::BuildScript => (dry_run_args.package.clone(), dry_run_args.package),
        PackageType::Directory => (
            dry_run_args.package.clone(),
            dry_run_args.package.join(BUILD_SCRIPT_FILENAME),
        ),
        _ => {
            let tmp_path = PathBuf::from(format!("/tmp/{}", Uuid::new_v4()));
            unpack_command(UnpackArgs {
                source_path: dry_run_args.package,
                destination_path: tmp_path.clone(),
            })
            .await;
            (tmp_path.clone(), tmp_path.join(BUILD_SCRIPT_FILENAME))
        }
    };
    log::info!("Unpacked package during run into {unpack_path:?}. Build script located at {build_script_path:?}");

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
    log::info!("Connected to container engine {}", build_script.container.engine);

    let references = build_script
        .commands
        .iter()
        .filter_map(|command| command.script_path.as_ref())
        .chain(build_script.overlays.iter().map(|overlay| &overlay.source))
        .chain(
            build_script
                .container
                .volumes
                .iter()
                .map(|(source_path, _)| source_path),
        )
        .collect::<Vec<_>>();

    if let PackageType::BuildScript = package_type {
        if !references.is_empty() {
            panic!(
                "Build script validation failed: A non-packaged script contains {} reference(s) to outside resources",
                references.len()
            )
        }
    } else {
        for reference_path in &references {
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

    let empty_commands = build_script
        .commands
        .iter()
        .filter(|command| command.script.is_none() && command.script_path.is_none() && command.command.is_none())
        .count();
    if empty_commands > 0 {
        panic!("Build script validation failed: {empty_commands} command(s) contain no reference to a script, a script path or an inline command");
    }

    log::info!("Validated the build script: {} reference(s) found", references.len());

    (build_script, container_engine, unpack_path)
}

pub trait AdjoinAbsolute {
    fn adjoin_absolute(&self, other: &Path) -> PathBuf;
}

impl AdjoinAbsolute for PathBuf {
    fn adjoin_absolute(&self, other: &Path) -> PathBuf {
        let other = other.to_string_lossy();
        self.join(other.trim_start_matches("/"))
    }
}
