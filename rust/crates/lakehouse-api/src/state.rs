//! Shared application state, threaded into every axum handler via
//! [`axum::extract::State`].

use std::path::PathBuf;
use std::sync::Arc;

use lakehouse_auth::openfga::LakekeeperAdminClient;
use lakehouse_auth::{
    LocalPasswordAuthenticator, OidcAuthenticator, OidcConfig, Secret, ServiceTokenAuthenticator,
    SessionAuthenticator,
};
use lakehouse_clickhouse::ChClient;
use lakehouse_core::secret::{
    AllowlistedSecretResolver, DynSecretResolver, EnvSecretResolver, FileSecretResolver,
    SecretError, SecretResolver, SecretValue,
};
use lakehouse_dagster::DgClient;
use lakehouse_embed::EmbedSecretResolver;
use lakehouse_iceberg::IcebergClient;
use lakehouse_llm::LlmClient;
use lakehouse_store::PgPool;
use lakehouse_trino::{TrinoClient, TrinoConfig};
use tokio::sync::RwLock;

use crate::bronze_stats_cache::BronzeStatsCache;
use crate::config::Config;
use crate::gold_lock::MartLocks;

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
    /// Resolves a connector's `secretRef` to an actual credential value —
    /// [`crate::connector_probe`]'s only source of one, per ADR 0002 (see
    /// its "Restricting connector secretRefs" addendum). **Deliberately
    /// NOT the general-purpose [`EnvSecretResolver`]** the rest of this
    /// process would use: `connector_probe` dials a `host` the SAME
    /// caller who supplies `secretRef` also controls, so handing it an
    /// unrestricted resolver would let a `connector:manage` principal name
    /// any process secret (`env:DATABASE_URL`, `env:CH_PASSWORD`, ...) and
    /// exfiltrate it to infrastructure they own. Always an
    /// [`AllowlistedSecretResolver`] wrapping a scheme-dispatching
    /// `env:`/`file:` resolver, scoped to exactly
    /// [`CONNECTOR_ALLOWED_SECRET_REF_PATTERNS`] — the credential-suffix
    /// `secretRef` shapes this deployment's OWN seeded connectors use
    /// (`rust/migrations/0022_prune_connector_seed.sql`), nothing else.
    /// `Arc<dyn DynSecretResolver>`, not a concrete type, so a later
    /// external-provider implementation swaps in (still allowlisted)
    /// without changing this field's type or any reader of it.
    pub connector_secret_resolver: Arc<dyn DynSecretResolver>,
    /// The configured authenticators, or `None` under the exact same
    /// condition as [`Self::pg`] being `None` (no Postgres pool). When
    /// `None`, `crate::auth::AuthenticatedPrincipal` and every protected
    /// route reply 503 rather than panic — see
    /// `crate::auth::authenticators`.
    pub auth: Option<AuthState>,
    /// Per-mart single-flight guard for `POST /api/gold/export/{mart}` —
    /// see [`crate::gold_lock`]'s module doc comment for the duplicate-row
    /// defect this closes. Always populated (unlike [`Self::pg`]/
    /// [`Self::auth`]): it needs no external dependency, just an
    /// in-process map.
    pub gold_export_locks: MartLocks,
    /// The shared, lazily-connected Lakekeeper Iceberg REST client used by
    /// `routes::lakehouse` (WS2 §4). Starts empty (`None` inside): the
    /// first `/api/lakehouse/*` request connects it, a failed connect is
    /// never cached, and a stale cached client is dropped and reconnected
    /// once on a `RestError::Catalog` — see `crate::lakehouse_catalog`'s
    /// module doc comment. An `RwLock`, not a plain `Mutex`, because most
    /// requests only need to read the cached client; only a (re)connect
    /// needs exclusive access.
    pub iceberg: Arc<RwLock<Option<Arc<IcebergClient>>>>,
    /// The 60 s TTL cache `routes::catalog::list` uses to enrich Bronze
    /// rows with `sizeBytes`/`freshnessLagSeconds` without a Lakekeeper
    /// round trip on every warm request — see
    /// [`crate::bronze_stats_cache`]'s module doc comment. Always
    /// populated (same pattern as [`Self::gold_export_locks`]): it needs
    /// no external dependency, just an in-process map.
    pub bronze_stats_cache: Arc<BronzeStatsCache>,
    /// `Trino` `/v1/statement` client for `routes::query::run`'s
    /// `engine: "trino"` path (WS2 §4). Always present, like
    /// [`Self::clickhouse`], never `Option` — the `trino` compose profile
    /// is optional, so an unreachable `Trino` is a per-request 503 (see
    /// `lakehouse_trino::TrinoError`'s mapping in `routes::query`), not a
    /// missing field this process has to branch on before every use.
    pub trino: Arc<TrinoClient>,
    /// 15 s cache for `health::probe_all` (WS5, grand plan §7) — never
    /// serves a result older than its own `checked_at` plus the window; a
    /// `services` tile or `/api/ops/services` read past the window always
    /// re-probes. `None` until the first read.
    pub health_cache: crate::health::HealthCache,
    /// The allowlist [`crate::pipeline_source::build_allowlist`] walked
    /// ONCE at startup from `PIPELINE_SOURCE_DIR` (default
    /// `/opt/pipeline-src/dispar_orchestrate` — `rust/Dockerfile`'s `COPY
    /// --from=pipeline_src`, WS4 item C2) — empty (never a panic) when
    /// that directory doesn't exist, e.g. a dev environment without the
    /// baked tree, in which case every `GET /api/pipelines/{id}/source`
    /// request 404s honestly rather than the process failing to boot.
    /// `Arc`, not rebuilt per request: the code location tree this image
    /// ships is fixed for the image's lifetime.
    pub pipeline_source_allowlist: Arc<crate::pipeline_source::SourceAllowlist>,
    /// Ring buffer of the last [`POLICY_DECISION_LATENCY_CAP`]
    /// `sql_rewrite::enforce` prefetch+rewrite durations (WS7 item C2) —
    /// `routes::query::run` records one sample per call, refused or not;
    /// `GET /api/ops/observability`'s `policyDecisionP95Ms` computes a
    /// real p95 from it instead of the literal `null` WS1's honesty pass
    /// left in place. Always populated (same pattern as
    /// [`Self::gold_export_locks`]/[`Self::bronze_stats_cache`]): needs no
    /// external dependency, just an in-process buffer.
    pub policy_decision_latencies: PolicyDecisionLatencies,
    /// Client for Lakekeeper's `management/v1/*` API, used by `POST
    /// /api/identity/tenants` to provision a tenant's
    /// warehouse and grant this stack's machine principals onto it — see
    /// `lakehouse_auth::openfga::LakekeeperAdminClient`'s module doc
    /// comment. `None` when
    /// `Config::lakekeeper_admin_token_file` is unreadable at startup
    /// (not provisioned on this deployment, wrong path, permissions):
    /// same degrade-honestly posture `bootstrap_agent_run_service`
    /// (`main.rs`) uses for its own missing-token case — a caller sees
    /// tenant provisioning as unavailable rather than the whole process
    /// panicking at boot or silently dialing Lakekeeper with an
    /// empty-string token that would only surface as a confusing 401
    /// later, deep inside a provisioning call.
    ///
    /// First (and, so far, only) read by
    /// `routes::identity::create_tenant` — the `#[allow(dead_code)]` this
    /// field carried while nothing read it yet is removed now that a
    /// reader exists.
    pub lakekeeper_admin: Option<Arc<LakekeeperAdminClient>>,
}

/// Build the admin-scoped [`LakekeeperAdminClient`] from the token file at
/// `token_path`, or `None` when this deployment has no usable admin token.
///
/// An EMPTY (or whitespace-only) file counts as "no token", not as a token
/// that happens to be the empty string: `read_to_string` succeeds on an
/// empty file, so without this check the process would build a client that
/// sends `Authorization: Bearer ` on every provisioning call and fail at
/// Lakekeeper with a 401 that names nothing — the exact confusing failure
/// the degrade-to-`None` path exists to avoid. A missing or unreadable file
/// degrades the same way.
fn lakekeeper_admin_client(token_path: &str, base_url: &str) -> Option<LakekeeperAdminClient> {
    match std::fs::read_to_string(token_path) {
        Ok(raw) if !raw.trim().is_empty() => Some(LakekeeperAdminClient::new(
            base_url.to_owned(),
            Secret::new(raw.trim().to_owned()),
        )),
        Ok(_) => {
            tracing::warn!(
                path = %token_path,
                "Lakekeeper admin token file is present but empty; tenant provisioning (POST /api/identity/tenants) will report itself unavailable"
            );
            None
        }
        Err(err) => {
            tracing::warn!(
                %err,
                path = %token_path,
                "Lakekeeper admin token file is unreadable; tenant provisioning (POST /api/identity/tenants) will report itself unavailable"
            );
            None
        }
    }
}

/// The cap [`PolicyDecisionLatencies`] keeps — old samples are evicted
/// oldest-first once the buffer is full, so the reported p95 always
/// reflects recent behavior rather than growing unbounded for the life of
/// the process.
const POLICY_DECISION_LATENCY_CAP: usize = 1000;

/// Thread-safe ring buffer of policy-decision durations, shared (via
/// [`Arc`]) across every clone of [`AppState`] so every request records
/// into, and every `/api/ops/observability` read sees, the SAME buffer.
///
/// A plain `Mutex<VecDeque<Duration>>`, not a lock-free structure: this is
/// written once per `routes::query::run` call and read at most once per
/// `/api/ops/observability` request — nowhere near a contention hot path
/// (WS7 item C2 Step 2's own "implementer's choice, documented").
#[derive(Clone, Default)]
pub struct PolicyDecisionLatencies(
    Arc<std::sync::Mutex<std::collections::VecDeque<std::time::Duration>>>,
);

impl PolicyDecisionLatencies {
    /// Records one duration, evicting the oldest sample once the buffer
    /// holds [`POLICY_DECISION_LATENCY_CAP`] entries. A poisoned lock (a
    /// prior panic while holding it, unreachable in practice — nothing in
    /// this module panics) still recovers its inner guard rather than
    /// panicking here too, since losing one latency sample is far less
    /// harmful than crashing every future request that records one.
    pub fn record(&self, elapsed: std::time::Duration) {
        let mut buf = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if buf.len() >= POLICY_DECISION_LATENCY_CAP {
            buf.pop_front();
        }
        buf.push_back(elapsed);
    }

    /// The real p95, in milliseconds, over every sample recorded so far.
    /// `None` when no query has run yet — "no data" is not "0ms", which
    /// would read as a real, excellent measurement rather than "not
    /// measured" (the same honesty rule every other `None`-capable metric
    /// in `routes::ops` already follows).
    #[must_use]
    pub fn p95_ms(&self) -> Option<f64> {
        let buf = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if buf.is_empty() {
            return None;
        }
        let mut millis: Vec<f64> = buf
            .iter()
            .map(std::time::Duration::as_secs_f64)
            .map(|s| s * 1000.0)
            .collect();
        millis.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::cast_precision_loss,
            reason = "millis.len() is bounded by POLICY_DECISION_LATENCY_CAP (1000), \
                      nowhere near usize/f64 precision limits"
        )]
        let idx = ((millis.len() as f64) * 0.95).ceil() as usize;
        let idx = idx.saturating_sub(1).min(millis.len() - 1);
        Some(millis[idx])
    }
}

/// The credential-suffix `secretRef` PATTERNS (see
/// [`lakehouse_core::secret::pattern_matches`])
/// [`AppState::connector_secret_resolver`] may resolve — see that field's
/// doc comment and ADR 0002's addendum. This is deliberately a fixed,
/// hardcoded list, not derived from configuration: widening it is a code
/// change to make and review, not something a request or an environment
/// variable can do — a `connector:manage` principal must never be able to
/// expand their own reach.
///
/// PATTERNS, not exact strings (WS3 plan review X1): the prior list was
/// three exact `env:CONNECTOR_*` names, and a bare `env:CONNECTOR_*` prefix
/// check (considered and rejected — see [`lakehouse_core::secret::pattern_matches`]'s
/// doc comment) would have admitted `env:CONNECTOR_PROBE_ALLOW_INTERNAL_HOSTS`,
/// a real [`Config`] flag, not a credential. Every pattern here ends in a
/// credential-shaped suffix (`_PASSWORD`, `_SECRET_KEY`, `_ACCESS_KEY`,
/// `_API_KEY`, `_TOKEN`), or is the single `file:` pattern below, so a
/// non-credential `CONNECTOR_*` config variable can never match no matter
/// what gets added under that prefix later.
///
/// These are CONNECTOR-DEDICATED variables, deliberately NOT the API's own
/// secrets. The earlier list named `env:POSTGRES_PASSWORD` (the console's
/// own database password) and `env:RUSTFS_ACCESS_KEY`/`env:RUSTFS_SECRET_KEY`
/// (the object store's root keys). Allowlisting those made the guard far
/// weaker than it read: `POST /api/connectors` takes a caller-chosen `host`,
/// and `connector_probe`'s SSRF guard blocks only internal ranges, so a
/// `connector:manage` principal could point a connector at infrastructure
/// they own and have the API authenticate to it with the console's real
/// database password. The allowlist stopped arbitrary refs; it did not stop
/// the refs that mattered most.
///
/// A deployment MAY set these equal to the real credentials — that is its
/// choice to make explicitly, in its own environment — but the API's own
/// secrets are no longer reachable through a connector by name. As of ADR
/// 0002 Addendum 3, a USER-created connector cannot name a ref at all — it
/// only chooses a source/kind, and the server derives the actual name from
/// the connector's own generated id
/// (`lakehouse_store::connectors::derive_secret_ref`), which always begins
/// `CONNECTOR_CONN_`/`connector_conn_` and so can never match one of these
/// reserved, seeded patterns. That makes this narrower than the runtime
/// check it replaced: naming a reserved ref is now structurally
/// impossible for a user-created connector, not merely refused at write
/// time.
pub(crate) const CONNECTOR_ALLOWED_SECRET_REF_PATTERNS: [&str; 7] = [
    "env:CONNECTOR_*_PASSWORD",
    "env:CONNECTOR_*_SECRET_KEY",
    "env:CONNECTOR_*_ACCESS_KEY",
    "env:CONNECTOR_*_API_KEY",
    "env:CONNECTOR_*_TOKEN",
    // `sftp`'s `SftpAuth::PublicKey` auth kind derives this suffix
    // (`CredentialKind::PrivateKey`, `lakehouse_store::connectors`) — keep
    // this list and `dagster/dispar_orchestrate/secret_resolver.py`'s
    // identical, same discipline as that module's own header comment.
    "env:CONNECTOR_*_PRIVATE_KEY",
    "file:/run/secrets/connector_*",
];

/// Dispatches a `secretRef` to [`EnvSecretResolver`] (`env:` scheme) or
/// [`FileSecretResolver`] (`file:` scheme) by its prefix, so
/// [`AppState::connector_secret_resolver`] can be a single
/// [`AllowlistedSecretResolver`] covering both schemes instead of the caller
/// having to pick a resolver before the allowlist check ever runs.
///
/// An unrecognized scheme is [`SecretError::UnsupportedRef`] here, at the
/// combining layer, rather than falling through silently — the same
/// fail-closed shape [`EnvSecretResolver`] and [`FileSecretResolver`] each
/// use for their own scheme mismatch.
#[derive(Debug)]
struct ConnectorSecretResolver {
    env: EnvSecretResolver,
    file: FileSecretResolver,
}

impl ConnectorSecretResolver {
    fn new() -> Self {
        Self {
            env: EnvSecretResolver::new(),
            // `/run/secrets` is the fixed Docker/Compose secrets mount this
            // deployment uses — see `FileSecretResolver`'s doc comment for
            // why a fixed base directory (not caller-supplied) matters.
            file: FileSecretResolver::new("/run/secrets"),
        }
    }
}

impl SecretResolver for ConnectorSecretResolver {
    async fn resolve(&self, secret_ref: &str) -> Result<SecretValue, SecretError> {
        if secret_ref.starts_with(lakehouse_core::secret::ENV_SECRET_REF_PREFIX) {
            self.env.resolve(secret_ref).await
        } else if secret_ref.starts_with(lakehouse_core::secret::FILE_SECRET_REF_PREFIX) {
            self.file.resolve(secret_ref).await
        } else {
            Err(SecretError::UnsupportedRef {
                secret_ref: secret_ref.to_owned(),
                resolver: "connector-scheme",
            })
        }
    }
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
        // `TrinoConfig::new` supplies the documented `max_rows`/`paging_cap`
        // defaults; only `base_url` and `max_rows` are overridden from
        // [`Config`] here — `paging_cap` has no env-derived override yet.
        let trino = TrinoClient::new(TrinoConfig {
            max_rows: config.trino_max_rows,
            ..TrinoConfig::new(config.trino_url.clone())
        });
        // `PIPELINE_SOURCE_DIR` — default `/opt/pipeline-src/dispar_orchestrate`,
        // `rust/Dockerfile`'s `COPY --from=pipeline_src`, WS4 item C2. Its
        // FINAL COMPONENT is the code location's Python package name, and
        // `build_allowlist` keys every file under it (a deployment baking
        // a differently-named code location points this at that package's
        // directory; see `pipeline_source::build_allowlist`). A dev
        // environment without the baked tree degrades to an empty
        // allowlist (every `/source` request then 404s, honestly, rather
        // than the process failing to boot at all).
        let pipeline_source_base = std::env::var("PIPELINE_SOURCE_DIR").map_or_else(
            |_| PathBuf::from("/opt/pipeline-src/dispar_orchestrate"),
            PathBuf::from,
        );
        let pipeline_source_allowlist = match crate::pipeline_source::build_allowlist(
            &pipeline_source_base,
        ) {
            Ok(allowlist) => allowlist,
            Err(err) => {
                tracing::warn!(
                    ?err,
                    path = %pipeline_source_base.display(),
                    "pipeline source tree not found or unreadable; GET /api/pipelines/{{id}}/source will 404 for every op"
                );
                crate::pipeline_source::SourceAllowlist::new()
            }
        };
        // Read the admin-scoped Lakekeeper bearer token
        // synchronously (`std::fs::read_to_string`, not
        // `crate::lakekeeper_token::read_token_file`): that helper is
        // `async` (it targets per-request reads, e.g. `routes::gold`'s
        // token read), while `AppState::new` is a plain sync fn called
        // from `main.rs` without an executor available yet — making it
        // `async` would ripple `.await` into every `AppState::new` call
        // site in `main.rs`/`tests/*.rs`, files this task does not own.
        // A missing/unreadable file degrades to `None`, never a panic or
        // an empty-string token — same posture `bootstrap_agent_run_service`
        // (`main.rs`) uses when its own token is unset.
        let lakekeeper_admin = lakekeeper_admin_client(
            &config.lakekeeper_admin_token_file,
            &config.lakekeeper_base_url,
        )
        .map(Arc::new);
        Self {
            config: Arc::new(config),
            clickhouse,
            dagster: Arc::new(dagster),
            embed_secret: Arc::new(embed_secret),
            llm: Arc::new(llm),
            pg,
            connector_secret_resolver: Arc::new(AllowlistedSecretResolver::new(
                ConnectorSecretResolver::new(),
                CONNECTOR_ALLOWED_SECRET_REF_PATTERNS
                    .iter()
                    .map(|s| (*s).to_owned()),
                "connector-allowlist",
            )),
            auth,
            gold_export_locks: MartLocks::default(),
            iceberg: Arc::new(RwLock::new(None)),
            bronze_stats_cache: Arc::new(BronzeStatsCache::new()),
            trino: Arc::new(trino),
            health_cache: Arc::new(tokio::sync::Mutex::new(None)),
            pipeline_source_allowlist: Arc::new(pipeline_source_allowlist),
            policy_decision_latencies: PolicyDecisionLatencies::default(),
            lakekeeper_admin,
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::collections::HashMap;
    use std::time::Duration;

    use super::*;

    // ── WS7 item C2: `policyDecisionP95Ms` is a real measurement ────────

    #[test]
    fn policy_decision_latencies_reports_none_when_empty() {
        let latencies = PolicyDecisionLatencies::default();
        assert_eq!(latencies.p95_ms(), None);
    }

    #[test]
    fn policy_decision_latencies_computes_a_real_p95_from_recorded_samples() {
        let latencies = PolicyDecisionLatencies::default();
        for ms in [10, 20, 30, 40, 100] {
            latencies.record(Duration::from_millis(ms));
        }
        // Sorted: [10, 20, 30, 40, 100]; p95 index = ceil(5 * 0.95) - 1 = 4
        // -> the largest sample.
        assert_eq!(latencies.p95_ms(), Some(100.0));
    }

    #[test]
    fn policy_decision_latencies_evicts_the_oldest_sample_once_full() {
        let latencies = PolicyDecisionLatencies::default();
        // Fill past the cap with a huge outlier, then push it out with
        // small samples — the outlier must be evicted, not retained
        // forever.
        latencies.record(Duration::from_secs(3600));
        for _ in 0..POLICY_DECISION_LATENCY_CAP {
            latencies.record(Duration::from_millis(1));
        }
        assert_eq!(latencies.p95_ms(), Some(1.0));
    }

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

    /// The regression guard for WS3 plan review X1: a credential-suffixed
    /// `secretRef` — including one Phase E's new adapters need
    /// (`env:CONNECTOR_MYSQL_PASSWORD`), not only the two dedicated refs
    /// migrations 0022/0023 seed — must be ADMITTED by the allowlist, while
    /// the API's own secrets and the `CONNECTOR_PROBE_ALLOW_INTERNAL_HOSTS`
    /// config flag (a real `Config` field, not a credential) must be
    /// REFUSED. The assertion is on the ERROR VARIANT, not merely
    /// `is_err()`: [`SecretError::NotFound`] means "allowed, but this test
    /// process has no such env var set"; [`SecretError::NotAllowed`] means
    /// "refused by the allowlist before ever looking". A test that only
    /// checked `is_err()` would keep passing even if the allowlist were
    /// deleted outright, since an unset env var is `NotFound` either way.
    #[tokio::test]
    async fn connector_secret_resolver_admits_credential_suffixed_refs_but_refuses_the_apis_own_secrets_and_the_probe_flag()
     {
        let cfg = Config::from_map(&HashMap::new()).unwrap();
        let state = AppState::new(cfg);

        for admitted in [
            "env:CONNECTOR_PG_PASSWORD",
            "env:CONNECTOR_S3_ACCESS_KEY",
            "env:CONNECTOR_S3_SECRET_KEY",
            "env:CONNECTOR_MYSQL_PASSWORD",
        ] {
            let err = state
                .connector_secret_resolver
                .resolve_dyn(admitted)
                .await
                .unwrap_err();
            assert!(
                matches!(err, lakehouse_core::secret::SecretError::NotFound { .. }),
                "{admitted} must be allowed by the pattern allowlist (NotFound, not \
                 NotAllowed, since the env var is unset in this test process); got {err:?}"
            );
        }

        for forbidden in [
            "env:POSTGRES_PASSWORD",
            "env:DATABASE_URL",
            "env:CH_PASSWORD",
            "env:CONNECTOR_PROBE_ALLOW_INTERNAL_HOSTS",
        ] {
            let err = state
                .connector_secret_resolver
                .resolve_dyn(forbidden)
                .await
                .unwrap_err();
            assert!(
                matches!(err, lakehouse_core::secret::SecretError::NotAllowed { .. }),
                "{forbidden} must be refused by the allowlist; got {err:?}"
            );
        }
    }

    /// `read_to_string` SUCCEEDS on an empty
    /// file, so "unreadable file degrades to `None`" alone does not cover
    /// the case an operator actually hits — a token file created by a mount
    /// or an init container that wrote nothing. Without this, the process
    /// would dial `Lakekeeper` with `Authorization: Bearer ` and the
    /// failure would surface as an unexplained 401 inside a provisioning
    /// call instead of "provisioning is unavailable here".
    #[test]
    fn an_empty_lakekeeper_admin_token_file_is_treated_as_no_token() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("admin.jwt");
        std::fs::write(&path, "   \n").unwrap();
        assert!(
            super::lakekeeper_admin_client(
                path.to_str().expect("a UTF-8 temp path"),
                "http://lakekeeper.invalid:8181"
            )
            .is_none()
        );
    }

    #[test]
    fn a_missing_lakekeeper_admin_token_file_is_treated_as_no_token() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("not-written.jwt");
        assert!(
            super::lakekeeper_admin_client(
                path.to_str().expect("a UTF-8 temp path"),
                "http://lakekeeper.invalid:8181"
            )
            .is_none()
        );
    }

    #[test]
    fn a_real_lakekeeper_admin_token_file_builds_a_client() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("admin.jwt");
        std::fs::write(&path, "  header.payload.signature\n").unwrap();
        assert!(
            super::lakekeeper_admin_client(
                path.to_str().expect("a UTF-8 temp path"),
                "http://lakekeeper.invalid:8181"
            )
            .is_some()
        );
    }
}
