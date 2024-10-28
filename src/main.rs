use std::{fmt::Display, path::PathBuf};

use clap::{Args, Parser, Subcommand, ValueEnum};
use dry_run::dry_run_command;
use package::{pack_command, unpack_command};
use run::run_command;
use serde::{Deserialize, Serialize};

pub mod container_engine;
pub mod dry_run;
pub mod package;
pub mod run;
pub mod schema;

#[derive(Parser, Debug, Clone)]
#[command(
    version = "0.3.1",
    about = "A tool for declarative creation of root filesystem images",
    propagate_version = true
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: CliCommand,
    #[arg(
        short = 'A',
        long = "async-threads",
        help = "The amount of asynchronous threads to give to Tokio",
        default_value_t = 1
    )]
    pub async_threads: usize,
    #[arg(
        short = 'B',
        long = "max-blocking-threads",
        help = "The limit to the amount of blocking threads for Tokio. Setting this limit may degrade file I/O performance!"
    )]
    pub max_blocking_threads: Option<usize>,
    #[arg(
        short = 'l',
        long = "log-level",
        help = "The level to set for logging",
        default_value = "info"
    )]
    pub log_level: LogLevel,
    #[arg(
        short = 'e',
        long = "no-exec-logs",
        help = "Disable logging of the output of scripts run inside the container, and pipe \"dd\" and \"mkfs\" output to /dev/null"
    )]
    pub no_exec_logs: bool,
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
    #[arg(long = "output", short = 'o', help = "The path to the produced root filesystem")]
    output_path: PathBuf,
}

#[derive(ValueEnum, Serialize, Deserialize, Clone, Copy, Default, Debug)]
#[serde(rename_all = "kebab-case")]
pub enum PackageType {
    TarGz,
    Tar,
    Directory,
    #[default]
    BuildScript,
}

#[derive(ValueEnum, Serialize, Deserialize, Clone, Copy, Default, Debug)]
pub enum LogLevel {
    Trace,
    Debug,
    #[default]
    Info,
    Warn,
    Error,
}

impl From<LogLevel> for log::Level {
    fn from(value: LogLevel) -> Self {
        match value {
            LogLevel::Trace => log::Level::Trace,
            LogLevel::Debug => log::Level::Debug,
            LogLevel::Info => log::Level::Info,
            LogLevel::Warn => log::Level::Warn,
            LogLevel::Error => log::Level::Error,
        }
    }
}

impl Display for PackageType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PackageType::TarGz => write!(f, "TarGz"),
            PackageType::Tar => write!(f, "Tar"),
            PackageType::Directory => write!(f, "Directory"),
            PackageType::BuildScript => write!(f, "BuildScript"),
        }
    }
}

fn main() {
    let cli = Cli::parse();

    simple_logger::init_with_level(cli.log_level.into()).expect("Could not initialize simple_logger");

    if std::env::consts::OS == "windows" {
        panic!("buildfs cannot run on Windows due to a lack of mkfs tools!");
    }

    if std::env::consts::OS == "macos" {
        log::warn!("Running buildfs on macOS is neither recommended nor supported. Proceed with heavy caution!!!");
    }

    let mut runtime_builder = tokio::runtime::Builder::new_multi_thread();
    runtime_builder.enable_all();
    runtime_builder.worker_threads(cli.async_threads);

    if let Some(max_blocking_threads) = cli.max_blocking_threads {
        runtime_builder.max_blocking_threads(max_blocking_threads);
    }

    runtime_builder
        .build()
        .expect("Could not start Tokio runtime")
        .block_on(async {
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
                    run_command(args, cli.no_exec_logs).await;
                }
            }
        });
}
