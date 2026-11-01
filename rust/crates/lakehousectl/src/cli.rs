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
    /// Take a backup of the stack, on the local disk.
    Backup(BackupArgs),
    /// Restore a backup of the stack, on the local disk.
    Restore(RestoreArgs),
    /// Move the stack to another release.
    Update(UpdateArgs),
    /// Move the stack back to the release it came from.
    Rollback(RollbackArgs),
    /// Show what the install is doing right now.
    Status(StatusArgs),
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
    /// The install will be offline, from a bundle. The image-store
    /// check runs then: an air-gapped install needs Docker's containerd
    /// image store, and an online one does not (REL-2 S3 D13).
    #[arg(long)]
    pub bundle: bool,
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

/// The arguments of `lakehousectl backup`.
#[derive(Debug, Args)]
pub struct BackupArgs {
    /// Where the install lives: everything is in this one directory.
    #[arg(long, default_value = "/opt/rantai-lakehouse")]
    pub dir: PathBuf,
}

/// The arguments of `lakehousectl restore`.
#[derive(Debug, Args)]
pub struct RestoreArgs {
    /// The backup directory to restore: the one `lakehousectl backup`
    /// wrote.
    pub backup: PathBuf,
    /// Where the install lives: everything is in this one directory.
    #[arg(long, default_value = "/opt/rantai-lakehouse")]
    pub dir: PathBuf,
}

/// The arguments of `lakehousectl update`.
#[derive(Debug, Args)]
pub struct UpdateArgs {
    /// The offline bundle's directory. Without it, the release is
    /// fetched from GitHub.
    #[arg(long)]
    pub bundle: Option<PathBuf>,
    /// Where the install lives: everything is in this one directory.
    #[arg(long, default_value = "/opt/rantai-lakehouse")]
    pub dir: PathBuf,
    /// The release to update to. Without it, the newest one.
    #[arg(long)]
    pub version: Option<String>,
    /// How long the stack may take to become ready, in seconds.
    ///
    /// The update rolls back when the stack is not ready in time
    /// (REL-2 S4 decision D9).
    #[arg(long, default_value_t = 600)]
    pub ready_timeout: u64,
}

/// The arguments of `lakehousectl rollback`.
#[derive(Debug, Args)]
pub struct RollbackArgs {
    /// Where the install lives: everything is in this one directory.
    #[arg(long, default_value = "/opt/rantai-lakehouse")]
    pub dir: PathBuf,
    /// Do not ask. The data written since the update is lost.
    #[arg(long)]
    pub yes: bool,
}

/// The arguments of `lakehousectl status`.
#[derive(Debug, Args)]
pub struct StatusArgs {
    /// Where the install lives: everything is in this one directory.
    #[arg(long, default_value = "/opt/rantai-lakehouse")]
    pub dir: PathBuf,
}
