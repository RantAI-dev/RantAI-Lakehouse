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
}

/// The arguments of `lakehousectl verify`.
#[derive(Debug, Args)]
pub struct VerifyArgs {
    /// The directory holding the release: `release.json` and
    /// `release.json.sigstore.json` beside each other.
    #[arg(long, default_value = ".")]
    pub dir: PathBuf,
}
