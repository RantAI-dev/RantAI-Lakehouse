//! Shared application state, threaded into every axum handler via
//! [`axum::extract::State`].

use std::sync::Arc;

use lakehouse_auth::{
    LocalPasswordAuthenticator, OidcAuthenticator, OidcConfig, ServiceTokenAuthenticator,
    SessionAuthenticator,
};
use lakehouse_clickhouse::ChClient;
use lakehouse_dagster::DgClient;
use lakehouse_embed::EmbedSecretResolver;
use lakehouse_llm::LlmClient;
use lakehouse_store::PgPool;

use crate::config::Config;

/// The three [`lakehouse_auth::Authenticator`]s this service configures,
/// bundled together so [`AppState::auth`] can stay a single `Option` field
/// mirroring [`AppState::pg`]'s "no Postgres, no Phase 2" pattern — auth
/// cannot function without Postgres either, since every identity, session,
/// and service-credential row lives there.
///
/// Task 3.5 added exactly what this doc comment always said it would: one
/// more field ([`Self::oidc`]) and one more branch in
/// `crate::auth::AuthenticatedPrincipal`'s bearer-token loop — no change to
/// [`AppState`] itself, [`crate::auth`]'s [`AuthenticatedPrincipal`] type,
/// or any handler. See this task's final report for the complete
/// "what actually had to change" accounting.
#[derive(Clone)]
pub struct AuthState {
    /// Verifies `{ email, password }` against `auth_identity` — used only
    /// by `POST /api/auth/login`.
    pub local: Arc<LocalPasswordAuthenticator>,
    /// Verifies the opaque session cookie.
    pub session: Arc<SessionAuthenticator>,
    /// Verifies the opaque `Authorization: Bearer` service token.
    pub service: Arc<ServiceTokenAuthenticator>,
    /// Verifies a `JWT` `Authorization: Bearer` id token against a
    /// configured `OIDC` provider's `JWKS` (Task 3.5). `None` when
    /// `OIDC_ISSUER`/`OIDC_CLIENT_ID` are not both set — see
    /// [`AppState::new`]. When `None`, `crate::auth`'s bearer dispatch
    /// simply never tries the `OIDC` path, which is exactly what "OIDC
    /// unconfigured behaves like today" requires.
    pub oidc: Option<Arc<OidcAuthenticator>>,
}

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
    /// (-> `ApiError::Unavailable`, 503) rather than panic on `.unwrap()` —
    /// see `routes::identity::pool`, the first (and so far only) reader of
    /// this field.
    pub pg: Option<Arc<PgPool>>,
    /// The configured authenticators, or `None` under the exact same
    /// condition as [`Self::pg`] being `None` (no Postgres pool). When
    /// `None`, `crate::auth::AuthenticatedPrincipal` and every protected
    /// route reply 503 rather than panic — see
    /// `crate::auth::authenticators`.
    pub auth: Option<AuthState>,
}

/// Translate [`Config`]'s flat `oidc_*` env-derived fields into
/// [`lakehouse_auth::OidcConfig`], or `None` if `OIDC` is not configured.
///
/// `OIDC` is considered configured only when BOTH `OIDC_ISSUER` and
/// `OIDC_CLIENT_ID` are set — see [`Config::oidc_issuer`]'s doc comment.
/// This is the one place that decision is made; every other piece of this
/// module and `crate::auth` just reacts to [`AuthState::oidc`] being
/// present or absent.
fn oidc_config(config: &Config) -> Option<OidcConfig> {
    let issuer = config.oidc_issuer.as_ref()?;
    let client_id = config.oidc_client_id.as_ref()?;
    let jwks_url = config
        .oidc_jwks_url
        .clone()
        .unwrap_or_else(|| format!("{}/.well-known/jwks.json", issuer.trim_end_matches('/')));
    Some(OidcConfig {
        issuer: issuer.clone(),
        client_id: client_id.clone(),
        provider_name: config.oidc_provider_name.clone(),
        jwks_url,
        jit_provisioning: config.oidc_jit_provisioning,
        role_map: config.oidc_role_map.clone(),
        groups_claim: config.oidc_groups_claim.clone(),
        clock_skew_seconds: config.oidc_clock_skew_seconds,
    })
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
        let auth = pg.as_ref().map(|pool| AuthState {
            local: Arc::new(LocalPasswordAuthenticator::new((**pool).clone())),
            session: Arc::new(SessionAuthenticator::new((**pool).clone())),
            service: Arc::new(ServiceTokenAuthenticator::new((**pool).clone())),
            oidc: oidc_config(&config)
                .map(|oidc_config| Arc::new(OidcAuthenticator::new(oidc_config, (**pool).clone()))),
        });
        Self {
            config: Arc::new(config),
            clickhouse,
            dagster: Arc::new(dagster),
            embed_secret: Arc::new(embed_secret),
            llm: Arc::new(llm),
            pg,
            auth,
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
