use std::{
    collections::HashMap, fs::Permissions, os::unix::fs::PermissionsExt, path::PathBuf, process::Stdio, sync::Arc,
};

use sys_mount::{Mount, UnmountDrop, UnmountFlags};
use tokio::{io::AsyncWriteExt, process::Command, task::JoinSet};
use uuid::Uuid;

use crate::{
    container_engine::{ContainerEngine, ExecParams},
    dry_run::{prepare_for_run, AdjoinAbsolute},
    schema::{
        BuildScript, BuildScriptCommand, BuildScriptExport, BuildScriptFilesystem, BuildScriptOverlay, FilesystemType,
    },
    RunArgs,
};

pub async fn run_command(run_args: RunArgs, no_exec_logs: bool) {
    let (build_script, container_engine, unpack_path, can_delete_unpack_path) =
        prepare_for_run(&run_args.dry_run_args).await;

    let (container_id, container_name, inline_mount_paths) =
        pull_and_start_container(&container_engine, &build_script, &unpack_path).await;

    run_commands_in_container(
        &inline_mount_paths,
        build_script.commands,
        &container_id,
        &container_name,
        &container_engine,
        no_exec_logs,
    )
    .await;

    let container_rootfs_path = export_and_remove_container(
        &container_engine,
        &container_name,
        can_delete_unpack_path,
        &unpack_path,
        inline_mount_paths,
        build_script.container.wait_timeout_s,
    )
    .await;

    let (rootfs_mount_path, unmount_drop) = init_rootfs(build_script.filesystem, &run_args, no_exec_logs).await;

    apply_overlays_and_finalize(
        Arc::new(container_rootfs_path),
        Arc::new(rootfs_mount_path),
        build_script.overlays,
        build_script.export,
        Arc::new(unpack_path),
        unmount_drop,
    )
    .await;
}

async fn pull_and_start_container(
    container_engine: &Box<dyn ContainerEngine>,
    build_script: &BuildScript,
    unpack_path: &PathBuf,
) -> (String, String, HashMap<String, (PathBuf, PathBuf)>) {
    container_engine.pull_image(&build_script.container.image).await;
    log::info!("Pulled image: {}", build_script.container.image.full_name());

    let base_script_path = PathBuf::from("/__scripts");
    let mut volumes = build_script
        .commands
        .iter()
        .filter_map(|command| command.script_path.clone())
        .map(|script_path| {
            (
                unpack_path.adjoin_absolute(&script_path),
                base_script_path.adjoin_absolute(&script_path),
            )
        })
        .collect::<HashMap<_, _>>();
    let mut inline_mount_paths = HashMap::new();

    for command in &build_script.commands {
        if let Some(ref script) = command.script_inline {
            let host_path = get_tmp_path();
            let mount_path = PathBuf::from(format!("/__scripts/{}", Uuid::new_v4()));
            tokio::fs::write(&host_path, script)
                .await
                .expect("Could not write inline script to a bind-mounted host path");
            tokio::fs::set_permissions(&host_path, Permissions::from_mode(0o111))
                .await
                .expect("Could not make inline script file executable");

            volumes.insert(host_path.clone(), mount_path.clone());
            inline_mount_paths.insert(script.clone(), (host_path, mount_path));
        }
    }

    log::debug!("Resolved script volumes fBuildScriptor container to: {volumes:?}");

    let (container_id, container_name) = container_engine
        .start_container(build_script.container.clone(), volumes)
        .await;
    log::info!("Created and started container with name {container_name} and ID {container_id}");

    (container_id, container_name, inline_mount_paths)
}

async fn run_commands_in_container(
    inline_mount_paths: &HashMap<String, (PathBuf, PathBuf)>,
    commands: Vec<BuildScriptCommand>,
    container_id: &str,
    container_name: &str,
    container_engine: &Box<dyn ContainerEngine>,
    no_exec_logs: bool,
) {
    let base_script_path = PathBuf::from("/__scripts");

    for command in commands {
        let mut exec_params = ExecParams {
            container_name,
            container_id,
            cmd: "".to_string(),
            uid: command.uid,
            gid: command.gid,
            working_dir: command.working_dir,
            privileged: command.privileged,
            env: command.env,
        };

        if let Some(command_text) = command.command {
            log::info!("Exec-ing simple command inside container: \"{command_text}\"");
            exec_params.cmd = command_text;
        }

        if let Some(script_path) = command.script_path {
            let actual_script_path = base_script_path.adjoin_absolute(&script_path);
            log::info!("Exec-ing script inside container that is bind-mounted into: {actual_script_path:?}");
            exec_params.cmd = actual_script_path.to_string_lossy().to_string();
        }

        if let Some(script) = command.script_inline {
            let (_, inline_script_path) = inline_mount_paths
                .get(&script)
                .expect("Could not resolve expectedly inserted mount path of an inlined script");
            log::info!("Exec-ing inline script inside container that is bind-mounted into: {inline_script_path:?}");
            exec_params.cmd = inline_script_path.to_string_lossy().to_string();
        }

        let mut exec_reader = container_engine.exec_in_container(exec_params).await;
        while let Some(output) = exec_reader.read().await {
            if !no_exec_logs {
                print!("{output}");
            }
        }
    }
}

async fn export_and_remove_container(
    container_engine: &Box<dyn ContainerEngine>,
    container_name: &str,
    can_delete_unpack_path: bool,
    unpack_path: &PathBuf,
    inline_mount_paths: HashMap<String, (PathBuf, PathBuf)>,
    wait_timeout: Option<u64>,
) -> PathBuf {
    let container_rootfs_tar_path = get_tmp_path();
    let container_rootfs_path = get_tmp_path();
    container_engine
        .export_container(&container_name, &container_rootfs_tar_path)
        .await;
    log::info!("Export of container rootfs finished into tarball located at {container_rootfs_tar_path:?}");

    let container_rootfs_path_clone = container_rootfs_path.clone();
    tokio::task::spawn_blocking(move || {
        let rootfs_tar_file =
            std::fs::File::open(&container_rootfs_tar_path).expect("Could not open rootfs tarball file");
        let mut archive = tar::Archive::new(rootfs_tar_file);
        archive
            .unpack(&container_rootfs_path_clone)
            .expect("Could not unpack rootfs tarball");
        drop(archive);

        std::fs::remove_file(&container_rootfs_tar_path).expect("Could not remove rootfs tarball");
        log::info!("Unpacked container rootfs from tarball into {container_rootfs_path_clone:?}");
    })
    .await
    .expect("Could not join on blocking task");

    container_engine.remove_container(&container_name, wait_timeout).await;
    log::info!("Stopped and removed container");

    let mut cleanup_join_set = JoinSet::new();
    for (_, (host_path, _)) in inline_mount_paths {
        cleanup_join_set.spawn_blocking(move || std::fs::remove_file(host_path));
    }

    if can_delete_unpack_path {
        let unpack_path = unpack_path.clone();
        cleanup_join_set.spawn_blocking(move || std::fs::remove_dir_all(unpack_path));
    }

    while let Some(result) = cleanup_join_set.join_next().await {
        result
            .expect("Could not join on a set of blocking tasks intended for removing files/directories")
            .expect("Could not cleanup a path");
    }

    log::info!("Cleaned up all temporary resources");
    container_rootfs_path
}

async fn init_rootfs(
    filesystem: BuildScriptFilesystem,
    run_args: &RunArgs,
    no_exec_logs: bool,
) -> (PathBuf, UnmountDrop<Mount>) {
    let dd_block_size_mib = match filesystem.block_size_mib {
        Some(mib) => mib,
        None => 1,
    };

    let mkfs_name = match filesystem.filesystem_type {
        FilesystemType::Ext4 => "mkfs.ext4",
        FilesystemType::Btrfs => "mkfs.btrfs",
        FilesystemType::Squashfs => "mksquashfs",
        FilesystemType::Vfat => "mkfs.vfat",
        FilesystemType::Xfs => "mkfs.xfs",
    };
    let mkfs_path = which::which(mkfs_name).expect("Could not locate appropriate mkfs binary in PATH");
    log::debug!("Located appropriate \"mkfs\" binary at: {mkfs_path:?}");

    let dd_path = which::which("dd").expect("Could not locate \"dd\" binary in PATH");
    log::debug!("Located \"dd\" binary at: {dd_path:?}");

    let mut dd_command = Command::new(dd_path);
    let rootfs_mount_path = get_tmp_path();
    dd_command.arg("if=/dev/zero");
    dd_command.arg(format!("of={}", run_args.output_path.to_string_lossy()));
    dd_command.arg(format!("bs={}M", dd_block_size_mib));
    dd_command.arg(format!("count={}", filesystem.size_mib / dd_block_size_mib));
    if no_exec_logs {
        dd_command.stdout(Stdio::null());
        dd_command.stderr(Stdio::null());
    }
    dd_command.args(filesystem.dd_args);

    let dd_exit_status = dd_command.status().await.expect("Failed to fork \"dd\" process");

    if !dd_exit_status.success() {
        panic!("\"dd\" invocation failed with exit status: {dd_exit_status}");
    }

    let mut mkfs_command = Command::new(mkfs_path);
    mkfs_command.arg(run_args.output_path.to_string_lossy().to_string());
    if no_exec_logs {
        mkfs_command.stdout(Stdio::null());
        mkfs_command.stderr(Stdio::null());
    }
    mkfs_command.args(filesystem.mkfs_args);

    let mkfs_exit_status = mkfs_command.status().await.expect("Failed to fork \"mkfs\" process");

    if !mkfs_exit_status.success() {
        panic!("\"mkfs\" invocation failed with exit status: {mkfs_exit_status}");
    }

    tokio::fs::create_dir(&rootfs_mount_path)
        .await
        .expect("Could not create filesystem mount point directory");
    let unmount_drop = Mount::builder()
        .fstype(match filesystem.filesystem_type {
            FilesystemType::Ext4 => "ext4",
            FilesystemType::Btrfs => "btrfs",
            FilesystemType::Squashfs => "squashfs",
            FilesystemType::Vfat => "vfat",
            FilesystemType::Xfs => "xfs",
        })
        .mount_autodrop(&run_args.output_path, &rootfs_mount_path, UnmountFlags::empty())
        .expect("Could not mount rootfs");

    log::info!(
        "Created the filesystem at {:?} with mount at {rootfs_mount_path:?}",
        run_args.output_path
    );

    (rootfs_mount_path, unmount_drop)
}

async fn apply_overlays_and_finalize(
    source_path: Arc<PathBuf>,
    destination_path: Arc<PathBuf>,
    overlays: Vec<BuildScriptOverlay>,
    export: BuildScriptExport,
    unpack_path: Arc<PathBuf>,
    unmount_drop: UnmountDrop<Mount>,
) {
    for overlay in overlays {
        if overlay.is_directory {
            let (unpack_path, destination_path) = (unpack_path.clone(), destination_path.clone());

            tokio::task::spawn_blocking(move || {
                fs_extra::dir::copy(
                    unpack_path.adjoin_absolute(&overlay.source.unwrap()),
                    destination_path.adjoin_absolute(&overlay.destination),
                    &fs_extra::dir::CopyOptions::default(),
                )
            })
            .await
            .expect("Join on blocking task failed")
            .expect("Recursively copying overlay failed");

            continue;
        }

        if let Some(parent_path) = overlay.destination.parent() {
            tokio::fs::create_dir_all(destination_path.adjoin_absolute(parent_path))
                .await
                .expect("Could not create parent directory tree for overlayed file");
        }

        if let Some(source_path) = overlay.source {
            tokio::fs::copy(
                unpack_path.adjoin_absolute(&source_path),
                destination_path.adjoin_absolute(&overlay.destination),
            )
            .await
            .expect("Could not copy overlayed file");
        }

        if let Some(source_inline) = overlay.source_inline {
            let mut file = tokio::fs::File::options()
                .create_new(true)
                .write(true)
                .open(destination_path.adjoin_absolute(&overlay.destination))
                .await
                .expect("Could not create and open overlayed inline file");
            file.write_all(source_inline.as_bytes())
                .await
                .expect("Could not write overlayed inline file's contents");
        }
    }

    log::info!("Applied all overlays to the mounted filesystem");

    let mut join_set = JoinSet::new();

    for dir_path in export.directories.include {
        let (source_path, destination_path) = (source_path.clone(), destination_path.clone());
        join_set.spawn(async move {
            let mut command = Command::new(which::which("cp").expect("Could not locate \"cp\" binary in PATH"));
            command.arg("-r");
            command.arg("-p");
            command.arg(source_path.adjoin_absolute(&dir_path));
            command.arg(destination_path.adjoin_absolute(&dir_path).parent().unwrap());
            let exit_status = command
                .status()
                .await
                .expect("Could not fork \"cp\" to perform recursive copy");

            if !exit_status.success() {
                panic!("\"cp\" exited with non-zero exit status: {exit_status}");
            }
        });
    }

    for dir_path in export.directories.create {
        let destination_path = destination_path.clone();
        join_set.spawn_blocking(move || {
            std::fs::create_dir_all(destination_path.adjoin_absolute(&dir_path))
                .expect("Could not create directory tree for export-created directory")
        });
    }

    for file_path in export.files.include {
        let (source_path, destination_path) = (source_path.clone(), destination_path.clone());
        join_set.spawn_blocking(move || {
            if let Some(parent_path) = file_path.parent() {
                std::fs::create_dir_all(destination_path.adjoin_absolute(parent_path))
                    .expect("Could not create parent directory tree for export-included file");
            }

            std::fs::copy(
                source_path.adjoin_absolute(&file_path),
                destination_path.adjoin_absolute(&file_path),
            )
            .expect("Could not move export-included file to destination");
        });
    }

    for file_path in export.files.create {
        let destination_path = destination_path.clone();
        join_set.spawn_blocking(move || {
            if let Some(parent_path) = file_path.parent() {
                std::fs::create_dir_all(destination_path.adjoin_absolute(parent_path))
                    .expect("Could not create parent directory tree for export-created file");
            }

            std::fs::File::create_new(destination_path.adjoin_absolute(&file_path))
                .expect("Could not create export-created file");
        });
    }

    log::info!(
        "Spawned {} threads for exporting into the mounted filesystem",
        join_set.len()
    );

    while let Some(result) = join_set.join_next().await {
        result.expect("Could not join on blocking I/O task");
    }

    drop(unmount_drop);
    log::info!("All export threads finished execution, filesystem unmounted");

    tokio::fs::remove_dir_all(source_path.as_path())
        .await
        .expect("Could not clean up unneeded container rootfs directory");
    log::info!("Root filesystem creation finished normally");
}

fn get_tmp_path() -> PathBuf {
    PathBuf::from(format!("/tmp/{}", Uuid::new_v4()))
}
