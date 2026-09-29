//! Writes and removes the credential FILES behind
//! [`CredentialSource::Managed`](lakehouse_store::connectors::CredentialSource::Managed)
//! refs — ADR 0002 Addendum 4
//! (`docs/adr/0002-secretref-resolution.md`).
//!
//! # Why this exists
//!
//! Addendum 3 left a user-created connector's credential to an operator:
//! the server derives a reference name, and someone with shell access to
//! the host sets an env var or drops a file under it. A user of the
//! console has no such access, so a connector they create can never be
//! tested or ingested without asking for it. This module is the write
//! path that closes that gap: the user types the credential once, the API
//! writes it to `/run/secrets/connector_managed_<id>_<suffix>`, and the
//! connector's ref names that file.
//!
//! # What does NOT change
//!
//! Reading. A managed ref is an ordinary `file:` ref matching the existing
//! `file:/run/secrets/connector_*` allowlist pattern, so the API's
//! `FileSecretResolver` (via `AppState::connector_secret_resolver`) and
//! Dagster's `secret_resolver.resolve_secret_ref` both read it exactly as
//! they read an operator-provisioned file. Nothing here widens what either
//! resolver will read.
//!
//! # Guarantees
//!
//! - **Only managed refs.** [`ConnectorSecretStore::write`] and
//!   [`ConnectorSecretStore::remove`] refuse any ref that does not start
//!   with [`MANAGED_SECRET_REF_PREFIX`] and consist of `[a-z0-9_]` after
//!   it. An operator's `env:`/`file:` credential, or a seeded,
//!   deployment-owned one, can never be overwritten or deleted through
//!   this module — and no path segment (`/`, `..`) can reach outside the
//!   store directory.
//! - **The value is never logged, printed, or returned.** It travels as a
//!   [`SecretValue`] (redacting `Debug`, no `Serialize`), and every error
//!   here names at most an [`std::io::ErrorKind`] — never a path containing
//!   the connector id alongside the value, never the value.
//! - **Atomic, owner-only writes.** A new value lands in a `0600` temporary
//!   file in the same directory, is `fsync`ed, then renamed over the final
//!   name, so a concurrent reader sees either the old credential or the new
//!   one, never a half-written file.
//!
//! # Honest limits
//!
//! The file is plaintext at rest on the `connector_secrets` volume — the
//! same posture as a Docker/Compose secret. It is not part of a database
//! backup; the volume must be backed up on its own. Encrypting it (or
//! moving it to Vault) is Addendum 4's named follow-up, and would replace
//! this module without changing any route or resolver.

use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;
use std::sync::Arc;

use lakehouse_core::secret::{DynSecretResolver, SecretError, SecretResolver, SecretValue};
use lakehouse_store::connectors::MANAGED_SECRET_REF_PREFIX;

/// Upper bound on a stored credential. Generous enough for an RSA private
/// key PEM (the `sftp` adapter's `private_key` kind); anything larger is
/// not a credential this build knows how to use.
pub const MAX_SECRET_BYTES: usize = 16 * 1024;

/// Why a credential could not be stored or removed. Every variant's
/// message is safe to show a caller: none carries the value, and I/O
/// failures are reduced to their [`std::io::ErrorKind`].
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SecretStoreError {
    /// The ref is not a `CredentialSource::Managed` ref — see the module
    /// doc comment's first guarantee.
    #[error("not a lakehouse-managed credential reference")]
    NotManaged,
    /// The value itself is unusable — see [`validate_secret_value`].
    #[error("{0}")]
    InvalidValue(&'static str),
    /// A filesystem operation failed.
    #[error(
        "the connector credential store could not be written ({0:?}); check that the connector_secrets volume is mounted writable at /run/secrets"
    )]
    Io(std::io::ErrorKind),
}

/// Refuse a credential value that would not survive a round trip through
/// the resolvers, or that is not a credential at all.
///
/// Both resolvers read a `file:` credential back `trim()`med
/// (`FileSecretResolver` in Rust, `.strip()` in Dagster's
/// `secret_resolver.py`). A value with leading or trailing whitespace would
/// therefore be stored intact and silently read back different — a
/// password that "was saved" yet never authenticates. Refusing it here, with
/// a message that says why, is the honest option.
///
/// # Errors
///
/// [`SecretStoreError::InvalidValue`] naming the problem.
pub fn validate_secret_value(value: &str) -> Result<(), SecretStoreError> {
    if value.trim().is_empty() {
        return Err(SecretStoreError::InvalidValue(
            "the credential must not be empty",
        ));
    }
    if value.trim() != value {
        return Err(SecretStoreError::InvalidValue(
            "the credential must not start or end with whitespace: it is read back trimmed, so \
             it would never match what the source database expects",
        ));
    }
    if value.len() > MAX_SECRET_BYTES {
        return Err(SecretStoreError::InvalidValue(
            "the credential is too large (max 16 KiB)",
        ));
    }
    if value.contains('\0') {
        return Err(SecretStoreError::InvalidValue(
            "the credential must not contain a NUL byte",
        ));
    }
    Ok(())
}

/// The directory managed credential files live in, and the only thing
/// that writes or removes them. See the module doc comment.
#[derive(Debug, Clone)]
pub struct ConnectorSecretStore {
    dir: PathBuf,
}

impl ConnectorSecretStore {
    /// A store rooted at `dir`. In a deployment this is `/run/secrets`, the
    /// same fixed base directory `FileSecretResolver` reads from — the two
    /// must agree, or a stored credential is never found.
    #[must_use]
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// The bare file name a managed `secret_ref` maps to, e.g.
    /// `file:/run/secrets/connector_managed_conn_x_password` →
    /// `connector_managed_conn_x_password`.
    fn file_name(secret_ref: &str) -> Result<String, SecretStoreError> {
        let tail = secret_ref
            .strip_prefix(MANAGED_SECRET_REF_PREFIX)
            .ok_or(SecretStoreError::NotManaged)?;
        if tail.is_empty()
            || !tail
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        {
            return Err(SecretStoreError::NotManaged);
        }
        Ok(format!("connector_managed_{tail}"))
    }

    /// Store `value` as the credential behind `secret_ref`, replacing any
    /// previous one atomically. See the module doc comment.
    ///
    /// # Errors
    ///
    /// [`SecretStoreError::NotManaged`] for a ref this module must not
    /// touch; [`SecretStoreError::InvalidValue`] per
    /// [`validate_secret_value`]; [`SecretStoreError::Io`] if the store
    /// directory is missing or not writable.
    pub async fn write(
        &self,
        secret_ref: &str,
        value: &SecretValue,
    ) -> Result<(), SecretStoreError> {
        let name = Self::file_name(secret_ref)?;
        validate_secret_value(value.expose_secret())?;
        let dir = self.dir.clone();
        let value = value.clone();
        tokio::task::spawn_blocking(move || write_atomically(&dir, &name, &value))
            .await
            .map_err(|_| SecretStoreError::Io(std::io::ErrorKind::Other))?
    }

    /// Delete the credential behind `secret_ref`. Removing one that is
    /// already gone is not an error — the outcome the caller wants holds.
    ///
    /// # Errors
    ///
    /// [`SecretStoreError::NotManaged`] for a ref this module must not
    /// touch; [`SecretStoreError::Io`] for any other failure.
    pub async fn remove(&self, secret_ref: &str) -> Result<(), SecretStoreError> {
        let path = self.dir.join(Self::file_name(secret_ref)?);
        tokio::task::spawn_blocking(move || match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(SecretStoreError::Io(err.kind())),
        })
        .await
        .map_err(|_| SecretStoreError::Io(std::io::ErrorKind::Other))?
    }
}

/// Write-to-temp, `fsync`, rename. The temporary name is a dotfile no
/// derived ref can ever name, created with `O_EXCL` so an existing file
/// (or symlink) at that name is never followed or clobbered.
fn write_atomically(
    dir: &std::path::Path,
    name: &str,
    value: &SecretValue,
) -> Result<(), SecretStoreError> {
    let io = |err: std::io::Error| SecretStoreError::Io(err.kind());
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let tmp = dir.join(format!(".{name}.{}.{nonce}.tmp", std::process::id()));
    let result = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp)
            .map_err(io)?;
        file.write_all(value.expose_secret().as_bytes())
            .map_err(io)?;
        file.sync_all().map_err(io)?;
        std::fs::rename(&tmp, dir.join(name)).map_err(io)?;
        // Best-effort: persist the rename itself. A failure here leaves the
        // new file in place and readable, so it is not reported.
        if let Ok(handle) = std::fs::File::open(dir) {
            let _ = handle.sync_all();
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// Resolves the CANDIDATE credentials a caller has not stored yet — one
/// per slot being changed — from memory, and delegates every other ref to
/// `fallback`. Lets `routes::connectors::set_credential` probe new
/// credentials before writing them anywhere, the same probe-first contract
/// `rotate_secret` keeps. Holding several at once is what lets an S3
/// access-key/secret-key pair be changed together: probing the new access
/// key alongside the OLD secret key would always fail.
///
/// It does not widen what can be read: every candidate ref is a
/// server-derived managed ref (which the allowlist admits anyway), and
/// every other ref still goes through the allowlisted `fallback`.
pub struct CandidateSecretResolver {
    candidates: Vec<(String, SecretValue)>,
    fallback: Arc<dyn DynSecretResolver>,
}

impl CandidateSecretResolver {
    /// Answer each `(ref, value)` in `candidates` from memory; ask
    /// `fallback` for the rest.
    #[must_use]
    pub fn new(
        candidates: Vec<(String, SecretValue)>,
        fallback: Arc<dyn DynSecretResolver>,
    ) -> Self {
        Self {
            candidates,
            fallback,
        }
    }
}

impl std::fmt::Debug for CandidateSecretResolver {
    /// Never prints a candidate value (or the refs it answers for).
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CandidateSecretResolver")
            .finish_non_exhaustive()
    }
}

impl SecretResolver for CandidateSecretResolver {
    async fn resolve(&self, secret_ref: &str) -> Result<SecretValue, SecretError> {
        match self.candidates.iter().find(|(r, _)| r == secret_ref) {
            Some((_, value)) => Ok(value.clone()),
            None => self.fallback.resolve_dyn(secret_ref).await,
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::collections::HashMap;
    use std::os::unix::fs::PermissionsExt;

    use lakehouse_core::secret::EnvSecretResolver;
    use lakehouse_store::connectors::{CredentialKind, CredentialSource, derive_secret_ref};

    use super::*;

    fn managed_ref(id: &str) -> String {
        derive_secret_ref(id, CredentialSource::Managed, CredentialKind::Password)
    }

    #[tokio::test]
    async fn write_stores_the_value_owner_only_where_the_resolver_looks() {
        let dir = tempfile::tempdir().unwrap();
        let store = ConnectorSecretStore::new(dir.path());
        let secret_ref = managed_ref("conn-orders-k3x9");
        store
            .write(&secret_ref, &SecretValue::new("s3cret-pass"))
            .await
            .unwrap();

        let path = dir
            .path()
            .join("connector_managed_conn_orders_k3x9_password");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "s3cret-pass");
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "a stored credential must be owner-only");
    }

    #[tokio::test]
    async fn write_replaces_the_previous_value_and_leaves_no_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        let store = ConnectorSecretStore::new(dir.path());
        let secret_ref = managed_ref("conn-a");
        store
            .write(&secret_ref, &SecretValue::new("first"))
            .await
            .unwrap();
        store
            .write(&secret_ref, &SecretValue::new("second"))
            .await
            .unwrap();

        let entries: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            entries,
            vec!["connector_managed_conn_a_password".to_owned()]
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("connector_managed_conn_a_password")).unwrap(),
            "second"
        );
    }

    /// The first guarantee: an operator's `file:`/`env:` ref, a seeded
    /// ref, or anything carrying a path segment is refused — nothing is
    /// written or deleted.
    #[tokio::test]
    async fn refuses_every_ref_that_is_not_managed() {
        let dir = tempfile::tempdir().unwrap();
        let store = ConnectorSecretStore::new(dir.path());
        let operator_file =
            derive_secret_ref("conn-a", CredentialSource::File, CredentialKind::Password);
        let operator_env =
            derive_secret_ref("conn-a", CredentialSource::Env, CredentialKind::Password);
        for bad in [
            operator_file.as_str(),
            operator_env.as_str(),
            "env:CONNECTOR_PG_PASSWORD",
            "file:/run/secrets/connector_managed_",
            "file:/run/secrets/connector_managed_../../etc/passwd",
            "file:/run/secrets/connector_managed_a/b",
            "file:/run/secrets/connector_managed_UPPER",
        ] {
            let value = SecretValue::new("x");
            assert_eq!(
                store.write(bad, &value).await,
                Err(SecretStoreError::NotManaged),
                "{bad}"
            );
            assert_eq!(
                store.remove(bad).await,
                Err(SecretStoreError::NotManaged),
                "{bad}"
            );
        }
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn validate_refuses_values_the_resolvers_would_read_back_differently() {
        assert!(validate_secret_value("correct horse battery staple").is_ok());
        for bad in [
            "",
            "   ",
            " leading",
            "trailing ",
            "trailing\n",
            "nul\0byte",
        ] {
            assert!(validate_secret_value(bad).is_err(), "{bad:?}");
        }
        assert!(validate_secret_value(&"a".repeat(MAX_SECRET_BYTES + 1)).is_err());
    }

    #[tokio::test]
    async fn an_invalid_value_is_never_written() {
        let dir = tempfile::tempdir().unwrap();
        let store = ConnectorSecretStore::new(dir.path());
        let result = store
            .write(&managed_ref("conn-a"), &SecretValue::new(" padded "))
            .await;
        assert!(matches!(result, Err(SecretStoreError::InvalidValue(_))));
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[tokio::test]
    async fn remove_deletes_and_tolerates_an_already_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let store = ConnectorSecretStore::new(dir.path());
        let secret_ref = managed_ref("conn-a");
        store
            .write(&secret_ref, &SecretValue::new("v"))
            .await
            .unwrap();
        store.remove(&secret_ref).await.unwrap();
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
        store.remove(&secret_ref).await.unwrap();
    }

    #[tokio::test]
    async fn a_missing_store_directory_is_an_io_error_not_a_panic() {
        let store = ConnectorSecretStore::new("/nonexistent/connector-secrets");
        let result = store
            .write(&managed_ref("conn-a"), &SecretValue::new("v"))
            .await;
        assert_eq!(
            result,
            Err(SecretStoreError::Io(std::io::ErrorKind::NotFound))
        );
    }

    #[tokio::test]
    async fn candidate_resolver_answers_its_own_ref_and_delegates_the_rest() {
        let mut map = HashMap::new();
        map.insert("OTHER_PASSWORD".to_owned(), "from-fallback".to_owned());
        let fallback: Arc<dyn DynSecretResolver> = Arc::new(EnvSecretResolver::with_map(map));
        let secondary = derive_secret_ref(
            "conn-a",
            CredentialSource::Managed,
            CredentialKind::SecretKey,
        );
        let resolver = CandidateSecretResolver::new(
            vec![
                (managed_ref("conn-a"), SecretValue::new("candidate")),
                (secondary.clone(), SecretValue::new("candidate-2")),
            ],
            fallback,
        );
        assert_eq!(
            resolver
                .resolve(&managed_ref("conn-a"))
                .await
                .unwrap()
                .expose_secret(),
            "candidate"
        );
        assert_eq!(
            resolver.resolve(&secondary).await.unwrap().expose_secret(),
            "candidate-2"
        );
        assert_eq!(
            resolver
                .resolve("env:OTHER_PASSWORD")
                .await
                .unwrap()
                .expose_secret(),
            "from-fallback"
        );
        assert!(!format!("{resolver:?}").contains("candidate"));
    }
}
