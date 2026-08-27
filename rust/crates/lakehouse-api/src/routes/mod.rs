//! Route mounting.
//!
//! Write-side routes (query/agent/dashboard/alerts/...) land in later
//! tasks; this chassis mounts the health check plus the five read-only
//! domains (catalog, overview, ops, governance, storage).

mod catalog;
mod governance;
mod ops;
mod overview;
mod storage;
mod support;

use axum::Router;
use axum::routing::get;

use crate::state::AppState;

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
        .with_state(state)
}

/// `GET /health` — a plain liveness check, no dependencies.
async fn health() -> &'static str {
    "ok"
}
