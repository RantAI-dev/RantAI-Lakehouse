//! `lakehouse-api` — the axum HTTP service for the `RantAI` Lakehouse
//! backend.
//!
//! This binary owns process-level concerns (config resolution, logging
//! setup, binding, graceful shutdown) that don't belong in a library crate.
//! `anyhow` is used here, and only here, for that reason — library crates
//! (`lakehouse-core`, `lakehouse-clickhouse`, and this crate's own modules)
//! use `thiserror` typed errors instead.

mod auth;
mod bounded;
mod bronze_stats_cache;
mod config;
mod connector_deprovision;
mod connector_discover;
mod connector_probe;
mod error;
mod gold_export;
mod gold_export_history;
mod gold_lock;
mod health;
mod json;
mod lakehouse_catalog;
mod lakekeeper_token;
mod next_run;
mod policy;
mod policy_engine;
mod routes;
mod sql_rewrite;
mod state;
mod tenant;
mod transform_grammar;

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

    // WS4 item G3: same shape, for Dagster's authored-pipeline schedule
    // factory (`dagster/dispar_orchestrate/authored_factory.py`, Phase E),
    // from `PIPELINE_RUN_TOKEN` — see `bootstrap_pipeline_run_service`'s
    // doc comment for what happens when it's unset, and why this identity
    // is scoped to both `pipeline:read` and `pipeline:write`.
    bootstrap_pipeline_run_service(&state).await;

    // WS0 item 11: same shape, for Dagster's `alerts_run_schedule`
    // (`dagster/dispar_orchestrate/alerts_run.py`), from
    // `ALERTS_RUN_TOKEN` — see `bootstrap_alerts_run_service`'s doc
    // comment for what happens when it's unset.
    bootstrap_alerts_run_service(&state).await;

    // Same shape again, for Dagster's maintenance job's policy fetch, from
    // `LAKEHOUSE_MAINTENANCE_TOKEN` — see
    // `bootstrap_lakehouse_maintenance_service`'s doc comment for what
    // happens when it's unset.
    bootstrap_lakehouse_maintenance_service(&state).await;

    // WS3 item 28 (plan review X7): same shape, for Dagster's ingest
    // schedule factory (`dagster/dispar_orchestrate`'s ingest_job/
    // ingest_schedules), from `INGEST_SERVICE_TOKEN` — see
    // `bootstrap_ingest_run_service`'s doc comment for what happens when
    // it's unset, and why this identity is scoped to `ingest:read` only.
    bootstrap_ingest_run_service(&state).await;

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

/// Fixed name of the service identity
/// [`bootstrap_pipeline_run_service`] provisions — same
/// `<domain>-run-scheduler` shape as [`AGENT_RUN_SERVICE_IDENTITY_NAME`]
/// (WS4 item G3).
const PIPELINE_RUN_SERVICE_IDENTITY_NAME: &str = "authored-pipeline-scheduler";

/// Fixed name of the service identity [`bootstrap_alerts_run_service`]
/// provisions — same `<domain>-run-scheduler` shape as
/// [`AGENT_RUN_SERVICE_IDENTITY_NAME`].
const ALERTS_RUN_SERVICE_IDENTITY_NAME: &str = "alerts-run-scheduler";

/// Fixed name of the service identity
/// [`bootstrap_lakehouse_maintenance_service`] provisions — same
/// `<domain>-<role>` naming as the other service identities above, though
/// this one is not a `-run-scheduler`: it authenticates a read of the
/// maintenance policy list, not a scheduled run trigger.
const LAKEHOUSE_MAINTENANCE_SERVICE_IDENTITY_NAME: &str = "lakehouse-maintenance-policy-reader";

/// Fixed name of the service identity [`bootstrap_ingest_run_service`]
/// provisions — same `<domain>-<role>` naming as
/// [`LAKEHOUSE_MAINTENANCE_SERVICE_IDENTITY_NAME`]: this identity reads,
/// it does not run a schedule trigger itself (the ingest job is triggered
/// per-connector by `POST /api/connectors/{id}/ingest/run`, which a human
/// or `agent:manage`/`connector:manage` principal calls — not this
/// identity).
const INGEST_SERVICE_IDENTITY_NAME: &str = "ingest-run-service";

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
///
/// The body below delegates to [`bootstrap_service_run_identity`], the
/// shared core [`bootstrap_alerts_run_service`] also calls (WS0 item 11) —
/// this function's own signature and behavior are unchanged by that
/// extraction, so every existing test that calls it directly keeps
/// passing unmodified.
async fn bootstrap_agent_run_service(state: &AppState) {
    bootstrap_service_run_identity(
        state,
        state.config.agent_run_token.clone(),
        AGENT_RUN_SERVICE_IDENTITY_NAME,
        // ONLY `agent:manage` — never `*:*`. This identity must not be
        // able to do anything beyond running a digital employee.
        // `routes::agents::run_employee` DOES check this permission.
        vec!["agent:manage".to_owned()],
        "AGENT_RUN_TOKEN",
    )
    .await;
}

/// Idempotently seed the ONE service identity + credential that lets
/// Dagster's authored-pipeline schedule factory
/// (`dagster/dispar_orchestrate/authored_factory.py`, Phase E) authenticate
/// against `lakehouse-api`, from [`Config::pipeline_run_token`] — the exact
/// same `auth_gate`-floor problem [`bootstrap_agent_run_service`]/
/// [`bootstrap_alerts_run_service`]/[`bootstrap_lakehouse_maintenance_service`]/
/// [`bootstrap_ingest_run_service`] solve for their own callers, reusing
/// the identical mechanism rather than a fifth, near-duplicate
/// implementation (WS4 item G3; Baseline dependency 7 — the generalized
/// [`bootstrap_service_run_identity`] core is reused, not redefined).
///
/// # Why this identity is scoped to `pipeline:read` AND `pipeline:write`
///
/// `authored_factory.py`'s job is not read-only: it both lists authored
/// pipeline definitions (`GET /api/pipelines?engine=authored`, gated by
/// `Policy::RequiresPermission("pipeline:read")` — `policy.rs`) AND, once
/// Phase E lands, triggers/updates the authored jobs it builds from them
/// (`POST /api/pipelines/{id}/trigger`, `POST /api/pipelines/{id}/status`,
/// gated by `pipeline:write`). A service identity's `scopes` column is an
/// array, not a single value — `0002_seed_identity.sql` already seeds
/// other identities with several tokens (e.g. `ARRAY['query:read',
/// 'catalog:read']`) — so there is no need to pick one permission that
/// covers both: this identity is scoped to exactly `pipeline:read` and
/// `pipeline:write`, nothing broader, never `*:*`, never
/// `catalog:write`/`connector:manage`, which this caller never touches. A
/// leaked `PIPELINE_RUN_TOKEN` lets its holder operate authored pipelines,
/// and nothing else.
///
/// A previous version of this comment claimed `pipeline:write` alone was
/// "the smallest single permission covering both" reads and writes. That
/// was false: `PermissionSet::has` (`lakehouse-auth/src/permissions.rs`)
/// matches resource AND action exactly, so `pipeline:write` never
/// satisfies a `pipeline:read` check. The scheduler got a 403 on the one
/// route it exists to call (`GET /api/pipelines`), `_fetch_authored_
/// pipelines` degraded honestly to an empty list, and no authored pipeline
/// ever ran — silently, with no error surfaced anywhere (WS4 item G3
/// follow-up fix).
///
/// # What happens when `PIPELINE_RUN_TOKEN` is absent
///
/// Logs a `tracing::warn!` and returns, seeding NOTHING — no identity, no
/// credential — same posture as every other `bootstrap_*_run_service`
/// above when its own token is unset. Phase E's schedule factory simply
/// cannot authenticate yet, which is the existing, safe, documented
/// posture this task follows, not a new one.
async fn bootstrap_pipeline_run_service(state: &AppState) {
    bootstrap_service_run_identity(
        state,
        state.config.pipeline_run_token.clone(),
        PIPELINE_RUN_SERVICE_IDENTITY_NAME,
        // `pipeline:read` AND `pipeline:write` — never `*:*`. See this
        // function's own doc comment for why both are needed (the
        // scheduler both lists and triggers authored pipelines) and why
        // an array of two scopes, not one permission, is the correct fix.
        vec!["pipeline:read".to_owned(), "pipeline:write".to_owned()],
        "PIPELINE_RUN_TOKEN",
    )
    .await;
}

/// Lets `dagster/dispar_orchestrate/alerts_run.py`'s scheduled trigger
/// authenticate against `POST /api/alerts/run`'s `Policy::RequiresAuth`
/// floor, from [`Config::agent_run_token`]'s sibling,
/// `Config::alerts_run_token` — the exact same `auth_gate`-floor problem
/// [`bootstrap_agent_run_service`] solves for digital-employee runs,
/// reusing the identical mechanism rather than a second, near-duplicate
/// implementation.
///
/// # Why this identity gets NO scopes
///
/// Unlike the agent-run identity, this one is seeded with an EMPTY scope
/// list. `POST`/`GET /api/alerts/run` is `Policy::RequiresAuth`
/// (`policy::POLICY_TABLE`) — any authenticated principal clears that
/// floor — and `routes::alerts::run`'s handler body checks no permission
/// at all: `routes::alerts::check_run_token` only compares the
/// `x-run-token` header (or falls back to requiring a
/// `PrincipalId::Service` principal) once `ALERTS_RUN_TOKEN` is set, which
/// it is here. The `x-run-token` check is what actually gates this route;
/// authenticating at all is enough to clear the floor. Granting this
/// identity `alert:write` (the permission `POST`/`PUT`/`DELETE
/// /api/alerts` require) would be strictly more privilege than the route
/// it authenticates for ever inspects — a leaked `ALERTS_RUN_TOKEN` would
/// then also let its holder create, edit, or delete alert rules, not just
/// trigger a run. An empty [`lakehouse_auth::PermissionSet`] still
/// authenticates (`verify_service_token` builds it from
/// `service_identity.scopes` regardless of whether that array is empty)
/// and satisfies `RequiresAuth`; it simply satisfies no
/// `RequiresPermission` check, which this route never performs.
async fn bootstrap_alerts_run_service(state: &AppState) {
    bootstrap_service_run_identity(
        state,
        state.config.alerts_run_token.clone(),
        ALERTS_RUN_SERVICE_IDENTITY_NAME,
        Vec::new(),
        "ALERTS_RUN_TOKEN",
    )
    .await;
}

/// Lets `dagster/dispar_orchestrate/maintenance.py`'s maintenance job
/// authenticate against `GET /api/lakehouse/maintenance-policies`'s
/// `Policy::RequiresAuth` floor, from
/// [`Config::lakehouse_maintenance_token`] — the exact same `auth_gate`-
/// floor problem [`bootstrap_agent_run_service`]/
/// [`bootstrap_alerts_run_service`] solve for their own callers, reusing
/// the identical mechanism rather than a third, near-duplicate
/// implementation.
///
/// # Why this identity gets NO scopes
///
/// `GET /api/lakehouse/maintenance-policies` is `Policy::RequiresAuth`
/// (`policy::POLICY_TABLE`) — any authenticated principal clears that
/// floor — and its handler, `routes::lakehouse::list_maintenance_policies`,
/// checks no permission at all: it returns the maintenance policy list to
/// any caller who authenticates. The policy list itself carries no secret.
/// Granting this identity `catalog:read` would be strictly more privilege
/// than the route it authenticates for ever inspects — a leaked
/// `LAKEHOUSE_MAINTENANCE_TOKEN` would then also let its holder read the
/// whole Iceberg catalog, not just the maintenance policy list. An empty
/// [`lakehouse_auth::PermissionSet`] still authenticates
/// (`verify_service_token` builds it from `service_identity.scopes`
/// regardless of whether that array is empty) and satisfies
/// `RequiresAuth`; it simply satisfies no `RequiresPermission` check,
/// which this route never performs.
async fn bootstrap_lakehouse_maintenance_service(state: &AppState) {
    bootstrap_service_run_identity(
        state,
        state.config.lakehouse_maintenance_token.clone(),
        LAKEHOUSE_MAINTENANCE_SERVICE_IDENTITY_NAME,
        Vec::new(),
        "LAKEHOUSE_MAINTENANCE_TOKEN",
    )
    .await;
}

/// Lets `dagster/dispar_orchestrate`'s ingest schedule factory
/// authenticate against `GET /api/connectors/ingestible`'s
/// `Policy::RequiresPermission("ingest:read")` and a CDC connector's
/// `dial` read (`ops/debezium/render_compose.py`'s caller), from
/// [`Config::ingest_service_token`] — the exact same `auth_gate`-floor
/// problem [`bootstrap_agent_run_service`]/[`bootstrap_alerts_run_service`]/
/// [`bootstrap_lakehouse_maintenance_service`] solve for their own
/// callers, reusing the identical mechanism rather than a fourth,
/// near-duplicate implementation.
///
/// # Why this identity is scoped to `ingest:read` ONLY (WS3 plan review X7)
///
/// This caller reads connector rows (`GET /api/connectors/ingestible`,
/// `routes::connectors::list_ingestible`) and a CDC connector's `dial`
/// (`ops/debezium/render_compose.py`) — it never creates, updates, or
/// deletes a connector.
/// `connector:manage` would be a STRICTLY BROADER grant than this caller
/// ever needs: a leaked `INGEST_SERVICE_TOKEN` would then also let its
/// holder create, edit, or delete connectors, not just list and read the
/// ones already there. `ingest:read` is exactly what
/// `routes::connectors::list_ingestible` (and every other route this
/// identity calls) checks — nothing broader, never `connector:manage`,
/// never `*:*`.
async fn bootstrap_ingest_run_service(state: &AppState) {
    bootstrap_service_run_identity(
        state,
        state.config.ingest_service_token.clone(),
        INGEST_SERVICE_IDENTITY_NAME,
        vec!["ingest:read".to_owned()],
        "INGEST_SERVICE_TOKEN",
    )
    .await;
}

/// Idempotently seed ONE service identity + credential that lets a
/// Dagster job authenticate against a token-guarded, `RequiresAuth`
/// route — the shared core [`bootstrap_agent_run_service`],
/// [`bootstrap_alerts_run_service`],
/// [`bootstrap_lakehouse_maintenance_service`], and
/// [`bootstrap_ingest_run_service`] all call. See
/// [`bootstrap_agent_run_service`]'s doc comment for the full "why this
/// exists at all" rationale — every caller hits the identical `auth_gate`
/// floor problem, on `POST /api/agents/employees/{id}/run`,
/// `POST /api/alerts/run`, and
/// `GET /api/lakehouse/maintenance-policies` respectively.
///
/// `identity_name` must be a fixed, unique `service_identity.name` (its
/// own `UNIQUE` constraint is what makes this idempotent across restarts,
/// same as [`bootstrap_admin`]'s `app_user.email` unique constraint).
/// `scopes` should be the narrowest set the caller's route needs — never
/// `*:*`; empty when the route checks no permission (see
/// [`bootstrap_alerts_run_service`]'s doc comment for why that is the
/// right choice there). `token` is `None` when the caller's own token env
/// var is unset, in which case this seeds nothing at all (see
/// [`bootstrap_agent_run_service`]'s doc comment, "What happens when ...
/// is absent").
///
/// Idempotency and rotation behavior are identical for every caller: see
/// [`bootstrap_agent_run_service`]'s doc comment section "Idempotent by
/// construction, not by a pre-check".
async fn bootstrap_service_run_identity(
    state: &AppState,
    token: Option<String>,
    identity_name: &'static str,
    scopes: Vec<String>,
    missing_token_env_var: &str,
) {
    let Some(token) = token else {
        tracing::warn!(
            "no {identity_name} service credential configured: set {missing_token_env_var} to \
             let its Dagster schedule authenticate (the schedule stays inert until then)"
        );
        return;
    };
    let Some(pool) = state.pg.as_deref() else {
        tracing::warn!(
            "cannot seed {identity_name} service identity: no Postgres pool is configured"
        );
        return;
    };

    let environment = if state.config.is_dev {
        "development"
    } else {
        "production"
    };
    let input = lakehouse_store::identity::CreateServiceIdentityInput {
        name: identity_name.to_owned(),
        scopes,
        environment: environment.to_owned(),
    };
    let service_identity_id =
        match lakehouse_store::identity::create_service_identity(pool, &input).await {
            Ok(identity) => {
                let Ok(id) = identity.id.parse() else {
                    tracing::error!(
                        "{identity_name} service identity was created but its id did not parse \
                         as a UUID"
                    );
                    return;
                };
                id
            }
            Err(lakehouse_store::StoreError::Conflict) => {
                tracing::info!("{identity_name} service identity already exists; reusing it");
                let Some(id) = find_service_identity_id_by_name(pool, identity_name).await else {
                    tracing::error!(
                        "{identity_name} service identity name is taken but its id could not be \
                         resolved"
                    );
                    return;
                };
                id
            }
            Err(err) => {
                tracing::error!(%err, "failed to seed {identity_name} service identity");
                return;
            }
        };

    let token = lakehouse_auth::Secret::new(token);
    if let Err(err) =
        lakehouse_auth::service_token::ensure_service_credential(pool, service_identity_id, &token)
            .await
    {
        tracing::error!(%err, "{identity_name} service identity was seeded but its credential was not");
        return;
    }
    tracing::info!("{identity_name} service credential seeded");
}

/// Look up an existing `service_identity.id` by its unique `name`. Called
/// from the [`bootstrap_service_run_identity`] Conflict branch shared by
/// [`bootstrap_agent_run_service`], [`bootstrap_alerts_run_service`], and
/// [`bootstrap_lakehouse_maintenance_service`], where a row with this name
/// is already known to exist — `None` there means the row vanished between
/// the failed insert and this lookup (logged by the caller as an error,
/// not a panic).
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
    /// reply `400 {"error": "unknown kind: lineage"}`).
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
            !text.contains("unknown kind"),
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

        async fn alerts_identity_row_count(pool: &sqlx::PgPool) -> i64 {
            let (count,): (i64,) =
                sqlx::query_as("SELECT count(*) FROM service_identity WHERE name = $1")
                    .bind(ALERTS_RUN_SERVICE_IDENTITY_NAME)
                    .fetch_one(pool)
                    .await
                    .expect("count service_identity rows");
            count
        }

        async fn alerts_credential_row_count(pool: &sqlx::PgPool) -> i64 {
            let (count,): (i64,) = sqlx::query_as(
                "SELECT count(*) FROM service_credential sc \
                 JOIN service_identity si ON si.id = sc.service_identity_id \
                 WHERE si.name = $1",
            )
            .bind(ALERTS_RUN_SERVICE_IDENTITY_NAME)
            .fetch_one(pool)
            .await
            .expect("count service_credential rows");
            count
        }

        /// With `ALERTS_RUN_TOKEN` set, boot seeds exactly one identity
        /// (no scopes — see [`bootstrap_alerts_run_service`]'s doc comment
        /// for why) and one matching credential; the token authenticates a
        /// real principal that nonetheless does NOT hold `alert:write` —
        /// proving [`bootstrap_service_run_identity`] behaves identically
        /// for this caller while honoring B9's narrower scope decision.
        #[tokio::test]
        async fn bootstrap_alerts_run_service_seeds_identity_and_credential() {
            let mut overrides = HashMap::new();
            overrides.insert(
                "ALERTS_RUN_TOKEN".to_owned(),
                "unit-test-alerts-token".to_owned(),
            );
            let state = fresh_state(&overrides).await;

            bootstrap_alerts_run_service(&state).await;

            let pool = state.pg.as_deref().expect("pg pool configured");
            assert_eq!(alerts_identity_row_count(pool).await, 1);
            assert_eq!(alerts_credential_row_count(pool).await, 1);

            let (scopes,): (Vec<String>,) =
                sqlx::query_as("SELECT scopes FROM service_identity WHERE name = $1")
                    .bind(ALERTS_RUN_SERVICE_IDENTITY_NAME)
                    .fetch_one(pool)
                    .await
                    .expect("read the seeded identity's scopes");
            assert_eq!(scopes, Vec::<String>::new());

            let principal = lakehouse_auth::service_token::verify_service_token(
                pool,
                &lakehouse_auth::Secret::new("unit-test-alerts-token".to_owned()),
            )
            .await
            .expect("the configured token must authenticate a real service principal");
            assert!(!principal.permissions.has("alert:write"));
        }

        /// With `ALERTS_RUN_TOKEN` unset, bootstrap creates nothing at all —
        /// no identity, no credential — same posture as
        /// [`bootstrap_agent_run_service`] when its own token is unset.
        #[tokio::test]
        async fn bootstrap_alerts_run_service_creates_nothing_when_token_is_unset() {
            let state = fresh_state(&HashMap::new()).await;

            bootstrap_alerts_run_service(&state).await;

            let pool = state.pg.as_deref().expect("pg pool configured");
            assert_eq!(alerts_identity_row_count(pool).await, 0);
            assert_eq!(alerts_credential_row_count(pool).await, 0);
        }

        async fn lakehouse_maintenance_identity_row_count(pool: &sqlx::PgPool) -> i64 {
            let (count,): (i64,) =
                sqlx::query_as("SELECT count(*) FROM service_identity WHERE name = $1")
                    .bind(LAKEHOUSE_MAINTENANCE_SERVICE_IDENTITY_NAME)
                    .fetch_one(pool)
                    .await
                    .expect("count service_identity rows");
            count
        }

        async fn lakehouse_maintenance_credential_row_count(pool: &sqlx::PgPool) -> i64 {
            let (count,): (i64,) = sqlx::query_as(
                "SELECT count(*) FROM service_credential sc \
                 JOIN service_identity si ON si.id = sc.service_identity_id \
                 WHERE si.name = $1",
            )
            .bind(LAKEHOUSE_MAINTENANCE_SERVICE_IDENTITY_NAME)
            .fetch_one(pool)
            .await
            .expect("count service_credential rows");
            count
        }

        /// With `LAKEHOUSE_MAINTENANCE_TOKEN` set, boot seeds exactly one
        /// identity (no scopes — see
        /// [`bootstrap_lakehouse_maintenance_service`]'s doc comment for
        /// why) and one matching credential; the token authenticates a
        /// real principal that nonetheless does NOT hold `catalog:read`.
        #[tokio::test]
        async fn bootstrap_lakehouse_maintenance_service_seeds_identity_and_credential() {
            let mut overrides = HashMap::new();
            overrides.insert(
                "LAKEHOUSE_MAINTENANCE_TOKEN".to_owned(),
                "unit-test-lakehouse-maintenance-token".to_owned(),
            );
            let state = fresh_state(&overrides).await;

            bootstrap_lakehouse_maintenance_service(&state).await;

            let pool = state.pg.as_deref().expect("pg pool configured");
            assert_eq!(lakehouse_maintenance_identity_row_count(pool).await, 1);
            assert_eq!(lakehouse_maintenance_credential_row_count(pool).await, 1);

            let (scopes,): (Vec<String>,) =
                sqlx::query_as("SELECT scopes FROM service_identity WHERE name = $1")
                    .bind(LAKEHOUSE_MAINTENANCE_SERVICE_IDENTITY_NAME)
                    .fetch_one(pool)
                    .await
                    .expect("read the seeded identity's scopes");
            assert_eq!(scopes, Vec::<String>::new());

            let principal = lakehouse_auth::service_token::verify_service_token(
                pool,
                &lakehouse_auth::Secret::new("unit-test-lakehouse-maintenance-token".to_owned()),
            )
            .await
            .expect("the configured token must authenticate a real service principal");
            assert!(!principal.permissions.has("catalog:read"));
        }

        /// With `LAKEHOUSE_MAINTENANCE_TOKEN` unset, bootstrap creates
        /// nothing at all — no identity, no credential — same posture as
        /// [`bootstrap_alerts_run_service`] when its own token is unset.
        #[tokio::test]
        async fn bootstrap_lakehouse_maintenance_service_creates_nothing_when_token_is_unset() {
            let state = fresh_state(&HashMap::new()).await;

            bootstrap_lakehouse_maintenance_service(&state).await;

            let pool = state.pg.as_deref().expect("pg pool configured");
            assert_eq!(lakehouse_maintenance_identity_row_count(pool).await, 0);
            assert_eq!(lakehouse_maintenance_credential_row_count(pool).await, 0);
        }

        async fn ingest_identity_row_count(pool: &sqlx::PgPool) -> i64 {
            let (count,): (i64,) =
                sqlx::query_as("SELECT count(*) FROM service_identity WHERE name = $1")
                    .bind(INGEST_SERVICE_IDENTITY_NAME)
                    .fetch_one(pool)
                    .await
                    .expect("count service_identity rows");
            count
        }

        async fn ingest_credential_row_count(pool: &sqlx::PgPool) -> i64 {
            let (count,): (i64,) = sqlx::query_as(
                "SELECT count(*) FROM service_credential sc \
                 JOIN service_identity si ON si.id = sc.service_identity_id \
                 WHERE si.name = $1",
            )
            .bind(INGEST_SERVICE_IDENTITY_NAME)
            .fetch_one(pool)
            .await
            .expect("count service_credential rows");
            count
        }

        /// With `INGEST_SERVICE_TOKEN` set, boot seeds exactly one identity
        /// scoped to `ingest:read` ONLY (WS3 plan review X7) and one
        /// matching credential; the token authenticates a real principal
        /// that holds `ingest:read` but NOT `connector:manage` — proving
        /// [`bootstrap_service_run_identity`] behaves identically for this
        /// caller while honoring X7's narrower scope decision.
        #[tokio::test]
        async fn bootstrap_ingest_run_service_seeds_identity_and_credential() {
            let mut overrides = HashMap::new();
            overrides.insert(
                "INGEST_SERVICE_TOKEN".to_owned(),
                "unit-test-ingest-token".to_owned(),
            );
            let state = fresh_state(&overrides).await;

            bootstrap_ingest_run_service(&state).await;

            let pool = state.pg.as_deref().expect("pg pool configured");
            assert_eq!(ingest_identity_row_count(pool).await, 1);
            assert_eq!(ingest_credential_row_count(pool).await, 1);

            let (scopes,): (Vec<String>,) =
                sqlx::query_as("SELECT scopes FROM service_identity WHERE name = $1")
                    .bind(INGEST_SERVICE_IDENTITY_NAME)
                    .fetch_one(pool)
                    .await
                    .expect("read the seeded identity's scopes");
            assert_eq!(scopes, vec!["ingest:read".to_owned()]);

            let principal = lakehouse_auth::service_token::verify_service_token(
                pool,
                &lakehouse_auth::Secret::new("unit-test-ingest-token".to_owned()),
            )
            .await
            .expect("the configured token must authenticate a real service principal");
            assert!(principal.permissions.has("ingest:read"));
            assert!(!principal.permissions.has("connector:manage"));
        }

        /// With `INGEST_SERVICE_TOKEN` unset, bootstrap creates nothing at
        /// all — no identity, no credential — same posture as
        /// [`bootstrap_lakehouse_maintenance_service`] when its own token
        /// is unset.
        #[tokio::test]
        async fn bootstrap_ingest_run_service_creates_nothing_when_token_is_unset() {
            let state = fresh_state(&HashMap::new()).await;

            bootstrap_ingest_run_service(&state).await;

            let pool = state.pg.as_deref().expect("pg pool configured");
            assert_eq!(ingest_identity_row_count(pool).await, 0);
            assert_eq!(ingest_credential_row_count(pool).await, 0);
        }

        async fn pipeline_run_identity_row_count(pool: &sqlx::PgPool) -> i64 {
            let (count,): (i64,) =
                sqlx::query_as("SELECT count(*) FROM service_identity WHERE name = $1")
                    .bind(PIPELINE_RUN_SERVICE_IDENTITY_NAME)
                    .fetch_one(pool)
                    .await
                    .expect("count service_identity rows");
            count
        }

        async fn pipeline_run_credential_row_count(pool: &sqlx::PgPool) -> i64 {
            let (count,): (i64,) = sqlx::query_as(
                "SELECT count(*) FROM service_credential sc \
                 JOIN service_identity si ON si.id = sc.service_identity_id \
                 WHERE si.name = $1",
            )
            .bind(PIPELINE_RUN_SERVICE_IDENTITY_NAME)
            .fetch_one(pool)
            .await
            .expect("count service_credential rows");
            count
        }

        /// With `PIPELINE_RUN_TOKEN` set, boot seeds exactly one identity
        /// scoped to `pipeline:read` AND `pipeline:write` (WS4 item G3) and
        /// one matching credential; the token authenticates a real
        /// principal that holds both `pipeline:read` (the list route the
        /// scheduler calls) and `pipeline:write` (the trigger route) but
        /// NOT `catalog:write`/`connector:manage` — proving
        /// [`bootstrap_service_run_identity`] behaves identically for this
        /// caller while honoring this task's minimal-but-sufficient scope
        /// decision. A test that only checked what the identity lacks
        /// would not have caught the identity also lacking `pipeline:read`
        /// (the bug this test guards against).
        #[tokio::test]
        async fn bootstrap_pipeline_run_service_seeds_identity_and_credential() {
            let mut overrides = HashMap::new();
            overrides.insert(
                "PIPELINE_RUN_TOKEN".to_owned(),
                "unit-test-pipeline-run-token".to_owned(),
            );
            let state = fresh_state(&overrides).await;

            bootstrap_pipeline_run_service(&state).await;

            let pool = state.pg.as_deref().expect("pg pool configured");
            assert_eq!(pipeline_run_identity_row_count(pool).await, 1);
            assert_eq!(pipeline_run_credential_row_count(pool).await, 1);

            let (scopes,): (Vec<String>,) =
                sqlx::query_as("SELECT scopes FROM service_identity WHERE name = $1")
                    .bind(PIPELINE_RUN_SERVICE_IDENTITY_NAME)
                    .fetch_one(pool)
                    .await
                    .expect("read the seeded identity's scopes");
            assert_eq!(
                scopes,
                vec!["pipeline:read".to_owned(), "pipeline:write".to_owned()]
            );

            let principal = lakehouse_auth::service_token::verify_service_token(
                pool,
                &lakehouse_auth::Secret::new("unit-test-pipeline-run-token".to_owned()),
            )
            .await
            .expect("the configured token must authenticate a real service principal");
            assert!(principal.permissions.has("pipeline:write"));
            assert!(principal.permissions.has("pipeline:read"));
            assert!(!principal.permissions.has("catalog:write"));
            assert!(!principal.permissions.has("connector:manage"));
        }

        /// Running the bootstrap twice (a process restart) does not
        /// duplicate the identity or the credential — same idempotency
        /// shape every other `bootstrap_*_run_service` proves for itself.
        #[tokio::test]
        async fn bootstrap_pipeline_run_service_is_idempotent_across_two_runs() {
            let mut overrides = HashMap::new();
            overrides.insert(
                "PIPELINE_RUN_TOKEN".to_owned(),
                "unit-test-pipeline-run-token".to_owned(),
            );
            let state = fresh_state(&overrides).await;

            bootstrap_pipeline_run_service(&state).await;
            bootstrap_pipeline_run_service(&state).await;

            let pool = state.pg.as_deref().expect("pg pool configured");
            assert_eq!(
                pipeline_run_identity_row_count(pool).await,
                1,
                "a second boot must not duplicate the service identity"
            );
            assert_eq!(
                pipeline_run_credential_row_count(pool).await,
                1,
                "a second boot with the same token must not duplicate the credential"
            );
        }

        /// With `PIPELINE_RUN_TOKEN` unset, bootstrap creates nothing at
        /// all — no identity, no credential — same posture as
        /// [`bootstrap_ingest_run_service`] when its own token is unset.
        #[tokio::test]
        async fn bootstrap_pipeline_run_service_creates_nothing_when_token_is_unset() {
            let state = fresh_state(&HashMap::new()).await;

            bootstrap_pipeline_run_service(&state).await;

            let pool = state.pg.as_deref().expect("pg pool configured");
            assert_eq!(pipeline_run_identity_row_count(pool).await, 0);
            assert_eq!(pipeline_run_credential_row_count(pool).await, 0);
        }
    }
}
