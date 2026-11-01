//! The install's state, `state.json` (REL-2 S4-T2).
//!
//! # What the state is
//!
//! One file, in the install's own directory, that says:
//!
//! - `currentVersion` — the release that is installed;
//! - `previousVersion` — the release that was installed before it;
//! - `lastBackup` — the backup that was taken last.
//!
//! The install writes it (REL-2 decision D5) and refuses to replace a
//! release with another one: `lakehousectl update` is the command that
//! does that (REL-2 S4 decision D17).
//!
//! This module owns the file's shape. The install, the backup and the
//! update all read and write it here, so they cannot disagree about
//! what it says (rule 4).

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::ToolError;

/// The version of the state's own shape.
pub const SCHEMA_VERSION: u64 = 1;

/// The state of an install.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct State {
    /// The version of the state's own shape.
    pub schema_version: u64,
    /// The release that is installed.
    pub current_version: Option<String>,
    /// The release that was installed before this one.
    pub previous_version: Option<String>,
    /// The backup that was taken last.
    pub last_backup: Option<String>,
}

impl State {
    /// The state of a first install: the release that is installed, and
    /// nothing before it.
    #[must_use]
    pub fn first_install(version: &str) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            current_version: Some(version.to_string()),
            previous_version: None,
            last_backup: None,
        }
    }
}

/// The path of the state file in an install directory.
#[must_use]
pub fn path(dir: &Path) -> std::path::PathBuf {
    dir.join("state.json")
}

/// Read the state of an install.
///
/// # Errors
///
/// Returns the error that says what could not be read: a state file
/// that is not there, or one that is not JSON.
pub fn read(dir: &Path) -> Result<State, ToolError> {
    let path = path(dir);
    let text = std::fs::read_to_string(&path).map_err(|source| ToolError::Io {
        context: format!("{} could not be read", path.display()),
        source,
    })?;
    serde_json::from_str(&text).map_err(|error| {
        ToolError::Failed(format!(
            "{} is not a state this tool understands: {error}",
            path.display()
        ))
    })
}

/// Write the state of an install.
///
/// # Errors
///
/// Returns the error that says what could not be written.
pub fn write(dir: &Path, state: &State) -> Result<(), ToolError> {
    let path = path(dir);
    let text = serde_json::to_string_pretty(state).map_err(|error| {
        ToolError::Failed(format!(
            "the state of the install could not be written: {error}"
        ))
    })?;
    std::fs::write(&path, format!("{text}\n")).map_err(|source| ToolError::Io {
        context: format!("{} could not be written", path.display()),
        source,
    })
}

/// Write the state of a FIRST install.
///
/// The state of an earlier install is never overwritten: the same
/// release may be installed again, and any other release is refused —
/// that is what `lakehousectl update` is for (REL-2 S3 review round 1
/// SHOULD-FIX 3).
///
/// # Errors
///
/// Returns the error that says what could not be written.
pub fn write_initial(dir: &Path, version: &str) -> Result<(), ToolError> {
    if path(dir).exists() {
        return Ok(());
    }
    write(dir, &State::first_install(version))
}

/// Refuse an install that would replace the release of another one.
///
/// The directory holds the state of the install that is there, and the
/// state names the release that is installed. The SAME release may be
/// installed again — a re-run of the same install — and its state is
/// kept as it is. Any OTHER release is refused, and the refusal says
/// which command does that work (REL-2 S3 review round 1 SHOULD-FIX 3).
///
/// # Errors
///
/// Returns the error that says which release is there, and which
/// command installs another one.
pub fn check_installed(dir: &Path, version: &str) -> Result<(), ToolError> {
    let path = path(dir);
    if !path.exists() {
        return Ok(());
    }
    let text = std::fs::read_to_string(&path).map_err(|source| ToolError::Io {
        context: format!("{} could not be read", path.display()),
        source,
    })?;
    let installed = serde_json::from_str::<serde_json::Value>(&text)
        .ok()
        .and_then(|document| document["currentVersion"].as_str().map(str::to_string));
    match installed {
        Some(installed) if installed == version => Ok(()),
        Some(installed) => Err(ToolError::Failed(format!(
            "{} already holds the release {installed}, and the release {version} would replace it. \
             `lakehousectl update` is the command that installs another release.",
            dir.display()
        ))),
        None => Err(ToolError::Failed(format!(
            "{} holds a state.json that names no release, so the tool cannot tell which one is \
             installed. It is refused rather than overwritten.",
            dir.display()
        ))),
    }
}

/// Record the backup that was taken last.
///
/// # Errors
///
/// Returns the error that says what could not be read or written: a
/// state file that is not there, or one that is not JSON.
pub fn set_last_backup(dir: &Path, backup: &Path) -> Result<(), ToolError> {
    let mut state = read(dir)?;
    state.last_backup = Some(backup.to_string_lossy().to_string());
    write(dir, &state)
}

/// The path of the state the install held BEFORE the last update.
///
/// The update keeps it beside the state, the way it keeps the old
/// Compose file and the old manifest as `*.previous` (REL-2 S4
/// decisions D17 and D18). The rollback puts it back, so the state
/// after a rollback is the state before the update — the release that
/// is installed, the one before it, and the backup that was taken
/// before THAT one.
#[must_use]
pub fn previous_path(dir: &Path) -> std::path::PathBuf {
    dir.join("state.json.previous")
}

/// Keep the state of the install before an update.
///
/// # Errors
///
/// Returns the error that says what could not be written.
pub fn keep_previous(dir: &Path, state: &State) -> Result<(), ToolError> {
    let text = serde_json::to_string_pretty(state).map_err(|error| {
        ToolError::Failed(format!(
            "the state of the install could not be written: {error}"
        ))
    })?;
    std::fs::write(previous_path(dir), format!("{text}\n")).map_err(|source| ToolError::Io {
        context: format!("{} could not be written", previous_path(dir).display()),
        source,
    })
}

/// The state the install held before the last update.
///
/// # Errors
///
/// Returns the error that says what could not be read.
pub fn read_previous(dir: &Path) -> Result<State, ToolError> {
    let path = previous_path(dir);
    let text = std::fs::read_to_string(&path).map_err(|source| ToolError::Io {
        context: format!("{} could not be read", path.display()),
        source,
    })?;
    serde_json::from_str(&text).map_err(|error| {
        ToolError::Failed(format!(
            "{} is not a state this tool understands: {error}",
            path.display()
        ))
    })
}

/// Set the release that is installed, and the one before it.
///
/// # Errors
///
/// Returns the error that says what could not be read or written.
pub fn set_versions(dir: &Path, current: &str, previous: Option<&str>) -> Result<(), ToolError> {
    let mut state = read(dir)?;
    state.current_version = Some(current.to_string());
    state.previous_version = previous.map(str::to_string);
    write(dir, &state)
}

/// Put the state of the install back to the one it held before the
/// update.
///
/// The state the update kept is the one to put back. An install that
/// has no such state — one that was updated by an older tool — gets
/// the release it returns to, and no release before it.
///
/// # Errors
///
/// Returns the error that says what could not be read or written.
pub fn restore_previous(dir: &Path, current: &str, backup: Option<&Path>) -> Result<(), ToolError> {
    if previous_path(dir).exists() {
        let state = read_previous(dir)?;
        return write(dir, &state);
    }
    let mut state = read(dir)?;
    state.current_version = Some(current.to_string());
    state.previous_version = None;
    if let Some(backup) = backup {
        state.last_backup = Some(backup.to_string_lossy().to_string());
    }
    write(dir, &state)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn the_state_of_a_first_install_names_the_release_and_nothing_else() {
        let state = State::first_install("0.2.0");
        assert_eq!(state.current_version.as_deref(), Some("0.2.0"));
        assert_eq!(state.previous_version, None);
        assert_eq!(state.last_backup, None);
    }

    #[test]
    fn the_state_is_written_and_read_back() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        write_initial(directory.path(), "0.2.0").expect("the state is written");

        let state = read(directory.path()).expect("the state is read");
        assert_eq!(state.current_version.as_deref(), Some("0.2.0"));
        assert_eq!(state.schema_version, SCHEMA_VERSION);
    }

    #[test]
    fn the_state_is_written_with_the_names_the_install_uses() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        write_initial(directory.path(), "0.2.0").expect("the state is written");

        let text = std::fs::read_to_string(path(directory.path())).expect("the state is readable");
        assert!(text.contains("\"currentVersion\""), "the state is {text}");
        assert!(text.contains("\"previousVersion\""), "the state is {text}");
        assert!(text.contains("\"lastBackup\""), "the state is {text}");
        assert!(text.contains("null"), "the state is {text}");
    }

    #[test]
    fn the_backup_that_was_taken_last_is_recorded() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        write_initial(directory.path(), "0.2.0").expect("the state is written");

        set_last_backup(directory.path(), Path::new("/backups/one"))
            .expect("the backup is recorded");

        let state = read(directory.path()).expect("the state is read");
        assert_eq!(state.last_backup.as_deref(), Some("/backups/one"));
        assert_eq!(
            state.current_version.as_deref(),
            Some("0.2.0"),
            "the release that is installed was changed"
        );
    }

    #[test]
    fn a_second_install_of_the_same_release_keeps_the_state_of_the_first() {
        // The state names the release before this one and the last
        // backup: a re-install of the same release changes nothing of
        // it (REL-2 S3 review round 1 SHOULD-FIX 3).
        let directory = tempfile::tempdir().expect("a temporary directory");
        let state = "{\n  \"schemaVersion\": 1,\n  \"currentVersion\": \"0.2.0\",\n  \
                     \"previousVersion\": \"0.1.0\",\n  \"lastBackup\": \"/backups/one\"\n}\n";
        std::fs::write(path(directory.path()), state).expect("the state is written");

        write_initial(directory.path(), "0.2.0").expect("the same release installs again");

        let after = std::fs::read_to_string(path(directory.path())).expect("the state is readable");
        assert_eq!(after, state, "the state of the first install was rewritten");
    }

    #[test]
    fn an_install_of_another_release_is_refused_and_points_at_the_update() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        std::fs::write(
            path(directory.path()),
            "{\"schemaVersion\": 1, \"currentVersion\": \"0.1.0\"}\n",
        )
        .expect("the state is written");

        let error =
            check_installed(directory.path(), "0.2.0").expect_err("another release is refused");
        let message = error.to_string();
        assert!(
            message.contains("lakehousectl update"),
            "the error says what to do: {message}"
        );
        assert!(
            message.contains("0.1.0"),
            "the error names the release that is there: {message}"
        );
    }

    #[test]
    fn a_state_that_names_no_release_is_refused_rather_than_overwritten() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        std::fs::write(path(directory.path()), "not json\n").expect("the state is written");

        let error = check_installed(directory.path(), "0.2.0")
            .expect_err("a state that names no release is refused");
        assert!(
            error.to_string().contains("state.json"),
            "the error says what failed: {error}"
        );
        assert_eq!(
            std::fs::read_to_string(path(directory.path())).expect("the state is read"),
            "not json\n",
            "the state was overwritten"
        );
    }
}
