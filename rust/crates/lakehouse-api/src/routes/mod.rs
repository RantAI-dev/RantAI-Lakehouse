//! Route mounting.
//!
//! Mounts the health check, the five read-only domains (catalog, overview,
//! ops, governance, storage), the write-side domains (alerts, query, agent,
//! dashboard, ...), and — new in Phase 2 — the Postgres-backed `identity`
//! domain under `/api/identity/*`.

mod agent;
mod ai;
mod alerts;
mod catalog;
mod dashboard;
mod embed;
mod governance;
mod identity;
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
use axum::middleware::{Next, from_fn};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use serde::Serialize;

use crate::json::ApiJson;
use crate::state::AppState;

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
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/api/catalog", get(catalog::list))
        .route("/api/catalog/{id}", get(catalog::detail))
        .route("/api/overview", get(overview::get).post(overview::refresh))
        .route("/api/ops/{kind}", get(ops::get))
        .route("/api/governance/lineage", get(governance::lineage))
        .route("/api/governance/{kind}", get(governance::get))
        .route("/api/storage", get(storage::get))
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
        .route("/api/pipelines", get(pipelines::list))
        .route("/api/pipelines/{id}/runs", get(pipelines::runs))
        .route(
            "/api/pipelines/{id}/trigger",
            axum::routing::post(pipelines::trigger),
        )
        .route("/api/dashboard", get(dashboard::get))
        .route(
            "/api/dashboard/specs",
            get(dashboard::specs_list)
                .post(dashboard::specs_create)
                .put(dashboard::specs_update)
                .delete(dashboard::specs_delete),
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
        // Phase 2 identity domain. Grouped under a single `/api/identity`
        // namespace rather than four top-level nouns (`/api/users`,
        // `/api/tenants`, ...): every Phase 1 route is already
        // `/api/<domain>[/<sub>]` (`/api/governance/{kind}`,
        // `/api/dashboard/specs`, `/api/alerts/run`), the console's
        // `identityService` is a single contract, and top-level `/api/users`
        // would be the first route whose path says nothing about which
        // domain owns it. Collection paths are plural nouns with GET =
        // list and POST = create, per REST.
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
