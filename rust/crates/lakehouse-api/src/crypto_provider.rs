//! Process-level cryptographic provider installation for `rustls`.
//!
//! PR #192 review BLOCKER 10: `lakehouse-api` pulls in both `ring` and
//! `aws-lc-rs` features for `rustls` (via `object_store`, `hyper-rustls`,
//! etc.). When multiple provider features are present, `rustls` 0.23 does
//! not choose a default provider automatically, and the consumers that ask
//! for the process-level default panic with "Could not automatically
//! determine the process-level `CryptoProvider` from Rustls crate features".
//! This module installs `ring` as the process-wide default provider.
//!
//! # Which consumers need it, and which do not
//!
//! Checked against the crates this workspace links (versions in
//! `rust/Cargo.lock`):
//!
//! - `sqlx-core` 0.9.0: **needs it.** Its `VerifyCa` path builds the server
//!   verifier with `WebPkiServerVerifier::builder(...)`
//!   (`sqlx-core-0.9.0/src/net/tls/tls_rustls.rs:155`), and that builder
//!   reads the process-level default
//!   (`rustls-0.23.46/src/webpki/server_verifier.rs:153`). The `Require`
//!   path names its own provider
//!   (`sqlx-core-0.9.0/src/net/tls/tls_rustls.rs:110`), so it does not.
//! - `hyper-rustls` 0.27.9 (through `reqwest`, and through `object_store`,
//!   which is `reqwest` underneath): **needs it.** Its connector builds the
//!   client config with `ClientConfig::builder()`
//!   (`hyper-rustls-0.27.9/src/connector/builder.rs:93`), which reads the
//!   process-level default.
//! - `tiberius` 0.13.0: **does not need it.** It takes the process-installed
//!   provider when there is one and falls back to `aws-lc-rs` when there is
//!   not
//!   (`tiberius-0.13.0/src/client/tls_stream/rustls_tls_stream.rs:173`
//!   and `:271`), so the install changes which provider a SQL Server dial
//!   uses -- from `aws-lc-rs` to `ring` -- but never panics.
//! - `lettre` 0.11.23: **does not need it**, for the same reason
//!   (`lettre-0.11.23/src/rustls_crypto.rs:5`: the process default, else
//!   `aws-lc-rs`).
//!
//! So the install is what keeps the `sqlx` `VerifyCa` dials and every
//! `reqwest`/`object_store` request from panicking, and choosing `ring`
//! matches the provider this workspace's own `rustls`/`tokio-rustls` and
//! `pg_probe` name explicitly.

/// An error indicating that installing the default `rustls` crypto provider
/// failed for a reason other than already being installed.
#[derive(Debug, thiserror::Error)]
#[error("failed to install default rustls crypto provider and no default provider is set")]
pub struct InstallProviderError;

/// Installs the `ring` cryptography provider as the process default for
/// `rustls`.
///
/// If a default provider has already been installed (e.g. earlier in this
/// process or in a shared test runner), this function succeeds.
///
/// # Errors
///
/// Returns [`InstallProviderError`] if installing the provider failed and
/// no default provider is present.
pub fn install_default_provider() -> Result<(), InstallProviderError> {
    if rustls::crypto::ring::default_provider()
        .install_default()
        .is_err()
        && rustls::crypto::CryptoProvider::get_default().is_none()
    {
        return Err(InstallProviderError);
    }
    Ok(())
}
