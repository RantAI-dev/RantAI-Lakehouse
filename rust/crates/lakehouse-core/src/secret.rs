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
    /// The reference was understood and could plausibly resolve, but is not
    /// on the caller's explicit allowlist — see [`AllowlistedSecretResolver`].
    ///
    /// Distinct from [`SecretError::NotFound`]: `NotFound` means "this
    /// resolver looked and there was nothing there"; `NotAllowed` means
    /// "this resolver refuses to even look, because this caller is not
    /// permitted to resolve this reference". Keeping them separate matters
    /// for diagnosability — a connector operator who typos a secretRef
    /// should see `NotFound`, not be told the (correctly spelled) name is
    /// forbidden.
    #[error("secretRef {secret_ref:?} is not on the {resolver} allowlist")]
    NotAllowed {
        /// The offending `secretRef`.
        secret_ref: String,
        /// Which resolver rejected it.
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

/// Matches `value` against `pattern`, which must contain EXACTLY one `*`
/// splitting it into a fixed prefix and a fixed suffix (e.g.
/// `"env:CONNECTOR_*_PASSWORD"` -> prefix `"env:CONNECTOR_"`, suffix
/// `"_PASSWORD"`).
///
/// Deliberately NOT a general glob: a pattern with two wildcards, or a bare
/// `env:CONNECTOR_*` with no suffix, is exactly the shape WS3 plan review
/// X1 found admits a non-credential configuration variable
/// (`env:CONNECTOR_PROBE_ALLOW_INTERNAL_HOSTS`, a real flag
/// `lakehouse_api::config::Config` already reads) — restricting the
/// matcher itself to prefix+suffix makes that class of pattern impossible
/// to express, not merely discouraged by convention. A pattern with zero
/// or more than one `*` never matches anything (fails closed): the pattern
/// list is a fixed Rust constant, not caller input, so a malformed pattern
/// is a programmer error caught by `debug_assert!`, not a runtime `Result`.
///
/// `pub`, not `pub(crate)`: `lakehouse_api::routes::connectors`'s
/// `reject_allowlisted_secret_ref` ("who may NAME a reserved ref") and
/// `lakehouse_api::state`'s `AllowlistedSecretResolver` ("which refs may
/// RESOLVE") must check the exact same patterns the exact same way, or the
/// two checks drift apart the way `0023_connector_dedicated_secret_refs.sql`'s
/// header describes happening to the old exact-list version (WS3 plan review
/// X1). Exporting this function is what makes "same matcher" checkable
/// across the crate boundary rather than merely asserted in a comment.
#[must_use]
pub fn pattern_matches(pattern: &str, value: &str) -> bool {
    let mut parts = pattern.splitn(3, '*');
    let (Some(prefix), Some(suffix), None) = (parts.next(), parts.next(), parts.next()) else {
        debug_assert!(
            false,
            "secret-ref pattern {pattern:?} must contain exactly one '*'"
        );
        return false;
    };
    value.len() >= prefix.len() + suffix.len()
        && value.starts_with(prefix)
        && value.ends_with(suffix)
}

/// Wraps a [`SecretResolver`] with an explicit allowlist of credential-
/// suffix `secretRef` PATTERNS it may resolve (see [`pattern_matches`]),
/// rejecting anything that matches none of them with
/// [`SecretError::NotAllowed`] before ever consulting the inner resolver.
///
/// # Why this exists (addendum to ADR 0002)
///
/// ADR 0002 designed [`SecretResolver`] for callers where the resolved
/// value's DESTINATION is fixed and operator-controlled — Lakekeeper's own
/// storage credential, dialing `RustFS`; a future Debezium source credential,
/// dialing a database an operator configured. In that shape, "which env var
/// can this caller name" was never a meaningful attack surface: the caller
/// choosing a `secretRef` is the same operator who configured the
/// destination.
///
/// `lakehouse-api::connector_probe` breaks that assumption: a
/// `connector:manage` principal supplies BOTH the `secretRef` AND the `host`
/// a probe dials, through `POST /api/connectors` and `POST
/// /api/connectors/{id}/test`. Handing `EnvSecretResolver` (which resolves
/// ANY `env:NAME`) to that code path means the principal can name
/// `env:DATABASE_URL`, `env:CH_PASSWORD`, `env:OIDC_CLIENT_SECRET`, or any
/// other process secret, point `host` at infrastructure they control, and
/// have this service dial out and authenticate with the resolved value —
/// exfiltrating it via a cleartext-password auth handshake (`sqlx`'s
/// Postgres wire protocol does not require TLS by default) or an S3
/// `Authorization` header. This wrapper closes that: `connector_probe`
/// is handed an `AllowlistedSecretResolver` restricted to exactly the
/// `secretRef`s this deployment's OWN seeded connectors use
/// (`rust/migrations/0022_prune_connector_seed.sql`), never the general
/// unrestricted [`EnvSecretResolver`] the rest of the process uses.
///
/// A `secretRef` outside the allowlist is rejected — never silently
/// ignored, never falls through to the inner resolver — so a caller gets a
/// clear, testable [`SecretError::NotAllowed`], not a confusing
/// [`SecretError::NotFound`] that looks like a typo.
#[derive(Debug)]
pub struct AllowlistedSecretResolver<R> {
    inner: R,
    /// Credential-suffix PATTERNS (see [`pattern_matches`]), never exact
    /// strings: a bare exact string has zero `*` characters, which
    /// `pattern_matches` rejects by construction, so every allowlist entry
    /// must be expressed as a `prefix*suffix` pattern (WS3 plan review X1).
    allowed_patterns: Vec<String>,
    /// Name surfaced in [`SecretError`] variants for diagnosability across
    /// a chain of resolvers — same rationale as [`EnvSecretResolver`]'s use
    /// of `"env"`.
    name: &'static str,
}

impl<R> AllowlistedSecretResolver<R> {
    /// Wrap `inner`, permitting only a `secretRef` matching one of
    /// `allowed_patterns` (each a credential-suffix pattern understood by
    /// [`pattern_matches`]). `name` identifies this resolver in error
    /// messages (e.g. `"connector-allowlist"`).
    #[must_use]
    pub fn new(
        inner: R,
        allowed_patterns: impl IntoIterator<Item = String>,
        name: &'static str,
    ) -> Self {
        Self {
            inner,
            allowed_patterns: allowed_patterns.into_iter().collect(),
            name,
        }
    }
}

impl<R: SecretResolver> SecretResolver for AllowlistedSecretResolver<R> {
    async fn resolve(&self, secret_ref: &str) -> Result<SecretValue, SecretError> {
        if !self
            .allowed_patterns
            .iter()
            .any(|pattern| pattern_matches(pattern, secret_ref))
        {
            return Err(SecretError::NotAllowed {
                secret_ref: secret_ref.to_owned(),
                resolver: self.name,
            });
        }
        self.inner.resolve(secret_ref).await
    }
}

/// Prefix a `file:`-scheme `secretRef` must carry, e.g.
/// `file:/run/secrets/connector_mysql_password`.
pub const FILE_SECRET_REF_PREFIX: &str = "file:";

/// Resolves `file:<path>`-shaped `secretRef`s by reading a file's (trimmed)
/// contents, REQUIRING the path's canonicalized form to fall under a fixed
/// base directory this resolver was constructed with.
///
/// Two layers refuse a bad path, and this doc comment states which:
/// (1) [`AllowlistedSecretResolver`]'s pattern check (wired into
/// `lakehouse_api::state::AppState`, in a follow-up task on this branch)
/// refuses a `secretRef` string that doesn't match
/// `"file:/run/secrets/connector_*"` BEFORE this resolver ever runs, so a
/// caller cannot even name a path outside `/run/secrets/connector_*` in the
/// first place; (2) THIS resolver's own `canonicalize` + base-dir check
/// refuses a path that passed (1) syntactically (e.g. via a `..` segment or
/// a symlink) but resolves outside the base directory at the filesystem
/// level — defense in depth, since (1) is a string comparison that cannot
/// see through a symlink.
#[derive(Debug, Clone)]
pub struct FileSecretResolver {
    base_dir: std::path::PathBuf,
}

impl FileSecretResolver {
    /// Wrap a base directory: only a resolved path that canonicalizes to
    /// somewhere under `base_dir` is ever read.
    #[must_use]
    pub fn new(base_dir: impl Into<std::path::PathBuf>) -> Self {
        Self {
            base_dir: base_dir.into(),
        }
    }
}

impl SecretResolver for FileSecretResolver {
    async fn resolve(&self, secret_ref: &str) -> Result<SecretValue, SecretError> {
        let Some(path) = secret_ref.strip_prefix(FILE_SECRET_REF_PREFIX) else {
            return Err(SecretError::UnsupportedRef {
                secret_ref: secret_ref.to_owned(),
                resolver: "file",
            });
        };
        let canonical_base =
            std::fs::canonicalize(&self.base_dir).map_err(|_| SecretError::UnsupportedRef {
                secret_ref: secret_ref.to_owned(),
                resolver: "file",
            })?;
        let canonical_path =
            std::fs::canonicalize(path).map_err(|_| SecretError::UnsupportedRef {
                secret_ref: secret_ref.to_owned(),
                resolver: "file",
            })?;
        if !canonical_path.starts_with(&canonical_base) {
            return Err(SecretError::UnsupportedRef {
                secret_ref: secret_ref.to_owned(),
                resolver: "file",
            });
        }
        let contents =
            std::fs::read_to_string(&canonical_path).map_err(|_| SecretError::NotFound {
                secret_ref: secret_ref.to_owned(),
                resolver: "file",
            })?;
        Ok(SecretValue::new(contents.trim().to_owned()))
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

    /// The exact attack `AllowlistedSecretResolver` exists to stop: a
    /// caller who can name any `env:` ref (here, `env:DATABASE_URL` — one
    /// of the reviewer's cited examples) must be refused even though the
    /// inner `EnvSecretResolver` would happily resolve it.
    #[tokio::test]
    async fn out_of_scope_env_ref_is_rejected_not_silently_resolved() {
        let mut map = HashMap::new();
        map.insert("DATABASE_URL".to_owned(), "s3cret-dsn".to_owned());
        map.insert("POSTGRES_PASSWORD".to_owned(), "allowed-value".to_owned());
        let inner = EnvSecretResolver::with_map(map);
        let resolver = AllowlistedSecretResolver::new(
            inner,
            ["env:*_PASSWORD".to_owned()],
            "connector-allowlist",
        );

        let err = resolver.resolve("env:DATABASE_URL").await.unwrap_err();
        assert!(
            matches!(err, SecretError::NotAllowed { .. }),
            "expected NotAllowed, got {err:?}"
        );
        assert!(!err.to_string().contains("s3cret-dsn"));
    }

    #[tokio::test]
    async fn allowlisted_ref_still_resolves_through_the_inner_resolver() {
        let mut map = HashMap::new();
        map.insert("POSTGRES_PASSWORD".to_owned(), "allowed-value".to_owned());
        let inner = EnvSecretResolver::with_map(map);
        let resolver = AllowlistedSecretResolver::new(
            inner,
            ["env:*_PASSWORD".to_owned()],
            "connector-allowlist",
        );

        let value = resolver.resolve("env:POSTGRES_PASSWORD").await.unwrap();
        assert_eq!(value.expose_secret(), "allowed-value");
    }

    /// A name that would resolve fine through the inner resolver, but was
    /// never added to the allowlist, must fail closed — never silently
    /// fall through to the inner resolver's answer.
    #[tokio::test]
    async fn empty_allowlist_rejects_everything() {
        let mut map = HashMap::new();
        map.insert("ANYTHING".to_owned(), "value".to_owned());
        let inner = EnvSecretResolver::with_map(map);
        let resolver =
            AllowlistedSecretResolver::new(inner, Vec::<String>::new(), "connector-allowlist");
        let err = resolver.resolve("env:ANYTHING").await.unwrap_err();
        assert!(matches!(err, SecretError::NotAllowed { .. }));
    }

    /// The exact regression X1 (WS3 plan judge review) found: `CONNECTOR_*`
    /// as a bare prefix admits `CONNECTOR_PROBE_ALLOW_INTERNAL_HOSTS` — a
    /// real, non-credential config variable this API already reads
    /// (`lakehouse_api::config::Config::connector_probe_allow_internal_hosts`)
    /// — because it happens to start with `CONNECTOR_`. Credential-SUFFIX
    /// patterns close this: the variable's name does not end in `_PASSWORD`/
    /// `_SECRET_KEY`/`_ACCESS_KEY`/`_API_KEY`/`_TOKEN`, so no pattern admits it.
    #[tokio::test]
    async fn allowlist_admits_credential_suffixed_connector_refs_but_not_a_config_flag() {
        let mut map = HashMap::new();
        map.insert("CONNECTOR_MYSQL_PASSWORD".to_owned(), "ok".to_owned());
        map.insert(
            "CONNECTOR_PROBE_ALLOW_INTERNAL_HOSTS".to_owned(),
            "true".to_owned(),
        );
        let inner = EnvSecretResolver::with_map(map);
        let resolver = AllowlistedSecretResolver::new(
            inner,
            [
                "env:CONNECTOR_*_PASSWORD".to_owned(),
                "env:CONNECTOR_*_SECRET_KEY".to_owned(),
                "env:CONNECTOR_*_ACCESS_KEY".to_owned(),
                "env:CONNECTOR_*_API_KEY".to_owned(),
                "env:CONNECTOR_*_TOKEN".to_owned(),
            ],
            "connector-allowlist",
        );
        assert!(
            resolver
                .resolve("env:CONNECTOR_MYSQL_PASSWORD")
                .await
                .is_ok()
        );
        let err = resolver
            .resolve("env:CONNECTOR_PROBE_ALLOW_INTERNAL_HOSTS")
            .await
            .unwrap_err();
        assert!(matches!(err, SecretError::NotAllowed { .. }));
    }

    /// Every non-credential `CONNECTOR_*` name that exists anywhere in this
    /// workspace today (`git grep -ohE 'CONNECTOR_[A-Z0-9_]+' | sort -u`, run
    /// 2026-09-11: `CONNECTOR_ALLOWED_SECRET_REFS` [a Rust const, not an env
    /// var], `CONNECTOR_COLUMNS` [unrelated], `CONNECTOR_PG_PASSWORD`
    /// [credential, SHOULD match], `CONNECTOR_PROBE_ALLOW_INTERNAL_HOSTS`
    /// [config, must NOT match], `CONNECTOR_S3_ACCESS_KEY`/
    /// `CONNECTOR_S3_SECRET_KEY` [credentials, SHOULD match]) is asserted
    /// here so a NEW config variable added later under the `CONNECTOR_`
    /// prefix is refused by default, not admitted by accident. The five
    /// patterns are inlined rather than imported from `lakehouse-api`: the
    /// authoritative list will live in `lakehouse_api::state::AppState`
    /// (a follow-up task on this branch), and this crate's own tests
    /// should not need to import it back.
    #[tokio::test]
    async fn allowlist_rejects_every_known_non_credential_connector_prefixed_name() {
        let resolver = AllowlistedSecretResolver::new(
            EnvSecretResolver::new(),
            [
                "env:CONNECTOR_*_PASSWORD".to_owned(),
                "env:CONNECTOR_*_SECRET_KEY".to_owned(),
                "env:CONNECTOR_*_ACCESS_KEY".to_owned(),
                "env:CONNECTOR_*_API_KEY".to_owned(),
                "env:CONNECTOR_*_TOKEN".to_owned(),
            ],
            "connector-allowlist",
        );
        for forbidden in ["env:CONNECTOR_PROBE_ALLOW_INTERNAL_HOSTS"] {
            let err = resolver.resolve(forbidden).await.unwrap_err();
            assert!(
                matches!(err, SecretError::NotAllowed { .. }),
                "{forbidden} must be refused"
            );
        }
    }

    #[tokio::test]
    async fn allowlist_still_rejects_the_apis_own_secrets() {
        let resolver = AllowlistedSecretResolver::new(
            EnvSecretResolver::new(),
            ["env:CONNECTOR_*_PASSWORD".to_owned()],
            "connector-allowlist",
        );
        for forbidden in [
            "env:POSTGRES_PASSWORD",
            "env:DATABASE_URL",
            "env:CH_PASSWORD",
        ] {
            let err = resolver.resolve(forbidden).await.unwrap_err();
            assert!(
                matches!(err, SecretError::NotAllowed { .. }),
                "{forbidden} must be refused"
            );
        }
    }

    #[test]
    fn pattern_matches_requires_both_the_fixed_prefix_and_the_fixed_suffix() {
        assert!(pattern_matches(
            "env:CONNECTOR_*_PASSWORD",
            "env:CONNECTOR_MYSQL_PASSWORD"
        ));
        assert!(!pattern_matches(
            "env:CONNECTOR_*_PASSWORD",
            "env:CONNECTOR_MYSQL_TOKEN"
        ));
        assert!(!pattern_matches(
            "env:CONNECTOR_*_PASSWORD",
            "env:OTHER_MYSQL_PASSWORD"
        ));
        // The `*` must not be permitted to span the `env:`/`file:` scheme
        // delimiter or otherwise degenerate into a bare-prefix match — a
        // candidate that is merely a prefix of the pattern (missing the
        // fixed suffix entirely) must not match.
        assert!(!pattern_matches(
            "env:CONNECTOR_*_PASSWORD",
            "env:CONNECTOR_"
        ));
        // A candidate exactly as long as the pattern's fixed parts with no
        // room for the wildcard segment still must not match if it doesn't
        // carry both the prefix and the suffix.
        assert!(!pattern_matches(
            "env:CONNECTOR_*_PASSWORD",
            "env:CONNECTOR_PASSWORD"
        ));
        // `env:CONNECTOR_PROBE_ALLOW_INTERNAL_HOSTS` — the exact variable
        // that motivated the credential-suffix design (WS3 plan review X1,
        // `config.rs:569`, `docker-compose.yml:197`, `.env.example:409`):
        // a real, non-credential configuration flag this API already
        // reads, admitted only by a bare `env:CONNECTOR_*` prefix pattern,
        // never by a credential-suffixed one.
        assert!(!pattern_matches(
            "env:CONNECTOR_*_PASSWORD",
            "env:CONNECTOR_PROBE_ALLOW_INTERNAL_HOSTS"
        ));
    }

    #[tokio::test]
    async fn file_resolver_reads_a_real_file_under_its_base_dir() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("connector_mysql_password"), "s3cret\n").unwrap();
        let resolver = FileSecretResolver::new(dir.path());
        let value = resolver
            .resolve(&format!(
                "file:{}/connector_mysql_password",
                dir.path().display()
            ))
            .await
            .unwrap();
        assert_eq!(value.expose_secret(), "s3cret");
    }

    /// A REAL `..` escape: `<base>/connector_x/../../outside/secret`
    /// canonicalizes to `<base's parent>/outside/secret`, genuinely outside
    /// `<base>` — unlike a prior-draft pattern like
    /// `connector_../etc/passwd`, whose path components (`connector_..`,
    /// `etc`, `passwd`) contain no `..` segment at all (`connector_..` is
    /// an ordinary directory NAME, not a traversal). `<base's
    /// parent>/outside/secret` is created so `canonicalize` SUCCEEDS,
    /// proving the base-directory check — not a missing-file error — is
    /// what refuses this.
    #[tokio::test]
    async fn file_resolver_rejects_a_real_traversal_escape() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path().join("secrets");
        std::fs::create_dir(&base).unwrap();
        let outside = dir.path().join("outside");
        std::fs::create_dir(&outside).unwrap();
        std::fs::write(outside.join("secret"), "s3cret-outside").unwrap();
        let resolver = FileSecretResolver::new(&base);
        let escape_ref = format!("file:{}/connector_x/../../outside/secret", base.display());
        let err = resolver.resolve(&escape_ref).await.unwrap_err();
        assert!(matches!(err, SecretError::UnsupportedRef { .. }));
        assert!(
            !err.to_string().contains("s3cret-outside"),
            "error must never contain the file's contents"
        );
    }

    /// An absolute path that never enters `base` at all (no `..` needed).
    #[tokio::test]
    async fn file_resolver_rejects_an_absolute_path_outside_its_base_dir() {
        let base_dir = tempfile::tempdir().expect("tempdir");
        let outside_dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(outside_dir.path().join("secret"), "s3cret-outside").unwrap();
        let resolver = FileSecretResolver::new(base_dir.path());
        let err = resolver
            .resolve(&format!("file:{}/secret", outside_dir.path().display()))
            .await
            .unwrap_err();
        assert!(matches!(err, SecretError::UnsupportedRef { .. }));
    }

    /// A SYMLINK inside `base` whose target resolves outside it — the case
    /// a pure string-prefix check (no `canonicalize`) would miss entirely,
    /// since the pre-canonicalization path string genuinely starts with
    /// `base`.
    #[tokio::test]
    #[cfg(unix)]
    async fn file_resolver_rejects_a_symlink_that_escapes_its_base_dir() {
        let base_dir = tempfile::tempdir().expect("tempdir");
        let outside_dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(outside_dir.path().join("secret"), "s3cret-outside").unwrap();
        std::os::unix::fs::symlink(
            outside_dir.path().join("secret"),
            base_dir.path().join("connector_link"),
        )
        .unwrap();
        let resolver = FileSecretResolver::new(base_dir.path());
        let err = resolver
            .resolve(&format!(
                "file:{}/connector_link",
                base_dir.path().display()
            ))
            .await
            .unwrap_err();
        assert!(matches!(err, SecretError::UnsupportedRef { .. }));
    }
}
