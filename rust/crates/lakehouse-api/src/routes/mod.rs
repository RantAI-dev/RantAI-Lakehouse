//! Route mounting.
//!
//! Mounts the health check, the five read-only domains (catalog, overview,
//! ops, governance, storage), the write-side domains (alerts, query, agent,
//! dashboard, ...), and — new in Phase 2 — the Postgres-backed `identity`
//! domain under `/api/identity/*`.

mod agent;
mod agents;
mod ai;
mod alerts;
pub mod auth;
mod catalog;
mod connectors;
mod dashboard;
mod embed;
mod governance;
mod identity;
mod knowledge;
mod ops;
mod overview;
mod pipelines;
mod query;
mod storage;
mod support;

use std::time::Duration;

use axum::Router;
use axum::extract::Request;
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
}

/// Default per-request timeout, used for every route whose TypeScript
/// handler does not declare `export const maxDuration` (most of them — see
/// [`route_timeout`]).
const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

/// Per-route request timeout, mirroring each TypeScript handler's `export
/// const maxDuration` (grep of every `src/app/api/**/route.ts`):
///
/// | route                          | TS `maxDuration` |
/// |---------------------------------|------------------|
/// | `/api/ai/chat`                  | 120              |
/// | `/api/agent/query`              | 90               |
/// | `/api/agent/ask`                | 60               |
/// | `/api/agent/text-to-sql`        | 60               |
/// | `/api/alerts/run`               | 60               |
/// | `/api/query/run`                | 60               |
/// | everything else (no export)     | [`DEFAULT_REQUEST_TIMEOUT`] (60) |
///
/// The timeout is NOT uniform in the TypeScript — `ai/chat`'s 120s and
/// `agent/query`'s 90s cover legitimate multi-round LLM tool loops that a
/// blanket 60s bound would 408 mid-flight. Matched on `req.uri().path()`
/// before route dispatch, so path params (`/api/catalog/{id}`, ...) never
/// need to appear here — none of the parameterized routes declare a
/// non-default `maxDuration` today.
fn route_timeout(path: &str) -> Duration {
    match path {
        "/api/ai/chat" => Duration::from_secs(120),
        "/api/agent/query" => Duration::from_secs(90),
        _ => DEFAULT_REQUEST_TIMEOUT,
    }
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
        .route("/api/pipelines/{id}/runs", get(pipelines::runs))
        .route(
            "/api/pipelines/{id}/trigger",
            axum::routing::post(pipelines::trigger),
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
}

/// The `/api/connectors/*` sub-router (Task 2.7), split out for the same
/// `clippy::too_many_lines` reason as [`pipelines_router`].
fn connectors_router() -> Router<AppState> {
    Router::new()
        .route(
            "/api/connectors",
            get(connectors::list).post(connectors::create),
        )
        .route("/api/connectors/{id}", get(connectors::detail))
        .route(
            "/api/connectors/{id}/test",
            axum::routing::post(connectors::test_connection),
        )
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
        .route(
            "/api/identity/workspace-settings",
            get(identity::workspace_settings),
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
            "/api/agents/tools",
            get(agents::list_tools).post(agents::register_tool),
        )
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
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/api/catalog", get(catalog::list))
        .route("/api/catalog/{id}", get(catalog::detail))
        .route("/api/overview", get(overview::get).post(overview::refresh))
        .merge(overview_alerts_router())
        .route("/api/ops/{kind}", get(ops::get))
        .route(
            "/api/ops/workloads/{id}/cancel",
            axum::routing::post(ops::cancel_workload),
        )
        .route("/api/governance/lineage", get(governance::lineage))
        .route(
            "/api/governance/policies",
            get(governance::list_policies).post(governance::create_policy),
        )
        .route(
            "/api/governance/{kind}",
            get(governance::get).post(governance::create_rule),
        )
        .merge(storage_router())
        .route(
            "/api/alerts",
            get(alerts::list)
                .post(alerts::create)
                .put(alerts::update)
                .delete(alerts::delete),
        )
        .route("/api/alerts/run", get(alerts::run).post(alerts::run))
        .route("/api/query/run", axum::routing::post(query::run))
        .route("/api/query/estimate", axum::routing::post(query::estimate))
        .route("/api/query/saved", get(query::list_saved))
        .route("/api/query/history", get(query::list_history))
        .route(
            "/api/query/collaboration",
            get(query::list_collaboration).post(query::create_collaboration_project),
        )
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
        .route("/api/agent/ask", axum::routing::post(agent::ask))
        .route("/api/agent/query", axum::routing::post(agent::query))
        .route(
            "/api/agent/text-to-sql",
            axum::routing::post(agent::text_to_sql),
        )
        .route("/api/ai/chat", axum::routing::post(ai::chat))
        .route(
            "/api/ai/sessions",
            get(ai::sessions_get)
                .post(ai::sessions_save)
                .delete(ai::sessions_delete),
        )
        .route("/api/ai/build-status", get(ai::build_status))
        // Phase 2 identity domain.
        .merge(identity_router())
        // Phase 2, Task 2.7: connector definitions.
        .merge(connectors_router())
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
    /// but the TypeScript declares longer `maxDuration`s for `ai/chat`
    /// (120s, an 8-round LLM tool loop) and `agent/query` (90s) — a
    /// blanket 60s bound 408'd a legitimate in-flight request. Pins the
    /// per-route table to the TS `export const maxDuration` grep.
    #[test]
    fn route_timeout_matches_typescript_max_duration() {
        assert_eq!(route_timeout("/api/ai/chat"), Duration::from_secs(120));
        assert_eq!(route_timeout("/api/agent/query"), Duration::from_secs(90));
        assert_eq!(
            route_timeout("/api/agent/ask"),
            DEFAULT_REQUEST_TIMEOUT,
            "TS declares maxDuration = 60, same as the default"
        );
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
