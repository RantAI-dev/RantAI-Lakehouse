//! Route mounting.
//!
//! Real data routes land in later tasks; this chassis only mounts the
//! health check.

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
        .with_state(state)
}

/// `GET /health` — a plain liveness check, no dependencies.
async fn health() -> &'static str {
    "ok"
}
