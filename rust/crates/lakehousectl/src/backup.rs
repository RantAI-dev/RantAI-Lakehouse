//! `lakehousectl backup` and `lakehousectl restore` (REL-2 S4-T2).
//!
//! # Where the steps live
//!
//! In the two scripts the release ships, under `tools/`
//! (`backup-local.sh` and `restore-from-backup.sh`). There is no Rust
//! port of them: the steps have ONE source, and the operator who reads
//! the script reads what actually runs (REL-2 S4 decision D15).
//!
//! This module runs the script, with the environment the deployment's
//! own directory needs:
//!
//! - `LAKEHOUSE_DIR` — the install's directory, the one holding
//!   `docker-compose.yml` and `.env`;
//! - `COMPOSE_PROJECT` — `rantai-lakehouse` (REL-2 decision D5), so the
//!   script works on the install's own project and not on a directory
//!   name.
//!
//! # What the backup records
//!
//! `lakehousectl backup` writes the backup it took into the install's
//! state (`state.json`'s `lastBackup`). The rollback of an update reads
//! that pointer (REL-2 S4 decision D18), so the state and the backup
//! have to agree.

use std::path::{Path, PathBuf};

use crate::cli::{BackupArgs, RestoreArgs};
use crate::docker;
use crate::error::ToolError;
use crate::state;
use crate::sys;

/// The line the backup script prints when it is done.
///
/// The script's own summary names the backup it made; the tool reads
/// that line, so the tool and the script cannot disagree about which
/// backup was taken.
const BACKUP_DONE: &str = "==> Backup complete: ";

/// Run `lakehousectl backup`.
///
/// # Errors
///
/// Returns the error that stopped the backup: a script the release does
/// not ship, a script that failed, or a state that could not be
/// written.
pub fn command(args: &BackupArgs) -> Result<(), ToolError> {
    let dir = &args.dir;
    let script = backup_script(dir);
    require_script(&script)?;
    let root = dir.join("backups");
    let output = run(&script, &[root.to_string_lossy().to_string()], dir)?;

    let backup = backup_dir_of(&output)?;
    state::set_last_backup(dir, &backup)?;
    println!("lakehousectl: the backup is recorded in the install's state.");
    Ok(())
}

/// Run `lakehousectl restore`.
///
/// # Errors
///
/// Returns the error that stopped the restore: a script the release
/// does not ship, or a script that failed.
pub fn command_restore(args: &RestoreArgs) -> Result<(), ToolError> {
    let dir = &args.dir;
    let script = restore_script(dir);
    require_script(&script)?;
    let backup = args.backup.to_string_lossy().to_string();
    run(&script, &["--local".to_string(), backup], dir)?;
    Ok(())
}

/// The script that takes a backup, in the install's own directory.
fn backup_script(dir: &Path) -> PathBuf {
    dir.join("tools").join("backup-local.sh")
}

/// The script that restores a backup, in the install's own directory.
fn restore_script(dir: &Path) -> PathBuf {
    dir.join("tools").join("restore-from-backup.sh")
}

/// Refuse a command whose script the release does not ship.
fn require_script(script: &Path) -> Result<(), ToolError> {
    if script.is_file() {
        return Ok(());
    }
    Err(ToolError::Failed(format!(
        "{} does not exist, so the release that is installed does not ship the tool that does \
         this work. A release built before REL-2 S4 does not; install a release that does, or \
         run the script from the repository yourself.",
        script.display()
    )))
}

/// Run one of the shipped scripts, with the environment the deployment
/// needs.
///
/// The script's own output goes to the operator: it says what it does,
/// step by step, and a backup that is silent is a backup nobody can
/// trust.
fn run(script: &Path, args: &[String], dir: &Path) -> Result<String, ToolError> {
    let borrowed: Vec<&str> = args.iter().map(String::as_str).collect();
    let output = sys::output_or_stderr_with_env(
        &script.to_string_lossy(),
        &borrowed,
        &[
            ("LAKEHOUSE_DIR", &dir.to_string_lossy()),
            ("COMPOSE_PROJECT", docker::PROJECT),
        ],
    )
    .map_err(|error| ToolError::Failed(format!("the backup tool failed: {error}")))?;
    println!("{output}");
    Ok(output)
}

/// The backup directory the script's summary names.
fn backup_dir_of(output: &str) -> Result<PathBuf, ToolError> {
    let mut found: Option<&str> = None;
    for line in output.lines() {
        if let Some(rest) = line.strip_prefix(BACKUP_DONE) {
            found = Some(rest.trim());
        }
    }
    let Some(directory) = found else {
        return Err(ToolError::Failed(
            "the backup tool finished, but it did not say which backup it made, so the install's \
             state cannot name it. The backup itself is there; look in the backups directory."
                .to_string(),
        ));
    };
    Ok(PathBuf::from(directory))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::os::unix::fs::PermissionsExt;

    use super::*;

    /// A fake tool, in the install's own directory, that records what
    /// it was asked to do.
    ///
    /// The record is written next to the install's own directory, so the
    /// test can read it without an environment variable the tool would
    /// have to pass.
    fn fake_tool(dir: &Path, name: &str, body: &str) {
        let tools = dir.join("tools");
        std::fs::create_dir_all(&tools).expect("the tools directory is created");
        let path = tools.join(name);
        std::fs::write(&path, body).expect("the tool is written");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
            .expect("the tool is executable");
    }

    /// The record the fake tool wrote.
    fn record(dir: &Path) -> String {
        std::fs::read_to_string(dir.join("record.txt")).expect("the record is readable")
    }

    const FAKE_BACKUP: &str = r#"#!/bin/sh
record="$(dirname "$0")/../record.txt"
{
  echo "LAKEHOUSE_DIR=$LAKEHOUSE_DIR"
  echo "COMPOSE_PROJECT=$COMPOSE_PROJECT"
  echo "ARGS=$*"
} >> "$record"
echo "==> Backup complete: $LAKEHOUSE_DIR/backups/20261101T000000Z-0.2.0"
"#;

    const FAKE_RESTORE: &str = r#"#!/bin/sh
record="$(dirname "$0")/../record.txt"
{
  echo "LAKEHOUSE_DIR=$LAKEHOUSE_DIR"
  echo "COMPOSE_PROJECT=$COMPOSE_PROJECT"
  echo "ARGS=$*"
} >> "$record"
echo "the restore is done"
"#;

    #[test]
    fn the_backup_runs_the_shipped_script_with_the_deployments_own_directory() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        fake_tool(directory.path(), "backup-local.sh", FAKE_BACKUP);
        state::write_initial(directory.path(), "0.2.0").expect("the state is written");

        let args = BackupArgs {
            dir: directory.path().to_path_buf(),
        };
        command(&args).expect("the backup runs");

        let written = record(directory.path());
        assert!(
            written.contains(&format!(
                "LAKEHOUSE_DIR={}",
                directory.path().to_string_lossy()
            )),
            "the tool was not told the deployment's directory: {written}"
        );
        assert!(
            written.contains("COMPOSE_PROJECT=rantai-lakehouse"),
            "the tool was not told the project: {written}"
        );
        assert!(
            written.contains(&format!(
                "ARGS={}",
                directory.path().join("backups").to_string_lossy()
            )),
            "the tool was not given the backup root: {written}"
        );
    }

    #[test]
    fn the_backup_records_where_it_is_in_the_installs_state() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        fake_tool(directory.path(), "backup-local.sh", FAKE_BACKUP);
        state::write_initial(directory.path(), "0.2.0").expect("the state is written");

        let args = BackupArgs {
            dir: directory.path().to_path_buf(),
        };
        command(&args).expect("the backup runs");

        let state = state::read(directory.path()).expect("the state is read");
        assert_eq!(
            state.last_backup,
            Some(
                directory
                    .path()
                    .join("backups/20261101T000000Z-0.2.0")
                    .to_string_lossy()
                    .to_string()
            ),
            "the backup was not recorded in the state"
        );
    }

    #[test]
    fn the_restore_runs_the_shipped_script_with_the_backup_directory() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        fake_tool(directory.path(), "restore-from-backup.sh", FAKE_RESTORE);

        let args = RestoreArgs {
            backup: PathBuf::from("/backups/one"),
            dir: directory.path().to_path_buf(),
        };
        command_restore(&args).expect("the restore runs");

        let written = record(directory.path());
        assert!(
            written.contains(&format!(
                "LAKEHOUSE_DIR={}",
                directory.path().to_string_lossy()
            )),
            "the tool was not told the deployment's directory: {written}"
        );
        assert!(
            written.contains("COMPOSE_PROJECT=rantai-lakehouse"),
            "the tool was not told the project: {written}"
        );
        assert!(
            written.contains("ARGS=--local /backups/one"),
            "the tool was not given the backup directory: {written}"
        );
    }

    #[test]
    fn a_release_that_ships_no_backup_tool_is_refused() {
        let directory = tempfile::tempdir().expect("a temporary directory");

        let error = command(&BackupArgs {
            dir: directory.path().to_path_buf(),
        })
        .expect_err("a release with no tool is refused");
        assert!(
            error.to_string().contains("backup-local.sh"),
            "the error names the tool that is missing: {error}"
        );
    }

    #[test]
    fn a_tool_that_fails_says_what_it_said() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        fake_tool(
            directory.path(),
            "backup-local.sh",
            "#!/bin/sh\necho 'the volume is not there' >&2\nexit 1\n",
        );

        let error = command(&BackupArgs {
            dir: directory.path().to_path_buf(),
        })
        .expect_err("a tool that fails is an error");
        let message = error.to_string();
        assert!(
            message.contains("the volume is not there"),
            "the error carries what the tool said: {message}"
        );
    }

    #[test]
    fn a_tool_that_does_not_say_which_backup_it_made_is_refused() {
        // The state and the backup have to agree: a `lastBackup` that
        // names nothing is worse than none.
        let directory = tempfile::tempdir().expect("a temporary directory");
        fake_tool(
            directory.path(),
            "backup-local.sh",
            "#!/bin/sh\necho 'done'\n",
        );
        state::write_initial(directory.path(), "0.2.0").expect("the state is written");

        let error = command(&BackupArgs {
            dir: directory.path().to_path_buf(),
        })
        .expect_err("a tool that names no backup is refused");
        assert!(
            error.to_string().contains("backups directory"),
            "the error says where the backup is: {error}"
        );
    }
}
