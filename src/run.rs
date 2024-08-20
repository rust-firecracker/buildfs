use std::{collections::HashMap, fs::Permissions, os::unix::fs::PermissionsExt, path::PathBuf};

use tokio::{process::Command, task::JoinSet};
use uuid::Uuid;

use crate::{
    container_engine::{ContainerEngine, ExecParams},
    dry_run::{prepare_for_run, AdjoinAbsolute},
    schema::{BuildScript, BuildScriptCommand, FilesystemType},
    RunArgs,
};

pub async fn run_command(run_args: RunArgs) {
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
    )
    .await;

    export_and_remove_container(
        &container_engine,
        &container_name,
        &run_args,
        can_delete_unpack_path,
        unpack_path,
        inline_mount_paths,
    )
    .await;

    init_rootfs(
        build_script.filesystem.filesystem_type,
        build_script.filesystem.size_mib,
        build_script.filesystem.dd_block_size_mib,
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
            print!("{output}");
        }
    }
}

async fn export_and_remove_container(
    container_engine: &Box<dyn ContainerEngine>,
    container_name: &str,
    run_args: &RunArgs,
    can_delete_unpack_path: bool,
    unpack_path: PathBuf,
    inline_mount_paths: HashMap<String, (PathBuf, PathBuf)>,
) {
    let container_rootfs_tar_path = get_tmp_path();
    let container_rootfs_path = get_tmp_path();
    container_engine
        .export_container(&container_name, &container_rootfs_tar_path)
        .await;
    log::info!("Export of container rootfs finished into tarball located at {container_rootfs_tar_path:?}");
    tokio::task::spawn_blocking(move || {
        let rootfs_tar_file =
            std::fs::File::open(&container_rootfs_tar_path).expect("Could not open rootfs tarball file");
        let mut archive = tar::Archive::new(rootfs_tar_file);
        archive
            .unpack(&container_rootfs_path)
            .expect("Could not unpack rootfs tarball");
        drop(archive);

        std::fs::remove_file(&container_rootfs_tar_path).expect("Could not remove rootfs tarball");
        log::info!("Unpacked container rootfs from tarball into {container_rootfs_path:?}");
    })
    .await
    .expect("Could not join on blocking task");

    container_engine
        .remove_container(&container_name, run_args.timeout)
        .await;
    log::info!("Stopped and removed container");

    let mut cleanup_join_set = JoinSet::new();
    for (_, (host_path, _)) in inline_mount_paths {
        cleanup_join_set.spawn_blocking(move || std::fs::remove_file(host_path));
    }

    if can_delete_unpack_path {
        cleanup_join_set.spawn_blocking(move || std::fs::remove_dir_all(unpack_path));
    }

    while let Some(result) = cleanup_join_set.join_next().await {
        result
            .expect("Could not join on a set of blocking tasks intended for removing files/directories")
            .expect("Could not cleanup a path");
    }

    log::info!("Cleaned up all temporary resources");
}

async fn init_rootfs(filesystem_type: FilesystemType, size_mib: u32, dd_block_size_mib: Option<u32>) {
    let dd_block_size_mib = match dd_block_size_mib {
        Some(mib) => mib,
        None => 1,
    };

    let mkfs_name = match filesystem_type {
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
    let rootfs_path = get_tmp_path();
    let rootfs_mount_path = get_tmp_path();
    dd_command.arg("if=/dev/zero");
    dd_command.arg(format!("of={}", rootfs_path.to_string_lossy()));
    dd_command.arg(format!("bs={}M", dd_block_size_mib));
    dd_command.arg(format!("count={}", size_mib / dd_block_size_mib));
    let dd_exit_status = dd_command.status().await.expect("Failed to fork \"dd\" process");

    if !dd_exit_status.success() {
        panic!("\"dd\" invocation failed with exit status: {dd_exit_status}");
    }

    let mut mkfs_command = Command::new(mkfs_path);
    mkfs_command.arg(rootfs_path.to_string_lossy().to_string());
    let mkfs_exit_status = mkfs_command.status().await.expect("Failed to fork \"mkfs\" process");

    if !mkfs_exit_status.success() {
        panic!("\"mkfs\" invocation failed with exit status: {mkfs_exit_status}");
    }

    log::info!("Ran \"dd\" and \"mkfs\" to initialize an empty filesystem blob at {rootfs_path:?}");

    tokio::fs::create_dir(&rootfs_mount_path)
        .await
        .expect("Could not create filesystem mount point directory");
    let mut mount_command = Command::new(which::which("mount").expect("Could not locate \"mount\" in PATH"));
    mount_command.arg(&rootfs_path);
    mount_command.arg(&rootfs_mount_path);
    let mount_exit_status = mount_command.status().await.expect("Failed to fork \"mount\" process");

    if !mount_exit_status.success() {
        panic!("\"mount\" invocation failed with exit status: {mount_exit_status}");
    }

    log::info!("Mounted filesystem at {rootfs_path:?} to {rootfs_mount_path:?}");
}

fn get_tmp_path() -> PathBuf {
    PathBuf::from(format!("/tmp/{}", Uuid::new_v4()))
}
