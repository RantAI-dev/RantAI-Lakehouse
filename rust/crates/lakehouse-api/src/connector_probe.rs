//! A REAL connectivity probe for `POST /api/connectors/{id}/test`, for the
//! connector types this build actually knows how to dial.
//!
//! # What this module supports, and why only this much
//!
//! Two connector `type`s can be genuinely dialed today, both because the
//! dependency to do so already sits in this workspace and both because
//! this deployment's compose stack actually runs one:
//!
//! - **`PostgreSQL`** (`sqlx`, already a `lakehouse-api` dependency for
//!   `lakehouse-store`) — opens a real connection and runs `SELECT 1`.
//! - **S3-compatible object storage** (`object_store`, already a workspace
//!   dependency via `lakehouse-iceberg`) — does an authenticated
//!   `list_with_delimiter` (a cheap, bounded listing call; no data read or
//!   written).
//!
//! Every other seeded/registered connector `type` (Kafka, MQTT, `MongoDB`,
//! Oracle, SAP/ERP, SFTP, a REST/vendor API, ...) has **no dial
//! implementation in this build** — see [`probe`]'s `_ =>` arm. Those
//! report [`Outcome::unsupported`], never a fabricated latency or success.
//! Adding a new supported type means adding both a real client dependency
//! and a new arm here — never widening the `_` arm to claim more than this
//! build can back up.
//!
//! # Credential handling
//!
//! [`probe`] receives a [`lakehouse_store::connectors::ConnectorDialInfo`]
//! (see that type's doc comment: no `Debug` impl at all) and a
//! [`DynSecretResolver`] to turn its `secret_ref`(s) into actual
//! [`SecretValue`]s (ADR 0002). The resolved value is used ONLY to build a
//! transient client (a `sqlx` connect string, an `object_store` builder
//! call) — it is never logged, never included in an [`Outcome::message`],
//! and never returned to the caller. On a connection failure, the message
//! reported is the underlying client error's `Display` text; both `sqlx`'s
//! and `object_store`'s connection-error `Display` impls report
//! transport/auth failure descriptions (e.g. "connection refused",
//! "invalid credentials"), not the credential value itself, so this holds
//! without this module needing its own redaction step.
//!
//! # Timeouts
//!
//! Every dial is bounded by [`DIAL_TIMEOUT`] via [`tokio::time::timeout`]
//! — a hanging operator-configured host can delay a `/test` request by at
//! most that long, never indefinitely, and never ties up the request task
//! beyond it. This sits well inside `routes::DEFAULT_REQUEST_TIMEOUT`'s
//! 60s outer bound. No retries: one attempt, one measured result.

use std::time::{Duration, Instant};

use lakehouse_core::secret::DynSecretResolver;
use lakehouse_store::connectors::ConnectorDialInfo;
use object_store::ObjectStore;
use object_store::aws::AmazonS3Builder;
use sqlx::Connection;
use sqlx::postgres::PgConnection;

/// Bound on a single dial attempt (connect + one cheap operation). Chosen
/// to be a "few seconds" per the task brief — long enough that a healthy
/// LAN-local compose service never times out under normal load, short
/// enough that a hung/firewalled host resolves the request quickly.
const DIAL_TIMEOUT: Duration = Duration::from_secs(5);

/// The result of attempting (or declining to attempt) a connectivity
/// probe. Never carries a fabricated `latency_ms` — see the field doc
/// comment.
pub struct Outcome {
    /// Whether the probe succeeded. Always `false` when `supported` is
    /// `false`.
    pub ok: bool,
    /// Whether this build knows how to dial this connector's type.
    pub supported: bool,
    /// Real measured elapsed time, or `None` when `supported` is `false`
    /// (no attempt was made).
    pub latency_ms: Option<i64>,
    /// Human-readable result message.
    pub message: String,
}

impl Outcome {
    fn unsupported(kind: &str) -> Self {
        Self {
            ok: false,
            supported: false,
            latency_ms: None,
            message: format!(
                "This build cannot test a {kind:?} connector: no live-dial implementation \
                 exists for this connector type yet. Supported today: PostgreSQL, \
                 S3-compatible object storage."
            ),
        }
    }

    fn success(elapsed: Duration, message: impl Into<String>) -> Self {
        Self {
            ok: true,
            supported: true,
            latency_ms: Some(elapsed_millis(elapsed)),
            message: message.into(),
        }
    }

    fn failure(elapsed: Duration, message: impl Into<String>) -> Self {
        Self {
            ok: false,
            supported: true,
            latency_ms: Some(elapsed_millis(elapsed)),
            message: message.into(),
        }
    }

    /// A supported type that could not even be attempted — a
    /// misconfigured `host`/`secret_ref` shape, or the referenced
    /// credential not resolving. Still `supported: true` (this build DOES
    /// know how to dial this type) but no dial occurred, so no latency
    /// exists to report.
    fn misconfigured(message: impl Into<String>) -> Self {
        Self {
            ok: false,
            supported: true,
            latency_ms: None,
            message: message.into(),
        }
    }
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "a dial bounded by DIAL_TIMEOUT (5s) never approaches i64::MAX milliseconds"
)]
fn elapsed_millis(elapsed: Duration) -> i64 {
    elapsed.as_millis() as i64
}

/// Attempt a real connectivity probe for `info`, resolving its
/// `secret_ref`(s) via `resolver`. Never panics on a malformed `host` or an
/// unresolvable `secret_ref` — both become [`Outcome::misconfigured`],
/// never a crash.
pub async fn probe(info: &ConnectorDialInfo, resolver: &dyn DynSecretResolver) -> Outcome {
    let kind = info.kind.to_lowercase();
    if kind.contains("postgres") {
        probe_postgres(info, resolver).await
    } else if kind.contains("object storage") || kind.contains("s3") {
        probe_s3(info, resolver).await
    } else {
        Outcome::unsupported(&info.kind)
    }
}

/// Parses `host` as `<user>@<host>:<port>/<database>` — the shape
/// `0022_prune_connector_seed.sql`'s `conn-pg-lakehouse` row uses. This is
/// NOT a DSN (it carries no password); [`probe_postgres`] combines it with
/// the resolved credential to build one, in-memory, for this dial only.
fn parse_postgres_host(host: &str) -> Option<(&str, &str)> {
    let (user, rest) = host.split_once('@')?;
    if user.is_empty() || rest.is_empty() {
        return None;
    }
    Some((user, rest))
}

async fn probe_postgres(info: &ConnectorDialInfo, resolver: &dyn DynSecretResolver) -> Outcome {
    let Some((user, host_and_db)) = parse_postgres_host(&info.host) else {
        return Outcome::misconfigured(
            "connector is misconfigured: PostgreSQL host must be shaped \
             \"<user>@<host>:<port>/<database>\"",
        );
    };
    let password = match resolver.resolve_dyn(&info.secret_ref).await {
        Ok(secret) => secret,
        Err(err) => {
            return Outcome::misconfigured(format!(
                "could not resolve the connector's credential: {err}"
            ));
        }
    };
    let url = format!(
        "postgres://{user}:{}@{host_and_db}",
        password.expose_secret()
    );

    let started = Instant::now();
    let attempt = tokio::time::timeout(DIAL_TIMEOUT, async {
        let mut conn = PgConnection::connect(&url).await?;
        sqlx::query("SELECT 1").execute(&mut conn).await?;
        conn.close().await
    })
    .await;
    let elapsed = started.elapsed();

    match attempt {
        Ok(Ok(())) => Outcome::success(elapsed, "Connected via PostgreSQL and ran SELECT 1."),
        Ok(Err(err)) => Outcome::failure(elapsed, format!("PostgreSQL connection failed: {err}")),
        Err(_) => Outcome::failure(
            elapsed,
            format!(
                "PostgreSQL connection timed out after {}s",
                DIAL_TIMEOUT.as_secs()
            ),
        ),
    }
}

/// Parses `host` as `<endpoint>|<bucket>` — the shape
/// `0022_prune_connector_seed.sql`'s `conn-s3-warehouse` row uses.
fn parse_s3_host(host: &str) -> Option<(&str, &str)> {
    let (endpoint, bucket) = host.split_once('|')?;
    if endpoint.is_empty() || bucket.is_empty() {
        return None;
    }
    Some((endpoint, bucket))
}

async fn probe_s3(info: &ConnectorDialInfo, resolver: &dyn DynSecretResolver) -> Outcome {
    let Some((endpoint, bucket)) = parse_s3_host(&info.host) else {
        return Outcome::misconfigured(
            "connector is misconfigured: S3 host must be shaped \"<endpoint>|<bucket>\"",
        );
    };
    let Some(secret_ref_secondary) = info.secret_ref_secondary.as_deref() else {
        return Outcome::misconfigured(
            "connector is misconfigured: an S3 connector needs both secretRef (access key id) \
             and a secondary secretRef (secret access key)",
        );
    };
    let access_key = match resolver.resolve_dyn(&info.secret_ref).await {
        Ok(secret) => secret,
        Err(err) => {
            return Outcome::misconfigured(format!(
                "could not resolve the connector's access-key credential: {err}"
            ));
        }
    };
    let secret_key = match resolver.resolve_dyn(secret_ref_secondary).await {
        Ok(secret) => secret,
        Err(err) => {
            return Outcome::misconfigured(format!(
                "could not resolve the connector's secret-key credential: {err}"
            ));
        }
    };

    let built = AmazonS3Builder::new()
        .with_endpoint(endpoint)
        .with_bucket_name(bucket)
        .with_access_key_id(access_key.expose_secret())
        .with_secret_access_key(secret_key.expose_secret())
        // Self-hosted (RustFS), not real AWS S3: path-style addressing, and
        // `object_store` must be told explicitly this is not talking to
        // AWS or it refuses a plain-http/self-signed endpoint outright —
        // same posture `lakehouse-iceberg::storage`'s client uses.
        .with_virtual_hosted_style_request(false)
        .with_allow_http(true)
        .build();
    let client = match built {
        Ok(client) => client,
        Err(err) => {
            return Outcome::misconfigured(format!("failed to build the S3 client: {err}"));
        }
    };

    let started = Instant::now();
    let attempt = tokio::time::timeout(DIAL_TIMEOUT, client.list_with_delimiter(None)).await;
    let elapsed = started.elapsed();

    match attempt {
        Ok(Ok(_)) => Outcome::success(elapsed, "Connected via S3 and listed the bucket."),
        Ok(Err(err)) => Outcome::failure(elapsed, format!("S3 connection failed: {err}")),
        Err(_) => Outcome::failure(
            elapsed,
            format!("S3 connection timed out after {}s", DIAL_TIMEOUT.as_secs()),
        ),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use lakehouse_core::secret::EnvSecretResolver;

    use super::*;

    fn info(
        kind: &str,
        host: &str,
        secret_ref: &str,
        secondary: Option<&str>,
    ) -> ConnectorDialInfo {
        ConnectorDialInfo {
            kind: kind.to_owned(),
            host: host.to_owned(),
            secret_ref: secret_ref.to_owned(),
            secret_ref_secondary: secondary.map(str::to_owned),
        }
    }

    #[tokio::test]
    async fn unsupported_kind_never_fabricates_a_latency_or_success() {
        let resolver = EnvSecretResolver::with_map(std::collections::HashMap::new());
        for kind in ["Kafka", "MQTT", "MongoDB", "Oracle", "SAP / ERP", "SFTP"] {
            let outcome = probe(&info(kind, "h", "env:X", None), &resolver).await;
            assert!(!outcome.supported, "{kind} must be unsupported");
            assert!(!outcome.ok, "{kind} must never report ok=true");
            assert!(
                outcome.latency_ms.is_none(),
                "{kind} must never report a latency"
            );
            assert!(
                outcome.message.contains(kind),
                "{kind}: {}",
                outcome.message
            );
        }
    }

    #[tokio::test]
    async fn postgres_malformed_host_is_misconfigured_not_a_panic() {
        let resolver = EnvSecretResolver::with_map(std::collections::HashMap::new());
        let outcome = probe(
            &info("PostgreSQL", "not-a-valid-host-shape", "env:X", None),
            &resolver,
        )
        .await;
        assert!(outcome.supported);
        assert!(!outcome.ok);
        assert!(outcome.latency_ms.is_none());
    }

    #[tokio::test]
    async fn postgres_unreachable_host_reports_real_elapsed_time_and_failure() {
        let mut map = std::collections::HashMap::new();
        map.insert("PG_TEST_PASSWORD".to_owned(), "irrelevant".to_owned());
        let resolver = EnvSecretResolver::with_map(map);
        // Port 1 on localhost: nothing listens there, so this fails fast
        // via connection-refused rather than exercising the 5s timeout —
        // keeps this test quick while still measuring a real elapsed time.
        let outcome = probe(
            &info(
                "PostgreSQL",
                "u@127.0.0.1:1/db",
                "env:PG_TEST_PASSWORD",
                None,
            ),
            &resolver,
        )
        .await;
        assert!(outcome.supported);
        assert!(!outcome.ok);
        assert!(outcome.latency_ms.is_some());
        assert!(outcome.message.contains("PostgreSQL connection failed"));
    }

    #[tokio::test]
    async fn s3_missing_secondary_secret_ref_is_misconfigured() {
        let resolver = EnvSecretResolver::with_map(std::collections::HashMap::new());
        let outcome = probe(
            &info("Object storage", "http://127.0.0.1:1|bucket", "env:X", None),
            &resolver,
        )
        .await;
        assert!(outcome.supported);
        assert!(!outcome.ok);
        assert!(outcome.latency_ms.is_none());
    }

    #[tokio::test]
    async fn s3_unreachable_endpoint_reports_real_elapsed_time_and_failure() {
        let mut map = std::collections::HashMap::new();
        map.insert("AK".to_owned(), "ak".to_owned());
        map.insert("SK".to_owned(), "sk".to_owned());
        let resolver = EnvSecretResolver::with_map(map);
        let outcome = probe(
            &info(
                "Object storage",
                "http://127.0.0.1:1|bucket",
                "env:AK",
                Some("env:SK"),
            ),
            &resolver,
        )
        .await;
        assert!(outcome.supported);
        assert!(!outcome.ok);
        assert!(outcome.latency_ms.is_some());
    }
}
