//! `lakehouse-api` — the axum HTTP service for the `RantAI` Lakehouse
//! backend.
//!
//! This binary owns process-level concerns (config resolution, logging
//! setup, binding, graceful shutdown) that don't belong in a library crate.
//! `anyhow` is used here, and only here, for that reason — library crates
//! (`lakehouse-core`, `lakehouse-clickhouse`, and this crate's own modules)
//! use `thiserror` typed errors instead.

mod auth;
mod config;
mod error;
mod json;
mod policy;
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

    // Task 3.2: idempotently seed the one bootstrap admin, from
    // `AUTH_BOOTSTRAP_EMAIL`/`AUTH_BOOTSTRAP_PASSWORD` — see
    // `bootstrap_admin`'s doc comment for what happens when either is unset.
    bootstrap_admin(&state).await;

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

/// Idempotently seed exactly one bootstrap admin account from
/// `AUTH_BOOTSTRAP_EMAIL`/`AUTH_BOOTSTRAP_PASSWORD`
/// ([`Config::auth_bootstrap_email`]/[`Config::auth_bootstrap_password`]),
/// so a fresh deployment has a way in before any other account exists.
///
/// # Idempotent by construction, not by a pre-check
///
/// This does not check "does the bootstrap admin already exist" before
/// inserting — it just tries the insert and treats the natural outcome of
/// re-running it (`app_user.email` already taken, surfaced as
/// [`lakehouse_store::StoreError::Conflict`] from
/// [`lakehouse_store::identity::create_user`]) as "already seeded, nothing
/// to do", logged at `info`. Every other failure logs at `error` and
/// leaves the process to boot anyway — a broken bootstrap-admin seed must
/// not stop [`main`] from serving the rest of the API.
///
/// # What happens when the env vars are absent
///
/// Logs a `tracing::warn!` naming exactly which two variables to set and
/// returns, seeding nothing. Never a hardcoded fallback credential — an
/// unset bootstrap admin means "no way in yet", not "log in with a value
/// nobody typed".
///
/// The created identity is `must_change_password = true` (see
/// `0019_auth.sql`/[`lakehouse_auth::password`]'s doc comments): it can log
/// in, but [`crate::routes::auth::change_password`] refuses to let it stay
/// on that credential without proving a fresh password client-side.
async fn bootstrap_admin(state: &AppState) {
    let (Some(email), Some(password)) = (
        state.config.auth_bootstrap_email.clone(),
        state.config.auth_bootstrap_password.clone(),
    ) else {
        tracing::warn!(
            "no bootstrap admin configured: set AUTH_BOOTSTRAP_EMAIL and \
             AUTH_BOOTSTRAP_PASSWORD to create one on next startup"
        );
        return;
    };
    let Some(pool) = state.pg.as_deref() else {
        tracing::warn!("cannot seed bootstrap admin: no Postgres pool is configured");
        return;
    };

    let input = lakehouse_store::identity::InviteUserInput {
        name: "Bootstrap Admin".to_owned(),
        email,
        roles: vec!["Platform Admin".to_owned()],
        tenants: Vec::new(),
    };
    let user = match lakehouse_store::identity::create_user(pool, &input).await {
        Ok(user) => user,
        Err(lakehouse_store::StoreError::Conflict) => {
            tracing::info!("bootstrap admin already exists; nothing to seed");
            return;
        }
        Err(err) => {
            tracing::error!(%err, "failed to seed bootstrap admin");
            return;
        }
    };

    let Ok(user_id) = user.id.parse() else {
        tracing::error!("bootstrap admin was created but its id did not parse as a UUID");
        return;
    };
    let password = lakehouse_auth::Secret::new(password);
    if let Err(err) =
        lakehouse_auth::password::create_local_identity(pool, user_id, &password, true).await
    {
        tracing::error!(%err, "bootstrap admin user was created but its password identity was not");
        return;
    }
    tracing::info!("bootstrap admin seeded; must_change_password = true");
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

    /// Registration tests: no live `ClickHouse`/`Dagster` is required to
    /// prove a route is *mounted* — a 503 (data-layer error) or 200 both
    /// prove that, a 404 does not. These intentionally don't assert
    /// response bodies; behavior-level fidelity is the parity harness's
    /// job (see `rust/tests/parity`).
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
    async fn overview_get_route_is_registered() {
        let resp = get(test_router(), "/api/overview").await;
        assert_ne!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn overview_post_route_is_registered() {
        let resp = post(test_router(), "/api/overview").await;
        assert_ne!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn ops_kind_route_is_registered() {
        let resp = get(test_router(), "/api/ops/workloads").await;
        assert_ne!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn governance_kind_route_is_registered() {
        let resp = get(test_router(), "/api/governance/quality").await;
        assert_ne!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn governance_lineage_route_is_registered() {
        let resp = get(test_router(), "/api/governance/lineage").await;
        assert_ne!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn storage_route_is_registered() {
        let resp = get(test_router(), "/api/storage").await;
        assert_ne!(resp.status(), StatusCode::NOT_FOUND);
    }

    /// The routing subtlety this task calls out explicitly: axum must match
    /// the static `/api/governance/lineage` segment before the
    /// `/api/governance/{kind}` capture, so a request for `lineage` reaches
    /// [`routes::governance::lineage`] and never the `{kind}` dispatch
    /// (which would otherwise treat `"lineage"` as an unrecognized kind and
    /// reply `400 {"error": "kind tak dikenal: lineage"}`).
    ///
    /// This holds regardless of `ClickHouse` availability: the `{kind}`
    /// dispatch's unknown-kind branch never touches the network and always
    /// replies 400 with that exact message; the `lineage` handler never
    /// produces a 400 or that message under any failure mode (network
    /// failures there 503, not 400). So this check is a reliable proxy for
    /// "did the router send this to the right handler" even without a live
    /// `ClickHouse`.
    #[tokio::test]
    async fn query_run_route_is_registered() {
        let resp = post(test_router(), "/api/query/run").await;
        assert_ne!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn query_estimate_route_is_registered() {
        let resp = post(test_router(), "/api/query/estimate").await;
        assert_ne!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn pipelines_list_route_is_registered() {
        let resp = get(test_router(), "/api/pipelines").await;
        assert_ne!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn pipelines_runs_route_is_registered() {
        let resp = get(test_router(), "/api/pipelines/refresh_lakehouse/runs").await;
        assert_ne!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn pipelines_trigger_route_is_registered() {
        let resp = post(test_router(), "/api/pipelines/refresh_lakehouse/trigger").await;
        assert_ne!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn dashboard_get_route_is_registered() {
        let resp = get(test_router(), "/api/dashboard").await;
        assert_ne!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn dashboard_specs_route_is_registered() {
        let resp = get(test_router(), "/api/dashboard/specs").await;
        assert_ne!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn dashboard_boards_route_is_registered() {
        let resp = get(test_router(), "/api/dashboard/boards").await;
        assert_ne!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn dashboard_fields_route_is_registered() {
        let resp = get(test_router(), "/api/dashboard/fields").await;
        assert_ne!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn dashboard_records_route_is_registered() {
        let resp = get(test_router(), "/api/dashboard/records").await;
        assert_ne!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn dashboard_values_route_is_registered() {
        let resp = get(test_router(), "/api/dashboard/values").await;
        assert_ne!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn dashboard_export_route_is_registered() {
        let resp = get(test_router(), "/api/dashboard/export").await;
        assert_ne!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn dashboard_embed_info_route_is_registered() {
        let resp = get(test_router(), "/api/dashboard/embed-info").await;
        assert_ne!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn embed_data_route_is_registered() {
        let resp = post(test_router(), "/api/embed/data").await;
        assert_ne!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn public_dashboard_route_is_registered() {
        let resp = get(test_router(), "/api/public/dashboard/some-token").await;
        assert_ne!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn agent_ask_route_is_registered() {
        let resp = post(test_router(), "/api/agent/ask").await;
        assert_ne!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn agent_query_route_is_registered() {
        let resp = post(test_router(), "/api/agent/query").await;
        assert_ne!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn agent_text_to_sql_route_is_registered() {
        let resp = post(test_router(), "/api/agent/text-to-sql").await;
        assert_ne!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn ai_chat_route_is_registered() {
        let resp = post(test_router(), "/api/ai/chat").await;
        assert_ne!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn ai_sessions_route_is_registered() {
        let resp = get(test_router(), "/api/ai/sessions").await;
        assert_ne!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn ai_build_status_route_is_registered() {
        let resp = get(test_router(), "/api/ai/build-status").await;
        assert_ne!(resp.status(), StatusCode::NOT_FOUND);
    }

    /// The final tally this task calls out explicitly: all 30 route paths
    /// (8 pre-existing + alerts×2 + the 21 ported in this task) must be
    /// mounted and reachable (concrete path segments substituted for
    /// captures so the request actually resolves to a handler, not just
    /// "some path"). A tripwire against silently dropping a route during a
    /// future refactor of `routes::router` — every path here must yield a
    /// non-404 status, and the list itself must have exactly 30 entries.
    #[tokio::test]
    async fn router_exposes_all_thirty_route_paths() {
        const EXPECTED_PATHS: [&str; 30] = [
            "/api/catalog",
            "/api/catalog/some-id",
            "/api/overview",
            "/api/ops/workloads",
            "/api/governance/lineage",
            "/api/governance/quality",
            "/api/storage",
            "/api/alerts",
            "/api/alerts/run",
            "/api/query/run",
            "/api/query/estimate",
            "/api/pipelines",
            "/api/pipelines/refresh_lakehouse/runs",
            "/api/pipelines/refresh_lakehouse/trigger",
            "/api/dashboard",
            "/api/dashboard/specs",
            "/api/dashboard/boards",
            "/api/dashboard/fields",
            "/api/dashboard/records",
            "/api/dashboard/values",
            "/api/dashboard/export",
            "/api/dashboard/embed-info",
            "/api/embed/data",
            "/api/public/dashboard/some-token",
            "/api/agent/ask",
            "/api/agent/query",
            "/api/agent/text-to-sql",
            "/api/ai/chat",
            "/api/ai/sessions",
            "/api/ai/build-status",
        ];
        assert_eq!(EXPECTED_PATHS.len(), 30);
        for uri in EXPECTED_PATHS {
            // GET is enough to prove a path is mounted: axum returns 405
            // Method Not Allowed (not 404) for a registered path hit with
            // the wrong verb, so this still distinguishes "unmounted" from
            // "mounted but wrong method" for the POST-only routes.
            let resp = get(test_router(), uri).await;
            assert_ne!(
                resp.status(),
                StatusCode::NOT_FOUND,
                "path not mounted: {uri}"
            );
        }
    }

    #[tokio::test]
    async fn governance_lineage_route_does_not_fall_through_to_kind_dispatch() {
        let resp = get(test_router(), "/api/governance/lineage").await;
        assert_ne!(resp.status(), StatusCode::BAD_REQUEST);
        let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let text = String::from_utf8_lossy(&bytes);
        assert!(
            !text.contains("kind tak dikenal"),
            "lineage route fell through to the {{kind}} dispatch: {text}"
        );
    }
}
