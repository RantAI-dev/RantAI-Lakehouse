//! `GET /api/ops/{kind}` — operational views: `observability`, `workloads`,
//! `services`.
//!
//! Ports `src/app/api/ops/[kind]/route.ts`. An unrecognized `kind` returns
//! HTTP 400 with `{"error": "unknown kind: <kind>"}`, verified against
//! `ops-unknown-kind.json` in the parity corpus.
//!
//! `usage` was cut in WS1: storage-by-tier used an invented
//! bytes-per-row constant and the tenant budget was a literal `100_000`
//! with compute units relabelled as spend. Nothing but the now-removed
//! Usage page called it, so `"usage"` now falls through to
//! [`Kind::Unknown`] like any other unrecognized value.

use axum::extract::{Extension, Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use lakehouse_auth::Principal;
use lakehouse_clickhouse::{ChClient, ChError};
use lakehouse_core::ident::SqlLiteral;
use lakehouse_store::PgPool;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::health;
use crate::json::ApiJson;
use crate::routes::support::{js_error, num_or_zero, str_col};
use crate::state::AppState;
use crate::tenant::{TENANT_ID, TENANT_SITE};
use lakehouse_dagster::DgError;

/// The three recognized `ops/{kind}` values. Ported from the `if (kind ===
/// ...)` chain in `ops/[kind]/route.ts`; anything else is [`Kind::Unknown`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// `ops/observability` — query SLOs.
    Observability,
    /// `ops/workloads` — currently running `ClickHouse` processes.
    Workloads,
    /// `ops/services` — component health checks.
    Services,
    /// Anything else, which the TypeScript rejects with HTTP 400. Also
    /// where `"usage"` lands now that the Usage page is gone (WS1).
    Unknown,
}

impl Kind {
    fn parse(kind: &str) -> Self {
        match kind {
            "observability" => Self::Observability,
            "workloads" => Self::Workloads,
            "services" => Self::Services,
            _ => Self::Unknown,
        }
    }
}

#[derive(Debug, thiserror::Error)]
enum OpsError {
    #[error("{0}")]
    ClickHouse(#[from] ChError),
    #[error("{0}")]
    Dagster(#[from] DgError),
}

/// `GET /api/ops/{kind}`.
pub async fn get(State(state): State<AppState>, Path(kind): Path<String>) -> Response {
    match Kind::parse(&kind) {
        Kind::Unknown => (
            StatusCode::BAD_REQUEST,
            ApiJson(json!({ "error": format!("unknown kind: {kind}") })),
        )
            .into_response(),
        parsed => match run(&state, parsed).await {
            Ok(body) => (StatusCode::OK, ApiJson(body)).into_response(),
            // Every branch shares one outer `catch (e)` returning
            // `{ error: String(e) }` at 503.
            Err(err) => (
                StatusCode::SERVICE_UNAVAILABLE,
                ApiJson(json!({ "error": js_error(err) })),
            )
                .into_response(),
        },
    }
}

async fn run(state: &AppState, kind: Kind) -> Result<Value, OpsError> {
    match kind {
        Kind::Observability => observability(state).await,
        Kind::Workloads => workloads(&state.clickhouse).await,
        Kind::Services => services(state).await,
        Kind::Unknown => unreachable!("Kind::Unknown is handled before `run` is called"),
    }
}

/// Build the `ops/observability` JSON from measured `p95`/`err`, extracted
/// so the unmeasured fields it nulls (and the dropped
/// `streamingLagSeconds` key) can be asserted without a `ClickHouse` call.
///
/// `ingest_lag` (WS5 item B3), `cache_hit_rate` (WS5 item B4),
/// `agent_success_rate` (WS5 item B5), and `policy_decision_p95_ms` (WS7
/// item C2) are real, computed `Option<f64>`s from the caller
/// (`observability`) -- `None` serializes to `null` the same way the
/// still-unmeasured fields below do, never a fabricated `0`.
fn observability_json(
    p95: i64,
    err: f64,
    ingest_lag: Option<f64>,
    cache_hit_rate: Option<f64>,
    agent_success_rate: Option<f64>,
    policy_decision_p95_ms: Option<f64>,
) -> Value {
    json!({
        "queryP95Ms": p95,
        "queryErrorRate": err,
        "ingestLagSeconds": ingest_lag,
        "cacheHitRate": cache_hit_rate,
        // WS7 item C2: a real p95 over `AppState::policy_decision_latencies`
        // once at least one query has run `sql_rewrite::enforce`'s
        // prefetch+rewrite step (`routes::query::run`); `null` — never a
        // fabricated `0` — until then. Nothing measures open incidents
        // yet, so that one stays `null` (P5 review / WS1 honesty pass
        // task 1.7). `streamingLagSeconds` is dropped outright: it has no
        // consumer in the TypeScript contract and nothing measures
        // streaming either.
        "policyDecisionP95Ms": policy_decision_p95_ms,
        "agentSuccessRate": agent_success_rate,
        "activeIncidents": Value::Null,
        "slos": [
            {
                "name": "Query p95 < 2s",
                "target": "2000ms",
                "current": format!("{p95}ms"),
                "ok": p95 < 2000,
            },
            {
                "name": "Query error rate < 1%",
                "target": "1%",
                "current": format!("{:.2}%", err * 100.0),
                "ok": err < 0.01,
            },
        ],
    })
}

async fn observability(state: &AppState) -> Result<Value, OpsError> {
    let ch = &state.clickhouse;
    let pg = state.pg.as_deref();
    let rows = ch
        .rows(
            "SELECT toString(round(quantile(0.95)(query_duration_ms))) p95,
                  toString(round(countIf(exception != '') / greatest(count(),1), 4)) err,
                  toString(count()) n
           FROM system.query_log
           WHERE type='QueryFinish' AND event_time > now() - INTERVAL 24 HOUR",
            None,
        )
        .await?;
    let q = rows.first();
    let p95 = num_or_zero(q, "p95");
    let err = q
        .and_then(|r| str_col(r, "err").parse::<f64>().ok())
        .unwrap_or(0.0);
    let ingest_lag = ingest_lag_seconds(ch).await;
    let cache_hit_rate = cache_hit_rate(ch).await;
    let agent_success_rate = agent_success_rate(pg).await;
    let policy_decision_p95_ms = state.policy_decision_latencies.p95_ms();
    Ok(observability_json(
        p95,
        err,
        ingest_lag,
        cache_hit_rate,
        agent_success_rate,
        policy_decision_p95_ms,
    ))
}

/// `observability.ingestLagSeconds` (WS5 item B3, corrected against WS3's
/// real, approved `bronze_meta.ingest_run` schema against a prior draft
/// that assumed a `finished_at` temporal column this table does not have
/// -- WS5 plan review U3). Per connector, the age of its own latest
/// **successful** run; across connectors, the max of those ages -- the
/// worst currently-stale connector, not the age of the single oldest row
/// ever recorded (which only ever grows and never reflects current
/// staleness). A connector with no successful run in the table
/// contributes nothing to this aggregate: a connector that has never once
/// ingested successfully is a real, separate gap this metric does not
/// encode -- it is visible elsewhere (connector health status), not
/// fabricated into a lag number here as "infinitely stale" (which would
/// hide every other connector's real staleness behind one always-failing
/// connector). If the table doesn't exist yet (WS3 not landed on this
/// branch) or the query otherwise fails, this returns `None` and logs at
/// `warn`, never a 500/503 for the rest of `/api/ops/observability`.
async fn ingest_lag_seconds(ch: &ChClient) -> Option<f64> {
    let rows = ch
        .rows(
            "SELECT connector_id, max(ended_at) AS latest_success \
             FROM lake.`bronze_meta.ingest_run` \
             WHERE status = 'succeeded' \
             GROUP BY connector_id",
            None,
        )
        .await;
    let rows = match rows {
        Ok(rows) => rows,
        Err(err) => {
            tracing::warn!(error = %err, "ingest_lag_seconds: bronze_meta.ingest_run unavailable");
            return None;
        }
    };
    let now = time::OffsetDateTime::now_utc();
    rows.iter()
        .filter_map(|r| {
            let ended_at_str = str_col(r, "latest_success");
            // `ended_at` is a `ClickHouse` `String` (WS3's own schema --
            // not a temporal type), so parse it the same way this file
            // already parses ISO-shaped `String` timestamps elsewhere
            // (`started_at_from_ms_column`'s siblings), never a
            // `dateDiff` push-down that assumes a temporal column type
            // this table does not have.
            time::OffsetDateTime::parse(
                ended_at_str,
                &time::format_description::well_known::Rfc3339,
            )
            .ok()
        })
        .map(|ended_at| (now - ended_at).as_seconds_f64())
        .fold(None, |acc: Option<f64>, age| {
            Some(acc.map_or(age, |a| a.max(age)))
        })
}

/// `observability.cacheHitRate` (WS5 item B4) -- real, verified live on
/// this instance (see the plan's "External signals verified" section).
/// 24h window, matching `q_row`'s window in `overview.rs` for consistency
/// across the two routes that both summarize the last day of
/// `system.query_log`. `None` when no cache-eligible queries ran in the
/// window (`hits + misses == 0`) -- "no data" is not "0% hit rate," which
/// would read as a real, poor measurement rather than "not measured."
async fn cache_hit_rate(ch: &ChClient) -> Option<f64> {
    let rows = ch
        .rows(
            "SELECT toString(sum(ProfileEvents['QueryCacheHits'])) hits, \
                    toString(sum(ProfileEvents['QueryCacheMisses'])) misses \
             FROM system.query_log \
             WHERE type = 'QueryFinish' AND event_time > now() - INTERVAL 24 HOUR",
            None,
        )
        .await
        .ok()?;
    let row = rows.first()?;
    let hits = str_col(row, "hits").parse::<f64>().ok()?;
    let misses = str_col(row, "misses").parse::<f64>().ok()?;
    let total = hits + misses;
    (total > 0.0).then_some(hits / total)
}

/// `overview.agentSuccessRate`/`ops.observability.agentSuccessRate` (WS5
/// item B5) -- the fraction of `agent_run` rows completed in the last 24h
/// that succeeded. `None` both when no Postgres pool is configured and
/// when there are zero completed runs (no denominator) -- "no data" is
/// not "0% success," which would read as a real, poor measurement rather
/// than "not measured." Any query failure also degrades to `None`, never
/// a 503 for the rest of `/api/ops/observability`.
#[allow(
    clippy::cast_precision_loss,
    reason = "agent_run counts in this deployment are nowhere near f64's 52-bit mantissa \
              limit; the precision this could lose is not reachable in practice"
)]
async fn agent_success_rate(pg: Option<&PgPool>) -> Option<f64> {
    let pool = pg?;
    let (succeeded, failed) = lakehouse_store::agents::count_agent_run_outcomes(pool)
        .await
        .inspect_err(|err| tracing::warn!(%err, "count_agent_run_outcomes failed"))
        .ok()?;
    let total = succeeded + failed;
    (total > 0).then_some(succeeded as f64 / total as f64)
}

/// Build one `ops/workloads` row from a `system.processes` fixture row.
/// `started_ms` is computed by `ClickHouse` in the same query as `elapsed`
/// (`now64() - elapsed`), not stamped by this process, so both numbers come
/// from one clock at one instant. A row whose `started_ms` fails to parse
/// (or is absent, e.g. in a test fixture) reports `startedAt: null` rather
/// than falling back to the request's own clock — that fallback was the
/// fabrication this replaces.
/// Shared by [`workload_row`] and [`cancel_workload_body`] (WS5 item A5): a
/// `started_ms` column from `system.processes` (see [`workload_row`]'s doc
/// comment for why this comes from `ClickHouse`'s own clock, not this
/// process's) is parsed into an ISO 8601 string, or `None` if it's
/// absent/unparseable — never a fallback to this process's own clock.
fn started_at_from_ms_column(p: &Map<String, Value>) -> Option<String> {
    str_col(p, "started_ms")
        .parse::<f64>()
        .ok()
        .map(|ms| lakehouse_dagster::iso_from_unix_seconds(ms / 1000.0))
}

fn workload_row(index: usize, p: &Map<String, Value>) -> Value {
    let elapsed_secs = str_col(p, "elapsed").parse::<f64>().unwrap_or(0.0);
    let started_at = started_at_from_ms_column(p);
    json!({
        "id": format!("w-{index}"),
        "principal": str_col(p, "user"),
        "tenant": TENANT_ID.as_str(),
        // No workload classifier exists; null instead of the invented
        // "hot-analytics" literal every row used to carry.
        "class": Value::Null,
        // Every row here comes from `system.processes`, which *is*
        // `ClickHouse` — this label is true regardless of workload shape
        // (mirrors the same deliberate call on `/api/query/run`).
        "engine": "hot-store",
        "status": "running",
        "elapsedMs": elapsed_ms(elapsed_secs),
        // No cost model exists; null instead of the literal `1`.
        "estimatedCost": Value::Null,
        "startedAt": started_at,
    })
}

async fn workloads(ch: &ChClient) -> Result<Value, OpsError> {
    let procs = ch
        .rows(
            "SELECT user, toString(elapsed) elapsed, substring(query,1,80) query,
                  toString(toUnixTimestamp64Milli(now64(3)) - toInt64(elapsed * 1000)) started_ms
         FROM system.processes WHERE query NOT LIKE '%system.processes%' LIMIT 50",
            None,
        )
        .await?;
    let workloads: Vec<Value> = procs
        .iter()
        .enumerate()
        .map(|(i, p)| workload_row(i, p))
        .collect();
    Ok(json!({ "workloads": workloads }))
}

/// Build one `ops/services` row from a real [`health::ServiceHealth`]
/// probe result (WS5 item A3) — replaces the old `service_row`, which took
/// a bare `Option<bool>` and could only ever cover `ClickHouse`/`Dagster`;
/// Lakekeeper/`RustFS`/`OpenFGA`/`Trino` are now genuinely probed too (or,
/// for the two optional services, honestly reported `"unknown"` when
/// unconfigured — see [`health::ServiceHealth::health_label`]).
fn platform_service_row(h: &health::ServiceHealth, deps: &[&str]) -> Value {
    json!({
        "id": h.id,
        "name": h.name,
        "health": h.health_label(),
        "checked": h.checked,
        "version": h.version,
        "site": TENANT_SITE.as_str(),
        "replicas": Value::Null,
        "errorRate": Value::Null,
        "latencyMs": h.latency_ms,
        "checkedAt": h.checked_at,
        "dependencies": deps,
    })
}

/// Declared dependency edges for the console's services graph. A fixed,
/// hardcoded map (not derived from the probe set) — matches this file's
/// pre-WS5 `dependencies` literals for `dagster`/`iceberg`(now
/// `lakekeeper`)/`rustfs`, plus two new edges for the two newly-probed
/// services this task adds.
fn deps_for(id: &str) -> &'static [&'static str] {
    match id {
        "dagster" => &["clickhouse"],
        "lakekeeper" => &["rustfs", "openfga"],
        "trino" => &["lakekeeper", "rustfs"],
        _ => &[],
    }
}

async fn services(state: &AppState) -> Result<Value, OpsError> {
    let probes = health::cached_probe_all(state).await;
    let services: Vec<Value> = probes
        .iter()
        .map(|h| platform_service_row(h, deps_for(h.id)))
        .collect();
    Ok(json!({ "services": services }))
}

/// `Math.round((Number(p.elapsed) || 0) * 1000)`.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "elapsed query time in milliseconds; always non-negative and \
              far below i64::MAX in practice"
)]
fn elapsed_ms(elapsed_secs: f64) -> i64 {
    (elapsed_secs * 1000.0).round() as i64
}

// ── cancelWorkload (Task 2.6) ───────────────────────────────────────────
//
// `GET /api/ops/workloads` (above) mints purely positional ids ("w-0",
// "w-1", ...) from a fresh `system.processes` scan every request — the
// parity corpus (`ops-workloads.json`) locks that exact response shape, so
// the real `query_id` `system.processes` actually has cannot be added to
// it without breaking parity. `cancel_workload` below re-derives it
// server-side instead: it re-runs (a superset of) the same query, walks to
// the same positional index, and only then knows the real `query_id` to
// target — which is inherently racy against a process list that can
// change between the two requests (the list a user is looking at may have
// already scrolled by the time they click "cancel"), same tradeoff any
// index-addressed live list has. That `query_id` is used only to build the
// `KILL QUERY` statement server-side; it is never serialized back to the
// client.

/// Build `KILL QUERY WHERE query_id = '<escaped>'` for `query_id`.
/// Extracted from [`cancel_workload_body`] purely so it can be unit tested
/// (string construction only) without ever touching `ClickHouse` — see the
/// module-level safety note in the Task 2.6 brief: no `KILL QUERY` may run
/// against the live cluster during this work.
fn kill_query_sql(query_id: &str) -> String {
    format!("KILL QUERY WHERE query_id = {}", SqlLiteral::from(query_id))
}

/// `POST /api/ops/workloads/{id}/cancel` — kill the `ClickHouse` query
/// backing workload `id` (a `"w-<index>"` id from `GET /api/ops/workloads`).
///
/// # Errors
///
/// This is a `Response`-returning handler (not `ApiResult`), matching the
/// rest of this file: 404 if `id` doesn't parse as `"w-<n>"` or `n` is out
/// of range against the current process list (including the ordinary case
/// where the workload already finished); 503 on a `ClickHouse` failure.
pub async fn cancel_workload(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    match cancel_workload_body(&state.clickhouse, &id).await {
        Ok(Some(body)) => (StatusCode::OK, ApiJson(body)).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            ApiJson(json!({ "error": format!("Workload {id} not found") })),
        )
            .into_response(),
        Err(err) => (
            StatusCode::SERVICE_UNAVAILABLE,
            ApiJson(json!({ "error": js_error(err) })),
        )
            .into_response(),
    }
}

async fn cancel_workload_body(ch: &ChClient, id: &str) -> Result<Option<Value>, ChError> {
    let Some(index) = id.strip_prefix("w-").and_then(|s| s.parse::<usize>().ok()) else {
        return Ok(None);
    };
    let procs = ch
        .rows(
            "SELECT query_id, user, toString(elapsed) elapsed, substring(query,1,80) query,
                  toString(toUnixTimestamp64Milli(now64(3)) - toInt64(elapsed * 1000)) started_ms
             FROM system.processes WHERE query NOT LIKE '%system.processes%' LIMIT 50",
            None,
        )
        .await?;
    let Some(row) = procs.get(index) else {
        return Ok(None);
    };
    let query_id = str_col(row, "query_id");
    if query_id.is_empty() {
        return Ok(None);
    }
    ch.exec(&kill_query_sql(query_id), None).await?;
    let elapsed_secs = str_col(row, "elapsed").parse::<f64>().unwrap_or(0.0);
    Ok(Some(json!({
        "id": id,
        "principal": str_col(row, "user"),
        "tenant": TENANT_ID.as_str(),
        "class": "hot-analytics",
        "engine": "hot-store",
        "status": "cancelled",
        "elapsedMs": elapsed_ms(elapsed_secs),
        "estimatedCost": 1,
        // J14 / WS5 plan review Y5: derived from the same
        // `started_ms` column (ClickHouse's own clock, computed in the
        // same query as `elapsed`) `workload_row` already uses — never
        // this process's own clock (`now_iso()`, the fabrication this
        // replaces). `None` (never a fallback to "now") when the row
        // exists but `started_ms` didn't parse.
        "startedAt": started_at_from_ms_column(row),
    })))
}

// ── GET /api/ops/logs (WS5 item G1, grand plan §13) ─────────────────────
//
// Deviation from the original draft (WS5 plan review U4, blocker,
// security): this route no longer serves Dagster run logs. WS4's own plan
// (`docs/superpowers/plans/2026-09-11-ws4-pipelines-detail-source-logs.md`,
// its own run-logs route item) reserves `GET
// /api/pipelines/{id}/runs/{runId}/logs` for that -- gated `pipeline:read`,
// not merely `RequiresAuth` -- since op-authored
// log text can carry a connection string or other operational secret
// verbatim, and every authenticated user should not be able to read every
// pipeline's run logs. That route is NOT YET implemented on this branch
// (confirmed: no `run_logs`/`logsForRun` call exists in
// `lakehouse-dagster` or `routes/pipelines.rs` as of this commit) -- this
// route defers to it by name/path regardless, rather than re-implementing
// a second, less-scoped copy of the same data under a wider gate.

/// The `system.text_log` message field is truncated to this many
/// characters by the query itself (a `ClickHouse`-side `substring`, not a
/// Rust-side re-truncation) -- even an `"ops:logs"`-scoped read must not
/// let one log line pull an unbounded blob into the response.
const MAX_LOG_MESSAGE_CHARS: usize = 2000;

/// The default `tail` when the caller omits `?tail=`.
const DEFAULT_LOG_TAIL: u32 = 200;

/// The hard ceiling `tail` is clamped to regardless of what the caller
/// asks for -- a caller must not be able to request an unbounded read.
const MAX_LOG_TAIL: u32 = 1000;

/// Query parameters for `GET /api/ops/logs`.
#[derive(Debug, Deserialize, Default)]
pub struct LogsQuery {
    service: Option<String>,
    tail: Option<u32>,
}

/// Clamp a caller-supplied `tail` to `[1, MAX_LOG_TAIL]`, defaulting to
/// `DEFAULT_LOG_TAIL` when absent. A caller cannot request more than
/// `MAX_LOG_TAIL` lines no matter what it passes.
fn bound_tail(tail: Option<u32>) -> u32 {
    tail.unwrap_or(DEFAULT_LOG_TAIL).clamp(1, MAX_LOG_TAIL)
}

/// The fixed allowlist `service` is matched against -- nothing
/// caller-supplied is ever interpolated into a command, file path, or
/// container name (AGENTS.md fail-closed principle). An unlisted or
/// unrecognized value is [`Self::Unsupported`], never an attempt to guess
/// how to read logs for a service this route does not know.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LogService {
    /// `system.text_log` -- real, gated on `"ops:logs"` (Step below).
    ClickHouse,
    /// Dagster run logs live on WS4's own, more narrowly scoped route --
    /// see the module note above. Reported honestly rather than served
    /// here a second time.
    Dagster,
    /// Lakekeeper, `RustFS`, `OpenFGA`, `Trino`, and any unrecognized
    /// value: no log-reading endpoint exists for these on the compose
    /// stack (confirmed against `docker-compose.yml`'s service list) --
    /// `supported: false` at 200, never `Docker` socket access (out of
    /// scope per grand plan §13).
    Unsupported,
}

impl LogService {
    fn parse(service: &str) -> Self {
        match service {
            "clickhouse" => Self::ClickHouse,
            "dagster" => Self::Dagster,
            _ => Self::Unsupported,
        }
    }
}

/// Whether `principal` may read `service=clickhouse` logs. Only
/// `"ops:logs"` -- an unseeded token only Platform Admin's `"*:*"` grant
/// satisfies (`lakehouse-auth/src/permissions.rs`) -- passes here, not
/// merely an authenticated principal: `system.text_log` carries every
/// query's SQL text verbatim, which can include a table function's
/// literal credentials (e.g. `s3(url, key, secret)`), so this is not an
/// Analyst-level read even though `/api/ops/logs` itself is only
/// `Policy::RequiresAuth` at the router level (`POLICY_TABLE`). This is
/// the same "router-level auth-only, handler-level stricter guard"
/// pattern `routes/alerts.rs::check_run_token` already establishes for
/// `/api/alerts/run`.
fn can_read_clickhouse_logs(principal: Option<&Principal>) -> bool {
    principal.is_some_and(|p| p.has("ops:logs"))
}

/// Reads `system.text_log`, capped at `tail` rows and `MAX_LOG_MESSAGE_CHARS`
/// per message (both server-side, via the query itself).
///
/// # Errors
///
/// Returns [`ChError`] on any `ClickHouse` failure; the caller ([`logs`])
/// classifies it into a fixed `"database error"` message before it
/// reaches a response -- upstream error text never reaches the caller
/// (AGENTS.md principle 4).
async fn clickhouse_logs(ch: &ChClient, tail: u32) -> Result<Vec<Value>, ChError> {
    let rows = ch
        .rows(
            &format!(
                "SELECT toString(event_time) event_time, level, \
                 substring(message, 1, {MAX_LOG_MESSAGE_CHARS}) message \
                 FROM system.text_log ORDER BY event_time DESC LIMIT {tail}"
            ),
            None,
        )
        .await?;
    Ok(rows
        .iter()
        .map(|r| {
            json!({
                "at": str_col(r, "event_time"),
                "level": str_col(r, "level"),
                "message": str_col(r, "message"),
            })
        })
        .collect())
}

/// `GET /api/ops/logs?service=&tail=` (WS5 item G1) -- an allowlisted,
/// bounded-tail read of one platform component's own logs.
///
/// `service` is matched against [`LogService::parse`]'s fixed allowlist;
/// an unlisted or unrecognized value degrades to the honest
/// `{"supported": false, "reason": "..."}` shape at 200, never an error
/// that reads as "this might work with a different spelling." `tail`
/// defaults to `DEFAULT_LOG_TAIL` and is clamped to `MAX_LOG_TAIL`
/// regardless of what the caller asks for.
///
/// # Errors
///
/// This is a `Response`-returning handler (matching the rest of this
/// file): 403 when `service=clickhouse` and the caller lacks
/// `"ops:logs"`; 503 with a fixed `"database error"` message (never
/// upstream `ClickHouse` text) on a query failure.
pub async fn logs(
    State(state): State<AppState>,
    Query(query): Query<LogsQuery>,
    principal: Option<Extension<Principal>>,
) -> Response {
    let tail = bound_tail(query.tail);
    match LogService::parse(query.service.as_deref().unwrap_or("")) {
        LogService::ClickHouse => {
            if !can_read_clickhouse_logs(principal.as_ref().map(|Extension(p)| p)) {
                return (
                    StatusCode::FORBIDDEN,
                    ApiJson(json!({ "error": "forbidden" })),
                )
                    .into_response();
            }
            match clickhouse_logs(&state.clickhouse, tail).await {
                Ok(lines) => (
                    StatusCode::OK,
                    ApiJson(
                        json!({ "supported": true, "service": "clickhouse", "lines": lines }),
                    ),
                )
                    .into_response(),
                Err(err) => {
                    // Upstream ClickHouse error text never reaches the
                    // caller -- classify, don't forward (AGENTS.md
                    // principle 4).
                    tracing::warn!(error = %err, "clickhouse_logs failed");
                    (
                        StatusCode::SERVICE_UNAVAILABLE,
                        ApiJson(json!({ "error": "database error" })),
                    )
                        .into_response()
                }
            }
        }
        LogService::Dagster => (
            StatusCode::OK,
            ApiJson(json!({
                "supported": false,
                "reason": "the scoped run-logs route (GET /api/pipelines/{id}/runs/{runId}/logs, gated pipeline:read) is planned but not yet implemented on this branch"
            })),
        )
            .into_response(),
        LogService::Unsupported => (
            StatusCode::OK,
            ApiJson(json!({
                "supported": false,
                "reason": "no log-reading endpoint exists for this service"
            })),
        )
            .into_response(),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn kind_parses_known_values() {
        assert_eq!(Kind::parse("observability"), Kind::Observability);
        assert_eq!(Kind::parse("workloads"), Kind::Workloads);
        assert_eq!(Kind::parse("services"), Kind::Services);
    }

    #[test]
    fn kind_parse_unknown_falls_back() {
        assert_eq!(Kind::parse("bogus-kind"), Kind::Unknown);
        assert_eq!(Kind::parse(""), Kind::Unknown);
        assert_eq!(Kind::parse("lineage"), Kind::Unknown);
    }

    #[test]
    fn kind_parse_usage_is_unknown_now_that_the_usage_page_is_cut() {
        // WS1 honesty pass: the Usage page and its API kind had no measured
        // backing (invented bytes-per-row constant, a literal budget), and
        // nothing else called `/api/ops/usage`. This is a deliberate
        // contract change, not a weakened test.
        assert_eq!(Kind::parse("usage"), Kind::Unknown);
    }

    #[test]
    fn unprobed_services_report_unknown_health_and_are_marked_unchecked() {
        let trino = health::ServiceHealth {
            id: "trino",
            name: "Trino",
            ok: false,
            checked: false,
            latency_ms: None,
            version: None,
            checked_at: "2026-08-27T04:00:10.075Z".to_owned(),
            error: None,
        };
        let row = platform_service_row(&trino, &["rustfs"]);
        assert_eq!(row["health"], "unknown");
        assert_eq!(row["checked"], false);
        for field in ["version", "replicas", "errorRate", "latencyMs"] {
            assert!(row[field].is_null(), "{field} should be null");
        }
    }

    #[test]
    fn probed_services_keep_their_real_health() {
        let down = health::ServiceHealth {
            id: "clickhouse",
            name: "ClickHouse",
            ok: false,
            checked: true,
            latency_ms: Some(12),
            version: None,
            checked_at: "2026-08-27T04:00:10.075Z".to_owned(),
            error: Some("unreachable".to_owned()),
        };
        let row = platform_service_row(&down, &[]);
        assert_eq!(row["health"], "unhealthy");
        assert_eq!(row["checked"], true);
        // Even a probed service reports null for the metrics nothing
        // measures — probing only tells us up/down, not version.
        assert!(row["version"].is_null());
        assert_eq!(row["latencyMs"], 12);
        assert_eq!(row["checkedAt"], "2026-08-27T04:00:10.075Z");

        let up = health::ServiceHealth {
            id: "dagster",
            name: "Dagster",
            ok: true,
            checked: true,
            latency_ms: Some(5),
            version: Some("1.13.17".to_owned()),
            checked_at: "2026-08-27T04:00:10.075Z".to_owned(),
            error: None,
        };
        let row = platform_service_row(&up, &["clickhouse"]);
        assert_eq!(row["health"], "healthy");
        assert_eq!(row["checked"], true);
        assert_eq!(row["version"], "1.13.17");
    }

    /// WS5 item A3 — `services(&state)` returns all six probes, including
    /// `"lakekeeper"`/`"openfga"`/`"trino"`, and an unconfigured
    /// Trino/OpenFGA report `health: "unknown"`, `checked: false` even
    /// though nothing was network-reachable in this test process.
    #[tokio::test]
    async fn services_returns_all_six_probes_with_unconfigured_ones_unknown() {
        let cfg = crate::config::Config::from_map(&std::collections::HashMap::new()).unwrap();
        let state = AppState::new(cfg);
        let body = services(&state).await.unwrap();
        let rows = body["services"].as_array().unwrap();
        let ids: Vec<&str> = rows.iter().map(|r| r["id"].as_str().unwrap()).collect();
        assert_eq!(
            ids,
            vec![
                "clickhouse",
                "dagster",
                "lakekeeper",
                "rustfs",
                "openfga",
                "trino"
            ]
        );
        for id in ["trino", "openfga"] {
            let row = rows.iter().find(|r| r["id"] == id).unwrap();
            assert_eq!(
                row["health"], "unknown",
                "{id} must be unknown, not unhealthy"
            );
            assert_eq!(row["checked"], false);
        }
    }

    #[test]
    fn observability_nulls_every_unmeasured_metric_and_drops_streaming_lag() {
        let v = observability_json(120, 0.002, None, None, None, None);
        assert_eq!(v["queryP95Ms"], 120);
        assert!((v["queryErrorRate"].as_f64().unwrap() - 0.002).abs() < f64::EPSILON);
        for field in [
            "ingestLagSeconds",
            "cacheHitRate",
            "policyDecisionP95Ms",
            "agentSuccessRate",
            "activeIncidents",
        ] {
            assert!(v[field].is_null(), "{field} should be null");
        }
        assert!(
            v.get("streamingLagSeconds").is_none(),
            "streamingLagSeconds must be removed, not just nulled"
        );
        assert!(v["slos"].is_array());
    }

    /// WS5 item B3 -- a real, computed ingest lag passes through as a real
    /// number, never left null once it is actually measured.
    #[test]
    fn observability_reports_a_real_ingest_lag_when_measured() {
        let v = observability_json(120, 0.002, Some(42.5), None, None, None);
        assert_eq!(v["ingestLagSeconds"], 42.5);
    }

    /// WS5 item B4 -- likewise for a real, computed cache hit rate.
    #[test]
    fn observability_reports_a_real_cache_hit_rate_when_measured() {
        let v = observability_json(120, 0.002, None, Some(0.8), None, None);
        assert_eq!(v["cacheHitRate"], 0.8);
    }

    /// WS5 item B5 -- likewise for a real, computed agent success rate.
    #[test]
    fn observability_reports_a_real_agent_success_rate_when_measured() {
        let v = observability_json(120, 0.002, None, None, Some(0.75), None);
        assert_eq!(v["agentSuccessRate"], 0.75);
    }

    /// WS7 item C2 -- likewise for a real, measured `policyDecisionP95Ms`,
    /// closing the `null` WS1's honesty pass left in place.
    #[test]
    fn observability_reports_a_real_policy_decision_p95_when_measured() {
        let v = observability_json(120, 0.002, None, None, None, Some(3.5));
        assert_eq!(v["policyDecisionP95Ms"], 3.5);
    }

    #[test]
    fn workload_started_at_is_derived_from_elapsed_not_the_request_time() {
        let mut row = Map::new();
        row.insert("user".to_owned(), Value::String("alice".to_owned()));
        row.insert("elapsed".to_owned(), Value::String("1.5".to_owned()));
        // `started_ms` is what `now64() - elapsed*1000` would produce on
        // the server; the epoch millisecond for 2026-08-27T04:00:10.075Z.
        row.insert(
            "started_ms".to_owned(),
            Value::String("1787803210075".to_owned()),
        );
        let v = workload_row(0, &row);
        assert_eq!(v["startedAt"], "2026-08-27T04:00:10.075Z");
        assert_eq!(v["engine"], "hot-store");
        assert!(v["class"].is_null());
        assert!(v["estimatedCost"].is_null());
    }

    #[test]
    fn workload_started_at_is_null_when_started_ms_does_not_parse() {
        let mut row = Map::new();
        row.insert("user".to_owned(), Value::String("bob".to_owned()));
        row.insert("elapsed".to_owned(), Value::String("0.2".to_owned()));
        row.insert("started_ms".to_owned(), Value::String(String::new()));
        let v = workload_row(1, &row);
        assert!(
            v["startedAt"].is_null(),
            "an unparseable started_ms must never fall back to the request's own clock"
        );
    }

    /// WS5 item A5 (J14 / plan review Y5) — `cancel_workload_body`'s
    /// `startedAt` is now derived from the same `started_ms` column
    /// `workload_row` already uses, via the shared
    /// `started_at_from_ms_column` helper. This test exercises the helper
    /// directly on a fixture row (never a live/mocked `ClickHouse` call —
    /// see the CRITICAL SAFETY note below).
    #[test]
    fn started_at_from_ms_column_is_derived_from_the_shared_clock_column() {
        let mut row = Map::new();
        row.insert(
            "started_ms".to_owned(),
            Value::String("1787803210075".to_owned()),
        );
        assert_eq!(
            started_at_from_ms_column(&row).as_deref(),
            Some("2026-08-27T04:00:10.075Z")
        );
    }

    #[test]
    fn started_at_from_ms_column_is_none_when_absent_or_unparseable() {
        // Never a fallback to the request's own clock — this is the exact
        // fabrication J14 found in `cancel_workload_body`'s old
        // `"startedAt": now_iso()`.
        let empty = Map::new();
        assert_eq!(started_at_from_ms_column(&empty), None);

        let mut bad = Map::new();
        bad.insert("started_ms".to_owned(), Value::String(String::new()));
        assert_eq!(started_at_from_ms_column(&bad), None);
    }

    // ── cancelWorkload (Task 2.6) ────────────────────────────────────────
    //
    // Pure string-construction/parsing checks only, per the CRITICAL
    // SAFETY constraint: nothing here talks to a real (or mocked)
    // ClickHouse, so a `KILL QUERY` is never issued anywhere in this test
    // suite. Live verification is limited to the id-out-of-range 404
    // path, which returns before `ch.exec` is ever reached — see
    // `cancel_workload_body`.

    #[test]
    fn kill_query_sql_targets_the_exact_query_id() {
        assert_eq!(
            kill_query_sql("abcd-1234"),
            "KILL QUERY WHERE query_id = 'abcd-1234'"
        );
    }

    #[test]
    fn kill_query_sql_escapes_single_quotes() {
        // query_id is server-derived (ClickHouse's own UUID-shaped id), so
        // this is defense in depth rather than a realistic input, but the
        // same escaping discipline as `lineage_body`'s `SqlLiteral` usage
        // applies here.
        assert_eq!(
            kill_query_sql("o'brien"),
            "KILL QUERY WHERE query_id = 'o''brien'"
        );
    }

    #[test]
    fn workload_id_without_w_prefix_does_not_parse() {
        assert!("not-w-0".strip_prefix("w-").is_none());
    }

    #[test]
    fn workload_id_index_parses_from_w_prefix() {
        let id = "w-3";
        let index = id.strip_prefix("w-").and_then(|s| s.parse::<usize>().ok());
        assert_eq!(index, Some(3));
    }

    #[test]
    fn workload_id_non_numeric_suffix_does_not_parse() {
        let id = "w-abc";
        let index = id.strip_prefix("w-").and_then(|s| s.parse::<usize>().ok());
        assert_eq!(index, None);
    }

    // ── observability.ingestLagSeconds (WS5 item B3) ────────────────────

    fn ch_client(url: &str) -> ChClient {
        ChClient::new(url.to_owned(), "default".to_owned(), String::new())
    }

    /// The worst currently-stale connector: per connector, the age of its
    /// own latest *successful* run -- across connectors, the max of those
    /// ages. Never the age of the single oldest row ever recorded (which
    /// only ever grows and would misreport every connector as equally
    /// stale forever).
    #[tokio::test]
    async fn ingest_lag_seconds_is_the_max_per_connector_latest_success_age() {
        let server = wiremock::MockServer::start().await;
        // connector-a's latest success is 10 minutes old; connector-b's is
        // 2 hours old -- connector-b is the worse (staler) of the two, so
        // the aggregate must be ~2h, not connector-a's 10 minutes and not
        // some other, older row from either connector's history.
        let now = time::OffsetDateTime::now_utc();
        let a_latest = now - time::Duration::minutes(10);
        let b_latest = now - time::Duration::hours(2);
        let body = json!({
            "meta": [],
            "data": [
                { "connector_id": "connector-a", "latest_success": a_latest
                    .format(&time::format_description::well_known::Rfc3339).unwrap() },
                { "connector_id": "connector-b", "latest_success": b_latest
                    .format(&time::format_description::well_known::Rfc3339).unwrap() },
            ],
            "rows": 2,
        });
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(body))
            .mount(&server)
            .await;

        let lag = ingest_lag_seconds(&ch_client(&server.uri())).await;
        let lag = lag.expect("a successful row must produce a lag");
        // ~2 hours = 7200s, with generous slack for test wall-clock drift.
        assert!(
            (7100.0..7300.0).contains(&lag),
            "expected ~2h staleness from connector-b, got {lag}s"
        );
    }

    /// A connector whose only rows are non-`succeeded` contributes nothing
    /// -- it must not be treated as "infinitely stale" (which would hide
    /// every other connector's real staleness) and must not crash the
    /// query (there is simply no `latest_success` row for it, since the
    /// query already filters `WHERE status = 'succeeded'`).
    #[tokio::test]
    async fn ingest_lag_seconds_ignores_a_connector_with_no_successful_run() {
        let server = wiremock::MockServer::start().await;
        let now = time::OffsetDateTime::now_utc();
        let only_success = now - time::Duration::minutes(5);
        // The query's own `WHERE status = 'succeeded'` means an
        // always-failing connector never appears in this result set at
        // all -- simulated here by simply not including one.
        let body = json!({
            "meta": [],
            "data": [
                { "connector_id": "connector-ok", "latest_success": only_success
                    .format(&time::format_description::well_known::Rfc3339).unwrap() },
            ],
            "rows": 1,
        });
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(body))
            .mount(&server)
            .await;

        let lag = ingest_lag_seconds(&ch_client(&server.uri())).await;
        let lag = lag.expect("the one successful connector must still produce a lag");
        assert!(
            (280.0..320.0).contains(&lag),
            "expected ~5m staleness, got {lag}s"
        );
    }

    /// A simulated "unknown table" failure (WS3's `bronze_meta.ingest_run`
    /// not landed on this branch yet) is `None`, never propagated as a
    /// 500/503 for the rest of `/api/ops/observability`.
    #[tokio::test]
    async fn ingest_lag_seconds_is_none_when_the_table_is_unavailable() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .respond_with(wiremock::ResponseTemplate::new(404).set_body_string(
                "Code: 60. DB::Exception: Table lake.bronze_meta.ingest_run doesn't exist",
            ))
            .mount(&server)
            .await;

        assert!(
            ingest_lag_seconds(&ch_client(&server.uri()))
                .await
                .is_none()
        );
    }

    // ── observability.cacheHitRate (WS5 item B4) ────────────────────────

    #[tokio::test]
    async fn cache_hit_rate_divides_hits_by_hits_plus_misses() {
        let server = wiremock::MockServer::start().await;
        let body = json!({
            "meta": [],
            "data": [{ "hits": "8", "misses": "2" }],
            "rows": 1,
        });
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(body))
            .mount(&server)
            .await;

        let rate = cache_hit_rate(&ch_client(&server.uri())).await;
        assert_eq!(rate, Some(0.8));
    }

    /// No cache-eligible queries ran in the window: both counters are
    /// zero, so this is "no data," not "0% hit rate" -- `None`, never `0`.
    #[tokio::test]
    async fn cache_hit_rate_is_none_when_there_is_no_denominator() {
        let server = wiremock::MockServer::start().await;
        let body = json!({
            "meta": [],
            "data": [{ "hits": "0", "misses": "0" }],
            "rows": 1,
        });
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(body))
            .mount(&server)
            .await;

        assert_eq!(cache_hit_rate(&ch_client(&server.uri())).await, None);
    }

    // ── observability.agentSuccessRate (WS5 item B5) ────────────────────
    //
    // The store-level `count_agent_run_outcomes` fixtures (running-runs
    // excluded, zero-denominator -> (0, 0)) live in
    // `lakehouse-store/tests/agents.rs` against a real Postgres per this
    // plan's own testing rule ("store tests
    // `#[sqlx::test(migrations = "../../migrations")]`"). This route-level
    // test only needs the "no pool configured" path, which never reaches
    // a database at all.

    #[tokio::test]
    async fn agent_success_rate_is_none_without_a_configured_pool() {
        assert_eq!(agent_success_rate(None).await, None);
    }

    // ── GET /api/ops/logs (WS5 item G1) ─────────────────────────────────

    use lakehouse_auth::{PermissionSet, PrincipalId};
    use uuid::Uuid;

    fn principal_with(permissions: &str) -> Principal {
        Principal {
            id: PrincipalId::User(Uuid::nil()),
            tenant_ids: Vec::new(),
            display_name: "test".to_owned(),
            permissions: PermissionSet::parse(permissions),
            provider: "session".to_owned(),
            must_change_password: false,
            role_names: Vec::new(),
        }
    }

    #[test]
    fn log_service_parse_recognizes_clickhouse_and_dagster_specially() {
        assert_eq!(LogService::parse("clickhouse"), LogService::ClickHouse);
        assert_eq!(LogService::parse("dagster"), LogService::Dagster);
    }

    #[test]
    fn log_service_parse_maps_every_other_known_and_unknown_value_to_unsupported() {
        for id in ["lakekeeper", "rustfs", "openfga", "trino", "bogus", ""] {
            assert_eq!(LogService::parse(id), LogService::Unsupported, "{id}");
        }
    }

    #[test]
    fn bound_tail_defaults_when_absent() {
        assert_eq!(bound_tail(None), DEFAULT_LOG_TAIL);
    }

    #[test]
    fn bound_tail_clamps_a_caller_supplied_value_above_the_ceiling() {
        // A caller must not be able to request an unbounded read.
        assert_eq!(bound_tail(Some(u32::MAX)), MAX_LOG_TAIL);
        assert_eq!(bound_tail(Some(50)), 50);
    }

    /// A principal holding every seeded non-`*:*` permission (Analyst,
    /// Data Engineer, Governance Admin's non-wildcard grants, ...) but not
    /// `*:*` itself must NOT be able to read `system.text_log` -- it
    /// carries every query's SQL text verbatim, which can include a table
    /// function's literal credentials.
    #[test]
    fn clickhouse_logs_permission_denies_every_non_wildcard_seeded_permission() {
        let principal = principal_with(
            "query:read, catalog:read, lineage:read, agent:approve, policy:review, \
             policy:*, residency:*, audit:read, pipeline:*, catalog:write, \
             connector:manage, feature:write, notebook:run, dashboard:read, \
             alert:write, workload:cancel, storage:restore, governance:write",
        );
        assert!(!can_read_clickhouse_logs(Some(&principal)));
        assert!(!can_read_clickhouse_logs(None));
    }

    #[test]
    fn clickhouse_logs_permission_allows_only_the_platform_admin_wildcard() {
        let admin = principal_with("*:*");
        assert!(can_read_clickhouse_logs(Some(&admin)));
    }

    #[tokio::test]
    async fn clickhouse_logs_truncates_messages_via_a_clickhouse_side_substring_and_bounds_limit() {
        let server = wiremock::MockServer::start().await;
        let body = json!({
            "meta": [],
            "data": [{ "event_time": "2026-09-17 00:00:00", "level": "Error", "message": "boom" }],
            "rows": 1,
        });
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::body_string_contains(format!(
                "substring(message, 1, {MAX_LOG_MESSAGE_CHARS})"
            )))
            .and(wiremock::matchers::body_string_contains("LIMIT 1000"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(body))
            .mount(&server)
            .await;

        // The query itself must clamp the LIMIT even when the caller
        // passes an out-of-range tail -- the truncation is server-side,
        // never a Rust-side re-truncation applied after an unbounded
        // fetch.
        let lines = clickhouse_logs(&ch_client(&server.uri()), MAX_LOG_TAIL)
            .await
            .expect("mocked ClickHouse call must succeed");
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0]["message"], "boom");
    }

    /// WS5 item G1 (P5 review fix) -- `service=dagster` must tell the
    /// caller the scoped run-logs route is planned, never point them at
    /// `GET /api/pipelines/{id}/runs/{runId}/logs` as if it already
    /// existed: that route has no `run_logs` handler and no
    /// `POLICY_TABLE` entry on this branch (confirmed against
    /// `routes/pipelines.rs` and `policy.rs`), so the earlier wording
    /// sent a caller straight at a 404.
    #[tokio::test]
    async fn logs_dagster_reports_the_scoped_route_as_planned_not_available() {
        let cfg = crate::config::Config::from_map(&std::collections::HashMap::new()).unwrap();
        let state = AppState::new(cfg);
        let response = logs(
            State(state),
            Query(LogsQuery {
                service: Some("dagster".to_owned()),
                tail: None,
            }),
            None,
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["supported"], false);
        assert_eq!(
            body["reason"],
            "the scoped run-logs route (GET /api/pipelines/{id}/runs/{runId}/logs, gated \
             pipeline:read) is planned but not yet implemented on this branch"
        );
    }
}
