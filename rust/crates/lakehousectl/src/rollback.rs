//! `lakehousectl rollback`: the stack goes back to the release it came
//! from (REL-2 S4 decision D18).
//!
//! # What a rollback is
//!
//! A RESTORE. Migrations stay forward-only, so going back means
//! bringing the data back from the backup the update took, and putting
//! the previous release's files back. The steps are the ones D18 names,
//! in the order it names them:
//!
//! 1. the stack stops;
//! 2. the backup the update took is restored;
//! 3. the `*.previous` files go back, and the stack starts;
//! 4. the stack is waited for, on the OLD version;
//! 5. the pause is cleared, the services that read it are recreated,
//!    and the state goes back.
//!
//! # The two refusals
//!
//! A rollback refuses when there is no previous release, or when there
//! is no backup: there would be nothing to go back to, or a release
//! without the data it was running with.
//!
//! # By hand
//!
//! A rollback by hand says FIRST that the data written since the
//! update is lost, and it needs `--yes` or a typed "yes" (D18). The
//! update's OWN rollback says no such thing: it says what failed and
//! what the rollback did.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::cli::RollbackArgs;
use crate::error::ToolError;
use crate::pause;
use crate::runner::{self, Local, Runner};
use crate::state;

/// How long the stack may take to become ready after a rollback.
const READY_TIMEOUT: Duration = Duration::from_secs(600);

/// Run `lakehousectl rollback`.
///
/// # Errors
///
/// Returns the error that stopped the rollback: a refusal (no previous
/// release, or no backup), a rollback that was not confirmed, or a step
/// that failed.
pub fn command(args: &RollbackArgs) -> Result<(), ToolError> {
    let dir = &args.dir;
    let mut state = state::read(dir)?;

    let version = state.previous_version.take();
    let backup = state.last_backup.take();
    let Some(version) = version else {
        return Err(ToolError::Failed(
            "the install's state names no previous release, so there is nothing to roll back to. \
             A rollback returns to the release the last update replaced."
                .to_string(),
        ));
    };
    let Some(backup) = backup else {
        return Err(ToolError::Failed(
            "the install's state names no backup, so there is nothing to restore. A rollback \
             restores the backup the last update took, and no update took one."
                .to_string(),
        ));
    };

    // The warning comes FIRST, before anything is touched (D18): the
    // data written since the update is lost by the restore, and the
    // operator has to know that before they say yes.
    println!("lakehousectl: rolling back to {version}.");
    println!(
        "lakehousectl: the data written since the update is LOST: the restore brings the backup \
         at {backup} back, and that backup was taken before the update."
    );
    if !args.yes && !confirmed()? {
        return Err(ToolError::Failed(
            "the rollback was not confirmed, so nothing was changed.".to_string(),
        ));
    }

    let mut runner = Local::new(dir);
    run(dir, &mut runner, &version, READY_TIMEOUT)?;
    println!("lakehousectl: the stack runs {version} again.");
    Ok(())
}

/// Run the rollback.
///
/// The steps are the ones D18 names, in the order it names them.
///
/// # Errors
///
/// Returns the error that stopped the rollback.
pub fn run(
    dir: &Path,
    runner: &mut dyn Runner,
    version: &str,
    timeout: Duration,
) -> Result<(), ToolError> {
    // The stack stops first: a database that is still running while its
    // data is written back is not a restored database.
    runner.down().map_err(ToolError::Failed)?;

    // The backup the update took.
    let state = state::read(dir)?;
    let Some(backup) = state.last_backup.as_ref().map(PathBuf::from) else {
        return Err(ToolError::Failed(
            "the install's state names no backup, so there is nothing to restore. A rollback \
             restores the backup the last update took, and no update took one."
                .to_string(),
        ));
    };
    runner.restore(&backup).map_err(ToolError::Failed)?;

    // The old files go back. A file that was never replaced has no
    // `.previous` beside it, and the one in place IS the old one.
    restore_previous(dir, "docker-compose.yml")?;
    restore_previous(dir, "release.json")?;

    // The stack up, with the old files.
    runner.up().map_err(ToolError::Failed)?;

    // The wait, on the old version.
    runner::wait_for_ready(runner, version, timeout).map_err(ToolError::Failed)?;

    // The state goes back, the pause is cleared, and the services that
    // read it are recreated.
    //
    // The state goes back FIRST: the stack runs the old release now,
    // and a pause that could not be cleared must not leave the state
    // naming the new one.
    //
    // The services are recreated because they read the pause when they
    // are CREATED (D16): clearing the line in the `.env` alone would
    // leave them running with the value they were created with, and
    // the jobs that delete files would stay skipped (REL-2 S4b review
    // round 2 BLOCKER 2).
    state::restore_previous(dir, version, Some(&backup))?;
    pause::clear_and_recreate(dir, runner)?;
    Ok(())
}

/// Put one file of the install back from its `.previous`.
///
/// A file with no `.previous` beside it was never replaced, so the one
/// in place is already the old one.
///
/// # Errors
///
/// Returns the error that says what could not be written.
fn restore_previous(dir: &Path, name: &str) -> Result<(), ToolError> {
    let previous = dir.join(format!("{name}.previous"));
    if !previous.exists() {
        return Ok(());
    }
    std::fs::copy(&previous, dir.join(name)).map_err(|source| ToolError::Io {
        context: format!("{} could not be put back", dir.join(name).display()),
        source,
    })?;
    Ok(())
}

/// Ask the operator to confirm, and read the answer.
///
/// # Errors
///
/// Returns the error that says the answer could not be read.
fn confirmed() -> Result<bool, ToolError> {
    print!("lakehousectl: type `yes` to go on: ");
    std::io::stdout().flush().map_err(|source| ToolError::Io {
        context: "the question could not be written".to_string(),
        source,
    })?;
    let mut answer = String::new();
    std::io::stdin()
        .read_line(&mut answer)
        .map_err(|source| ToolError::Io {
            context: "the answer could not be read".to_string(),
            source,
        })?;
    Ok(answer.trim().eq_ignore_ascii_case("yes"))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::time::Instant;

    use super::*;
    use crate::runner::ApiState;

    /// A runner that records the calls and fails where it is told to.
    struct Fake {
        /// The calls, in order.
        calls: Vec<String>,
        /// Whether the pause was set when each call was made.
        ///
        /// The services are created with the value the `.env` holds at
        /// that moment, so the fake records it: the rollback starts the
        /// stack with the pause still set, and starts it again without
        /// it once the pause is cleared.
        paused: Vec<bool>,
        /// The install's own directory.
        dir: PathBuf,
        /// The steps that fail, when they are set.
        failing: Vec<String>,
        /// The number of `up` calls.
        ups: usize,
        /// The time the fake thinks it is.
        now: Instant,
        /// The version the API reports.
        api_version: String,
    }

    impl Fake {
        fn new(dir: &Path) -> Self {
            Self {
                calls: Vec::new(),
                paused: Vec::new(),
                dir: dir.to_path_buf(),
                failing: Vec::new(),
                ups: 0,
                now: Instant::now(),
                api_version: "0.1.0".to_string(),
            }
        }

        fn failing_at(dir: &Path, step: &str) -> Self {
            Self {
                failing: vec![step.to_string()],
                ..Self::new(dir)
            }
        }

        /// The calls, without the probes.
        fn steps(&self) -> Vec<String> {
            self.calls
                .iter()
                .filter(|call| !call.contains("api-state"))
                .cloned()
                .collect()
        }

        /// The calls, without the probes, and whether the pause was set
        /// at each of them.
        fn steps_with_the_pause(&self) -> Vec<(String, bool)> {
            self.calls
                .iter()
                .zip(self.paused.iter())
                .filter(|(call, _)| !call.contains("api-state"))
                .map(|(call, paused)| (String::clone(call), *paused))
                .collect()
        }

        /// Record a call, and whether the pause was set when it was
        /// made.
        fn record(&mut self, step: &str) {
            let paused = self.pause_is_set();
            self.calls.push(step.to_string());
            self.paused.push(paused);
        }

        /// Whether the install's `.env` holds the pause right now.
        fn pause_is_set(&self) -> bool {
            let Ok(env) = std::fs::read_to_string(self.dir.join(".env")) else {
                return false;
            };
            let prefix = format!("{}=", pause::VARIABLE);
            env.lines().any(|line| line.trim().starts_with(&prefix))
        }
    }

    impl Runner for Fake {
        fn now(&self) -> Instant {
            self.now
        }

        fn sleep(&mut self, duration: Duration) {
            self.now += duration;
        }

        fn fetch_images(
            &mut self,
            _manifest: &crate::manifest::Manifest,
            _bundle: Option<&Path>,
        ) -> Result<(), String> {
            self.record("fetch-images");
            Ok(())
        }

        fn stop_writers(&mut self) -> Result<(), String> {
            self.record("stop-writers");
            Ok(())
        }

        fn backup(&mut self, _version: &str) -> Result<PathBuf, String> {
            self.record("backup");
            Ok(PathBuf::from("/backups/one"))
        }

        fn restore(&mut self, _backup: &Path) -> Result<(), String> {
            self.record("restore");
            Ok(())
        }

        fn up(&mut self) -> Result<(), String> {
            // The stack is started more than once — with the pause, and
            // again without it once the pause is cleared — so the fake
            // tells the starts apart: a test can fail one of them and
            // not the other (REL-2 S4b review round 2 BLOCKER 2).
            self.ups += 1;
            let step = if self.ups == 1 { "up" } else { "up-again" };
            self.record(step);
            if let Some(position) = self.failing.iter().position(|failing| failing == step) {
                self.failing.remove(position);
                return Err(format!("the {step} failed"));
            }
            Ok(())
        }

        fn down(&mut self) -> Result<(), String> {
            self.record("down");
            Ok(())
        }

        fn api_state(&mut self) -> Result<ApiState, String> {
            self.record("api-state");
            Ok(ApiState {
                ready: true,
                version: Some(self.api_version.clone()),
            })
        }

        fn console_ready(&mut self) -> Result<bool, String> {
            self.record("console-ready");
            Ok(true)
        }
    }

    /// An install's own directory, with the state of an install of
    /// `0.2.0` that was updated from `0.1.0`.
    fn install_with_state(
        current: &str,
        previous: Option<&str>,
        backup: Option<&str>,
    ) -> tempfile::TempDir {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let mut state = state::State::first_install(current);
        state.previous_version = previous.map(str::to_string);
        state.last_backup = backup.map(str::to_string);
        state::write(directory.path(), &state).expect("the state is written");
        std::fs::write(directory.path().join(".env"), "POSTGRES_PASSWORD=secret\n")
            .expect("the env file is written");
        directory
    }

    #[test]
    fn a_rollback_without_a_previous_release_is_refused() {
        let directory = install_with_state("0.2.0", None, Some("/backups/one"));
        let error = command(&RollbackArgs {
            dir: directory.path().to_path_buf(),
            yes: true,
        })
        .expect_err("a rollback with no previous release is refused");
        let message = error.to_string();
        assert!(
            message.contains("previous release"),
            "the error says what is missing: {message}"
        );
    }

    #[test]
    fn a_rollback_without_a_backup_is_refused() {
        let directory = install_with_state("0.2.0", Some("0.1.0"), None);
        let error = command(&RollbackArgs {
            dir: directory.path().to_path_buf(),
            yes: true,
        })
        .expect_err("a rollback with no backup is refused");
        let message = error.to_string();
        assert!(
            message.contains("backup"),
            "the error says what is missing: {message}"
        );
    }

    #[test]
    fn the_rollback_stops_the_stack_restores_the_backup_and_starts_it_again() {
        let directory = install_with_state("0.2.0", Some("0.1.0"), Some("/backups/one"));
        let mut runner = Fake::new(directory.path());

        run(
            directory.path(),
            &mut runner,
            "0.1.0",
            Duration::from_secs(60),
        )
        .expect("the rollback runs");

        assert_eq!(
            runner.steps(),
            vec![
                "down".to_string(),
                "restore".to_string(),
                "up".to_string(),
                "console-ready".to_string(),
                // The last start recreates the services that read the
                // pause, now that it is cleared (REL-2 S4b review
                // round 2 BLOCKER 2).
                "up-again".to_string(),
            ],
            "the steps ran in another order"
        );
    }

    #[test]
    fn the_previous_files_go_back_and_the_state_does_too() {
        let directory = install_with_state("0.2.0", Some("0.1.0"), Some("/backups/one"));
        let path = directory.path();
        std::fs::write(path.join("docker-compose.yml"), "services: {}\n").expect("the compose");
        std::fs::write(
            path.join("docker-compose.yml.previous"),
            "services: {old: true}\n",
        )
        .expect("the old compose");
        std::fs::write(path.join("release.json"), "{\"version\": \"0.2.0\"}\n")
            .expect("the manifest");
        std::fs::write(
            path.join("release.json.previous"),
            "{\"version\": \"0.1.0\"}\n",
        )
        .expect("the old manifest");
        let mut runner = Fake::new(path);

        run(path, &mut runner, "0.1.0", Duration::from_secs(60)).expect("the rollback runs");

        assert_eq!(
            std::fs::read_to_string(path.join("docker-compose.yml")).expect("the compose"),
            "services: {old: true}\n",
            "the old Compose file was not put back"
        );
        assert_eq!(
            std::fs::read_to_string(path.join("release.json")).expect("the manifest"),
            "{\"version\": \"0.1.0\"}\n",
            "the old manifest was not put back"
        );
        let state = state::read(path).expect("the state is read");
        assert_eq!(
            state.current_version.as_deref(),
            Some("0.1.0"),
            "the state does not name the release the rollback returned to"
        );
    }

    #[test]
    fn the_pause_is_cleared_by_the_rollback() {
        let directory = install_with_state("0.2.0", Some("0.1.0"), Some("/backups/one"));
        let path = directory.path();
        std::fs::write(
            path.join(".env"),
            "POSTGRES_PASSWORD=secret\nMAINTENANCE_PAUSED=true\n",
        )
        .expect("the env file is written");
        let mut runner = Fake::new(path);

        run(path, &mut runner, "0.1.0", Duration::from_secs(60)).expect("the rollback runs");

        let env = std::fs::read_to_string(path.join(".env")).expect("the env file is readable");
        assert!(
            !env.contains("MAINTENANCE_PAUSED"),
            "the pause was not cleared: {env}"
        );
    }

    #[test]
    fn the_rollback_recreates_the_services_after_the_pause_is_cleared() {
        // REL-2 S4b review round 2 BLOCKER 2: the Dagster services read
        // MAINTENANCE_PAUSED when they are CREATED, so the rollback's
        // own start creates them WITH the pause. Clearing the line in
        // the `.env` is not enough: they have to be created again
        // without it, or the jobs that delete files stay skipped.
        let directory = install_with_state("0.2.0", Some("0.1.0"), Some("/backups/one"));
        let path = directory.path();
        std::fs::write(
            path.join(".env"),
            "POSTGRES_PASSWORD=secret\nMAINTENANCE_PAUSED=true\n",
        )
        .expect("the env file is written");
        let mut runner = Fake::new(path);

        run(path, &mut runner, "0.1.0", Duration::from_secs(60)).expect("the rollback runs");

        assert_eq!(
            runner.steps_with_the_pause(),
            vec![
                ("down".to_string(), true),
                ("restore".to_string(), true),
                // The stack is created with the pause...
                ("up".to_string(), true),
                ("console-ready".to_string(), true),
                // ...and created again without it, right after the pause
                // is cleared.
                ("up-again".to_string(), false),
            ],
            "the services were not recreated without the pause"
        );
    }

    #[test]
    fn a_rollback_that_cannot_recreate_the_services_says_so() {
        // REL-2 S4b review round 2 BLOCKER 2: the second start can
        // fail, and then the operator has to be told — the jobs that
        // delete files stay skipped.
        let directory = install_with_state("0.2.0", Some("0.1.0"), Some("/backups/one"));
        let path = directory.path();
        std::fs::write(
            path.join(".env"),
            "POSTGRES_PASSWORD=secret\nMAINTENANCE_PAUSED=true\n",
        )
        .expect("the env file is written");
        let mut runner = Fake::failing_at(path, "up-again");

        let error = run(path, &mut runner, "0.1.0", Duration::from_secs(60))
            .expect_err("the rollback does not hide it");

        let message = error.to_string();
        assert!(
            message.contains("the services that read the pause could not be recreated"),
            "the error says the services were not recreated: {message}"
        );
        assert!(
            message.contains("stay skipped"),
            "the error says what stays skipped: {message}"
        );
        let env = std::fs::read_to_string(path.join(".env")).expect("the env file is readable");
        assert!(
            !env.contains("MAINTENANCE_PAUSED"),
            "the pause was not cleared: {env}"
        );
        let state = state::read(path).expect("the state is read");
        assert_eq!(
            state.current_version.as_deref(),
            Some("0.1.0"),
            "the state does not name the release the stack runs"
        );
    }
}
