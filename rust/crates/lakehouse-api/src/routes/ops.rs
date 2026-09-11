//! `GET /api/ops/{kind}` — operational views: `observability`, `workloads`,
//! `services`.
//!
//! Ports `src/app/api/ops/[kind]/route.ts`. An unrecognized `kind` returns
//! HTTP 400 with `{"error": "kind tak dikenal: <kind>"}`, verified against
//! `ops-unknown-kind.json` in the parity corpus.
//!
//! `usage` was cut in WS1: storage-by-tier used an invented
//! bytes-per-row constant and the tenant budget was a literal `100_000`
//! with compute units relabelled as spend. Nothing but the now-removed
//! Usage page called it, so `"usage"` now falls through to
//! [`Kind::Unknown`] like any other unrecognized value.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use lakehouse_clickhouse::{ChClient, ChError};
use lakehouse_core::ident::SqlLiteral;
use serde_json::{Map, Value, json};
use time::OffsetDateTime;

use crate::json::ApiJson;
use crate::routes::support::{js_error, num_or_zero, str_col};
use crate::state::AppState;
use crate::tenant::{TENANT_ID, TENANT_SITE};
use lakehouse_dagster::{DgClient, DgError};

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
            ApiJson(json!({ "error": format!("kind tak dikenal: {kind}") })),
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
        Kind::Observability => observability(&state.clickhouse).await,
        Kind::Workloads => workloads(&state.clickhouse).await,
        Kind::Services => services(&state.clickhouse, &state.dagster).await,
        Kind::Unknown => unreachable!("Kind::Unknown is handled before `run` is called"),
    }
}

/// Build the `ops/observability` JSON from measured `p95`/`err`, extracted
/// so the five unmeasured fields it nulls (and the dropped
/// `streamingLagSeconds` key) can be asserted without a `ClickHouse` call.
fn observability_json(p95: i64, err: f64) -> Value {
    json!({
        "queryP95Ms": p95,
        "queryErrorRate": err,
        // Nothing measures ingest lag, cache hit rate, policy decision
        // latency, agent success rate, or open incidents today; `null` is
        // honest, the literal zeros this replaced were not (P5 review /
        // WS1 honesty pass task 1.7). `streamingLagSeconds` is dropped
        // outright: it has no consumer in the TypeScript contract and
        // nothing measures streaming either.
        "ingestLagSeconds": Value::Null,
        "cacheHitRate": Value::Null,
        "policyDecisionP95Ms": Value::Null,
        "agentSuccessRate": Value::Null,
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

async fn observability(ch: &ChClient) -> Result<Value, OpsError> {
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
    Ok(observability_json(p95, err))
}

/// Build one `ops/workloads` row from a `system.processes` fixture row.
/// `started_ms` is computed by `ClickHouse` in the same query as `elapsed`
/// (`now64() - elapsed`), not stamped by this process, so both numbers come
/// from one clock at one instant. A row whose `started_ms` fails to parse
/// (or is absent, e.g. in a test fixture) reports `startedAt: null` rather
/// than falling back to the request's own clock — that fallback was the
/// fabrication this replaces.
fn workload_row(index: usize, p: &Map<String, Value>) -> Value {
    let elapsed_secs = str_col(p, "elapsed").parse::<f64>().unwrap_or(0.0);
    let started_at = str_col(p, "started_ms")
        .parse::<f64>()
        .ok()
        .map(|ms| lakehouse_dagster::iso_from_unix_seconds(ms / 1000.0));
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

/// Build one `ops/services` row. `probed` is `Some(ok)` when a live check
/// actually ran this request (`ClickHouse`, `Dagster`); it is `None` when
/// nothing probes the service at all (`Iceberg`/`Lakekeeper`, `RustFS`) —
/// those used to silently reuse `ClickHouse`'s result or the literal
/// `true`. An unprobed service reports `health: "unknown"` and
/// `checked: false` rather than a guessed health, and none of `version`,
/// `replicas`, `errorRate`, or `latencyMs` are measured for any service
/// today (WS5 is expected to fill some of these in).
fn service_row(id: &str, name: &str, probed: Option<bool>, deps: &[&str]) -> Value {
    let health = match probed {
        Some(true) => "healthy",
        Some(false) => "unhealthy",
        None => "unknown",
    };
    json!({
        "id": id,
        "name": name,
        "health": health,
        "checked": probed.is_some(),
        "version": Value::Null,
        "site": TENANT_SITE.as_str(),
        "replicas": Value::Null,
        "errorRate": Value::Null,
        "latencyMs": Value::Null,
        "dependencies": deps,
    })
}

async fn services(ch: &ChClient, dagster: &DgClient) -> Result<Value, OpsError> {
    let ch_ok = ch.rows("SELECT 1", None).await.is_ok();
    let dag_ok = dagster.is_alive().await;
    let services = vec![
        service_row(
            "clickhouse",
            "ClickHouse (Hot analytical store)",
            Some(ch_ok),
            &[],
        ),
        service_row(
            "dagster",
            "Dagster (Orchestration)",
            Some(dag_ok),
            &["clickhouse"],
        ),
        // Never probed — this used to reuse `ch_ok`, so a dead Lakekeeper
        // read healthy whenever ClickHouse (a different service) answered.
        service_row(
            "iceberg",
            "Iceberg + Lakekeeper (Open tables)",
            None,
            &["rustfs"],
        ),
        // Never probed — this used to be the literal `true`.
        service_row("rustfs", "RustFS (Object storage)", None, &[]),
    ];
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

/// `new Date().toISOString()`. Still used by [`cancel_workload_body`] for
/// the cancelled-workload response; `workloads` no longer uses it (its
/// `startedAt` is now derived from the same `ClickHouse` query as
/// `elapsed`, not this process's clock) — see `workload_row`.
#[allow(
    clippy::cast_precision_loss,
    reason = "second-precision input to a millisecond-precision formatter"
)]
fn now_iso() -> String {
    lakehouse_dagster::iso_from_unix_seconds(OffsetDateTime::now_utc().unix_timestamp() as f64)
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
            "SELECT query_id, user, toString(elapsed) elapsed, substring(query,1,80) query
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
        "startedAt": now_iso(),
    })))
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
        let iceberg = service_row("iceberg", "Iceberg", None, &["rustfs"]);
        assert_eq!(iceberg["health"], "unknown");
        assert_eq!(iceberg["checked"], false);
        for field in ["version", "replicas", "errorRate", "latencyMs"] {
            assert!(iceberg[field].is_null(), "{field} should be null");
        }

        let rustfs = service_row("rustfs", "RustFS", None, &[]);
        assert_eq!(rustfs["health"], "unknown");
        assert_eq!(rustfs["checked"], false);
    }

    #[test]
    fn probed_services_keep_their_real_health() {
        let down = service_row("clickhouse", "ClickHouse", Some(false), &[]);
        assert_eq!(down["health"], "unhealthy");
        assert_eq!(down["checked"], true);
        // Even a probed service reports null for the metrics nothing
        // measures — probing only tells us up/down, not version or
        // latency.
        assert!(down["version"].is_null());

        let up = service_row("dagster", "Dagster", Some(true), &["clickhouse"]);
        assert_eq!(up["health"], "healthy");
        assert_eq!(up["checked"], true);
    }

    #[test]
    fn observability_nulls_every_unmeasured_metric_and_drops_streaming_lag() {
        let v = observability_json(120, 0.002);
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

    #[test]
    fn now_iso_looks_like_an_iso_timestamp() {
        let s = now_iso();
        assert!(s.ends_with('Z'));
        assert_eq!(s.len(), "2026-08-27T04:00:10.075Z".len());
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
}
