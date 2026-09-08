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
mod connector_deprovision;
mod connector_probe;
mod error;
mod gold_export;
mod gold_lock;
mod json;
mod policy;
mod routes;
mod state;
mod tenant;

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

    // Bring the OLTP schema up to date before serving. `migrate` is idempotent
    // (`sqlx` records applied versions in `_sqlx_migrations`), so this is safe
    // on every boot.
    //
    // A failure is logged rather than fatal, matching the reasoning on
    // `AppState::pg`: the Phase 1 routes (catalog, overview, query, storage)
    // only need ClickHouse and Dagster, and an unreachable Postgres must not
    // take the whole console down. The Phase 2 routes then degrade to 503 via
    // `StoreError::Unavailable` instead of serving an empty schema.
    if let Some(pool) = state.pg.as_ref() {
        match lakehouse_store::migrate(pool).await {
            Ok(()) => tracing::info!("database migrations applied"),
            Err(err) => tracing::error!(%err, "failed to apply database migrations"),
        }
    } else {
        tracing::warn!("no Postgres pool configured; Phase 2 routes will be unavailable");
    }

    // Task 3.2: idempotently seed the one bootstrap admin, from
    // `AUTH_BOOTSTRAP_EMAIL`/`AUTH_BOOTSTRAP_PASSWORD` — see
    // `bootstrap_admin`'s doc comment for what happens when either is unset.
    bootstrap_admin(&state).await;

    // Copilot-operations-handover, Tier 3: idempotently seed the service
    // identity Dagster's digital-employee schedules authenticate as, from
    // `AGENT_RUN_TOKEN` — see `bootstrap_agent_run_service`'s doc comment
    // for what happens when it's unset.
    bootstrap_agent_run_service(&state).await;

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

/// Fixed name of the service identity [`bootstrap_agent_run_service`]
/// provisions. Unique (`service_identity_name_unique`), so a second boot
/// finds the same row instead of erroring or creating a duplicate — the
/// same idempotency shape [`bootstrap_admin`] uses for its own fixture
/// (`app_user.email` unique) and `tests/parity.rs`'s harness fixture uses
/// for its own (`PARITY_SERVICE_IDENTITY_NAME`).
const AGENT_RUN_SERVICE_IDENTITY_NAME: &str = "agent-run-scheduler";

/// Idempotently seed the ONE service identity + credential that lets
/// Dagster's digital-employee schedule factory
/// (`dagster/dispar_orchestrate/agent_runs.py`) authenticate against
/// `POST /api/agents/employees/{id}/run`, from
/// [`Config::agent_run_token`].
///
/// # Why this exists at all
///
/// `POST /api/agents/employees/{id}/run`'s [`crate::policy::POLICY_TABLE`]
/// entry is `Policy::RequiresAuth` — a FLOOR, not a ceiling (see that
/// table's own doc comment on `/api/alerts/run`/`/api/gold/export/{mart}`
/// for the same shape): `crate::policy::auth_gate` demands a real,
/// authenticated [`lakehouse_auth::Principal`] BEFORE
/// `routes::agents::check_employee_run_auth`'s own `x-run-token` check
/// ever runs. A compose stack that sets `AGENT_RUN_TOKEN` for Dagster to
/// send as `x-run-token` but never mints Dagster a credential to
/// authenticate WITH still gets 401 at the `auth_gate` layer, before the
/// route's own token check is reached — exactly the gap
/// `gold_export_job` (commit `f8bccc8`) and `agent_run_job` (commit
/// `1c299db`) were left deliberately unscheduled over. This function
/// closes it: it seeds Dagster a real [`lakehouse_auth::Principal`] that
/// authenticates via `Authorization: Bearer <AGENT_RUN_TOKEN>` (opaque —
/// routed to [`lakehouse_auth::service_token::ServiceTokenAuthenticator`]
/// by `crate::auth`'s bearer-shape dispatch), scoped to EXACTLY
/// `agent:manage` — nothing broader, never `*:*`.
///
/// Reusing [`Config::agent_run_token`] (rather than minting a second,
/// bootstrap-only secret) is deliberate: it is already the exact value
/// `routes::agents::check_employee_run_auth` matches against `x-run-token`,
/// already threaded to both `lakehouse-api` and
/// `dagster-code-location` in compose, and already documented in
/// `.env.example` — one env var, one meaning ("the credential Dagster's
/// scheduled agent runs use"), satisfied twice (as the bearer credential
/// AND as the `x-run-token` header), matching
/// `routes::agents::check_employee_run_auth`'s own preference for the
/// token branch (`RunAuth::Token`, `trigger = "schedule"`) whenever both
/// are present and match.
///
/// # Idempotent by construction, not by a pre-check
///
/// Same shape as [`bootstrap_admin`]: this tries
/// [`lakehouse_store::identity::create_service_identity`] and treats its
/// natural failure mode on a re-run
/// (`service_identity.name` already taken, surfaced as
/// [`lakehouse_store::StoreError::Conflict`]) as "already seeded", looking
/// up the existing identity's id instead of erroring. The credential side
/// is idempotent for a different, complementary reason:
/// [`lakehouse_auth::service_token::ensure_service_credential`] inserts
/// with `ON CONFLICT (token_hash) DO NOTHING`, so re-hashing and
/// re-inserting the SAME configured token on every restart is a no-op —
/// the existing credential is never duplicated, revoked, or replaced. If
/// `AGENT_RUN_TOKEN` is rotated to a new value, the old credential row is
/// left untouched (so an in-flight caller using the old token is not
/// abruptly cut off mid-restart) and a second, additional credential is
/// seeded for the new value — exactly how a human rotating their own
/// password keeps their old session alive until it naturally expires.
///
/// # What happens when `AGENT_RUN_TOKEN` is absent
///
/// Logs a `tracing::warn!` and returns, seeding NOTHING — no identity, no
/// credential — same posture as [`bootstrap_admin`] when its own two env
/// vars are unset. Scheduled digital employees simply cannot run yet,
/// which is the existing, safe, documented (`gold_export_job`-precedent)
/// posture this task is closing, not weakening.
async fn bootstrap_agent_run_service(state: &AppState) {
    let Some(token) = state.config.agent_run_token.clone() else {
        tracing::warn!(
            "no agent-run service credential configured: set AGENT_RUN_TOKEN to let \
             Dagster's digital-employee schedules authenticate against \
             POST /api/agents/employees/{{id}}/run (schedules stay inert until then)"
        );
        return;
    };
    let Some(pool) = state.pg.as_deref() else {
        tracing::warn!("cannot seed agent-run service identity: no Postgres pool is configured");
        return;
    };

    let environment = if state.config.is_dev {
        "development"
    } else {
        "production"
    };
    let input = lakehouse_store::identity::CreateServiceIdentityInput {
        name: AGENT_RUN_SERVICE_IDENTITY_NAME.to_owned(),
        // ONLY `agent:manage` — never `*:*`. This identity must not be
        // able to do anything beyond running a digital employee.
        scopes: vec!["agent:manage".to_owned()],
        environment: environment.to_owned(),
    };
    let service_identity_id =
        match lakehouse_store::identity::create_service_identity(pool, &input).await {
            Ok(identity) => {
                let Ok(id) = identity.id.parse() else {
                    tracing::error!(
                        "agent-run service identity was created but its id did not parse as a UUID"
                    );
                    return;
                };
                id
            }
            Err(lakehouse_store::StoreError::Conflict) => {
                tracing::info!("agent-run service identity already exists; reusing it");
                let Some(id) =
                    find_service_identity_id_by_name(pool, AGENT_RUN_SERVICE_IDENTITY_NAME).await
                else {
                    tracing::error!(
                        "agent-run service identity name is taken but its id could not be resolved"
                    );
                    return;
                };
                id
            }
            Err(err) => {
                tracing::error!(%err, "failed to seed agent-run service identity");
                return;
            }
        };

    let token = lakehouse_auth::Secret::new(token);
    if let Err(err) =
        lakehouse_auth::service_token::ensure_service_credential(pool, service_identity_id, &token)
            .await
    {
        tracing::error!(%err, "agent-run service identity was seeded but its credential was not");
        return;
    }
    tracing::info!(
        "agent-run service credential seeded (scope: agent:manage); \
         digital-employee schedules can now authenticate"
    );
}

/// Look up an existing `service_identity.id` by its unique `name`. Only
/// ever called from [`bootstrap_agent_run_service`]'s Conflict branch,
/// where a row with this name is already known to exist — `None` there
/// means the row vanished between the failed insert and this lookup
/// (logged by the caller as an error, not a panic).
async fn find_service_identity_id_by_name(pool: &sqlx::PgPool, name: &str) -> Option<uuid::Uuid> {
    let row: Option<(uuid::Uuid,)> =
        sqlx::query_as("SELECT id FROM service_identity WHERE name = $1")
            .bind(name)
            .fetch_optional(pool)
            .await
            .ok()?;
    row.map(|(id,)| id)
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

    /// `bootstrap_agent_run_service` needs a real, migrated Postgres — the
    /// HTTP-level `tests/*.rs` harness (`common::spin_up_with_env`) can't
    /// call it directly, since it is a private function of this `bin`
    /// crate (see `tests/security_regressions.rs`'s
    /// `create_bootstrap_admin`, which reimplements `bootstrap_admin`'s
    /// shape for the exact same reason). These tests exercise the REAL
    /// function instead, against a fresh per-test database on the same
    /// `lakehouse-test-support` testcontainer every other Postgres-backed
    /// test in this workspace uses.
    mod agent_run_service_bootstrap {
        use std::collections::HashMap;

        use sqlx::postgres::PgPoolOptions;
        use uuid::Uuid;

        // Force-links `lakehouse-test-support` so its `#[ctor]` Postgres
        // testcontainer bootstrap actually runs for this test binary — see
        // `tests/common/mod.rs`'s identical comment.
        use lakehouse_test_support as _;

        use super::*;

        /// Same shape as `tests/common::spin_up_with_env`, trimmed to just
        /// what these tests need: a fresh, migrated database and the
        /// resulting [`AppState`], with `overrides` merged into the config
        /// env map.
        async fn fresh_state(overrides: &HashMap<String, String>) -> AppState {
            let base_url = lakehouse_test_support::database_url();
            let admin_pool = PgPoolOptions::new()
                .max_connections(1)
                .connect(&base_url)
                .await
                .expect("connect to the test-support admin database");
            let db_name = format!("lakehouse_api_main_test_{}", Uuid::new_v4().simple());
            sqlx::query(&format!(r#"CREATE DATABASE "{db_name}""#))
                .execute(&admin_pool)
                .await
                .expect("create a fresh per-test database");
            admin_pool.close().await;

            let (base, _old_db) = base_url
                .rsplit_once('/')
                .expect("a postgres:// URL with a path");
            let db_url = format!("{base}/{db_name}");

            let pool = PgPoolOptions::new()
                .max_connections(5)
                .connect(&db_url)
                .await
                .expect("connect to the fresh per-test database");
            lakehouse_store::migrate(&pool)
                .await
                .expect("apply migrations to the fresh per-test database");

            let mut env = HashMap::new();
            env.insert("DATABASE_URL".to_owned(), db_url);
            for (k, v) in overrides {
                env.insert(k.clone(), v.clone());
            }
            let config = Config::from_map(&env).expect("a valid test Config");
            AppState::new(config)
        }

        async fn identity_row_count(pool: &sqlx::PgPool) -> i64 {
            let (count,): (i64,) =
                sqlx::query_as("SELECT count(*) FROM service_identity WHERE name = $1")
                    .bind(AGENT_RUN_SERVICE_IDENTITY_NAME)
                    .fetch_one(pool)
                    .await
                    .expect("count service_identity rows");
            count
        }

        async fn credential_row_count(pool: &sqlx::PgPool) -> i64 {
            let (count,): (i64,) = sqlx::query_as(
                "SELECT count(*) FROM service_credential sc \
                 JOIN service_identity si ON si.id = sc.service_identity_id \
                 WHERE si.name = $1",
            )
            .bind(AGENT_RUN_SERVICE_IDENTITY_NAME)
            .fetch_one(pool)
            .await
            .expect("count service_credential rows");
            count
        }

        /// With `AGENT_RUN_TOKEN` set, boot seeds exactly one identity
        /// (scoped `agent:manage`) and one matching credential.
        #[tokio::test]
        async fn creates_identity_and_credential_when_token_is_set() {
            let mut overrides = HashMap::new();
            overrides.insert("AGENT_RUN_TOKEN".to_owned(), "unit-test-token".to_owned());
            let state = fresh_state(&overrides).await;

            bootstrap_agent_run_service(&state).await;

            let pool = state.pg.as_deref().expect("pg pool configured");
            assert_eq!(identity_row_count(pool).await, 1);
            assert_eq!(credential_row_count(pool).await, 1);

            let (scopes,): (Vec<String>,) =
                sqlx::query_as("SELECT scopes FROM service_identity WHERE name = $1")
                    .bind(AGENT_RUN_SERVICE_IDENTITY_NAME)
                    .fetch_one(pool)
                    .await
                    .expect("read the seeded identity's scopes");
            assert_eq!(scopes, vec!["agent:manage".to_owned()]);
        }

        /// Running the bootstrap twice (a process restart) does not
        /// duplicate the identity or the credential, and does not
        /// invalidate the credential already seeded — restarting with the
        /// SAME `AGENT_RUN_TOKEN` must be a no-op the second time.
        #[tokio::test]
        async fn is_idempotent_across_two_runs() {
            let mut overrides = HashMap::new();
            overrides.insert("AGENT_RUN_TOKEN".to_owned(), "unit-test-token".to_owned());
            let state = fresh_state(&overrides).await;

            bootstrap_agent_run_service(&state).await;
            bootstrap_agent_run_service(&state).await;

            let pool = state.pg.as_deref().expect("pg pool configured");
            assert_eq!(
                identity_row_count(pool).await,
                1,
                "a second boot must not duplicate the service identity"
            );
            assert_eq!(
                credential_row_count(pool).await,
                1,
                "a second boot with the same token must not duplicate the credential"
            );

            // The ORIGINAL token still authenticates after the second boot
            // — proves the existing credential was reused, not replaced
            // with a fresh (and therefore different) hash.
            let principal = lakehouse_auth::service_token::verify_service_token(
                pool,
                &lakehouse_auth::Secret::new("unit-test-token".to_owned()),
            )
            .await
            .expect("the original token must still authenticate after a second boot");
            assert!(principal.permissions.has("agent:manage"));
        }

        /// With `AGENT_RUN_TOKEN` unset, bootstrap creates nothing at all —
        /// no identity, no credential — same posture as `bootstrap_admin`
        /// when its own env vars are unset.
        #[tokio::test]
        async fn creates_nothing_when_token_is_unset() {
            let state = fresh_state(&HashMap::new()).await;

            bootstrap_agent_run_service(&state).await;

            let pool = state.pg.as_deref().expect("pg pool configured");
            assert_eq!(identity_row_count(pool).await, 0);
            assert_eq!(credential_row_count(pool).await, 0);
        }
    }
}
