//! `lakehousectl install`: the whole stack, in one command (REL-2 S3-T3).
//!
//! # What happens, in order
//!
//! 1. **The release is obtained.** Offline, from the bundle the
//!    operator carried in; online, from the GitHub release.
//! 2. **The manifest is verified** against the key compiled into this
//!    binary (REL-2 decision D3). Nothing else is trusted before that.
//! 3. **The Compose file is checked** against the checksum the signed
//!    manifest carries.
//! 4. **The pre-flight runs** (REL-2 S3-T2): a host below the release's
//!    minimums is refused, with what to fix.
//! 5. **The layout is written** (REL-2 decision D5): the Compose file,
//!    the `.env` (mode 0600), the manifest, its signature, `backups/`
//!    and `state.json` — one directory, `/opt/rantai-lakehouse` by
//!    default. Every OTHER file the release ships is written next to
//!    them, at the relative path the release ships it at (REL-2 S4
//!    decision D14): the relative bind-mount sources of the Compose
//!    file, and the two tools under `tools/`.
//! 6. **The images are fetched.** Online, by digest; offline, from the
//!    bundle's `docker save` files, each one's checksum checked against
//!    the bundle's list and its ID against the signed manifest.
//! 7. **The stack starts**, and the tool waits for `/health/ready` to
//!    report the release's own version, and for the console to answer.
//! 8. **The console's address and the first administrator's sign-in
//!    are printed.** The password is shown once and must be changed at
//!    first sign-in.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::cli::InstallArgs;
use crate::docker;
use crate::error::ToolError;
use crate::files;
use crate::http;
use crate::images;
use crate::manifest::Manifest;
use crate::preflight;
use crate::release;
use crate::secrets;
use crate::state;
use crate::sys;
use crate::verify;

/// How long the stack may take to become ready.
const READY_TIMEOUT: Duration = Duration::from_secs(600);
/// How long the console may take to answer.
const CONSOLE_TIMEOUT: Duration = Duration::from_secs(120);
/// How long the tool waits between two probes.
const PROBE_INTERVAL: Duration = Duration::from_secs(5);
/// The API's port when the `.env` does not say.
const DEFAULT_API_PORT: u16 = 8080;
/// The console's port when the `.env` does not say.
const DEFAULT_CONSOLE_PORT: u16 = 3000;

/// Run `lakehousectl install`.
///
/// # Errors
///
/// Returns the error that stopped the install. Every one of them says
/// what failed; the ones that come from the machine say what to fix.
pub fn command(args: &InstallArgs) -> Result<(), ToolError> {
    preflight::validate_ignored(&args.ignore_preflight).map_err(ToolError::Usage)?;

    let release = release::obtain(
        args.bundle.as_deref(),
        args.version.as_deref(),
        &mut release::download,
    )?;
    let key = verify::public_key()?;
    verify::verify_manifest(&release.manifest, &release.bundle, &key)?;
    println!(
        "lakehousectl: the manifest of release {} verifies against the release key.",
        release::version_of(&release.manifest)?
    );

    let manifest: Manifest = serde_json::from_slice(&release.manifest)
        .map_err(|error| ToolError::Failed(format!("the manifest could not be read: {error}")))?;

    files::check_compose(&manifest, &release.compose)?;

    // The files the release ships (REL-2 S4 decision D14): the relative
    // bind-mount sources of the release's Compose file, and the two
    // tools under `tools/`. They are fetched (or read from the
    // bundle) and checked against the SIGNED manifest BEFORE anything
    // is written — a file that does not match is a refusal, not a
    // warning.
    let shipped = files::fetch(&manifest, release.bundle_dir.as_deref(), &mut |url| {
        release::download(url)
    })?;

    check_preflight(args, &release.compose)?;

    let secrets = write_layout(&args.dir, &release, &manifest, &shipped)?;
    let env = crate::env::read(&args.dir.join(".env"));

    images::fetch(&manifest, release.bundle_dir.as_deref())?;
    start(&args.dir, &manifest, &env)?;

    print_sign_in(&args.dir, &secrets, &env);
    Ok(())
}

/// Run the pre-flight checks.
fn check_preflight(args: &InstallArgs, compose: &str) -> Result<(), ToolError> {
    for skipped in &args.ignore_preflight {
        println!("lakehousectl: the {skipped} check was skipped, as asked.");
    }
    let facts = preflight::collect(compose, &args.dir);
    // The image-store check runs for a BUNDLE install only: an online
    // install pulls every image by its digest, and a pull registers the
    // digest on either store (REL-2 S3 D13).
    let failures = preflight::run(&facts, &args.ignore_preflight, args.bundle.is_some());
    if failures.is_empty() {
        return Ok(());
    }
    for failure in &failures {
        eprintln!("lakehousectl: {}: {}", failure.check, failure.message);
    }
    Err(ToolError::Preflight(failures.len()))
}

/// Write the install's layout (REL-2 decision D5).
fn write_layout(
    dir: &Path,
    release: &release::Release,
    manifest: &Manifest,
    shipped: &files::ShippedFiles,
) -> Result<secrets::Secrets, ToolError> {
    // The state of an earlier install is not overwritten: the same
    // release may be installed again, and any other release is refused
    // — that is what `lakehousectl update` is for (REL-2 S3 review
    // round 1 SHOULD-FIX 3).
    state::check_installed(dir, &manifest.version)?;

    std::fs::create_dir_all(dir).map_err(|source| ToolError::Io {
        context: format!("{} could not be created", dir.display()),
        source,
    })?;

    let backups = dir.join("backups");
    std::fs::create_dir_all(&backups).map_err(|source| ToolError::Io {
        context: format!("{} could not be created", backups.display()),
        source,
    })?;
    set_mode(&backups, 0o700)?;

    // The release's own files are written again: they are the bytes of
    // the release, checked against the signed manifest, so a re-install
    // of the same release writes the same bytes.
    write(&dir.join("docker-compose.yml"), release.compose.as_bytes())?;
    write(&dir.join("release.json"), &release.manifest)?;
    write(&dir.join("release.json.sigstore.json"), &release.bundle)?;

    // Every other file the release ships (REL-2 S4 decision D14): the
    // relative bind-mount sources of the Compose file, and the two
    // tools under `tools/`. Each is written at the relative path the
    // release ships it at, so a mount finds its file and
    // `lakehousectl backup` finds its script.
    files::write(dir, shipped)?;

    // The state of the FIRST install is kept: it names the release that
    // is installed, the one before it, and the last backup. A
    // re-install of the same release changes nothing of it.
    state::write_initial(dir, &manifest.version)?;

    secrets::write_env(&dir.join(".env"), sys::port_is_free).map_err(|source| ToolError::Io {
        context: format!("{} could not be written", dir.join(".env").display()),
        source,
    })
}

/// Start the stack and wait for it.
fn start(dir: &Path, manifest: &Manifest, env: &BTreeMap<String, String>) -> Result<(), ToolError> {
    println!("lakehousectl: starting the stack.");
    docker::compose_up(dir, &dir.join(".env")).map_err(ToolError::Failed)?;

    let api_port = crate::env::port(env, "API_HOST_PORT", DEFAULT_API_PORT);
    let console_port = crate::env::port(env, "CONSOLE_HOST_PORT", DEFAULT_CONSOLE_PORT);

    wait_for_ready(api_port, &manifest.version)?;
    wait_for_console(console_port)?;
    Ok(())
}

/// Wait for the API to report the release's own version as ready.
fn wait_for_ready(port: u16, version: &str) -> Result<(), ToolError> {
    let deadline = Instant::now() + READY_TIMEOUT;
    let mut last = "the API did not answer".to_string();
    while Instant::now() < deadline {
        match http::get("127.0.0.1", port, "/health/ready") {
            Ok(response) if response.status == 200 => {
                match serde_json::from_str::<Value>(&response.body) {
                    Ok(document) => {
                        let ready = document["ready"] == Value::Bool(true);
                        let reported = document["version"].as_str().unwrap_or_default();
                        if ready && reported == version {
                            println!("lakehousectl: the API is ready and runs {version}.");
                            return Ok(());
                        }
                        last = format!(
                            "the API answered but is not ready (ready: {ready}, version: {reported})"
                        );
                    }
                    Err(error) => last = format!("the API's answer is not JSON: {error}"),
                }
            }
            Ok(response) => last = format!("the API answered {}", response.status),
            Err(error) => last = error.to_string(),
        }
        std::thread::sleep(PROBE_INTERVAL);
    }
    Err(ToolError::Failed(format!(
        "the stack did not become ready within {} minutes: {last}",
        READY_TIMEOUT.as_secs() / 60
    )))
}

/// Wait for the console to answer.
fn wait_for_console(port: u16) -> Result<(), ToolError> {
    let deadline = Instant::now() + CONSOLE_TIMEOUT;
    let mut last = "the console did not answer".to_string();
    while Instant::now() < deadline {
        match http::get("127.0.0.1", port, "/") {
            Ok(response) => {
                if response.status == 200 {
                    println!("lakehousectl: the console is answering.");
                    return Ok(());
                }
                last = format!("the console answered {}", response.status);
            }
            Err(error) => last = error.to_string(),
        }
        std::thread::sleep(PROBE_INTERVAL);
    }
    Err(ToolError::Failed(format!(
        "the console did not answer within {} minutes: {last}",
        CONSOLE_TIMEOUT.as_secs() / 60
    )))
}

/// Print where the console is, and the first administrator's sign-in.
fn print_sign_in(dir: &Path, secrets: &secrets::Secrets, env: &BTreeMap<String, String>) {
    let console_port = crate::env::port(env, "CONSOLE_HOST_PORT", DEFAULT_CONSOLE_PORT);
    println!();
    println!("lakehousectl: the console is at http://<this host>:{console_port}");
    println!(
        "lakehousectl: the first administrator is {}.",
        secrets.admin_email
    );
    match &secrets.admin_password {
        Some(password) => {
            println!("lakehousectl: its password is shown once, and it must be changed at first");
            println!("lakehousectl: sign-in:");
            println!("  {password}");
        }
        None => {
            println!(
                "lakehousectl: its password was set by an earlier install and was not changed. It \
                 is in {}.",
                dir.join(".env").display()
            );
        }
    }
    println!();
    println!(
        "lakehousectl: the settings are in {}.",
        dir.join(".env").display()
    );
}

/// Write a file.
fn write(path: &Path, bytes: &[u8]) -> Result<(), ToolError> {
    std::fs::write(path, bytes).map_err(|source| ToolError::Io {
        context: format!("{} could not be written", path.display()),
        source,
    })
}

/// Set a file's mode.
fn set_mode(path: &Path, mode: u32) -> Result<(), ToolError> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).map_err(|source| {
        ToolError::Io {
            context: format!("{} could not be given the mode {mode:o}", path.display()),
            source,
        }
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    /// A release of a version, as the install sees it.
    fn release_of(version: &str) -> release::Release {
        release::Release {
            manifest: format!("{{\"version\":\"{version}\"}}").into_bytes(),
            bundle: b"{}".to_vec(),
            compose: "services: {}\n".to_string(),
            bundle_dir: None,
        }
    }

    /// A manifest of a version.
    fn manifest_of(version: &str) -> Manifest {
        Manifest {
            version: version.to_string(),
            min_from: None,
            stops: Vec::new(),
            images: BTreeMap::new(),
            image_ids: BTreeMap::new(),
            files: BTreeMap::new(),
        }
    }

    #[test]
    fn the_install_writes_every_file_the_release_ships() {
        // REL-2 S4 decision D14: the relative bind-mount sources of the
        // release's Compose file, and the two tools under `tools/`. Each
        // is written at the relative path the release ships it at, so a
        // mount finds its file and `lakehousectl backup` finds its
        // script.
        let directory = tempfile::tempdir().expect("a temporary directory");
        let mut shipped: files::ShippedFiles = std::collections::BTreeMap::new();
        shipped.insert(
            "ops/backups/clickhouse/backups.xml".to_string(),
            b"<clickhouse/>".to_vec(),
        );
        shipped.insert("tools/backup-local.sh".to_string(), b"#!/bin/sh\n".to_vec());

        write_layout(
            directory.path(),
            &release_of("0.2.0"),
            &manifest_of("0.2.0"),
            &shipped,
        )
        .expect("the install writes the layout");

        assert_eq!(
            std::fs::read(directory.path().join("ops/backups/clickhouse/backups.xml"))
                .expect("the mounted file is there"),
            b"<clickhouse/>".to_vec(),
            "the mounted file was not written at its own relative path"
        );
        assert!(
            directory.path().join("tools/backup-local.sh").exists(),
            "the tool was not written under tools/"
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
        std::fs::write(directory.path().join("state.json"), state).expect("the state is written");

        write_layout(
            directory.path(),
            &release_of("0.2.0"),
            &manifest_of("0.2.0"),
            &files::ShippedFiles::new(),
        )
        .expect("the same release installs again");

        let after = std::fs::read_to_string(directory.path().join("state.json"))
            .expect("the state is readable");
        assert_eq!(after, state, "the state of the first install was rewritten");
    }

    #[test]
    fn an_install_of_another_release_is_refused_and_points_at_the_update() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        std::fs::write(
            directory.path().join("state.json"),
            "{\"schemaVersion\": 1, \"currentVersion\": \"0.1.0\"}\n",
        )
        .expect("the state is written");

        let error = write_layout(
            directory.path(),
            &release_of("0.2.0"),
            &manifest_of("0.2.0"),
            &files::ShippedFiles::new(),
        )
        .expect_err("another release is refused");
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
        std::fs::write(directory.path().join("state.json"), "not json\n")
            .expect("the state is written");

        let error = write_layout(
            directory.path(),
            &release_of("0.2.0"),
            &manifest_of("0.2.0"),
            &files::ShippedFiles::new(),
        )
        .expect_err("a state that names no release is refused");
        assert!(
            error.to_string().contains("state.json"),
            "the error says what failed: {error}"
        );
        assert_eq!(
            std::fs::read_to_string(directory.path().join("state.json"))
                .expect("the state is read"),
            "not json\n",
            "the state was overwritten"
        );
    }
}
