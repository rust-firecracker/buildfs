use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};
use dry_run::dry_run_command;
use package::{pack_command, unpack_command};
use run::run_command;
use serde::{Deserialize, Serialize};

mod container_engine;
mod dry_run;
mod package;
mod run;
mod schema;

#[derive(Parser, Debug, Clone)]
#[command(
    version = "0.1",
    about = "A tool for declarative creation of root filesystem images",
    propagate_version = true
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: CliCommand,
}

#[derive(Subcommand, Debug, Clone)]
pub enum CliCommand {
    #[command(about = "Pack a build script with its dependencies into an executable package")]
    Pack {
        #[command(flatten)]
        args: PackArgs,
    },
    #[command(about = "Unpack an executable package's build script and dependencies")]
    Unpack {
        #[command(flatten)]
        args: UnpackArgs,
    },
    #[command(about = "Dry-run an executable package to determine whether it is correctly configured")]
    DryRun {
        #[command(flatten)]
        args: DryRunArgs,
    },
    #[command(about = "Run an executable package to produce a root filesystem")]
    Run {
        #[command(flatten)]
        args: RunArgs,
    },
}

#[derive(Args, Clone, Debug)]
pub struct UnpackArgs {
    #[arg(help = "The path of the package to unpack")]
    source_path: PathBuf,
    #[arg(help = "The path to the location of the unpacked content(s)")]
    destination_path: PathBuf,
}

#[derive(Args, Clone, Debug)]
pub struct PackArgs {
    source_path: PathBuf,
    destination_path: PathBuf,
    #[arg(long = "type", short = 't', help = "The type of package to produce")]
    package_type: PackageType,
}

#[derive(Args, Clone, Debug)]
pub struct DryRunArgs {
    package: PathBuf,
}

#[derive(Args, Clone, Debug)]
pub struct RunArgs {
    #[command(flatten)]
    dry_run_args: DryRunArgs,
    #[arg(
        long = "output",
        short = 'o',
        help = "The overridden path to the produced root filesystem"
    )]
    output_path: Option<PathBuf>,
}

#[derive(ValueEnum, Serialize, Deserialize, Clone, Copy, Default, Debug)]
#[serde(rename_all = "kebab-case")]
pub enum PackageType {
    Tarball,
    Tar,
    Directory,
    #[default]
    BuildScript,
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let cli = Cli::parse();

    match cli.command {
        CliCommand::Pack { args } => {
            pack_command(args).await;
        }
        CliCommand::Unpack { args } => {
            unpack_command(args).await;
        }
        CliCommand::DryRun { args } => {
            dry_run_command(args).await;
        }
        CliCommand::Run { args } => {
            run_command(args).await;
        }
    }
}
