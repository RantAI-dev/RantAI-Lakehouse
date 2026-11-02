//! `lakehousectl self-update`: the tool updates itself (REL-2 S4 decision D20).
//!
//! # The three steps
//!
//! D20 names them, and they are the trust chain of the whole product:
//!
//! 1. the newest release's `release.json` and its signature are read
//!    and verified with the key compiled into this binary;
//! 2. `lakehousectl-linux-x86_64` is downloaded and checked against
//!    that manifest's `files["lakehousectl"]`;
//! 3. the running binary is replaced ATOMICALLY — a write to a
//!    temporary file beside it, and then a rename — and the tool says
//!    which release it now is.
//!
//! Nothing is replaced before it has been checked against the SIGNED
//! manifest: a binary that does not hash to what the release published
//! is a refusal, not a warning.
//!
//! # Offline
//!
//! The self-update needs the network. An air-gapped host updates the
//! tool with the copy in its bundle: the bundle carries
//! `lakehousectl-linux-x86_64`, and that is the new tool (D20). The
//! help text says so.

use std::path::{Path, PathBuf};

use crate::error::ToolError;
use crate::files;
use crate::manifest::Manifest;
use crate::release;
use crate::verify;

/// The name of the installer's own asset in a release.
///
/// The release publishes the installer under this name, and the
/// manifest names its checksum under `files["lakehousectl"]` (D20).
pub const INSTALLER_ASSET: &str = "lakehousectl-linux-x86_64";

/// The name of the installer's checksum in the manifest.
const INSTALLER_FILE: &str = "lakehousectl";

/// Run `lakehousectl self-update`.
///
/// # Errors
///
/// Returns the error that stopped the self-update: a release that
/// could not be fetched, a manifest that does not verify, an installer
/// that does not match the manifest, or a binary that could not be
/// replaced.
pub fn command() -> Result<(), ToolError> {
    let binary = std::env::current_exe().map_err(|source| ToolError::Io {
        context: "the running tool could not be found".to_string(),
        source,
    })?;
    let key = verify::public_key()?;
    let version = run(&mut release::download, &binary, &key)?;
    println!("lakehousectl: the tool is now the one of release {version}.");
    Ok(())
}

/// The steps of the self-update, with the download, the binary and the
/// key injected.
///
/// The injection is what lets a test drive the whole path with no
/// network and with a key of its own: the manifest it fetches is
/// signed by that key, so the check that matters — the one that
/// refuses an installer that does not match the manifest — is the one
/// the test exercises.
///
/// # Errors
///
/// Returns the error that stopped the self-update.
pub fn run<F>(fetch: &mut F, binary: &Path, key: &[u8]) -> Result<String, ToolError>
where
    F: FnMut(&str) -> Result<Vec<u8>, ToolError>,
{
    // Step 1 — the newest release's manifest, and its signature.
    let newest = fetch(release::LATEST_URL)?;
    let version = release::latest_version_of(&String::from_utf8_lossy(&newest))?;
    let manifest_bytes = fetch(&release::asset_url(&version, "release.json"))?;
    let bundle = fetch(&release::asset_url(&version, "release.json.sigstore.json"))?;
    verify::verify_manifest(&manifest_bytes, &bundle, key)?;
    println!("lakehousectl: the manifest of release {version} verifies against the release key.");

    let manifest: Manifest = serde_json::from_slice(&manifest_bytes)
        .map_err(|error| ToolError::Failed(format!("the manifest could not be read: {error}")))?;

    // Step 2 — the installer itself, checked against the manifest.
    let Some(expected) = manifest.files.get(INSTALLER_FILE) else {
        return Err(ToolError::Failed(
            "the manifest names no checksum for the installer. A release that does not say which \
             installer it ships is a release whose tool cannot be updated."
                .to_string(),
        ));
    };
    let bytes = fetch(&release::asset_url(&version, INSTALLER_ASSET))?;
    files::check(INSTALLER_FILE, expected, &bytes)?;

    // Step 3 — the binary is replaced, atomically.
    replace(binary, &bytes)?;
    Ok(version)
}

/// Replace a binary with new bytes, atomically.
///
/// The bytes are written to a temporary file BESIDE the binary — the
/// same directory, so the same filesystem — and then renamed over it.
/// A rename is atomic: the binary is either the old one or the new
/// one, never half of each. And the old one keeps running until the
/// process that runs it ends, so replacing the tool does not break the
/// tool that is replacing it.
///
/// # Errors
///
/// Returns the error that says what could not be written or renamed.
fn replace(binary: &Path, bytes: &[u8]) -> Result<(), ToolError> {
    let temporary = temporary_path(binary);
    std::fs::write(&temporary, bytes).map_err(|source| ToolError::Io {
        context: format!("{} could not be written", temporary.display()),
        source,
    })?;
    set_executable(&temporary)?;
    std::fs::rename(&temporary, binary).map_err(|source| ToolError::Io {
        context: format!("{} could not be replaced", binary.display()),
        source,
    })?;
    Ok(())
}

/// The temporary file the new binary is written to.
fn temporary_path(binary: &Path) -> PathBuf {
    let name = binary.file_name().map_or_else(
        || "lakehousectl".to_string(),
        |name| name.to_string_lossy().to_string(),
    );
    binary.with_file_name(format!("{name}.new"))
}

/// Give a file the mode a program is run with.
///
/// # Errors
///
/// Returns the error that says what could not be written.
fn set_executable(path: &Path) -> Result<(), ToolError> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).map_err(|source| {
        ToolError::Io {
            context: format!("{} could not be given the mode 755", path.display()),
            source,
        }
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::os::unix::fs::PermissionsExt;

    use base64::Engine as _;
    use ecdsa::signature::Signer as _;
    use p256::ecdsa::{Signature, SigningKey};
    use p256::pkcs8::EncodePublicKey as _;
    use sha2::{Digest, Sha256};

    use super::*;

    /// The key the test signs with.
    ///
    /// A FIXED key: a test that signed with a random one would sign a
    /// different manifest on every run, and the manifest is part of
    /// what the test checks.
    fn test_key() -> SigningKey {
        SigningKey::from_bytes(&[42u8; 32].into()).expect("the key is built")
    }

    /// The public key of the test key, as the PEM the verifier reads.
    fn test_public_key() -> Vec<u8> {
        let der = test_key()
            .verifying_key()
            .to_public_key_der()
            .expect("the public key is written");
        let encoded = base64::engine::general_purpose::STANDARD.encode(der.as_bytes());
        let mut pem = String::from("-----BEGIN PUBLIC KEY-----\n");
        for line in encoded.as_bytes().chunks(64) {
            pem.push_str(std::str::from_utf8(line).expect("the key is text"));
            pem.push('\n');
        }
        pem.push_str("-----END PUBLIC KEY-----\n");
        pem.into_bytes()
    }

    /// The Sigstore bundle of a manifest, signed by the test key.
    fn sign(manifest: &[u8]) -> Vec<u8> {
        bundle_of(manifest, manifest)
    }

    /// A Sigstore bundle that names a manifest's digest and signs
    /// ANOTHER file.
    ///
    /// The digest check passes and the signature check does not — which
    /// is the case a bundle that carries a valid signature over some
    /// other file is made of.
    fn sign_another(manifest: &[u8]) -> Vec<u8> {
        bundle_of(manifest, b"another manifest")
    }

    /// The Sigstore bundle of `digest`, with a signature over `signed`.
    fn bundle_of(digest_of: &[u8], signed: &[u8]) -> Vec<u8> {
        let signature: Signature = test_key().sign(signed);
        let digest = base64::engine::general_purpose::STANDARD.encode(Sha256::digest(digest_of));
        let signature = base64::engine::general_purpose::STANDARD.encode(signature.to_der());
        let bundle = serde_json::json!({
            "mediaType": "application/vnd.dev.sigstore.bundle.v0.3+json",
            "messageSignature": {
                "messageDigest": {"algorithm": "SHA2_256", "digest": digest},
                "signature": signature,
            },
        });
        serde_json::to_vec(&bundle).expect("the bundle is written")
    }

    /// The manifest of a release that ships an installer.
    fn manifest_of(version: &str, installer: &[u8]) -> Vec<u8> {
        let checksum = hex::encode(Sha256::digest(installer));
        let manifest = serde_json::json!({
            "schemaVersion": 1,
            "version": version,
            "minFrom": "0.0.0",
            "stops": [],
            "images": {},
            "files": {"lakehousectl": checksum},
        });
        serde_json::to_vec(&manifest).expect("the manifest is written")
    }

    /// The bytes the tool would download, for one release.
    fn release_bytes(version: &str, installer: &[u8]) -> Vec<(String, Vec<u8>)> {
        let manifest = manifest_of(version, installer);
        vec![
            (
                release::LATEST_URL.to_string(),
                format!("{{\"tag_name\": \"v{version}\"}}").into_bytes(),
            ),
            (
                release::asset_url(version, "release.json"),
                manifest.clone(),
            ),
            (
                release::asset_url(version, "release.json.sigstore.json"),
                sign(&manifest),
            ),
            (
                release::asset_url(version, INSTALLER_ASSET),
                installer.to_vec(),
            ),
        ]
    }

    /// A download that serves the given bytes, and refuses the rest.
    fn downloader(bytes: Vec<(String, Vec<u8>)>) -> impl FnMut(&str) -> Result<Vec<u8>, ToolError> {
        move |url| {
            bytes
                .iter()
                .find(|(known, _)| known == url)
                .map(|(_, body)| body.clone())
                .ok_or_else(|| ToolError::Failed(format!("{url} is not in the release")))
        }
    }

    /// A binary the test may replace.
    fn binary() -> (tempfile::TempDir, PathBuf) {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let path = directory.path().join("lakehousectl");
        std::fs::write(&path, b"the old tool").expect("the binary is written");
        (directory, path)
    }

    #[test]
    fn the_tool_replaces_itself_with_the_installer_of_the_newest_release() {
        // D20: the tool is replaced with the binary the SIGNED
        // manifest names.
        let installer = b"the new tool";
        let mut fetch = downloader(release_bytes("0.2.0", installer));
        let (_directory, path) = binary();

        let version = run(&mut fetch, &path, &test_public_key()).expect("the tool updates itself");

        assert_eq!(version, "0.2.0", "the tool named another release");
        assert_eq!(
            std::fs::read(&path).expect("the binary is readable"),
            installer.to_vec(),
            "the binary was not replaced"
        );
        let mode = std::fs::metadata(&path)
            .expect("the binary is there")
            .permissions()
            .mode();
        assert_eq!(
            mode & 0o111,
            0o111,
            "the binary is not executable: {mode:o}"
        );
    }

    #[test]
    fn an_installer_that_does_not_match_the_manifest_is_refused() {
        // D20: the binary is checked against the SIGNED manifest
        // before it replaces the running one. The release serves
        // another binary than the one the manifest names.
        let mut bytes = release_bytes("0.2.0", b"the new tool");
        for (url, body) in &mut bytes {
            if url.ends_with(INSTALLER_ASSET) {
                *body = b"another tool".to_vec();
            }
        }
        let mut fetch = downloader(bytes);
        let (_directory, path) = binary();

        let error = run(&mut fetch, &path, &test_public_key())
            .expect_err("an installer that does not match is refused");

        assert!(
            error.to_string().contains("lakehousectl"),
            "the error names the file: {error}"
        );
        assert_eq!(
            std::fs::read(&path).expect("the binary is readable"),
            b"the old tool".to_vec(),
            "the binary was replaced anyway"
        );
    }

    #[test]
    fn a_manifest_that_does_not_verify_is_refused() {
        // The manifest of another key is not the release's manifest.
        let installer = b"the new tool";
        let mut bytes = release_bytes("0.2.0", installer);
        let manifest = bytes
            .iter()
            .find(|(url, _)| url.ends_with("release.json"))
            .map(|(_, body)| body.clone())
            .expect("the release carries a manifest");
        for (url, body) in &mut bytes {
            if url.ends_with("release.json.sigstore.json") {
                *body = sign_another(&manifest);
            }
        }
        let mut fetch = downloader(bytes);
        let (_directory, path) = binary();

        let error = run(&mut fetch, &path, &test_public_key())
            .expect_err("a manifest that does not verify is refused");

        assert!(
            error.to_string().contains("release key"),
            "the error says what failed: {error}"
        );
        assert_eq!(
            std::fs::read(&path).expect("the binary is readable"),
            b"the old tool".to_vec(),
            "the binary was replaced anyway"
        );
    }

    #[test]
    fn the_new_binary_is_written_beside_the_old_one_and_then_renamed() {
        // D20: the replacement is a write and then a rename, so the
        // binary is never half of each.
        let (_directory, path) = binary();
        replace(&path, b"the new tool").expect("the binary is replaced");

        assert_eq!(
            std::fs::read(&path).expect("the binary is readable"),
            b"the new tool".to_vec(),
            "the binary was not replaced"
        );
        assert!(
            !temporary_path(&path).exists(),
            "the temporary file was left behind"
        );
    }
}
