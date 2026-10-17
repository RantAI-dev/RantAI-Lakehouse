//! Process-level cryptographic provider installation for `rustls`.
//!
//! PR #192 review BLOCKER 10: `lakehouse-api` pulls in both `ring` and
//! `aws-lc-rs` features for `rustls` (via `object_store`, `hyper-rustls`,
//! etc.). When multiple provider features are present, `rustls` 0.23 does
//! not choose a default provider automatically, causing any TLS dial relying
//! on the process default (such as `sqlx`, `tiberius`, `reqwest`, `lettre`)
//! to panic. This module installs `ring` as the process-wide default provider.

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
