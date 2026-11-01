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
//!    bundle's `docker save` files, each one's checksum checked against
//!    the bundle's list and its ID against the signed manifest.
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
/// How much of a file is hashed at a time.
const HASH_BUFFER_BYTES: usize = 64 * 1024;

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
    obtain_with(args, &mut download)
}

/// The release, with the download injected.
///
/// The injection is what lets a test drive the online path with no
/// network: what the test holds to is the SHAPE of that path — which
/// assets are asked for, and that nothing lands on the disk.
fn obtain_with<F>(args: &InstallArgs, fetch: &mut F) -> Result<Release, ToolError>
where
    F: FnMut(&str) -> Result<Vec<u8>, ToolError>,
{
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
    let manifest = fetch(&asset_url(&version, "release.json"))?;
    let bundle = fetch(&asset_url(&version, "release.json.sigstore.json"))?;
    let compose =
        String::from_utf8(fetch(&asset_url(&version, "docker-compose.yml"))?).map_err(|error| {
            ToolError::Failed(format!("the release's Compose file is not text: {error}"))
        })?;
    Ok(Release {
        manifest,
        bundle,
        compose,
        bundle_dir: None,
    })
}

/// The URL of one asset of a release.
fn asset_url(version: &str, name: &str) -> String {
    format!("{RELEASES_URL}/v{version}/{name}")
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

/// Download one file of a release, into memory.
///
/// The bytes never touch the disk. The release's assets are small text
/// files, and a file in a shared directory is a file another local
/// user could own: the installer runs as root, so a symlink left where
/// the tool writes would let a local user have root write anywhere on
/// the host. The path this used was
/// `temp_dir()/lakehousectl-release-<version>` — a name anybody could
/// guess, and a directory an earlier run left behind (REL-2 S3 review
/// round 1 BLOCKER 1).
fn download(url: &str) -> Result<Vec<u8>, ToolError> {
    sys::output_bytes(
        "curl",
        &["--fail", "--silent", "--show-error", "--location", url],
    )
    .map_err(|error| {
        ToolError::Failed(format!(
            "{url} could not be downloaded: {error}. Is this host online, and is `curl` installed?"
        ))
    })
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
    // The state of an earlier install is not overwritten: the same
    // release may be installed again, and any other release is refused
    // — that is what `lakehousectl update` is for (REL-2 S3 review
    // round 1 SHOULD-FIX 3).
    check_state(dir, &manifest.version)?;

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

    // The state of the FIRST install is kept: it names the release that
    // is installed, the one before it, and the last backup. A
    // re-install of the same release changes nothing of it.
    if !dir.join("state.json").exists() {
        let state = serde_json::json!({
            "schemaVersion": 1,
            "currentVersion": manifest.version,
            "previousVersion": Value::Null,
            "lastBackup": Value::Null,
        });
        write(&dir.join("state.json"), format!("{state}\n").as_bytes())?;
    }

    secrets::write_env(&dir.join(".env")).map_err(|source| ToolError::Io {
        context: format!("{} could not be written", dir.join(".env").display()),
        source,
    })
}

/// Refuse an install that would replace the release of another one.
///
/// The directory holds the state of the install that is there, and the
/// state names the release that is installed. The SAME release may be
/// installed again — a re-run of the same install — and its state is
/// kept as it is. Any OTHER release is refused, and the refusal says
/// which command does that work (REL-2 S3 review round 1 SHOULD-FIX 3).
fn check_state(dir: &Path, version: &str) -> Result<(), ToolError> {
    let path = dir.join("state.json");
    if !path.exists() {
        return Ok(());
    }
    let text = read_to_string(&path)?;
    let installed = serde_json::from_str::<Value>(&text)
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
        // The file is hashed BEFORE it is loaded: a copy that was
        // damaged on its way to the host fails here, with a message
        // that says so, and not as a `docker load` that fails halfway
        // (REL-2 S3 review round 1 SHOULD-FIX 2).
        verify_checksum(&checksums, &tar, directory)?;
        println!("lakehousectl: loading {}", tar.display());
        docker::load(&tar).map_err(ToolError::Failed)?;

        let Some(expected) = manifest.image_ids.get(&reference) else {
            return Err(ToolError::Failed(format!(
                "the manifest names no id for {reference}, so the loaded image cannot be checked \
                 against it."
            )));
        };
        let found = docker::image_id(&reference).map_err(ToolError::Failed)?;
        // Docker's two image stores identify an image differently: the
        // classic store reports the CONFIG digest, the containerd store
        // the image INDEX digest. Both are named in the SIGNED
        // manifest — the config digest in `imageIds`, the registry
        // digest in `images` — so the loaded image is accepted when its
        // ID is one of them, and refused when it is neither.
        let registry_digest = Manifest::digest_of(&reference).unwrap_or_default();
        if found != *expected && found != registry_digest {
            return Err(ToolError::Failed(format!(
                "the image {reference} has the id {found}, and the manifest names {expected} and \
                 {registry_digest}. The image is not the one the release published, so the \
                 install stops here."
            )));
        }
    }
    Ok(())
}

/// Check one file of the bundle against the bundle's checksums.
///
/// The bundle's `SHA256SUMS` is NOT signed: it catches a file that was
/// damaged or truncated on its way to the host, and nothing else. The
/// TRUST check is the image's ID, compared with the SIGNED manifest's
/// `imageIds` after the load (REL-2 S3 review round 1 SHOULD-FIX 2).
fn verify_checksum(checksums: &str, path: &Path, root: &Path) -> Result<(), ToolError> {
    let Some(expected) = checksum_of(checksums, path, root) else {
        return Err(ToolError::Failed(format!(
            "{} is not in the bundle's SHA256SUMS, so nothing says it is the file the release \
             published.",
            path.display()
        )));
    };
    let found = sha256_of(path)?;
    if found != expected {
        return Err(ToolError::Failed(format!(
            "{} does not match the bundle's SHA256SUMS: the list names {expected}, this one hashes \
             to {found}. The file was damaged on its way here.",
            path.display()
        )));
    }
    Ok(())
}

/// The checksum the bundle's list names for a file.
///
/// The list holds one line per file: the checksum, then the name, with
/// the path relative to the bundle's root.
fn checksum_of(checksums: &str, path: &Path, root: &Path) -> Option<String> {
    let relative = path.strip_prefix(root).ok()?;
    let relative = relative.to_string_lossy();
    checksums.lines().find_map(|line| {
        let mut fields = line.split_whitespace();
        let digest = fields.next()?;
        let name = fields.next()?;
        (name.trim_start_matches('*') == relative).then(|| digest.to_string())
    })
}

/// The SHA-256 of a file, as hexadecimal.
///
/// The file is read in pieces: an image tar is hundreds of megabytes,
/// and the installer has no reason to hold one in memory.
fn sha256_of(path: &Path) -> Result<String, ToolError> {
    let file = std::fs::File::open(path).map_err(|source| ToolError::Io {
        context: format!("{} could not be read", path.display()),
        source,
    })?;
    let mut reader = std::io::BufReader::new(file);
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; HASH_BUFFER_BYTES];
    loop {
        let read =
            std::io::Read::read(&mut reader, &mut buffer).map_err(|source| ToolError::Io {
                context: format!("{} could not be read", path.display()),
                source,
            })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::collections::BTreeSet;

    use super::*;

    /// The names the temporary directory holds that begin with the
    /// tool's own prefix.
    ///
    /// Only the tool's own names are looked at: the other tests of the
    /// workspace write in the same directory, and what matters here is
    /// that the INSTALLER leaves nothing of its own behind.
    fn tool_entries() -> BTreeSet<String> {
        let Ok(entries) = std::fs::read_dir(std::env::temp_dir()) else {
            return BTreeSet::new();
        };
        entries
            .filter_map(std::result::Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().to_string())
            .filter(|name| name.starts_with("lakehousectl"))
            .collect()
    }

    #[test]
    fn the_online_install_reads_the_release_into_memory_and_writes_no_file() {
        // The installer runs as root: a file it writes in a directory
        // of a name anybody can guess is a file another local user can
        // own, and a symlink there would let that user have root write
        // anywhere on the host (REL-2 S3 review round 1 BLOCKER 1).
        let mut asked: Vec<String> = Vec::new();
        let args = InstallArgs {
            bundle: None,
            dir: PathBuf::from("/nonexistent/lakehousectl-install"),
            version: Some("0.2.0".to_string()),
            ignore_preflight: Vec::new(),
        };
        let before = tool_entries();
        let release = obtain_with(&args, &mut |url| {
            asked.push(url.to_string());
            Ok(b"{}".to_vec())
        })
        .expect("the release is obtained");
        let after = tool_entries();

        assert_eq!(
            asked,
            vec![
                asset_url("0.2.0", "release.json"),
                asset_url("0.2.0", "release.json.sigstore.json"),
                asset_url("0.2.0", "docker-compose.yml"),
            ],
            "the install asked for the wrong assets"
        );
        assert_eq!(
            release.manifest, b"{}",
            "the bytes were not the fetched ones"
        );
        assert_eq!(
            before, after,
            "the install left something of its own in the temporary directory"
        );
        assert!(
            !std::env::temp_dir()
                .join("lakehousectl-release-0.2.0")
                .exists(),
            "the install used the shared directory of an earlier release"
        );
    }

    /// A bundle with one image tar and the list of checksums.
    fn bundle_with_one_image() -> (tempfile::TempDir, PathBuf, String) {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let images = directory.path().join("images");
        std::fs::create_dir_all(&images).expect("the images directory is created");
        let tar = images.join("alpine.tar");
        std::fs::write(&tar, b"the image").expect("the tar is written");
        let digest = hex::encode(Sha256::digest(b"the image"));
        let checksums = format!("{digest}  images/alpine.tar\n");
        (directory, tar, checksums)
    }

    #[test]
    fn a_bundle_file_that_matches_its_checksum_is_accepted() {
        let (directory, tar, checksums) = bundle_with_one_image();
        verify_checksum(&checksums, &tar, directory.path()).expect("the file matches its checksum");
    }

    #[test]
    fn a_damaged_bundle_file_is_refused() {
        // The copy was damaged on its way to the host: the checksum
        // says so before `docker load` reads a broken tar.
        let (directory, tar, checksums) = bundle_with_one_image();
        std::fs::write(&tar, b"the imag3").expect("the tar is written again");

        let error = verify_checksum(&checksums, &tar, directory.path())
            .expect_err("a damaged file is refused");
        let message = error.to_string();
        assert!(
            message.contains("SHA256SUMS"),
            "the error says what failed: {message}"
        );
        assert!(
            message.contains("images/alpine.tar"),
            "the error names the file: {message}"
        );
    }

    #[test]
    fn a_bundle_file_the_list_does_not_name_is_refused() {
        let (directory, _tar, _checksums) = bundle_with_one_image();
        let other = directory.path().join("images").join("other.tar");
        std::fs::write(&other, b"another image").expect("the tar is written");

        let error = verify_checksum("", &other, directory.path())
            .expect_err("a file the list does not name is refused");
        assert!(
            error.to_string().contains("SHA256SUMS"),
            "the error says what failed: {error}"
        );
    }

    /// A release of a version, as the install sees it.
    fn release_of(version: &str) -> Release {
        Release {
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
            images: BTreeMap::new(),
            image_ids: BTreeMap::new(),
            files: BTreeMap::new(),
        }
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
