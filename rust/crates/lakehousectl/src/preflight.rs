//! The pre-flight checks: what the host must have before the stack is
//! installed on it (REL-2 S3-T2).
//!
//! # Why these checks
//!
//! The spec's numbers are signed (REL-2, 2026-10-31): Linux on `x86_64`,
//! Docker 24 or newer, Compose 2.24 or newer, 4 CPUs or more, 16 GiB of
//! memory or more, 50 GiB of free disk or more, and the published ports
//! free. A host below a minimum is REFUSED, with a message that says
//! what to fix (`REL-2-AC3`) — a stack that starts on a host too small
//! for it fails later, in a way nobody can act on.
//!
//! # How they are written
//!
//! Every check is a function of a [`Facts`] struct, and the facts are
//! gathered once from the machine. So each check can be tested with
//! facts injected: a test says "a host with two CPUs is refused" without
//! needing a host with two CPUs.
//!
//! The names of the checks are what `--ignore-preflight` takes. An
//! operator may skip ONE check, and the skip is said out loud: a check
//! that was skipped is a check that did not happen, and the operator
//! has to know that.

use std::collections::BTreeMap;
use std::path::Path;

use crate::cli::PreflightArgs;
use crate::error::ToolError;
use crate::sys;

/// The names of the checks, in the order they run.
pub const CHECKS: [&str; 7] = ["os", "docker", "compose", "cpu", "memory", "disk", "ports"];

/// Docker 24 or newer.
const MIN_DOCKER: (u32, u32) = (24, 0);
/// Compose 2.24 or newer.
const MIN_COMPOSE: (u32, u32) = (2, 24);
/// Four CPUs or more.
const MIN_CPUS: usize = 4;
/// 16 GiB of memory or more.
const MIN_MEMORY_BYTES: u64 = 16 * 1024 * 1024 * 1024;
/// 50 GiB of free disk or more.
const MIN_DISK_BYTES: u64 = 50 * 1024 * 1024 * 1024;
/// One GiB, for the messages.
const GIB: u64 = 1024 * 1024 * 1024;

/// A port the release's Compose file publishes on the host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishedPort {
    /// Where the port comes from: the name of the variable that sets
    /// it, or the literal the Compose file carries.
    pub source: String,
    /// The host port.
    pub port: u16,
    /// Whether something on this host already holds it.
    pub busy: bool,
}

/// What the pre-flight checks read.
///
/// Gathered once ([`collect`]) and read by the checks, so that every
/// check is a pure function of this struct.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Facts {
    /// The operating system the tool runs on.
    pub os: String,
    /// The architecture the tool runs on.
    pub arch: String,
    /// The Docker server's version, if Docker answered.
    pub docker_version: Option<String>,
    /// The Compose plugin's version, if it answered.
    pub compose_version: Option<String>,
    /// The CPUs the tool may use.
    pub cpus: usize,
    /// The machine's memory in bytes, if it could be read.
    pub memory_bytes: Option<u64>,
    /// The free disk under the install directory, in bytes.
    pub disk_free_bytes: Option<u64>,
    /// The ports the release's Compose file publishes on the host.
    pub published_ports: Vec<PublishedPort>,
}

/// One pre-flight check: a function of the facts that either passes or
/// says what to fix.
type Check = fn(&Facts) -> Result<(), String>;

/// A pre-flight failure: the check that failed, and what to fix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    /// The name of the check that failed.
    pub check: &'static str,
    /// What to fix, in one sentence.
    pub message: String,
}
/// The operating system and the architecture.
///
/// # Errors
///
/// Returns the message that says what to fix.
pub fn check_os(facts: &Facts) -> Result<(), String> {
    if facts.os != "linux" {
        return Err(format!(
            "this release runs on Linux, and this host is {}. Install it on a Linux host.",
            facts.os
        ));
    }
    if facts.arch != "x86_64" {
        return Err(format!(
            "this release runs on x86_64, and this host is {}. The arm64 build is not out yet \
             (REL-2 decision D12).",
            facts.arch
        ));
    }
    Ok(())
}

/// The Docker server's version.
///
/// # Errors
///
/// Returns the message that says what to fix.
pub fn check_docker(facts: &Facts) -> Result<(), String> {
    let Some(version) = facts.docker_version.as_deref() else {
        return Err(
            "Docker was not found on this host. Install Docker Engine 24 or newer, and make \
             sure the current user may talk to it."
                .to_string(),
        );
    };
    let Some(parsed) = parse_version(version) else {
        return Err(format!(
            "Docker's version {version:?} could not be read. Install Docker Engine 24 or newer."
        ));
    };
    if parsed < MIN_DOCKER {
        return Err(format!(
            "Docker {version} is too old: this release needs Docker 24 or newer. Upgrade Docker \
             and try again."
        ));
    }
    Ok(())
}

/// The Compose plugin's version.
///
/// # Errors
///
/// Returns the message that says what to fix.
pub fn check_compose(facts: &Facts) -> Result<(), String> {
    let Some(version) = facts.compose_version.as_deref() else {
        return Err(
            "Docker Compose was not found on this host. Install the Compose plugin, version 2.24 \
             or newer."
                .to_string(),
        );
    };
    let Some(parsed) = parse_version(version) else {
        return Err(format!(
            "Docker Compose's version {version:?} could not be read. Install the Compose plugin, \
             version 2.24 or newer."
        ));
    };
    if parsed < MIN_COMPOSE {
        return Err(format!(
            "Docker Compose {version} is too old: this release needs Compose 2.24 or newer. \
             Upgrade the Compose plugin and try again."
        ));
    }
    Ok(())
}

/// The number of CPUs.
///
/// # Errors
///
/// Returns the message that says what to fix.
pub fn check_cpu(facts: &Facts) -> Result<(), String> {
    if facts.cpus < MIN_CPUS {
        return Err(format!(
            "this host has {} CPU(s), and the release needs {MIN_CPUS} or more. A smaller host \
             is not supported.",
            facts.cpus
        ));
    }
    Ok(())
}

/// The machine's memory.
///
/// # Errors
///
/// Returns the message that says what to fix.
pub fn check_memory(facts: &Facts) -> Result<(), String> {
    let Some(bytes) = facts.memory_bytes else {
        return Err(
            "the host's memory could not be read (/proc/meminfo). The release needs 16 GiB or \
             more."
                .to_string(),
        );
    };
    if bytes < MIN_MEMORY_BYTES {
        return Err(format!(
            "this host has {} GiB of memory, and the release needs 16 GiB or more. A smaller host \
             is not supported.",
            bytes / GIB
        ));
    }
    Ok(())
}

/// The free disk under the install directory.
///
/// # Errors
///
/// Returns the message that says what to fix.
pub fn check_disk(facts: &Facts) -> Result<(), String> {
    let Some(bytes) = facts.disk_free_bytes else {
        return Err(
            "the free disk under the install directory could not be read. The release needs 50 \
             GiB or more."
                .to_string(),
        );
    };
    if bytes < MIN_DISK_BYTES {
        return Err(format!(
            "there are {} GiB free under the install directory, and the release needs 50 GiB or \
             more. Free some space, or install somewhere else.",
            bytes / GIB
        ));
    }
    Ok(())
}

/// The ports the release publishes.
///
/// # Errors
///
/// Returns the message that says what to fix.
pub fn check_ports(facts: &Facts) -> Result<(), String> {
    let taken: Vec<String> = facts
        .published_ports
        .iter()
        .filter(|port| port.busy)
        .map(|port| format!("{} ({})", port.port, port.source))
        .collect();
    if taken.is_empty() {
        return Ok(());
    }
    Err(format!(
        "these ports are already in use: {}. The console and the API publish them. Stop what \
         holds them, or set the matching variable in .env to another port.",
        taken.join(", ")
    ))
}

/// Check that every name the operator passed names a check.
///
/// # Errors
///
/// Returns the message that says what to fix: a name that names no
/// check would skip nothing and look like it skipped something.
pub fn validate_ignored(ignored: &[String]) -> Result<(), String> {
    for name in ignored {
        if !CHECKS.contains(&name.as_str()) {
            return Err(format!(
                "{name:?} is not a pre-flight check. The checks are: {}.",
                CHECKS.join(", ")
            ));
        }
    }
    Ok(())
}

/// Run the checks, except the ones the operator asked to skip.
///
/// # Errors
///
/// Never: the failures are returned, not raised, so the operator sees
/// every one of them at once.
pub fn run(facts: &Facts, ignored: &[String]) -> Vec<Failure> {
    let checks: [(&str, Check); 7] = [
        ("os", check_os),
        ("docker", check_docker),
        ("compose", check_compose),
        ("cpu", check_cpu),
        ("memory", check_memory),
        ("disk", check_disk),
        ("ports", check_ports),
    ];
    let mut failures = Vec::new();
    for (name, check) in checks {
        if ignored.iter().any(|skipped| skipped == name) {
            continue;
        }
        if let Err(message) = check(facts) {
            failures.push(Failure {
                check: name,
                message,
            });
        }
    }
    failures
}

/// Run `lakehousectl preflight`: check this host against the release's
/// minimums.
///
/// The install runs the same checks before it does anything, so an
/// operator can ask the question on its own.
///
/// # Errors
///
/// Returns [`ToolError::Io`] when the Compose file cannot be read,
/// [`ToolError::Usage`] when a check name names no check, and
/// [`ToolError::Preflight`] when a check failed.
pub fn command(args: &PreflightArgs) -> Result<(), ToolError> {
    let compose = std::fs::read_to_string(&args.compose).map_err(|source| ToolError::Io {
        context: format!("{} could not be read", args.compose.display()),
        source,
    })?;
    validate_ignored(&args.ignore_preflight).map_err(ToolError::Usage)?;

    let facts = collect(&compose, &args.dir);
    for skipped in &args.ignore_preflight {
        println!("lakehousectl: the {skipped} check was skipped, as asked.");
    }
    let failures = run(&facts, &args.ignore_preflight);
    if failures.is_empty() {
        println!("lakehousectl: this host meets the release's minimums.");
        return Ok(());
    }
    for failure in &failures {
        eprintln!("lakehousectl: {}: {}", failure.check, failure.message);
    }
    Err(ToolError::Preflight(failures.len()))
}

/// Gather the facts from this machine.
pub fn collect(compose_text: &str, dir: &Path) -> Facts {
    let overrides = read_env_overrides(&dir.join(".env"));
    let published_ports = published_ports(compose_text, &overrides)
        .into_iter()
        .map(|port| {
            let busy = sys::port_is_busy(port.port).unwrap_or(false);
            PublishedPort { busy, ..port }
        })
        .collect();

    Facts {
        // The binary is built for one platform, so these are the
        // platform it runs on: there is no other host it can run on.
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        docker_version: sys::output("docker", &["version", "--format", "{{.Server.Version}}"]).ok(),
        compose_version: sys::output("docker", &["compose", "version", "--short"]).ok(),
        cpus: std::thread::available_parallelism().map_or(0, std::num::NonZeroUsize::get),
        memory_bytes: memory_bytes(),
        disk_free_bytes: sys::free_bytes(&nearest_existing(dir)).ok(),
        published_ports,
    }
}

/// The nearest directory that exists.
///
/// The install directory is usually not there yet — that is what an
/// install does — and the free disk is the free disk of the
/// filesystem that will hold it.
fn nearest_existing(path: &Path) -> std::path::PathBuf {
    let mut candidate = path;
    while !candidate.exists() {
        match candidate.parent() {
            Some(parent) => candidate = parent,
            None => break,
        }
    }
    candidate.to_path_buf()
}

/// The machine's memory in bytes, from `/proc/meminfo`.
fn memory_bytes() -> Option<u64> {
    let meminfo = std::fs::read_to_string("/proc/meminfo").ok()?;
    for line in meminfo.lines() {
        let Some(rest) = line.strip_prefix("MemTotal:") else {
            continue;
        };
        let kib: u64 = rest.trim().trim_end_matches(" kB").trim().parse().ok()?;
        return Some(kib * 1024);
    }
    None
}

/// The variables an `.env` file sets, as a map.
///
/// Only the lines that assign a value count; a commented-out line and
/// a line without `=` are not settings.
fn read_env_overrides(path: &Path) -> BTreeMap<String, String> {
    let mut overrides = BTreeMap::new();
    let Ok(text) = std::fs::read_to_string(path) else {
        return overrides;
    };
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((name, value)) = line.split_once('=') else {
            continue;
        };
        overrides.insert(
            name.trim().to_string(),
            value
                .trim()
                .trim_matches('"')
                .trim_matches('\'')
                .to_string(),
        );
    }
    overrides
}

/// The ports a Compose file publishes on the host.
///
/// The file is read line by line: a `ports:` key and the list items
/// under it. A port may be a literal (`"8080:8080"`), a variable with
/// a default (`"${API_HOST_PORT:-8080}:8080"`), or a variable without
/// one (`"${API_HOST_PORT}:8080"`) — the last one is only known when
/// the operator's `.env` sets it, and it is skipped when it is not.
///
/// An entry that publishes no host port (`"8080"`) is not a host port
/// and is skipped.
pub fn published_ports(
    compose_text: &str,
    overrides: &BTreeMap<String, String>,
) -> Vec<PublishedPort> {
    let mut ports = Vec::new();
    let mut inside = false;
    let mut key_indent = 0usize;
    for line in compose_text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let indent = line.len() - line.trim_start().len();
        if trimmed == "ports:" {
            inside = true;
            key_indent = indent;
            continue;
        }
        if !inside {
            continue;
        }
        if !trimmed.starts_with('-') {
            // Any key at or above the `ports:` key's own indent ends
            // the block.
            if indent <= key_indent {
                inside = false;
            }
            continue;
        }
        let entry = trimmed
            .trim_start_matches('-')
            .trim()
            .trim_matches('"')
            .trim_matches('\'');
        if let Some(port) = host_port(entry, overrides) {
            ports.push(port);
        }
    }
    ports
}

/// The host port one Compose `ports:` entry publishes.
fn host_port(entry: &str, overrides: &BTreeMap<String, String>) -> Option<PublishedPort> {
    if entry.contains('[') {
        // An IPv6 host address: the fields cannot be split on ':'
        // without knowing where the address ends.
        return None;
    }
    let fields = split_fields(entry);
    if fields.len() < 2 {
        // No host port: the container's port is published nowhere.
        return None;
    }
    let host = fields[fields.len() - 2];
    let (source, value) = resolve_port(host, overrides)?;
    let port: u16 = value.trim().parse().ok()?;
    Some(PublishedPort {
        source,
        port,
        busy: false,
    })
}

/// Split a Compose `ports:` entry into its fields.
///
/// A colon inside a variable's default (`${PORT:-3000}`) is not a field
/// separator, so the split counts the braces.
fn split_fields(entry: &str) -> Vec<&str> {
    let mut fields = Vec::new();
    let mut depth = 0usize;
    let mut start = 0usize;
    for (index, character) in entry.char_indices() {
        match character {
            '{' => depth += 1,
            '}' => depth = depth.saturating_sub(1),
            ':' if depth == 0 => {
                fields.push(&entry[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }
    fields.push(&entry[start..]);
    fields
}

/// The value of one host port field: a literal, or a variable.
fn resolve_port(field: &str, overrides: &BTreeMap<String, String>) -> Option<(String, String)> {
    let field = field.trim();
    if let Some(body) = field
        .strip_prefix("${")
        .and_then(|rest| rest.strip_suffix('}'))
    {
        let (name, default) = match body.split_once(":-") {
            Some((name, default)) => (name, Some(default.to_string())),
            None => (body, None),
        };
        if let Some(value) = overrides.get(name) {
            return Some((name.to_string(), value.clone()));
        }
        let default = default?;
        return Some((name.to_string(), default));
    }
    Some((field.to_string(), field.to_string()))
}

/// A version as its leading numbers, e.g. `24.0.7` or `2.24.0-desktop.1`.
fn parse_version(version: &str) -> Option<(u32, u32)> {
    let mut numbers = version
        .trim()
        .split(|character: char| !character.is_ascii_digit())
        .filter(|part| !part.is_empty())
        .take(2)
        .map(str::parse::<u32>);
    let major = numbers.next()?.ok()?;
    let minor = numbers.next().unwrap_or(Ok(0)).unwrap_or(0);
    Some((major, minor))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn facts() -> Facts {
        Facts {
            os: "linux".to_string(),
            arch: "x86_64".to_string(),
            docker_version: Some("24.0.7".to_string()),
            compose_version: Some("2.24.0".to_string()),
            cpus: 4,
            memory_bytes: Some(16 * GIB),
            disk_free_bytes: Some(50 * GIB),
            published_ports: Vec::new(),
        }
    }

    #[test]
    fn a_host_that_meets_every_minimum_passes_every_check() {
        let failures = run(&facts(), &[]);
        assert!(failures.is_empty(), "unexpected failures: {failures:?}");
    }

    #[test]
    fn a_host_that_is_not_linux_is_refused() {
        let mut facts = facts();
        facts.os = "macos".to_string();
        let failure = check_os(&facts).expect_err("macOS is not supported");
        assert!(failure.contains("Linux"), "the message is {failure}");
    }

    #[test]
    fn a_host_that_is_not_x86_64_is_refused() {
        let mut facts = facts();
        facts.arch = "aarch64".to_string();
        let failure = check_os(&facts).expect_err("arm64 is not out yet");
        assert!(failure.contains("x86_64"), "the message is {failure}");
    }

    #[test]
    fn a_host_without_docker_is_refused() {
        let mut facts = facts();
        facts.docker_version = None;
        let failure = check_docker(&facts).expect_err("no Docker is no install");
        assert!(failure.contains("Docker"), "the message is {failure}");
    }

    #[test]
    fn docker_older_than_24_is_refused() {
        let mut facts = facts();
        facts.docker_version = Some("23.0.6".to_string());
        let failure = check_docker(&facts).expect_err("Docker 23 is too old");
        assert!(failure.contains("24 or newer"), "the message is {failure}");
    }

    #[test]
    fn docker_24_is_enough() {
        let mut facts = facts();
        facts.docker_version = Some("24.0.0".to_string());
        assert!(check_docker(&facts).is_ok());
    }

    #[test]
    fn compose_older_than_2_24_is_refused() {
        let mut facts = facts();
        facts.compose_version = Some("2.23.0".to_string());
        let failure = check_compose(&facts).expect_err("Compose 2.23 is too old");
        assert!(
            failure.contains("2.24 or newer"),
            "the message is {failure}"
        );
    }

    #[test]
    fn compose_2_24_is_enough() {
        let mut facts = facts();
        facts.compose_version = Some("2.24.0-desktop.1".to_string());
        assert!(check_compose(&facts).is_ok());
    }

    #[test]
    fn a_host_with_too_few_cpus_is_refused() {
        let mut facts = facts();
        facts.cpus = 2;
        let failure = check_cpu(&facts).expect_err("two CPUs are too few");
        assert!(failure.contains("4 or more"), "the message is {failure}");
    }

    #[test]
    fn a_host_with_8_gib_of_memory_is_refused() {
        let mut facts = facts();
        facts.memory_bytes = Some(8 * GIB);
        let failure = check_memory(&facts).expect_err("8 GiB is too little");
        assert!(
            failure.contains("16 GiB or more"),
            "the message is {failure}"
        );
    }

    #[test]
    fn a_host_with_too_little_free_disk_is_refused() {
        let mut facts = facts();
        facts.disk_free_bytes = Some(10 * GIB);
        let failure = check_disk(&facts).expect_err("10 GiB is too little");
        assert!(
            failure.contains("50 GiB or more"),
            "the message is {failure}"
        );
    }

    #[test]
    fn a_port_that_is_taken_is_refused() {
        let mut facts = facts();
        facts.published_ports = vec![
            PublishedPort {
                source: "CONSOLE_HOST_PORT".to_string(),
                port: 3000,
                busy: false,
            },
            PublishedPort {
                source: "API_HOST_PORT".to_string(),
                port: 8080,
                busy: true,
            },
        ];
        let failure = check_ports(&facts).expect_err("8080 is taken");
        assert!(failure.contains("8080"), "the message is {failure}");
        assert!(!failure.contains("3000"), "the message is {failure}");
    }

    #[test]
    fn an_ignored_check_does_not_run() {
        let mut facts = facts();
        facts.cpus = 1;
        let failures = run(&facts, &["cpu".to_string()]);
        assert!(failures.is_empty(), "unexpected failures: {failures:?}");
    }

    #[test]
    fn an_unknown_check_name_is_refused() {
        let error =
            validate_ignored(&["colour".to_string()]).expect_err("an unknown name is a mistake");
        assert!(error.contains("colour"), "the message is {error}");
    }

    #[test]
    fn the_compose_file_names_the_ports_it_publishes() {
        let compose = "\
services:
  console:
    ports:
      - \"${CONSOLE_HOST_PORT:-3000}:3000\"
  lakehouse-api:
    ports:
      - \"${API_HOST_PORT:-8080}:8080\"
  postgres:
    ports:
      - \"${POSTGRES_HOST_PORT:-5432}:5432\"
  init:
    ports:
      - \"8080\"
";
        let ports = published_ports(compose, &BTreeMap::new());
        assert_eq!(
            ports,
            vec![
                PublishedPort {
                    source: "CONSOLE_HOST_PORT".to_string(),
                    port: 3000,
                    busy: false,
                },
                PublishedPort {
                    source: "API_HOST_PORT".to_string(),
                    port: 8080,
                    busy: false,
                },
                PublishedPort {
                    source: "POSTGRES_HOST_PORT".to_string(),
                    port: 5432,
                    busy: false,
                },
            ]
        );
    }

    #[test]
    fn the_operators_env_moves_a_published_port() {
        let compose = "\
services:
  console:
    ports:
      - \"${CONSOLE_HOST_PORT:-3000}:3000\"
";
        let overrides = BTreeMap::from([("CONSOLE_HOST_PORT".to_string(), "3100".to_string())]);
        let ports = published_ports(compose, &overrides);
        assert_eq!(ports[0].port, 3100);
        assert_eq!(ports[0].source, "CONSOLE_HOST_PORT");
    }

    #[test]
    fn a_variable_without_a_default_is_skipped_when_nothing_sets_it() {
        let compose = "\
services:
  console:
    ports:
      - \"${CONSOLE_HOST_PORT}:3000\"
";
        assert_eq!(published_ports(compose, &BTreeMap::new()), []);
    }

    #[test]
    fn the_repositorys_own_compose_file_names_the_ports_it_publishes() {
        /// The deployment template the release is rendered from.
        const TEMPLATE: &str = include_str!("../../../../docker-compose.yml");

        let ports = published_ports(TEMPLATE, &BTreeMap::new());
        let numbers: Vec<u16> = ports.iter().map(|port| port.port).collect();
        assert!(
            numbers.contains(&3000),
            "the console's port is not among the published ports: {numbers:?}"
        );
        assert!(
            numbers.contains(&8080),
            "the API's port is not among the published ports: {numbers:?}"
        );
        assert!(
            numbers.contains(&5432),
            "Postgres' port is not among the published ports: {numbers:?}"
        );
    }
}
