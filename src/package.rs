use std::{collections::HashMap, fs::File, path::PathBuf};

use flate2::Compression;
use tokio::task::JoinSet;

use crate::{schema::BuildScript, PackArgs, PackageType, UnpackArgs};

pub static BUILD_SCRIPT_FILENAME: &'static str = "build.toml";

pub async fn get_package_type(path: &PathBuf) -> PackageType {
    let package_type = {
        let metadata = tokio::fs::metadata(path)
            .await
            .expect("Could not inspect file/directory metadata");
        if metadata.is_dir() {
            return PackageType::Directory;
        }

        let extension = path.extension().expect("File has no extension").to_string_lossy();
        match extension.to_string().as_str() {
            "toml" => PackageType::BuildScript,
            "tar" => PackageType::Tar,
            "tar.gz" => PackageType::TarGz,
            _ => {
                panic!("File extension {extension} is not recognizable as a type of package");
            }
        }
    };
    log::info!("Detected package type of {path:?} to be {package_type}");

    package_type
}

pub async fn unpack_command(unpack_args: UnpackArgs) {
    let package_type = get_package_type(&unpack_args.source_path).await;
    tokio::fs::create_dir_all(&unpack_args.destination_path)
        .await
        .expect("Could not ensure that the destination directory exists");

    tokio::task::spawn_blocking(move || {
        let file = File::open(&unpack_args.source_path).expect("Could not open source file representing the package");

        match package_type {
            PackageType::TarGz => {
                let gz_decoder = flate2::read::GzDecoder::new(file);
                let mut archive = tar::Archive::new(gz_decoder);
                archive
                    .unpack(&unpack_args.destination_path)
                    .expect("Extracting package tarball failed");
                log::info!(
                    "Extraction of {:?} into {:?} finished",
                    unpack_args.source_path,
                    unpack_args.destination_path
                );
            }
            PackageType::Tar => {
                let mut archive = tar::Archive::new(file);
                archive
                    .unpack(&unpack_args.destination_path)
                    .expect("Extracting package tar failed");
                log::info!(
                    "Extraction of {:?} into {:?} finished",
                    unpack_args.source_path,
                    unpack_args.destination_path
                );
            }
            package_type => {
                log::warn!("Tried to unpack a package of type {package_type}, which cannot be unpacked");
            }
        }
    })
    .await
    .expect("Join on blocking task failed");
}

pub async fn pack_command(pack_args: PackArgs) {
    if let PackageType::BuildScript = pack_args.package_type {
        tokio::fs::copy(pack_args.source_path, pack_args.destination_path)
            .await
            .expect("Could not copy build script to its destination path");
        return;
    }

    tokio::fs::create_dir_all(&pack_args.destination_path)
        .await
        .expect("Could not ensure destination path exists as a directory");

    let source_parent_path = pack_args
        .source_path
        .parent()
        .expect("Source path has no parent for lookup");

    let build_script_json = tokio::fs::read_to_string(&pack_args.source_path)
        .await
        .expect("Could not read source build script");
    let build_script = toml::from_str::<BuildScript>(&build_script_json)
        .expect("Could not decode the given build script file from TOML");
    let mut paths = HashMap::with_capacity(1);
    paths.insert(
        pack_args.source_path.clone(),
        pack_args.destination_path.join(BUILD_SCRIPT_FILENAME),
    );

    for command in build_script.commands {
        if let Some(script_path) = command.script_path {
            paths.insert(
                source_parent_path.join(&script_path),
                pack_args.destination_path.join(&script_path),
            );
        }
    }

    for source_path in build_script
        .overlays
        .iter()
        .filter_map(|overlay| overlay.source.as_ref())
    {
        paths.insert(
            source_parent_path.join(source_path),
            pack_args.destination_path.join(source_path),
        );
    }

    let mut copy_join_set = JoinSet::new();
    for (src_path, dst_path) in paths {
        copy_join_set.spawn_blocking(move || std::fs::copy(src_path, dst_path));
    }

    while let Some(result) = copy_join_set.join_next().await {
        result
            .expect("Joining on copy blocking task failed")
            .expect("Copy blocking task failed");
    }

    if let PackageType::Directory = pack_args.package_type {
        return;
    }

    tokio::task::spawn_blocking(move || {
        let mut tmp_destination_path = pack_args.destination_path.clone();
        tmp_destination_path.as_mut_os_string().push("_tmp");
        std::fs::rename(&pack_args.destination_path, &tmp_destination_path)
            .expect("Could not rename destination directory to a temporary one");

        let file = File::create(pack_args.destination_path).expect("Could not create destination file");

        if let PackageType::TarGz = pack_args.package_type {
            let gz_encoder = flate2::write::GzEncoder::new(file, Compression::best());
            let mut tar = tar::Builder::new(gz_encoder);
            tar.append_dir_all(".", &tmp_destination_path)
                .expect("Could not insert into tarball");
        } else {
            let mut tar = tar::Builder::new(file);
            tar.append_dir_all(".", &tmp_destination_path)
                .expect("Could not insert into tar");
        }

        std::fs::remove_dir_all(tmp_destination_path).expect("Could not remove temporary destination");
    })
    .await
    .expect("Could not join on blocking task");
}
