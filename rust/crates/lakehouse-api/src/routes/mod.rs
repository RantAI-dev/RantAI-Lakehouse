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
mod authored_pipelines;
mod catalog;
mod catalog_governance;
mod catalog_profile;
mod catalog_query;
mod catalog_search;
mod catalog_source;
mod connectors;
mod dashboard;
mod dashboard_folders;
mod dashboard_sources;
mod embed;
mod gold;
mod governance;
mod home;
mod identity;
mod knowledge;
mod lakehouse;
mod lineage;
mod notifications;
mod ops;
mod overview;
mod pipelines;
mod quality;
mod query;
pub(crate) mod schema_versions;
mod storage;
pub(crate) mod support;
mod uploads;

use std::time::Duration;

use axum::Router;
use axum::extract::{DefaultBodyLimit, Request};
use axum::http::StatusCode;
use axum::middleware::{Next, from_fn, from_fn_with_state};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
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

/// Per-route request timeout, mirroring each TypeScript handler's `export
/// const maxDuration` (grep of every `src/app/api/**/route.ts`):
///
/// | route                          | TS `maxDuration` |
/// |---------------------------------|------------------|
/// | `/api/ai/chat`                  | 120              |
/// | `/api/alerts/run`               | 60               |
/// | `/api/query/run`                | 60               |
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
        "/api/uploads" => UPLOAD_REQUEST_TIMEOUT,
        _ => DEFAULT_REQUEST_TIMEOUT,
    }
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
        .route("/api/pipelines/runnable", get(authored_pipelines::runnable))
        .route(
            "/api/pipelines/{id}",
            get(pipelines::detail)
                .put(authored_pipelines::update)
                .delete(authored_pipelines::delete),
        )
        .route(
            "/api/pipelines/{id}/schedule-ticks",
            get(authored_pipelines::schedule_ticks),
        )
        .route("/api/pipelines/{id}/source", get(pipelines::source))
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

/// The `/api/connectors/*` sub-router (Task 2.7), split out for the same
/// `clippy::too_many_lines` reason as [`pipelines_router`].
///
/// Every `/api/connectors/{id}/*` route sits behind
/// [`connectors::require_connector_in_tenants`]: the caller must belong to
/// the connector's tenant. `PUT .../tenant` stays outside it — it is
/// identity administration (`identity:write`), and the only way a
/// connector with no tenant gets one.
fn connectors_router(state: &AppState) -> Router<AppState> {
    let per_connector = Router::new()
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
        .route(
            "/api/connectors/{id}/discover",
            axum::routing::post(connectors::discover),
        )
        .route(
            "/api/connectors/{id}/debezium-properties",
            get(connectors::debezium_properties),
        )
        .route(
            "/api/connectors/{id}/ingest-spec",
            get(connectors::ingest_spec_get).put(connectors::ingest_spec_put),
        )
        .route(
            "/api/connectors/{id}/ingest/run",
            axum::routing::post(connectors::ingest_run),
        )
        .route(
            "/api/connectors/{id}/ingest/runs",
            get(connectors::ingest_run_history),
        )
        .route_layer(from_fn_with_state(
            state.clone(),
            connectors::require_connector_in_tenants,
        ));
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
        // The assignment route for connector
        // rows `0042_tenant_provisioning.sql` leaves `tenant_id = NULL`.
        .route(
            "/api/connectors/{id}/tenant",
            axum::routing::put(connectors::assign_connector_tenant),
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
        .route(
            "/api/uploads/{id}/ingest",
            axum::routing::post(uploads::ingest),
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
        .merge(per_upload)
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
        .route("/api/dashboard/fields", get(dashboard::fields))
        .route("/api/dashboard/records", get(dashboard::records))
        .route("/api/dashboard/values", get(dashboard::values))
        .route("/api/dashboard/export", get(dashboard::export))
        .route("/api/dashboard/embed-info", get(dashboard::embed_info))
        .route("/api/embed/data", axum::routing::post(embed::data))
        .route(
            "/api/public/dashboard/{token}",
            get(embed::public_dashboard),
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
        // Phase 2 identity domain.
        .merge(identity_router())
        // Phase 2, Task 2.7: connector definitions.
        .merge(connectors_router(&state))
        // ADR 0014: files uploaded from the console and loaded into raw
        // tables. Tenant-scoped like connectors.
        .merge(uploads_router(&state))
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
        .layer(from_fn_with_state(state.clone(), auth_gate))
        .layer(from_fn(timeout_middleware))
        .with_state(state)
}

/// `GET /health` — a plain liveness check, no dependencies.
async fn health() -> &'static str {
    "ok"
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
async fn timeout_middleware(req: Request, next: Next) -> Response {
    let timeout = route_timeout(req.uri().path());
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
    #[test]
    fn chat_timeout_defaults_to_120s_and_clamps_an_operator_override() {
        assert_eq!(chat_timeout_from(None), Duration::from_secs(120));
        assert_eq!(chat_timeout_from(Some("")), Duration::from_secs(120));
        assert_eq!(chat_timeout_from(Some("abc")), Duration::from_secs(120));
        assert_eq!(chat_timeout_from(Some("600")), Duration::from_secs(600));
        assert_eq!(chat_timeout_from(Some("5")), Duration::from_secs(30));
        assert_eq!(chat_timeout_from(Some("99999")), Duration::from_secs(1800));
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
        ] {
            assert_eq!(
                route_timeout(per_upload),
                DEFAULT_REQUEST_TIMEOUT,
                "{per_upload}"
            );
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

    fn test_router() -> axum::Router {
        let cfg = Config::from_map(&std::collections::HashMap::new()).unwrap();
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
