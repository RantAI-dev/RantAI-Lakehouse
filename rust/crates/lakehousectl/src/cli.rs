//! The command line of `lakehousectl`.
//!
//! The commands are `clap`'s derive API: the help text an operator
//! reads is written next to the argument it describes, so the two
//! cannot drift apart (REL-2 S3-T1).

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

use crate::error::ToolError;

/// The operator's tool for the `RantAI` Lakehouse.
#[derive(Debug, Parser)]
#[command(
    name = "lakehousectl",
    version,
    about = "Install and update the `RantAI` Lakehouse, online or air-gapped.",
    long_about = "Install and update the `RantAI` Lakehouse, online or air-gapped.\n\n\
                  An air-gapped host updates the TOOL with the copy in its bundle: the bundle \
                  carries `lakehousectl-linux-x86_64`, and that is the new tool. \
                  `lakehousectl self-update` needs the network."
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
    /// Update the tool itself.
    SelfUpdate,
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

/// The shape of an installation (`BI-44`).
///
/// `full` is the unchanged default: the release's own Compose file and all
/// its services. `bi-only` installs the console, the API, one engine and
/// the application database, reading the operator's tables in place.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum InstallModeArg {
    /// The whole stack, exactly as earlier releases installed it.
    Full,
    /// One engine, no warehouse services, no data copied in.
    BiOnly,
}

impl InstallModeArg {
    /// The spelling the API and the `.env` read.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::BiOnly => "bi-only",
        }
    }
}

/// The single engine a `bi-only` installation runs (`BI-44`).
///
/// Both engines running side by side defeats the memory reason for the
/// mode, so a `--engine` only names the one that is deployed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum QueryEngineArg {
    /// The default engine of the full stack.
    #[value(name = "clickhouse")]
    ClickHouse,
    /// The federated engine, with operator-supplied catalog properties.
    Trino,
}

impl QueryEngineArg {
    /// The spelling the API and the `.env` read.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ClickHouse => "clickhouse",
            Self::Trino => "trino",
        }
    }
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
    /// Which shape to install: `full` (the default) or `bi-only`.
    #[arg(long, value_enum, default_value = "full")]
    pub mode: InstallModeArg,
    /// The single engine of a `bi-only` install: `clickhouse` (the default)
    /// or `trino`. A `full` install keeps its own engine profiles.
    #[arg(long, value_enum, default_value = "clickhouse")]
    pub engine: QueryEngineArg,
}

impl InstallArgs {
    /// Check that the mode and engine describe one install, never two.
    ///
    /// A `full` install has its own `ClickHouse` profile and may add `Trino`
    /// through `COMPOSE_PROFILES`; naming `--engine trino` there would
    /// claim a single-engine install the tool is not making. Fail closed
    /// with the reason instead of writing a `.env` that lies.
    ///
    /// # Errors
    ///
    /// Returns a usage error when `full` is combined with `trino`.
    pub fn validated_install(&self) -> Result<(InstallModeArg, QueryEngineArg), ToolError> {
        if self.mode == InstallModeArg::Full && self.engine == QueryEngineArg::Trino {
            return Err(ToolError::Usage(
                "--engine trino chooses the single engine of a --mode bi-only installation; \
                 a --mode full install keeps its own engine profiles"
                    .to_string(),
            ));
        }
        Ok((self.mode, self.engine))
    }
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
    /// Skip one pre-flight check, by name. The check is not run and
    /// the skip is said out loud.
    #[arg(long = "ignore-preflight", value_name = "CHECK")]
    pub ignore_preflight: Vec<String>,
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn install_args(argv: &[&str]) -> InstallArgs {
        let cli = Cli::try_parse_from(argv).expect("the install command parses");
        let Command::Install(parsed) = cli.command else {
            panic!("the command was not install")
        };
        parsed
    }

    #[test]
    fn an_install_without_flags_keeps_the_full_clickhouse_defaults() {
        let args = install_args(&["lakehousectl", "install"]);
        assert_eq!(args.mode, InstallModeArg::Full);
        assert_eq!(args.engine, QueryEngineArg::ClickHouse);
        assert_eq!(args.mode.as_str(), "full");
        assert_eq!(args.engine.as_str(), "clickhouse");
        assert_eq!(
            args.validated_install().expect("the defaults agree"),
            (InstallModeArg::Full, QueryEngineArg::ClickHouse)
        );
    }

    #[test]
    fn the_documented_mode_and_engine_spellings_parse() {
        let args = install_args(&[
            "lakehousectl",
            "install",
            "--mode",
            "bi-only",
            "--engine",
            "trino",
        ]);
        assert_eq!(args.mode, InstallModeArg::BiOnly);
        assert_eq!(args.engine, QueryEngineArg::Trino);
        assert_eq!(args.mode.as_str(), "bi-only");
        assert_eq!(args.engine.as_str(), "trino");
        assert_eq!(
            args.validated_install().expect("bi-only and trino agree"),
            (InstallModeArg::BiOnly, QueryEngineArg::Trino)
        );

        let args = install_args(&[
            "lakehousectl",
            "install",
            "--mode",
            "bi-only",
            "--engine",
            "clickhouse",
        ]);
        assert_eq!(args.engine, QueryEngineArg::ClickHouse);
        assert!(args.validated_install().is_ok());
    }

    #[test]
    fn unknown_modes_and_engines_are_refused_by_the_parser() {
        assert!(Cli::try_parse_from(["lakehousectl", "install", "--mode", "other"]).is_err());
        assert!(Cli::try_parse_from(["lakehousectl", "install", "--engine", "other"]).is_err());
        assert!(Cli::try_parse_from(["lakehousectl", "install", "--mode", "BI-ONLY"]).is_err());
    }

    #[test]
    fn a_full_install_with_trino_is_refused_before_anything_is_written() {
        let args = install_args(&["lakehousectl", "install", "--engine", "trino"]);
        let error = args
            .validated_install()
            .expect_err("a full stack does not have one engine");
        let message = error.to_string();
        assert!(
            message.contains("--engine trino"),
            "the error names the flag: {message}"
        );
        assert!(
            message.contains("--mode bi-only"),
            "the error says where the flag belongs: {message}"
        );
    }
}
