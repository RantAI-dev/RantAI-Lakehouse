//! Environment-driven configuration, resolved once at startup.
//!
//! Every default below reproduces a TypeScript `process.env.X ?? "..."` (or,
//! for `LLM_KEY`, `||`) fallback verbatim. The two operators are NOT
//! interchangeable: JavaScript's `??` falls back only when the variable is
//! `undefined` (an explicitly-set empty string is preserved), while `||`
//! also falls back on the empty string. [`Config::llm_key`] is the one field
//! in this module that uses `||` semantics — see
//! `src/services/clients/llm.ts:10`. Every other field uses `??` semantics.
//!
//! Sources ported (see doc comments on each field for the exact line):
//! `src/services/clients/clickhouse.ts`, `src/services/clients/dagster.ts`,
//! `src/services/clients/llm.ts`, `src/services/clients/embed-jwt.ts`,
//! `src/app/api/alerts/run/route.ts`, `src/services/clients/notify.ts`.
//! [`Config::port`] and [`Config::database_url`] are the two exceptions:
//! both have no TypeScript counterpart to port ([`Config::port`] is
//! Rust-only, [`Config::database_url`] is Phase-2-only), and each says so
//! on its own doc comment.

use std::collections::HashMap;

use thiserror::Error;

/// Errors that can occur while resolving [`Config`] from environment
/// variables.
///
/// Only `PORT` can fail resolution: it is load-bearing and Rust-only (the
/// TypeScript backend runs under Next.js and never binds a port itself), so
/// an unparseable value refusing to boot is the correct, Rust-specific
/// failure mode. `SMTP_PORT` deliberately has no equivalent error variant —
/// see the doc comment on [`Config::smtp_port`].
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ConfigError {
    /// `PORT` was set but is not a valid `u16`.
    #[error("PORT must be a valid u16, got {0:?}")]
    InvalidPort(String),
}

/// Resolved application configuration.
///
/// `Debug` is implemented by hand (not derived) so secret fields
/// (`ch_password`, `llm_key`, `embed_secret`, `alerts_run_token`,
/// `smtp_pass`, `database_url`, `lakekeeper_credential_secret_ref`,
/// `rustfs_access_key_secret_ref`, `rustfs_secret_key_secret_ref`) never
/// appear in a `{:?}`-formatted log line. The three `*_secret_ref` fields
/// are references, not values (see `lakehouse_core::secret`'s module doc),
/// but are redacted anyway as defense in depth against a caller pasting a
/// raw secret into a reference field by mistake — the same stance
/// `lakehouse_store::connectors::ConnectorRow` takes for its own
/// `secret_ref` field. `database_url` is redacted in full (not field-by-field like
/// `ch_url`/`ch_password`) because Postgres connection strings embed the
/// username and password inline (`postgres://user:pass@host/db`) — there
/// is no separate "password field" to redact around. `AppState`
/// carries an `Arc<Config>` into every handler, so a stray
/// `tracing::debug!(?state)` (or any other ad-hoc debug dump) must not be
/// able to leak these into JSON logs — this repo already runs
/// `check-no-secrets.sh` in CI because a secret leaked once.
#[derive(Clone, PartialEq, Eq)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "each bool here is an independent, unrelated env-derived toggle \
              (SMTP_SECURE, dev-mode cookie posture, OIDC JIT provisioning, \
              the connector-probe SSRF opt-out) — a state machine or enum \
              would have to model every combination of a set that is not \
              actually a state machine"
)]
pub struct Config {
    /// Whether [`crate::connector_probe`] may dial private/internal address
    /// ranges (RFC1918, loopback, link-local including the cloud metadata
    /// endpoint, IPv6 unique-local) named by a connector's `host`.
    ///
    /// Default `false` — SSRF-safe by default. `POST /api/connectors` lets
    /// the caller choose that `host`, so without this a `connector:manage`
    /// principal gets an internal port scanner with output. This
    /// deployment's own seeded connectors (`postgres:5432`,
    /// `http://rustfs:9000`) ARE internal names, so the compose stack opts
    /// out explicitly — an opt-out rather than a default, so the safe
    /// posture is what a deployment gets unless it says otherwise. `true`
    /// only when the env var is exactly `"true"`.
    pub connector_probe_allow_internal_hosts: bool,
    /// `ClickHouse` HTTP interface URL. Default
    /// `"http://localhost:18123"` (`clickhouse.ts:11`, `??`).
    pub ch_url: String,
    /// `ClickHouse` basic-auth user. Default `"default"`
    /// (`clickhouse.ts:12`, `??`).
    pub ch_user: String,
    /// `ClickHouse` basic-auth password. Default `""`
    /// (`clickhouse.ts:13`, `??` — an explicitly empty value is preserved,
    /// not re-defaulted).
    pub ch_password: String,
    /// Dagster GraphQL endpoint. Default
    /// `"http://localhost:13030/graphql"` (`dagster.ts:6`, `??`).
    pub dagster_url: String,
    /// Dagster repository name. Default `"__repository__"`
    /// (`dagster.ts:7`, `??`).
    pub dagster_repo: String,
    /// Dagster repository location. Default
    /// `"dispar_orchestrate.definitions"` (`dagster.ts:8`, `??`).
    pub dagster_location: String,
    /// LLM chat-completions base URL. Default
    /// `"https://api.minimax.io/v1"` (`llm.ts:7`, `??`).
    pub llm_url: String,
    /// LLM model name. Default `"MiniMax-M3"` (`llm.ts:8`, `??`).
    pub llm_model: String,
    /// LLM API key: `LLM_KEY`, falling back to `MINIMAX_API_KEY`, falling
    /// back to `""` (`llm.ts:10`, `||` — an explicitly *empty* `LLM_KEY`
    /// also falls through to `MINIMAX_API_KEY`, unlike every other field
    /// here).
    pub llm_key: String,
    /// Embed JWT signing secret. `None` when unset (mirrors the truthy
    /// check `if (process.env.EMBED_SECRET)` at `embed-jwt.ts:37`; the
    /// TypeScript then falls back to a generated, `ClickHouse`-persisted
    /// secret, which is out of scope for this chassis).
    pub embed_secret: Option<String>,
    /// Shared token required to call `/api/alerts/run`. `None` when unset,
    /// meaning the endpoint requires no auth (`alerts/run/route.ts:16-19`,
    /// truthy check).
    pub alerts_run_token: Option<String>,
    /// SMTP host. `None` when unset — email delivery is disabled
    /// (`notify.ts:32-33`, truthy check).
    pub smtp_host: Option<String>,
    /// SMTP port. Default `587` (`notify.ts:37`, `??`). An unparseable
    /// `SMTP_PORT` also falls back to `587` (with a `tracing::warn!` naming
    /// the bad value) rather than failing config resolution: the
    /// TypeScript's `Number(process.env.SMTP_PORT ?? 587)` never throws — a
    /// bad value there degrades to a broken email send, not a boot failure.
    /// SMTP is not load-bearing for the API surface, so in a big-bang
    /// cutover a stray `SMTP_PORT=` must not take the whole process down;
    /// `PORT` (below) is the one field that legitimately still hard-fails,
    /// since it is Rust-only and load-bearing.
    pub smtp_port: u16,
    /// Whether to use implicit TLS: the *effective* value nodemailer would
    /// use, matching `notify.ts:40` in full —
    /// `SMTP_SECURE === "true" || port === 465`, not just the raw env var.
    ///
    /// H2: the TS call site ORs in a `port === 465` rule that a bare
    /// `SMTP_SECURE` passthrough would silently drop, leaving it as an
    /// obligation on whatever caller eventually sends email — a caller that
    /// doesn't exist yet, and so has no chance to enforce it. Folding the
    /// rule in here, at config-resolution time (where `smtp_port` is
    /// already known), means this field is always the complete, correct
    /// answer and there is nothing left for a future caller to get wrong.
    pub smtp_secure: bool,
    /// SMTP auth username. `None` when unset, in which case the
    /// TypeScript sends no auth at all (`notify.ts:41`, truthy check).
    pub smtp_user: Option<String>,
    /// SMTP auth password. Default `""` (`notify.ts:41`, `??`).
    pub smtp_pass: String,
    /// SMTP `From` header. `SMTP_FROM`, falling back to `SMTP_USER`,
    /// falling back to `"rantai-lake@localhost"` (`notify.ts:44`, `??`
    /// chain).
    pub smtp_from: String,
    /// Port this service listens on. Default `8080`. Not derived from the
    /// TypeScript (which runs under Next.js), specific to this Rust
    /// service.
    pub port: u16,
    /// Whether this process is running in local development. Controls
    /// ONLY the `Secure` attribute on the auth session cookie (see
    /// `routes::auth`) — never any other request behavior, and NEVER
    /// bypasses authentication itself (there is no such flag in this
    /// codebase; see Task 3.2's report for why `AUTH_DISABLED`-style
    /// escape hatches were rejected).
    ///
    /// Resolved from `APP_ENV`, falling back to `NODE_ENV` (the same
    /// variable the Next.js frontend already sets) — `true` only when the
    /// value is exactly `"development"` or `"local"`; `false` (the safe
    /// default) otherwise, including when both are unset. Failing closed
    /// to `Secure=true` on a misconfigured/unset environment is the right
    /// default: a deployment that forgot to set `APP_ENV` must not
    /// silently ship a cookie that survives plaintext HTTP.
    pub is_dev: bool,
    /// Email for the idempotent bootstrap admin account (see
    /// `main::bootstrap_admin`). `None` when unset — no bootstrap admin is
    /// created, and startup logs instructions for setting this and
    /// [`Self::auth_bootstrap_password`].
    pub auth_bootstrap_email: Option<String>,
    /// Password for the idempotent bootstrap admin account. `None` when
    /// unset. Never logged or rendered — see the [`Config`] type doc
    /// comment.
    pub auth_bootstrap_password: Option<String>,
    /// `OIDC` issuer URL (Task 3.5). `None` when unset — `OIDC` is treated
    /// as unconfigured and [`crate::state::AuthState::oidc`] stays `None`,
    /// exactly as if this field never existed (local password auth keeps
    /// working; nothing fails at boot). Both this AND
    /// [`Self::oidc_client_id`] must be set for `OIDC` to be considered
    /// configured — see `crate::state::AppState::new`.
    pub oidc_issuer: Option<String>,
    /// This application's client id as registered with the `OIDC`
    /// provider. `None` when unset.
    pub oidc_client_id: Option<String>,
    /// `OIDC` client secret. `None` when unset. Reserved for a future
    /// authorization-code exchange (a login-UI concern, out of this task's
    /// scope) — `lakehouse_auth::oidc::OidcAuthenticator` verifies already-
    /// issued bearer tokens against the provider's public JWKS and needs no
    /// shared secret to do that, so this field is parsed (and kept out of
    /// `Debug`) but not currently read by anything.
    pub oidc_client_secret: Option<String>,
    /// A short, operator-chosen label for the configured provider (e.g.
    /// `"okta"`, `"entra"`). Combined with `"oidc:"` to form
    /// `auth_identity.provider`. Default `"default"`.
    pub oidc_provider_name: String,
    /// Explicit override for the JWKS endpoint URL. `None` when unset, in
    /// which case `crate::state::AppState::new` derives
    /// `"{issuer}/.well-known/jwks.json"` — the conventional `OIDC`
    /// discovery-document location every provider this crate documents
    /// (Okta, Entra, Google, Keycloak) publishes at.
    pub oidc_jwks_url: Option<String>,
    /// Whether an unrecognized `OIDC` subject is allowed to provision a new
    /// `app_user`. Default `false` — see
    /// `lakehouse_auth::oidc::OidcConfig::jit_provisioning`'s doc comment
    /// for why the default matters. `true` only when `OIDC_JIT_PROVISIONING`
    /// is exactly `"true"`.
    pub oidc_jit_provisioning: bool,
    /// Maps an `OIDC` group/role claim value to a local `role.name`, parsed
    /// from `OIDC_ROLE_MAP` (e.g.
    /// `"lakehouse-admins=Platform Admin,analysts=Analyst"`). Empty when
    /// unset. See [`parse_role_map`].
    pub oidc_role_map: HashMap<String, String>,
    /// Which claim in an `OIDC` token carries the caller's groups/roles.
    /// Default `"groups"`.
    pub oidc_groups_claim: String,
    /// Clock-skew tolerance (seconds) `OIDC` token validation applies to
    /// `exp`/`nbf`. Default `60`. An unparseable value falls back to the
    /// default (like [`Self::smtp_port`], not load-bearing enough to fail
    /// boot over).
    pub oidc_clock_skew_seconds: u64,
    /// Lakekeeper Iceberg REST catalog base URI (P1,
    /// `lakehouse-iceberg::IcebergClientConfig::catalog_uri`). Default
    /// matches `docker-compose.yml`'s `lakekeeper` service port mapping.
    /// Rust/Phase-1b-only: no TypeScript equivalent, since the original
    /// backend never spoke to a catalog.
    pub lakekeeper_catalog_uri: String,
    /// Lakekeeper warehouse identifier this deployment writes Bronze
    /// tables into. Already the fully-resolved warehouse name — see ADR
    /// 0003 for the `TENANT_ID` → warehouse naming convention; this field
    /// is NOT `TENANT_ID` itself, callers that need the mapping applied
    /// combine `tenant::TENANT_ID` with the convention ADR 0003 defines.
    /// Default `"default"`.
    pub lakekeeper_warehouse: String,
    /// `secretRef` (see `lakehouse_core::secret`) for Lakekeeper's own
    /// `OAuth2` client-credential, when Lakekeeper authorization is enabled.
    /// `None` when unset, meaning Lakekeeper is assumed to be running in
    /// no-auth (open) mode — see the P1b report for R1's status in this
    /// deployment. This field carries a REFERENCE (an `env:VAR_NAME`
    /// string), never a credential value — same guarantee
    /// `lakehouse_store::connectors`'s `secret_ref` field carries, and
    /// resolved the same way, through a
    /// `lakehouse_core::secret::SecretResolver`.
    pub lakekeeper_credential_secret_ref: Option<String>,
    /// S3-compatible object store endpoint backing the Lakekeeper
    /// warehouse (`RustFS` by default; `SeaweedFS` in P2 — see
    /// `docs/STORAGE-COMPATIBILITY.md`, once P2 lands). Default matches
    /// `docker-compose.yml`'s `rustfs` service port mapping.
    pub rustfs_s3_endpoint: String,
    /// S3 region string sent to the object store client. `RustFS` does not
    /// enforce AWS region semantics, but the S3 API requires *a* value.
    /// Default `"us-east-1"`.
    pub rustfs_s3_region: String,
    /// Bucket the lakehouse warehouse's Iceberg tables live under. Default
    /// matches `docker-compose.yml`'s `LAKEHOUSE_WAREHOUSE_BUCKET` default.
    pub lakehouse_warehouse_bucket: String,
    /// `secretRef` for the `RustFS`/S3 access key. Only used as a fallback
    /// when Lakekeeper is not vending per-table credentials (e.g. a direct
    /// `object_store` health check outside the catalog path) — the G1 test
    /// itself must NOT use this field on the write path; see
    /// `lakehouse-iceberg`'s crate doc comment on why vended credentials,
    /// not static ones, are the point. `None` when unset.
    pub rustfs_access_key_secret_ref: Option<String>,
    /// `secretRef` for the `RustFS`/S3 secret key. Same caveat as
    /// [`Self::rustfs_access_key_secret_ref`].
    pub rustfs_secret_key_secret_ref: Option<String>,
    /// Postgres connection string for Phase 2 OLTP storage (`lakehouse-store`).
    /// Default `"postgres://lakehouse:lakehouse@localhost:5432/lakehouse"`
    /// (`??` semantics, like every other URL field here). Rust/Phase-2-only:
    /// no TypeScript equivalent exists, since Phase 1 never wrote to
    /// persistent storage. An unreachable or misconfigured Postgres is
    /// deliberately NOT a boot-time failure — see the doc comment on
    /// `lakehouse_store::connect_lazy` — so, unlike `port`, this field has
    /// no dedicated [`ConfigError`] variant either; whatever string is here
    /// is handed to `connect_lazy` as-is and any problem with it surfaces
    /// lazily, at first use, as an ordinary request-time error.
    pub database_url: String,
    /// ADR 0010/0011 — path to a file holding the `gold-export` Lakekeeper
    /// principal's pre-minted static bearer token (`iceberg-catalog-rest`'s
    /// `token` property — see `lakehouse-iceberg::catalog`'s module doc on
    /// why this build uses a static token, not an `OAuth2` exchange).
    /// Deliberately a **file path**, not a `secretRef`: the token is
    /// minted at compose bring-up by `ops/oidc-mock` onto a shared volume
    /// (`lakehouse_oidc_tokens`) every writer in this stack already reads
    /// directly from disk the same way (`docker-compose.yml`'s
    /// `g1-test-runner`, `debezium-server`, `trino`, ...) — there is no
    /// static value to put behind an `env:` `secretRef` ahead of time.
    /// Always set to a default path — `/tokens/gold-export.jwt`, matching
    /// where `docker-compose.yml`'s `lakehouse-api` service mounts the
    /// shared token volume — rather than `None`-when-unset: unlike a real
    /// secret, there is no meaningful "intentionally absent" state here,
    /// only "the file isn't there (yet)", which `routes::gold` treats as
    /// Gold export being unconfigured (503) at request time, when it
    /// actually tries to read the file.
    pub lakekeeper_gold_export_token_file: String,
    /// `ClickHouse` schema Gold marts live in (ADR 0010: `serving.*`).
    /// `routes::gold`'s export route reads `{gold_source_schema}.{mart}`.
    /// Default `"serving"`.
    pub gold_source_schema: String,
    /// Shared token gating `POST /api/gold/export/{mart}`, same D4 shape
    /// as [`Self::alerts_run_token`]: with this set, a matching
    /// `x-run-token` header/`?token=` is required; with it unset, only a
    /// service-identity principal is let through. `None` when unset.
    pub gold_export_run_token: Option<String>,
}

/// Placeholder shown for secret fields instead of their real value.
const REDACTED: &str = "<redacted>";

impl std::fmt::Debug for Config {
    /// Renders every field verbatim except the secret ones, which are
    /// rendered as `"<redacted>"` regardless of whether they're set — see
    /// the type-level doc comment for why this can't be `#[derive(Debug)]`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config")
            .field("ch_url", &self.ch_url)
            .field("ch_user", &self.ch_user)
            .field("ch_password", &REDACTED)
            .field("dagster_url", &self.dagster_url)
            .field("dagster_repo", &self.dagster_repo)
            .field("dagster_location", &self.dagster_location)
            .field("llm_url", &self.llm_url)
            .field("llm_model", &self.llm_model)
            .field("llm_key", &REDACTED)
            .field(
                "embed_secret",
                &self.embed_secret.as_ref().map(|_| REDACTED),
            )
            .field(
                "alerts_run_token",
                &self.alerts_run_token.as_ref().map(|_| REDACTED),
            )
            .field("smtp_host", &self.smtp_host)
            .field("smtp_port", &self.smtp_port)
            .field("smtp_secure", &self.smtp_secure)
            .field("smtp_user", &self.smtp_user)
            .field("smtp_pass", &REDACTED)
            .field("smtp_from", &self.smtp_from)
            .field("lakekeeper_catalog_uri", &self.lakekeeper_catalog_uri)
            .field("lakekeeper_warehouse", &self.lakekeeper_warehouse)
            .field(
                "lakekeeper_credential_secret_ref",
                &self
                    .lakekeeper_credential_secret_ref
                    .as_ref()
                    .map(|_| REDACTED),
            )
            .field("rustfs_s3_endpoint", &self.rustfs_s3_endpoint)
            .field("rustfs_s3_region", &self.rustfs_s3_region)
            .field(
                "lakehouse_warehouse_bucket",
                &self.lakehouse_warehouse_bucket,
            )
            .field(
                "rustfs_access_key_secret_ref",
                &self.rustfs_access_key_secret_ref.as_ref().map(|_| REDACTED),
            )
            .field(
                "rustfs_secret_key_secret_ref",
                &self.rustfs_secret_key_secret_ref.as_ref().map(|_| REDACTED),
            )
            .field("port", &self.port)
            .field("is_dev", &self.is_dev)
            .field("auth_bootstrap_email", &self.auth_bootstrap_email)
            .field(
                "auth_bootstrap_password",
                &self.auth_bootstrap_password.as_ref().map(|_| REDACTED),
            )
            .field("oidc_issuer", &self.oidc_issuer)
            .field("oidc_client_id", &self.oidc_client_id)
            .field(
                "oidc_client_secret",
                &self.oidc_client_secret.as_ref().map(|_| REDACTED),
            )
            .field("oidc_provider_name", &self.oidc_provider_name)
            .field("oidc_jwks_url", &self.oidc_jwks_url)
            .field("oidc_jit_provisioning", &self.oidc_jit_provisioning)
            .field("oidc_role_map", &self.oidc_role_map)
            .field("oidc_groups_claim", &self.oidc_groups_claim)
            .field(
                "connector_probe_allow_internal_hosts",
                &self.connector_probe_allow_internal_hosts,
            )
            .field("oidc_clock_skew_seconds", &self.oidc_clock_skew_seconds)
            .field("database_url", &REDACTED)
            .field(
                "lakekeeper_gold_export_token_file",
                &self.lakekeeper_gold_export_token_file,
            )
            .field("gold_source_schema", &self.gold_source_schema)
            .field(
                "gold_export_run_token",
                &self.gold_export_run_token.as_ref().map(|_| REDACTED),
            )
            .finish()
    }
}

/// `??`-style lookup: fall back to `default` only when `key` is absent from
/// `env`. A present-but-empty value is returned as-is.
fn or_default(env: &HashMap<String, String>, key: &str, default: &str) -> String {
    env.get(key).cloned().unwrap_or_else(|| default.to_owned())
}

/// Truthy-style lookup, matching JavaScript's `if (process.env.X)`: `None`
/// when `key` is absent *or* present-but-empty.
fn truthy(env: &HashMap<String, String>, key: &str) -> Option<String> {
    env.get(key).filter(|v| !v.is_empty()).cloned()
}

/// Parse `OIDC_ROLE_MAP`'s `"group1=Role One,group2=Role Two"` format into
/// a lookup from external group name to local `role.name`. A malformed
/// entry (no `=`, or an empty group/role name) is skipped rather than
/// failing config resolution — the same "operator-edited free text degrades
/// gracefully" stance `lakehouse_auth::permissions::PermissionSet::parse`
/// takes for `role.permissions`, and for the same reason: a broken mapping
/// entry silently granting nothing is a safer failure mode than refusing to
/// boot over a typo in one group name.
fn parse_role_map(raw: &str) -> HashMap<String, String> {
    raw.split(',')
        .filter_map(|entry| {
            let (group, role) = entry.split_once('=')?;
            let (group, role) = (group.trim(), role.trim());
            if group.is_empty() || role.is_empty() {
                return None;
            }
            Some((group.to_owned(), role.to_owned()))
        })
        .collect()
}

impl Config {
    /// Resolve configuration from an explicit map, for testability without
    /// touching real process environment.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError`] if `PORT` is set to a value that does not
    /// parse as a `u16`. An unparseable `SMTP_PORT` does NOT error — see
    /// [`Config::smtp_port`].
    pub fn from_map(env: &HashMap<String, String>) -> Result<Self, ConfigError> {
        let smtp_port = match env.get("SMTP_PORT") {
            Some(raw) => raw.parse::<u16>().unwrap_or_else(|_| {
                tracing::warn!(
                    smtp_port = %raw,
                    "SMTP_PORT is not a valid u16; falling back to 587"
                );
                587
            }),
            None => 587,
        };
        let port = match env.get("PORT") {
            Some(raw) => raw
                .parse::<u16>()
                .map_err(|_| ConfigError::InvalidPort(raw.clone()))?,
            None => 8080,
        };

        // LLM_KEY || MINIMAX_API_KEY || "" — the one `||` chain in this
        // config; an empty LLM_KEY falls through to MINIMAX_API_KEY.
        let llm_key = truthy(env, "LLM_KEY")
            .or_else(|| truthy(env, "MINIMAX_API_KEY"))
            .unwrap_or_default();

        // SMTP_FROM ?? SMTP_USER ?? "rantai-lake@localhost" — a `??` chain,
        // so an explicitly empty SMTP_FROM is preserved and does NOT fall
        // through to SMTP_USER.
        let smtp_from = env.get("SMTP_FROM").cloned().unwrap_or_else(|| {
            env.get("SMTP_USER")
                .cloned()
                .unwrap_or_else(|| "rantai-lake@localhost".to_owned())
        });

        Ok(Self {
            ch_url: or_default(env, "CH_URL", "http://localhost:18123"),
            ch_user: or_default(env, "CH_USER", "default"),
            ch_password: or_default(env, "CH_PASSWORD", ""),
            dagster_url: or_default(env, "DAGSTER_URL", "http://localhost:13030/graphql"),
            dagster_repo: or_default(env, "DAGSTER_REPO", "__repository__"),
            dagster_location: or_default(env, "DAGSTER_LOCATION", "dispar_orchestrate.definitions"),
            llm_url: or_default(env, "LLM_URL", "https://api.minimax.io/v1"),
            llm_model: or_default(env, "LLM_MODEL", "MiniMax-M3"),
            llm_key,
            embed_secret: truthy(env, "EMBED_SECRET"),
            alerts_run_token: truthy(env, "ALERTS_RUN_TOKEN"),
            smtp_host: truthy(env, "SMTP_HOST"),
            smtp_port,
            // notify.ts:40 in full: `SMTP_SECURE === "true" || port === 465`.
            smtp_secure: env.get("SMTP_SECURE").is_some_and(|v| v == "true") || smtp_port == 465,
            smtp_user: truthy(env, "SMTP_USER"),
            smtp_pass: or_default(env, "SMTP_PASS", ""),
            smtp_from,
            port,
            is_dev: env
                .get("APP_ENV")
                .or_else(|| env.get("NODE_ENV"))
                .is_some_and(|v| v == "development" || v == "local"),
            auth_bootstrap_email: truthy(env, "AUTH_BOOTSTRAP_EMAIL"),
            auth_bootstrap_password: truthy(env, "AUTH_BOOTSTRAP_PASSWORD"),
            oidc_issuer: truthy(env, "OIDC_ISSUER"),
            oidc_client_id: truthy(env, "OIDC_CLIENT_ID"),
            oidc_client_secret: truthy(env, "OIDC_CLIENT_SECRET"),
            oidc_provider_name: or_default(env, "OIDC_PROVIDER_NAME", "default"),
            oidc_jwks_url: truthy(env, "OIDC_JWKS_URL"),
            oidc_jit_provisioning: env
                .get("OIDC_JIT_PROVISIONING")
                .is_some_and(|v| v == "true"),
            oidc_role_map: parse_role_map(env.get("OIDC_ROLE_MAP").map_or("", String::as_str)),
            oidc_groups_claim: or_default(env, "OIDC_GROUPS_CLAIM", "groups"),
            connector_probe_allow_internal_hosts: env
                .get("CONNECTOR_PROBE_ALLOW_INTERNAL_HOSTS")
                .is_some_and(|v| v == "true"),
            oidc_clock_skew_seconds: env
                .get("OIDC_CLOCK_SKEW_SECONDS")
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(60),
            lakekeeper_catalog_uri: or_default(
                env,
                "LAKEKEEPER_CATALOG_URI",
                "http://localhost:8181/catalog",
            ),
            lakekeeper_warehouse: or_default(env, "LAKEKEEPER_WAREHOUSE", "default"),
            lakekeeper_credential_secret_ref: truthy(env, "LAKEKEEPER_CREDENTIAL_SECRET_REF"),
            rustfs_s3_endpoint: or_default(env, "RUSTFS_S3_ENDPOINT", "http://localhost:9010"),
            rustfs_s3_region: or_default(env, "RUSTFS_S3_REGION", "us-east-1"),
            lakehouse_warehouse_bucket: or_default(
                env,
                "LAKEHOUSE_WAREHOUSE_BUCKET",
                "lakehouse-warehouse",
            ),
            rustfs_access_key_secret_ref: truthy(env, "RUSTFS_ACCESS_KEY_SECRET_REF"),
            rustfs_secret_key_secret_ref: truthy(env, "RUSTFS_SECRET_KEY_SECRET_REF"),
            database_url: or_default(
                env,
                "DATABASE_URL",
                "postgres://lakehouse:lakehouse@localhost:5432/lakehouse",
            ),
            lakekeeper_gold_export_token_file: or_default(
                env,
                "LAKEKEEPER_GOLD_EXPORT_TOKEN_FILE",
                "/tokens/gold-export.jwt",
            ),
            gold_source_schema: or_default(env, "GOLD_SOURCE_SCHEMA", "serving"),
            gold_export_run_token: truthy(env, "GOLD_EXPORT_RUN_TOKEN"),
        })
    }

    /// Resolve configuration from the real process environment.
    ///
    /// # Errors
    ///
    /// See [`Config::from_map`].
    pub fn from_env() -> Result<Self, ConfigError> {
        let env: HashMap<String, String> = std::env::vars().collect();
        Self::from_map(&env)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn map(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    /// H1: `{:?}` must never leak a secret value, however it's populated.
    #[test]
    fn debug_redacts_all_secret_fields() {
        let env = map(&[
            ("CH_PASSWORD", "s3cret-ch-pass"),
            ("LLM_KEY", "s3cret-llm-key"),
            ("EMBED_SECRET", "s3cret-embed"),
            ("ALERTS_RUN_TOKEN", "s3cret-alerts-token"),
            ("SMTP_PASS", "s3cret-smtp-pass"),
            (
                "DATABASE_URL",
                "postgres://u:s3cret-pg-pass@db.internal:5432/lakehouse",
            ),
        ]);
        let cfg = Config::from_map(&env).unwrap();
        let rendered = format!("{cfg:?}");
        for secret in [
            "s3cret-ch-pass",
            "s3cret-llm-key",
            "s3cret-embed",
            "s3cret-alerts-token",
            "s3cret-smtp-pass",
            "s3cret-pg-pass",
            "db.internal",
        ] {
            assert!(
                !rendered.contains(secret),
                "Debug output leaked secret {secret:?}: {rendered}"
            );
        }
        assert!(rendered.contains("<redacted>"));
    }

    #[test]
    fn defaults_match_typescript_fallbacks() {
        let cfg = Config::from_map(&HashMap::new()).unwrap();
        assert_eq!(cfg.ch_url, "http://localhost:18123");
        assert_eq!(cfg.ch_user, "default");
        assert_eq!(cfg.ch_password, "");
        assert_eq!(cfg.dagster_url, "http://localhost:13030/graphql");
        assert_eq!(cfg.dagster_repo, "__repository__");
        assert_eq!(cfg.dagster_location, "dispar_orchestrate.definitions");
        assert_eq!(cfg.llm_url, "https://api.minimax.io/v1");
        assert_eq!(cfg.llm_model, "MiniMax-M3");
        assert_eq!(cfg.llm_key, "");
        assert_eq!(cfg.embed_secret, None);
        assert_eq!(cfg.alerts_run_token, None);
        assert_eq!(cfg.smtp_host, None);
        assert_eq!(cfg.smtp_port, 587);
        assert!(!cfg.smtp_secure);
        assert_eq!(cfg.smtp_user, None);
        assert_eq!(cfg.smtp_pass, "");
        assert_eq!(cfg.smtp_from, "rantai-lake@localhost");
        assert_eq!(
            cfg.database_url,
            "postgres://lakehouse:lakehouse@localhost:5432/lakehouse"
        );
        assert_eq!(cfg.lakekeeper_catalog_uri, "http://localhost:8181/catalog");
        assert_eq!(cfg.lakekeeper_warehouse, "default");
        assert_eq!(cfg.lakekeeper_credential_secret_ref, None);
        assert_eq!(cfg.rustfs_s3_endpoint, "http://localhost:9010");
        assert_eq!(cfg.rustfs_s3_region, "us-east-1");
        assert_eq!(cfg.lakehouse_warehouse_bucket, "lakehouse-warehouse");
        assert_eq!(cfg.rustfs_access_key_secret_ref, None);
        assert_eq!(cfg.rustfs_secret_key_secret_ref, None);
        // OIDC unconfigured by default — graceful degradation, see the
        // `oidc_issuer`/`oidc_client_id` field doc comments.
        assert_eq!(cfg.oidc_issuer, None);
        assert_eq!(cfg.oidc_client_id, None);
        assert_eq!(cfg.oidc_client_secret, None);
        assert_eq!(cfg.oidc_provider_name, "default");
        assert_eq!(cfg.oidc_jwks_url, None);
        assert!(!cfg.oidc_jit_provisioning);
        assert!(cfg.oidc_role_map.is_empty());
        assert_eq!(cfg.oidc_groups_claim, "groups");
        assert_eq!(cfg.oidc_clock_skew_seconds, 60);
        assert_eq!(
            cfg.lakekeeper_gold_export_token_file,
            "/tokens/gold-export.jwt"
        );
        assert_eq!(cfg.gold_source_schema, "serving");
        assert_eq!(cfg.gold_export_run_token, None);
    }

    #[test]
    fn oidc_role_map_parses_the_documented_format() {
        let env = map(&[(
            "OIDC_ROLE_MAP",
            "lakehouse-admins=Platform Admin,analysts=Analyst",
        )]);
        let cfg = Config::from_map(&env).unwrap();
        assert_eq!(
            cfg.oidc_role_map
                .get("lakehouse-admins")
                .map(String::as_str),
            Some("Platform Admin")
        );
        assert_eq!(
            cfg.oidc_role_map.get("analysts").map(String::as_str),
            Some("Analyst")
        );
    }

    #[test]
    fn oidc_role_map_skips_malformed_entries() {
        let env = map(&[("OIDC_ROLE_MAP", "no-equals-sign,=empty-group,role-only=")]);
        let cfg = Config::from_map(&env).unwrap();
        assert!(cfg.oidc_role_map.is_empty());
    }

    #[test]
    fn oidc_jit_provisioning_requires_the_exact_string_true() {
        let env = map(&[("OIDC_JIT_PROVISIONING", "TRUE")]);
        let cfg = Config::from_map(&env).unwrap();
        assert!(!cfg.oidc_jit_provisioning);

        let env = map(&[("OIDC_JIT_PROVISIONING", "true")]);
        let cfg = Config::from_map(&env).unwrap();
        assert!(cfg.oidc_jit_provisioning);
    }

    #[test]
    fn invalid_oidc_clock_skew_falls_back_to_default_instead_of_erroring() {
        let env = map(&[("OIDC_CLOCK_SKEW_SECONDS", "not-a-number")]);
        let cfg = Config::from_map(&env).unwrap();
        assert_eq!(cfg.oidc_clock_skew_seconds, 60);
    }

    /// H1 for the new secret field: `{:?}` must never leak
    /// `OIDC_CLIENT_SECRET`.
    #[test]
    fn debug_redacts_oidc_client_secret() {
        let env = map(&[("OIDC_CLIENT_SECRET", "s3cret-oidc-client-secret")]);
        let cfg = Config::from_map(&env).unwrap();
        let rendered = format!("{cfg:?}");
        assert!(!rendered.contains("s3cret-oidc-client-secret"));
    }

    /// H1 for the new `secretRef`-shaped fields: `{:?}` must never leak the
    /// reference string, even though it is not a value — see the type-level
    /// doc comment for why these are redacted anyway.
    #[test]
    fn debug_redacts_secret_ref_fields() {
        let env = map(&[
            (
                "LAKEKEEPER_CREDENTIAL_SECRET_REF",
                "env:LAKEKEEPER_CREDENTIAL",
            ),
            ("RUSTFS_ACCESS_KEY_SECRET_REF", "env:RUSTFS_ACCESS_KEY"),
            ("RUSTFS_SECRET_KEY_SECRET_REF", "env:RUSTFS_SECRET_KEY"),
        ]);
        let cfg = Config::from_map(&env).unwrap();
        let rendered = format!("{cfg:?}");
        for secret_ref in [
            "env:LAKEKEEPER_CREDENTIAL",
            "env:RUSTFS_ACCESS_KEY",
            "env:RUSTFS_SECRET_KEY",
        ] {
            assert!(
                !rendered.contains(secret_ref),
                "Debug output leaked secretRef {secret_ref:?}: {rendered}"
            );
        }
    }

    #[test]
    fn lakekeeper_and_rustfs_fields_are_overridable() {
        let env = map(&[
            (
                "LAKEKEEPER_CATALOG_URI",
                "http://lakekeeper.internal:8181/catalog",
            ),
            ("LAKEKEEPER_WAREHOUSE", "tenant-acme"),
            ("RUSTFS_S3_ENDPOINT", "http://rustfs.internal:9000"),
            ("RUSTFS_S3_REGION", "eu-west-1"),
            ("LAKEHOUSE_WAREHOUSE_BUCKET", "acme-warehouse"),
        ]);
        let cfg = Config::from_map(&env).unwrap();
        assert_eq!(
            cfg.lakekeeper_catalog_uri,
            "http://lakekeeper.internal:8181/catalog"
        );
        assert_eq!(cfg.lakekeeper_warehouse, "tenant-acme");
        assert_eq!(cfg.rustfs_s3_endpoint, "http://rustfs.internal:9000");
        assert_eq!(cfg.rustfs_s3_region, "eu-west-1");
        assert_eq!(cfg.lakehouse_warehouse_bucket, "acme-warehouse");
    }

    #[test]
    fn database_url_is_overridable() {
        let env = map(&[("DATABASE_URL", "postgres://x:y@example.com/z")]);
        let cfg = Config::from_map(&env).unwrap();
        assert_eq!(cfg.database_url, "postgres://x:y@example.com/z");
    }

    #[test]
    fn llm_key_falls_back_to_minimax_api_key() {
        let env = map(&[("MINIMAX_API_KEY", "mk-123")]);
        let cfg = Config::from_map(&env).unwrap();
        assert_eq!(cfg.llm_key, "mk-123");
    }

    #[test]
    fn llm_key_prefers_explicit_llm_key() {
        let env = map(&[("LLM_KEY", "explicit"), ("MINIMAX_API_KEY", "mk-123")]);
        let cfg = Config::from_map(&env).unwrap();
        assert_eq!(cfg.llm_key, "explicit");
    }

    #[test]
    fn empty_llm_key_falls_through_to_minimax() {
        // `||` semantics: an explicitly empty LLM_KEY is falsy in JS, so it
        // falls through to MINIMAX_API_KEY — unlike `??`.
        let env = map(&[("LLM_KEY", ""), ("MINIMAX_API_KEY", "mk-123")]);
        let cfg = Config::from_map(&env).unwrap();
        assert_eq!(cfg.llm_key, "mk-123");
    }

    #[test]
    fn empty_ch_password_is_preserved_not_defaulted() {
        // `??` semantics: an explicitly empty CH_PASSWORD is NOT undefined,
        // so it is kept as-is rather than replaced by a default.
        let env = map(&[("CH_PASSWORD", "")]);
        let cfg = Config::from_map(&env).unwrap();
        assert_eq!(cfg.ch_password, "");
    }

    #[test]
    fn port_defaults_to_8080() {
        let cfg = Config::from_map(&HashMap::new()).unwrap();
        assert_eq!(cfg.port, 8080);
    }

    #[test]
    fn invalid_port_is_an_error() {
        let env = map(&[("PORT", "not-a-number")]);
        let err = Config::from_map(&env).unwrap_err();
        assert_eq!(err, ConfigError::InvalidPort("not-a-number".to_owned()));
    }

    #[test]
    fn smtp_from_falls_back_to_smtp_user_then_literal() {
        let env = map(&[("SMTP_USER", "bot@example.com")]);
        let cfg = Config::from_map(&env).unwrap();
        assert_eq!(cfg.smtp_from, "bot@example.com");
    }

    #[test]
    fn smtp_secure_requires_exact_string_true() {
        let env = map(&[("SMTP_SECURE", "TRUE")]);
        let cfg = Config::from_map(&env).unwrap();
        assert!(!cfg.smtp_secure);
    }

    /// H2: `smtp_secure` is the *effective* nodemailer value —
    /// `notify.ts:40`'s `port === 465` half of the OR must be folded in even
    /// when `SMTP_SECURE` itself is unset.
    #[test]
    fn smtp_secure_is_true_when_port_465_even_without_smtp_secure_env() {
        let env = map(&[("SMTP_PORT", "465")]);
        let cfg = Config::from_map(&env).unwrap();
        assert!(cfg.smtp_secure);
    }

    #[test]
    fn smtp_secure_true_env_wins_regardless_of_port() {
        let env = map(&[("SMTP_SECURE", "true"), ("SMTP_PORT", "25")]);
        let cfg = Config::from_map(&env).unwrap();
        assert!(cfg.smtp_secure);
    }

    /// B3: an unparseable `SMTP_PORT` must NOT prevent the process from
    /// booting — it degrades to the default port (587), matching the
    /// TypeScript's `Number(x ?? 587)`, which never throws. Only `PORT`
    /// (Rust-only, load-bearing) is allowed to hard-fail config resolution.
    #[test]
    fn invalid_smtp_port_falls_back_to_default_instead_of_erroring() {
        let env = map(&[("SMTP_PORT", "nope")]);
        let cfg = Config::from_map(&env).unwrap();
        assert_eq!(cfg.smtp_port, 587);
    }

    #[test]
    fn empty_smtp_port_falls_back_to_default_instead_of_erroring() {
        let env = map(&[("SMTP_PORT", "")]);
        let cfg = Config::from_map(&env).unwrap();
        assert_eq!(cfg.smtp_port, 587);
    }

    /// A wildly out-of-range value (`u16::MAX` + 1) also falls back rather
    /// than erroring, exactly like the empty/non-numeric cases above.
    #[test]
    fn out_of_range_smtp_port_falls_back_to_default_instead_of_erroring() {
        let env = map(&[("SMTP_PORT", "70000")]);
        let cfg = Config::from_map(&env).unwrap();
        assert_eq!(cfg.smtp_port, 587);
    }

    /// `PORT` remains hard-failing: it is Rust-only and load-bearing, unlike
    /// `SMTP_PORT`.
    #[test]
    fn invalid_port_still_hard_fails_config_resolution() {
        let env = map(&[("PORT", "nope")]);
        let err = Config::from_map(&env).unwrap_err();
        assert_eq!(err, ConfigError::InvalidPort("nope".to_owned()));
    }
}
