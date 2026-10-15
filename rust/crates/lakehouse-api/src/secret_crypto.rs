//! Seals and opens the managed connector credential files —
//! `SRC-12` decision 4, ADR 0002 addendum 5
//! (`docs/adr/0002-secretref-resolution.md`).
//!
//! # Format
//!
//! One managed file is `version byte (0x01) || 96-bit random nonce ||
//! AES-256-GCM ciphertext with its 16-byte tag`. The associated data is the
//! file's own name, so a sealed file copied under another connector's name
//! does not open. Only this crate seals; the orchestrator
//! (`dagster/dispar_orchestrate/secret_resolver.py`) opens with the same
//! format, and `tests/fixtures/connector_secret_vector.json` is checked by
//! both test suites so the two cannot drift.
//!
//! # Keys
//!
//! `CONNECTOR_SECRETS_KEY` is 32 random bytes in base64. During a key change
//! (decision 5) `CONNECTOR_SECRETS_KEY_PREVIOUS` still OPENS files; it never
//! seals. See [`Keyring`].
//!
//! # What this module never does
//!
//! It never prints, logs or returns a key, and its errors are fixed texts
//! (decision 12): they say which of "key missing", "key malformed", "key
//! wrong" or "file not in this format" happened, never any byte of either.

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::Engine as _;
use rand::Rng as _;

/// First byte of a sealed file. A future format change bumps it, so an old
/// reader refuses a new file instead of misreading it.
pub const FORMAT_VERSION: u8 = 1;
const NONCE_LEN: usize = 12;
const TAG_LEN: usize = 16;
const KEY_LEN: usize = 32;

/// The shortest possible sealed file: version, nonce, and the tag of an
/// empty plaintext.
pub const MIN_SEALED_LEN: usize = 1 + NONCE_LEN + TAG_LEN;

/// Every managed credential file's name begins with this (the part of a
/// `CredentialSource::Managed` ref after `file:/run/secrets/`).
pub const MANAGED_FILE_PREFIX: &str = "connector_managed_";

/// Why a credential could not be sealed or opened. The messages are fixed
/// and value-free (decision 12).
#[derive(Debug, Clone, Copy, thiserror::Error, PartialEq, Eq)]
pub enum CryptoError {
    /// No `CONNECTOR_SECRETS_KEY`.
    #[error(
        "CONNECTOR_SECRETS_KEY is not set; connector credentials cannot be stored or read without it"
    )]
    KeyMissing,
    /// The key (or the previous key) is not 32 bytes in base64.
    #[error(
        "CONNECTOR_SECRETS_KEY (or CONNECTOR_SECRETS_KEY_PREVIOUS) is not 32 random bytes in base64"
    )]
    KeyInvalid,
    /// The file is in this format but neither configured key opens it.
    #[error("the stored credential cannot be opened with the configured key")]
    WrongKey,
    /// The bytes are not a sealed file (too short, or another version).
    #[error("the stored credential file is not in a format this build reads")]
    Unreadable,
    /// The cipher refused to seal; not expected for a 32-byte key.
    #[error("the credential could not be sealed")]
    SealFailed,
}

/// One AES-256-GCM key. `Debug` prints nothing of it.
#[derive(Clone)]
pub struct SecretsKey(Aes256Gcm);

impl std::fmt::Debug for SecretsKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretsKey(<redacted>)")
    }
}

impl SecretsKey {
    /// Parse a key given as base64 of exactly 32 bytes.
    ///
    /// # Errors
    ///
    /// [`CryptoError::KeyInvalid`] for anything else.
    pub fn from_base64(encoded: &str) -> Result<Self, CryptoError> {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded.trim())
            .map_err(|_| CryptoError::KeyInvalid)?;
        if bytes.len() != KEY_LEN {
            return Err(CryptoError::KeyInvalid);
        }
        Aes256Gcm::new_from_slice(&bytes)
            .map(Self)
            .map_err(|_| CryptoError::KeyInvalid)
    }

    /// `plaintext` sealed for the file called `file_name`, with a fresh
    /// random nonce.
    fn seal(&self, file_name: &str, plaintext: &[u8]) -> Result<Vec<u8>, CryptoError> {
        let mut nonce = [0_u8; NONCE_LEN];
        rand::rng().fill_bytes(&mut nonce);
        let sealed = self
            .0
            .encrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: plaintext,
                    aad: file_name.as_bytes(),
                },
            )
            .map_err(|_| CryptoError::SealFailed)?;
        let mut out = Vec::with_capacity(1 + NONCE_LEN + sealed.len());
        out.push(FORMAT_VERSION);
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&sealed);
        Ok(out)
    }

    /// The plaintext of `sealed`, or `None` when this key does not open it
    /// (wrong key, wrong file name, or damaged bytes — GCM cannot tell
    /// them apart, and neither should the caller).
    fn try_open(&self, file_name: &str, sealed: &[u8]) -> Option<Vec<u8>> {
        let (nonce, ciphertext) = sealed.get(1..)?.split_at_checked(NONCE_LEN)?;
        self.0
            .decrypt(
                Nonce::from_slice(nonce),
                Payload {
                    msg: ciphertext,
                    aad: file_name.as_bytes(),
                },
            )
            .ok()
    }
}

/// Whether `bytes` have the shape of a sealed file (not whether they open).
/// The start-up pass uses it to tell a sealed file whose key is not
/// configured from a plain one that still has to be sealed: a credential is
/// printable text, never begins with the control byte `0x01`, and a real
/// seal is at least [`MIN_SEALED_LEN`] bytes long.
#[must_use]
pub fn looks_sealed(bytes: &[u8]) -> bool {
    bytes.first() == Some(&FORMAT_VERSION) && bytes.len() >= MIN_SEALED_LEN
}

/// The key that seals, and the older key that may still open.
#[derive(Debug, Clone)]
pub struct Keyring {
    current: SecretsKey,
    previous: Option<SecretsKey>,
}

/// A file opened by a [`Keyring`], and whether the current key did it. The
/// key-change command re-seals what only the previous key opened.
#[derive(Debug)]
pub struct Opened {
    /// The credential's bytes.
    pub plaintext: Vec<u8>,
    /// `false` when only `CONNECTOR_SECRETS_KEY_PREVIOUS` opened it.
    pub with_current_key: bool,
}

impl Keyring {
    /// Build the ring from the two environment values. An empty or absent
    /// previous key means none.
    ///
    /// # Errors
    ///
    /// [`CryptoError::KeyMissing`] without a current key;
    /// [`CryptoError::KeyInvalid`] if either given key is malformed.
    pub fn from_values(current: Option<&str>, previous: Option<&str>) -> Result<Self, CryptoError> {
        let current = current
            .filter(|v| !v.trim().is_empty())
            .ok_or(CryptoError::KeyMissing)?;
        let previous = previous
            .filter(|v| !v.trim().is_empty())
            .map(SecretsKey::from_base64)
            .transpose()?;
        Ok(Self {
            current: SecretsKey::from_base64(current)?,
            previous,
        })
    }

    /// Seal `plaintext` for the file `file_name` with the CURRENT key.
    ///
    /// # Errors
    ///
    /// [`CryptoError::SealFailed`].
    pub fn seal(&self, file_name: &str, plaintext: &[u8]) -> Result<Vec<u8>, CryptoError> {
        self.current.seal(file_name, plaintext)
    }

    /// Open `sealed`, the content of the file `file_name`, with the current
    /// key, then the previous one.
    ///
    /// # Errors
    ///
    /// [`CryptoError::Unreadable`] if the bytes are not in the sealed
    /// format; [`CryptoError::WrongKey`] if neither key opens them.
    pub fn open(&self, file_name: &str, sealed: &[u8]) -> Result<Opened, CryptoError> {
        if !looks_sealed(sealed) {
            return Err(CryptoError::Unreadable);
        }
        if let Some(plaintext) = self.current.try_open(file_name, sealed) {
            return Ok(Opened {
                plaintext,
                with_current_key: true,
            });
        }
        if let Some(plaintext) = self
            .previous
            .as_ref()
            .and_then(|key| key.try_open(file_name, sealed))
        {
            return Ok(Opened {
                plaintext,
                with_current_key: false,
            });
        }
        Err(CryptoError::WrongKey)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    /// 32 bytes, base64: `0..31`, the key of the shared vector.
    const VECTOR: &str = include_str!("../tests/fixtures/connector_secret_vector.json");

    fn vector_field(name: &str) -> String {
        let json: serde_json::Value = serde_json::from_str(VECTOR).unwrap();
        json[name].as_str().unwrap().to_owned()
    }

    fn decode(b64: &str) -> Vec<u8> {
        base64::engine::general_purpose::STANDARD
            .decode(b64)
            .unwrap()
    }

    fn other_key() -> String {
        base64::engine::general_purpose::STANDARD.encode([7_u8; KEY_LEN])
    }

    /// The vector was written by Python's `cryptography`; opening it here is
    /// what proves the two implementations agree (decision 4's test).
    #[test]
    fn the_shared_vector_written_by_python_opens_here() {
        let ring = Keyring::from_values(Some(&vector_field("key_base64")), None).unwrap();
        let opened = ring
            .open(
                &vector_field("file_name"),
                &decode(&vector_field("sealed_base64")),
            )
            .unwrap();
        assert!(opened.with_current_key);
        assert_eq!(
            String::from_utf8(opened.plaintext).unwrap(),
            vector_field("plaintext")
        );
    }

    #[test]
    fn the_shared_vector_does_not_open_under_another_file_name() {
        let ring = Keyring::from_values(Some(&vector_field("key_base64")), None).unwrap();
        let err = ring
            .open(
                &vector_field("other_file_name"),
                &decode(&vector_field("sealed_base64")),
            )
            .unwrap_err();
        assert_eq!(err, CryptoError::WrongKey);
    }

    #[test]
    fn a_sealed_value_round_trips_with_a_fresh_nonce_each_time() {
        let ring = Keyring::from_values(Some(&other_key()), None).unwrap();
        let a = ring
            .seal("connector_managed_a_password", b"hunter2")
            .unwrap();
        let b = ring
            .seal("connector_managed_a_password", b"hunter2")
            .unwrap();
        assert_ne!(a, b, "two seals of one value must not repeat a nonce");
        assert_eq!(a[0], FORMAT_VERSION);
        assert!(!a.windows(7).any(|w| w == b"hunter2"));
        let opened = ring.open("connector_managed_a_password", &a).unwrap();
        assert_eq!(opened.plaintext, b"hunter2");
    }

    #[test]
    fn a_wrong_key_is_named_as_such_and_never_opens() {
        let writer = Keyring::from_values(Some(&other_key()), None).unwrap();
        let blob = writer.seal("connector_managed_a_password", b"v").unwrap();
        let stranger = Keyring::from_values(Some(&vector_field("key_base64")), None).unwrap();
        assert_eq!(
            stranger
                .open("connector_managed_a_password", &blob)
                .unwrap_err(),
            CryptoError::WrongKey
        );
    }

    #[test]
    fn a_file_copied_under_another_connectors_name_does_not_open() {
        let ring = Keyring::from_values(Some(&other_key()), None).unwrap();
        let sealed = ring.seal("connector_managed_a_password", b"v").unwrap();
        assert_eq!(
            ring.open("connector_managed_b_password", &sealed)
                .unwrap_err(),
            CryptoError::WrongKey
        );
    }

    #[test]
    fn a_previous_key_opens_but_the_current_key_is_the_one_that_seals() {
        let old = other_key();
        let new = vector_field("key_base64");
        let old_ring = Keyring::from_values(Some(&old), None).unwrap();
        let sealed = old_ring.seal("connector_managed_a_password", b"v").unwrap();

        let ring = Keyring::from_values(Some(&new), Some(&old)).unwrap();
        let opened = ring.open("connector_managed_a_password", &sealed).unwrap();
        assert!(!opened.with_current_key);

        let resealed = ring.seal("connector_managed_a_password", b"v").unwrap();
        let only_new = Keyring::from_values(Some(&new), None).unwrap();
        assert!(
            only_new
                .open("connector_managed_a_password", &resealed)
                .is_ok(),
            "a seal must be made with the current key, never the previous one"
        );
        assert!(
            old_ring
                .open("connector_managed_a_password", &resealed)
                .is_err()
        );
    }

    #[test]
    fn bytes_that_are_not_a_seal_are_unreadable_not_a_wrong_key() {
        let ring = Keyring::from_values(Some(&other_key()), None).unwrap();
        for bytes in [
            &b""[..],
            b"plain-password",
            &[FORMAT_VERSION; 5],
            &[9_u8; 40],
        ] {
            assert_eq!(
                ring.open("connector_managed_a_password", bytes)
                    .unwrap_err(),
                CryptoError::Unreadable
            );
        }
    }

    #[test]
    fn a_flipped_byte_does_not_open() {
        let ring = Keyring::from_values(Some(&other_key()), None).unwrap();
        let mut sealed = ring.seal("connector_managed_a_password", b"value").unwrap();
        let last = sealed.len() - 1;
        sealed[last] ^= 1;
        assert_eq!(
            ring.open("connector_managed_a_password", &sealed)
                .unwrap_err(),
            CryptoError::WrongKey
        );
    }

    #[test]
    fn keys_are_checked_and_never_printed() {
        assert_eq!(
            Keyring::from_values(None, None).unwrap_err(),
            CryptoError::KeyMissing
        );
        assert_eq!(
            Keyring::from_values(Some("  "), None).unwrap_err(),
            CryptoError::KeyMissing
        );
        for bad in ["not base64!", "c2hvcnQ="] {
            assert_eq!(
                Keyring::from_values(Some(bad), None).unwrap_err(),
                CryptoError::KeyInvalid,
                "{bad}"
            );
        }
        assert_eq!(
            Keyring::from_values(Some(&other_key()), Some("c2hvcnQ=")).unwrap_err(),
            CryptoError::KeyInvalid
        );
        let ring = Keyring::from_values(Some(&other_key()), Some("")).unwrap();
        let shown = format!("{ring:?}");
        assert!(shown.contains("redacted"));
        assert!(!shown.contains(&other_key()));
    }
}
