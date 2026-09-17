//! Real service-health probes backing `GET /api/ops/services` and
//! `overview.services` (WS5, grand plan §7).
//!
//! # What changed from the grand plan's own claim
//!
//! §7 lists `bronze_meta.replication_slot.lag_seconds` as an
//! `observability.ingestLagSeconds` input; that column does not exist —
//! `replication_slot` carries only `wal_retained_bytes`/
//! `confirmed_flush_lag_bytes` (byte counts), and there is no honest
//! byte-to-seconds conversion without a measured write throughput. See
//! `docs/superpowers/plans/2026-09-11-ws5-platform-signals.md`'s honesty
//! check for the full correction — this module does not attempt that
//! conversion, and the WAL-slot signal instead feeds the alerts engine
//! directly through `serving.replication_slot_health`, unrelated to
//! anything in this file.
//!
//! # The six probes, and why exactly these six
//!
//! `ClickHouse`, `Dagster`, Lakekeeper, `RustFS`, `OpenFGA`, `Trino` — the
//! deployment's core stack plus the two services that are genuinely
//! optional per compose profile. Each `probe_<service>` fn is bounded by
//! [`PROBE_TIMEOUT`] so one dead dependency can never stall the others —
//! [`probe_all`] runs all six concurrently via `tokio::join!`.
//!
//! # Unconfigured is not unhealthy
//!
//! `OpenFGA`/`Trino` are only dialed when their base URL is configured
//! (`crate::config::Config::openfga_url`/`trino_health_url` — see that
//! module's config doc comments for why `Trino` needs a second,
//! `None`-able field distinct from the always-defaulted query-engine
//! `trino_url`). Unconfigured reports [`ServiceHealth::checked`] `false`
//! and [`ServiceHealth::health_label`] `"unknown"`, never `"unhealthy"` —
//! see [`unknown`]'s doc comment. `RustFS`'s two credential refs are the
//! same story: unset means "never dialed," not "dialed and failed."
//!
//! # The 15 s cache
//!
//! [`cached_probe_all`] never serves a result whose own `checked_at` is
//! more than [`CACHE_WINDOW`] old — every read compares against the
//! current time, there is no background refresh timer that could drift.
//! The cache itself lives in `crate::state::AppState::health_cache`; this
//! module only owns the type and the comparison logic.

use std::sync::Arc;
use std::time::{Duration, Instant};

use lakehouse_clickhouse::{ChClient, ChError};
use lakehouse_core::secret::{SecretError, SecretResolver, SecretValue};
use object_store::ObjectStore;
use object_store::aws::AmazonS3Builder;
use serde::Serialize;
use time::OffsetDateTime;

use crate::config::Config;
use crate::connector_probe::classify_object_store_error;
use crate::state::AppState;

/// Every probe in this module is bounded by this timeout, matching
/// `DgClient::is_alive`'s existing 3 s bound — one dead service must never
/// stall `probe_all`'s `tokio::join!`.
const PROBE_TIMEOUT: Duration = Duration::from_secs(3);

/// One service's health, as this probe set can honestly report it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceHealth {
    /// Fixed, lowercase service id (`"clickhouse"`, `"dagster"`, …), used
    /// both in the JSON response and as this probe's row key.
    pub id: &'static str,
    /// Human-readable display name, e.g. `"ClickHouse (Hot analytical
    /// store)"`.
    pub name: &'static str,
    /// `true` only when a probe actually ran and it succeeded. `false` for
    /// both an unconfigured (unprobed) service AND a probed-but-failing
    /// one — callers distinguish those two cases via [`Self::checked`],
    /// never by `ok` alone (an unconfigured service must never count
    /// toward "unhealthy").
    pub ok: bool,
    /// `true` only when this service's URL was configured and a probe
    /// actually ran this request. `false` means "unknown" — no probe
    /// happened at all, not "probed and failed."
    pub checked: bool,
    /// Round-trip time of the probe request, when one ran. `None` for an
    /// unconfigured service.
    pub latency_ms: Option<i64>,
    /// The service's self-reported version string, when the probe's
    /// response carried one. `None` when unconfigured, unreachable, or the
    /// response didn't include a version field.
    pub version: Option<String>,
    /// ISO-8601 timestamp of when this probe last actually ran (or, for an
    /// unconfigured service, when this row was built).
    pub checked_at: String,
    /// A short, classified failure reason (never raw upstream error text —
    /// see [`classify_ch_error`]'s doc comment for why). `None` on success
    /// or when unconfigured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl ServiceHealth {
    /// WS1's `Health` vocabulary (`"healthy" | "degraded" | "unhealthy" |
    /// "unknown"`, `src/lib/status.ts:57`) as this probe set can honestly
    /// produce it: no probe here ever reports `"degraded"` (every probe is
    /// a binary reachability check), so this is a three-way, not
    /// four-way, map.
    #[must_use]
    pub fn health_label(&self) -> &'static str {
        match (self.checked, self.ok) {
            (false, _) => "unknown",
            (true, true) => "healthy",
            (true, false) => "unhealthy",
        }
    }
}

/// An unconfigured/unprobed service: `checked: false`, `ok: false` — see
/// [`ServiceHealth::health_label`], which maps this combination to
/// `"unknown"`, never `"unhealthy"`. This is the ONLY row an unconfigured
/// service ever produces — never a probe attempt, never a guessed health.
fn unknown(id: &'static str, name: &'static str, now: OffsetDateTime) -> ServiceHealth {
    ServiceHealth {
        id,
        name,
        ok: false,
        checked: false,
        latency_ms: None,
        version: None,
        checked_at: checked_at_iso(now),
        error: None,
    }
}

/// A probed-and-failed service.
fn unhealthy(
    id: &'static str,
    name: &'static str,
    now: OffsetDateTime,
    latency_ms: Option<i64>,
    error: impl Into<String>,
) -> ServiceHealth {
    ServiceHealth {
        id,
        name,
        ok: false,
        checked: true,
        latency_ms,
        version: None,
        checked_at: checked_at_iso(now),
        error: Some(error.into()),
    }
}

/// A probed-and-succeeded service.
fn healthy(
    id: &'static str,
    name: &'static str,
    now: OffsetDateTime,
    latency_ms: Option<i64>,
    version: Option<String>,
) -> ServiceHealth {
    ServiceHealth {
        id,
        name,
        ok: true,
        checked: true,
        latency_ms,
        version,
        checked_at: checked_at_iso(now),
        error: None,
    }
}

#[allow(
    clippy::cast_precision_loss,
    reason = "second-precision input to a millisecond-precision formatter, matching ops.rs's now_iso"
)]
fn checked_at_iso(now: OffsetDateTime) -> String {
    lakehouse_dagster::iso_from_unix_seconds(now.unix_timestamp() as f64)
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "a dial bounded by PROBE_TIMEOUT (3s) never approaches i64::MAX milliseconds"
)]
fn elapsed_millis(elapsed: Duration) -> i64 {
    elapsed.as_millis() as i64
}

/// Classify a [`ChError`] without ever calling `.to_string()`/`Display` on
/// the whole error: `ChError::Server(String)` carries `ClickHouse`'s raw
/// response body verbatim (see the module doc comment on that variant), so
/// passing it through would leak arbitrary server detail into a health
/// response nothing authenticates to read.
fn classify_ch_error(err: &ChError) -> String {
    match err {
        ChError::Transport(_) => "unreachable".to_owned(),
        ChError::Server(_) => "query failed".to_owned(),
        ChError::Cancelled => "timed out".to_owned(),
    }
}

// `probe_dagster` (below) dials `/server_info` with a raw `reqwest::Client`
// rather than going through `DgClient` — the plan's own test signature
// (`probe_dagster(&reqwest::Client::new(), url)`) calls for a bare client,
// and `/server_info` is a plain REST endpoint, not the GraphQL surface
// `DgClient`/`DgError` model. `DgError` is therefore never produced on
// this path, so there is no `classify_dg_error` twin here to keep — one
// would be dead code, which this workspace's lints deny.

async fn probe_clickhouse(ch: &ChClient) -> ServiceHealth {
    let now = OffsetDateTime::now_utc();
    let started = Instant::now();
    let attempt = tokio::time::timeout(PROBE_TIMEOUT, ch.rows("SELECT version()", None)).await;
    let elapsed = elapsed_millis(started.elapsed());
    match attempt {
        Ok(Ok(rows)) => {
            let version = rows
                .first()
                .and_then(|r| r.get("version()"))
                .and_then(|v| v.as_str())
                .map(str::to_owned);
            healthy(
                "clickhouse",
                "ClickHouse (Hot analytical store)",
                now,
                Some(elapsed),
                version,
            )
        }
        Ok(Err(err)) => unhealthy(
            "clickhouse",
            "ClickHouse (Hot analytical store)",
            now,
            Some(elapsed),
            classify_ch_error(&err),
        ),
        Err(_) => unhealthy(
            "clickhouse",
            "ClickHouse (Hot analytical store)",
            now,
            Some(elapsed),
            "timed out",
        ),
    }
}

/// `url` is the full `.../server_info` URL (not a base) — callers already
/// hold `Config::dagster_url` (the GraphQL endpoint) and derive this by
/// replacing its `/graphql` suffix, same derivation `DgClient::is_alive`
/// already does internally.
async fn probe_dagster(http: &reqwest::Client, url: &str) -> ServiceHealth {
    let now = OffsetDateTime::now_utc();
    let started = Instant::now();
    let attempt = tokio::time::timeout(PROBE_TIMEOUT, http.get(url).send()).await;
    let elapsed = elapsed_millis(started.elapsed());
    match attempt {
        Ok(Ok(resp)) if resp.status().is_success() => {
            let version = resp
                .json::<serde_json::Value>()
                .await
                .ok()
                .and_then(|body| {
                    body.get("dagster_version")
                        .and_then(|v| v.as_str())
                        .map(str::to_owned)
                });
            healthy(
                "dagster",
                "Dagster (Orchestration)",
                now,
                Some(elapsed),
                version,
            )
        }
        Ok(Ok(resp)) => unhealthy(
            "dagster",
            "Dagster (Orchestration)",
            now,
            Some(elapsed),
            format!("unexpected response ({})", resp.status().as_u16()),
        ),
        Ok(Err(_)) => unhealthy(
            "dagster",
            "Dagster (Orchestration)",
            now,
            Some(elapsed),
            "unreachable",
        ),
        Err(_) => unhealthy(
            "dagster",
            "Dagster (Orchestration)",
            now,
            Some(elapsed),
            "timed out",
        ),
    }
}

/// Strips everything from the first `/` after the scheme's `://` onward
/// (keeping only `scheme://host:port`) — `lakekeeper_catalog_uri`'s
/// default already includes a `/catalog` path suffix that `/health`/
/// `/management/v1/info` do not hang off (verified live against a running
/// `lake-catalog` container — see the plan's "External signals verified"
/// table). A string with fewer than three `/`s is already a bare origin
/// and is returned unchanged.
#[must_use]
fn lakekeeper_probe_base(catalog_uri: &str) -> String {
    let Some(scheme_end) = catalog_uri.find("://") else {
        return catalog_uri.to_owned();
    };
    let after_scheme = scheme_end + 3;
    match catalog_uri[after_scheme..].find('/') {
        Some(offset) => catalog_uri[..after_scheme + offset].to_owned(),
        None => catalog_uri.to_owned(),
    }
}

async fn probe_lakekeeper(http: &reqwest::Client, base: &str) -> ServiceHealth {
    let now = OffsetDateTime::now_utc();
    let started = Instant::now();
    let health_attempt =
        tokio::time::timeout(PROBE_TIMEOUT, http.get(format!("{base}/health")).send()).await;
    let elapsed = elapsed_millis(started.elapsed());
    let ok = matches!(&health_attempt, Ok(Ok(resp)) if resp.status().is_success());
    if !ok {
        return unhealthy(
            "lakekeeper",
            "Iceberg + Lakekeeper (Open tables)",
            now,
            Some(elapsed),
            match health_attempt {
                Ok(Ok(resp)) => format!("unexpected response ({})", resp.status().as_u16()),
                Ok(Err(_)) => "unreachable".to_owned(),
                Err(_) => "timed out".to_owned(),
            },
        );
    }
    // `/management/v1/info` is best-effort: a failure here does not flip
    // `ok` to `false` — `/health` already answered that question — it
    // only leaves `version` unset.
    let version = tokio::time::timeout(
        PROBE_TIMEOUT,
        http.get(format!("{base}/management/v1/info")).send(),
    )
    .await
    .ok()
    .and_then(Result::ok)
    .filter(|resp| resp.status().is_success());
    let version = match version {
        Some(resp) => resp
            .json::<serde_json::Value>()
            .await
            .ok()
            .and_then(|body| {
                body.get("version")
                    .and_then(|v| v.as_str())
                    .map(str::to_owned)
            }),
        None => None,
    };
    healthy(
        "lakekeeper",
        "Iceberg + Lakekeeper (Open tables)",
        now,
        Some(elapsed),
        version,
    )
}

/// A `RustFS`-only secret resolver, admitting only the exact `secretRef`
/// strings this deployment configured for
/// `rustfs_access_key_secret_ref`/`rustfs_secret_key_secret_ref` — never
/// `AppState::connector_secret_resolver` (allowlisted to a disjoint set of
/// three `CONNECTOR_*` PATTERNS, which would refuse both `RustFS` refs as
/// `NotAllowed`, WS5 plan review U5) and never
/// [`lakehouse_core::secret::AllowlistedSecretResolver`], whose
/// `pattern_matches` requires every allowlist entry to contain EXACTLY one
/// `*` wildcard (see that function's own doc comment) — an ordinary
/// `secretRef` string like `env:RUSTFS_ACCESS_KEY` has none, so handing it
/// to that type as a literal "pattern" would trip its own
/// `debug_assert!(false, ...)` in every debug/test build. This resolver
/// instead does a direct equality check against the (at most two) refs it
/// was constructed with.
#[derive(Debug)]
struct ExactMatchSecretResolver<R> {
    inner: R,
    allowed: Vec<String>,
    name: &'static str,
}

impl<R> ExactMatchSecretResolver<R> {
    fn new(inner: R, allowed: impl IntoIterator<Item = String>, name: &'static str) -> Self {
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

/// Never `checked: false` -> dial. When either
/// `rustfs_access_key_secret_ref`/`rustfs_secret_key_secret_ref` is
/// unconfigured, this returns [`unknown`] and never builds a resolver or
/// touches the network at all — an unconfigured credential is a config
/// gap, not a probed failure (WS5 plan review U5/Y1's exact posture,
/// applied to `RustFS` too).
///
/// Deliberately takes only `&Config` — no `DynSecretResolver`/
/// `connector_secret_resolver` parameter — so a future edit cannot widen
/// this function's signature to silently reintroduce the wrong resolver
/// (WS5 plan review U5's own regression-test ask).
async fn probe_rustfs(config: &Config) -> ServiceHealth {
    let now = OffsetDateTime::now_utc();
    let (Some(access_ref), Some(secret_ref)) = (
        config.rustfs_access_key_secret_ref.as_deref(),
        config.rustfs_secret_key_secret_ref.as_deref(),
    ) else {
        return unknown("rustfs", "RustFS (Object storage)", now);
    };
    let resolver = ExactMatchSecretResolver::new(
        lakehouse_core::secret::EnvSecretResolver::new(),
        [access_ref.to_owned(), secret_ref.to_owned()],
        "rustfs-health-probe",
    );
    let Ok(access_key) = resolver.resolve(access_ref).await else {
        return unhealthy(
            "rustfs",
            "RustFS (Object storage)",
            now,
            None,
            "credential unavailable",
        );
    };
    let Ok(secret_key) = resolver.resolve(secret_ref).await else {
        return unhealthy(
            "rustfs",
            "RustFS (Object storage)",
            now,
            None,
            "credential unavailable",
        );
    };
    let built = AmazonS3Builder::new()
        .with_endpoint(config.rustfs_s3_endpoint.clone())
        .with_region(config.rustfs_s3_region.clone())
        .with_bucket_name(config.lakehouse_warehouse_bucket.clone())
        .with_access_key_id(access_key.expose_secret())
        .with_secret_access_key(secret_key.expose_secret())
        // Self-hosted (RustFS), not real AWS S3 — same posture
        // `connector_probe::probe_s3` and `lakehouse-iceberg::storage` use.
        .with_virtual_hosted_style_request(false)
        .with_allow_http(true)
        .build();
    let Ok(client) = built else {
        return unhealthy(
            "rustfs",
            "RustFS (Object storage)",
            now,
            None,
            "misconfigured",
        );
    };
    let started = Instant::now();
    let attempt = tokio::time::timeout(PROBE_TIMEOUT, client.list_with_delimiter(None)).await;
    let elapsed = elapsed_millis(started.elapsed());
    match attempt {
        Ok(Ok(_)) => healthy(
            "rustfs",
            "RustFS (Object storage)",
            now,
            Some(elapsed),
            None,
        ),
        Ok(Err(err)) => unhealthy(
            "rustfs",
            "RustFS (Object storage)",
            now,
            Some(elapsed),
            classify_object_store_error(&err),
        ),
        Err(_) => unhealthy(
            "rustfs",
            "RustFS (Object storage)",
            now,
            Some(elapsed),
            "timed out",
        ),
    }
}

async fn probe_openfga(http: &reqwest::Client, base: &str) -> ServiceHealth {
    let now = OffsetDateTime::now_utc();
    let started = Instant::now();
    let attempt =
        tokio::time::timeout(PROBE_TIMEOUT, http.get(format!("{base}/healthz")).send()).await;
    let elapsed = elapsed_millis(started.elapsed());
    match attempt {
        Ok(Ok(resp)) if resp.status().is_success() => healthy(
            "openfga",
            "OpenFGA (Authorization)",
            now,
            Some(elapsed),
            None,
        ),
        Ok(Ok(resp)) => unhealthy(
            "openfga",
            "OpenFGA (Authorization)",
            now,
            Some(elapsed),
            format!("unexpected response ({})", resp.status().as_u16()),
        ),
        Ok(Err(_)) => unhealthy(
            "openfga",
            "OpenFGA (Authorization)",
            now,
            Some(elapsed),
            "unreachable",
        ),
        Err(_) => unhealthy(
            "openfga",
            "OpenFGA (Authorization)",
            now,
            Some(elapsed),
            "timed out",
        ),
    }
}

async fn probe_openfga_if_configured(http: &reqwest::Client, url: Option<&str>) -> ServiceHealth {
    match url {
        None => unknown(
            "openfga",
            "OpenFGA (Authorization)",
            OffsetDateTime::now_utc(),
        ),
        Some(base) => probe_openfga(http, base).await,
    }
}

/// `version` is always `None` — Trino's `/v1/info` response shape was
/// never observed live (the `trino` compose profile was not running when
/// this plan was written; see the plan's honesty-check note 3). A future
/// editor should fill `version` in only after checking a real response
/// body, not guess a field name.
async fn probe_trino(http: &reqwest::Client, base: &str) -> ServiceHealth {
    let now = OffsetDateTime::now_utc();
    let started = Instant::now();
    let attempt =
        tokio::time::timeout(PROBE_TIMEOUT, http.get(format!("{base}/v1/info")).send()).await;
    let elapsed = elapsed_millis(started.elapsed());
    match attempt {
        Ok(Ok(resp)) if resp.status().is_success() => healthy(
            "trino",
            "Trino (Federated query engine)",
            now,
            Some(elapsed),
            None,
        ),
        Ok(Ok(resp)) => unhealthy(
            "trino",
            "Trino (Federated query engine)",
            now,
            Some(elapsed),
            format!("unexpected response ({})", resp.status().as_u16()),
        ),
        Ok(Err(_)) => unhealthy(
            "trino",
            "Trino (Federated query engine)",
            now,
            Some(elapsed),
            "unreachable",
        ),
        Err(_) => unhealthy(
            "trino",
            "Trino (Federated query engine)",
            now,
            Some(elapsed),
            "timed out",
        ),
    }
}

async fn probe_trino_if_configured(http: &reqwest::Client, url: Option<&str>) -> ServiceHealth {
    match url {
        None => unknown(
            "trino",
            "Trino (Federated query engine)",
            OffsetDateTime::now_utc(),
        ),
        Some(base) => probe_trino(http, base).await,
    }
}

/// `Lakekeeper` is probed only when `LAKEKEEPER_CATALOG_URI` is set to a
/// non-empty value. Unlike `trino_health_url`/`openfga_url` (both
/// `Option<String>`), `lakekeeper_catalog_uri` is a plain `String` with a
/// compose-shaped default, so before this a deployment that simply does not
/// run a catalog had no way to say so: the probe always fired, always
/// failed, and the console reported a permanent `unhealthy` for a service
/// that was never meant to be there. "Not deployed" and "deployed and
/// broken" are different facts (AGENTS.md principle 2) — an empty value is
/// the opt-out, reported `unknown`/`checked: false` exactly like the other
/// two optional services.
async fn probe_lakekeeper_if_configured(
    http: &reqwest::Client,
    catalog_uri: &str,
) -> ServiceHealth {
    if catalog_uri.trim().is_empty() {
        return unknown(
            "lakekeeper",
            "Iceberg + Lakekeeper (Open tables)",
            OffsetDateTime::now_utc(),
        );
    }
    probe_lakekeeper(http, &lakekeeper_probe_base(catalog_uri)).await
}

/// Probe all six services concurrently, in this fixed, documented order:
/// `clickhouse, dagster, lakekeeper, rustfs, openfga, trino`. "No fixed
/// list" in the grand plan means the ROUTE does not hardcode a subset for
/// the reader to filter — not that the probe set itself is dynamic; these
/// six are exactly `docker-compose.yml`'s core stack plus the two
/// genuinely optional services.
pub async fn probe_all(state: &AppState) -> Vec<ServiceHealth> {
    let http = reqwest::Client::new();
    let dagster_server_info_url = format!(
        "{}/server_info",
        state.config.dagster_url.replace("/graphql", "")
    );
    let (ch, dagster, lakekeeper, rustfs, openfga, trino) = tokio::join!(
        probe_clickhouse(&state.clickhouse),
        probe_dagster(&http, &dagster_server_info_url),
        probe_lakekeeper_if_configured(&http, &state.config.lakekeeper_catalog_uri),
        probe_rustfs(&state.config),
        probe_openfga_if_configured(&http, state.config.openfga_url.as_deref()),
        probe_trino_if_configured(&http, state.config.trino_health_url.as_deref()),
    );
    vec![ch, dagster, lakekeeper, rustfs, openfga, trino]
}

/// 15 s cache for [`probe_all`] (WS5) — see the module doc comment.
/// `None` until the first read.
pub type HealthCache = Arc<tokio::sync::Mutex<Option<(Vec<ServiceHealth>, OffsetDateTime)>>>;

/// How long a cached probe result may be served before a read re-probes.
const CACHE_WINDOW: time::Duration = time::Duration::seconds(15);

/// Core cache logic with an injected clock and probe fn, so the 15 s window
/// is tested without ever sleeping or touching a real clock.
async fn cached_probe_all_with<F, Fut>(
    cache: &HealthCache,
    now: impl Fn() -> OffsetDateTime,
    probe: F,
) -> Vec<ServiceHealth>
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = Vec<ServiceHealth>>,
{
    let mut guard = cache.lock().await;
    let current = now();
    if let Some((cached, checked_at)) = guard.as_ref()
        && current - *checked_at < CACHE_WINDOW
    {
        return cached.clone();
    }
    let fresh = probe().await;
    *guard = Some((fresh.clone(), current));
    fresh
}

/// Production entry point `routes::ops`/`routes::overview` call — never
/// serves a result whose own `checked_at` is more than [`CACHE_WINDOW`]
/// old; a read past the window always re-probes.
pub async fn cached_probe_all(state: &AppState) -> Vec<ServiceHealth> {
    cached_probe_all_with(&state.health_cache, OffsetDateTime::now_utc, || {
        probe_all(state)
    })
    .await
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use serde_json::json;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    #[tokio::test]
    async fn dagster_probe_reports_version_from_server_info() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/server_info"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({ "dagster_version": "1.13.17" })),
            )
            .mount(&server)
            .await;
        let health = probe_dagster(
            &reqwest::Client::new(),
            &format!("{}/server_info", server.uri()),
        )
        .await;
        assert!(health.ok);
        assert_eq!(health.version.as_deref(), Some("1.13.17"));
        assert!(health.error.is_none());
    }

    #[tokio::test]
    async fn dagster_probe_classifies_a_timeout_without_leaking_the_raw_error() {
        // An unroutable address -- the probe must time out inside its own
        // 3s bound, not hang the whole probe_all() call, and must never
        // format the wrapped reqwest::Error (would leak host/port).
        let health = probe_dagster(&reqwest::Client::new(), "http://127.0.0.1:1/server_info").await;
        assert!(!health.ok);
        let err = health.error.as_deref().unwrap();
        assert!(
            !err.contains("127.0.0.1"),
            "leaked the dialed address: {err}"
        );
    }

    #[test]
    fn clickhouse_probe_never_echoes_the_raw_server_error_body() {
        // ChError::Server(String) carries ClickHouse's raw response body
        // verbatim -- a probe failure must not pass that through into
        // ServiceHealth.error.
        let ch_error = ChError::Server(
            "Code: 62. DB::Exception: Syntax error: internal secret detail\n".to_owned(),
        );
        let classified = classify_ch_error(&ch_error);
        assert!(!classified.contains("internal secret detail"));
    }

    #[tokio::test]
    async fn lakekeeper_probe_derives_its_base_from_the_catalog_uri() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/health"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "health": "ok" })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/management/v1/info"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "version": "0.13.1" })))
            .mount(&server)
            .await;
        // `lakekeeper_catalog_uri` includes the `/catalog` REST path suffix
        // -- the health/info endpoints hang off the bare origin, not that
        // suffix, so the base must be derived by stripping any path
        // component, not by string concatenation onto the configured URI
        // as-is.
        let catalog_uri = format!("{}/catalog", server.uri());
        let health = probe_lakekeeper(
            &reqwest::Client::new(),
            &lakekeeper_probe_base(&catalog_uri),
        )
        .await;
        assert!(health.ok);
        assert_eq!(health.version.as_deref(), Some("0.13.1"));
    }

    #[test]
    fn lakekeeper_probe_base_strips_the_path_but_leaves_a_bare_origin_alone() {
        assert_eq!(
            lakekeeper_probe_base("http://localhost:8181/catalog"),
            "http://localhost:8181"
        );
        assert_eq!(
            lakekeeper_probe_base("http://localhost:8181"),
            "http://localhost:8181"
        );
    }

    #[tokio::test]
    async fn unconfigured_trino_and_openfga_report_unknown_without_a_probe() {
        let cfg = crate::config::Config::from_map(&std::collections::HashMap::new()).unwrap();
        let health =
            probe_trino_if_configured(&reqwest::Client::new(), cfg.trino_health_url.as_deref())
                .await;
        assert!(!health.ok);
        assert!(!health.checked);
        assert_eq!(health.health_label(), "unknown");

        let health =
            probe_openfga_if_configured(&reqwest::Client::new(), cfg.openfga_url.as_deref()).await;
        assert!(!health.checked);
        assert_eq!(health.health_label(), "unknown");
    }

    /// An empty `LAKEKEEPER_CATALOG_URI` means "this deployment does not run
    /// a catalog" and must report `unknown` without probing — not the
    /// permanent `unhealthy` a always-on probe produced for every
    /// catalog-less deployment.
    #[tokio::test]
    async fn unconfigured_lakekeeper_reports_unknown_without_a_probe() {
        let health = probe_lakekeeper_if_configured(&reqwest::Client::new(), "   ").await;
        assert!(!health.checked);
        assert!(!health.ok);
        assert_eq!(health.health_label(), "unknown");
    }

    #[tokio::test]
    async fn configured_trino_is_probed_and_checked() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/info"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;
        let health =
            probe_trino_if_configured(&reqwest::Client::new(), Some(server.uri().as_str())).await;
        assert!(health.checked);
        assert!(health.ok);
        assert_eq!(
            health.version, None,
            "Trino's /v1/info shape was never observed live"
        );
    }

    /// WS5 plan review U5 -- an unconfigured `RustFS` credential must never
    /// be dialed or reported as an outage.
    #[tokio::test]
    async fn unconfigured_rustfs_reports_unknown_without_a_probe() {
        let cfg = crate::config::Config::from_map(&std::collections::HashMap::new()).unwrap();
        let health = probe_rustfs(&cfg).await;
        assert!(!health.checked);
        assert_eq!(health.health_label(), "unknown");
    }

    /// The exact-match resolver admits only the refs it was constructed
    /// with -- a probe-supplied ref that is NOT one of the two configured
    /// refs is refused, never silently resolved. Proves `probe_rustfs`
    /// cannot be widened to a general-purpose resolver without this test
    /// catching it.
    #[tokio::test]
    async fn exact_match_secret_resolver_refuses_anything_outside_its_allowlist() {
        let mut map = std::collections::HashMap::new();
        map.insert("RUSTFS_ACCESS_KEY".to_owned(), "irrelevant".to_owned());
        map.insert("DATABASE_URL".to_owned(), "postgres://leak".to_owned());
        let resolver = ExactMatchSecretResolver::new(
            lakehouse_core::secret::EnvSecretResolver::with_map(map),
            ["env:RUSTFS_ACCESS_KEY".to_owned()],
            "rustfs-health-probe",
        );
        assert!(resolver.resolve("env:RUSTFS_ACCESS_KEY").await.is_ok());
        let err = resolver.resolve("env:DATABASE_URL").await.unwrap_err();
        assert!(matches!(err, SecretError::NotAllowed { .. }));
    }

    #[test]
    fn health_label_maps_checked_and_ok_correctly() {
        let now = OffsetDateTime::now_utc();
        assert_eq!(unknown("x", "X", now).health_label(), "unknown");
        assert_eq!(
            unhealthy("x", "X", now, None, "down").health_label(),
            "unhealthy"
        );
        assert_eq!(healthy("x", "X", now, None, None).health_label(), "healthy");
    }

    struct FixedClock(std::sync::atomic::AtomicI64);
    impl FixedClock {
        fn new(unix_seconds: i64) -> Self {
            Self(std::sync::atomic::AtomicI64::new(unix_seconds))
        }
        fn advance(&self, secs: i64) {
            self.0.fetch_add(secs, std::sync::atomic::Ordering::SeqCst);
        }
        fn now(&self) -> OffsetDateTime {
            OffsetDateTime::from_unix_timestamp(self.0.load(std::sync::atomic::Ordering::SeqCst))
                .unwrap_or(OffsetDateTime::UNIX_EPOCH)
        }
    }

    #[tokio::test]
    async fn cached_probe_is_not_reprobed_within_the_window() {
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let cache: HealthCache = Arc::new(tokio::sync::Mutex::new(None));
        let clock = FixedClock::new(1_800_000_000);
        let probe = || {
            calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            async { vec![] }
        };
        let _ = cached_probe_all_with(&cache, || clock.now(), probe).await;
        let _ = cached_probe_all_with(&cache, || clock.now(), probe).await;
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "second call within the 15s window must not re-probe"
        );
    }

    #[tokio::test]
    async fn cached_probe_is_reprobed_once_the_window_elapses() {
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let cache: HealthCache = Arc::new(tokio::sync::Mutex::new(None));
        let clock = FixedClock::new(1_800_000_000);
        let probe = || {
            calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            async { vec![] }
        };
        let _ = cached_probe_all_with(&cache, || clock.now(), probe).await;
        clock.advance(16);
        let _ = cached_probe_all_with(&cache, || clock.now(), probe).await;
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst),
            2,
            "a read past the 15s window must re-probe"
        );
    }
}
