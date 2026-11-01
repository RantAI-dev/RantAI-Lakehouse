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
//!    default.
//! 6. **The images are fetched.** Online, by digest; offline, from the
//!    bundle's `docker save` files, each one's ID checked against the
//!    signed manifest.
//! 7. **The stack starts**, and the tool waits for `/health/ready` to
//!    report the release's own version, and for the console to answer.
//! 8. **The console's address and the first administrator's sign-in
//!    are printed.** The password is shown once and must be changed at
//!    first sign-in.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::cli::InstallArgs;
use crate::docker;
use crate::error::ToolError;
use crate::http;
use crate::manifest::{Manifest, image_name};
use crate::preflight;
use crate::secrets;
use crate::sys;
use crate::verify;

/// Where a release's assets live.
const RELEASES_URL: &str = "https://github.com/RantAI-dev/RantAI-Lakehouse/releases/download";
/// Where the newest release is named.
const LATEST_URL: &str = "https://api.github.com/repos/RantAI-dev/RantAI-Lakehouse/releases/latest";
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

/// The release the install is about.
struct Release {
    /// The manifest's bytes.
    manifest: Vec<u8>,
    /// The manifest's Sigstore bundle.
    bundle: Vec<u8>,
    /// The release's Compose file.
    compose: String,
    /// The bundle's directory, when the install is offline.
    bundle_dir: Option<PathBuf>,
}

/// Run `lakehousectl install`.
///
/// # Errors
///
/// Returns the error that stopped the install. Every one of them says
/// what failed; the ones that come from the machine say what to fix.
pub fn command(args: &InstallArgs) -> Result<(), ToolError> {
    preflight::validate_ignored(&args.ignore_preflight).map_err(ToolError::Usage)?;

    let release = obtain(args)?;
    let key = verify::public_key()?;
    verify::verify_manifest(&release.manifest, &release.bundle, &key)?;
    println!(
        "lakehousectl: the manifest of release {} verifies against the release key.",
        version_of(&release.manifest)?
    );

    let manifest: Manifest = serde_json::from_slice(&release.manifest)
        .map_err(|error| ToolError::Failed(format!("the manifest could not be read: {error}")))?;

    check_compose(&manifest, &release.compose)?;
    check_preflight(args, &release.compose)?;

    let secrets = write_layout(&args.dir, &release, &manifest)?;
    let env = read_env(&args.dir.join(".env"));

    fetch_images(&manifest, release.bundle_dir.as_deref())?;
    start(&args.dir, &manifest, &env)?;

    print_sign_in(&args.dir, &secrets, &env);
    Ok(())
}

/// The version the manifest names.
fn version_of(manifest: &[u8]) -> Result<String, ToolError> {
    let document: Value = serde_json::from_slice(manifest)
        .map_err(|error| ToolError::Failed(format!("the manifest could not be read: {error}")))?;
    document["version"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| ToolError::Failed("the manifest names no version".to_string()))
}

/// The release, from the bundle or from GitHub.
fn obtain(args: &InstallArgs) -> Result<Release, ToolError> {
    if let Some(directory) = &args.bundle {
        let manifest = read(&directory.join("release.json"))?;
        let bundle = read(&directory.join("release.json.sigstore.json"))?;
        let compose = read_to_string(&directory.join("docker-compose.yml"))?;
        return Ok(Release {
            manifest,
            bundle,
            compose,
            bundle_dir: Some(directory.clone()),
        });
    }

    let version = match &args.version {
        Some(version) => version.trim_start_matches('v').to_string(),
        None => latest_version()?,
    };
    let directory = std::env::temp_dir().join(format!("lakehousectl-release-{version}"));
    std::fs::create_dir_all(&directory).map_err(|source| ToolError::Io {
        context: format!("{} could not be created", directory.display()),
        source,
    })?;
    let manifest = download(
        &format!("{RELEASES_URL}/v{version}/release.json"),
        &directory.join("release.json"),
    )?;
    let bundle = download(
        &format!("{RELEASES_URL}/v{version}/release.json.sigstore.json"),
        &directory.join("release.json.sigstore.json"),
    )?;
    let compose = String::from_utf8(download(
        &format!("{RELEASES_URL}/v{version}/docker-compose.yml"),
        &directory.join("docker-compose.yml"),
    )?)
    .map_err(|error| {
        ToolError::Failed(format!("the release's Compose file is not text: {error}"))
    })?;
    Ok(Release {
        manifest,
        bundle,
        compose,
        bundle_dir: None,
    })
}

/// The version of the newest release.
fn latest_version() -> Result<String, ToolError> {
    let body = sys::output(
        "curl",
        &[
            "--fail",
            "--silent",
            "--show-error",
            "--location",
            LATEST_URL,
        ],
    )
    .map_err(|error| {
        ToolError::Failed(format!(
            "the newest release could not be read from {LATEST_URL}: {error}. Is this host \
                 online, and is `curl` installed?"
        ))
    })?;
    let document: Value = serde_json::from_str(&body).map_err(|error| {
        ToolError::Failed(format!("the newest release could not be read: {error}"))
    })?;
    let tag = document["tag_name"]
        .as_str()
        .ok_or_else(|| ToolError::Failed("the newest release names no tag".to_string()))?;
    Ok(tag.trim_start_matches('v').to_string())
}

/// Download one file of a release.
fn download(url: &str, path: &Path) -> Result<Vec<u8>, ToolError> {
    let target = path.to_string_lossy().to_string();
    sys::output(
        "curl",
        &[
            "--fail",
            "--silent",
            "--show-error",
            "--location",
            "--output",
            &target,
            url,
        ],
    )
    .map_err(|error| {
        ToolError::Failed(format!(
            "{url} could not be downloaded: {error}. Is this host online, and is `curl` installed?"
        ))
    })?;
    read(path)
}

/// Check the Compose file against the checksum the signed manifest carries.
fn check_compose(manifest: &Manifest, compose: &str) -> Result<(), ToolError> {
    let Some(expected) = manifest.files.get("docker-compose.yml") else {
        return Err(ToolError::Failed(
            "the manifest names no checksum for the Compose file. A release that does not say \
             which Compose file it ships is a release that cannot be installed."
                .to_string(),
        ));
    };
    let found = hex::encode(Sha256::digest(compose.as_bytes()));
    if found != *expected {
        return Err(ToolError::Failed(format!(
            "the Compose file does not match the manifest's checksum: the manifest names \
             {expected}, this one hashes to {found}."
        )));
    }
    Ok(())
}

/// Run the pre-flight checks.
fn check_preflight(args: &InstallArgs, compose: &str) -> Result<(), ToolError> {
    for skipped in &args.ignore_preflight {
        println!("lakehousectl: the {skipped} check was skipped, as asked.");
    }
    let facts = preflight::collect(compose, &args.dir);
    let failures = preflight::run(&facts, &args.ignore_preflight);
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
    release: &Release,
    manifest: &Manifest,
) -> Result<secrets::Secrets, ToolError> {
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

    write(&dir.join("docker-compose.yml"), release.compose.as_bytes())?;
    write(&dir.join("release.json"), &release.manifest)?;
    write(&dir.join("release.json.sigstore.json"), &release.bundle)?;

    let state = serde_json::json!({
        "schemaVersion": 1,
        "currentVersion": manifest.version,
        "previousVersion": Value::Null,
        "lastBackup": Value::Null,
    });
    write(&dir.join("state.json"), format!("{state}\n").as_bytes())?;

    secrets::write_env(&dir.join(".env")).map_err(|source| ToolError::Io {
        context: format!("{} could not be written", dir.join(".env").display()),
        source,
    })
}

/// Fetch every image the release runs.
fn fetch_images(manifest: &Manifest, bundle_dir: Option<&Path>) -> Result<(), ToolError> {
    let Some(directory) = bundle_dir else {
        for reference in manifest.references() {
            println!("lakehousectl: pulling {reference}");
            docker::pull(&reference).map_err(ToolError::Failed)?;
        }
        return Ok(());
    };

    let checksums = read_to_string(&directory.join("SHA256SUMS"))?;
    for reference in manifest.references() {
        let tar = directory
            .join("images")
            .join(format!("{}.tar", image_name(&reference)));
        if !checksums_cover(&checksums, &tar, directory) {
            return Err(ToolError::Failed(format!(
                "{} is not in the bundle's SHA256SUMS, so nothing says it is the file the release \
                 published.",
                tar.display()
            )));
        }
        println!("lakehousectl: loading {}", tar.display());
        docker::load(&tar).map_err(ToolError::Failed)?;

        let Some(expected) = manifest.image_ids.get(&reference) else {
            return Err(ToolError::Failed(format!(
                "the manifest names no id for {reference}, so the loaded image cannot be checked \
                 against it."
            )));
        };
        let found = docker::image_id(&reference).map_err(ToolError::Failed)?;
        if found != *expected {
            return Err(ToolError::Failed(format!(
                "the image {reference} has the id {found}, and the manifest names {expected}. The \
                 image is not the one the release published, so the install stops here."
            )));
        }
    }
    Ok(())
}

/// Whether the bundle's checksums cover a file.
fn checksums_cover(checksums: &str, path: &Path, root: &Path) -> bool {
    let Ok(relative) = path.strip_prefix(root) else {
        return false;
    };
    let relative = relative.to_string_lossy();
    checksums.lines().any(|line| {
        line.split_whitespace()
            .nth(1)
            .is_some_and(|name| name.trim_start_matches('*') == relative)
    })
}

/// Start the stack and wait for it.
fn start(dir: &Path, manifest: &Manifest, env: &BTreeMap<String, String>) -> Result<(), ToolError> {
    println!("lakehousectl: starting the stack.");
    docker::compose_up(dir, &dir.join(".env")).map_err(ToolError::Failed)?;

    let api_port = port(env, "API_HOST_PORT", DEFAULT_API_PORT);
    let console_port = port(env, "CONSOLE_HOST_PORT", DEFAULT_CONSOLE_PORT);

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
    let console_port = port(env, "CONSOLE_HOST_PORT", DEFAULT_CONSOLE_PORT);
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

/// The value the `.env` sets for a port.
fn port(env: &BTreeMap<String, String>, name: &str, fallback: u16) -> u16 {
    env.get(name)
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(fallback)
}

/// The settings the install's `.env` holds.
fn read_env(path: &Path) -> BTreeMap<String, String> {
    let mut settings = BTreeMap::new();
    let Ok(text) = std::fs::read_to_string(path) else {
        return settings;
    };
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((name, value)) = line.split_once('=') {
            settings.insert(
                name.trim().to_string(),
                value
                    .trim()
                    .trim_matches('"')
                    .trim_matches('\'')
                    .to_string(),
            );
        }
    }
    settings
}

/// Read a file.
fn read(path: &Path) -> Result<Vec<u8>, ToolError> {
    std::fs::read(path).map_err(|source| ToolError::Io {
        context: format!("{} could not be read", path.display()),
        source,
    })
}

/// Read a file as text.
fn read_to_string(path: &Path) -> Result<String, ToolError> {
    std::fs::read_to_string(path).map_err(|source| ToolError::Io {
        context: format!("{} could not be read", path.display()),
        source,
    })
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
