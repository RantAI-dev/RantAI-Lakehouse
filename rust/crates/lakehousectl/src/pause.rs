//! `MAINTENANCE_PAUSED`: the flag that stops the jobs that delete files
//! (REL-2 S4 decision D16).
//!
//! # What the flag is for
//!
//! An update restores the catalog from a backup. The catalog names
//! data files in the object store, and a restore brings the catalog
//! back to the moment of the backup — while the files written after it
//! stay where they are, as orphans. That is safe only while nothing
//! deletes files: the snapshot expiry and the orphan-file removal
//! would delete a file the RESTORED catalog still names.
//!
//! So the update sets the flag in the install's `.env` BEFORE the
//! backup, and clears it after the update succeeds or after it is
//! rolled back. The Dagster services read the variable, and `up -d`
//! recreates them (D16).
//!
//! # Where it lives
//!
//! In the install's own `.env`, next to the other settings. The file
//! holds secrets, so it stays readable by its owner only.
//!
//! # How it is cleared
//!
//! Clearing the line in the `.env` is not enough on its own: the
//! services that read the flag read it when they are CREATED, so they
//! have to be recreated after it is cleared. [`clear_and_recreate`]
//! does both, and every path that clears the pause comes through it —
//! the update's success, the rollback and the update's way back (REL-2
//! S4b review round 2 BLOCKER 2).

use std::path::Path;

use crate::env;
use crate::error::ToolError;
use crate::runner::Runner;

/// The variable the Dagster services read.
pub const VARIABLE: &str = "MAINTENANCE_PAUSED";

/// The value that pauses the jobs.
///
/// The jobs read the variable with `== "true"` (D16), so anything else
/// is not a pause.
pub const PAUSED: &str = "true";

/// Set the pause in the install's `.env`.
///
/// # Errors
///
/// Returns the error that says what could not be written.
pub fn set(dir: &Path) -> Result<(), ToolError> {
    let path = dir.join(".env");
    let text = read(&path)?;
    write(&path, &with_variable(&text, PAUSED))
}

/// Clear the pause in the install's `.env`.
///
/// The line is REMOVED, not emptied: an emptied line would still be a
/// line the operator reads and wonders about, and the jobs read the
/// value with `== "true"`, so an absent line is not a pause.
///
/// # Errors
///
/// Returns the error that says what could not be written.
pub fn clear(dir: &Path) -> Result<(), ToolError> {
    let path = dir.join(".env");
    if !path.exists() {
        return Ok(());
    }
    let text = read(&path)?;
    write(&path, &without_variable(&text))
}

/// Clear the pause, and recreate the services that read it.
///
/// # Why the services are recreated
///
/// The Dagster services read `MAINTENANCE_PAUSED` when they are
/// CREATED (D16). Clearing the line in the `.env` changes nothing for
/// a container that already runs: it keeps the value it was created
/// with, so the snapshot expiry and the orphan-file removal stay
/// skipped, and the storage grows with nothing saying so.
///
/// So the services are recreated: `up -d --pull never` recreates only
/// the services whose environment changed, and it pulls nothing (REL-2
/// S3 D13). Every path that clears the pause — the update's success,
/// the rollback and the update's way back — comes through here, so
/// none of them can clear the pause and leave the services running
/// with it (REL-2 S4b review round 2 BLOCKER 2).
///
/// # Errors
///
/// Returns the error that says what could not be written, or that the
/// services could not be recreated.
pub fn clear_and_recreate(dir: &Path, runner: &mut dyn Runner) -> Result<(), ToolError> {
    clear(dir).map_err(|error| {
        ToolError::Failed(format!(
            "the pause could not be cleared: {error}. The jobs that delete files stay skipped \
             until the pause is cleared."
        ))
    })?;
    runner.up().map_err(|error| {
        ToolError::Failed(format!(
            "the services that read the pause could not be recreated: {error}. The jobs that \
             delete files stay skipped until they are recreated."
        ))
    })
}

/// The `.env` with the variable set to `value`.
///
/// A line that sets the variable already is REPLACED, so the file never
/// holds two of them and the last one is not the one that counts.
fn with_variable(text: &str, value: &str) -> String {
    env::with_variable(text, VARIABLE, value)
}

/// The `.env` with the variable's line removed.
fn without_variable(text: &str) -> String {
    let mut lines: Vec<String> = Vec::new();
    for line in text.lines() {
        if env::name_of(line).is_some_and(|name| name == VARIABLE) {
            continue;
        }
        lines.push(line.to_string());
    }
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

/// Read the `.env`.
fn read(path: &Path) -> Result<String, ToolError> {
    std::fs::read_to_string(path).map_err(|source| ToolError::Io {
        context: format!("{} could not be read", path.display()),
        source,
    })
}

/// Write the `.env`, and keep it readable by its owner only.
///
/// The mode is set on every write: the file holds the stack's secrets,
/// and a file that was copied around may have been made readable by
/// somebody else (REL-2 decision D6).
fn write(path: &Path, text: &str) -> Result<(), ToolError> {
    std::fs::write(path, text).map_err(|source| ToolError::Io {
        context: format!("{} could not be written", path.display()),
        source,
    })?;
    set_owner_only(path)
}

/// Give the file the mode 0600.
fn set_owner_only(path: &Path) -> Result<(), ToolError> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(|source| {
        ToolError::Io {
            context: format!("{} could not be given the mode 600", path.display()),
            source,
        }
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::os::unix::fs::PermissionsExt;

    use super::*;

    /// An install's own directory, with an `.env` in it.
    fn install_with_env(env: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let path = directory.path().join(".env");
        std::fs::write(&path, env).expect("the env file is written");
        (directory, path)
    }

    #[test]
    fn the_pause_is_set_in_the_installs_own_env_file() {
        let (directory, path) = install_with_env("POSTGRES_PASSWORD=secret\n");
        set(directory.path()).expect("the pause is set");

        let text = std::fs::read_to_string(&path).expect("the env file is readable");
        assert!(
            text.contains("MAINTENANCE_PAUSED=true"),
            "the pause is not in the env file: {text}"
        );
        assert!(
            text.contains("POSTGRES_PASSWORD=secret"),
            "the other settings were lost: {text}"
        );
    }

    #[test]
    fn the_pause_replaces_the_line_that_was_there() {
        let (directory, path) =
            install_with_env("MAINTENANCE_PAUSED=false\nPOSTGRES_PASSWORD=secret\n");
        set(directory.path()).expect("the pause is set");

        let text = std::fs::read_to_string(&path).expect("the env file is readable");
        assert_eq!(
            text.matches("MAINTENANCE_PAUSED").count(),
            1,
            "the env file holds the pause twice: {text}"
        );
        assert!(
            text.contains("MAINTENANCE_PAUSED=true"),
            "the pause is not set: {text}"
        );
    }

    #[test]
    fn the_pause_is_cleared_and_the_other_settings_are_kept() {
        let (directory, path) =
            install_with_env("POSTGRES_PASSWORD=secret\nMAINTENANCE_PAUSED=true\n");
        clear(directory.path()).expect("the pause is cleared");

        let text = std::fs::read_to_string(&path).expect("the env file is readable");
        assert!(
            !text.contains("MAINTENANCE_PAUSED"),
            "the pause is still in the env file: {text}"
        );
        assert!(
            text.contains("POSTGRES_PASSWORD=secret"),
            "the other settings were lost: {text}"
        );
    }

    #[test]
    fn a_commented_out_pause_is_not_a_pause() {
        let (directory, path) = install_with_env("# MAINTENANCE_PAUSED=true\n");
        clear(directory.path()).expect("the pause is cleared");

        let text = std::fs::read_to_string(&path).expect("the env file is readable");
        assert!(
            text.contains("# MAINTENANCE_PAUSED=true"),
            "a comment was removed: {text}"
        );
    }

    #[test]
    fn the_env_file_stays_readable_by_its_owner_only() {
        let (directory, path) = install_with_env("POSTGRES_PASSWORD=secret\n");
        set(directory.path()).expect("the pause is set");

        let mode = std::fs::metadata(&path)
            .expect("the env file is there")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "the env file's mode is {mode:o}");
    }

    #[test]
    fn a_pause_that_cannot_be_set_is_an_error() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let error = set(directory.path()).expect_err("an install with no env file is an error");
        assert!(
            error.to_string().contains(".env"),
            "the error says what failed: {error}"
        );
    }

    #[test]
    fn clearing_the_pause_of_an_install_with_no_env_file_is_not_an_error() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        clear(directory.path()).expect("there is nothing to clear");
    }
}
