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
//! [`SecretValue`]s (ADR 0002). **The resolver `probe` is handed must
//! already be scoped to a fixed allowlist** — see
//! `lakehouse_core::secret::AllowlistedSecretResolver` and
//! `crate::state::AppState::connector_secret_resolver`'s doc comment — NOT
//! the general-purpose `EnvSecretResolver` the rest of this process uses.
//! `probe` dials a `host` the SAME caller who supplied `secret_ref` also
//! controls (via `POST /api/connectors`), so an unrestricted resolver here
//! would let a `connector:manage` principal name any process secret
//! (`env:DATABASE_URL`, `env:CH_PASSWORD`, ...) and exfiltrate it to
//! infrastructure they own. The resolved value is used ONLY to build a
//! transient client (a `sqlx` connect string, an `object_store` builder
//! call) — it is never logged, never included in an [`Outcome::message`],
//! and never returned to the caller.
//!
//! # Error messages never echo upstream data
//!
//! A connection failure is reported as one of a small, fixed set of
//! generic failure classes (`classify_sqlx_error`/`classify_object_store_error`)
//! — "connection refused", "timed out", "authentication failed", "TLS
//! error", ... — never the underlying error's raw `Display` text. That
//! distinction matters specifically for `object_store`: a non-2xx HTTP
//! response from the dialed host is wrapped with the response body
//! included verbatim in its `Display` impl, and `probe_s3`'s `host`/`GET`
//! target is caller-controlled (see above), so echoing that text back
//! would turn `/test` into a working blind SSRF response-reader. Same
//! reasoning for `probe_postgres`.
//!
//! # SSRF: private/internal ranges are blocked before dialling
//!
//! [`probe_postgres`] and [`probe_s3`] resolve `host` via DNS
//! ([`resolve_checked`]) and refuse to dial it if ANY resolved address
//! falls in a private/internal range (RFC1918, loopback, link-local —
//! which covers cloud metadata endpoints at `169.254.169.254` — or IPv6
//! unique-local) — see [`is_blocked_ip`]. Checking is done against the
//! resolved address, not the literal `host` string: a hostname that
//! resolves to `10.0.0.5` is blocked exactly the same as a literal
//! `10.0.0.5`, so the check cannot be bypassed by pointing DNS at an
//! attacker-controlled name that merely LOOKS external. This is
//! `allow_internal_hosts: bool`-gated (default: blocked — see
//! `crate::config::Config::connector_probe_allow_internal_hosts`) because
//! this deployment's own seeded connectors
//! (`rust/migrations/0022_prune_connector_seed.sql`) legitimately point at
//! `postgres:5432` and `http://rustfs:9000`, both internal compose-network
//! names; a demo/compose deployment opts out of the block explicitly
//! rather than the block being off by default everywhere.
//!
//! # Timeouts
//!
//! Every dial is bounded by [`DIAL_TIMEOUT`] via [`tokio::time::timeout`]
//! — a hanging operator-configured host can delay a `/test` request by at
//! most that long, never indefinitely, and never ties up the request task
//! beyond it. This sits well inside `routes::DEFAULT_REQUEST_TIMEOUT`'s
//! 60s outer bound. No retries: one attempt, one measured result.

use std::net::IpAddr;
use std::time::{Duration, Instant};

use lakehouse_core::secret::DynSecretResolver;
use lakehouse_store::connectors::ConnectorDialInfo;
use object_store::ObjectStore;
use object_store::aws::AmazonS3Builder;
use sqlx::Connection;
use sqlx::postgres::{PgConnectOptions, PgConnection, PgSslMode};

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
/// `secret_ref`(s) via `resolver` (see the module doc comment: MUST already
/// be scoped to a fixed allowlist, never the general-purpose resolver).
/// `allow_internal_hosts` gates the SSRF blocklist — see the module doc
/// comment and [`Config::connector_probe_allow_internal_hosts`]. Never
/// panics on a malformed `host` or an unresolvable `secret_ref` — both
/// become [`Outcome::misconfigured`], never a crash.
///
/// [`Config::connector_probe_allow_internal_hosts`]: crate::config::Config::connector_probe_allow_internal_hosts
pub async fn probe(
    info: &ConnectorDialInfo,
    resolver: &dyn DynSecretResolver,
    allow_internal_hosts: bool,
) -> Outcome {
    let kind = info.kind.to_lowercase();
    if kind.contains("postgres") {
        probe_postgres(info, resolver, allow_internal_hosts).await
    } else if kind.contains("object storage") || kind.contains("s3") {
        probe_s3(info, resolver, allow_internal_hosts).await
    } else {
        Outcome::unsupported(&info.kind)
    }
}

/// A private/internal address this build refuses to dial by default — see
/// the module doc comment's "SSRF" section. Covers:
///
/// - RFC1918 IPv4 (`10/8`, `172.16/12`, `192.168/16`) via
///   [`std::net::Ipv4Addr::is_private`].
/// - Loopback (`127/8`, `::1`).
/// - Link-local (`169.254/16` — this is where cloud metadata services
///   (AWS/GCP/Azure instance metadata) live, `fe80::/10`).
/// - The unspecified address (`0.0.0.0`, `::`), which several TCP stacks
///   treat as "this host".
/// - IPv6 unique-local (`fc00::/7`).
/// - An IPv4-mapped IPv6 address (`::ffff:a.b.c.d`) whose embedded IPv4
///   address is itself any of the above — otherwise this whole check is
///   bypassable by asking DNS for an AAAA record wrapping a blocked IPv4
///   address.
fn is_blocked_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_private() || v4.is_loopback() || v4.is_link_local() || v4.is_unspecified()
        }
        IpAddr::V6(v6) => {
            if v6.is_loopback() || v6.is_unspecified() {
                return true;
            }
            if let Some(mapped) = v6.to_ipv4_mapped() {
                return mapped.is_private()
                    || mapped.is_loopback()
                    || mapped.is_link_local()
                    || mapped.is_unspecified();
            }
            let first_segment = v6.segments()[0];
            let is_unique_local = first_segment & 0xfe00 == 0xfc00; // fc00::/7
            let is_link_local = first_segment & 0xffc0 == 0xfe80; // fe80::/10
            is_unique_local || is_link_local
        }
    }
}

/// Resolve `host:port` via DNS and refuse it if any resolved address is
/// private/internal (unless `allow_internal_hosts`) — see the module doc
/// comment's "SSRF" section for why this resolves rather than
/// pattern-matching the literal `host` string. Returns `Err` with a message
/// safe to surface directly (never includes upstream response data — there
/// is none at this stage, only DNS resolution).
async fn resolve_checked(host: &str, port: u16, allow_internal_hosts: bool) -> Result<(), String> {
    let addrs: Vec<std::net::SocketAddr> = match tokio::net::lookup_host((host, port)).await {
        Ok(iter) => iter.collect(),
        Err(err) => return Err(format!("could not resolve host {host:?}: {err}")),
    };
    if addrs.is_empty() {
        return Err(format!("host {host:?} did not resolve to any address"));
    }
    if allow_internal_hosts {
        return Ok(());
    }
    for addr in &addrs {
        if is_blocked_ip(&addr.ip()) {
            return Err(format!(
                "refusing to dial {host:?}: it resolves to {}, a private/internal address this \
                 build blocks by default (set CONNECTOR_PROBE_ALLOW_INTERNAL_HOSTS=true to \
                 allow this for a trusted internal deployment)",
                addr.ip()
            ));
        }
    }
    Ok(())
}

/// The pieces [`probe_postgres`] needs to build a [`PgConnectOptions`]
/// field-by-field, parsed from `host` shaped
/// `<user>@<host>:<port>/<database>` (`0022_prune_connector_seed.sql`'s
/// `conn-pg-lakehouse` row). This is NOT a DSN (it carries no password) —
/// [`probe_postgres`] combines it with the resolved credential in-memory,
/// for this dial only, and NEVER by interpolating strings into a
/// `postgres://` URL: `PgConnectOptions` fields are individually bound, so
/// a `host`/`database` value that smuggled `sslmode=disable`,
/// `options=...`, or a `?`-query string cannot reach `sqlx` as anything
/// other than a literal hostname/database name.
struct PgDialTarget<'a> {
    user: &'a str,
    host: &'a str,
    port: u16,
    database: &'a str,
}

fn parse_postgres_host(host: &str) -> Option<PgDialTarget<'_>> {
    let (user, rest) = host.split_once('@')?;
    if user.is_empty() {
        return None;
    }
    let (host_and_port, database) = rest.split_once('/')?;
    if database.is_empty() {
        return None;
    }
    let (host, port_str) = host_and_port.rsplit_once(':')?;
    if host.is_empty() {
        return None;
    }
    let port: u16 = port_str.parse().ok()?;
    Some(PgDialTarget {
        user,
        host,
        port,
        database,
    })
}

/// Classify a `sqlx` connection error into one of a small set of generic
/// failure classes — see the module doc comment's "Error messages never
/// echo upstream data" section for why this never formats `err` itself.
fn classify_sqlx_error(err: &sqlx::Error) -> &'static str {
    match err {
        sqlx::Error::Io(io_err) => match io_err.kind() {
            std::io::ErrorKind::ConnectionRefused => "connection refused",
            std::io::ErrorKind::TimedOut => "timed out",
            std::io::ErrorKind::PermissionDenied => "permission denied",
            _ => "connection failed",
        },
        sqlx::Error::Tls(_) => "TLS error",
        sqlx::Error::Database(db_err) => match db_err.code().as_deref() {
            // PostgreSQL error codes: 28P01 invalid_password,
            // 28000 invalid_authorization_specification.
            Some("28P01" | "28000") => "authentication failed",
            _ => "database rejected the connection",
        },
        sqlx::Error::PoolTimedOut => "timed out",
        _ => "connection failed",
    }
}

async fn probe_postgres(
    info: &ConnectorDialInfo,
    resolver: &dyn DynSecretResolver,
    allow_internal_hosts: bool,
) -> Outcome {
    let Some(target) = parse_postgres_host(&info.host) else {
        return Outcome::misconfigured(
            "connector is misconfigured: PostgreSQL host must be shaped \
             \"<user>@<host>:<port>/<database>\"",
        );
    };
    if let Err(message) = resolve_checked(target.host, target.port, allow_internal_hosts).await {
        return Outcome::misconfigured(message);
    }
    let password = match resolver.resolve_dyn(&info.secret_ref).await {
        Ok(secret) => secret,
        Err(err) => {
            return Outcome::misconfigured(format!(
                "could not resolve the connector's credential: {err}"
            ));
        }
    };
    let options = PgConnectOptions::new()
        .host(target.host)
        .port(target.port)
        .username(target.user)
        .password(password.expose_secret())
        .database(target.database)
        // Pinned explicitly rather than left to sqlx's default so the
        // effective TLS posture is a decision this module states, not an
        // artifact of whatever `sqlx` happens to default to. `Prefer`
        // (attempt TLS, fall back to plaintext) matches this deployment's
        // compose-network Postgres, which does not terminate TLS.
        .ssl_mode(PgSslMode::Prefer);

    let started = Instant::now();
    let attempt = tokio::time::timeout(DIAL_TIMEOUT, async {
        let mut conn = PgConnection::connect_with(&options).await?;
        sqlx::query("SELECT 1").execute(&mut conn).await?;
        conn.close().await
    })
    .await;
    let elapsed = started.elapsed();

    match attempt {
        Ok(Ok(())) => Outcome::success(elapsed, "Connected via PostgreSQL and ran SELECT 1."),
        Ok(Err(err)) => Outcome::failure(
            elapsed,
            format!(
                "PostgreSQL connection failed: {}",
                classify_sqlx_error(&err)
            ),
        ),
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

/// Extracts `(host, port)` from an `http://`/`https://` endpoint URL,
/// without pulling in a full URL-parsing dependency — `object_store`'s
/// `AmazonS3Builder::with_endpoint` only ever needs `host`/`port` from
/// this to be handed to [`resolve_checked`], not the whole URL structure.
fn parse_endpoint_host_port(endpoint: &str) -> Option<(&str, u16)> {
    let (authority, default_port) = if let Some(rest) = endpoint.strip_prefix("https://") {
        (rest, 443)
    } else if let Some(rest) = endpoint.strip_prefix("http://") {
        (rest, 80)
    } else {
        return None;
    };
    let authority = authority.split(['/', '?', '#']).next()?;
    if authority.is_empty() {
        return None;
    }
    match authority.rsplit_once(':') {
        Some((host, port_str)) => {
            let port: u16 = port_str.parse().ok()?;
            Some((host, port))
        }
        None => Some((authority, default_port)),
    }
}

/// Classify an `object_store` request error into one of a small set of
/// generic failure classes — see the module doc comment's "Error messages
/// never echo upstream data" section. Deliberately never formats `err`
/// itself: `object_store::Error::Generic`'s `Display` impl includes the
/// upstream HTTP response body verbatim, and this probe's `host` is
/// caller-controlled (see the module doc comment), so doing so would leak
/// arbitrary response bodies from wherever the caller pointed this probe.
fn classify_object_store_error(err: &object_store::Error) -> &'static str {
    match err {
        object_store::Error::NotFound { .. } => "not found",
        object_store::Error::PermissionDenied { .. } => "permission denied",
        object_store::Error::Unauthenticated { .. } => "authentication failed",
        object_store::Error::NotSupported { .. } => "not supported",
        _ => classify_via_reqwest_source(err),
    }
}

/// Walks an error's `source()` chain looking for a wrapped
/// [`reqwest::Error`] to classify connect/timeout failures more precisely.
/// Only ever reads `reqwest::Error`'s typed accessors (`is_connect`,
/// `is_timeout`) — never its `Display` text, which can itself carry
/// upstream detail.
fn classify_via_reqwest_source(err: &(dyn std::error::Error + 'static)) -> &'static str {
    let mut current: Option<&(dyn std::error::Error + 'static)> = err.source();
    while let Some(inner) = current {
        if let Some(reqwest_err) = inner.downcast_ref::<reqwest::Error>() {
            if reqwest_err.is_connect() {
                return "connection refused";
            }
            if reqwest_err.is_timeout() {
                return "timed out";
            }
            if reqwest_err.is_status() {
                return "request rejected by upstream";
            }
        }
        current = inner.source();
    }
    "connection failed"
}

async fn probe_s3(
    info: &ConnectorDialInfo,
    resolver: &dyn DynSecretResolver,
    allow_internal_hosts: bool,
) -> Outcome {
    let Some((endpoint, bucket)) = parse_s3_host(&info.host) else {
        return Outcome::misconfigured(
            "connector is misconfigured: S3 host must be shaped \"<endpoint>|<bucket>\"",
        );
    };
    let Some((endpoint_host, endpoint_port)) = parse_endpoint_host_port(endpoint) else {
        return Outcome::misconfigured(
            "connector is misconfigured: S3 endpoint must be an http(s):// URL",
        );
    };
    if let Err(message) = resolve_checked(endpoint_host, endpoint_port, allow_internal_hosts).await
    {
        return Outcome::misconfigured(message);
    }
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
        Ok(Err(err)) => Outcome::failure(
            elapsed,
            format!(
                "S3 connection failed: {}",
                classify_object_store_error(&err)
            ),
        ),
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
            let outcome = probe(&info(kind, "h", "env:X", None), &resolver, false).await;
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
            false,
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
        // `allow_internal_hosts: true` — this test is specifically about
        // the dial-failure path, not the SSRF blocklist (covered
        // separately below), and 127.0.0.1 is itself blocked by default.
        let outcome = probe(
            &info(
                "PostgreSQL",
                "u@127.0.0.1:1/db",
                "env:PG_TEST_PASSWORD",
                None,
            ),
            &resolver,
            true,
        )
        .await;
        assert!(outcome.supported);
        assert!(!outcome.ok);
        assert!(outcome.latency_ms.is_some());
        assert!(outcome.message.contains("PostgreSQL connection failed"));
    }

    /// Blocker 2: a loopback host must be refused by default, before any
    /// dial is attempted — no latency, because no attempt was made.
    #[tokio::test]
    async fn postgres_loopback_host_is_blocked_by_default() {
        let mut map = std::collections::HashMap::new();
        map.insert("PG_TEST_PASSWORD".to_owned(), "irrelevant".to_owned());
        let resolver = EnvSecretResolver::with_map(map);
        let outcome = probe(
            &info(
                "PostgreSQL",
                "u@127.0.0.1:1/db",
                "env:PG_TEST_PASSWORD",
                None,
            ),
            &resolver,
            false,
        )
        .await;
        assert!(outcome.supported);
        assert!(!outcome.ok);
        assert!(
            outcome.latency_ms.is_none(),
            "a blocked host must never be dialed, so no latency exists"
        );
        assert!(
            outcome.message.contains("private/internal"),
            "{}",
            outcome.message
        );
    }

    /// A private RFC1918 host (not just loopback) must also be blocked.
    #[tokio::test]
    async fn postgres_rfc1918_host_is_blocked_by_default() {
        let mut map = std::collections::HashMap::new();
        map.insert("PG_TEST_PASSWORD".to_owned(), "irrelevant".to_owned());
        let resolver = EnvSecretResolver::with_map(map);
        let outcome = probe(
            &info(
                "PostgreSQL",
                "u@10.1.2.3:5432/db",
                "env:PG_TEST_PASSWORD",
                None,
            ),
            &resolver,
            false,
        )
        .await;
        assert!(!outcome.ok);
        assert!(outcome.latency_ms.is_none());
    }

    /// The documented opt-out: with `allow_internal_hosts: true`, a
    /// loopback/private host is no longer blocked and the probe proceeds
    /// to a real (here, failing) dial attempt — proving the flag actually
    /// takes effect, not just that the default blocks.
    #[tokio::test]
    async fn postgres_loopback_host_is_reachable_when_internal_hosts_allowed() {
        let mut map = std::collections::HashMap::new();
        map.insert("PG_TEST_PASSWORD".to_owned(), "irrelevant".to_owned());
        let resolver = EnvSecretResolver::with_map(map);
        let outcome = probe(
            &info(
                "PostgreSQL",
                "u@127.0.0.1:1/db",
                "env:PG_TEST_PASSWORD",
                None,
            ),
            &resolver,
            true,
        )
        .await;
        assert!(outcome.supported);
        assert!(!outcome.ok);
        assert!(
            outcome.latency_ms.is_some(),
            "allow_internal_hosts=true must let this reach the real dial attempt"
        );
        assert!(outcome.message.contains("PostgreSQL connection failed"));
    }

    #[tokio::test]
    async fn s3_missing_secondary_secret_ref_is_misconfigured() {
        let resolver = EnvSecretResolver::with_map(std::collections::HashMap::new());
        let outcome = probe(
            &info("Object storage", "http://127.0.0.1:1|bucket", "env:X", None),
            &resolver,
            true,
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
            true,
        )
        .await;
        assert!(outcome.supported);
        assert!(!outcome.ok);
        assert!(outcome.latency_ms.is_some());
    }

    /// Blocker 2 for the S3 probe: a loopback endpoint is blocked before
    /// any HTTP request is made.
    #[tokio::test]
    async fn s3_loopback_endpoint_is_blocked_by_default() {
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
            false,
        )
        .await;
        assert!(outcome.supported);
        assert!(!outcome.ok);
        assert!(outcome.latency_ms.is_none());
        assert!(outcome.message.contains("private/internal"));
    }

    /// A link-local address (169.254/16 — cloud metadata service range)
    /// must be blocked too, not just RFC1918/loopback.
    #[tokio::test]
    async fn s3_link_local_metadata_endpoint_is_blocked_by_default() {
        let mut map = std::collections::HashMap::new();
        map.insert("AK".to_owned(), "ak".to_owned());
        map.insert("SK".to_owned(), "sk".to_owned());
        let resolver = EnvSecretResolver::with_map(map);
        let outcome = probe(
            &info(
                "Object storage",
                "http://169.254.169.254|bucket",
                "env:AK",
                Some("env:SK"),
            ),
            &resolver,
            false,
        )
        .await;
        assert!(!outcome.ok);
        assert!(outcome.latency_ms.is_none());
    }

    #[test]
    fn is_blocked_ip_covers_every_documented_range() {
        let blocked = [
            "127.0.0.1",       // loopback
            "10.0.0.1",        // RFC1918
            "172.16.5.5",      // RFC1918
            "192.168.1.1",     // RFC1918
            "169.254.169.254", // link-local / cloud metadata
            "0.0.0.0",         // unspecified
            "::1",             // IPv6 loopback
            "fc00::1",         // IPv6 unique-local
            "fe80::1",         // IPv6 link-local
            "::ffff:10.0.0.1", // IPv4-mapped IPv6, private
        ];
        for ip in blocked {
            let addr: IpAddr = ip.parse().unwrap();
            assert!(is_blocked_ip(&addr), "{ip} should be blocked");
        }

        let allowed = [
            "8.8.8.8",
            "1.1.1.1",
            "2606:4700:4700::1111", // Cloudflare public IPv6
        ];
        for ip in allowed {
            let addr: IpAddr = ip.parse().unwrap();
            assert!(!is_blocked_ip(&addr), "{ip} should not be blocked");
        }
    }

    /// Blocker 2's core requirement: the check must apply to the RESOLVED
    /// address, not the literal `host` string — a hostname that resolves
    /// to a blocked address is blocked exactly like the literal address
    /// would be, so pointing DNS at an innocuous-looking name cannot
    /// bypass the filter.
    #[tokio::test]
    async fn resolve_checked_blocks_a_hostname_that_resolves_to_loopback() {
        let err = resolve_checked("localhost", 1, false).await.unwrap_err();
        assert!(err.contains("private/internal"), "{err}");
    }

    #[tokio::test]
    async fn resolve_checked_allows_a_hostname_when_internal_hosts_allowed() {
        resolve_checked("localhost", 1, true)
            .await
            .expect("allow_internal_hosts=true must let a loopback-resolving host through");
    }
}
