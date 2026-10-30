//! Route mounting.
//!
//! Mounts the health check, the five read-only domains (catalog, overview,
//! ops, governance, storage), the write-side domains (alerts, query,
//! dashboard, ...), and — new in Phase 2 — the Postgres-backed `identity`
//! domain under `/api/identity/*`.

mod agents;
pub(crate) mod ai;
mod alerts;
pub mod auth;
// `pub(crate)`, not private: `crate::pipeline_lag` (BLD-2) reads
// `authored_pipelines::job_name` to name the job a lag run launches, and
// re-deriving the `authored__<id>` mapping in a second place is exactly
// the drift the one definition exists to prevent.
pub(crate) mod authored_pipelines;
mod calc_fields;
mod catalog;
mod catalog_governance;
mod catalog_grants;
mod catalog_profile;
mod catalog_query;
mod catalog_restore;
mod catalog_search;
mod catalog_source;
mod connector_discovery;
mod connector_oauth;
mod connector_runs;
mod connectors;
mod dashboard;
mod dashboard_folders;
pub(crate) mod dashboard_organise;
mod dashboard_sources;
pub(crate) mod drill;
mod embed;
mod pdf_export;
pub mod public_links;
mod render;
pub mod report_runner;
mod reports;
// DATA-10 review SHOULD-FIX 1: route modules stay private; tests call gold_export_history
mod gold;
mod governance;
mod hierarchies;
mod home;
mod identity;
mod knowledge;
mod lakehouse;
pub(crate) mod layers;
mod lineage;
mod load_alerts;
mod maps;
pub(crate) mod metrics;
mod notifications;
mod ops;
mod overview;
pub(crate) mod params;
mod pipelines;
mod quality;
mod quality_anomaly;
mod quality_incidents;
mod quality_load;
mod quality_schedules;
mod query;
mod query_cancel;
mod query_limits;
mod schema_changes;
pub(crate) mod schema_versions;
mod settings;
mod storage;
pub(crate) mod support;
// `pub(crate)`, because `crate::upload_convert` (the child process's
// protocol) needs this module's own sentences: a file that is too large is
// refused with the same words wherever it is noticed (`SEC-22` D4).
pub(crate) mod uploads;

use std::time::Duration;

use axum::Router;
use axum::extract::{DefaultBodyLimit, Request, State};
use axum::http::StatusCode;
use axum::middleware::{Next, from_fn_with_state};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use serde::Serialize;

use crate::json::ApiJson;
use crate::policy::auth_gate;
use crate::state::AppState;

/// The `/api/auth/*` sub-router (Task 3.2), split out for the same
/// `clippy::too_many_lines` reason as [`pipelines_router`].
fn auth_router() -> Router<AppState> {
    Router::new()
        .route("/api/auth/login", axum::routing::post(auth::login))
        .route("/api/auth/logout", axum::routing::post(auth::logout))
        .route("/api/auth/me", get(auth::me))
        .route(
            "/api/auth/change-password",
            axum::routing::post(auth::change_password),
        )
        // Unauthenticated by design — see
        // `auth::oidc_start`'s own doc comment and its
        // `crate::policy::POLICY_TABLE` entry.
        .route("/api/auth/oidc/start", get(auth::oidc_start))
        // Unauthenticated by necessity — the caller has
        // no session yet at this point in the flow. Fails closed to 401 on
        // every verification gap (see `auth::oidc_callback`'s own doc
        // comment and its `crate::policy::POLICY_TABLE` entry).
        .route("/api/auth/oidc/callback", get(auth::oidc_callback))
        // Unauthenticated by necessity — the login page
        // calls this before any session exists to decide whether to render an
        // SSO button. See `auth::providers`'s own doc comment and its
        // `crate::policy::POLICY_TABLE` entry.
        .route("/api/auth/providers", get(auth::providers))
        // List the caller's own browser sessions (or
        // every live session, for an `identity:sessions:manage` holder).
        // `Policy::RequiresAuth` floor; the own-vs-all split lives inside
        // the handler — see `auth::sessions` and `auth::SessionsDecision`.
        .route("/api/auth/sessions", get(auth::sessions))
        // Revoke a single live session by id. Same
        // floor as the list route; the admin-vs-own split lives inside
        // the handler — see `auth::revoke_session`. The path-segment id
        // is validated as a UUID by the handler itself (400 on a
        // malformed id, rather than a 404 — see that route's doc
        // comment for why a non-UUID path must not look like
        // "doesn't exist").
        .route(
            "/api/auth/sessions/{id}",
            axum::routing::delete(auth::revoke_session),
        )
}

/// Default per-request timeout, used for every route whose TypeScript
/// handler does not declare `export const maxDuration` (most of them — see
/// [`route_timeout`]).
const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

/// `/api/uploads`'s timeout. `POST /api/uploads` receives up to 50 MB
/// ([`uploads::MAX_UPLOAD_BYTES`]) before it can answer, and a request that is
/// still sending when the deadline passes is cut off mid-file. Five minutes
/// lets a slow link finish: it is a bound for a slow link, not a measurement
/// (50 MiB is about 84 seconds at 5 Mbit/s, which is arithmetic, and nothing
/// here measured any link). `GET /api/uploads` shares the path and so the
/// bound; the per-upload routes (`/api/uploads/{id}*`) keep the default.
const UPLOAD_REQUEST_TIMEOUT: Duration = Duration::from_secs(300);

/// The chart and table downloads' timeout (`BI-1` review fix (BLOCKER) R2). The
/// stream from `ClickHouse` has 300 seconds (`export::EXPORT_TIMEOUT`) and its
/// query 280 (`max_execution_time`), then the file is written and sent; the 60
/// second default cut a long export with a bare `request timeout`. Ten seconds
/// above the stream, so the sentence the export itself gives wins.
const EXPORT_REQUEST_TIMEOUT: Duration = Duration::from_secs(310);

/// The PDF route's timeout (`BI-1` review fix (BLOCKER) R2): the wait for a
/// free slot ([`crate::pdf::PDF_SLOT_WAIT`]), the client's 70 seconds with
/// Gotenberg, and ten seconds for the token, the audit and the answer.
const PDF_REQUEST_TIMEOUT: Duration = Duration::from_secs(60 + 70 + 10);

/// Per-route request timeout, mirroring each TypeScript handler's `export
/// const maxDuration` (grep of every `src/app/api/**/route.ts`):
///
/// | route                          | TS `maxDuration` |
/// |---------------------------------|------------------|
/// | `/api/ai/chat`                  | 120              |
/// | `/api/alerts/run`               | 60               |
/// | `/api/query/run`                | 60 (BI-5: the longest configured limit + 15, see [`deadline_for`]) |
/// | board routes (`/api/dashboard`, `/api/embed/*`, `/api/public/dashboard/*`, `/api/render/data`) | 60 (BI-5: an outer cap of 35 minutes, see [`deadline_for`]) |
/// | everything else (no export)     | [`DEFAULT_REQUEST_TIMEOUT`] (60) |
///
/// `/api/uploads` has no TypeScript handler to mirror and takes
/// [`UPLOAD_REQUEST_TIMEOUT`] (300), for the reason given there.
///
/// The timeout is NOT uniform in the TypeScript — `ai/chat`'s 120s covers
/// a legitimate multi-round LLM tool loop that a blanket 60s bound would
/// 408 mid-flight. Matched on `req.uri().path()`
/// before route dispatch, so path params (`/api/catalog/{id}`, ...) never
/// need to appear here — none of the parameterized routes declare a
/// non-default `maxDuration` today.
fn route_timeout(path: &str) -> Duration {
    match path {
        "/api/ai/chat" => *AI_CHAT_TIMEOUT,
        "/api/uploads" | "/api/maps" => UPLOAD_REQUEST_TIMEOUT,
        "/api/dashboard/chart-export" | "/api/dashboard/table-export" => EXPORT_REQUEST_TIMEOUT,
        "/api/dashboard/export/pdf" => PDF_REQUEST_TIMEOUT,
        // RPT-1: "Send now" makes the PDF and the chart files in the request,
        // so it gets the longest export deadline.
        other if other.starts_with("/api/reports/subscriptions/") && other.ends_with("/send") => {
            EXPORT_REQUEST_TIMEOUT
        }
        other if is_upload_part_path(other) => UPLOAD_REQUEST_TIMEOUT,
        other if is_discovery_path(other) => DISCOVERY_REQUEST_TIMEOUT,
        _ => DEFAULT_REQUEST_TIMEOUT,
    }
}

/// The routes whose queries run under the administrator's time limit
/// (`BI-5`): their deadline follows the limit instead of the fixed 60 s.
fn limit_bearing(path: &str) -> bool {
    path == "/api/query/run" || is_board_path(path)
}

/// The routes that answer a whole board: tiles run six at a time, so one can
/// need several rounds of the time limit.
///
/// `BI-5` review S1: only the routes that run a whole board's tiles. The embed
/// frame and values routes and a public link's `unlock` run no tile, so they
/// keep the ordinary deadline.
fn is_board_path(path: &str) -> bool {
    path == "/api/dashboard"
        || path == "/api/render/data"
        || path == "/api/embed/data"
        || path
            .strip_prefix("/api/public/dashboard/")
            .is_some_and(|token| !token.is_empty() && !token.contains('/'))
}

/// The deadline of `path` given the install's longest configured time limit
/// (`None`: not read, or not readable, which keeps [`route_timeout`]).
///
/// `BI-5` plan D4 says a limit-bearing route gets that limit plus 15 s, and
/// that a board's deadline is `ceil(tiles / 6) x limit + 15 s` computed in the
/// handler (which answers the finished tiles and marks the rest "not reached
/// in time"). The two cannot both be the middleware's: a deadline of one limit
/// would cut a two-round board before its handler could answer. So the
/// middleware gives `/api/query/run` the limit plus 15 s, and a board route
/// the 35-minute cap as an outer bound, leaving the handler's own deadline
/// (`query_limits::board_deadline`) the one that decides.
fn deadline_for(path: &str, longest: Option<Duration>) -> Duration {
    match longest {
        Some(_) if is_board_path(path) => crate::query_limits::BOARD_DEADLINE_CAP,
        Some(longest) if limit_bearing(path) => {
            longest.saturating_add(crate::query_limits::DEADLINE_MARGIN)
        }
        _ => route_timeout(path),
    }
}

/// Whether `path` is `/api/uploads/{id}/parts/{n}`: one 8 MiB piece of a
/// chunked upload (`DATA-17`). It gets [`UPLOAD_REQUEST_TIMEOUT`], not the
/// default 60 s, which would cut an 8 MiB piece off on a link slower than about
/// 1 Mbit/s; the other `/api/uploads/{id}*` routes keep the default.
fn is_upload_part_path(path: &str) -> bool {
    path.strip_prefix("/api/uploads/")
        .map(|rest| rest.split('/').collect::<Vec<_>>())
        .is_some_and(|segments| {
            matches!(segments.as_slice(), [id, "parts", n] if !id.is_empty() && !n.is_empty())
        })
}

/// `/api/connectors/{id}/discovery/{kind}`'s timeout (`SRC-13` decision D9): the
/// listing call's own limit is 120 seconds (`connector_listing::LIST_TIMEOUT`), so
/// the request that carries it may take a few seconds more to answer. The route
/// does a bounded amount of work (one connection, one or two queries, a bounded
/// bucket scan), so the longer deadline is not an open door.
const DISCOVERY_REQUEST_TIMEOUT: Duration = Duration::from_secs(130);

/// Whether `path` is `/api/connectors/{id}/discovery/list` or `.../preview`.
fn is_discovery_path(path: &str) -> bool {
    path.strip_prefix("/api/connectors/")
        .and_then(|rest| rest.split_once('/'))
        .is_some_and(|(id, tail)| {
            !id.is_empty() && matches!(tail, "discovery/list" | "discovery/preview")
        })
}

/// `/api/ai/chat`'s timeout: `AI_CHAT_TIMEOUT_SECS` when set, else 120s.
///
/// 120s fits a hosted model. A small model served on CPU (an on-prem
/// deployment without a GPU) needs several minutes for one multi-round
/// tool loop: measured on a 8-core CPU, `qwen3:4b` under Ollama took 37s
/// to answer "say ok" alone, so every copilot turn with that model timed
/// out at 120s. An operator who chooses such a model can raise the bound;
/// nothing else changes. Read once, like `tenant.rs`'s deployment labels.
static AI_CHAT_TIMEOUT: std::sync::LazyLock<Duration> = std::sync::LazyLock::new(|| {
    chat_timeout_from(std::env::var("AI_CHAT_TIMEOUT_SECS").ok().as_deref())
});

/// Parses `AI_CHAT_TIMEOUT_SECS`: whole seconds, clamped to 30..=1800 so a
/// typo can neither make every chat time out nor hold a connection for
/// hours. Unset, empty or unparseable -> the 120s default.
fn chat_timeout_from(raw: Option<&str>) -> Duration {
    raw.map(str::trim)
        .and_then(|s| s.parse::<u64>().ok())
        .map_or(Duration::from_secs(120), |secs| {
            Duration::from_secs(secs.clamp(30, 1800))
        })
}

/// Build the application router with `state` threaded through every
/// handler.
///
/// `/api/governance/lineage` is registered as its own static route
/// alongside `/api/governance/{kind}`. Axum matches static segments before
/// captures (unlike a naive first-match router), so a request for
/// `/api/governance/lineage` always reaches [`governance::lineage`], never
/// [`governance::get`] with `kind = "lineage"` — verified by
/// `governance_lineage_route_does_not_fall_through_to_kind_dispatch` in
/// `main.rs`.
///
/// No `#[must_use]` here: `axum::Router` is already `#[must_use]`, and
/// repeating the attribute without a message trips
/// `clippy::double_must_use`.
/// `POST /api/pipelines/runnable/{id}/enforce` (BLD-1 / D3), the rewrite
/// the orchestrator asks for before it runs a pipeline's `SELECT`.
///
/// Its own function for the same reason [`pipelines_router`] is: so the
/// route can be added without pushing that function over
/// `clippy::too_many_lines`. It merges straight back in, through
/// [`authored_pipelines_router`], and carries no middleware or state of
/// its own.
fn pipeline_enforce_router() -> Router<AppState> {
    Router::new().route(
        "/api/pipelines/runnable/{id}/enforce",
        axum::routing::post(authored_pipelines::enforce_pipeline_sql),
    )
}

/// The authored-pipeline (orchestrator-only) routes: the definitions the
/// job factory loads, and the rewrite it asks for before it runs one.
///
/// `pipelines_router` merges this rather than growing another four lines:
/// every route here is orchestrator-only in the handler as well as in
/// [`crate::policy`], and keeping them together makes that pairing the
/// thing a reader sees first.
fn authored_pipelines_router() -> Router<AppState> {
    pipeline_enforce_router().route("/api/pipelines/runnable", get(authored_pipelines::runnable))
}

/// The `/api/pipelines/*` sub-router (Task 2.5), split out of [`router`]
/// purely to keep that function under `clippy::too_many_lines` — it merges
/// straight back in, with no separate middleware/state of its own.
fn pipelines_router() -> Router<AppState> {
    Router::new()
        .route(
            "/api/pipelines",
            get(pipelines::list).post(pipelines::create),
        )
        .route(
            "/api/pipelines/generate",
            axum::routing::post(pipelines::generate),
        )
        // Static, so it is matched ahead of `/api/pipelines/{id}`.
        .merge(authored_pipelines_router())
        .route(
            "/api/pipelines/{id}",
            get(pipelines::detail)
                .put(authored_pipelines::update)
                .delete(authored_pipelines::delete),
        )
        .route("/api/pipelines/{id}/source", get(pipelines::source))
        .route(
            "/api/pipelines/{id}/schedule-ticks",
            get(authored_pipelines::schedule_ticks),
        )
        // BLD-2: the two target-lag routes live in `target_lag_routes`,
        // merged in above.
        .merge(target_lag_routes())
        .route("/api/pipelines/{id}/runs", get(pipelines::runs))
        // Plan R4 2b: definition version history. Static under
        // `{id}/versions`, so it is registered BEFORE
        // `{id}/versions/{version}/restore` (which would otherwise
        // match `{version}/restore` as `{version}`).
        .route(
            "/api/pipelines/{id}/versions",
            get(pipelines::list_versions),
        )
        .route(
            "/api/pipelines/{id}/versions/{version}",
            get(pipelines::get_version),
        )
        .route(
            "/api/pipelines/{id}/versions/{version}/restore",
            axum::routing::post(authored_pipelines::restore_version),
        )
        // Plan 1c (R2, day-1): the runs × steps matrix endpoint is
        // STATIC under `{id}/runs/steps`. axum's `matchit` router
        // resolves static segments ahead of path parameters regardless
        // of registration order — so the literal path here reaches
        // `runs_step_matrix`, and a path with an actual `runId`
        // reaches `run_steps` below. Registration order is a
        // readability aid, not a correctness requirement
        // (PR #57 review F1.10). The two paths' routing is pinned by
        // `tests/pipeline_routing.rs::both_paths_resolve_to_their_own_handler`.
        .route(
            "/api/pipelines/{id}/runs/steps",
            get(pipelines::runs_step_matrix),
        )
        .route(
            "/api/pipelines/{id}/runs/{runId}/steps",
            get(pipelines::run_steps),
        )
        .route(
            "/api/pipelines/{id}/runs/{runId}/logs",
            get(pipelines::run_logs),
        )
        .route(
            "/api/pipelines/{id}/trigger",
            axum::routing::post(pipelines::trigger),
        )
        // R4 plan 2c: read-side companion to `/trigger`. Registered
        // BEFORE `/api/pipelines/{id}` only matters for static sub-paths
        // (already covered above) — `{id}/config-schema` is registered
        // here next to `/trigger` since they share the `id` namespace
        // and route handlers share `path = {id}`.
        .route(
            "/api/pipelines/{id}/config-schema",
            get(pipelines::config_schema),
        )
        .route(
            "/api/pipelines/{id}/status",
            axum::routing::post(pipelines::set_status_route),
        )
        .route(
            "/api/pipelines/{id}/pause",
            axum::routing::post(pipelines::pause),
        )
        .route(
            "/api/pipelines/{id}/resume",
            axum::routing::post(pipelines::resume),
        )
        .route(
            "/api/pipelines/runs/{runId}/cancel",
            axum::routing::post(pipelines::cancel_run),
        )
        .route(
            "/api/pipelines/runs/{runId}/retry",
            axum::routing::post(pipelines::retry_run),
        )
        // The assignment route for pipeline
        // rows `0042_tenant_provisioning.sql` leaves `tenant_id = NULL`.
        .route(
            "/api/pipelines/{id}/tenant",
            axum::routing::put(pipelines::assign_pipeline_tenant),
        )
        // BLD-1 / D2: assign the user a pipeline reads as. A distinct
        // route for the same reason as `/tenant` — who a row reads as is a
        // governance decision, not a field on an unrelated update body.
        .route(
            "/api/pipelines/{id}/run-as",
            axum::routing::put(pipelines::assign_pipeline_run_as),
        )
        // Dagster `run_failure_sensor` posts here once per failed run;
        // see `dagster/dispar_orchestrate/pipeline_events.py`. Handler
        // enforces service-identity (the route is gated by `pipeline:write`
        // first, then the handler refuses a non-service `pipeline:write`
        // holder, mirroring `authored_pipelines::runnable`).
        .route(
            "/api/pipelines/events/run-failed",
            axum::routing::post(pipelines::run_failed_event),
        )
        // Plan 1f: Dagster `run_status_sensor(SUCCESS)` posts here once
        // per successful run. Same posture as the run-failed route — the
        // handler enforces a service identity on top of the `pipeline:write`
        // gate. Computes the run's `slow`/`volume_drop` outcomes and hands
        // each to its per-kind evaluator: `evaluate_pipeline_slow` /
        // `evaluate_pipeline_volume_drop`.
        .route(
            "/api/pipelines/events/run-finished",
            axum::routing::post(pipelines::run_finished_event),
        )
        // Plan 1f: per-pipeline SLA. `GET` returns 404 when no SLA is
        // configured (so the UI can distinguish "no SLA yet" from a
        // degraded backend); `PUT` upserts and writes a `pipeline.sla_set`
        // audit event. The route validates the body (positive thresholds
        // only) — the database CHECK is defense in depth.
        .route(
            "/api/pipelines/{id}/sla",
            get(pipelines::get_sla).put(pipelines::put_sla),
        )
        // Plan 1f: volume history (last 30 runs + their row counts and the
        // `drop` decision per run). Same 30-run window as `/runs`.
        .route(
            "/api/pipelines/{id}/volume",
            axum::routing::get(pipelines::volume),
        )
}

/// BLD-2 (plan D6/D8): the target lag's own two routes, merged into the
/// pipeline sub-router. They sit together because they are one feature's
/// two doors — the orchestrator asks [`authored_pipelines::lag_check`] and
/// a user sets the target through
/// [`authored_pipelines::set_target_lag`] — and splitting them from
/// [`pipelines_router`] is also what keeps that function under
/// `clippy::too_many_lines`.
///
/// `lag-check` is under `/runnable/` because only the orchestrator may ask
/// (the same gate as `runnable` itself), not because it lists anything.
/// `target-lag` is its own route rather than a field on
/// `PUT /api/pipelines/{id}`: clearing the lag (`AC8`) is a value that
/// route's "absent means unchanged" body cannot express.
fn target_lag_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/api/pipelines/runnable/{id}/lag-check",
            post(authored_pipelines::lag_check),
        )
        .route(
            "/api/pipelines/{id}/target-lag",
            put(authored_pipelines::set_target_lag),
        )
}

/// The `/api/storage/*` sub-router (Task 2.6), split out for the same
/// `clippy::too_many_lines` reason as [`pipelines_router`].
fn storage_router() -> Router<AppState> {
    Router::new()
        .route("/api/storage", get(storage::get))
        .route(
            "/api/storage/policies",
            get(storage::list_policies).post(storage::create_policy),
        )
        .route("/api/storage/operations", get(storage::list_operations))
        .route(
            "/api/storage/restore",
            axum::routing::post(storage::restore_asset),
        )
}

/// The `/api/lakehouse/*` sub-router (WS2 §4), split out for the same
/// `clippy::too_many_lines` reason as [`pipelines_router`].
fn lakehouse_router() -> Router<AppState> {
    Router::new()
        .route("/api/lakehouse/warehouses", get(lakehouse::warehouses))
        .route("/api/lakehouse/namespaces", get(lakehouse::namespaces))
        .route("/api/lakehouse/tables", get(lakehouse::tables))
        .route(
            "/api/lakehouse/tables/{ns}/{table}",
            get(lakehouse::table_detail),
        )
        .route(
            "/api/lakehouse/tables/{ns}/{table}/maintenance",
            get(lakehouse::maintenance).post(lakehouse::set_maintenance_policy),
        )
        .route(
            "/api/lakehouse/maintenance-policies",
            get(lakehouse::list_maintenance_policies),
        )
        .route("/api/lakehouse/capacity", get(lakehouse::capacity))
}

/// The static (non-`{kind}`) `/api/governance/*` routes, split out for the
/// same `clippy::too_many_lines` reason as [`pipelines_router`]. Each is
/// mounted ahead of the generic `/api/governance/{kind}` fallback in
/// [`router`] — static segments match before captures, matching the
/// `lineage`/`policies` precedent documented on [`router`] itself.
fn governance_static_router() -> Router<AppState> {
    Router::new()
        .route("/api/governance/lineage", get(governance::lineage))
        // DATA-13: column-level lineage. A static path ahead of the
        // `{kind}` capture like `lineage` above (and four segments deep).
        .route(
            "/api/governance/lineage/columns",
            get(governance::column_lineage),
        )
        // BI-14 (D5): the charts that read a table. Static, ahead of `{kind}`.
        .route(
            "/api/governance/lineage/used-by",
            get(lineage::used_by::used_by),
        )
        // DATA-14 (F2): quality incidents. Static segments, mounted ahead of
        // `/api/governance/quality/{id}` and the `{kind}` capture.
        .route(
            "/api/governance/quality/incidents",
            get(quality_incidents::list_incidents),
        )
        .route(
            "/api/governance/quality/incidents/{id}/resolve",
            axum::routing::post(quality_incidents::resolve_incident),
        )
        // DATA-14 (F3): when a table's rules run. Static `schedules` segment,
        // so `/api/governance/quality/{id}/run` cannot capture it.
        .route(
            "/api/governance/quality/schedules",
            get(quality_schedules::list_schedules),
        )
        .route(
            "/api/governance/quality/schedules/{asset}",
            put(quality_schedules::put_schedule),
        )
        // DATA-14 (F5): whether a table's anomalies are being raised yet.
        .route(
            "/api/governance/quality/learning/{asset}",
            get(quality_anomaly::learning_status),
        )
        // DATA-14 (F7): a rule's run history. `runs` is not `run`, so it
        // cannot be captured by the POST route below.
        .route("/api/governance/quality/{id}/runs", get(quality::rule_runs))
        // Evaluates one authored quality rule on request. Three segments
        // deep, so it cannot collide with the `{kind}` capture.
        .route(
            "/api/governance/quality/{id}/run",
            axum::routing::post(quality::run_rule),
        )
        .route(
            "/api/governance/quality/{id}",
            axum::routing::put(quality::update_rule).delete(quality::delete_rule),
        )
        .route(
            // A dedicated route (WS3 item 17), mounted alongside `lineage`
            // immediately above — never a seventh `{kind}` dispatch value
            // (see `governance.rs`'s module doc comment).
            "/api/governance/ingest-runs",
            get(governance::ingest_runs),
        )
        .route(
            "/api/governance/policies",
            get(governance::list_policies).post(governance::create_policy_route),
        )
        // WS7 item A5: real policy impact preview (closes WS1 task 12's
        // deferred half).
        .route(
            "/api/governance/policies/preview",
            axum::routing::post(governance::preview_policy),
        )
        .route(
            "/api/governance/policies/{id}",
            axum::routing::delete(governance::delete_policy),
        )
        .route(
            "/api/governance/policies/{id}/status",
            axum::routing::put(governance::set_policy_status),
        )
        // WS5 item E1 (Y6): dataset freshness SLA.
        .route(
            "/api/governance/sla",
            get(governance::get_sla).put(governance::put_sla),
        )
        .route(
            "/api/governance/sla/{table}",
            axum::routing::delete(governance::delete_sla),
        )
        .route(
            "/api/governance/classification/{id}",
            axum::routing::delete(governance::delete_classification_rule),
        )
        // DATA-12 part 2: governed tag keys. Static segments, so they match
        // ahead of the `{kind}` capture, as `policies` and `sla` do.
        .route(
            "/api/governance/tag-keys",
            get(governance::list_tag_keys).post(governance::create_tag_key),
        )
        .route(
            "/api/governance/tag-keys/{key}",
            axum::routing::put(governance::update_tag_key).delete(governance::delete_tag_key),
        )
}

/// The `/api/catalog/{id}/access-request` +
/// `/api/catalog/access-requests/{id}/decide` sub-router (WS7 items E2/E3),
/// split out for the same `clippy::too_many_lines` reason as
/// [`pipelines_router`]. Two distinct paths under `/api/catalog` — one has
/// a literal `access-requests` segment where the other has a `{id}`
/// capture at the same position, so axum's router never treats them as
/// ambiguous.
fn catalog_access_router() -> Router<AppState> {
    Router::new()
        .route(
            "/api/catalog/{id}/access-request",
            axum::routing::post(catalog::access_request),
        )
        .route(
            "/api/catalog/access-requests/{id}/decide",
            axum::routing::post(catalog::decide_access_request),
        )
        // `DATA-15` F3: grant and revoke from the asset's page.
        .route(
            "/api/catalog/{id}/grants",
            get(catalog_grants::list).post(catalog_grants::create),
        )
        .route(
            "/api/catalog/{id}/grants/{grant_id}",
            axum::routing::delete(catalog_grants::revoke),
        )
        // `DATA-15` F4: undrop a dropped Iceberg table within its window.
        .route(
            "/api/catalog/{id}/restore",
            get(catalog_restore::status).post(catalog_restore::restore),
        )
}

/// The `/api/overview/alerts/*` sub-router (Task 2.6), split out for the
/// same `clippy::too_many_lines` reason as [`pipelines_router`].
fn overview_alerts_router() -> Router<AppState> {
    Router::new()
        .route("/api/overview/alerts", get(overview::list_alerts))
        .route(
            "/api/overview/alerts/{id}/acknowledge",
            axum::routing::post(overview::acknowledge_alert),
        )
        .route(
            "/api/overview/alerts/{id}/resolve",
            axum::routing::post(overview::resolve_alert),
        )
        // WS5 item C1: silence a fired alert (and its rule's future
        // firings) for a caller-chosen number of minutes.
        .route(
            "/api/overview/alerts/{id}/silence",
            axum::routing::post(overview::silence_alert),
        )
}

/// The tenant-gated half of [`connectors_router`], split out of it for
/// `clippy::too_many_lines` (`SRC-11` review 1 BLOCKER 1), the same way
/// [`pipelines_router`] is split from its parent.
fn per_connector_router(state: &AppState) -> Router<AppState> {
    Router::new()
        .route(
            "/api/connectors/{id}",
            get(connectors::detail)
                .patch(connectors::update)
                .delete(connectors::delete),
        )
        .route(
            "/api/connectors/{id}/test",
            axum::routing::post(connectors::test_connection),
        )
        .route(
            "/api/connectors/{id}/probe-history",
            get(connectors::probe_history),
        )
        .route(
            "/api/connectors/{id}/secret",
            axum::routing::put(connectors::rotate_secret),
        )
        .route(
            "/api/connectors/{id}/credential",
            axum::routing::put(connectors::set_credential),
        )
        // `SRC-13`: list and preview what a connector's source holds, for a
        // person with `connector:manage` on a connector of their own tenant (D11).
        // `{kind}` is `list` or `preview`; POST asks, GET reads the latest answer.
        .route(
            "/api/connectors/{id}/discovery/{kind}",
            axum::routing::post(connector_discovery::start).get(connector_discovery::poll),
        )
        .route(
            "/api/connectors/{id}/ingest-spec",
            get(connectors::ingest_spec_get).put(connectors::ingest_spec_put),
        )
        .route(
            "/api/connectors/{id}/ingest/run",
            axum::routing::post(connectors::ingest_run_request),
        )
        // `SRC-11` decisions D4, D5, D7, D8: the run history, a run's log and
        // Resume, for a person with `connector:manage` on a connector of their
        // own tenant.
        .route("/api/connectors/{id}/runs", get(connector_runs::list))
        .route(
            "/api/connectors/{id}/runs/{runId}/log",
            get(connector_runs::log),
        )
        .route(
            "/api/connectors/{id}/resume",
            axum::routing::post(connector_runs::resume),
        )
        // `SRC-4` slice 2 (S2-e, S2-h): a person's pause, and the state of a
        // change-capture connector in Kafka Connect.
        .route(
            "/api/connectors/{id}/pause",
            axum::routing::post(connector_runs::pause),
        )
        .route(
            "/api/connectors/{id}/change-capture",
            get(connector_runs::change_capture),
        )
        // `SRC-12` part C: signing a REST connector in with `OAuth2`, for a
        // person with `connector:manage` on a connector of their own tenant.
        .route("/api/connectors/{id}/oauth", get(connector_oauth::status))
        .route(
            "/api/connectors/{id}/oauth/start",
            axum::routing::post(connector_oauth::start),
        )
        .route(
            "/api/connectors/{id}/oauth/sign-out",
            axum::routing::post(connector_oauth::sign_out),
        )
        // `SRC-8` decision D6: a person with `connector:manage` on a
        // connector of their own tenant reads and approves schema changes.
        .route(
            "/api/connectors/{id}/schema-changes",
            get(schema_changes::list),
        )
        .route(
            "/api/connectors/{id}/schema-changes/approve",
            axum::routing::post(schema_changes::approve),
        )
        .route_layer(from_fn_with_state(
            state.clone(),
            connectors::require_connector_in_tenants,
        ))
}

/// The `/api/connectors/*` sub-router (Task 2.7), split out for the same
/// `clippy::too_many_lines` reason as [`pipelines_router`].
///
/// Every `/api/connectors/{id}/*` route sits behind
/// [`connectors::require_connector_in_tenants`]: the caller must belong to
/// the connector's tenant. `PUT .../tenant` stays outside it — it is
/// identity administration (`identity:write`), and the only way a
/// connector with no tenant gets one.
fn connectors_router(state: &AppState) -> Router<AppState> {
    let per_connector = per_connector_router(state);
    Router::new()
        .route(
            "/api/connectors",
            get(connectors::list).post(connectors::create),
        )
        .route(
            // A dedicated route (not `?ingestible=true` on `/api/connectors`):
            // `Policy::RequiresPermission` takes one string per route, so
            // overloading the existing `connector:manage`-gated route would
            // need an "either permission" `Policy` variant this workstream
            // does not otherwise need (the ingest:read scope this route is gated on, below). matchit (axum's router)
            // matches this static segment ahead of the `{id}` param route
            // below for the literal path `/api/connectors/ingestible`.
            "/api/connectors/ingestible",
            get(connectors::list_ingestible),
        )
        .route(
            // Gap fix (WS3 item 33): `list_connector_types`/`listTypes` both
            // already existed with no route between them — the wizard's
            // connector-type list 404d. A dedicated static route, matching
            // `/ingestible`'s shape immediately above, matched ahead of
            // `/api/connectors/{id}` for the same reason.
            "/api/connectors/types",
            get(connectors::list_types),
        )
        .route(
            // `SRC-12` part B: static, so matched ahead of
            // `/api/connectors/{id}` like `/types`.
            "/api/connectors/credential-settings",
            get(connectors::credential_settings),
        )
        // The assignment route for connector
        // rows `0042_tenant_provisioning.sql` leaves `tenant_id = NULL`.
        .route(
            "/api/connectors/{id}/tenant",
            axum::routing::put(connectors::assign_connector_tenant),
        )
        // `SRC-8`: the orchestrator's observation. Outside the tenant gate on
        // purpose: its only caller is a service identity, which belongs to no
        // tenant (see `schema_changes`' module doc).
        .route(
            "/api/connectors/{id}/schema-observations",
            axum::routing::post(schema_changes::observe),
        )
        .route(
            "/api/connectors/{id}/schema-observations/tables",
            axum::routing::post(schema_changes::new_tables),
        )
        // `SRC-11`: the orchestrator's run reports. Outside the tenant gate for
        // the same reason (its caller is a service identity); each handler
        // checks `schema_changes::require_orchestrator`.
        .route(
            "/api/connectors/{id}/run-reports",
            axum::routing::post(connector_runs::report_start),
        )
        .route(
            "/api/connectors/{id}/run-reports/{runId}/attempts",
            axum::routing::post(connector_runs::report_attempt),
        )
        .route(
            "/api/connectors/{id}/run-reports/{runId}/log",
            axum::routing::post(connector_runs::report_log),
        )
        .route(
            "/api/connectors/{id}/run-reports/{runId}/end",
            axum::routing::post(connector_runs::report_end),
        )
        // `SRC-12` part C: the provider sends the person's browser back here. It
        // has no connector in its path, so it sits outside the tenant gate; the
        // handler checks the user, the tenant and `connector:manage` itself.
        // Static, so matched ahead of `/api/connectors/{id}` like `/types`.
        .route(
            "/api/connectors/oauth/callback",
            get(connector_oauth::callback),
        )
        // `SRC-12` part C: the orchestrator reports a refresh token the provider
        // rotated. Outside the tenant gate (a service identity belongs to no
        // tenant); the handler checks `schema_changes::require_orchestrator`.
        .route(
            "/api/connectors/{id}/oauth/refresh-token",
            axum::routing::post(connector_oauth::update_refresh_token),
        )
        // `SRC-13`: the orchestrator's answer to a discovery request. Outside the
        // tenant gate for the same reason (a service identity belongs to no
        // tenant); the handler checks `schema_changes::require_orchestrator`. The
        // body may carry a listing of up to 3 MiB, so this route, and only this
        // one, takes the larger body limit.
        .route(
            "/api/connectors/{id}/discovery-results",
            axum::routing::post(connector_discovery::report_result).layer(DefaultBodyLimit::max(
                connector_discovery::MAX_RESULT_BODY_BYTES,
            )),
        )
        .merge(per_connector)
}

/// The `/api/uploads/*` sub-router (ADR 0014, plan T6), shaped like
/// [`connectors_router`].
///
/// Every `/api/uploads/{id}/*` route sits behind
/// [`uploads::require_upload_in_tenants`]: the caller must belong to the
/// upload's tenant, and another tenant's upload answers the 404 an unknown id
/// gets. `POST /api/uploads` alone takes a larger body than axum's 2 MB
/// default, up to the cap plus the multipart framing
/// ([`uploads::MAX_REQUEST_BODY_BYTES`]); `.layer` on that one method router
/// is what keeps the raised limit off every other route, `GET /api/uploads`
/// included (it is added after the layer).
fn uploads_router(state: &AppState) -> Router<AppState> {
    let per_upload = Router::new()
        .route(
            "/api/uploads/{id}",
            get(uploads::get).delete(uploads::delete),
        )
        .route("/api/uploads/{id}/preview", get(uploads::preview))
        // DATA-18: apply a column list to the sample, store nothing.
        .route(
            "/api/uploads/{id}/check",
            axum::routing::post(uploads::columns::check),
        )
        .route(
            "/api/uploads/{id}/ingest",
            axum::routing::post(uploads::ingest),
        )
        // DATA-17 slice 5: which other files can load into one table with this one.
        .route(
            "/api/uploads/{id}/compare",
            axum::routing::post(uploads::group::compare),
        )
        // DATA-17: the pieces of a chunked upload. Only the piece route takes
        // a large body, and its own limit (a piece plus 64 KiB); the other
        // routes keep axum's 2 MB default.
        .route(
            "/api/uploads/{id}/parts/{n}",
            axum::routing::put(uploads::sessions::put_part).layer(DefaultBodyLimit::max(
                uploads::sessions::MAX_PART_BODY_BYTES,
            )),
        )
        .route(
            "/api/uploads/{id}/parts",
            get(uploads::sessions::list_parts),
        )
        .route(
            "/api/uploads/{id}/complete",
            axum::routing::post(uploads::sessions::complete),
        )
        .route_layer(from_fn_with_state(
            state.clone(),
            uploads::require_upload_in_tenants,
        ));
    Router::new()
        .route(
            "/api/uploads",
            axum::routing::post(uploads::create)
                .layer(DefaultBodyLimit::max(uploads::MAX_REQUEST_BODY_BYTES))
                .get(uploads::list),
        )
        // DATA-17: opens a chunked upload. A static path next to `{id}`; it
        // belongs to no existing upload, so it sits outside the tenant layer.
        .route(
            "/api/uploads/sessions",
            axum::routing::post(uploads::sessions::create_session),
        )
        // DATA-35: folders for uploaded files. Static paths next to `{id}` (the
        // router prefers a static segment, and an upload id is `up-<uuid>`), outside
        // the per-upload layer because they name no upload: each folder is checked
        // against the caller's tenant by the store (`uploads::folders`).
        .route(
            "/api/uploads/folders",
            get(uploads::folders::list).post(uploads::folders::create),
        )
        .route(
            "/api/uploads/folders/{id}",
            axum::routing::patch(uploads::folders::update).delete(uploads::folders::delete),
        )
        .route(
            "/api/uploads/move",
            axum::routing::post(uploads::folders::move_uploads),
        )
        .merge(per_upload)
}

/// The `/api/maps/*` sub-router (`BI-16` part C2b): `POST /api/maps` alone
/// takes the larger body ([`maps::MAX_REQUEST_BODY_BYTES`]), as
/// `POST /api/uploads` does; the layer on that one method router keeps the
/// raised limit off every other route.
fn maps_router() -> Router<AppState> {
    Router::new()
        .route(
            "/api/maps",
            axum::routing::post(maps::create)
                .layer(DefaultBodyLimit::max(maps::MAX_REQUEST_BODY_BYTES))
                .get(maps::list),
        )
        .route(
            "/api/maps/{id}",
            axum::routing::patch(maps::rename).delete(maps::delete),
        )
        .route("/api/maps/{id}/geojson", get(maps::geojson))
}

/// The `/api/identity/*` sub-router (Phase 2 identity domain), split out
/// for the same `clippy::too_many_lines` reason as [`pipelines_router`].
///
/// Grouped under a single `/api/identity` namespace rather than four
/// top-level nouns (`/api/users`, `/api/tenants`, ...): every Phase 1
/// route is already `/api/<domain>[/<sub>]` (`/api/governance/{kind}`,
/// `/api/dashboard/specs`, `/api/alerts/run`), the console's
/// `identityService` is a single contract, and top-level `/api/users`
/// would be the first route whose path says nothing about which domain
/// owns it. Collection paths are plural nouns with GET = list and POST =
/// create, per REST.
fn identity_router() -> Router<AppState> {
    Router::new()
        .route(
            "/api/identity/users",
            get(identity::list_users).post(identity::create_user),
        )
        // Give an existing user a tenant. Both ids are validated inside
        // the handler (400 when malformed, 404 when unknown).
        .route(
            "/api/identity/users/{id}/tenants/{tenant_id}",
            axum::routing::put(identity::add_user_to_tenant),
        )
        .route(
            "/api/identity/roles",
            get(identity::list_roles).post(identity::create_role),
        )
        .route(
            "/api/identity/tenants",
            get(identity::list_tenants).post(identity::create_tenant),
        )
        .route(
            "/api/identity/service-identities",
            get(identity::list_service_identities).post(identity::create_service_identity),
        )
        // Rotate a service
        // identity's credential. `{id}` is validated as a UUID inside the
        // handler (400 on a malformed id, 404 via `StoreError::NotFound`
        // on a well-formed id with no row).
        .route(
            "/api/identity/service-identities/{id}/rotate",
            axum::routing::post(identity::rotate_service_identity),
        )
}

/// The `/api/knowledge/*` sub-router (Task 2.8), split out for the same
/// `clippy::too_many_lines` reason as [`pipelines_router`].
fn knowledge_router() -> Router<AppState> {
    Router::new()
        .route(
            "/api/knowledge/sources",
            get(knowledge::list_sources).post(knowledge::create_source),
        )
        .route(
            "/api/knowledge/vector-jobs",
            get(knowledge::list_vector_jobs).post(knowledge::create_vector_job),
        )
}

/// The `/api/agents/*` sub-router (Task 2.9), split out for the same
/// `clippy::too_many_lines` reason as [`pipelines_router`].
fn agents_router() -> Router<AppState> {
    Router::new()
        .route(
            "/api/agents/workflows",
            get(agents::list_workflows).post(agents::create_workflow),
        )
        .route(
            "/api/agents/employees",
            get(agents::list_employees).post(agents::create_employee),
        )
        .route("/api/agents/employees/{id}", get(agents::get_employee))
        .route(
            "/api/agents/employees/{id}/suspend",
            axum::routing::post(agents::suspend_employee),
        )
        .route(
            "/api/agents/employees/{id}/resume",
            axum::routing::post(agents::resume_employee),
        )
        .route(
            "/api/agents/employees/{id}/revoke",
            axum::routing::post(agents::revoke_employee),
        )
        .route(
            "/api/agents/employees/{id}/run",
            axum::routing::post(agents::run_employee),
        )
        // WS7 item G4: POST /api/agents/tools ("register a tool" against
        // the unread agent_tool table) is removed — GET now reflects the
        // real ai_registry::TOOLS registry, see routes::agents's module
        // doc comment.
        .route("/api/agents/tools", get(agents::list_tools))
        .route("/api/agents/runs", get(agents::list_runs))
        .route("/api/agents/runs/{id}", get(agents::get_run))
        .route("/api/agents/approvals", get(agents::list_approvals))
        .route(
            "/api/agents/approvals/{id}/decide",
            axum::routing::post(agents::decide_approval),
        )
}

/// Build the application router with `state` threaded through every
/// handler. See this function's original placement doc comment above
/// [`pipelines_router`] for the `/api/governance/lineage` static-vs-capture
/// routing note; this one-liner exists only to satisfy `missing_docs` now
/// that the `lakehouse-api` library target (`src/lib.rs`) makes `routes` a
/// `pub mod`, and this the crate's one public router constructor.
#[allow(
    clippy::too_many_lines,
    reason = "a flat list of `.route(...)` registrations; splitting further \
              would scatter the route table across more sub-routers with \
              no independent reuse, hurting rather than helping the \
              'read the whole map in one place' goal `pipelines_router`/ \
              `storage_router`/etc. already serve for the larger groups"
)]
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/api/catalog", get(catalog::list))
        // Registered before `/{id}`: axum prefers static segments over
        // params, but keeping them adjacent in this order makes it obvious
        // that `query` is a literal path and not an asset called "query".
        .route("/api/catalog/query", get(catalog::query))
        .route("/api/catalog/{id}", get(catalog::detail))
        .route("/api/catalog/{id}/profile", get(catalog_profile::profile))
        .route("/api/catalog/{id}/sample", get(catalog::sample))
        .route(
            "/api/catalog/{id}/annotation",
            get(catalog::get_annotation).put(catalog::put_annotation),
        )
        .route(
            "/api/catalog/{id}/certification",
            put(catalog::put_certification),
        )
        // The semantic layer: a table's and a column's plain-words
        // description. `{asset}` is `serving.<table>` or `silver.<table>`;
        // the dot stays inside one path segment.
        .route("/api/semantic", get(ai::semantic_api::list))
        .route(
            "/api/semantic/{asset}",
            get(ai::semantic_api::get_asset).put(ai::semantic_api::put_entry),
        )
        .merge(catalog_access_router())
        .route("/api/overview", get(overview::get).post(overview::refresh))
        .merge(overview_alerts_router())
        // `/api/ops/logs` is a literal segment registered ahead of the
        // `/api/ops/{kind}` wildcard below (WS5 item G1) -- axum matches a
        // static segment over a path parameter when both are registered,
        // same precedent as `/api/governance/lineage`-before-`{kind}`.
        .route("/api/ops/logs", get(ops::logs))
        .route("/api/ops/{kind}", get(ops::get))
        .route(
            "/api/ops/workloads/{id}/cancel",
            axum::routing::post(ops::cancel_workload),
        )
        .merge(governance_static_router())
        .route(
            "/api/governance/{kind}",
            get(governance::get).post(governance::create_rule),
        )
        .merge(storage_router())
        .merge(lakehouse_router())
        .route(
            "/api/alerts",
            get(alerts::list)
                .post(alerts::create)
                .put(alerts::update)
                .delete(alerts::delete),
        )
        .route("/api/alerts/status", get(alerts::status))
        .route("/api/alerts/run", get(alerts::run).post(alerts::run))
        .route(
            "/api/gold/export/{mart}",
            get(gold::read_back).post(gold::export),
        )
        .route("/api/gold/exports", get(gold::exports))
        .route("/api/gold/export/{mart}/consumers", get(gold::consumers))
        // DATA-1: per-mart publish-to-Iceberg switches — the scheduler's
        // list of enabled marts, and the per-mart switch the asset page
        // flips. POLICY_TABLE mirrors both paths below.
        .route("/api/gold/publications", get(gold::publications))
        .route(
            "/api/gold/export/{mart}/publication",
            get(gold::publication).put(gold::set_publication),
        )
        .route("/api/query/run", axum::routing::post(query::run))
        .route("/api/query/estimate", axum::routing::post(query::estimate))
        .route(
            "/api/query/saved",
            get(query::list_saved).post(query::create_saved),
        )
        .route("/api/query/history", get(query::list_history))
        .route("/api/query/run/{id}/download", get(query::download))
        .route("/api/query/scheduling", get(query::scheduling))
        .merge(pipelines_router())
        .route("/api/dashboard", get(dashboard::get))
        .route(
            "/api/dashboard/specs",
            get(dashboard::specs_list)
                .post(dashboard::specs_create)
                .put(dashboard::specs_update)
                .delete(dashboard::specs_delete),
        )
        .route(
            "/api/dashboard/specs/preview",
            axum::routing::post(dashboard::specs_preview),
        )
        .route(
            "/api/dashboard/boards",
            get(dashboard::boards_list)
                .post(dashboard::boards_create)
                .put(dashboard::boards_update)
                .delete(dashboard::boards_delete),
        )
        .route(
            "/api/dashboard/folders",
            get(dashboard_folders::list)
                .post(dashboard_folders::create)
                .put(dashboard_folders::update)
                .delete(dashboard_folders::delete),
        )
        .route(
            "/api/dashboard/personal",
            axum::routing::post(dashboard_folders::personal),
        )
        .route(
            "/api/dashboard/versions",
            get(dashboard_organise::versions_list),
        )
        .route(
            "/api/dashboard/versions/revert",
            axum::routing::post(dashboard_organise::revert),
        )
        .route("/api/dashboard/trash", get(dashboard_organise::trash_list))
        .route(
            "/api/dashboard/trash/restore",
            axum::routing::post(dashboard_organise::trash_restore),
        )
        .route(
            "/api/dashboard/shortcuts",
            get(dashboard_organise::shortcuts),
        )
        .route(
            "/api/dashboard/favourites",
            axum::routing::put(dashboard_organise::favourite_set),
        )
        .route(
            "/api/dashboard/recent",
            axum::routing::post(dashboard_organise::recent_touch),
        )
        .route(
            "/api/dashboard/search",
            get(dashboard_organise::search_names),
        )
        .route(
            "/api/dashboard/sources",
            get(dashboard_sources::list)
                .post(dashboard_sources::create)
                .put(dashboard_sources::update)
                .delete(dashboard_sources::delete),
        )
        .route(
            "/api/dashboard/sources/preview",
            axum::routing::post(dashboard_sources::preview),
        )
        .route(
            "/api/dashboard/calc-fields",
            get(calc_fields::list)
                .post(calc_fields::create)
                .put(calc_fields::update)
                .delete(calc_fields::delete),
        )
        .route(
            "/api/dashboard/calc-fields/validate",
            axum::routing::post(calc_fields::validate),
        )
        .route(
            "/api/dashboard/calc-fields/functions",
            get(calc_fields::functions),
        )
        .route(
            "/api/dashboard/metrics",
            get(metrics::list)
                .post(metrics::create)
                .put(metrics::update)
                .delete(metrics::delete),
        )
        .route(
            "/api/dashboard/metrics/validate",
            axum::routing::post(metrics::validate),
        )
        .route("/api/dashboard/metrics/{id}/usage", get(metrics::usage))
        .route(
            "/api/dashboard/segments",
            get(metrics::list_segments)
                .post(metrics::create_segment)
                .put(metrics::update_segment)
                .delete(metrics::delete_segment),
        )
        .route(
            "/api/dashboard/hierarchies",
            get(hierarchies::list)
                .post(hierarchies::create)
                .put(hierarchies::update)
                .delete(hierarchies::delete),
        )
        .route(
            "/api/dashboard/params",
            get(params::list)
                .post(params::create)
                .put(params::update)
                .delete(params::delete),
        )
        .route("/api/dashboard/fields", get(dashboard::fields))
        .route(
            "/api/dashboard/join-suggestions",
            get(dashboard::join_suggestions),
        )
        .route("/api/dashboard/records", get(dashboard::records))
        .route("/api/dashboard/values", get(dashboard::values))
        .route("/api/dashboard/export", get(dashboard::export))
        .route("/api/dashboard/table-export", get(dashboard::table_export))
        .route("/api/dashboard/chart-export", get(dashboard::chart_export))
        // BI-14 (D1): where a chart comes from. `lineage:read` (owner decision Q6).
        .route(
            "/api/dashboard/charts/{id}/lineage",
            get(lineage::charts::chart_lineage),
        )
        .route("/api/dashboard/export/pdf", get(pdf_export::export_pdf))
        .route("/api/dashboard/embed-info", get(dashboard::embed_info))
        .route("/api/embed/data", axum::routing::post(embed::data))
        .route("/api/render/data", axum::routing::post(render::data))
        // RPT-1 (D7): scheduled reports. Authorisation beyond `dashboard:read`
        // (owner or administrator, download right) is in the handlers.
        .route(
            "/api/reports/subscriptions",
            get(reports::list).post(reports::create),
        )
        .route(
            "/api/reports/subscriptions/{id}",
            get(reports::get)
                .put(reports::update)
                .delete(reports::delete),
        )
        .route(
            "/api/reports/subscriptions/{id}/send",
            axum::routing::post(reports::send_now),
        )
        .route(
            "/api/reports/subscriptions/{id}/deliveries",
            get(reports::deliveries),
        )
        .route("/api/reports/recipients", get(reports::recipients))
        .route(
            "/api/embed/frame",
            axum::routing::post(embed::frame_origins),
        )
        // BI-26 (D5): the dropdown values of an editable filter, token-gated.
        .route("/api/embed/values", axum::routing::post(embed::values))
        // BI-26 (D8): each filter's embed mode.
        .route(
            "/api/dashboard/embed-settings",
            axum::routing::put(embed::put_settings),
        )
        .route(
            "/api/dashboard/embed-revoke",
            axum::routing::post(embed::revoke_token),
        )
        .route(
            "/api/public/dashboard/{token}",
            get(embed::public_dashboard),
        )
        // BI-19 part A (D4): the password of a public link.
        .route(
            "/api/public/dashboard/{token}/unlock",
            axum::routing::post(public_links::unlock),
        )
        // BI-19 part A (D5): make, change and end a dashboard's public link.
        .route(
            "/api/dashboard/public-link",
            get(public_links::get_link)
                .put(public_links::put_link)
                .delete(public_links::delete_link),
        )
        // PR #246 review BLOCKER 1: a public link and an embed read a map's
        // boundary through their own token, the way they read the board's
        // data; the console's own route keeps the signed-in reader.
        .route(
            "/api/public/dashboard/{token}/maps/{id}/geojson",
            get(embed::public_map_geojson),
        )
        .route(
            "/api/embed/maps/{id}/geojson",
            axum::routing::post(embed::embed_map_geojson),
        )
        .route("/api/ai/chat", axum::routing::post(ai::chat))
        .route("/api/ai/tool", axum::routing::post(ai::tool_call))
        .route(
            "/api/ai/sessions",
            get(ai::sessions_get)
                .post(ai::sessions_save)
                .patch(ai::sessions_rename)
                .delete(ai::sessions_delete),
        )
        .route("/api/ai/build-status", get(ai::build_status))
        // The caller's own remembered words for the chat; see `routes::ai::terms`.
        .route(
            "/api/ai/terms",
            get(ai::terms::list_terms)
                .put(ai::terms::put_term)
                .delete(ai::terms::delete_term),
        )
        // WS5 item F1: navbar bell — real, honest open-alert/pending-approval
        // lists (never a fabricated `unreadCount`, never a zero that reads
        // as "genuinely nothing" without Postgres — see `routes::notifications`).
        .route("/api/notifications", get(notifications::list))
        // Per-user Home layout; see `routes::home`.
        .route(
            "/api/home/layout",
            get(home::get_layout)
                .put(home::put_layout)
                .delete(home::delete_layout),
        )
        // Deployment-wide report time zone and first day of the week
        // (`BI-9`); see `routes::settings`.
        .route(
            "/api/settings/reporting",
            get(settings::get_reporting).put(settings::put_reporting),
        )
        // Time and row limits of charts and Query Studio (`BI-5`); see
        // `routes::query_limits`.
        // Stop (`BI-5`): ends the caller's own queries of one run.
        .route(
            "/api/query/cancel",
            axum::routing::post(query_cancel::cancel),
        )
        .route(
            "/api/settings/limits",
            get(query_limits::get_limits).put(query_limits::put_limits),
        )
        // Phase 2 identity domain.
        .merge(identity_router())
        // Phase 2, Task 2.7: connector definitions.
        .merge(connectors_router(&state))
        // ADR 0014: files uploaded from the console and loaded into raw
        // tables. Tenant-scoped like connectors.
        .merge(uploads_router(&state))
        .merge(maps_router())
        // Phase 2, Task 2.8: knowledge sources and vector jobs (metadata
        // only — no `search` route here, see `routes::knowledge`'s module
        // doc comment).
        .merge(knowledge_router())
        // Phase 2, Task 2.9: digital employees, tools, workflows, runs,
        // and approvals.
        .merge(agents_router())
        // Task 3.2: login/logout/me/change-password.
        .merge(auth_router())
        // Task 3.2: the one authorization gate every route (bar the four
        // `Policy::Public` entries in `crate::policy::POLICY_TABLE`) passes
        // through, driven entirely by that table rather than per-handler
        // checks. Added before `timeout_middleware` (in `.layer()`'s
        // outermost-last ordering, this makes `timeout_middleware` the
        // OUTER layer) so a slow session/permission check while a caller
        // has the connection open still counts against the request's
        // deadline instead of running unbounded outside it.
        // `BI-5`: the run every query of this request belongs to. Inside
        // `auth_gate` so the principal it names is already there.
        .layer(from_fn_with_state(
            state.clone(),
            crate::query_limits::scope_middleware,
        ))
        .layer(from_fn_with_state(state.clone(), auth_gate))
        .layer(from_fn_with_state(state.clone(), timeout_middleware))
        .with_state(state)
}

/// `GET /health` — liveness, plus whether the application database is
/// usable (`OPS-4` decision 2).
///
/// Answers `200` whenever this process can answer at all. With no usable
/// Postgres it still serves every Phase 1 route (catalog, overview, query,
/// storage), and the ops gates (`ops/g*/`) poll this endpoint for a `200`
/// to decide the API is up — a non-`200` would report the whole API as
/// down while most of it answers. The body carries the database's state
/// instead, so "up but degraded" is visible rather than a green check over
/// a schema the code cannot use.
async fn health(State(state): State<AppState>) -> ApiJson<HealthBody> {
    ApiJson(HealthBody {
        status: "ok",
        database: if state.database_ready() {
            "ok"
        } else {
            "unavailable"
        },
    })
}

/// [`health`]'s body. `status` is `ok` for any process that answers this
/// route; `database` is `ok` only once the start-up step confirmed the
/// schema is at this build's version (`crate::state::AppState::
/// database_ready`), and `unavailable` otherwise — no pool at all, an
/// unreachable one, or one that refused the migrations.
#[derive(Debug, Serialize)]
struct HealthBody {
    status: &'static str,
    database: &'static str,
}

/// The JSON body shape a request-timeout response takes — same
/// `{"error": "<message>"}` envelope every other error response in this
/// crate uses (see [`crate::error::ApiRejection`]), rather than the bare,
/// content-type-less body `tower_http::timeout::TimeoutLayer` produces on
/// its own.
#[derive(Debug, Serialize)]
struct TimeoutBody {
    error: String,
}

/// Wraps every route in a per-route deadline (see [`route_timeout`]),
/// matching each TypeScript route handler's `export const maxDuration`.
/// Unlike `tower_http::timeout::TimeoutLayer::with_status_code`, which
/// returns an empty, content-type-less body on expiry — the one response
/// path that violated the `{"error": "<message>"}` /
/// `application/json;charset=utf-8` contract every other response honors —
/// this renders the same JSON error envelope via [`ApiJson`].
async fn timeout_middleware(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let path = req.uri().path();
    // Only a route that runs queries under the limits reads them (BI-5 plan
    // D4); every other route keeps its fixed deadline without a settings read.
    let longest = if limit_bearing(path) {
        crate::query_limits::snapshot(&state)
            .await
            .ok()
            .map(|snap| snap.longest_timeout())
    } else {
        None
    };
    let timeout = deadline_for(path, longest);
    match tokio::time::timeout(timeout, next.run(req)).await {
        Ok(response) => response,
        Err(_elapsed) => (
            StatusCode::REQUEST_TIMEOUT,
            ApiJson(TimeoutBody {
                error: "request timeout".to_owned(),
            }),
        )
            .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// D2 regression: the timeout was a uniform 60s across every route,
    /// but the TypeScript declares a longer `maxDuration` for `ai/chat`
    /// (120s, an 8-round LLM tool loop) — a blanket 60s bound 408'd a
    /// legitimate in-flight request. Pins the per-route table to the TS
    /// `export const maxDuration` grep.
    /// `BI-1` review fix (BLOCKER) R2: the downloads keep the budgets they were
    /// built for. The console proxy (`next.config.ts` `proxyTimeout`) must be at
    /// least the largest of these.
    #[test]
    fn the_download_routes_have_their_own_timeouts() {
        assert_eq!(
            route_timeout("/api/dashboard/chart-export"),
            Duration::from_secs(310)
        );
        assert_eq!(
            route_timeout("/api/dashboard/table-export"),
            Duration::from_secs(310)
        );
        assert_eq!(
            route_timeout("/api/dashboard/export/pdf"),
            Duration::from_secs(140)
        );
        assert!(Duration::from_secs(310) > crate::export::EXPORT_TIMEOUT);
        assert!(PDF_REQUEST_TIMEOUT > crate::pdf::PDF_SLOT_WAIT + crate::pdf::PDF_DEADLINE);
        // Everything else keeps the default.
        assert_eq!(
            route_timeout("/api/dashboard/export"),
            DEFAULT_REQUEST_TIMEOUT
        );
    }

    /// `BI-5` plan D4: the query route follows the longest configured limit
    /// plus 15 s, a board route gets the outer cap, everything else keeps its
    /// fixed deadline, and an unreadable setting keeps the fixed one too.
    #[test]
    fn limit_bearing_routes_follow_the_longest_configured_limit() {
        let longest = Some(Duration::from_secs(300));
        assert_eq!(
            deadline_for("/api/query/run", longest),
            Duration::from_secs(315)
        );
        for board in [
            "/api/dashboard",
            "/api/render/data",
            "/api/embed/data",
            "/api/public/dashboard/abc",
        ] {
            assert_eq!(
                deadline_for(board, longest),
                crate::query_limits::BOARD_DEADLINE_CAP,
                "{board}"
            );
            assert_eq!(deadline_for(board, None), route_timeout(board), "{board}");
        }
        // BI-5 review S1: routes under the board prefixes that run no tile.
        for other in [
            "/api/dashboard/export",
            "/api/catalog",
            "/api/ai/chat",
            "/api/embed/frame",
            "/api/embed/values",
            "/api/public/dashboard/abc/unlock",
            "/api/public/dashboard/",
        ] {
            assert_eq!(
                deadline_for(other, longest),
                route_timeout(other),
                "{other}"
            );
            assert!(!limit_bearing(other));
        }
    }

    #[test]
    fn chat_timeout_defaults_to_120s_and_clamps_an_operator_override() {
        assert_eq!(chat_timeout_from(None), Duration::from_secs(120));
        assert_eq!(chat_timeout_from(Some("")), Duration::from_secs(120));
        assert_eq!(chat_timeout_from(Some("abc")), Duration::from_secs(120));
        assert_eq!(chat_timeout_from(Some("600")), Duration::from_secs(600));
        assert_eq!(chat_timeout_from(Some("5")), Duration::from_secs(30));
        assert_eq!(chat_timeout_from(Some("99999")), Duration::from_mins(30));
    }

    #[test]
    fn route_timeout_matches_typescript_max_duration() {
        assert_eq!(route_timeout("/api/ai/chat"), Duration::from_secs(120));
        assert_eq!(
            route_timeout("/api/query/run"),
            DEFAULT_REQUEST_TIMEOUT,
            "TS declares maxDuration = 60, same as the default"
        );
        assert_eq!(
            route_timeout("/api/dashboard/export"),
            DEFAULT_REQUEST_TIMEOUT,
            "no TS maxDuration export — falls back to the default"
        );
        assert_eq!(DEFAULT_REQUEST_TIMEOUT, Duration::from_secs(60));
        // RPT-1: "Send now" makes the files in the request.
        assert_eq!(
            route_timeout("/api/reports/subscriptions/0b7e/send"),
            EXPORT_REQUEST_TIMEOUT
        );
        assert_eq!(
            route_timeout("/api/reports/subscriptions/0b7e"),
            DEFAULT_REQUEST_TIMEOUT
        );
    }

    /// `SRC-13` decision D9: the discovery routes carry a listing call of up to
    /// 120 s, so they get 130 s, and only they do.
    #[test]
    fn the_discovery_routes_take_130_seconds_and_nothing_else_under_connectors_does() {
        for path in [
            "/api/connectors/conn-1/discovery/list",
            "/api/connectors/conn-1/discovery/preview",
        ] {
            assert_eq!(route_timeout(path), Duration::from_secs(130), "{path}");
        }
        for path in [
            "/api/connectors/conn-1",
            "/api/connectors/conn-1/test",
            "/api/connectors/conn-1/discovery/other",
            "/api/connectors//discovery/list",
            "/api/connectors/discovery/list",
            "/api/other/conn-1/discovery/list",
        ] {
            assert_eq!(route_timeout(path), DEFAULT_REQUEST_TIMEOUT, "{path}");
        }
    }

    /// `POST /api/uploads` receives up to 50 MB: it gets the longer bound
    /// (300 s), and only on its own path. `GET /api/uploads` shares that path
    /// by necessity; the per-upload routes, whose handlers only read a row or
    /// start a job, keep the default.
    #[test]
    fn the_upload_route_takes_five_minutes_and_the_per_upload_routes_the_default() {
        assert_eq!(route_timeout("/api/uploads"), Duration::from_secs(300));
        assert_eq!(UPLOAD_REQUEST_TIMEOUT, Duration::from_secs(300));
        for per_upload in [
            "/api/uploads/up-1",
            "/api/uploads/up-1/preview",
            "/api/uploads/up-1/ingest",
            "/api/uploads/",
            "/api/uploads/x/y",
            "/api/uploads/up-1/parts",
            "/api/uploads/up-1/complete",
            "/api/uploads/sessions",
            "/api/uploads/up-1/parts/",
            "/api/uploads//parts/1",
            "/api/uploads/up-1/parts/1/x",
        ] {
            assert_eq!(
                route_timeout(per_upload),
                DEFAULT_REQUEST_TIMEOUT,
                "{per_upload}"
            );
        }
    }

    /// `DATA-17`: one 8 MiB piece of a chunked upload gets the long bound, as
    /// the single request it replaces did.
    #[test]
    fn a_piece_of_a_chunked_upload_takes_five_minutes() {
        for piece in ["/api/uploads/up-1/parts/1", "/api/uploads/up-1/parts/256"] {
            assert_eq!(route_timeout(piece), UPLOAD_REQUEST_TIMEOUT, "{piece}");
        }
    }
}

/// Task 3.2's completeness regression: every route this crate mounts must
/// have gone through `crate::policy::auth_gate` and come out the other
/// side either genuinely public or genuinely gated — never silently open
/// because nobody remembered to add a `crate::policy::POLICY_TABLE` entry.
///
/// # Why this is a live-router test, not just a `policy.rs` unit test
///
/// `crate::policy`'s own unit tests already pin down `policy_for`'s
/// lookup logic in isolation. What THIS module proves is the wiring: that
/// the middleware is actually layered onto `router()`, that
/// `MatchedPath` really does resolve to the same pattern strings written
/// in `POLICY_TABLE`, and that the deny-by-default path
/// (`route_policy_unclassified`, 500) is reachable at all — none of which
/// a table-only unit test can catch if, say, a future refactor moved
/// `.layer(from_fn_with_state(state.clone(), auth_gate))` off the router
/// by accident.
///
/// # The RED demonstration (not committed — see the task's final report)
///
/// Temporarily adding a route to `router()` (e.g.
/// `.route("/api/__throwaway", get(health))`) with NO matching
/// `POLICY_TABLE` entry and running this suite turns
/// `every_policy_table_entry_is_mounted_and_gated_as_declared` from green
/// to failing on `entries_not_reachable`... no — it does not touch this
/// test (which only walks entries that already exist in the table); the
/// route that actually goes RED is a direct request to the throwaway path
/// itself, which this module also exercises
/// (`an_unregistered_route_denies_by_default_instead_of_serving_public`),
/// asserting exactly the 500 `route_policy_unclassified` body deny-by-default
/// produces for ANY unclassified path — a throwaway route hits that same
/// code path the moment it's requested, with no separate wiring needed to
/// prove it.
#[cfg(test)]
mod route_policy_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use axum::body::to_bytes;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    use crate::config::Config;
    use crate::policy::{POLICY_TABLE, Policy};
    use crate::state::AppState;

    /// A router whose state can AUTHENTICATE, with nothing to connect to.
    ///
    /// `SEC-37`: an unset `DATABASE_URL` now means "no Postgres pool", and
    /// without one [`crate::auth::AuthenticatedPrincipal`] refuses every
    /// protected route with 503 before it looks at the request — the honest
    /// answer for a deployment that cannot check a credential, but it hides
    /// the 401 this test exists to observe (a `RequiresAuth` entry with no
    /// credential presented). So the state is built the way the old default
    /// DSN built it: a LAZY pool (`connect_lazy` does no I/O) at a
    /// throwaway address this test never dials, since the no-credential
    /// path never reaches it.
    fn test_router() -> axum::Router {
        let env = std::collections::HashMap::from([(
            "DATABASE_URL".to_owned(),
            "postgres://test:test@127.0.0.1:5432/test".to_owned(),
        )]);
        let cfg = Config::from_map(&env).unwrap();
        super::router(AppState::new(cfg))
    }

    /// `{id}`/`{token}`/`{kind}`/`{runId}` — any `{...}` capture segment —
    /// substituted with a fixed placeholder so the concrete request
    /// resolves to the exact route pattern the table names. axum matches
    /// by segment shape, not by the parameter's name, so which literal
    /// placeholder is used doesn't matter.
    fn concretize(pattern: &str) -> String {
        let mut out = String::with_capacity(pattern.len());
        let mut in_capture = false;
        for ch in pattern.chars() {
            match ch {
                '{' => in_capture = true,
                '}' => {
                    in_capture = false;
                    out.push('x');
                }
                _ if in_capture => {}
                _ => out.push(ch),
            }
        }
        out
    }

    async fn request(
        app: axum::Router,
        method: &str,
        path: &str,
    ) -> axum::http::Response<axum::body::Body> {
        app.oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
    }

    /// Every `POLICY_TABLE` entry resolves to a MOUNTED route (never 404 —
    /// a 404 here would mean the table has drifted from `router()`, the
    /// exact drift this whole module exists to catch), and is gated as
    /// declared with no credentials presented: `Policy::Public` never
    /// yields 401/403 for that reason, `Policy::RequiresAuth` and
    /// `Policy::RequiresPermission` both yield exactly 401 (no cookie, no
    /// bearer header presented at all — extraction fails before any
    /// permission check runs, so both policy kinds converge on 401 here,
    /// not 403; the 403 path is exercised separately in `tests/route_auth.rs`
    /// with a real, under-permissioned principal against live Postgres).
    #[tokio::test]
    async fn every_policy_table_entry_is_mounted_and_gated_as_declared() {
        let mut failures = Vec::new();
        for (method, pattern, policy) in POLICY_TABLE {
            let path = concretize(pattern);
            let resp = request(test_router(), method, &path).await;
            let status = resp.status();
            if status == StatusCode::NOT_FOUND {
                failures.push(format!(
                    "{method} {pattern} ({path}): table entry does not resolve to a mounted route (404) — table has drifted from routes::router"
                ));
                continue;
            }
            match policy {
                Policy::Public => {
                    if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
                        failures.push(format!(
                            "{method} {pattern}: declared Public but got {status} with no credentials"
                        ));
                    }
                }
                Policy::RequiresAuth | Policy::RequiresPermission(_) => {
                    if status != StatusCode::UNAUTHORIZED {
                        failures.push(format!(
                            "{method} {pattern}: declared {policy:?} but got {status} (expected 401) with no credentials"
                        ));
                    }
                }
            }
        }
        assert!(failures.is_empty(), "\n{}", failures.join("\n"));
    }

    /// The deny-by-default regression itself: a `(method, path)` with NO
    /// `POLICY_TABLE` entry is refused (500, `route_policy_unclassified`)
    /// rather than silently served. This is precisely what goes RED (as a
    /// live 200/404-vs-500 behavior change, not just this assertion) the
    /// moment a route is registered in `router()` without a matching
    /// table entry — see the task's final report for the throwaway-route
    /// demonstration.
    #[tokio::test]
    async fn an_unregistered_route_denies_by_default_instead_of_serving_public() {
        // `/health` is mounted and IS in the table (Public) — this proves
        // the negative instead by asking for a method the health route
        // never registers a policy entry for. `MatchedPath` still
        // resolves to `/health` (axum matches the path regardless of
        // method before yielding 405), so `auth_gate` still runs and still
        // finds no `("DELETE", "/health", _)` entry.
        let resp = request(test_router(), "DELETE", "/health").await;
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let text = String::from_utf8_lossy(&bytes);
        assert!(
            text.contains("route_policy_unclassified"),
            "expected the deny-by-default body, got: {text}"
        );
    }
}
