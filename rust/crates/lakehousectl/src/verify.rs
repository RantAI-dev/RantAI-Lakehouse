//! Verification of the release manifest against the release key (REL-2 S3-T1).
//!
//! # Why this module exists
//!
//! One idea carries the whole release design: **sign one file, verify
//! everything through it** (REL-2 decision D3). The manifest
//! (`release.json`) is that file, and its signature is a Sigstore
//! bundle (`release.json.sigstore.json`, REL-2 S2 D3a): cosign v3 writes
//! the signature and the manifest's digest into one JSON file.
//!
//! This module is the Rust half of the recipe REL-2 S2 proved in Python
//! (`ops/release/test_sigstore_bundle.py`), against the same fixture:
//!
//! - `messageSignature.messageDigest` is the SHA-256 of the manifest
//!   beside the bundle;
//! - `messageSignature.signature` is a base64 DER ECDSA P-256 signature
//!   over that same SHA-256;
//! - the public key is COMPILED INTO the binary
//!   (`include_bytes!` of `ops/release/release.pub`), so the check needs
//!   no network, no registry and no `cosign`.
//!
//! The transparency-log fields a bundle may carry are IGNORED on
//! purpose: the pipeline signs with no tlog (REL-2 S2 D3a), so there is
//! no log entry to verify, and an air-gapped host could not reach one
//! anyway.
//!
//! # The key
//!
//! A release build verifies against the key compiled in, and against no
//! other. The `test-key` feature — never enabled in a release build —
//! lets a test or the CI dry run point at another key, because the dry
//! run signs its bundle with a throwaway key it generates in the job
//! (REL-2 S3-T5). A release build that finds that variable set says so
//! and ignores it.

use base64::Engine as _;
use ecdsa::signature::Verifier;
use p256::ecdsa::{Signature, VerifyingKey};
use p256::pkcs8::DecodePublicKey;
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::cli::VerifyArgs;
use crate::error::ToolError;

/// The release's public key, compiled into the binary.
///
/// Committed at `ops/release/release.pub` (REL-2 decision D3), so the
/// key a signature is checked against is the key the repository holds —
/// not one the host can choose.
pub const RELEASE_PUBLIC_KEY: &[u8] = include_bytes!("../../../../ops/release/release.pub");

/// The environment variable that points a test build at another key.
const TEST_KEY_VARIABLE: &str = "LAKEHOUSECTL_PUBLIC_KEY_FILE";

/// The manifest did not verify.
#[derive(Debug, thiserror::Error)]
pub enum VerifyError {
    /// The Sigstore bundle could not be read as the bundle we expect.
    #[error("the Sigstore bundle is not readable as a release signature: {0}")]
    Malformed(String),
    /// The bundle's digest does not name the manifest beside it.
    #[error(
        "the signature is over a different file: the bundle names the digest {expected}, \
         the manifest beside it hashes to {found}"
    )]
    DigestMismatch {
        /// The digest the bundle names.
        expected: String,
        /// The digest of the manifest that was checked.
        found: String,
    },
    /// The bundle's signature is not the key's.
    #[error("the release key did not sign this manifest")]
    SignatureInvalid,
    /// The public key could not be read.
    #[error("the release public key is not readable: {0}")]
    KeyInvalid(String),
    /// The digest algorithm is not the one the release uses.
    #[error("the signature covers a {0} digest; the release signs a SHA-256 digest")]
    UnsupportedAlgorithm(String),
}

/// The parts of a Sigstore bundle this tool reads.
///
/// Every field is optional in the type and checked at use: a bundle
/// that is missing one of them is a bundle this tool cannot verify, and
/// it says so rather than guessing.
///
/// The bundle's KEY HINT and any transparency-log entry are NOT read at
/// all: the key that verifies is the one compiled in, and the pipeline
/// signs with no tlog (REL-2 S2 D3a), so there is no log entry to
/// verify and nothing to depend on.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SigstoreBundle {
    /// The media type of the bundle.
    media_type: Option<String>,
    /// The signature itself.
    message_signature: Option<MessageSignature>,
}

/// The signature over the manifest.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MessageSignature {
    /// The digest the signature covers.
    message_digest: Option<MessageDigest>,
    /// The signature, base64 DER.
    signature: Option<String>,
}

/// The digest a signature covers.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MessageDigest {
    /// The digest algorithm.
    algorithm: Option<String>,
    /// The digest, base64.
    digest: Option<String>,
}

/// The public key this build verifies against.
///
/// A release build always uses the key compiled in. The `test-key`
/// feature (never enabled in a release build) lets a test or the CI
/// dry run point at another key file: the dry run signs its bundle
/// with a throwaway key it generates in the job, and the installer has
/// to be able to verify that bundle (REL-2 S3-T5).
///
/// # Errors
///
/// Returns [`VerifyError::KeyInvalid`] when the key file named by
/// `LAKEHOUSECTL_PUBLIC_KEY_FILE` cannot be read.
#[cfg(not(feature = "test-key"))]
// The `Result` is what the `test-key` build's version returns, and one
// signature for both keeps the caller free of a `cfg`.
#[allow(clippy::unnecessary_wraps, reason = "the test-key build can fail here")]
pub fn public_key() -> Result<Vec<u8>, VerifyError> {
    if std::env::var_os(TEST_KEY_VARIABLE).is_some() {
        // Said out loud, because a release build that silently ignored
        // a key would be a release that verifies something else than
        // its operator believes.
        eprintln!(
            "lakehousectl: {TEST_KEY_VARIABLE} is set and ignored: this build verifies \
             against the release key compiled into it."
        );
    }
    Ok(RELEASE_PUBLIC_KEY.to_vec())
}

/// The public key this build verifies against.
///
/// # Errors
///
/// Returns [`VerifyError::KeyInvalid`] when the key file named by
/// `LAKEHOUSECTL_PUBLIC_KEY_FILE` cannot be read.
#[cfg(feature = "test-key")]
pub fn public_key() -> Result<Vec<u8>, VerifyError> {
    let Some(path) = std::env::var_os(TEST_KEY_VARIABLE) else {
        return Ok(RELEASE_PUBLIC_KEY.to_vec());
    };
    std::fs::read(std::path::Path::new(&path)).map_err(|error| {
        VerifyError::KeyInvalid(format!(
            "{} could not be read: {error}",
            path.to_string_lossy()
        ))
    })
}

/// Check a manifest against a Sigstore bundle and a public key.
///
/// The check is the recipe of REL-2 S2 D3a, in the order that recipe
/// gives:
///
/// 1. the bundle's `messageDigest` must be the SHA-256 of the manifest
///    — without it a bundle could carry a valid signature over some
///    OTHER file;
/// 2. the signature must be the key's, over that manifest.
///
/// # Errors
///
/// Returns [`VerifyError`] when the bundle is not readable, when the
/// digest does not name the manifest, when the key cannot be read, or
/// when the key did not sign the manifest.
pub fn verify_manifest(
    manifest: &[u8],
    bundle: &[u8],
    public_key_pem: &[u8],
) -> Result<(), VerifyError> {
    let bundle: SigstoreBundle = serde_json::from_slice(bundle)
        .map_err(|error| VerifyError::Malformed(error.to_string()))?;

    let media_type = bundle
        .media_type
        .ok_or_else(|| VerifyError::Malformed("the bundle names no media type".into()))?;
    if !media_type.starts_with("application/vnd.dev.sigstore.bundle") {
        return Err(VerifyError::Malformed(format!(
            "the bundle's media type is {media_type}, which is not a Sigstore bundle"
        )));
    }

    let signature = bundle
        .message_signature
        .ok_or_else(|| VerifyError::Malformed("the bundle carries no signature".into()))?;
    let digest = signature
        .message_digest
        .ok_or_else(|| VerifyError::Malformed("the bundle names no digest".into()))?;
    let algorithm = digest.algorithm.clone().unwrap_or_default();
    if algorithm != "SHA2_256" {
        return Err(VerifyError::UnsupportedAlgorithm(algorithm));
    }
    let expected = digest
        .digest
        .ok_or_else(|| VerifyError::Malformed("the bundle's digest is empty".into()))?;
    let expected = base64::engine::general_purpose::STANDARD
        .decode(expected.as_bytes())
        .map_err(|error| {
            VerifyError::Malformed(format!("the bundle's digest is not base64: {error}"))
        })?;

    let found = Sha256::digest(manifest);
    if expected[..] != found[..] {
        return Err(VerifyError::DigestMismatch {
            expected: hex::encode(expected),
            found: hex::encode(found),
        });
    }

    let signature = signature
        .signature
        .ok_or_else(|| VerifyError::Malformed("the bundle's signature is empty".into()))?;
    let signature = base64::engine::general_purpose::STANDARD
        .decode(signature.as_bytes())
        .map_err(|error| {
            VerifyError::Malformed(format!("the bundle's signature is not base64: {error}"))
        })?;
    let signature = Signature::from_der(&signature)
        .map_err(|error| VerifyError::Malformed(format!("the signature is not DER: {error}")))?;

    let key_pem = std::str::from_utf8(public_key_pem)
        .map_err(|error| VerifyError::KeyInvalid(format!("the key is not text: {error}")))?;
    let key = VerifyingKey::from_public_key_pem(key_pem)
        .map_err(|error| VerifyError::KeyInvalid(error.to_string()))?;

    // `verify` hashes the message with SHA-256 before checking it,
    // which is what cosign signed: an ECDSA P-256 signature over the
    // SHA-256 of the manifest.
    key.verify(manifest, &signature)
        .map_err(|_| VerifyError::SignatureInvalid)
}

/// Run `lakehousectl verify`: check a release directory against the
/// release key.
///
/// # Errors
///
/// Returns [`ToolError`] when a file of the release cannot be read, or
/// when the manifest does not verify.
pub fn command(args: &VerifyArgs) -> Result<(), ToolError> {
    let manifest_path = args.dir.join("release.json");
    let bundle_path = args.dir.join("release.json.sigstore.json");
    let manifest = std::fs::read(&manifest_path).map_err(|source| ToolError::Io {
        context: format!("{} could not be read", manifest_path.display()),
        source,
    })?;
    let bundle = std::fs::read(&bundle_path).map_err(|source| ToolError::Io {
        context: format!("{} could not be read", bundle_path.display()),
        source,
    })?;
    let key = public_key()?;
    verify_manifest(&manifest, &bundle, &key)?;
    println!(
        "lakehousectl: the manifest in {} verifies against the release key.",
        args.dir.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    /// The fixture REL-2 S2 made with a throwaway key: the manifest
    /// that was signed, the bundle cosign wrote for it, and the public
    /// key of the key that signed it. The private half of that key is
    /// NOT in the repository.
    const FIXTURE_MANIFEST: &[u8] =
        include_bytes!("../../../../ops/release/fixtures/sigstore/release.json");
    const FIXTURE_BUNDLE: &[u8] =
        include_bytes!("../../../../ops/release/fixtures/sigstore/release.json.sigstore.json");
    const FIXTURE_PUBLIC_KEY: &[u8] =
        include_bytes!("../../../../ops/release/fixtures/sigstore/throwaway.pub");

    #[test]
    fn the_fixture_manifest_verifies_against_its_own_key() {
        assert!(verify_manifest(FIXTURE_MANIFEST, FIXTURE_BUNDLE, FIXTURE_PUBLIC_KEY).is_ok());
    }

    #[test]
    fn a_tampered_manifest_does_not_verify() {
        let mut tampered = FIXTURE_MANIFEST.to_vec();
        tampered.push(b'\n');
        let error = verify_manifest(&tampered, FIXTURE_BUNDLE, FIXTURE_PUBLIC_KEY)
            .expect_err("a tampered manifest must not verify");
        assert!(
            matches!(error, VerifyError::DigestMismatch { .. }),
            "the tampered manifest failed with {error:?}, not with a digest mismatch"
        );
    }

    #[test]
    fn a_manifest_signed_by_another_key_does_not_verify() {
        let error = verify_manifest(FIXTURE_MANIFEST, FIXTURE_BUNDLE, RELEASE_PUBLIC_KEY)
            .expect_err("the release key did not sign the fixture");
        assert!(
            matches!(error, VerifyError::SignatureInvalid),
            "the fixture failed with {error:?}, not with an invalid signature"
        );
    }

    #[test]
    fn a_bundle_that_is_not_a_bundle_is_refused() {
        let error = verify_manifest(FIXTURE_MANIFEST, b"not json", FIXTURE_PUBLIC_KEY)
            .expect_err("a bundle that is not JSON must not verify");
        assert!(matches!(error, VerifyError::Malformed(_)));
    }

    #[test]
    fn a_bundle_without_a_signature_is_refused() {
        let error = verify_manifest(FIXTURE_MANIFEST, b"{}", FIXTURE_PUBLIC_KEY)
            .expect_err("a bundle without a signature must not verify");
        assert!(matches!(error, VerifyError::Malformed(_)));
    }

    #[test]
    fn a_key_that_is_not_a_key_is_refused() {
        let error = verify_manifest(FIXTURE_MANIFEST, FIXTURE_BUNDLE, b"not a key")
            .expect_err("a key that is not a key must not verify");
        assert!(matches!(error, VerifyError::KeyInvalid(_)));
    }
}
