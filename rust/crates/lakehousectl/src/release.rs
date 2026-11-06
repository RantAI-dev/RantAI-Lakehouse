//! Where a release's assets live, and how a release is obtained
//! (REL-2 S3-T1, REL-2 S4-T4).
//!
//! One place owns the release's URLs and the shape of an obtained
//! release: the install (`install`), the update (`update`) and the
//! self-update (`self_update`) all read a release through here, so
//! they cannot disagree about where a release lives or what it holds
//! (rule 4).

use std::path::{Path, PathBuf};

use crate::error::ToolError;
use crate::sys;

/// Where a release's assets live.
pub const RELEASES_URL: &str = "https://github.com/RantAI-dev/RantAI-Lakehouse/releases/download";

/// Where the newest release is named.
pub const LATEST_URL: &str =
    "https://api.github.com/repos/RantAI-dev/RantAI-Lakehouse/releases/latest";

/// The URL of one asset of a release.
///
/// The asset's name is the file's own name: the release publishes one
/// asset per file it ships.
#[must_use]
pub fn asset_url(version: &str, name: &str) -> String {
    format!("{RELEASES_URL}/v{version}/{name}")
}

/// A release, as the tool obtained it.
#[derive(Debug, Clone)]
pub struct Release {
    /// The manifest's bytes.
    pub manifest: Vec<u8>,
    /// The manifest's Sigstore bundle.
    pub bundle: Vec<u8>,
    /// The release's Compose file.
    pub compose: String,
    /// The bundle's directory, when the release came from one.
    pub bundle_dir: Option<PathBuf>,
}

/// Obtain a release: from the bundle, or from GitHub.
///
/// The download is INJECTED, so the online path can be driven with no
/// network at all: a test holds to the shape of that path — which
/// assets are asked for, and that nothing lands on the disk.
///
/// # Errors
///
/// Returns the error that says what could not be obtained.
pub fn obtain(
    bundle: Option<&Path>,
    version: Option<&str>,
    fetch: &mut dyn FnMut(&str) -> Result<Vec<u8>, ToolError>,
) -> Result<Release, ToolError> {
    if let Some(directory) = bundle {
        return Ok(Release {
            manifest: read(&directory.join("release.json"))?,
            bundle: read(&directory.join("release.json.sigstore.json"))?,
            compose: read_to_string(&directory.join("docker-compose.yml"))?,
            bundle_dir: Some(directory.to_path_buf()),
        });
    }

    let version = match version {
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

/// The version the manifest names.
///
/// # Errors
///
/// Returns the error that says what could not be read.
pub fn version_of(manifest: &[u8]) -> Result<String, ToolError> {
    let document: serde_json::Value = serde_json::from_slice(manifest)
        .map_err(|error| ToolError::Failed(format!("the manifest could not be read: {error}")))?;
    document["version"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| ToolError::Failed("the manifest names no version".to_string()))
}

/// The version of the newest release.
///
/// # Errors
///
/// Returns the error that says what could not be read, and what to
/// fix.
pub fn latest_version() -> Result<String, ToolError> {
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
    latest_version_of(&body)
}

/// The version the newest-release document names.
///
/// # Errors
///
/// Returns the error that says what could not be read.
pub fn latest_version_of(body: &str) -> Result<String, ToolError> {
    let document: serde_json::Value = serde_json::from_str(body).map_err(|error| {
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
///
/// # Errors
///
/// Returns the error that says what could not be downloaded.
pub fn download(url: &str) -> Result<Vec<u8>, ToolError> {
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn the_asset_of_a_release_is_under_the_release_tag() {
        assert_eq!(
            asset_url("0.2.0", "docker-compose.yml"),
            "https://github.com/RantAI-dev/RantAI-Lakehouse/releases/download/v0.2.0/docker-compose.yml"
        );
    }

    #[test]
    fn the_online_release_is_read_into_memory_and_no_file_is_written() {
        // The tool runs as root: a file it writes in a directory of a
        // name anybody can guess is a file another local user can own,
        // and a symlink there would let that user have root write
        // anywhere on the host (REL-2 S3 review round 1 BLOCKER 1).
        let mut asked: Vec<String> = Vec::new();
        let release = obtain(None, Some("0.2.0"), &mut |url| {
            asked.push(url.to_string());
            Ok(b"{}".to_vec())
        })
        .expect("the release is obtained");

        assert_eq!(
            asked,
            vec![
                asset_url("0.2.0", "release.json"),
                asset_url("0.2.0", "release.json.sigstore.json"),
                asset_url("0.2.0", "docker-compose.yml"),
            ],
            "the tool asked for the wrong assets"
        );
        assert_eq!(
            release.manifest, b"{}",
            "the bytes were not the fetched ones"
        );
        assert!(
            !std::env::temp_dir()
                .join("lakehousectl-release-0.2.0")
                .exists(),
            "the tool used the shared directory of an earlier release"
        );
    }

    #[test]
    fn the_newest_release_is_named_by_its_tag() {
        let version =
            latest_version_of(r#"{"tag_name": "v0.2.0"}"#).expect("the newest release is named");
        assert_eq!(version, "0.2.0");
    }

    #[test]
    fn a_newest_release_that_names_no_tag_is_refused() {
        assert!(latest_version_of("{}").is_err());
    }

    #[test]
    fn the_version_of_a_manifest_is_read() {
        let version = version_of(br#"{"version": "0.2.0"}"#).expect("the version is read");
        assert_eq!(version, "0.2.0");
    }

    #[test]
    fn a_manifest_that_names_no_version_is_refused() {
        assert!(version_of(b"{}").is_err());
    }
}
