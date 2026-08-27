//! `lakehouse-api` — the axum HTTP service for the `RantAI` Lakehouse
//! backend.
//!
//! This binary owns process-level concerns (config resolution, logging
//! setup, binding, graceful shutdown) that don't belong in a library crate.
//! `anyhow` is used here, and only here, for that reason — library crates
//! (`lakehouse-core`, `lakehouse-clickhouse`, and this crate's own modules)
//! use `thiserror` typed errors instead.

mod config;
mod dagster;
mod error;
mod routes;
mod state;

use anyhow::Context;
use tracing_subscriber::EnvFilter;

use config::Config;
use state::AppState;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .json()
        .init();

    let config = Config::from_env().context("failed to resolve configuration from environment")?;
    let port = config.port;
    let state = AppState::new(config);
    let app = routes::router(state);

    let addr = format!("0.0.0.0:{port}");
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .with_context(|| format!("failed to bind {addr}"))?;
    tracing::info!(%addr, "lakehouse-api listening");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("server error")?;

    Ok(())
}

/// Resolves once ctrl-c is received, so `axum::serve` can shut down
/// gracefully.
async fn shutdown_signal() {
    // `unwrap`/`expect` are denied outside tests; a failure to install the
    // ctrl-c handler is logged and treated as "never shuts down early"
    // rather than panicking the process.
    if let Err(err) = tokio::signal::ctrl_c().await {
        tracing::error!(%err, "failed to install ctrl-c handler");
        std::future::pending::<()>().await;
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use axum::body::to_bytes;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    use super::*;

    #[tokio::test]
    async fn health_returns_200_ok() {
        let cfg = Config::from_map(&std::collections::HashMap::new()).unwrap();
        let app = routes::router(AppState::new(cfg));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(&bytes[..], b"ok");
    }

    /// Build a fresh router for a registration test. Each test gets its own
    /// `Router` (routers aren't `Clone`-shared across `oneshot` calls here)
    /// so tests can run concurrently without interfering.
    fn test_router() -> axum::Router {
        let cfg = Config::from_map(&std::collections::HashMap::new()).unwrap();
        routes::router(AppState::new(cfg))
    }

    async fn get(app: axum::Router, uri: &str) -> axum::http::Response<axum::body::Body> {
        app.oneshot(
            Request::builder()
                .uri(uri)
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
    }

    /// Registration tests: no live `ClickHouse` is required to prove a
    /// route is *mounted* — a 503 (data-layer error) or 200 both prove
    /// that, a 404 does not. These intentionally don't assert response
    /// bodies; behavior-level fidelity is the parity harness's job (see
    /// `rust/tests/parity`).
    #[tokio::test]
    async fn catalog_list_route_is_registered() {
        let resp = get(test_router(), "/api/catalog").await;
        assert_ne!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn catalog_detail_route_is_registered() {
        let resp = get(test_router(), "/api/catalog/some-slug").await;
        assert_ne!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn storage_route_is_registered() {
        let resp = get(test_router(), "/api/storage").await;
        assert_ne!(resp.status(), StatusCode::NOT_FOUND);
    }

    async fn post(app: axum::Router, uri: &str) -> axum::http::Response<axum::body::Body> {
        app.oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn overview_get_route_is_registered() {
        let resp = get(test_router(), "/api/overview").await;
        assert_ne!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn overview_post_route_is_registered() {
        let resp = post(test_router(), "/api/overview").await;
        assert_ne!(resp.status(), StatusCode::NOT_FOUND);
    }
}
