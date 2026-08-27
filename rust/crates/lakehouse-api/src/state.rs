//! Shared application state, threaded into every axum handler via
//! [`axum::extract::State`].

use std::sync::Arc;

use lakehouse_clickhouse::ChClient;
use lakehouse_dagster::DgClient;

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
        Self {
            config: Arc::new(config),
            clickhouse: Arc::new(clickhouse),
            dagster: Arc::new(dagster),
        }
    }
}
