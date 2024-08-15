use std::{collections::HashMap, fs::File};

use flate2::Compression;
use tokio::task::JoinSet;

use crate::{schema::BuildScript, PackArgs, PackageType};

static BUILD_SCRIPT_NAME: &'static str = "build.toml";

pub async fn unpack_command(pack_args: PackArgs) {
    tokio::fs::create_dir_all(&pack_args.destination_path)
        .await
        .expect("Could not ensure that the destination directory exists");

    tokio::task::spawn_blocking(move || {
        let file = File::open(pack_args.source_path).expect("Could not open source file representing the package");

        match pack_args.package_type {
            PackageType::Tarball => {
                let gz_decoder = flate2::read::GzDecoder::new(file);
                let mut archive = tar::Archive::new(gz_decoder);
                archive
                    .unpack(pack_args.destination_path)
                    .expect("Extracting package tarball failed");
            }
            PackageType::Tar => {
                let mut archive = tar::Archive::new(file);
                archive
                    .unpack(pack_args.destination_path)
                    .expect("Extracting package tar failed");
            }
            _ => {
                println!("The given package type cannot be unpacked as it wasn't packed in the first place");
            }
        }
    })
    .await
    .expect("Blocking task for extraction failed");
}

pub async fn pack_command(pack_args: PackArgs) {
    if let PackageType::OnlyBuildScript = pack_args.package_type {
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
        pack_args.destination_path.join(BUILD_SCRIPT_NAME),
    );

    for command in build_script.commands {
        if let Some(script_path) = command.script_path {
            paths.insert(
                source_parent_path.join(&script_path),
                pack_args.destination_path.join(&script_path),
            );
        }
    }

    for overlay in build_script.overlays {
        paths.insert(
            source_parent_path.join(&overlay.source),
            pack_args.destination_path.join(&overlay.source),
        );
    }

    let mut copy_join_set = JoinSet::new();
    for (src_path, dst_path) in paths {
        copy_join_set.spawn_blocking(move || std::fs::copy(src_path, dst_path));
    }

    while let Some(Ok(result)) = copy_join_set.join_next().await {
        if let Err(err) = result {
            panic!("Copy task failed: {err}");
        }
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

        if let PackageType::Tarball = pack_args.package_type {
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
