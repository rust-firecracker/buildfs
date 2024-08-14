use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};
use serde::{Deserialize, Serialize};

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
    #[command(about = "Pack or unpack a package that can be executed by buildfs")]
    Package {
        #[command(flatten)]
        args: PackageArgs,
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
pub struct PackageArgs {
    #[arg(
        long = "source",
        short = 's',
        help = "The path of the source config to pack",
        default_value = "config.toml"
    )]
    source_path: PathBuf,
    #[arg(
        long = "dest",
        short = 'd',
        help = "The path to the destination package/unpack directory"
    )]
    destination_path: PathBuf,
    #[arg(long = "type", short = 't', help = "The package's type", value_enum, default_value_t)]
    package_type: PackageType,
    #[command(flatten)]
    pack_or_unpack: PackOrUnpackGroup,
}

#[derive(Args, Clone, Debug)]
pub struct DryRunArgs {
    package: PathBuf,
    #[arg(long = "type", short = 't', help = "The package's type", value_enum, default_value_t)]
    package_type: PackageType,
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
    #[default]
    Tarball,
    Tar,
    Zip,
}

#[derive(Args, Debug, Clone)]
#[group(required = true, multiple = false)]
pub struct PackOrUnpackGroup {
    #[arg(
        long = "unpack",
        short = 'U',
        help = "Unpack the source package into the destination directory"
    )]
    unpack: bool,
    #[arg(
        long = "pack",
        short = 'P',
        help = "Pack the source config into the destination package"
    )]
    pack: bool,
}

fn main() {
    let cli = Cli::parse();
    dbg!(cli);
}
