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
//! - **A pair changes together or not at all.** [`ConnectorSecretStore::stage`]
//!   writes every new value aside first; [`StagedCredentials::publish`]
//!   puts them in place while keeping each replaced file, so
//!   [`PublishedCredentials::rollback`] can restore the old pair exactly
//!   when the ref change that should follow fails. Without this, an S3
//!   access key replaced in place could end up beside the old secret key.
//!
//! # Encrypted at rest (`SRC-12`, decision 4, ADR 0002 addendum 5)
//!
//! What lands on the volume is not the value but its AES-256-GCM seal
//! ([`crate::secret_crypto`]), under the installation key
//! `CONNECTOR_SECRETS_KEY`. Only this store seals; readers open
//! ([`crate::state`]'s resolver, and the orchestrator's). At start-up
//! [`ConnectorSecretStore::startup_pass`] seals any plain file an older
//! build wrote and removes temporary and backup files an interrupted change
//! left behind (decision 9: no older value survives on disk), and
//! [`ConnectorSecretStore::reseal_all`] is the key-change command's engine
//! (decision 5).
//!
//! # Honest limits
//!
//! The key sits in the same environment as the API and the orchestrator, so
//! this protects a copied volume or backup, not a compromised process. The
//! volume is still not part of a database backup; back it up with
//! `lakehouse_postgres_data`, and keep the key where the backup is not.
//! Losing the key loses every managed credential.

use std::io::Write;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use lakehouse_core::secret::{DynSecretResolver, SecretError, SecretResolver, SecretValue};
use lakehouse_store::connectors::MANAGED_SECRET_REF_PREFIX;

use crate::secret_crypto::{CryptoError, Keyring, MANAGED_FILE_PREFIX, looks_sealed};

/// The keys as [`crate::state::AppState`] resolved them from the
/// environment, kept as a `Result` so a missing or malformed key surfaces
/// as its own fixed text on the first credential write instead of one
/// vague "cannot store".
pub type SharedKeyring = Result<Arc<Keyring>, CryptoError>;

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
        "the connector credential store could not be written ({0:?}); check that the connector_secrets volume is mounted writable (CONNECTOR_SECRETS_DIR, /run/secrets by default)"
    )]
    Io(std::io::ErrorKind),
    /// The installation key is missing or malformed (`SRC-12` decision 12:
    /// a fixed text naming the setting, never a key).
    #[error("{0}")]
    Key(CryptoError),
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
    /// Seals what is written; see [`SharedKeyring`].
    keyring: SharedKeyring,
    /// Serializes credential CHANGES (see [`Self::lock`]). Shared by every
    /// clone, so the whole process has one.
    changes: Arc<tokio::sync::Mutex<()>>,
}

impl ConnectorSecretStore {
    /// A store rooted at `dir`. In a deployment this is `/run/secrets`, the
    /// same fixed base directory `FileSecretResolver` reads from — the two
    /// must agree, or a stored credential is never found.
    #[must_use]
    pub fn new(dir: impl Into<PathBuf>, keyring: SharedKeyring) -> Self {
        Self {
            dir: dir.into(),
            keyring,
            changes: Arc::new(tokio::sync::Mutex::new(())),
        }
    }

    /// The keys, or the reason there are none.
    ///
    /// # Errors
    ///
    /// [`SecretStoreError::Key`] when `CONNECTOR_SECRETS_KEY` is missing or
    /// malformed.
    pub fn keyring(&self) -> Result<&Keyring, SecretStoreError> {
        self.keyring
            .as_deref()
            .map_err(|err| SecretStoreError::Key(*err))
    }

    /// Hold while changing a connector's credentials, from staging the new
    /// values to committing or rolling them back. Two changes to the same
    /// file interleaving could otherwise restore each other's backups.
    /// Credential changes are rare, so one lock for every connector costs
    /// nothing worth a finer one.
    pub async fn lock(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.changes.lock().await
    }

    /// Write every `(ref, value)` aside, without touching a live
    /// credential, ready to [`StagedCredentials::publish`] together. Refs
    /// and values are validated before anything is written.
    ///
    /// # Errors
    ///
    /// As [`Self::write`]. Nothing is left behind on failure.
    pub async fn stage(
        &self,
        values: &[(String, SecretValue)],
    ) -> Result<StagedCredentials, SecretStoreError> {
        let keyring = self.keyring()?;
        let mut names = Vec::with_capacity(values.len());
        let mut sealed = Vec::with_capacity(values.len());
        for (secret_ref, value) in values {
            let name = Self::file_name(secret_ref)?;
            validate_secret_value(value.expose_secret())?;
            sealed.push(
                keyring
                    .seal(&name, value.expose_secret().as_bytes())
                    .map_err(SecretStoreError::Key)?,
            );
            names.push(name);
        }
        let dir = self.dir.clone();
        tokio::task::spawn_blocking(move || {
            let mut entries = Vec::with_capacity(names.len());
            for (name, bytes) in names.into_iter().zip(&sealed) {
                match write_staged(&dir, &name, bytes) {
                    Ok(staged) => entries.push(StagedFile { name, staged }),
                    Err(err) => {
                        remove_staged(&entries);
                        return Err(err);
                    }
                }
            }
            Ok(StagedCredentials { dir, entries })
        })
        .await
        .map_err(|_| SecretStoreError::Io(std::io::ErrorKind::Other))?
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
        let sealed = self
            .keyring()?
            .seal(&name, value.expose_secret().as_bytes())
            .map_err(SecretStoreError::Key)?;
        let dir = self.dir.clone();
        tokio::task::spawn_blocking(move || write_atomically(&dir, &name, &sealed))
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

    /// Start-up pass (`SRC-12` decisions 4 and 9). Seals every plain managed
    /// file an older build left (sealed copy written and `fsync`ed, then
    /// renamed over the plain one, so the plain value is gone), and removes
    /// the temporary and backup files of an interrupted change, so no older
    /// value survives on disk. Files already sealed are left alone, even
    /// when only the previous key opens them: moving those to the current
    /// key is [`Self::reseal_all`]'s job (decision 5).
    ///
    /// Run before the API serves, so no change is in flight. Reports names,
    /// never contents.
    ///
    /// # Errors
    ///
    /// [`SecretStoreError::Key`] without a usable key;
    /// [`SecretStoreError::Io`] if the directory cannot be listed.
    pub async fn startup_pass(&self) -> Result<ScanReport, SecretStoreError> {
        let keyring = self.keyring()?.clone();
        let dir = self.dir.clone();
        tokio::task::spawn_blocking(move || scan_dir(&dir, &keyring, false))
            .await
            .map_err(|_| SecretStoreError::Io(std::io::ErrorKind::Other))?
    }

    /// The key-change command's engine (`SRC-12` decision 5): re-seal every
    /// managed file with the CURRENT key, whichever key sealed it (the
    /// previous key must still be configured to open the old ones). A file
    /// neither key opens is reported and left untouched. Also removes
    /// leftovers, like [`Self::startup_pass`].
    ///
    /// A file replaced by the API between this command reading and renaming
    /// it is detected (inode, size and modification time are compared just
    /// before the rename) and reported as failed rather than overwritten;
    /// the window is small but not zero, so run it while no credential is
    /// being changed.
    ///
    /// # Errors
    ///
    /// As [`Self::startup_pass`].
    pub async fn reseal_all(&self) -> Result<ScanReport, SecretStoreError> {
        let keyring = self.keyring()?.clone();
        let dir = self.dir.clone();
        tokio::task::spawn_blocking(move || scan_dir(&dir, &keyring, true))
            .await
            .map_err(|_| SecretStoreError::Io(std::io::ErrorKind::Other))?
    }
}

/// New values written aside by [`ConnectorSecretStore::stage`]. Not live
/// until [`Self::publish`].
#[must_use = "staged credentials are not live until published"]
#[derive(Debug)]
pub struct StagedCredentials {
    dir: PathBuf,
    entries: Vec<StagedFile>,
}

#[derive(Debug)]
struct StagedFile {
    name: String,
    staged: PathBuf,
}

impl StagedCredentials {
    /// Put every staged value in place. A live file being replaced is kept
    /// (hard-linked aside first) so [`PublishedCredentials::rollback`] can
    /// restore it; each rename is atomic, so a reader sees the old value or
    /// the new one.
    ///
    /// # Errors
    ///
    /// [`SecretStoreError::Io`]; everything already put in place by this
    /// call is restored first, so a failure changes nothing.
    pub async fn publish(self) -> Result<PublishedCredentials, SecretStoreError> {
        tokio::task::spawn_blocking(move || {
            let Self { dir, entries } = self;
            let mut published: Vec<PublishedFile> = Vec::with_capacity(entries.len());
            for (i, entry) in entries.iter().enumerate() {
                match publish_one(&dir, entry) {
                    Ok(file) => published.push(file),
                    Err(err) => {
                        restore(&dir, &published);
                        remove_staged(&entries[i..]);
                        return Err(err);
                    }
                }
            }
            sync_dir(&dir);
            Ok(PublishedCredentials {
                dir,
                entries: published,
            })
        })
        .await
        .map_err(|_| SecretStoreError::Io(std::io::ErrorKind::Other))?
    }
}

/// Values [`StagedCredentials::publish`] put in place, with the files they
/// replaced kept aside until [`Self::commit`] (the change stands) or
/// [`Self::rollback`] (it does not).
#[must_use = "published credentials keep their replaced files until committed or rolled back"]
#[derive(Debug)]
pub struct PublishedCredentials {
    dir: PathBuf,
    entries: Vec<PublishedFile>,
}

#[derive(Debug)]
struct PublishedFile {
    name: String,
    /// The replaced file, when there was one.
    backup: Option<PathBuf>,
}

impl PublishedCredentials {
    /// The change stands: drop the replaced files.
    pub async fn commit(self) {
        let _ = tokio::task::spawn_blocking(move || {
            for entry in &self.entries {
                if let Some(backup) = &entry.backup {
                    let _ = std::fs::remove_file(backup);
                }
            }
            sync_dir(&self.dir);
        })
        .await;
    }

    /// The change did not happen: put every replaced file back and remove
    /// every new one, leaving the files exactly as they were before
    /// [`StagedCredentials::publish`].
    pub async fn rollback(self) {
        let _ = tokio::task::spawn_blocking(move || restore(&self.dir, &self.entries)).await;
    }
}

/// Distinguishes temporary names made within the same nanosecond.
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A dotfile name no derived ref can ever name, unique to this call.
fn temp_path(dir: &Path, name: &str, purpose: &str) -> PathBuf {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let count = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    dir.join(format!(
        ".{name}.{}.{nonce}.{count}.{purpose}",
        std::process::id()
    ))
}

/// Write `bytes` (a sealed credential) to a new `0600` temporary file and
/// `fsync` it. Created with `O_EXCL`, so an existing file (or symlink) at
/// that name is never followed or clobbered.
fn write_staged(dir: &Path, name: &str, bytes: &[u8]) -> Result<PathBuf, SecretStoreError> {
    let io = |err: std::io::Error| SecretStoreError::Io(err.kind());
    let tmp = temp_path(dir, name, "tmp");
    let result = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp)
            .map_err(io)?;
        file.write_all(bytes).map_err(io)?;
        file.sync_all().map_err(io)
    })();
    match result {
        Ok(()) => Ok(tmp),
        Err(err) => {
            let _ = std::fs::remove_file(&tmp);
            Err(err)
        }
    }
}

/// Keep the live file (if any) aside, then rename the staged one over it.
fn publish_one(dir: &Path, entry: &StagedFile) -> Result<PublishedFile, SecretStoreError> {
    let io = |err: std::io::Error| SecretStoreError::Io(err.kind());
    let live = dir.join(&entry.name);
    let backup = match std::fs::symlink_metadata(&live) {
        Ok(_) => {
            let backup = temp_path(dir, &entry.name, "bak");
            std::fs::hard_link(&live, &backup).map_err(io)?;
            Some(backup)
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
        Err(err) => return Err(io(err)),
    };
    if let Err(err) = std::fs::rename(&entry.staged, &live) {
        if let Some(backup) = &backup {
            let _ = std::fs::remove_file(backup);
        }
        return Err(io(err));
    }
    Ok(PublishedFile {
        name: entry.name.clone(),
        backup,
    })
}

/// Undo [`publish_one`] for each of `entries`, newest first.
fn restore(dir: &Path, entries: &[PublishedFile]) {
    for entry in entries.iter().rev() {
        let live = dir.join(&entry.name);
        match &entry.backup {
            Some(backup) => {
                let _ = std::fs::rename(backup, &live);
            }
            None => {
                let _ = std::fs::remove_file(&live);
            }
        }
    }
    sync_dir(dir);
}

fn remove_staged(entries: &[StagedFile]) {
    for entry in entries {
        let _ = std::fs::remove_file(&entry.staged);
    }
}

/// Best-effort: persist renames in `dir`. A failure leaves the files in
/// place and readable, so it is not reported.
fn sync_dir(dir: &Path) {
    if let Ok(handle) = std::fs::File::open(dir) {
        let _ = handle.sync_all();
    }
}

/// Write-to-temp, `fsync`, rename — one value, replacing any previous one.
fn write_atomically(dir: &Path, name: &str, bytes: &[u8]) -> Result<(), SecretStoreError> {
    let tmp = write_staged(dir, name, bytes)?;
    if let Err(err) = std::fs::rename(&tmp, dir.join(name)) {
        let _ = std::fs::remove_file(&tmp);
        return Err(SecretStoreError::Io(err.kind()));
    }
    sync_dir(dir);
    Ok(())
}

/// What [`ConnectorSecretStore::startup_pass`] and
/// [`ConnectorSecretStore::reseal_all`] did. File NAMES only (a name carries
/// a connector id, never a value).
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ScanReport {
    /// Plain files that are now sealed.
    pub sealed: Vec<String>,
    /// Sealed files moved from the previous key to the current one.
    pub resealed: Vec<String>,
    /// Sealed files already under the current key (or, in the start-up
    /// pass, under the previous one, left for the key-change command).
    pub unchanged: usize,
    /// Temporary and backup files of an interrupted change, now removed.
    pub leftovers_removed: Vec<String>,
    /// Files that could not be made usable: sealed under an unknown key,
    /// not in either format, or changed while being re-sealed.
    pub unreadable: Vec<String>,
}

/// A leftover of [`write_staged`] / [`publish_one`]: `.{name}.….tmp|bak`.
fn is_leftover(file_name: &str) -> bool {
    file_name.starts_with(".connector_managed_")
        && Path::new(file_name)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("tmp") || ext.eq_ignore_ascii_case("bak"))
}

/// Walk the store directory once: remove leftovers, then seal plain managed
/// files and (when `move_to_current`) move previous-key files to the
/// current key.
fn scan_dir(
    dir: &Path,
    keyring: &Keyring,
    move_to_current: bool,
) -> Result<ScanReport, SecretStoreError> {
    let io = |err: std::io::Error| SecretStoreError::Io(err.kind());
    let mut report = ScanReport::default();
    let mut names: Vec<String> = Vec::new();
    for entry in std::fs::read_dir(dir).map_err(io)? {
        let entry = entry.map_err(io)?;
        if let Some(name) = entry.file_name().to_str() {
            names.push(name.to_owned());
        }
    }
    names.sort();
    for name in names {
        if is_leftover(&name) {
            match std::fs::remove_file(dir.join(&name)) {
                Ok(()) => report.leftovers_removed.push(name),
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => report.unreadable.push(name),
            }
        } else if name.starts_with(MANAGED_FILE_PREFIX) {
            match seal_one(dir, &name, keyring, move_to_current) {
                Ok(Sealing::Sealed) => report.sealed.push(name),
                Ok(Sealing::Resealed) => report.resealed.push(name),
                Ok(Sealing::Unchanged) => report.unchanged += 1,
                Err(()) => report.unreadable.push(name),
            }
        }
    }
    sync_dir(dir);
    Ok(report)
}

enum Sealing {
    Sealed,
    Resealed,
    Unchanged,
}

/// Identity of a file as a reader saw it: if it differs just before the
/// rename, someone replaced the file in between.
fn identity(meta: &std::fs::Metadata) -> (u64, u64, i64, i64) {
    (meta.ino(), meta.len(), meta.mtime(), meta.mtime_nsec())
}

/// Make one managed file sealed under the current key, as far as
/// `move_to_current` asks. `Err(())` carries nothing: the caller names the
/// file, and no detail of the content may leave this function.
fn seal_one(
    dir: &Path,
    name: &str,
    keyring: &Keyring,
    move_to_current: bool,
) -> Result<Sealing, ()> {
    let path = dir.join(name);
    let meta = std::fs::symlink_metadata(&path).map_err(|_| ())?;
    if !meta.is_file() {
        return Err(());
    }
    let bytes = std::fs::read(&path).map_err(|_| ())?;
    let (plaintext, outcome) = if looks_sealed(&bytes) {
        let opened = keyring.open(name, &bytes).map_err(|_| ())?;
        if opened.with_current_key || !move_to_current {
            return Ok(Sealing::Unchanged);
        }
        (opened.plaintext, Sealing::Resealed)
    } else {
        // An older build's plain file: text, read back trimmed.
        let text = String::from_utf8(bytes).map_err(|_| ())?;
        if text.trim().is_empty() {
            return Err(());
        }
        (text.trim().as_bytes().to_vec(), Sealing::Sealed)
    };
    let sealed = keyring.seal(name, &plaintext).map_err(|_| ())?;
    let tmp = write_staged(dir, name, &sealed).map_err(|_| ())?;
    // Replaced since it was read? Then this copy is stale: drop it.
    let unchanged =
        std::fs::symlink_metadata(&path).is_ok_and(|now| identity(&now) == identity(&meta));
    if !unchanged || std::fs::rename(&tmp, &path).is_err() {
        let _ = std::fs::remove_file(&tmp);
        return Err(());
    }
    Ok(outcome)
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

    use base64::Engine as _;

    use lakehouse_core::secret::EnvSecretResolver;
    use lakehouse_store::connectors::{CredentialKind, CredentialSource, derive_secret_ref};

    use super::*;

    /// A fixed test key; the value is not a secret.
    fn test_key() -> String {
        base64::engine::general_purpose::STANDARD.encode([9_u8; 32])
    }

    fn ring_with(current: &str, previous: Option<&str>) -> Keyring {
        Keyring::from_values(Some(current), previous).unwrap()
    }

    fn new_store(dir: &Path) -> ConnectorSecretStore {
        ConnectorSecretStore::new(dir, Ok(Arc::new(ring_with(&test_key(), None))))
    }

    /// The credential in the file at `path`, opened with the test key. Panics
    /// when the file is not a seal under that key, which is itself the
    /// assertion most tests want.
    fn read_opened(path: impl AsRef<Path>) -> Result<String, std::io::Error> {
        let path = path.as_ref();
        let name = path.file_name().unwrap().to_str().unwrap();
        let bytes = std::fs::read(path)?;
        let opened = ring_with(&test_key(), None).open(name, &bytes).unwrap();
        Ok(String::from_utf8(opened.plaintext).unwrap())
    }

    fn managed_ref(id: &str) -> String {
        derive_secret_ref(id, CredentialSource::Managed, CredentialKind::Password)
    }

    #[tokio::test]
    async fn write_stores_the_value_owner_only_where_the_resolver_looks() {
        let dir = tempfile::tempdir().unwrap();
        let store = new_store(dir.path());
        let secret_ref = managed_ref("conn-orders-k3x9");
        store
            .write(&secret_ref, &SecretValue::new("s3cret-pass"))
            .await
            .unwrap();

        let path = dir
            .path()
            .join("connector_managed_conn_orders_k3x9_password");
        assert_eq!(read_opened(&path).unwrap(), "s3cret-pass");
        // SRC-12 decision 4: what is on the volume is the seal, not the value.
        let raw = std::fs::read(&path).unwrap();
        assert!(!raw.windows(11).any(|w| w == b"s3cret-pass"));
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "a stored credential must be owner-only");
    }

    fn managed(id: &str, kind: CredentialKind) -> String {
        derive_secret_ref(id, CredentialSource::Managed, kind)
    }

    /// Every file in `dir`, sorted, so a test can assert nothing was left
    /// behind (no staged value, no backup).
    fn listing(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    fn pair(id: &str, access: &str, secret: &str) -> Vec<(String, SecretValue)> {
        vec![
            (
                managed(id, CredentialKind::AccessKey),
                SecretValue::new(access),
            ),
            (
                managed(id, CredentialKind::SecretKey),
                SecretValue::new(secret),
            ),
        ]
    }

    /// Staging writes nothing live; publishing puts the whole pair in
    /// place; committing leaves exactly the two credential files.
    #[tokio::test]
    async fn a_committed_pair_replaces_both_values_and_leaves_nothing_else() {
        let dir = tempfile::tempdir().unwrap();
        let store = new_store(dir.path());
        store
            .stage(&pair("conn-s3", "old-access", "old-secret"))
            .await
            .unwrap()
            .publish()
            .await
            .unwrap()
            .commit()
            .await;

        let staged = store
            .stage(&pair("conn-s3", "new-access", "new-secret"))
            .await
            .unwrap();
        let access = dir.path().join("connector_managed_conn_s3_access_key");
        assert_eq!(
            read_opened(&access).unwrap(),
            "old-access",
            "staging must not touch the live credential"
        );
        staged.publish().await.unwrap().commit().await;

        assert_eq!(read_opened(&access).unwrap(), "new-access");
        assert_eq!(
            read_opened(dir.path().join("connector_managed_conn_s3_secret_key")).unwrap(),
            "new-secret"
        );
        assert_eq!(
            listing(dir.path()),
            [
                "connector_managed_conn_s3_access_key",
                "connector_managed_conn_s3_secret_key"
            ]
        );
    }

    /// The case the pair exists for: when the ref change after publishing
    /// fails, rollback restores BOTH old values, never a new access key
    /// beside an old secret key.
    #[tokio::test]
    async fn a_rolled_back_pair_restores_both_old_values_exactly() {
        let dir = tempfile::tempdir().unwrap();
        let store = new_store(dir.path());
        store
            .stage(&pair("conn-s3", "old-access", "old-secret"))
            .await
            .unwrap()
            .publish()
            .await
            .unwrap()
            .commit()
            .await;

        store
            .stage(&pair("conn-s3", "new-access", "new-secret"))
            .await
            .unwrap()
            .publish()
            .await
            .unwrap()
            .rollback()
            .await;

        assert_eq!(
            read_opened(dir.path().join("connector_managed_conn_s3_access_key")).unwrap(),
            "old-access"
        );
        assert_eq!(
            read_opened(dir.path().join("connector_managed_conn_s3_secret_key")).unwrap(),
            "old-secret"
        );
        assert_eq!(
            listing(dir.path()),
            [
                "connector_managed_conn_s3_access_key",
                "connector_managed_conn_s3_secret_key"
            ]
        );
    }

    /// Rolling back a credential that did not exist before removes it.
    #[tokio::test]
    async fn rolling_back_a_new_credential_removes_it() {
        let dir = tempfile::tempdir().unwrap();
        let store = new_store(dir.path());
        let token = vec![(
            managed("conn-rest", CredentialKind::Token),
            SecretValue::new("t0ken"),
        )];
        store
            .stage(&token)
            .await
            .unwrap()
            .publish()
            .await
            .unwrap()
            .rollback()
            .await;
        assert!(listing(dir.path()).is_empty());
    }

    /// A bad value anywhere in the pair stages nothing at all.
    #[tokio::test]
    async fn staging_refuses_the_whole_pair_when_one_value_is_unusable() {
        let dir = tempfile::tempdir().unwrap();
        let store = new_store(dir.path());
        let err = store
            .stage(&pair("conn-s3", "fine", " padded"))
            .await
            .unwrap_err();
        assert!(matches!(err, SecretStoreError::InvalidValue(_)));
        assert!(listing(dir.path()).is_empty());
    }

    #[tokio::test]
    async fn write_replaces_the_previous_value_and_leaves_no_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        let store = new_store(dir.path());
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
            read_opened(dir.path().join("connector_managed_conn_a_password")).unwrap(),
            "second"
        );
    }

    /// The first guarantee: an operator's `file:`/`env:` ref, a seeded
    /// ref, or anything carrying a path segment is refused — nothing is
    /// written or deleted.
    #[tokio::test]
    async fn refuses_every_ref_that_is_not_managed() {
        let dir = tempfile::tempdir().unwrap();
        let store = new_store(dir.path());
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
        let store = new_store(dir.path());
        let result = store
            .write(&managed_ref("conn-a"), &SecretValue::new(" padded "))
            .await;
        assert!(matches!(result, Err(SecretStoreError::InvalidValue(_))));
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[tokio::test]
    async fn remove_deletes_and_tolerates_an_already_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let store = new_store(dir.path());
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
        let store = new_store(Path::new("/nonexistent/connector-secrets"));
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

    // ── SRC-12 part A: start-up pass, key change, missing key ───────────

    /// Decision 4, first start: a plain file an older build wrote is sealed
    /// in place and the plain copy is gone; decision 9: leftovers of an
    /// interrupted change are removed.
    #[tokio::test]
    async fn the_startup_pass_seals_a_plain_file_and_removes_leftovers() {
        let dir = tempfile::tempdir().unwrap();
        let name = "connector_managed_conn_old_password";
        std::fs::write(dir.path().join(name), "plain-pass\n").unwrap();
        std::fs::write(
            dir.path()
                .join(".connector_managed_conn_old_password.1.2.3.bak"),
            "older-pass",
        )
        .unwrap();
        std::fs::write(
            dir.path()
                .join(".connector_managed_conn_old_password.1.2.4.tmp"),
            "newer-pass",
        )
        .unwrap();
        // An operator's file is none of this module's business.
        std::fs::write(dir.path().join("connector_conn_ops_password"), "ops").unwrap();

        let report = new_store(dir.path()).startup_pass().await.unwrap();

        assert_eq!(report.sealed, [name]);
        assert_eq!(report.leftovers_removed.len(), 2);
        assert!(report.unreadable.is_empty());
        assert_eq!(read_opened(dir.path().join(name)).unwrap(), "plain-pass");
        assert_eq!(
            listing(dir.path()),
            ["connector_conn_ops_password", name],
            "no plain copy, no temporary, no backup may remain"
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("connector_conn_ops_password")).unwrap(),
            "ops"
        );
        let mode = std::fs::metadata(dir.path().join(name))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);

        // A second start changes nothing.
        let again = new_store(dir.path()).startup_pass().await.unwrap();
        assert_eq!(again.unchanged, 1);
        assert!(again.sealed.is_empty() && again.leftovers_removed.is_empty());
    }

    /// A sealed file whose key is not configured is reported, never taken
    /// for a plain value and sealed a second time.
    #[tokio::test]
    async fn the_startup_pass_leaves_a_file_sealed_under_an_unknown_key() {
        let dir = tempfile::tempdir().unwrap();
        let name = "connector_managed_conn_a_password";
        let stranger = ring_with(
            &base64::engine::general_purpose::STANDARD.encode([3_u8; 32]),
            None,
        );
        let sealed = stranger.seal(name, b"v").unwrap();
        std::fs::write(dir.path().join(name), &sealed).unwrap();

        let report = new_store(dir.path()).startup_pass().await.unwrap();

        assert_eq!(report.unreadable, [name]);
        assert_eq!(std::fs::read(dir.path().join(name)).unwrap(), sealed);
    }

    /// Decision 5: with the new key current and the old one as previous, the
    /// command moves every file to the new key; afterwards the new key alone
    /// opens all of them, and the old key alone opens none.
    #[tokio::test]
    async fn reseal_all_moves_every_file_to_the_current_key() {
        let dir = tempfile::tempdir().unwrap();
        let old = test_key();
        let new = base64::engine::general_purpose::STANDARD.encode([5_u8; 32]);
        let old_store = new_store(dir.path());
        old_store
            .stage(&pair("conn-s3", "the-access", "the-secret"))
            .await
            .unwrap()
            .publish()
            .await
            .unwrap()
            .commit()
            .await;

        let changing =
            ConnectorSecretStore::new(dir.path(), Ok(Arc::new(ring_with(&new, Some(&old)))));
        // Before the command, the previous key still opens (decision 5).
        let before = changing.startup_pass().await.unwrap();
        assert_eq!(before.unchanged, 2);
        let report = changing.reseal_all().await.unwrap();
        assert_eq!(report.resealed.len(), 2);
        assert!(report.unreadable.is_empty());

        let only_new = ring_with(&new, None);
        let only_old = ring_with(&old, None);
        for name in [
            "connector_managed_conn_s3_access_key",
            "connector_managed_conn_s3_secret_key",
        ] {
            let bytes = std::fs::read(dir.path().join(name)).unwrap();
            assert!(only_new.open(name, &bytes).is_ok(), "{name}");
            assert!(only_old.open(name, &bytes).is_err(), "{name}");
        }

        // Running it again finds nothing to do.
        let again = changing.reseal_all().await.unwrap();
        assert!(again.resealed.is_empty());
        assert_eq!(again.unchanged, 2);
        assert_eq!(listing(dir.path()).len(), 2);
    }

    /// Fail closed: with no key (or a malformed one) nothing is written, and
    /// the error is the fixed text naming the setting.
    #[tokio::test]
    async fn without_a_key_nothing_is_written_and_the_error_names_the_setting() {
        let dir = tempfile::tempdir().unwrap();
        for (given, expected) in [
            (None, CryptoError::KeyMissing),
            (Some("not-a-key"), CryptoError::KeyInvalid),
        ] {
            let store = ConnectorSecretStore::new(
                dir.path(),
                Keyring::from_values(given, None).map(Arc::new),
            );
            let err = store
                .write(&managed_ref("conn-a"), &SecretValue::new("v"))
                .await
                .unwrap_err();
            assert_eq!(err, SecretStoreError::Key(expected));
            assert!(err.to_string().contains("CONNECTOR_SECRETS_KEY"));
            assert!(store.stage(&pair("conn-s3", "a", "b")).await.is_err());
            assert!(store.startup_pass().await.is_err());
        }
        assert!(listing(dir.path()).is_empty());
    }

    /// Decision 4: the sealed file does not open when copied under another
    /// connector's name.
    #[tokio::test]
    async fn a_stored_file_copied_to_another_connectors_name_does_not_open() {
        let dir = tempfile::tempdir().unwrap();
        let store = new_store(dir.path());
        store
            .write(&managed_ref("conn-a"), &SecretValue::new("a-pass"))
            .await
            .unwrap();
        std::fs::copy(
            dir.path().join("connector_managed_conn_a_password"),
            dir.path().join("connector_managed_conn_b_password"),
        )
        .unwrap();
        let bytes = std::fs::read(dir.path().join("connector_managed_conn_b_password")).unwrap();
        assert!(
            ring_with(&test_key(), None)
                .open("connector_managed_conn_b_password", &bytes)
                .is_err()
        );
    }
}
