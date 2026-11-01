//! The command line of `lakehousectl`.
//!
//! The commands are `clap`'s derive API: the help text an operator
//! reads is written next to the argument it describes, so the two
//! cannot drift apart (REL-2 S3-T1).

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

/// The operator's tool for the `RantAI` Lakehouse.
#[derive(Debug, Parser)]
#[command(
    name = "lakehousectl",
    version,
    about = "Install and update the `RantAI` Lakehouse, online or air-gapped.",
    long_about = None
)]
pub struct Cli {
    /// The command to run.
    #[command(subcommand)]
    pub command: Command,
}

/// The commands `lakehousectl` has.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Check a release bundle against the release key.
    Verify(VerifyArgs),
    /// Check this host against the release's minimums.
    Preflight(PreflightArgs),
    /// Install the whole stack, online or from an offline bundle.
    Install(InstallArgs),
}

/// The arguments of `lakehousectl verify`.
#[derive(Debug, Args)]
pub struct VerifyArgs {
    /// The directory holding the release: `release.json` and
    /// `release.json.sigstore.json` beside each other.
    #[arg(long, default_value = ".")]
    pub dir: PathBuf,
}

/// The arguments of `lakehousectl preflight`.
#[derive(Debug, Args)]
pub struct PreflightArgs {
    /// The release's Compose file: the ports it publishes are read
    /// from it.
    #[arg(long)]
    pub compose: PathBuf,
    /// Where the install would live: the free disk is read under it.
    #[arg(long, default_value = "/opt/rantai-lakehouse")]
    pub dir: PathBuf,
    /// Skip one check, by name. The check is not run and the skip is
    /// said out loud.
    #[arg(long = "ignore-preflight", value_name = "CHECK")]
    pub ignore_preflight: Vec<String>,
}

/// The arguments of `lakehousectl install`.
#[derive(Debug, Args)]
pub struct InstallArgs {
    /// The offline bundle's directory. Without it, the release is
    /// fetched from GitHub.
    #[arg(long)]
    pub bundle: Option<PathBuf>,
    /// Where the install lives: everything is in this one directory.
    #[arg(long, default_value = "/opt/rantai-lakehouse")]
    pub dir: PathBuf,
    /// The release to install. Without it, the newest one.
    #[arg(long)]
    pub version: Option<String>,
    /// Skip one pre-flight check, by name. The check is not run and the
    /// skip is said out loud.
    #[arg(long = "ignore-preflight", value_name = "CHECK")]
    pub ignore_preflight: Vec<String>,
}
