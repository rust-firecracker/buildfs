use std::{collections::HashMap, path::PathBuf};

use crate::{
    container_engine::ExecParams,
    dry_run::{prepare_for_run, AdjoinAbsolute},
    RunArgs,
};

pub async fn run_command(run_args: RunArgs) {
    let (build_script, container_engine, unpack_path) = prepare_for_run(run_args.dry_run_args).await;

    container_engine.pull_image(&build_script.container.image).await;
    log::info!("Pulled image: {}", build_script.container.image.full_name());

    let base_script_path = PathBuf::from("/__scripts");
    let extra_volumes = build_script
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

    let container_name = container_engine
        .start_container(build_script.container, extra_volumes)
        .await;
    log::info!("Created and started container with name {container_name}");

    for command in build_script.commands {
        let mut exec_params = ExecParams {
            container_name: &container_name,
            cmd: "".to_string(),
            uid: command.uid,
            gid: command.gid,
            working_dir: command.working_dir,
            privileged: command.privileged,
            env: command.env,
            attach_stdout: command.attach_stdout,
            attach_stderr: command.attach_stderr,
            attach_stdin: command.attach_stdin,
        };

        if let Some(command_text) = command.command {
            log::info!("Exec-ing simple command inside container: \"{command_text}\"");
            exec_params.cmd = command_text;
        }

        let mut exec_reader = container_engine.exec_in_container(exec_params).await;
        while let Some(output) = exec_reader.read().await {
            print!("{output}");
        }

        log::info!("Exec complete, output (both stdout and stderr) printed above");
    }
}
