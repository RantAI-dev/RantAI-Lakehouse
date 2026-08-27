//! Shared application state, threaded into every axum handler via
//! [`axum::extract::State`].

use std::sync::Arc;

use lakehouse_clickhouse::ChClient;
use lakehouse_dagster::DgClient;
use lakehouse_embed::EmbedSecretResolver;
use lakehouse_llm::LlmClient;
use lakehouse_store::PgPool;

use crate::config::Config;

/// State shared across all route handlers.
///
/// Cheap to clone: every field is behind an [`Arc`]. `lakehouse-clickhouse`
/// doesn't derive `Clone` on [`ChClient`] itself (it holds a pooled
/// `reqwest::Client` but isn't `Clone` at the type level), so it's wrapped
/// here rather than modifying that crate. Later tasks add more clients here
/// (LLM, ...); keep every addition similarly cheap to clone.
#[derive(Clone)]
pub struct AppState {
    /// Resolved application configuration. Read by the `/api/alerts/run`
    /// handler (`ALERTS_RUN_TOKEN`, `SMTP_*`).
    pub config: Arc<Config>,
    /// `ClickHouse` HTTP client.
    pub clickhouse: Arc<ChClient>,
    /// `Dagster` GraphQL client.
    pub dagster: Arc<DgClient>,
    /// Resolves and caches the signed-embedding (`JWT`) secret.
    pub embed_secret: Arc<EmbedSecretResolver>,
    /// `OpenAI`-compatible chat-completions client.
    pub llm: Arc<LlmClient>,
    /// Phase 2 OLTP pool (`lakehouse-store`).
    ///
    /// `Option`, not a bare `Arc<PgPool>`: `lakehouse_store::connect_lazy`
    /// only ever fails on a malformed `DATABASE_URL` (never on Postgres
    /// being unreachable — see its doc comment), and that one failure mode
    /// must not stop `lakehouse-api` from booting and serving the Phase 1
    /// routes, none of which touch Postgres. When this is `None`, a Phase 2
    /// handler must reply with `lakehouse_store::StoreError::Unavailable`
    /// (-> `ApiError::Internal`, 500) rather than panic on `.unwrap()`.
    #[allow(
        dead_code,
        reason = "this task (2.1) only builds the OLTP foundation; no route \
                  handler reads this field yet — the first Phase 2 domain \
                  migrated onto Postgres will be its first reader"
    )]
    pub pg: Option<Arc<PgPool>>,
}

impl AppState {
    /// Build application state from a resolved [`Config`].
    #[must_use]
    pub fn new(config: Config) -> Self {
        let clickhouse = ChClient::new(
            config.ch_url.clone(),
            config.ch_user.clone(),
            config.ch_password.clone(),
        );
        let dagster = DgClient::with_repository(
            config.dagster_url.clone(),
            config.dagster_repo.clone(),
            config.dagster_location.clone(),
        );
        let clickhouse = Arc::new(clickhouse);
        let embed_secret =
            EmbedSecretResolver::new(config.embed_secret.clone(), clickhouse.clone());
        let llm = LlmClient::new(
            config.llm_url.clone(),
            config.llm_model.clone(),
            config.llm_key.clone(),
        );
        // `connect_lazy` performs no I/O, so this never blocks on, or fails
        // because of, Postgres being down — see the field doc comment and
        // `lakehouse_store::connect_lazy`'s. It can only fail on a
        // malformed `DATABASE_URL`, which is logged and degrades to `None`
        // rather than aborting startup.
        let pg = match lakehouse_store::connect_lazy(&config.database_url) {
            Ok(pool) => Some(Arc::new(pool)),
            Err(err) => {
                tracing::warn!(%err, "DATABASE_URL is not a valid Postgres connection string; Phase 2 routes will report the database as unavailable");
                None
            }
        };
        Self {
            config: Arc::new(config),
            clickhouse,
            dagster: Arc::new(dagster),
            embed_secret: Arc::new(embed_secret),
            llm: Arc::new(llm),
            pg,
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::collections::HashMap;

    use super::*;

    /// The boot-behavior guarantee this module exists to provide: building
    /// `AppState` never blocks on, or fails because of, Postgres being
    /// unreachable — the default `DATABASE_URL` points at `localhost:5432`,
    /// which is not running in this test process, and `AppState::new` must
    /// still return synchronously with a populated `pg` pool.
    ///
    /// `#[tokio::test]` (not plain `#[test]`), even though `AppState::new`
    /// itself is synchronous: `sqlx`'s lazy pool sets up an idle-connection
    /// reaper against the ambient Tokio runtime as part of construction, so
    /// it needs one present — exactly the situation it runs in for real,
    /// since `main` only ever calls this from inside `#[tokio::main]`.
    #[tokio::test]
    async fn app_state_boots_with_default_database_url_and_no_live_postgres() {
        let cfg = Config::from_map(&HashMap::new()).unwrap();
        let state = AppState::new(cfg);
        assert!(state.pg.is_some());
    }

    /// The one failure mode `connect_lazy` actually has: a `DATABASE_URL`
    /// that isn't a parseable Postgres URL at all. `AppState::new` must
    /// still return (not panic), just with `pg: None`.
    #[tokio::test]
    async fn app_state_degrades_to_no_pool_on_malformed_database_url() {
        let mut env = HashMap::new();
        env.insert("DATABASE_URL".to_owned(), "not a postgres url".to_owned());
        let cfg = Config::from_map(&env).unwrap();
        let state = AppState::new(cfg);
        assert!(state.pg.is_none());
    }
}
