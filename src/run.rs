use std::{collections::HashMap, path::PathBuf};

use crate::{
    dry_run::{prepare_for_run, AdjoinAbsolute},
    RunArgs,
};

pub async fn run_command(run_args: RunArgs) {
    let (build_script, container_engine, unpack_path) = prepare_for_run(run_args.dry_run_args).await;

    container_engine.pull_image(&build_script.container.image).await;
    println!("Pulled container image");

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

    container_engine
        .start_container(build_script.container, extra_volumes)
        .await;
    println!("Created and started container");
}
