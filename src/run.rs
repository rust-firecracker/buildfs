use std::{collections::HashMap, fs::Permissions, os::unix::fs::PermissionsExt, path::PathBuf};

use tokio::task::JoinSet;
use uuid::Uuid;

use crate::{
    container_engine::ExecParams,
    dry_run::{prepare_for_run, AdjoinAbsolute},
    RunArgs,
};

pub async fn run_command(run_args: RunArgs) {
    let (build_script, container_engine, unpack_path, can_delete_unpack_path) =
        prepare_for_run(run_args.dry_run_args).await;

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
        if let Some(ref script) = command.script {
            let host_path = PathBuf::from(format!("/tmp/{}", Uuid::new_v4()));
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

    log::debug!("Resolved script volumes for container to: {volumes:?}");

    let (container_id, container_name) = container_engine.start_container(build_script.container, volumes).await;
    log::info!("Created and started container with name {container_name} and ID {container_id}");

    for command in build_script.commands {
        let mut exec_params = ExecParams {
            container_name: &container_name,
            container_id: &container_id,
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

        if let Some(script) = command.script {
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
