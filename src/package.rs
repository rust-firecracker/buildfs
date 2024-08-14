use std::{fs::File, path::PathBuf};

use tar::Archive;

use crate::{schema::BuildScript, PackArgs, PackageType};

pub async fn unpack_command(pack_args: PackArgs) {
    tokio::fs::create_dir_all(&pack_args.destination_path)
        .await
        .expect("Could not ensure that the destination directory exists");

    tokio::task::spawn_blocking(move || {
        let file = File::open(pack_args.source_path).expect("Could not open source file representing the package");

        match pack_args.package_type {
            PackageType::Tarball => {
                let gz_decoder = flate2::read::GzDecoder::new(file);
                let mut archive = Archive::new(gz_decoder);
                archive
                    .unpack(pack_args.destination_path)
                    .expect("Extracting package tarball failed");
            }
            PackageType::Tar => {
                let mut archive = Archive::new(file);
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
    match pack_args.package_type {
        PackageType::OnlyBuildScript => {
            tokio::fs::copy(pack_args.source_path, pack_args.destination_path)
                .await
                .expect("Could not copy build script to its destination path");
            return;
        }
        PackageType::Directory => {
            tokio::task::spawn_blocking(move || copy_dir_blocking(&pack_args.source_path, &pack_args.destination_path))
                .await
                .expect("Join on blocking task failed")
                .expect("Blocking task to copy failed");
            return;
        }
        _ => {}
    }

    let build_script_json = tokio::fs::read_to_string(pack_args.source_path)
        .await
        .expect("Could not read source build script");
    let build_script = toml::from_str::<BuildScript>(&build_script_json)
        .expect("Could not decode the given build script file from TOML");
}

fn copy_dir_blocking(source_path: &PathBuf, destination_path: &PathBuf) -> Result<(), std::io::Error> {
    std::fs::create_dir_all(destination_path)?;

    for entry in std::fs::read_dir(source_path)? {
        let entry = entry?;

        if entry
            .file_type()
            .expect("Could not get file type of directory entry")
            .is_dir()
        {
            copy_dir_blocking(&entry.path(), &destination_path.as_path().join(entry.file_name()))?;
        } else {
            std::fs::copy(entry.path(), destination_path.as_path().join(entry.file_name()))?;
        }
    }

    Ok(())
}
