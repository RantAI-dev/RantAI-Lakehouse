//! Resolving a `secretRef` string to a credential value — never the other
//! way around.
//!
//! `lakehouse-store::connectors`'s module doc has carried this guarantee
//! since Phase 2: a connector's `secret_ref: String` names WHERE a
//! credential lives (an env var name, a secret-manager path), never the
//! credential itself, and nothing in that module resolves the reference to
//! a value. This module is the resolver ADR 0002
//! (`docs/adr/0002-secretref-resolution.md`) designs: a trait
//! ([`SecretResolver`]) plus one concrete implementation
//! ([`EnvSecretResolver`]) today, with file-based and external-provider
//! implementations able to land later without a breaking change to this
//! trait — see the ADR for the full design record and why each choice here
//! is shaped the way it is.
//!
//! # The guarantee this module must not weaken
//!
//! A resolved secret is returned to the caller as a [`SecretValue`], never
//! logged, never `Debug`-printed, never serialized. [`SecretValue`] has a
//! hand-written [`std::fmt::Debug`] that always renders `"<redacted>"`, the
//! same pattern `lakehouse_api::config::Config` and
//! `lakehouse_store::connectors::ConnectorRow` already use for credential
//! fields. There is deliberately no [`serde::Serialize`] impl on
//! [`SecretValue`] at all — not a redacting one, an *absent* one — so a
//! `secret.into()` accidentally handed to `serde_json::to_value` fails to
//! compile rather than needing a runtime redaction to remember to write.

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use thiserror::Error;

/// A resolved credential value.
///
/// Intentionally does not implement [`serde::Serialize`] — see the module
/// doc comment. Cloning is allowed (a resolved secret often needs to be
/// handed to more than one client, e.g. an S3 access key and a session
/// token both used to build one `object_store` client), but every clone
/// still redacts under `Debug`.
#[derive(Clone, PartialEq, Eq)]
pub struct SecretValue(String);

impl SecretValue {
    /// Wrap a resolved credential value.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Borrow the underlying secret value.
    ///
    /// Named `expose_secret` (matching the convention the `secrecy` crate
    /// popularized) rather than `as_str`/`value`, so that every call site
    /// reads, at the point of use, as a conscious decision to handle a
    /// secret rather than an unremarkable getter.
    #[must_use]
    pub fn expose_secret(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SecretValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

/// Errors resolving a `secretRef` to a value.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum SecretError {
    /// The `secretRef` string was not in a shape this resolver understands
    /// (e.g. an `env:` resolver given a `file:`-scheme reference).
    #[error("secretRef {secret_ref:?} is not a shape the {resolver} resolver understands")]
    UnsupportedRef {
        /// The offending `secretRef`, safe to log — it is a reference, not
        /// a value (the same distinction `connectors.rs` draws).
        secret_ref: String,
        /// Which resolver rejected it, for diagnosability across a chain.
        resolver: &'static str,
    },
    /// The reference was understood but nothing was found at it (e.g. the
    /// named environment variable is unset).
    #[error("secretRef {secret_ref:?} resolved to nothing via {resolver}")]
    NotFound {
        /// The offending `secretRef`.
        secret_ref: String,
        /// Which resolver reported the miss.
        resolver: &'static str,
    },
}

/// Resolves a `secretRef` string to a [`SecretValue`].
///
/// # Why a trait now, for one implementation
///
/// ADR 0002 is explicitly load-bearing for work that has not landed yet:
/// Lakekeeper storage-credential vending, Debezium source credentials, and
/// dlt connection secrets all need to turn a `secretRef` into a value, and
/// none of them should have to care whether that reference points at an
/// environment variable, a mounted file, or Vault/AWS Secrets
/// Manager/whatever a customer's platform team already runs. Defining the
/// seam now — even with only [`EnvSecretResolver`] behind it — means a
/// later file-based or external-provider resolver is a new
/// implementation of this trait, not a signature change propagated through
/// every caller.
///
/// # Object safety and async
///
/// Deliberately `async fn` via `#[async_trait]`-free native async-in-trait
/// (stabilized on this workspace's MSRV, `1.88`) rather than a sync
/// signature: a real external-provider implementation (Vault, AWS Secrets
/// Manager) is a network call, and forcing a sync signature today would
/// mean every future implementation either blocks a runtime thread or the
/// trait needs a breaking change later. Getting this right now is exactly
/// what "admits later implementations without a breaking change" means in
/// practice.
pub trait SecretResolver: fmt::Debug + Send + Sync {
    /// Resolve `secret_ref` to a value.
    ///
    /// # Errors
    ///
    /// Returns [`SecretError`] if `secret_ref` is not a shape this resolver
    /// understands, or is understood but nothing is found at it.
    fn resolve(
        &self,
        secret_ref: &str,
    ) -> impl std::future::Future<Output = Result<SecretValue, SecretError>> + Send;
}

/// Object-safe wrapper around [`SecretResolver`] for callers that need a
/// `dyn`-compatible handle (e.g. holding one behind an `Arc<dyn ..>` in
/// application state, since `SecretResolver`'s native `async fn` makes the
/// trait itself not object-safe).
#[async_trait::async_trait]
pub trait DynSecretResolver: fmt::Debug + Send + Sync {
    /// Object-safe equivalent of [`SecretResolver::resolve`].
    ///
    /// # Errors
    ///
    /// See [`SecretResolver::resolve`].
    async fn resolve_dyn(&self, secret_ref: &str) -> Result<SecretValue, SecretError>;
}

#[async_trait::async_trait]
impl<T: SecretResolver> DynSecretResolver for T {
    async fn resolve_dyn(&self, secret_ref: &str) -> Result<SecretValue, SecretError> {
        self.resolve(secret_ref).await
    }
}

/// Prefix an `env:`-scheme `secretRef` must carry, e.g. `env:CH_PASSWORD`.
///
/// A bare, unprefixed name (`"CH_PASSWORD"`) is intentionally NOT accepted:
/// once a second resolver scheme (`file:`) exists, an unprefixed reference
/// would be ambiguous about which resolver it belongs to. Requiring the
/// prefix from day one avoids a breaking change to every stored
/// `secretRef` when the second scheme lands.
pub const ENV_SECRET_REF_PREFIX: &str = "env:";

/// Resolves `env:<VAR_NAME>`-shaped `secretRef`s against the process
/// environment (or an injected map, for testability).
///
/// This is the only [`SecretResolver`] implementation that exists today. It
/// is deliberately the least trustworthy option long-term (env vars are
/// visible to the whole process, show up in `/proc/<pid>/environ`, and are
/// easy to leak into a crash dump) — it is here because it is what P1b
/// needs to unblock Lakekeeper storage credentials and the G1 test, not
/// because it is the recommended production shape. See the "Consequences"
/// section of ADR 0002 for the operator guidance that goes with this.
#[derive(Clone)]
pub struct EnvSecretResolver {
    /// `None` reads the real process environment via [`std::env::var`].
    /// `Some` is used by tests to resolve against an explicit map without
    /// mutating global process state (the same rationale
    /// `lakehouse_api::config::Config::from_map` documents for its own
    /// `HashMap`-based constructor).
    overrides: Option<Arc<HashMap<String, String>>>,
}

impl fmt::Debug for EnvSecretResolver {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Deliberately does not print `overrides` even though it isn't
        // secret-shaped metadata per se — a test map passed to this
        // resolver may itself contain the very credential values this
        // module exists to protect, so treat it the same as any other
        // secret-bearing field.
        f.debug_struct("EnvSecretResolver").finish_non_exhaustive()
    }
}

impl Default for EnvSecretResolver {
    fn default() -> Self {
        Self::new()
    }
}

impl EnvSecretResolver {
    /// Resolve against the real process environment.
    #[must_use]
    pub fn new() -> Self {
        Self { overrides: None }
    }

    /// Resolve against an explicit map instead of the process environment.
    #[must_use]
    pub fn with_map(map: HashMap<String, String>) -> Self {
        Self {
            overrides: Some(Arc::new(map)),
        }
    }

    fn lookup(&self, key: &str) -> Option<String> {
        match &self.overrides {
            Some(map) => map.get(key).cloned(),
            None => std::env::var(key).ok(),
        }
    }
}

impl SecretResolver for EnvSecretResolver {
    async fn resolve(&self, secret_ref: &str) -> Result<SecretValue, SecretError> {
        let Some(var_name) = secret_ref.strip_prefix(ENV_SECRET_REF_PREFIX) else {
            return Err(SecretError::UnsupportedRef {
                secret_ref: secret_ref.to_owned(),
                resolver: "env",
            });
        };
        if var_name.is_empty() {
            return Err(SecretError::UnsupportedRef {
                secret_ref: secret_ref.to_owned(),
                resolver: "env",
            });
        }
        self.lookup(var_name)
            .map(SecretValue::new)
            .ok_or_else(|| SecretError::NotFound {
                secret_ref: secret_ref.to_owned(),
                resolver: "env",
            })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::collections::HashMap;

    use super::*;

    #[test]
    fn debug_never_renders_the_value() {
        let secret = SecretValue::new("s3cret-value");
        let rendered = format!("{secret:?}");
        assert_eq!(rendered, "<redacted>");
        assert!(!rendered.contains("s3cret-value"));
    }

    #[test]
    fn resolver_debug_never_renders_overrides() {
        let mut map = HashMap::new();
        map.insert("FOO".to_owned(), "s3cret".to_owned());
        let resolver = EnvSecretResolver::with_map(map);
        let rendered = format!("{resolver:?}");
        assert!(!rendered.contains("s3cret"));
    }

    #[tokio::test]
    async fn resolves_env_prefixed_ref_from_map() {
        let mut map = HashMap::new();
        map.insert("MY_VAR".to_owned(), "the-value".to_owned());
        let resolver = EnvSecretResolver::with_map(map);
        let value = resolver.resolve("env:MY_VAR").await.unwrap();
        assert_eq!(value.expose_secret(), "the-value");
    }

    #[tokio::test]
    async fn rejects_unprefixed_ref() {
        let resolver = EnvSecretResolver::with_map(HashMap::new());
        let err = resolver.resolve("MY_VAR").await.unwrap_err();
        assert!(matches!(err, SecretError::UnsupportedRef { .. }));
    }

    #[tokio::test]
    async fn rejects_empty_var_name() {
        let resolver = EnvSecretResolver::with_map(HashMap::new());
        let err = resolver.resolve("env:").await.unwrap_err();
        assert!(matches!(err, SecretError::UnsupportedRef { .. }));
    }

    #[tokio::test]
    async fn not_found_for_unset_var() {
        let resolver = EnvSecretResolver::with_map(HashMap::new());
        let err = resolver.resolve("env:NOPE_NOT_SET").await.unwrap_err();
        assert!(matches!(err, SecretError::NotFound { .. }));
    }

    #[tokio::test]
    async fn dyn_wrapper_delegates() {
        let mut map = HashMap::new();
        map.insert("MY_VAR".to_owned(), "the-value".to_owned());
        let resolver: Arc<dyn DynSecretResolver> = Arc::new(EnvSecretResolver::with_map(map));
        let value = resolver.resolve_dyn("env:MY_VAR").await.unwrap();
        assert_eq!(value.expose_secret(), "the-value");
    }
}
