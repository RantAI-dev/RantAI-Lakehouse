//! What the update and the rollback do to the machine (REL-2 S4 decisions D17 and D18).
//!
//! # Why the steps are behind a trait
//!
//! Every step that touches the machine is a method of [`Runner`], so
//! the whole path can be driven by a fake that records the calls and
//! fails where it is told to. That is the same shape the pre-flight
//! checks have (REL-2 S3-T2): a rule is tested with facts injected,
//! not with a machine — and an update that stops the writers, takes a
//! backup and rolls back is not something a unit test may do to a real
//! host.
//!
//! # What the runner owns
//!
//! - the images of the release (pulled, or loaded from a bundle);
//! - the writers (stopped for an update);
//! - the backup (taken, and restored);
//! - the stack (started, and stopped);
//! - the API and the console (asked whether they are ready).
//!
//! The files the update swaps, the pause and the state are NOT the
//! runner's: they are the update's own, and they are on the disk where
//! a test can read them.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::backup;
use crate::docker;
use crate::env;
use crate::http;
use crate::images;
use crate::manifest::Manifest;

/// How long the tool waits between two probes.
const PROBE_INTERVAL: Duration = Duration::from_secs(5);

/// The API's port when the `.env` does not say.
const DEFAULT_API_PORT: u16 = 8080;
/// The console's port when the `.env` does not say.
const DEFAULT_CONSOLE_PORT: u16 = 3000;

/// The services that WRITE, and are stopped for an update (REL-2 S4 decision D17).
///
/// `Postgres`, `ClickHouse`, `RustFS` and `Lakekeeper` keep running:
/// they are what the backup is made of.
pub const WRITERS: [&str; 5] = [
    "lakehouse-api",
    "console",
    "dagster-daemon",
    "dagster-code-location",
    "dagster-webserver",
];

/// What the API says about itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiState {
    /// Whether the API reports itself ready.
    pub ready: bool,
    /// The version it reports.
    pub version: Option<String>,
}

/// Every step of the update and the rollback that touches the machine.
pub trait Runner {
    /// The time now.
    ///
    /// The waiting is driven by this and by [`Runner::sleep`], so a
    /// test can wait for a timeout without waiting for it.
    fn now(&self) -> Instant;

    /// Wait.
    fn sleep(&mut self, duration: Duration);

    /// Pull or load every image the release runs.
    ///
    /// A failure here leaves the running stack as it was: no image is
    /// needed by a container that is already running (REL-2 S4
    /// decision D17).
    fn fetch_images(&mut self, manifest: &Manifest, bundle: Option<&Path>) -> Result<(), String>;

    /// Stop the writers. The databases keep running.
    fn stop_writers(&mut self) -> Result<(), String>;

    /// Take a backup of the stack, and say which backup it is.
    ///
    /// The version is the release the backup belongs to.
    fn backup(&mut self, version: &str) -> Result<PathBuf, String>;

    /// Restore a backup of the stack.
    fn restore(&mut self, backup: &Path) -> Result<(), String>;

    /// Start the stack, from the files in the install's own directory.
    fn up(&mut self) -> Result<(), String>;

    /// Stop every service of the stack.
    fn down(&mut self) -> Result<(), String>;

    /// What the API says about itself.
    fn api_state(&mut self) -> Result<ApiState, String>;

    /// Whether the console answers.
    fn console_ready(&mut self) -> Result<bool, String>;
}

/// Wait for the API to report the release as ready, and for the console.
///
/// The API must report the version the caller names: an API that is
/// ready but runs another release is not the release that was asked
/// for, and an update that accepted it would report a success it did
/// not have.
///
/// # Errors
///
/// Returns the message that says what the stack answered instead.
pub fn wait_for_ready(
    runner: &mut dyn Runner,
    version: &str,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = runner.now() + timeout;
    loop {
        let message = match runner.api_state() {
            Ok(state) if state.ready && state.version.as_deref() == Some(version) => {
                match runner.console_ready() {
                    Ok(true) => return Ok(()),
                    Ok(false) => "the console did not answer".to_string(),
                    Err(error) => error,
                }
            }
            Ok(state) => format!(
                "the API answered but is not ready (ready: {}, version: {})",
                state.ready,
                state.version.as_deref().unwrap_or("none")
            ),
            Err(error) => error,
        };
        if runner.now() >= deadline {
            return Err(format!(
                "the stack did not become ready within {} seconds: {message}",
                timeout.as_secs()
            ));
        }
        runner.sleep(PROBE_INTERVAL);
    }
}

/// The runner that really runs the machine's commands.
pub struct Local {
    /// The install's own directory.
    dir: PathBuf,
}

impl Local {
    /// The runner for an install in `dir`.
    #[must_use]
    pub fn new(dir: &Path) -> Self {
        Self {
            dir: dir.to_path_buf(),
        }
    }

    /// The install's `.env`.
    fn env_file(&self) -> PathBuf {
        self.dir.join(".env")
    }

    /// The API's host port.
    fn api_port(&self) -> u16 {
        let settings = env::read(&self.env_file());
        env::port(&settings, "API_HOST_PORT", DEFAULT_API_PORT)
    }

    /// The console's host port.
    fn console_port(&self) -> u16 {
        let settings = env::read(&self.env_file());
        env::port(&settings, "CONSOLE_HOST_PORT", DEFAULT_CONSOLE_PORT)
    }

    /// The writers that this deployment actually runs.
    ///
    /// A service of a profile the deployment does not run is not
    /// stopped: Compose refuses a service it does not know (REL-2 S4
    /// decision D17).
    fn writers(&self) -> Result<Vec<String>, String> {
        let services = docker::compose_services(&self.dir, &self.env_file())?;
        Ok(services
            .into_iter()
            .filter(|service| WRITERS.contains(&service.as_str()))
            .collect())
    }
}

impl Runner for Local {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn sleep(&mut self, duration: Duration) {
        std::thread::sleep(duration);
    }

    fn fetch_images(&mut self, manifest: &Manifest, bundle: Option<&Path>) -> Result<(), String> {
        images::fetch(manifest, bundle).map_err(|error| error.to_string())
    }

    fn stop_writers(&mut self) -> Result<(), String> {
        let writers = self.writers()?;
        if writers.is_empty() {
            return Ok(());
        }
        docker::compose_stop(&self.dir, &self.env_file(), &writers)
    }

    fn backup(&mut self, version: &str) -> Result<PathBuf, String> {
        backup::take(&self.dir, Some(version)).map_err(|error| error.to_string())
    }

    fn restore(&mut self, backup: &Path) -> Result<(), String> {
        backup::restore(&self.dir, backup).map_err(|error| error.to_string())
    }

    fn up(&mut self) -> Result<(), String> {
        docker::compose_up(&self.dir, &self.env_file())
    }

    fn down(&mut self) -> Result<(), String> {
        docker::compose_stop_all(&self.dir, &self.env_file())
    }

    fn api_state(&mut self) -> Result<ApiState, String> {
        let response = http::get("127.0.0.1", self.api_port(), "/health/ready")
            .map_err(|error| error.to_string())?;
        if response.status != 200 {
            return Ok(ApiState {
                ready: false,
                version: None,
            });
        }
        let document: Value = serde_json::from_str(&response.body)
            .map_err(|error| format!("the API's answer is not JSON: {error}"))?;
        Ok(ApiState {
            ready: document["ready"] == Value::Bool(true),
            version: document["version"].as_str().map(str::to_string),
        })
    }

    fn console_ready(&mut self) -> Result<bool, String> {
        let response =
            http::get("127.0.0.1", self.console_port(), "/").map_err(|error| error.to_string())?;
        Ok(response.status == 200)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    /// A runner that fails at one named step, and records the calls.
    struct Fake {
        /// The calls, in order.
        calls: Vec<String>,
        /// The step that fails, when it is set.
        failing: Option<String>,
        /// The time the fake thinks it is.
        now: Instant,
        /// The version the API reports.
        api_version: String,
        /// Whether the API reports itself ready.
        api_ready: bool,
    }

    impl Fake {
        fn new() -> Self {
            Self {
                calls: Vec::new(),
                failing: None,
                now: Instant::now(),
                api_version: "0.2.0".to_string(),
                api_ready: true,
            }
        }

        /// The call, or the failure it stands for.
        fn call(&mut self, step: &str) -> Result<(), String> {
            self.calls.push(step.to_string());
            if self.failing.as_deref() == Some(step) {
                return Err(format!("the {step} failed"));
            }
            Ok(())
        }
    }

    impl Runner for Fake {
        fn now(&self) -> Instant {
            self.now
        }

        fn sleep(&mut self, duration: Duration) {
            self.now += duration;
            self.calls.push("sleep".to_string());
        }

        fn fetch_images(
            &mut self,
            _manifest: &Manifest,
            _bundle: Option<&Path>,
        ) -> Result<(), String> {
            self.call("fetch-images")
        }

        fn stop_writers(&mut self) -> Result<(), String> {
            self.call("stop-writers")
        }

        fn backup(&mut self, _version: &str) -> Result<PathBuf, String> {
            self.call("backup")?;
            Ok(PathBuf::from("/backups/one"))
        }

        fn restore(&mut self, _backup: &Path) -> Result<(), String> {
            self.call("restore")
        }

        fn up(&mut self) -> Result<(), String> {
            self.call("up")
        }

        fn down(&mut self) -> Result<(), String> {
            self.call("down")
        }

        fn api_state(&mut self) -> Result<ApiState, String> {
            self.calls.push("api-state".to_string());
            if self.failing.as_deref() == Some("api-state") {
                return Err("the API did not answer".to_string());
            }
            Ok(ApiState {
                ready: self.api_ready,
                version: Some(self.api_version.clone()),
            })
        }

        fn console_ready(&mut self) -> Result<bool, String> {
            self.call("console-ready")?;
            Ok(true)
        }
    }

    #[test]
    fn the_stack_is_ready_when_the_api_reports_the_version_as_ready() {
        let mut runner = Fake::new();
        wait_for_ready(&mut runner, "0.2.0", Duration::from_secs(60)).expect("the stack is ready");
        assert_eq!(
            runner.calls,
            vec!["api-state".to_string(), "console-ready".to_string()]
        );
    }

    #[test]
    fn an_api_that_reports_another_version_is_not_ready() {
        let mut runner = Fake::new();
        runner.api_version = "0.1.0".to_string();
        let error = wait_for_ready(&mut runner, "0.2.0", Duration::from_secs(10))
            .expect_err("another version is not ready");
        assert!(
            error.contains("0.1.0"),
            "the error says what the API reported: {error}"
        );
    }

    #[test]
    fn a_stack_that_never_becomes_ready_fails_when_the_time_is_up() {
        let mut runner = Fake::new();
        runner.api_ready = false;
        let error = wait_for_ready(&mut runner, "0.2.0", Duration::from_secs(20))
            .expect_err("the stack is not ready");
        assert!(
            error.contains("20 seconds"),
            "the error says how long it waited: {error}"
        );
    }

    #[test]
    fn the_console_is_waited_for_as_well() {
        let mut runner = Fake {
            failing: Some("console-ready".to_string()),
            ..Fake::new()
        };
        let error = wait_for_ready(&mut runner, "0.2.0", Duration::from_secs(10))
            .expect_err("the console does not answer");
        assert!(
            error.contains("console"),
            "the error says what did not answer: {error}"
        );
    }
}
