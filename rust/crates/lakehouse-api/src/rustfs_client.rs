//! One place that turns the `RUSTFS_*` settings into an S3 client over the
//! warehouse bucket, and the one rule for how its credentials are
//! resolved.
//!
//! Two callers need that client and must not each carry their own copy of
//! the rule: the `RustFS` health probe ([`crate::health`]) and the store
//! for uploaded files ([`crate::upload_store`], ADR 0014, decision 1). A
//! second copy is how one of them ends up on the wrong resolver.
//!
//! # The credential rule
//!
//! The access key and the secret key come from
//! `RUSTFS_ACCESS_KEY_SECRET_REF` / `RUSTFS_SECRET_KEY_SECRET_REF`
//! (ADR 0002 `secretRef`s, never a literal key in config), resolved
//! through an [`ExactMatchSecretResolver`]: it admits exactly those two
//! strings and refuses everything else as `NotAllowed`. The two refs are
//! deliberately NOT resolved through `AppState::connector_secret_resolver`:
//! that one is allowlisted to a disjoint set of `CONNECTOR_*` patterns, so
//! it refuses both (WS5 plan review U5), and its allowlist names
//! connector-dedicated variables, never the API's own secrets
//! (`docs/CODE-STANDARD.md` section 3.3). Nor through
//! [`lakehouse_core::secret::AllowlistedSecretResolver`]: its entries are
//! `prefix*suffix` patterns that need exactly one `*`
//! ([`lakehouse_core::secret::pattern_matches`]), and an ordinary
//! `env:RUSTFS_ACCESS_KEY` has none, which trips that function's
//! `debug_assert!` in every debug and test build. So [`build_client`] takes
//! only `&Config`: nothing a caller passes in can widen what it resolves.
//!
//! # Fail closed, no I/O
//!
//! Either ref unset is [`RustfsClientError::NotConfigured`], returned before
//! any resolver is built. A ref that resolves to an empty or whitespace-only
//! value is `NotConfigured` too (finding `D1`, below). Building the client opens no connection, so a
//! deployment without object storage fails here, not on the network. The
//! endpoint is the deployment's own setting (`RUSTFS_S3_ENDPOINT`), never a
//! request value, so no SSRF check applies, unlike connector dials
//! (`connector_dial`).

use lakehouse_core::secret::{EnvSecretResolver, SecretError, SecretResolver, SecretValue};
use object_store::aws::{AmazonS3, AmazonS3Builder};

use crate::config::Config;

/// Why no client could be built. Carries the detail for the log; neither
/// caller returns it to a user (the health probe reports a fixed word, the
/// upload store a fixed sentence).
#[derive(Debug, thiserror::Error)]
pub enum RustfsClientError {
    /// `RUSTFS_ACCESS_KEY_SECRET_REF` or `RUSTFS_SECRET_KEY_SECRET_REF` is
    /// unset, or a ref resolves to an empty or whitespace-only value. An
    /// unconfigured credential is a configuration gap, not a failure that
    /// was probed.
    #[error(
        "RUSTFS_ACCESS_KEY_SECRET_REF or RUSTFS_SECRET_KEY_SECRET_REF is not set or resolves to nothing"
    )]
    NotConfigured,
    /// A ref is set but does not resolve: the variable it names is unset,
    /// or the ref is not an `env:` reference. The error names the ref,
    /// never a value.
    #[error("a RustFS credential reference did not resolve: {0}")]
    CredentialUnavailable(SecretError),
    /// Both credentials resolved, but `object_store` rejected the endpoint
    /// or bucket settings.
    #[error("the S3 client could not be built: {0}")]
    Misconfigured(object_store::Error),
}

/// A secret resolver that admits only the exact `secretRef` strings this
/// deployment configured for `rustfs_access_key_secret_ref` and
/// `rustfs_secret_key_secret_ref`, in front of `inner`.
///
/// It does a direct equality check against the (at most two) refs it was
/// constructed with, rather than using
/// [`lakehouse_core::secret::AllowlistedSecretResolver`], whose patterns
/// each need exactly one `*` wildcard (see that type's `pattern_matches`).
#[derive(Debug)]
pub(crate) struct ExactMatchSecretResolver<R> {
    inner: R,
    allowed: Vec<String>,
    name: &'static str,
}

impl<R> ExactMatchSecretResolver<R> {
    /// `allowed` are the refs that may be resolved; `name` labels this
    /// resolver in a `NotAllowed` error.
    pub(crate) fn new(
        inner: R,
        allowed: impl IntoIterator<Item = String>,
        name: &'static str,
    ) -> Self {
        Self {
            inner,
            allowed: allowed.into_iter().collect(),
            name,
        }
    }
}

impl<R: SecretResolver> SecretResolver for ExactMatchSecretResolver<R> {
    async fn resolve(&self, secret_ref: &str) -> Result<SecretValue, SecretError> {
        if !self.allowed.iter().any(|a| a == secret_ref) {
            return Err(SecretError::NotAllowed {
                secret_ref: secret_ref.to_owned(),
                resolver: self.name,
            });
        }
        self.inner.resolve(secret_ref).await
    }
}

/// Build the S3 client over the warehouse bucket, resolving the two
/// credential refs from the process environment.
///
/// # Errors
///
/// [`RustfsClientError::NotConfigured`] when either ref is unset (nothing
/// is resolved and nothing is dialled) or resolves to an empty or
/// whitespace-only value, [`RustfsClientError::CredentialUnavailable`]
/// when a ref does not resolve, [`RustfsClientError::Misconfigured`] when
/// `object_store` rejects the endpoint or bucket.
pub async fn build_client(config: &Config) -> Result<AmazonS3, RustfsClientError> {
    build_client_with(config, EnvSecretResolver::new()).await
}

/// [`build_client`] with the source of the credential values injected, so a
/// test can resolve against an explicit map instead of the process
/// environment. `inner` is only the SOURCE of values: whatever it is, the
/// exact-match allowlist of the two configured refs is applied in front of
/// it, so it cannot widen what this function resolves.
///
/// # Errors
///
/// As [`build_client`].
pub(crate) async fn build_client_with<R: SecretResolver>(
    config: &Config,
    inner: R,
) -> Result<AmazonS3, RustfsClientError> {
    let (Some(access_ref), Some(secret_ref)) = (
        config.rustfs_access_key_secret_ref.as_deref(),
        config.rustfs_secret_key_secret_ref.as_deref(),
    ) else {
        return Err(RustfsClientError::NotConfigured);
    };
    let resolver = ExactMatchSecretResolver::new(
        inner,
        [access_ref.to_owned(), secret_ref.to_owned()],
        "rustfs-client",
    );
    let access_key = resolver
        .resolve(access_ref)
        .await
        .map_err(RustfsClientError::CredentialUnavailable)?;
    let secret_key = resolver
        .resolve(secret_ref)
        .await
        .map_err(RustfsClientError::CredentialUnavailable)?;
    // Finding `D1`: compose points the refs at `UPLOAD_S3_ACCESS_KEY` /
    // `UPLOAD_S3_SECRET_KEY` by default, and those are passed through as
    // empty strings when the operator sets nothing. An empty value is
    // "not configured", the same as an unset ref; without this an
    // unconfigured stack would build a client with empty keys, the health
    // tile would turn from "unknown" to a failure, and uploads would answer
    // "authentication failed" instead of "not configured". Checked after
    // both resolve, so no value is ever put in an error.
    if access_key.expose_secret().trim().is_empty() || secret_key.expose_secret().trim().is_empty()
    {
        return Err(RustfsClientError::NotConfigured);
    }
    AmazonS3Builder::new()
        .with_endpoint(config.rustfs_s3_endpoint.clone())
        .with_region(config.rustfs_s3_region.clone())
        .with_bucket_name(config.lakehouse_warehouse_bucket.clone())
        .with_access_key_id(access_key.expose_secret())
        .with_secret_access_key(secret_key.expose_secret())
        // Self-hosted (`RustFS`), not real AWS S3 — same posture
        // `connector_probe::probe_s3` and `lakehouse-iceberg::storage` use:
        // plain HTTP inside the compose network (TLS ends at the ingress in
        // a real deployment), and path-style addressing
        // (`host/bucket/key`), because a bucket-as-subdomain needs DNS
        // entries no self-hosted object store in this stack provides.
        .with_virtual_hosted_style_request(false)
        .with_allow_http(true)
        .build()
        .map_err(RustfsClientError::Misconfigured)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::collections::HashMap;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    fn config(env: &[(&str, &str)]) -> Config {
        let map: HashMap<String, String> = env
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        Config::from_map(&map).unwrap()
    }

    /// A source that counts how often it is asked, to prove that a
    /// deployment without both refs never reaches a resolver at all.
    #[derive(Debug, Default)]
    struct CountingResolver {
        calls: Arc<AtomicUsize>,
    }

    impl SecretResolver for CountingResolver {
        async fn resolve(&self, secret_ref: &str) -> Result<SecretValue, SecretError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Err(SecretError::NotFound {
                secret_ref: secret_ref.to_owned(),
                resolver: "counting",
            })
        }
    }

    const BOTH_REFS: [(&str, &str); 2] = [
        ("RUSTFS_ACCESS_KEY_SECRET_REF", "env:TEST_RUSTFS_ACCESS"),
        ("RUSTFS_SECRET_KEY_SECRET_REF", "env:TEST_RUSTFS_SECRET"),
    ];

    fn values() -> EnvSecretResolver {
        EnvSecretResolver::with_map(HashMap::from([
            ("TEST_RUSTFS_ACCESS".to_owned(), "access-id".to_owned()),
            ("TEST_RUSTFS_SECRET".to_owned(), "secret-value".to_owned()),
            ("DATABASE_URL".to_owned(), "postgres://leak".to_owned()),
        ]))
    }

    #[tokio::test]
    async fn an_unset_ref_is_not_configured_and_no_resolver_is_consulted() {
        for env in [Vec::new(), vec![BOTH_REFS[0]], vec![BOTH_REFS[1]]] {
            let calls = Arc::new(AtomicUsize::new(0));
            let inner = CountingResolver {
                calls: Arc::clone(&calls),
            };
            let err = build_client_with(&config(&env), inner).await.unwrap_err();
            assert!(matches!(err, RustfsClientError::NotConfigured), "{err:?}");
            assert_eq!(calls.load(Ordering::SeqCst), 0, "{env:?}");
        }
    }

    /// Finding `D1`: a ref that resolves, but to nothing, is the compose
    /// default for a stack whose operator set no upload credential.
    #[tokio::test]
    async fn an_empty_or_whitespace_value_is_not_configured_and_nothing_is_dialled() {
        for blank in ["", "   ", "\t\n"] {
            for blank_key in ["TEST_RUSTFS_ACCESS", "TEST_RUSTFS_SECRET"] {
                let mut map = HashMap::from([
                    ("TEST_RUSTFS_ACCESS".to_owned(), "access-id".to_owned()),
                    ("TEST_RUSTFS_SECRET".to_owned(), "secret-value".to_owned()),
                ]);
                map.insert(blank_key.to_owned(), blank.to_owned());
                let err = build_client_with(&config(&BOTH_REFS), EnvSecretResolver::with_map(map))
                    .await
                    .unwrap_err();
                assert!(
                    matches!(err, RustfsClientError::NotConfigured),
                    "{blank_key} = {blank:?}: {err:?}"
                );
            }
        }
    }

    #[tokio::test]
    async fn a_ref_that_does_not_resolve_is_credential_unavailable() {
        // The refs are set, the values are not there.
        let err = build_client_with(
            &config(&BOTH_REFS),
            EnvSecretResolver::with_map(HashMap::new()),
        )
        .await
        .unwrap_err();
        assert!(
            matches!(
                err,
                RustfsClientError::CredentialUnavailable(SecretError::NotFound { .. })
            ),
            "{err:?}"
        );

        // A ref that is not an `env:` reference is refused, not guessed at.
        let cfg = config(&[
            ("RUSTFS_ACCESS_KEY_SECRET_REF", "file:/run/secrets/access"),
            ("RUSTFS_SECRET_KEY_SECRET_REF", "env:TEST_RUSTFS_SECRET"),
        ]);
        let err = build_client_with(&cfg, values()).await.unwrap_err();
        assert!(
            matches!(
                err,
                RustfsClientError::CredentialUnavailable(SecretError::UnsupportedRef { .. })
            ),
            "{err:?}"
        );
    }

    #[tokio::test]
    async fn both_refs_resolving_builds_a_client_over_the_configured_bucket() {
        let cfg = config(&[
            BOTH_REFS[0],
            BOTH_REFS[1],
            ("LAKEHOUSE_WAREHOUSE_BUCKET", "bucket-for-the-test"),
        ]);
        let client = build_client_with(&cfg, values()).await.unwrap();
        assert_eq!(client.to_string(), "AmazonS3(bucket-for-the-test)");
    }

    /// The exact-match resolver admits only the refs it was constructed
    /// with -- a ref that is NOT one of the two configured refs is refused,
    /// never silently resolved, even when the source could resolve it.
    #[tokio::test]
    async fn exact_match_secret_resolver_refuses_anything_outside_its_allowlist() {
        let resolver = ExactMatchSecretResolver::new(
            values(),
            ["env:TEST_RUSTFS_ACCESS".to_owned()],
            "rustfs-client",
        );
        assert!(resolver.resolve("env:TEST_RUSTFS_ACCESS").await.is_ok());
        let err = resolver.resolve("env:DATABASE_URL").await.unwrap_err();
        assert!(matches!(err, SecretError::NotAllowed { .. }));
        let err = resolver
            .resolve("env:TEST_RUSTFS_SECRET")
            .await
            .unwrap_err();
        assert!(
            matches!(err, SecretError::NotAllowed { .. }),
            "a ref the source could resolve is still refused unless configured"
        );
    }

    /// A source that records every ref it is asked for, then resolves it
    /// from the injected map.
    #[derive(Debug)]
    struct Recording {
        asked: Arc<std::sync::Mutex<Vec<String>>>,
        inner: EnvSecretResolver,
    }

    impl SecretResolver for Recording {
        async fn resolve(&self, secret_ref: &str) -> Result<SecretValue, SecretError> {
            self.asked.lock().unwrap().push(secret_ref.to_owned());
            self.inner.resolve(secret_ref).await
        }
    }

    /// The source behind the allowlist is asked for each configured ref
    /// once, the access key first, and for nothing else.
    #[tokio::test]
    async fn the_client_resolves_each_configured_ref_once_and_nothing_else() {
        let asked = Arc::new(std::sync::Mutex::new(Vec::new()));
        let source = Recording {
            asked: Arc::clone(&asked),
            inner: values(),
        };
        build_client_with(&config(&BOTH_REFS), source)
            .await
            .unwrap();
        assert_eq!(
            *asked.lock().unwrap(),
            ["env:TEST_RUSTFS_ACCESS", "env:TEST_RUSTFS_SECRET"]
        );
    }
}
