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
/// `smtp_pass`, `database_url`) never appear in a `{:?}`-formatted log
/// line. `database_url` is redacted in full (not field-by-field like
/// `ch_url`/`ch_password`) because Postgres connection strings embed the
/// username and password inline (`postgres://user:pass@host/db`) — there
/// is no separate "password field" to redact around. `AppState`
/// carries an `Arc<Config>` into every handler, so a stray
/// `tracing::debug!(?state)` (or any other ad-hoc debug dump) must not be
/// able to leak these into JSON logs — this repo already runs
/// `check-no-secrets.sh` in CI because a secret leaked once.
#[derive(Clone, PartialEq, Eq)]
pub struct Config {
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
            .field("port", &self.port)
            .field("is_dev", &self.is_dev)
            .field("auth_bootstrap_email", &self.auth_bootstrap_email)
            .field(
                "auth_bootstrap_password",
                &self.auth_bootstrap_password.as_ref().map(|_| REDACTED),
            )
            .field("database_url", &REDACTED)
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
            database_url: or_default(
                env,
                "DATABASE_URL",
                "postgres://lakehouse:lakehouse@localhost:5432/lakehouse",
            ),
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
