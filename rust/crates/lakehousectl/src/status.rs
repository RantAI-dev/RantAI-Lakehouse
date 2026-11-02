//! `lakehousectl status`: what the install is doing right now (REL-2 S4 decision D19).
//!
//! # What it shows
//!
//! D19 names five things, and the command shows all five:
//!
//! - the installed release, and the one before it (`state.json`);
//! - every service's state (`docker compose ps --format json`);
//! - the API's own `/health/ready`;
//! - the backups, newest first, with their size;
//! - whether a newer release exists. ONLINE it says so; OFFLINE it
//!   says that it did not check — a status that hid the check would be
//!   a status that lied.
//!
//! # How it is written
//!
//! The facts are gathered once ([`Facts`]) and the report is a
//! function of them ([`report`]), so the report can be tested with
//! facts injected: a status that shows a stack that is not there is a
//! report, not a machine.

use std::fmt::Write as _;
use std::path::Path;

use serde_json::Value;

use crate::cli::StatusArgs;
use crate::docker;
use crate::env;
use crate::error::ToolError;
use crate::http;
use crate::release;
use crate::runner::ApiState;
use crate::state;
use crate::version;

/// The API's port when the `.env` does not say.
const DEFAULT_API_PORT: u16 = 8080;

/// A backup of the install.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Backup {
    /// The backup's directory name: `<stamp>-<version>`.
    pub name: String,
    /// The size of the whole set, in bytes.
    pub size: u64,
}

/// What the status reads.
#[derive(Debug, Clone, Default)]
pub struct Facts {
    /// The state of the install.
    pub state: state::State,
    /// The services' state, as Compose's own JSON.
    pub services: String,
    /// What the API says about itself, when it answered.
    pub api: Option<ApiState>,
    /// The backups, newest first.
    pub backups: Vec<Backup>,
    /// The newest release, when it could be read.
    pub newest: Option<String>,
    /// Why the newest release could not be read.
    pub newest_error: Option<String>,
}

/// Run `lakehousectl status`.
///
/// # Errors
///
/// Returns the error that says what could not be read: a state file
/// that is not there, or one that is not JSON.
pub fn command(args: &StatusArgs) -> Result<(), ToolError> {
    let facts = gather(&args.dir)?;
    println!("{}", report(&facts));
    Ok(())
}

/// Gather what the status shows.
///
/// The STATE is needed: without it there is no install to report on,
/// and the command says so. Every other fact is best-effort — a status
/// that stops at the first thing it cannot read says nothing about the
/// rest of the install.
fn gather(dir: &Path) -> Result<Facts, ToolError> {
    let state = state::read(dir)?;
    let env_file = dir.join(".env");
    let services = docker::compose_ps(dir, &env_file).unwrap_or_default();
    let api = api_state(dir);
    let backups = backups(dir);
    let (newest, newest_error) = newest_release();
    Ok(Facts {
        state,
        services,
        api,
        backups,
        newest,
        newest_error,
    })
}

/// What the API says about itself.
fn api_state(dir: &Path) -> Option<ApiState> {
    let settings = env::read(&dir.join(".env"));
    let port = env::port(&settings, "API_HOST_PORT", DEFAULT_API_PORT);
    let response = http::get("127.0.0.1", port, "/health/ready").ok()?;
    if response.status != 200 {
        return Some(ApiState {
            ready: false,
            version: None,
        });
    }
    let document: Value = serde_json::from_str(&response.body).ok()?;
    Some(ApiState {
        ready: document["ready"] == Value::Bool(true),
        version: document["version"].as_str().map(str::to_string),
    })
}

/// The backups of the install, newest first.
///
/// The name of a backup is `<stamp>-<version>`, and the stamp is a
/// UTC timestamp — so the names sort by time.
fn backups(dir: &Path) -> Vec<Backup> {
    let root = dir.join("backups");
    let Ok(entries) = std::fs::read_dir(&root) else {
        return Vec::new();
    };
    let mut found: Vec<Backup> = entries
        .filter_map(std::result::Result::ok)
        .filter(|entry| entry.path().is_dir())
        .map(|entry| Backup {
            name: entry.file_name().to_string_lossy().to_string(),
            size: size_of(&entry.path()),
        })
        .collect();
    found.sort_by(|one, other| other.name.cmp(&one.name));
    found
}

/// The size of a directory, in bytes.
fn size_of(path: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(path) else {
        return 0;
    };
    entries
        .filter_map(std::result::Result::ok)
        .map(|entry| {
            entry.metadata().map_or(0, |metadata| {
                if metadata.is_dir() {
                    size_of(&entry.path())
                } else {
                    metadata.len()
                }
            })
        })
        .sum()
}

/// The newest release, and why it could not be read.
fn newest_release() -> (Option<String>, Option<String>) {
    match release::latest_version() {
        Ok(version) => (Some(version), None),
        Err(error) => (None, Some(error.to_string())),
    }
}

/// The report, as the facts make it.
#[must_use]
pub fn report(facts: &Facts) -> String {
    let mut out = String::new();

    let _ = writeln!(
        out,
        "the installed release: {}",
        facts
            .state
            .current_version
            .as_deref()
            .unwrap_or("none (the install's state names no release)")
    );
    let _ = writeln!(
        out,
        "the release before it: {}",
        facts
            .state
            .previous_version
            .as_deref()
            .unwrap_or("none (there was no release before it)")
    );

    out.push_str("the services:\n");
    let services = services_of(&facts.services);
    if services.is_empty() {
        out.push_str("  none could be read\n");
    }
    for service in &services {
        let _ = writeln!(out, "{service}");
    }

    match &facts.api {
        Some(api) if api.ready => {
            let _ = writeln!(
                out,
                "the API: ready, and it runs {}",
                api.version.as_deref().unwrap_or("an unnamed release")
            );
        }
        Some(api) => {
            let _ = writeln!(
                out,
                "the API: NOT ready (it answered: ready: {}, version: {})",
                api.ready,
                api.version.as_deref().unwrap_or("none")
            );
        }
        None => out.push_str("the API: it did not answer\n"),
    }

    out.push_str("the backups:\n");
    if facts.backups.is_empty() {
        out.push_str("  none\n");
    }
    for backup in &facts.backups {
        let _ = writeln!(out, "  {} ({} bytes)", backup.name, backup.size);
    }

    match (&facts.newest, &facts.newest_error) {
        (Some(newest), _) => {
            let installed = facts.state.current_version.as_deref();
            match installed {
                Some(installed) => {
                    let newer =
                        version::compare(newest, installed).is_ok_and(std::cmp::Ordering::is_gt);
                    if newer {
                        let _ = writeln!(
                            out,
                            "a newer release exists: {newest} (the installed one is {installed})"
                        );
                    } else {
                        let _ = writeln!(
                            out,
                            "the newest release is {newest}, and it is the one that is installed"
                        );
                    }
                }
                None => {
                    let _ = writeln!(out, "the newest release is {newest}");
                }
            }
        }
        (None, Some(error)) => {
            let _ = writeln!(
                out,
                "the newest release was NOT checked: this host could not reach the release feed \
                 ({error})"
            );
        }
        (None, None) => out.push_str("the newest release was not checked\n"),
    }

    out
}

/// The services of the deployment, as Compose's own JSON says them.
///
/// Compose prints the services as a JSON array in some versions and as
/// one JSON object per line in others, so both are read.
fn services_of(json: &str) -> Vec<String> {
    let text = json.trim();
    if text.is_empty() {
        return Vec::new();
    }
    if let Ok(Value::Array(items)) = serde_json::from_str::<Value>(text) {
        return items.iter().filter_map(service_line).collect();
    }
    text.lines()
        .filter_map(|line| {
            serde_json::from_str::<Value>(line)
                .ok()
                .as_ref()
                .and_then(service_line)
        })
        .collect()
}

/// One service, as a line of the report.
fn service_line(service: &Value) -> Option<String> {
    let name = service["Service"].as_str().or(service["Name"].as_str())?;
    let state = service["State"].as_str().unwrap_or("unknown");
    let status = service["Status"].as_str().unwrap_or("");
    let health = service["Health"].as_str().unwrap_or("");
    let mut line = format!("  {name}: {state}");
    if !status.is_empty() {
        let _ = write!(line, " ({status}");
        if !health.is_empty() {
            let _ = write!(line, ", {health}");
        }
        line.push(')');
    }
    Some(line)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    /// The facts of an install that is running.
    fn facts_of() -> Facts {
        Facts {
            state: state::State::first_install("0.2.0"),
            services: "{\"Name\":\"api\",\"Service\":\"lakehouse-api\",\"State\":\"running\",\
                       \"Status\":\"Up 2 minutes\",\"Health\":\"healthy\"}\n"
                .to_string(),
            api: Some(ApiState {
                ready: true,
                version: Some("0.2.0".to_string()),
            }),
            backups: vec![Backup {
                name: "20261102T101500Z-0.1.0".to_string(),
                size: 4096,
            }],
            newest: Some("0.3.0".to_string()),
            newest_error: None,
        }
    }

    #[test]
    fn the_report_shows_the_installed_release() {
        let report = report(&facts_of());
        assert!(
            report.contains("the installed release: 0.2.0"),
            "the report is {report}"
        );
    }

    #[test]
    fn the_report_shows_every_service() {
        let report = report(&facts_of());
        assert!(
            report.contains("lakehouse-api: running"),
            "the report is {report}"
        );
        assert!(report.contains("healthy"), "the report is {report}");
    }

    #[test]
    fn the_report_shows_the_dagster_services() {
        // REL-2 S4b review round 3 BLOCKER 3, decision D22: the Dagster
        // services are in the `dagster` profile, and the status must
        // list them — a status that hid a running service would be a
        // status that lied.
        let mut facts = facts_of();
        facts.services = concat!(
            "{\"Name\":\"dagster-daemon-1\",\"Service\":\"dagster-daemon\",\"State\":\"running\",\
             \"Status\":\"Up 3 minutes\",\"Health\":\"healthy\"}\n",
            "{\"Name\":\"dagster-code-location-1\",\"Service\":\"dagster-code-location\",\
             \"State\":\"running\",\"Status\":\"Up 3 minutes\",\"Health\":\"healthy\"}\n",
        )
        .to_string();

        let report = report(&facts);

        assert!(
            report.contains("dagster-daemon: running"),
            "the report does not show the Dagster daemon: {report}"
        );
        assert!(
            report.contains("dagster-code-location: running"),
            "the report does not show the Dagster code location: {report}"
        );
    }

    #[test]
    fn the_report_shows_the_api() {
        let report = report(&facts_of());
        assert!(
            report.contains("the API: ready, and it runs 0.2.0"),
            "the report is {report}"
        );
    }

    #[test]
    fn the_report_shows_the_backups_with_their_size() {
        let report = report(&facts_of());
        assert!(
            report.contains("20261102T101500Z-0.1.0 (4096 bytes)"),
            "the report is {report}"
        );
    }

    #[test]
    fn the_report_says_that_a_newer_release_exists() {
        let report = report(&facts_of());
        assert!(
            report.contains("a newer release exists: 0.3.0"),
            "the report is {report}"
        );
    }

    #[test]
    fn the_report_says_that_the_newest_release_is_the_installed_one() {
        let mut facts = facts_of();
        facts.newest = Some("0.2.0".to_string());
        let report = report(&facts);
        assert!(
            report.contains("the newest release is 0.2.0, and it is the one that is installed"),
            "the report is {report}"
        );
    }

    #[test]
    fn the_report_says_that_the_newest_release_was_not_checked() {
        // D19: offline, the status says that it did not check. A
        // status that hid the check would be a status that lied.
        let mut facts = facts_of();
        facts.newest = None;
        facts.newest_error = Some("this host is offline".to_string());
        let report = report(&facts);
        assert!(
            report.contains("the newest release was NOT checked"),
            "the report is {report}"
        );
        assert!(
            report.contains("this host is offline"),
            "the report says why: {report}"
        );
    }

    #[test]
    fn an_api_that_did_not_answer_is_said_so() {
        let mut facts = facts_of();
        facts.api = None;
        let report = report(&facts);
        assert!(
            report.contains("the API: it did not answer"),
            "the report is {report}"
        );
    }

    #[test]
    fn an_api_that_is_not_ready_is_said_so() {
        let mut facts = facts_of();
        facts.api = Some(ApiState {
            ready: false,
            version: Some("0.2.0".to_string()),
        });
        let report = report(&facts);
        assert!(
            report.contains("the API: NOT ready"),
            "the report is {report}"
        );
    }

    #[test]
    fn the_services_are_read_from_a_json_array() {
        let services = services_of(
            "[{\"Service\":\"lakehouse-api\",\"State\":\"running\",\"Status\":\"Up\"}]",
        );
        assert_eq!(services.len(), 1, "the services are {services:?}");
        assert!(
            services[0].contains("lakehouse-api: running"),
            "the service is {services:?}"
        );
    }

    #[test]
    fn the_services_are_read_from_one_object_per_line() {
        let services = services_of(
            "{\"Service\":\"lakehouse-api\",\"State\":\"running\"}\n\
             {\"Service\":\"console\",\"State\":\"exited\"}\n",
        );
        assert_eq!(services.len(), 2, "the services are {services:?}");
        assert!(
            services[1].contains("console: exited"),
            "the service is {services:?}"
        );
    }

    #[test]
    fn the_backups_are_newest_first() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let root = directory.path().join("backups");
        std::fs::create_dir_all(root.join("20261101T101500Z-0.1.0")).expect("the first backup");
        std::fs::create_dir_all(root.join("20261102T101500Z-0.2.0")).expect("the second backup");
        std::fs::write(root.join("20261101T101500Z-0.1.0/manifest.json"), "{}").expect("a part");

        let found = backups(directory.path());

        assert_eq!(found.len(), 2, "the backups are {found:?}");
        assert_eq!(
            found[0].name, "20261102T101500Z-0.2.0",
            "the backups are not newest first: {found:?}"
        );
        assert_eq!(found[1].size, 2, "the size is {found:?}");
    }

    #[test]
    fn an_install_with_no_backups_has_none() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        assert_eq!(backups(directory.path()), Vec::new());
    }
}
