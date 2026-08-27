//! Route mounting.
//!
//! Write-side routes (query/agent/dashboard/alerts/...) and governance land
//! in later commits; this mounts the health check plus catalog, storage,
//! overview, and ops.

mod catalog;
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
        .route("/api/storage", get(storage::get))
        .with_state(state)
}

/// `GET /health` — a plain liveness check, no dependencies.
async fn health() -> &'static str {
    "ok"
}
