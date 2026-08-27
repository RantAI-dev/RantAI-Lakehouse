//! Shared application state, threaded into every axum handler via
//! [`axum::extract::State`].

use std::sync::Arc;

use lakehouse_clickhouse::ChClient;

use crate::config::Config;

/// State shared across all route handlers.
///
/// Cheap to clone: both fields are behind an [`Arc`]. `lakehouse-clickhouse`
/// doesn't derive `Clone` on [`ChClient`] itself (it holds a pooled
/// `reqwest::Client` but isn't `Clone` at the type level), so it's wrapped
/// here rather than modifying that crate. Later tasks add more clients here
/// (LLM, Dagster, ...); keep every addition similarly cheap to clone.
///
/// `#[allow(dead_code)]`: no handler reads these fields yet — route
/// handlers that do land in later tasks; this crate only mounts `/health`,
/// which needs no state.
#[derive(Clone)]
#[allow(dead_code)]
pub struct AppState {
    /// Resolved application configuration.
    pub config: Arc<Config>,
    /// `ClickHouse` HTTP client.
    pub clickhouse: Arc<ChClient>,
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
        Self {
            config: Arc::new(config),
            clickhouse: Arc::new(clickhouse),
        }
    }
}
