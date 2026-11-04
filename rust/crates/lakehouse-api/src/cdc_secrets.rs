//! The source password of a change-capture connector, on its way to `Kafka
//! Connect` (`SRC-4` slice 2, decision S2-a, ADR 0016 point 4).
//!
//! # Why a file the API writes
//!
//! A managed credential is sealed with AES-256-GCM (ADR 0002 addendum 5) and
//! only the API and the orchestrator can open it, and an external
//! secret-manager reference must pass `SECRET_MANAGER_ALLOWED_PREFIXES` at
//! every use (addendum 6). `Kafka Connect` can do neither, and its connector
//! JSON is stored in a topic and returned by its REST API, so the password
//! cannot be a literal in it. So the API resolves the connector's `secretRef`
//! through its own allowlisted resolver
//! ([`crate::state::AppState::connector_secret_resolver`]) and writes the plain
//! value to `<CDC_SECRETS_DIR>/<connector id>`, and the connector JSON names it
//! with `Kafka Connect`'s `DirectoryConfigProvider`
//! (`${dir:/run/cdc-secrets:<id>}`,
//! [`lakehouse_store::cdc::connect_password_reference`]).
//!
//! # Where the plain value lives
//!
//! Only on a `tmpfs` volume (`lakehouse_cdc_secrets`), read-write in
//! `lakehouse-api` and read-only in `kafka-connect`, mounted nowhere else. It
//! is memory only: it never reaches a disk, a backup or an image layer. That
//! is the same exposure as the worker's own process memory, which holds the
//! password to dial the source anyway. `tmpfs` empties on a restart, so the
//! API rewrites every file on start and on every provisioning call.
//!
//! # File mode
//!
//! `0444`, not `0400` as the plan first wrote it: the two containers run as
//! different users, and a file only the API can read is a file `Kafka Connect`
//! cannot. The directory is mounted only in those two containers, so the
//! volume, not the mode, is the boundary.
//!
//! # Guarantees
//!
//! - The id is validated as a plain identifier before it names a file, so no
//!   path segment can reach outside the directory.
//! - A file is written to a temporary name and renamed over, so the worker
//!   reads the old password or the new one, never half of one.
//! - Errors are fixed sentences; none carries the value or the path.

use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;

use lakehouse_core::secret::{DynSecretResolver, SecretValue};
use lakehouse_store::cdc::connect_connector_name;

/// What a [`CdcSecretStore::write`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteOutcome {
    /// There was no file; there is one now.
    Created,
    /// The file held a different value; it holds this one now. The worker
    /// reads the password when a task starts, so the connector must be
    /// restarted for it to take effect.
    Changed,
    /// The file already held this value.
    Unchanged,
}

/// Why a password could not be put in place. Every message is fixed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CdcSecretError {
    /// The connector id is not a plain identifier.
    #[error("the connector id cannot name a change-capture secret")]
    InvalidId,
    /// The connector's `secretRef` did not resolve (unset, not allowed, or
    /// unsupported).
    #[error("the connector's credential could not be resolved")]
    Unresolved,
    /// The secrets directory could not be written.
    #[error("the change-capture secrets directory is not writable")]
    Io,
}

/// The directory behind `lakehouse_cdc_secrets` (`Config::cdc_secrets_dir`), and the only thing that
/// writes it. See the module docs.
#[derive(Debug, Clone)]
pub struct CdcSecretStore {
    dir: PathBuf,
}

impl CdcSecretStore {
    /// A store rooted at `dir`.
    #[must_use]
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// The file name for `connector_id`, after validating the id.
    fn file_name(connector_id: &str) -> Result<String, CdcSecretError> {
        // The validation that makes the id a connector name is the one that
        // makes it a file name: no separator, no `:`, no control character.
        connect_connector_name(connector_id)
            .map(|_| connector_id.to_owned())
            .map_err(|_| CdcSecretError::InvalidId)
    }

    /// Put `value` in `connector_id`'s file.
    ///
    /// # Errors
    ///
    /// [`CdcSecretError::InvalidId`] or [`CdcSecretError::Io`].
    pub fn write(
        &self,
        connector_id: &str,
        value: &SecretValue,
    ) -> Result<WriteOutcome, CdcSecretError> {
        let name = Self::file_name(connector_id)?;
        let live = self.dir.join(&name);
        let wanted = value.expose_secret().as_bytes();
        let existed = match std::fs::read(&live) {
            Ok(current) if current == wanted => return Ok(WriteOutcome::Unchanged),
            Ok(_) => true,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => false,
            Err(_) => return Err(CdcSecretError::Io),
        };
        // A dotfile no connector id can name: ids start with a letter or digit.
        let tmp = self.dir.join(format!(".{name}.{}.tmp", std::process::id()));
        let _ = std::fs::remove_file(&tmp);
        let written = (|| {
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o444)
                .open(&tmp)?;
            file.write_all(wanted)?;
            file.sync_all()?;
            std::fs::rename(&tmp, &live)
        })();
        if written.is_err() {
            let _ = std::fs::remove_file(&tmp);
            return Err(CdcSecretError::Io);
        }
        Ok(if existed {
            WriteOutcome::Changed
        } else {
            WriteOutcome::Created
        })
    }

    /// Remove `connector_id`'s file. A file that is not there is success.
    ///
    /// # Errors
    ///
    /// [`CdcSecretError::InvalidId`] or [`CdcSecretError::Io`].
    pub fn remove(&self, connector_id: &str) -> Result<(), CdcSecretError> {
        let name = Self::file_name(connector_id)?;
        match std::fs::remove_file(self.dir.join(name)) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err(CdcSecretError::Io),
        }
    }

    /// Resolve `secret_ref` through `resolver` (the connector resolver, with
    /// its allowlist) and write the value to `connector_id`'s file. The
    /// resolver's error text is dropped: it can name the reference.
    ///
    /// # Errors
    ///
    /// [`CdcSecretError::Unresolved`] when the reference does not resolve,
    /// otherwise as [`Self::write`].
    pub async fn materialise(
        &self,
        resolver: &dyn DynSecretResolver,
        connector_id: &str,
        secret_ref: &str,
    ) -> Result<WriteOutcome, CdcSecretError> {
        Self::file_name(connector_id)?;
        let value = resolver
            .resolve_dyn(secret_ref)
            .await
            .map_err(|_| CdcSecretError::Unresolved)?;
        self.write(connector_id, &value)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::os::unix::fs::PermissionsExt;

    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("cdc-secrets-test-{name}-{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_value_is_written_once_changed_and_unchanged() {
        let dir = temp_dir("write");
        let store = CdcSecretStore::new(&dir);
        let id = "conn-orders-ab12";
        assert!(!dir.join(id).exists());
        assert_eq!(
            store
                .write(id, &SecretValue::new("s3cret".to_owned()))
                .unwrap(),
            WriteOutcome::Created
        );
        assert!(dir.join(id).exists());
        assert_eq!(std::fs::read_to_string(dir.join(id)).unwrap(), "s3cret");
        let mode = std::fs::metadata(dir.join(id))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o444);
        assert_eq!(
            store
                .write(id, &SecretValue::new("s3cret".to_owned()))
                .unwrap(),
            WriteOutcome::Unchanged
        );
        assert_eq!(
            store
                .write(id, &SecretValue::new("rotated".to_owned()))
                .unwrap(),
            WriteOutcome::Changed
        );
        assert_eq!(std::fs::read_to_string(dir.join(id)).unwrap(), "rotated");
        // Nothing but the one file is left behind.
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        store.remove(id).unwrap();
        store.remove(id).unwrap();
        assert!(!dir.join(id).exists());
    }

    #[test]
    fn an_id_that_could_leave_the_directory_is_refused() {
        let dir = temp_dir("ids");
        let store = CdcSecretStore::new(&dir);
        for id in ["", "../x", "a/b", "a:b", ".hidden", "-a"] {
            assert_eq!(
                store.write(id, &SecretValue::new("x".to_owned())),
                Err(CdcSecretError::InvalidId),
                "{id}"
            );
            assert_eq!(store.remove(id), Err(CdcSecretError::InvalidId), "{id}");
        }
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
    }

    #[test]
    fn a_missing_directory_is_a_fixed_error_that_names_neither_path_nor_value() {
        let store = CdcSecretStore::new("/nonexistent-cdc-secrets-dir");
        let err = store
            .write("conn-a", &SecretValue::new("hunter2".to_owned()))
            .unwrap_err();
        assert_eq!(err, CdcSecretError::Io);
        let text = err.to_string();
        assert!(!text.contains("hunter2") && !text.contains("nonexistent"));
    }
}
