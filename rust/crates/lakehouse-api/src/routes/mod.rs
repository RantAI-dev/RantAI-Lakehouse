//! Route mounting.
//!
//! Mounts the health check, the five read-only domains (catalog, overview,
//! ops, governance, storage), and `alerts` (the first write-side domain).
//! Remaining write-side routes (query/agent/dashboard/...) land in later
//! tasks.

mod alerts;
mod catalog;
mod governance;
mod ops;
mod overview;
mod storage;
mod support;

use std::time::Duration;

use axum::Router;
use axum::http::StatusCode;
use axum::routing::get;
use tower_http::timeout::TimeoutLayer;

use crate::state::AppState;

/// Per-request timeout, matching `export const maxDuration = 60` on the
/// TypeScript route handlers (e.g. `src/app/api/query/run/route.ts:5`).
/// Next.js enforces that as a platform-level function deadline; here it's a
/// `tower_http` middleware layer wrapping every route — different
/// mechanism, same bound, so no request can hang the service past 60s.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

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
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            REQUEST_TIMEOUT,
        ))
        .with_state(state)
}

/// `GET /health` — a plain liveness check, no dependencies.
async fn health() -> &'static str {
    "ok"
}
