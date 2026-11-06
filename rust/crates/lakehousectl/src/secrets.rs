//! The secrets of a fresh install (REL-2 decision D6).
//!
//! # What this module does
//!
//! It writes the install's `.env`: the repository's `.env.example` — the
//! list of keys — with a value generated for every key that has none.
//! It is the Rust port of `ops/init-env.sh`, and it follows the same
//! rules:
//!
//! - the values are generated only when they are missing, and an
//!   existing `.env` is NEVER overwritten: an install that already has
//!   one keeps its values;
//! - the file is written readable by its owner only (mode 0600);
//! - the first administrator's password is generated and shown once, and
//!   it must be changed at first sign-in (`AUTH_BOOTSTRAP_*`).
//!
//! # The values
//!
//! - the passwords and the keys are 24 random bytes as hexadecimal, and
//!   `CONNECTOR_SECRETS_KEY` is 32 random bytes in base64 (SRC-12): it
//!   is the key every stored connector credential is sealed with, and
//!   losing it loses every one of them;
//! - the tenant warehouse pair may equal the `RustFS` pair (ADR 0002
//!   Addendum 2, "may equal, must never alias"), the way
//!   `init-env.sh` fills it.
//!
//! # The profiles
//!
//! The Dagster services carry the `dagster` profile, so a stack that
//! does not run it runs no pipelines at all: no upload loads, no
//! connector runs, no maintenance jobs. A release install runs that
//! profile, and the install's `.env` is where it says so: Compose
//! reads `COMPOSE_PROFILES` from the file it is given with
//! `--env-file` (REL-2 S4b review round 3 BLOCKER 3, decision D22).
//!
//! So a fresh install writes `COMPOSE_PROFILES=dagster`, and the
//! update adds it to an older install's `.env` when the file names no
//! profile. An operator's own value is KEPT — the profiles an
//! operator set are theirs, and the operator adds `trino` or
//! `seaweedfs` there.
//!
//! # The ports the profile needs
//!
//! Turning the profile on needs no port at all. The Dagster webserver
//! is published on the host's loopback only, on port 3001 by default
//! (`127.0.0.1:${DAGSTER_WEBSERVER_HOST_PORT:-3001}:3000` in
//! `docker-compose.yml`), so it does not clash with the console's 3000
//! and it is not reachable from outside the host (REL-2 S4b review
//! round 4, D23). The tool picks no port: it used to look for a free
//! one above the console's, and that logic is gone with the reason it
//! answered — a port the tool chose is a port nobody else can reason
//! about.

use std::fmt::Write as _;
use std::path::Path;

use base64::Engine as _;
use rand::Rng;

/// The template the `.env` is written from.
///
/// The repository's `.env.example` is the list of keys (REL-2 decision
/// D6), so the installer and the deployment read the same list. It is
/// compiled in: the installer has to work on a host that has never seen
/// this repository.
pub const ENV_EXAMPLE: &str = include_str!("../../../../.env.example");

/// The keys `ops/init-env.sh` fills with hexadecimal.
const FILLED_HEX: [&str; 8] = [
    "POSTGRES_PASSWORD",
    "RUSTFS_ACCESS_KEY",
    "RUSTFS_SECRET_KEY",
    "LAKEKEEPER_ENCRYPTION_KEY",
    "SEAWEEDFS_ACCESS_KEY",
    "SEAWEEDFS_SECRET_KEY",
    "AUTH_BOOTSTRAP_PASSWORD",
    "RENDER_TOKEN_SECRET",
];

/// The key that is base64, not hexadecimal (SRC-12).
const FILLED_BASE64: &str = "CONNECTOR_SECRETS_KEY";

/// The tenant warehouse pair, which may equal the `RustFS` pair.
const TENANT_ACCESS_KEY: &str = "TENANT_WAREHOUSE_S3_ACCESS_KEY";
const TENANT_SECRET_KEY: &str = "TENANT_WAREHOUSE_S3_SECRET_KEY";

/// How many bytes the hexadecimal values hold.
const HEX_BYTES: usize = 24;
/// How many bytes the base64 value holds.
const BASE64_BYTES: usize = 32;

/// The variable Compose reads the profiles from.
///
/// The Dagster services carry the `dagster` profile, and Compose reads
/// the profiles to run from the file it is given with `--env-file` —
/// which is the install's own `.env` (REL-2 S4b review round 3
/// BLOCKER 3, decision D22).
pub const PROFILES_VARIABLE: &str = "COMPOSE_PROFILES";

/// The profiles a release install runs.
///
/// The Dagster services carry the `dagster` profile, and a stack that
/// does not run it runs no pipelines at all (REL-2 S4b review round 3
/// BLOCKER 3, decision D22).
pub const RELEASE_PROFILES: &str = "dagster";

/// The tenant that owns the shared catalog and the `Dagster`-job list.
///
/// `0002_seed_identity.sql` seeds the four demo tenants, and
/// `11111111-1111-4111-8111-000000000001` is the group tenant: the
/// first row, the tenant of the seeded identities, and the tenant
/// `0042_tenant_provisioning.sql` gives the seeded connectors and
/// pipelines. `docs/OPERATIONS.md` names it as the owner of a stack
/// that still runs the seeded identities (REL-10).
pub const GROUP_TENANT_ID: &str = "11111111-1111-4111-8111-000000000001";

/// The variable that names the shared catalog's tenant.
///
/// The shared catalog and the `Dagster`-job half of `GET
/// /api/pipelines` have no tenant column, so with more than one tenant
/// and this unset the API refuses both for every principal without the
/// unrestricted grant, and every job with no user — a scheduled
/// export, an alert or a digest, a report, an embed, a public link — is
/// refused with a 422 that names this setting (SEC-50,
/// `.env.example`). An install that leaves it empty runs no scheduled
/// work at all (REL-10).
pub const CATALOG_TENANT_VARIABLE: &str = "CATALOG_TENANT_ID";

/// The variable the console's host port comes from.
///
/// The tool does not set it either — the ports are the operator's and
/// the Compose file's (REL-2 S4b review round 4, D23). The tests read
/// the two names to hold the `.env` to them.
#[cfg(test)]
const CONSOLE_PORT_VARIABLE: &str = "CONSOLE_HOST_PORT";

/// The variable the Dagster webserver's host port comes from.
///
/// The tool does not set it: the webserver is published on the host's
/// loopback only, on port 3001 by default (REL-2 S4b review round 4,
/// D23).
#[cfg(test)]
const DAGSTER_WEBSERVER_PORT_VARIABLE: &str = "DAGSTER_WEBSERVER_HOST_PORT";

/// What a fresh `.env` carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Secrets {
    /// The first administrator's email address.
    pub admin_email: String,
    /// The first administrator's password, when this run generated it.
    ///
    /// `None` when the `.env` was already there: the password in it was
    /// generated by an earlier install, and it is not printed again.
    pub admin_password: Option<String>,
}

/// Write the install's `.env`, if it is not there yet.
///
/// The release's profiles are named in the file either way: a fresh
/// install writes them, and an existing `.env` that names none gets
/// them (REL-2 S4b review round 3 BLOCKER 3, decision D22). The
/// operator's own VALUES are never changed.
///
/// The tool picks no port: the Dagster webserver's host port is the
/// Compose file's own default, on the host's loopback only (REL-2 S4b
/// review round 4, D23).
///
/// # Errors
///
/// Returns the error the operating system gave when the file could not
/// be written.
pub fn write_env(path: &Path) -> Result<Secrets, std::io::Error> {
    if path.exists() {
        // Never overwritten. The mode is still set: a `.env` an
        // operator copied around must not be readable by anyone else.
        set_owner_only(path)?;
        // The administrator's email is the one the file HOLDS, not the
        // template's: the operator may have changed it, and the
        // address the tool prints is the one the stack will use (REL-2
        // S3 review round 1 SHOULD-FIX 4).
        let text = std::fs::read_to_string(path)?;
        // The profiles are added when the file names none: the Dagster
        // services carry the `dagster` profile, and a stack that does
        // not run it runs no pipelines at all (REL-2 S4b review round 3
        // BLOCKER 3, decision D22).
        let with_profiles = with_release_profiles(&text);
        // And the shared catalog's tenant, for the same reason an older
        // install's `.env` gets the profiles: an install made before
        // REL-10 writes `CATALOG_TENANT_ID=` empty, and a stack with
        // more than one tenant refuses every job with no user until it
        // is set. An operator's own value is never changed.
        let with_tenant = with_the_catalog_tenant(&with_profiles);
        if with_tenant != text {
            write_replacing(path, &with_tenant)?;
        }
        return Ok(Secrets {
            admin_email: read_value(&with_tenant, "AUTH_BOOTSTRAP_EMAIL").unwrap_or_default(),
            admin_password: None,
        });
    }

    let mut text = ENV_EXAMPLE.to_string();
    let mut generated: Vec<(String, String)> = Vec::new();
    for name in FILLED_HEX {
        let value = hex_secret();
        if replace_assignment(&mut text, name, &value) {
            generated.push((name.to_string(), value));
        }
    }
    let connector_key = base64_secret();
    if replace_assignment(&mut text, FILLED_BASE64, &connector_key) {
        generated.push((FILLED_BASE64.to_string(), connector_key));
    }

    // The tenant warehouse pair follows the RustFS pair (ADR 0002
    // Addendum 2): it may equal it, and it must never alias it.
    let rustfs_access = generated
        .iter()
        .find(|(name, _)| name == "RUSTFS_ACCESS_KEY")
        .map(|(_, value)| value.clone())
        .or_else(|| read_value(&text, "RUSTFS_ACCESS_KEY"));
    let rustfs_secret = generated
        .iter()
        .find(|(name, _)| name == "RUSTFS_SECRET_KEY")
        .map(|(_, value)| value.clone())
        .or_else(|| read_value(&text, "RUSTFS_SECRET_KEY"));
    if let Some(value) = rustfs_access {
        replace_assignment(&mut text, TENANT_ACCESS_KEY, &value);
    }
    if let Some(value) = rustfs_secret {
        replace_assignment(&mut text, TENANT_SECRET_KEY, &value);
    }

    // The release runs the `dagster` profile, and the template names no
    // profile: the key is added, so the stack the install starts runs
    // the Dagster services (REL-2 S4b review round 3 BLOCKER 3,
    // decision D22).
    let text = with_release_profiles(&text);

    // The shared catalog belongs to the group tenant: the template
    // leaves the key empty, and a stack with more than one tenant
    // refuses every job with no user until it is set (REL-10 D1).
    let text = with_the_catalog_tenant(&text);

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // The file is written through a temporary one, so a half-written
    // `.env` is never left behind, and it is never readable by anyone
    // but its owner.
    write_replacing(path, &text)?;

    let admin_password = generated
        .iter()
        .find(|(name, _)| name == "AUTH_BOOTSTRAP_PASSWORD")
        .map(|(_, value)| value.clone());
    Ok(Secrets {
        admin_email: read_value(&text, "AUTH_BOOTSTRAP_EMAIL").unwrap_or_default(),
        admin_password,
    })
}

/// The `.env` with the release's profiles, when it names none.
///
/// The tool turns the `dagster` profile ON when the file names no
/// profile: the Dagster services carry it, and a stack that does not
/// run it runs no pipelines at all — no upload loads, no connector
/// runs, no maintenance jobs (REL-2 S4b review round 3 BLOCKER 3,
/// decision D22).
///
/// A file that SETS the variable is left alone: the profiles an
/// operator set are theirs, and the operator adds `trino` or
/// `seaweedfs` there. A commented-out line sets nothing, so it does
/// not count.
///
/// The tool picks no port: the Dagster webserver is published on the
/// host's loopback only, on port 3001 by default, so it needs no port
/// of its own (REL-2 S4b review round 4, D23).
fn with_release_profiles(text: &str) -> String {
    let with_profiles = with_the_profiles(text);
    if with_profiles == text {
        // The operator's own profiles: the tool changes nothing.
        return text.to_string();
    }
    with_profiles
}

/// The `.env` with `COMPOSE_PROFILES=dagster`, when it names no
/// profile.
fn with_the_profiles(text: &str) -> String {
    if read_value(text, PROFILES_VARIABLE).is_some() {
        return text.to_string();
    }
    let mut out = text.to_string();
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    let _ = writeln!(out, "{PROFILES_VARIABLE}={RELEASE_PROFILES}");
    out
}

/// The `.env` naming the group tenant, when it names no catalog tenant.
///
/// An install made before REL-10 writes `CATALOG_TENANT_ID=` empty, and
/// the one-step installer's `.env` did not carry the key at all. A
/// stack with more than one tenant refuses the shared catalog, the
/// `Dagster`-job list and every job with no user until the setting
/// names a tenant (SEC-50), so the tool sets it to the group tenant —
/// `0002_seed_identity.sql`'s first row, the tenant of the seeded
/// identities and of the seeded connectors and pipelines
/// (`0042_tenant_provisioning.sql`).
///
/// A file that SETS the variable to any value is left alone: the
/// value is the operator's, and the API refuses a malformed one at
/// startup rather than the tool guessing. A commented-out line sets
/// nothing, so it does not count.
fn with_the_catalog_tenant(text: &str) -> String {
    match read_value(text, CATALOG_TENANT_VARIABLE) {
        Some(value) if !value.trim().is_empty() => text.to_string(),
        Some(_) => {
            // The line is there and empty: the value goes where it is,
            // so the key keeps its place and its comment above it.
            let mut out = text.to_string();
            replace_empty_assignment(&mut out, CATALOG_TENANT_VARIABLE, GROUP_TENANT_ID);
            out
        }
        None => {
            let mut out = text.to_string();
            if !out.is_empty() && !out.ends_with('\n') {
                out.push('\n');
            }
            let _ = writeln!(out, "{CATALOG_TENANT_VARIABLE}={GROUP_TENANT_ID}");
            out
        }
    }
}

/// Replace the empty assignment of a key with a value.
///
/// `replace_assignment` matches the template's exact `NAME=` line, which
/// is what the template writes; an operator's file may carry spaces or
/// quotes around an empty value, and this matches the key instead. A
/// line with a value is not touched.
fn replace_empty_assignment(text: &mut String, name: &str, value: &str) {
    let lines: Vec<String> = text
        .lines()
        .map(|line| {
            let trimmed = line.trim();
            if trimmed.starts_with('#') {
                return line.to_string();
            }
            let Some((key, assigned)) = trimmed.split_once('=') else {
                return line.to_string();
            };
            if key.trim() != name {
                return line.to_string();
            }
            if assigned
                .trim()
                .trim_matches('"')
                .trim_matches('\'')
                .trim()
                .is_empty()
            {
                return format!("{name}={value}");
            }
            line.to_string()
        })
        .collect();
    let mut joined = lines.join("\n");
    if text.ends_with('\n') {
        joined.push('\n');
    }
    *text = joined;
}

/// Make the install's `.env` name the release's profiles.
///
/// An older install's `.env` names none: the Dagster services carry
/// the `dagster` profile, and a stack that does not run it runs no
/// pipelines at all — no upload loads, no connector runs, no
/// maintenance jobs (REL-2 S4b review round 3 BLOCKER 3, decision
/// D22). The key is ADDED when the file names no profile, and an
/// operator's own value is kept.
///
/// The tool picks no port: the Dagster webserver's host port is the
/// Compose file's own default, on the host's loopback only (REL-2 S4b
/// review round 4, D23).
///
/// Returns whether the file was changed.
///
/// # Errors
///
/// Returns the error the operating system gave when the file could not
/// be read or written.
pub fn ensure_release_profiles(dir: &Path) -> Result<bool, std::io::Error> {
    rewrite_env(dir, with_release_profiles)
}

/// Make the install's `.env` name the group tenant as the shared
/// catalog's tenant.
///
/// An install made before REL-10 has `CATALOG_TENANT_ID=` empty:
/// a stack with more than one tenant refuses the shared catalog, the
/// `Dagster`-job list and every job with no user until it is set
/// (SEC-50), and the update sets it BEFORE it starts anything. An
/// operator's own value is kept, valid or not: the API refuses a
/// malformed one at startup, and the tool does not guess (REL-10 D2).
///
/// Returns whether the file was changed.
///
/// # Errors
///
/// Returns the error the operating system gave when the file could not
/// be read or written.
pub fn ensure_catalog_tenant(dir: &Path) -> Result<bool, std::io::Error> {
    rewrite_env(dir, with_the_catalog_tenant)
}

/// Rewrite the install's `.env` with `rewrite`, when it changes it.
///
/// One reader and one writer for both settings, so install and update
/// fill the file the same way (REL-10 D3).
///
/// # Errors
///
/// Returns the error the operating system gave when the file could not
/// be read or written, named with the path.
fn rewrite_env(dir: &Path, rewrite: fn(&str) -> String) -> Result<bool, std::io::Error> {
    let path = dir.join(".env");
    let text = std::fs::read_to_string(&path).map_err(|error| {
        std::io::Error::new(
            error.kind(),
            format!("{} could not be read: {error}", path.display()),
        )
    })?;
    let rewritten = rewrite(&text);
    if rewritten == text {
        return Ok(false);
    }
    write_replacing(&path, &rewritten).map_err(|error| {
        std::io::Error::new(
            error.kind(),
            format!("{} could not be written: {error}", path.display()),
        )
    })?;
    Ok(true)
}

/// The value a `.env` sets for a key.
#[must_use]
pub fn read_value(text: &str, name: &str) -> Option<String> {
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = line.split_once('=')
            && key.trim() == name
        {
            return Some(
                value
                    .trim()
                    .trim_matches('"')
                    .trim_matches('\'')
                    .to_string(),
            );
        }
    }
    None
}

/// Replace an empty assignment with a value.
///
/// Returns whether the assignment was there: a key the template does
/// not carry is not added, so the `.env` holds the template's keys and
/// nothing else.
fn replace_assignment(text: &mut String, name: &str, value: &str) -> bool {
    let prefix = format!("{name}=");
    let mut replaced = false;
    let mut lines: Vec<String> = text
        .lines()
        .map(|line| {
            if line == prefix {
                replaced = true;
                return format!("{prefix}{value}");
            }
            line.to_string()
        })
        .collect();
    if replaced {
        lines.push(String::new());
        *text = lines.join("\n");
    }
    replaced
}

/// A secret as 24 random bytes in hexadecimal.
fn hex_secret() -> String {
    let mut bytes = [0u8; HEX_BYTES];
    rand::rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

/// A secret as 32 random bytes in base64.
fn base64_secret() -> String {
    let mut bytes = [0u8; BASE64_BYTES];
    rand::rng().fill_bytes(&mut bytes);
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// Write a file its owner alone may read.
fn write_owner_only(path: &Path, text: &str) -> Result<(), std::io::Error> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;

    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(text.as_bytes())?;
    file.sync_all()
}

/// Write a file through a temporary one, and put it in place.
///
/// The file is written through a temporary one, so a half-written
/// `.env` is never left behind, and it is never readable by anyone but
/// its owner.
fn write_replacing(path: &Path, text: &str) -> Result<(), std::io::Error> {
    let temporary = path.with_extension("env.new");
    write_owner_only(&temporary, text)?;
    std::fs::rename(&temporary, path)
}

/// Make a file readable by its owner alone.
fn set_owner_only(path: &Path) -> Result<(), std::io::Error> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn every_key_the_template_leaves_empty_is_filled() {
        let mut text = ENV_EXAMPLE.to_string();
        for name in FILLED_HEX {
            assert!(
                replace_assignment(&mut text, name, "value"),
                "the template has no empty {name}"
            );
        }
        assert!(replace_assignment(&mut text, FILLED_BASE64, "value"));
        for name in FILLED_HEX {
            assert_eq!(
                read_value(&text, name).as_deref(),
                Some("value"),
                "{name} was not filled"
            );
        }
    }

    #[test]
    fn a_value_that_is_already_there_is_kept() {
        let mut text = "POSTGRES_PASSWORD=already-there\n".to_string();
        assert!(!replace_assignment(&mut text, "POSTGRES_PASSWORD", "new"));
        assert_eq!(
            read_value(&text, "POSTGRES_PASSWORD").as_deref(),
            Some("already-there")
        );
    }

    #[test]
    fn a_key_the_template_does_not_carry_is_not_added() {
        let mut text = "POSTGRES_PASSWORD=\n".to_string();
        assert!(!replace_assignment(&mut text, "NOT_A_KEY", "value"));
        assert!(!text.contains("NOT_A_KEY"));
    }

    #[test]
    fn the_generated_secrets_differ_from_each_other() {
        assert_ne!(hex_secret(), hex_secret());
        assert_ne!(base64_secret(), base64_secret());
    }

    #[test]
    fn the_connector_key_is_thirty_two_bytes_in_base64() {
        let key = base64_secret();
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(key.as_bytes())
            .expect("the key is base64");
        assert_eq!(bytes.len(), BASE64_BYTES);
    }

    #[test]
    fn a_fresh_env_holds_every_secret_and_the_admin_password() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let path = directory.path().join(".env");
        let secrets = write_env(&path).expect("the .env is written");
        let text = std::fs::read_to_string(&path).expect("the .env is readable");

        for name in FILLED_HEX {
            let value = read_value(&text, name).expect("the key is set");
            assert!(!value.is_empty(), "{name} is empty");
        }
        assert!(read_value(&text, FILLED_BASE64).is_some());
        assert_eq!(
            read_value(&text, TENANT_ACCESS_KEY),
            read_value(&text, "RUSTFS_ACCESS_KEY")
        );
        assert_eq!(
            read_value(&text, TENANT_SECRET_KEY),
            read_value(&text, "RUSTFS_SECRET_KEY")
        );
        assert_eq!(
            secrets.admin_password,
            read_value(&text, "AUTH_BOOTSTRAP_PASSWORD")
        );
        assert!(secrets.admin_password.is_some());
    }

    #[test]
    fn a_second_run_changes_nothing() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let path = directory.path().join(".env");
        let first = write_env(&path).expect("the .env is written");
        let before = std::fs::read_to_string(&path).expect("the .env is readable");
        let second = write_env(&path).expect("the second run works");
        let after = std::fs::read_to_string(&path).expect("the .env is readable");

        assert_eq!(before, after, "the second run changed the .env");
        assert!(
            first.admin_password.is_some(),
            "the first run generated a password"
        );
        assert_eq!(second.admin_password, None, "the password is shown once");
    }

    #[test]
    fn a_fresh_env_takes_the_administrators_email_from_the_template() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let path = directory.path().join(".env");
        let secrets = write_env(&path).expect("the .env is written");
        assert_eq!(
            secrets.admin_email,
            read_value(ENV_EXAMPLE, "AUTH_BOOTSTRAP_EMAIL").unwrap_or_default(),
            "the email of a fresh install is the template's"
        );
    }

    #[test]
    fn the_administrators_email_is_the_one_the_env_holds() {
        // The operator changed it: the address the tool prints is the
        // one the stack will use, not the template's (REL-2 S3 review
        // round 1 SHOULD-FIX 4).
        let directory = tempfile::tempdir().expect("a temporary directory");
        let path = directory.path().join(".env");
        std::fs::write(&path, "AUTH_BOOTSTRAP_EMAIL=operator@example.org\n")
            .expect("the .env is written");

        let secrets = write_env(&path).expect("the .env is read");
        assert_eq!(
            secrets.admin_email, "operator@example.org",
            "the email of the template was shown instead of the one the .env holds"
        );
        assert_eq!(secrets.admin_password, None, "the password is shown once");
    }

    #[test]
    fn the_env_is_readable_by_its_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().expect("a temporary directory");
        let path = directory.path().join(".env");
        write_env(&path).expect("the .env is written");
        let mode = std::fs::metadata(&path)
            .expect("the .env has metadata")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "the .env is mode {mode:o}");
    }

    #[test]
    fn a_fresh_env_runs_the_dagster_profile() {
        // REL-2 S4b review round 3 BLOCKER 3, decision D22: the Dagster
        // services carry the `dagster` profile, so a stack that does not
        // run it runs no pipelines at all.
        let directory = tempfile::tempdir().expect("a temporary directory");
        let path = directory.path().join(".env");

        write_env(&path).expect("the .env is written");

        let text = std::fs::read_to_string(&path).expect("the .env is readable");
        assert_eq!(
            read_value(&text, PROFILES_VARIABLE).as_deref(),
            Some(RELEASE_PROFILES),
            "the .env names no profile: {text}"
        );
    }

    #[test]
    fn an_env_that_names_no_profile_gets_the_releases_one() {
        // An older install's `.env` names no profile: the install adds
        // it, and the operator's own values are kept (REL-2 S4b review
        // round 3 BLOCKER 3, decision D22).
        let directory = tempfile::tempdir().expect("a temporary directory");
        let path = directory.path().join(".env");
        std::fs::write(&path, "POSTGRES_PASSWORD=secret\n").expect("the .env is written");

        write_env(&path).expect("the .env is read");

        let text = std::fs::read_to_string(&path).expect("the .env is readable");
        assert_eq!(
            read_value(&text, PROFILES_VARIABLE).as_deref(),
            Some(RELEASE_PROFILES),
            "the .env names no profile: {text}"
        );
        assert!(
            text.contains("POSTGRES_PASSWORD=secret"),
            "the operator's own values were lost: {text}"
        );
    }

    #[test]
    fn the_profiles_an_operator_set_are_kept() {
        // REL-2 S4b review round 3 BLOCKER 3, decision D22: "The
        // operator adds `trino` or `seaweedfs` there."
        let directory = tempfile::tempdir().expect("a temporary directory");
        let path = directory.path().join(".env");
        std::fs::write(&path, "COMPOSE_PROFILES=dagster,trino\n").expect("the .env is written");

        write_env(&path).expect("the .env is read");

        let text = std::fs::read_to_string(&path).expect("the .env is readable");
        assert_eq!(
            read_value(&text, PROFILES_VARIABLE).as_deref(),
            Some("dagster,trino"),
            "the operator's own profiles were replaced: {text}"
        );
    }

    #[test]
    fn a_commented_out_profile_is_not_a_profile() {
        // A comment sets nothing, so the install adds the key beside it.
        let directory = tempfile::tempdir().expect("a temporary directory");
        let path = directory.path().join(".env");
        std::fs::write(&path, "# COMPOSE_PROFILES=trino\n").expect("the .env is written");

        write_env(&path).expect("the .env is read");

        let text = std::fs::read_to_string(&path).expect("the .env is readable");
        assert_eq!(
            read_value(&text, PROFILES_VARIABLE).as_deref(),
            Some(RELEASE_PROFILES),
            "the .env names no profile: {text}"
        );
        assert!(
            text.contains("# COMPOSE_PROFILES=trino"),
            "a comment was removed: {text}"
        );
    }

    #[test]
    fn a_fresh_env_names_the_group_tenant_as_the_catalogs_tenant() {
        // REL-10 D1: the shared catalog belongs to the group tenant, so
        // an installed stack runs its scheduled work without an
        // operator editing the `.env` by hand.
        let directory = tempfile::tempdir().expect("a temporary directory");
        let path = directory.path().join(".env");

        write_env(&path).expect("the .env is written");

        let text = std::fs::read_to_string(&path).expect("the .env is readable");
        assert_eq!(
            read_value(&text, CATALOG_TENANT_VARIABLE).as_deref(),
            Some(GROUP_TENANT_ID),
            "the .env names no catalog tenant: {text}"
        );
    }

    #[test]
    fn an_env_with_an_empty_catalog_tenant_gets_the_group_tenant() {
        // REL-10 D1/D3: the template and a 0.2.0 install write
        // `CATALOG_TENANT_ID=` empty.
        let directory = tempfile::tempdir().expect("a temporary directory");
        let path = directory.path().join(".env");
        std::fs::write(&path, "POSTGRES_PASSWORD=secret\nCATALOG_TENANT_ID=\n")
            .expect("the .env is written");

        write_env(&path).expect("the .env is read");

        let text = std::fs::read_to_string(&path).expect("the .env is readable");
        assert_eq!(
            read_value(&text, CATALOG_TENANT_VARIABLE).as_deref(),
            Some(GROUP_TENANT_ID),
            "the empty catalog tenant was not filled: {text}"
        );
        assert!(
            text.contains("POSTGRES_PASSWORD=secret"),
            "the operator's own values were lost: {text}"
        );
    }

    #[test]
    fn the_catalog_tenant_an_operator_set_is_kept() {
        // REL-10 D2: "A line with any value, valid or not, is the
        // operator's and is left alone." The file must come back
        // byte-for-byte unchanged.
        for value in [
            "22222222-2222-4222-8222-000000000002",
            "not-a-uuid",
            "\"11111111-1111-4111-8111-000000000003\"",
        ] {
            let directory = tempfile::tempdir().expect("a temporary directory");
            let path = directory.path().join(".env");
            // The profiles are already named, so the only thing this
            // run could change is the catalog tenant.
            let before = format!(
                "POSTGRES_PASSWORD=secret\nCATALOG_TENANT_ID={value}\nCOMPOSE_PROFILES={RELEASE_PROFILES}\n"
            );
            std::fs::write(&path, &before).expect("the .env is written");

            write_env(&path).expect("the .env is read");

            let after = std::fs::read_to_string(&path).expect("the .env is readable");
            assert_eq!(
                after, before,
                "an operator's catalog tenant ({value}) was changed"
            );
        }
    }

    #[test]
    fn an_env_with_no_catalog_tenant_line_gets_it() {
        // REL-10 D2: the key is added when the line is missing
        // entirely, the way the profiles are.
        let directory = tempfile::tempdir().expect("a temporary directory");
        let path = directory.path().join(".env");
        std::fs::write(&path, "POSTGRES_PASSWORD=secret\n").expect("the .env is written");

        write_env(&path).expect("the .env is read");

        let text = std::fs::read_to_string(&path).expect("the .env is readable");
        assert_eq!(
            read_value(&text, CATALOG_TENANT_VARIABLE).as_deref(),
            Some(GROUP_TENANT_ID),
            "the .env has no catalog tenant line: {text}"
        );
    }

    #[test]
    fn a_commented_out_catalog_tenant_is_not_a_tenant() {
        // A comment sets nothing: the key is added beside it, the way
        // the profiles rule says.
        let directory = tempfile::tempdir().expect("a temporary directory");
        let path = directory.path().join(".env");
        std::fs::write(
            &path,
            "# CATALOG_TENANT_ID=22222222-2222-4222-8222-000000000002\n",
        )
        .expect("the .env is written");

        write_env(&path).expect("the .env is read");

        let text = std::fs::read_to_string(&path).expect("the .env is readable");
        assert_eq!(
            read_value(&text, CATALOG_TENANT_VARIABLE).as_deref(),
            Some(GROUP_TENANT_ID),
            "the .env names no catalog tenant: {text}"
        );
        assert!(
            text.contains("# CATALOG_TENANT_ID=22222222-2222-4222-8222-000000000002"),
            "a comment was removed: {text}"
        );
    }

    #[test]
    fn the_update_fills_an_empty_catalog_tenant() {
        // REL-10 D2: the update fills the line before it starts
        // anything, and reports that it changed the file.
        let directory = tempfile::tempdir().expect("a temporary directory");
        std::fs::write(
            directory.path().join(".env"),
            "POSTGRES_PASSWORD=secret\nCATALOG_TENANT_ID=\n",
        )
        .expect("the .env is written");

        let changed = ensure_catalog_tenant(directory.path()).expect("the tenant is set");

        assert!(changed, "the .env was not changed");
        let text =
            std::fs::read_to_string(directory.path().join(".env")).expect("the .env is readable");
        assert_eq!(
            read_value(&text, CATALOG_TENANT_VARIABLE).as_deref(),
            Some(GROUP_TENANT_ID),
            "the empty catalog tenant was not filled: {text}"
        );
    }

    #[test]
    fn the_update_keeps_the_catalog_tenant_an_operator_set() {
        // REL-10 D2: a value the operator set is never changed.
        let directory = tempfile::tempdir().expect("a temporary directory");
        let before = "CATALOG_TENANT_ID=22222222-2222-4222-8222-000000000002\n";
        std::fs::write(directory.path().join(".env"), before).expect("the .env is written");

        let changed = ensure_catalog_tenant(directory.path()).expect("the tenant is set");

        assert!(!changed, "the .env was changed");
        let text =
            std::fs::read_to_string(directory.path().join(".env")).expect("the .env is readable");
        assert_eq!(text, before, "an operator's catalog tenant was changed");
    }

    #[test]
    fn the_update_adds_the_profile_to_an_older_install() {
        // REL-2 S4b review round 3 BLOCKER 3, decision D22: "the update
        // adds it when an older install's `.env` has no
        // `COMPOSE_PROFILES`".
        let directory = tempfile::tempdir().expect("a temporary directory");
        std::fs::write(directory.path().join(".env"), "POSTGRES_PASSWORD=secret\n")
            .expect("the .env is written");

        let changed = ensure_release_profiles(directory.path()).expect("the profiles are set");

        assert!(changed, "the .env was not changed");
        let text =
            std::fs::read_to_string(directory.path().join(".env")).expect("the .env is readable");
        assert_eq!(
            read_value(&text, PROFILES_VARIABLE).as_deref(),
            Some(RELEASE_PROFILES),
            "the .env names no profile: {text}"
        );
    }

    #[test]
    fn the_update_keeps_the_profiles_an_operator_set() {
        // The operator's own value is kept, and the file is not
        // touched at all (REL-2 S4b review round 3 BLOCKER 3, decision
        // D22).
        let directory = tempfile::tempdir().expect("a temporary directory");
        std::fs::write(
            directory.path().join(".env"),
            "COMPOSE_PROFILES=dagster,seaweedfs\n",
        )
        .expect("the .env is written");

        let changed = ensure_release_profiles(directory.path()).expect("the profiles are set");

        assert!(!changed, "the .env was changed");
        let text =
            std::fs::read_to_string(directory.path().join(".env")).expect("the .env is readable");
        assert_eq!(
            read_value(&text, PROFILES_VARIABLE).as_deref(),
            Some("dagster,seaweedfs"),
            "the operator's own profiles were replaced: {text}"
        );
    }

    #[test]
    fn the_tool_picks_no_port_for_the_dagster_webserver() {
        // REL-2 S4b review round 4, D23: the webserver is published on
        // the host's loopback only, on port 3001 by default, so it needs
        // no port of its own and the tool chooses none. The file holds
        // the template's value, and the console keeps 3000.
        let directory = tempfile::tempdir().expect("a temporary directory");
        let path = directory.path().join(".env");

        write_env(&path).expect("the .env is written");

        let text = std::fs::read_to_string(&path).expect("the .env is readable");
        assert_eq!(
            read_value(&text, DAGSTER_WEBSERVER_PORT_VARIABLE).as_deref(),
            Some("3001"),
            "the webserver's port is not the template's: {text}"
        );
        assert_eq!(
            read_value(&text, CONSOLE_PORT_VARIABLE).as_deref(),
            Some("3000"),
            "the console's port changed: {text}"
        );
    }

    #[test]
    fn the_tool_adds_no_port_for_the_dagster_webserver() {
        // REL-2 S4b review round 4, D23: the tool picks no port. An
        // `.env` that names no webserver port gets none — the Compose
        // file's own default is the port, and a port the tool chose is a
        // port nobody else can reason about.
        let directory = tempfile::tempdir().expect("a temporary directory");
        let path = directory.path().join(".env");
        std::fs::write(&path, "CONSOLE_HOST_PORT=3000\n").expect("the .env is written");

        write_env(&path).expect("the .env is read");

        let text = std::fs::read_to_string(&path).expect("the .env is readable");
        assert_eq!(
            read_value(&text, DAGSTER_WEBSERVER_PORT_VARIABLE),
            None,
            "the tool picked a port: {text}"
        );
    }

    #[test]
    fn the_dagster_webserver_port_an_operator_set_is_kept() {
        // The operator's own value is theirs: the tool changes no port.
        let directory = tempfile::tempdir().expect("a temporary directory");
        let path = directory.path().join(".env");
        std::fs::write(
            &path,
            "CONSOLE_HOST_PORT=3000\nDAGSTER_WEBSERVER_HOST_PORT=3100\n",
        )
        .expect("the .env is written");

        write_env(&path).expect("the .env is read");

        let text = std::fs::read_to_string(&path).expect("the .env is readable");
        assert_eq!(
            read_value(&text, DAGSTER_WEBSERVER_PORT_VARIABLE).as_deref(),
            Some("3100"),
            "the operator's own port was replaced: {text}"
        );
    }

    #[test]
    fn the_profiles_the_update_adds_are_readable_by_the_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().expect("a temporary directory");
        std::fs::write(directory.path().join(".env"), "POSTGRES_PASSWORD=secret\n")
            .expect("the .env is written");

        ensure_release_profiles(directory.path()).expect("the profiles are set");

        let mode = std::fs::metadata(directory.path().join(".env"))
            .expect("the .env has metadata")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "the .env is mode {mode:o}");
    }

    #[test]
    fn an_install_without_an_env_file_cannot_set_the_profiles() {
        // The update fails closed: a stack without the Dagster services
        // runs no pipelines at all, so it must not be started as if it
        // did (REL-2 S4b review round 3 BLOCKER 3, decision D22).
        let directory = tempfile::tempdir().expect("a temporary directory");
        let error = ensure_release_profiles(directory.path())
            .expect_err("an install with no .env file is an error");
        assert!(
            error.to_string().contains(".env"),
            "the error says what failed: {error}"
        );
    }
}
