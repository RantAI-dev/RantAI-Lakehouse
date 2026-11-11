//! Effective deployment settings, resolved in exactly one place and read
//! through a short cache (`ADM-6` to `ADM-9`, slices 1 and 2 of
//! `docs/superpowers/plans/2026-11-06-adm-6-9-console-settings.md`).
//!
//! # One merge rule
//!
//! For each field the effective value is the environment's when it is set
//! and non-empty, else the console's saved value, else the code default
//! (plan D1). That precedence is implemented ONLY in this module: the
//! pre-`ADM-7` merge (`routes::alerts::smtp_config`, six fields read
//! straight from [`Config`]) is gone, and every reader — every sender, the
//! login throttle, the retention sweep, the embed lifetime and the
//! assistant's client — asks this module, so there is no second place where
//! environment and saved values are combined (rule 4: no duplicated guard).
//!
//! # The units
//!
//! Two groups of settings are ONE unit each, because a per-field merge would
//! let a secret kept in the server's files be sent to an address the console
//! chose (`ADM-7 review R1`):
//!
//! * the email connection — the host, the port, the security, the account
//!   and the password: when the environment sets any one of them, all five
//!   are the server's and the console may change none of them;
//! * the assistant's provider — the address, the model and the key: when
//!   the environment holds a key (`LLM_KEY` or `MINIMAX_API_KEY`), all three
//!   are the server's, so the environment's key is never sent to an address
//!   the console chose (`ADM-9`). With no key in the environment the key is
//!   the console's, and the address and the model are per field: an
//!   environment that sets `LLM_URL` owns the address alone, and one that
//!   sets `LLM_MODEL` owns the model alone (`ADM-9 review R4`).
//!
//! # The cache
//!
//! [`SettingsCache`] holds a saved row for at most [`SETTINGS_CACHE_TTL`] per
//! process (a `tokio::sync::RwLock` with the read time). The email `PUT` (the
//! console's email screen) drops the cached row at once, so a save on this
//! process is visible to the next send without waiting for the TTL. Other
//! processes re-read within the TTL; pushing a change across processes faster
//! than 30 s is out of scope (plan §3).
//!
//! # Secrets
//!
//! A resolved secret is held as a [`SecretValue`] (redacting `Debug`, no
//! `Serialize`). It never appears in a log, an error or a response; a GET
//! answers `set` only, and the audit row records only the field NAME. The
//! saved password and key are sealed with the installation keyring under
//! [`EMAIL_PASSWORD_SEAL_NAME`] and [`ASSISTANT_KEY_SEAL_NAME`]; a row that
//! does not open (the key changed) is read as "not set" and logged, never
//! returned.

use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use lakehouse_auth::throttle::ThrottlePolicy;
use lakehouse_core::secret::SecretValue;
use lakehouse_llm::LlmClient;
use lakehouse_notify::{SmtpCaBundle, SmtpConfig};
use lakehouse_store::assistant_settings::AssistantSettings;
use lakehouse_store::email_settings::EmailSettings;
use lakehouse_store::sign_in_settings::SignInSettings;
use lakehouse_store::sso_settings::SsoSettings;
use lakehouse_store::{PgPool, StoreError};
use tokio::sync::{Mutex, RwLock};
use uuid::Uuid;

use crate::config::Config;
use crate::internal_hosts::InternalHosts;
use crate::secret_crypto::Keyring;
use crate::state::AppState;

/// How long one read of a saved settings row is reused before the store is
/// asked again (plan D3).
pub const SETTINGS_CACHE_TTL: Duration = Duration::from_secs(30);

/// How long one read of the saved email row is reused before the store is
/// asked again (plan D3).
pub const EMAIL_CACHE_TTL: Duration = SETTINGS_CACHE_TTL;

/// How long one built SSO runtime that can sign someone in is reused
/// (`ADM-12` review round 2, R2-S1).
///
/// A proposed number from that review: a working connection costs a
/// discovery read, four address look-ups and a fresh key-set cache to
/// rebuild, so it is kept for ten minutes; a connection that is *not* ready
/// is still retried after [`SETTINGS_CACHE_TTL`], so a provider that comes
/// back becomes usable without a save. A save clears either at once.
pub const READY_SSO_RUNTIME_TTL: Duration = Duration::from_secs(600);

/// The associated-data name the email password is sealed under (plan D2).
pub const EMAIL_PASSWORD_SEAL_NAME: &str = "settings/email/password";

/// The associated-data name the assistant's key is sealed under (plan D2).
pub const ASSISTANT_KEY_SEAL_NAME: &str = "settings/assistant/key";

/// The associated-data name the SSO client secret is sealed under (plan D2).
pub const SSO_CLIENT_SECRET_SEAL_NAME: &str = "settings/sso/client_secret";

/// Where a field's effective value comes from (plan D1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// Set in the server's environment; the console may not change it.
    Server,
    /// Saved from the console.
    Console,
    /// Neither: the code default.
    Default,
}

impl Source {
    /// The wire string the console reads: which of the three it is.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Server => "server",
            Self::Console => "console",
            Self::Default => "default",
        }
    }
}

/// The two ways the sender may secure the connection, matching the console's
/// `security` field (`"tls"` = implicit TLS, `"starttls"` = STARTTLS).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmailSecurity {
    /// Implicit TLS from the first byte (`SMTP_SECURE` / port 465).
    Tls,
    /// `STARTTLS` is required, never plaintext (`SEC-56` D3).
    Starttls,
}

impl EmailSecurity {
    /// The wire string the console reads and sends back.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Tls => "tls",
            Self::Starttls => "starttls",
        }
    }

    /// The `SmtpConfig::secure` boolean this maps onto.
    #[must_use]
    pub const fn secure(self) -> bool {
        matches!(self, Self::Tls)
    }

    /// Parse the console's value; `None` for anything else (the table's
    /// `CHECK` already closes it to the two, this is the shape backstop).
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "tls" => Some(Self::Tls),
            "starttls" => Some(Self::Starttls),
            _ => None,
        }
    }
}

/// The resolved email settings: the effective value of every field plus
/// where it came from, and the CA bundle read at start-up (never a console
/// setting).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveEmail {
    /// The mail server host; `None` means email is not set up.
    pub host: Option<String>,
    /// Where [`Self::host`] came from.
    pub host_source: Source,
    /// The mail server port, 1 to 65535.
    pub port: u16,
    /// Where [`Self::port`] came from.
    pub port_source: Source,
    /// How the connection is secured (implicit TLS, or required STARTTLS).
    pub security: EmailSecurity,
    /// Where [`Self::security`] came from.
    pub security_source: Source,
    /// The SMTP auth user; `None` means no auth is sent.
    pub user: Option<String>,
    /// Where [`Self::user`] came from.
    pub user_source: Source,
    /// The resolved SMTP auth password, as a [`SecretValue`]: redacting
    /// `Debug`, no `Serialize`, never logged or returned.
    pub password: SecretValue,
    /// Whether a password is in effect at all (the value itself is never
    /// serialized).
    pub password_set: bool,
    /// Where the password came from.
    pub password_source: Source,
    /// The `From` address.
    pub from: String,
    /// Where [`Self::from`] came from.
    pub from_source: Source,
    /// The console's public address, used in emailed links.
    pub console_public_url: Option<String>,
    /// Where [`Self::console_public_url`] came from.
    pub console_public_url_source: Source,
    /// The CA bundle read from `SMTP_CA_FILE` at start-up (`SEC-56`); not a
    /// console setting, so it has no source.
    pub ca_bundle: Option<SmtpCaBundle>,
}

impl EffectiveEmail {
    /// The [`SmtpConfig`] a sender consumes. The password is materialized
    /// here, from the [`SecretValue`], only because [`SmtpConfig`]'s shape
    /// predates this resolver; it is never logged or returned.
    #[must_use]
    pub fn smtp(&self) -> SmtpConfig {
        SmtpConfig {
            host: self.host.clone(),
            port: self.port,
            secure: self.security.secure(),
            user: self.user.clone(),
            pass: self.password.expose_secret().to_owned(),
            from: self.from.clone(),
            ca_bundle: self.ca_bundle.clone(),
        }
    }
}

/// A saved settings row the API caches (plan D3).
///
/// Each section's row is read at most once per [`SETTINGS_CACHE_TTL`] per
/// process. One trait, so the three sections (email, sign-in, assistant)
/// share one cache and one read rule instead of three copies of it.
pub trait SettingsRow: Clone + Send + Sync + 'static {
    /// Read the saved row from the store.
    fn read(pool: &PgPool) -> impl Future<Output = Result<Option<Self>, StoreError>> + Send;
}

/// The saved row of one settings section, cached for [`SETTINGS_CACHE_TTL`]
/// (plan D3). Cheap to clone: one `Arc`-shared [`RwLock`].
#[derive(Debug)]
pub struct SettingsCache<T: SettingsRow> {
    inner: RwLock<Option<(Instant, Option<T>)>>,
}

impl<T: SettingsRow> SettingsCache<T> {
    /// An empty cache.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(None),
        }
    }

    /// The saved row, from the cache when it is fresh, else the store. The
    /// row is cloned out, so the returned value owns its data.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Database`] when the store read fails.
    pub async fn row(&self, pool: &PgPool) -> Result<Option<T>, StoreError> {
        if let Some(row) = self.fresh_row().await {
            return Ok(row);
        }
        let mut guard = self.inner.write().await;
        if let Some((at, row)) = guard.as_ref()
            && at.elapsed() < SETTINGS_CACHE_TTL
        {
            return Ok(row.clone());
        }
        let row = T::read(pool).await?;
        *guard = Some((Instant::now(), row.clone()));
        Ok(row)
    }

    /// The cached row when it is still within [`SETTINGS_CACHE_TTL`].
    async fn fresh_row(&self) -> Option<Option<T>> {
        let guard = self.inner.read().await;
        let (at, row) = guard.as_ref()?;
        (at.elapsed() < SETTINGS_CACHE_TTL).then(|| row.clone())
    }

    /// Drop the cached row, so the next read asks the store again. A
    /// settings `PUT` calls this on the process that saved, so a change is
    /// visible to the next reader there without waiting for the TTL (plan
    /// D3).
    pub async fn clear(&self) {
        *self.inner.write().await = None;
    }
}

impl<T: SettingsRow> Default for SettingsCache<T> {
    fn default() -> Self {
        Self::new()
    }
}

/// The saved email row, cached for [`SETTINGS_CACHE_TTL`] (plan D3).
pub type EmailSettingsCache = SettingsCache<EmailSettings>;

impl SettingsRow for EmailSettings {
    fn read(pool: &PgPool) -> impl Future<Output = Result<Option<Self>, StoreError>> + Send {
        lakehouse_store::email_settings::get(pool)
    }
}

impl SettingsRow for SignInSettings {
    fn read(pool: &PgPool) -> impl Future<Output = Result<Option<Self>, StoreError>> + Send {
        lakehouse_store::sign_in_settings::get(pool)
    }
}

impl SettingsRow for AssistantSettings {
    fn read(pool: &PgPool) -> impl Future<Output = Result<Option<Self>, StoreError>> + Send {
        lakehouse_store::assistant_settings::get(pool)
    }
}

/// The saved SSO settings row, cached for [`SETTINGS_CACHE_TTL`] (plan D1/D3).
pub type SsoSettingsCache = SettingsCache<SsoSettings>;

impl SettingsRow for SsoSettings {
    fn read(pool: &PgPool) -> impl Future<Output = Result<Option<Self>, StoreError>> + Send {
        lakehouse_store::sso_settings::get(pool)
    }
}

/// How often one caller may ask for a test email (`ADM-7` plan D10).
pub const TEST_EMAIL_INTERVAL: Duration = Duration::from_secs(10);

/// How often each caller may ask for a test email (`ADM-7` plan D10: at most
/// one per caller per [`TEST_EMAIL_INTERVAL`]).
///
/// Per process, and per caller: the counter keeps the button from turning
/// into a mail cannon, and says so with a `429` and a `Retry-After`. It is
/// not a shared quota, so two API processes each allow their own.
#[derive(Debug, Default)]
pub struct TestEmailLimiter {
    /// When each caller last asked.
    last: Mutex<HashMap<Uuid, Instant>>,
}

impl TestEmailLimiter {
    /// A limiter no caller has used yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Claim the right to send a test email to `caller` at `now`.
    ///
    /// `Ok(())` when the caller may send now; `Err(seconds)` when it asked
    /// too soon, with the whole seconds it has to wait.
    ///
    /// # Errors
    ///
    /// `Err(seconds)` — not a failure but the answer: the caller asked
    /// again within [`TEST_EMAIL_INTERVAL`], and the number is how long it
    /// has to wait.
    pub async fn claim(&self, caller: Uuid, now: Instant) -> Result<(), u64> {
        let mut last = self.last.lock().await;
        // Entries older than the interval are of no use any more; dropping
        // them keeps the map as small as the number of callers who asked
        // recently.
        last.retain(|_, at| now.duration_since(*at) < TEST_EMAIL_INTERVAL);
        if let Some(at) = last.get(&caller) {
            let wait = TEST_EMAIL_INTERVAL.saturating_sub(now.duration_since(*at));
            return Err(wait.as_secs().max(1));
        }
        last.insert(caller, now);
        Ok(())
    }
}

/// The saved email row for a request, or `None` when no Postgres pool is
/// configured (a Phase 1 deployment) or nothing was saved.
///
/// # Errors
///
/// Returns [`StoreError::Database`] when the store read fails.
pub async fn email_row(state: &AppState) -> Result<Option<EmailSettings>, StoreError> {
    match state.pg.as_deref() {
        Some(pool) => state.email_settings_cache.row(pool).await,
        None => Ok(None),
    }
}

/// The effective email settings for a send. A store read that fails degrades
/// to the environment's values (logged), never fails the send: email is not
/// load-bearing for the API surface, and an alert must still go out on the
/// environment's `SMTP_*` when the settings database is briefly down.
pub async fn email(state: &AppState) -> EffectiveEmail {
    match email_or_fail(state).await {
        Ok(effective) => effective,
        Err(err) => {
            tracing::warn!(
                error = %err,
                "reading saved email settings failed; using the environment's values"
            );
            let keyring = state.connector_secret_store.keyring().ok();
            resolve_email(&state.config, None, keyring)
        }
    }
}

/// The effective email settings, with the store's failure reported instead of
/// hidden behind the environment's values.
///
/// The settings screen uses this: answering "not set" for a value that IS
/// set (because the database could not be asked) would be a lie. A send
/// uses [`email`], which degrades instead.
///
/// # Errors
///
/// Returns [`StoreError::Database`] when the saved row cannot be read.
pub async fn email_or_fail(state: &AppState) -> Result<EffectiveEmail, StoreError> {
    let stored = email_row(state).await?;
    let keyring = state.connector_secret_store.keyring().ok();
    Ok(resolve_email(&state.config, stored.as_ref(), keyring))
}

/// The one merge (plan D1): environment wins, then the saved row, then the
/// code default. Pure, so it is unit-tested without a database. `keyring`
/// opens the saved password; `None` (no installation key) reads it as not
/// set.
#[must_use]
pub fn resolve_email(
    config: &Config,
    stored: Option<&EmailSettings>,
    keyring: Option<&Keyring>,
) -> EffectiveEmail {
    // `ADM-7 review R1`: the connection fields are ONE unit.
    //
    // The host, the port, the security, the account and the password are
    // how the sender logs in to a mail server; they belong together. When
    // the environment sets any one of `SMTP_HOST`, `SMTP_USER` or
    // `SMTP_PASS`, the whole unit belongs to the server: the resolver uses
    // only the environment's values for the five, the GET reports them
    // `server`, and a PUT that sends any of them is the existing 409.
    //
    // Why one unit and not one field: with `SMTP_PASS` set and `SMTP_HOST`
    // unset, the host's source was `default`, so a holder of
    // `settings:write` could save a host of their choosing — and every
    // send, the test button included, would then log in to that host with
    // the environment's account and password. The operator's secret would
    // have gone to whoever holds that permission, which is what plan D1
    // ("the server's file wins") exists to prevent. The server's file wins
    // for the connection as a whole, or not at all.
    //
    // `from` and `consolePublicUrl` stay per field: they are not part of
    // the login, and a `From` address or a public address on their own
    // cannot receive a secret.
    let server_owns_connection =
        config.smtp_host.is_some() || config.smtp_user.is_some() || !config.smtp_pass.is_empty();

    let (host, host_source) = if config.smtp_host.is_some() {
        (config.smtp_host.clone(), Source::Server)
    } else if server_owns_connection {
        // The environment owns the connection and names no host: there is
        // no host to use, and the console's saved one is not used.
        (None, Source::Server)
    } else if let Some(host) = stored.and_then(|s| s.smtp_host.as_deref()) {
        (Some(host.to_owned()), Source::Console)
    } else {
        (None, Source::Default)
    };

    let (port, port_source) = if config.smtp_port_from_env {
        (config.smtp_port, Source::Server)
    } else if server_owns_connection {
        // `config.smtp_port` is the environment's port, or the code default
        // when the environment named none.
        (config.smtp_port, Source::Server)
    } else if let Some(port) = stored.and_then(|s| s.smtp_port) {
        (u16::try_from(port).unwrap_or(587), Source::Console)
    } else {
        (587, Source::Default)
    };

    // `config.smtp_secure` already folds `SMTP_SECURE === "true" || port ===
    // 465` (config.rs), so a `true` here is exactly "the environment made it
    // implicit TLS"; a console save never changes this field.
    let (security, security_source) = if config.smtp_secure {
        (EmailSecurity::Tls, Source::Server)
    } else if server_owns_connection {
        (EmailSecurity::Starttls, Source::Server)
    } else if let Some(security) = stored
        .and_then(|s| s.smtp_security.as_deref())
        .and_then(EmailSecurity::parse)
    {
        (security, Source::Console)
    } else {
        (EmailSecurity::Starttls, Source::Default)
    };

    let (user, user_source) = if config.smtp_user.is_some() {
        (config.smtp_user.clone(), Source::Server)
    } else if server_owns_connection {
        (None, Source::Server)
    } else if let Some(user) = stored.and_then(|s| s.smtp_user.as_deref()) {
        (Some(user.to_owned()), Source::Console)
    } else {
        (None, Source::Default)
    };

    let (password, password_set, password_source) = if !config.smtp_pass.is_empty() {
        (
            SecretValue::new(config.smtp_pass.clone()),
            true,
            Source::Server,
        )
    } else if server_owns_connection {
        // The environment owns the connection and sets no password: the
        // console's saved one is not used.
        (SecretValue::new(""), false, Source::Server)
    } else if let Some(opened) = open_console_password(stored, keyring) {
        (SecretValue::new(opened), true, Source::Console)
    } else {
        (SecretValue::new(""), false, Source::Default)
    };

    // `from` is its own console field: the environment value it owns is
    // `SMTP_FROM` only. When `SMTP_FROM` is unset, `config.smtp_from` is the
    // pre-existing `SMTP_USER`/literal fallback, which a console save may
    // still override.
    let (from, from_source) = if config.smtp_from_from_env {
        (config.smtp_from.clone(), Source::Server)
    } else if let Some(from) = stored.and_then(|s| s.smtp_from.as_deref()) {
        (from.to_owned(), Source::Console)
    } else {
        (config.smtp_from.clone(), Source::Default)
    };

    let (console_public_url, console_public_url_source) = if config.console_public_url.is_some() {
        (config.console_public_url.clone(), Source::Server)
    } else if let Some(url) = stored.and_then(|s| s.console_public_url.as_deref()) {
        (Some(url.to_owned()), Source::Console)
    } else {
        (None, Source::Default)
    };

    EffectiveEmail {
        host,
        host_source,
        port,
        port_source,
        security,
        security_source,
        user,
        user_source,
        password,
        password_set,
        password_source,
        from,
        from_source,
        console_public_url,
        console_public_url_source,
        ca_bundle: config.smtp_ca.clone(),
    }
}

/// Open the saved email password, or `None` when it is not set, does not
/// open (the key changed or is missing), or is not UTF-8. The two failure
/// shapes are logged once per process and never returned (plan D2).
fn open_console_password(
    stored: Option<&EmailSettings>,
    keyring: Option<&Keyring>,
) -> Option<String> {
    let sealed = stored.and_then(|s| s.smtp_pass.as_deref())?;
    let Some(ring) = keyring else {
        log_once(
            &NO_KEY_LOGGED,
            "no installation key is configured; the saved email password is treated as not set",
        );
        return None;
    };
    let Ok(opened) = ring.open(EMAIL_PASSWORD_SEAL_NAME, sealed) else {
        log_once(
            &UNOPENABLE_PASSWORD_LOGGED,
            "the saved email password does not open with the configured key; treating it as not set",
        );
        return None;
    };
    let Ok(plain) = String::from_utf8(opened.plaintext) else {
        log_once(
            &UNOPENABLE_PASSWORD_LOGGED,
            "the saved email password is not UTF-8; treating it as not set",
        );
        return None;
    };
    // An empty password is no password: the console's "Clear" sends `null`,
    // but a save that sent an empty string must not look like a set one.
    if plain.is_empty() {
        return None;
    }
    Some(plain)
}

/// The resolver runs on every send, so a row that cannot be opened would
/// repeat its warning forever. Each of these failures is reported once per
/// process (plan D2: "logged once, never returned").
static NO_KEY_LOGGED: AtomicBool = AtomicBool::new(false);
static UNOPENABLE_PASSWORD_LOGGED: AtomicBool = AtomicBool::new(false);
/// The same two failures for the assistant's key, which is a different
/// secret and a different row.
static NO_ASSISTANT_KEY_LOGGED: AtomicBool = AtomicBool::new(false);
static UNOPENABLE_ASSISTANT_KEY_LOGGED: AtomicBool = AtomicBool::new(false);

/// Log `message` the first time this process sees this failure.
fn log_once(flag: &AtomicBool, message: &str) {
    if !flag.swap(true, Ordering::Relaxed) {
        tracing::warn!("{}", message);
    }
}

/// A fresh, empty [`EmailSettingsCache`] as an [`Arc`] — the value
/// [`crate::state::AppState`] holds.
#[must_use]
pub fn email_cache() -> Arc<EmailSettingsCache> {
    Arc::new(EmailSettingsCache::new())
}

/// A fresh, empty sign-in cache as an [`Arc`] — the value
/// [`crate::state::AppState`] holds.
#[must_use]
pub fn sign_in_cache() -> Arc<SettingsCache<SignInSettings>> {
    Arc::new(SettingsCache::new())
}

/// A fresh, empty assistant cache as an [`Arc`] — the value
/// [`crate::state::AppState`] holds.
#[must_use]
pub fn assistant_cache() -> Arc<SettingsCache<AssistantSettings>> {
    Arc::new(SettingsCache::new())
}

/// A fresh, empty SSO cache as an [`Arc`] — the value
/// [`crate::state::AppState`] holds.
#[must_use]
pub fn sso_cache() -> Arc<SsoSettingsCache> {
    Arc::new(SsoSettingsCache::new())
}

// ── The sign-in limits (`ADM-8`) ──────────────────────────────────────────

/// The resolved sign-in limits: the effective value of every field plus
/// where it came from (plan D1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveSignIn {
    /// Failed password tries tolerated per key before the lockout.
    pub max_failures: u32,
    /// Where [`Self::max_failures`] came from.
    pub max_failures_source: Source,
    /// The window (seconds) the failed tries are counted in.
    pub failure_window_secs: u32,
    /// Where [`Self::failure_window_secs`] came from.
    pub failure_window_secs_source: Source,
    /// How long (seconds) a locked key stays locked.
    pub lockout_secs: u32,
    /// Where [`Self::lockout_secs`] came from.
    pub lockout_secs_source: Source,
    /// How long (days) sign-in records are kept.
    pub retention_days: u32,
    /// Where [`Self::retention_days`] came from.
    pub retention_days_source: Source,
    /// The longest life (seconds) a signed embed link may have.
    pub embed_max_lifetime_secs: u64,
    /// Where [`Self::embed_max_lifetime_secs`] came from.
    pub embed_max_lifetime_secs_source: Source,
}

impl EffectiveSignIn {
    /// The login-throttle policy these limits describe (`SEC-5`).
    ///
    /// One place builds it: the login handler, the retention sweep and the
    /// console's Sign-in tab all read the same numbers.
    #[must_use]
    pub fn throttle_policy(&self) -> ThrottlePolicy {
        ThrottlePolicy {
            max_failures: self.max_failures,
            window: time::Duration::seconds(i64::from(self.failure_window_secs)),
            lockout: time::Duration::seconds(i64::from(self.lockout_secs)),
        }
    }
}

/// The one merge for the sign-in limits (plan D1): the environment wins,
/// then the saved row, then the code default. Pure, so it is unit-tested
/// without a database.
#[must_use]
pub fn resolve_sign_in(config: &Config, stored: Option<&SignInSettings>) -> EffectiveSignIn {
    let (max_failures, max_failures_source) = if config.login_max_failures_from_env {
        (config.login_max_failures, Source::Server)
    } else if let Some(value) = stored.and_then(|s| s.max_failures) {
        (
            u32::try_from(value).unwrap_or(config.login_max_failures),
            Source::Console,
        )
    } else {
        (config.login_max_failures, Source::Default)
    };
    let (failure_window_secs, failure_window_secs_source) =
        if config.login_failure_window_secs_from_env {
            (config.login_failure_window_secs, Source::Server)
        } else if let Some(value) = stored.and_then(|s| s.failure_window_secs) {
            (
                u32::try_from(value).unwrap_or(config.login_failure_window_secs),
                Source::Console,
            )
        } else {
            (config.login_failure_window_secs, Source::Default)
        };
    let (lockout_secs, lockout_secs_source) = if config.login_lockout_secs_from_env {
        (config.login_lockout_secs, Source::Server)
    } else if let Some(value) = stored.and_then(|s| s.lockout_secs) {
        (
            u32::try_from(value).unwrap_or(config.login_lockout_secs),
            Source::Console,
        )
    } else {
        (config.login_lockout_secs, Source::Default)
    };
    let (retention_days, retention_days_source) = if config.auth_retention_days_from_env {
        (config.auth_retention_days, Source::Server)
    } else if let Some(value) = stored.and_then(|s| s.retention_days) {
        (
            u32::try_from(value).unwrap_or(config.auth_retention_days),
            Source::Console,
        )
    } else {
        (config.auth_retention_days, Source::Default)
    };
    let (embed_max_lifetime_secs, embed_max_lifetime_secs_source) =
        if config.embed_token_max_lifetime_from_env {
            (config.embed_token_max_lifetime_seconds, Source::Server)
        } else if let Some(value) = stored.and_then(|s| s.embed_max_lifetime_secs) {
            (
                u64::try_from(value).unwrap_or(config.embed_token_max_lifetime_seconds),
                Source::Console,
            )
        } else {
            (config.embed_token_max_lifetime_seconds, Source::Default)
        };
    EffectiveSignIn {
        max_failures,
        max_failures_source,
        failure_window_secs,
        failure_window_secs_source,
        lockout_secs,
        lockout_secs_source,
        retention_days,
        retention_days_source,
        embed_max_lifetime_secs,
        embed_max_lifetime_secs_source,
    }
}

/// The saved sign-in row for a request, or `None` when no Postgres pool is
/// configured (a Phase 1 deployment) or nothing was saved.
///
/// # Errors
///
/// Returns [`StoreError::Database`] when the store read fails.
pub async fn sign_in_row(state: &AppState) -> Result<Option<SignInSettings>, StoreError> {
    match state.pg.as_deref() {
        Some(pool) => state.sign_in_settings_cache.row(pool).await,
        None => Ok(None),
    }
}

/// The effective sign-in limits. A store read that fails degrades to the
/// environment's values (logged), never fails the reader: the login
/// throttle must keep working when the settings database is briefly down.
pub async fn sign_in(state: &AppState) -> EffectiveSignIn {
    match sign_in_or_fail(state).await {
        Ok(effective) => effective,
        Err(err) => {
            tracing::warn!(
                error = %err,
                "reading saved sign-in settings failed; using the environment's values"
            );
            resolve_sign_in(&state.config, None)
        }
    }
}

/// The effective sign-in limits, with the store's failure reported instead
/// of hidden behind the environment's values.
///
/// The console's Sign-in tab uses this: answering "not set" for a value that
/// IS set (because the database could not be asked) would be a lie.
///
/// # Errors
///
/// Returns [`StoreError::Database`] when the saved row cannot be read.
pub async fn sign_in_or_fail(state: &AppState) -> Result<EffectiveSignIn, StoreError> {
    let stored = sign_in_row(state).await?;
    Ok(resolve_sign_in(&state.config, stored.as_ref()))
}

// ── The assistant's provider (`ADM-9`) ────────────────────────────────────

/// The resolved assistant settings: the effective address, model and key,
/// and where each came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveAssistant {
    /// The provider's base URL, e.g. `https://api.example.com/v1`.
    pub url: String,
    /// Where [`Self::url`] came from.
    pub url_source: Source,
    /// The provider's model name.
    pub model: String,
    /// Where [`Self::model`] came from.
    pub model_source: Source,
    /// The provider's key, as a [`SecretValue`]: redacting `Debug`, no
    /// `Serialize`, never logged or returned.
    pub key: SecretValue,
    /// Whether a key is in effect at all (the value itself is never
    /// serialized).
    pub key_set: bool,
    /// Where the key came from.
    pub key_source: Source,
}

/// The assistant client and the settings it was built from (`ADM-9`).
///
/// The settings are kept beside the client so the resolver can tell whether
/// the client is still the one the effective values describe — and so the
/// answer the console gets names the provider the assistant really uses.
#[derive(Clone)]
pub struct AssistantClient {
    /// The client the assistant talks through.
    pub client: Arc<LlmClient>,
    /// The effective settings it was built from.
    pub settings: EffectiveAssistant,
}

/// The one merge for the assistant's provider (plan D1, D13), with the
/// `ADM-7 review R1` lesson applied: the address, the model and the key are
/// ONE unit.
///
/// When the environment holds a key (`LLM_KEY` or `MINIMAX_API_KEY`), the
/// whole unit is the server's: the effective address and model are the
/// environment's, the console may change none of the three, and the
/// environment's key is never sent to an address the console chose. The
/// console's saved values are not used at all in that case.
///
/// With no key in the environment, the key is the console's, but the
/// address and the model are per field (`ADM-9 review R4`): an environment
/// that sets `LLM_URL` owns the address alone, and one that sets
/// `LLM_MODEL` owns the model alone — the same rule slice 1 uses for
/// `SMTP_PORT` (`smtp_port_from_env`). The console's saved value of that
/// field is then not used, the GET reports it `server`, and a save that
/// sends it is the existing 409. Where the environment names neither, the
/// console's saved value is used, and where it has saved none the code's
/// default is; the source of such a value is `default`, which means "the
/// console has not set this" — the console may change it.
#[must_use]
pub fn resolve_assistant(
    config: &Config,
    stored: Option<&AssistantSettings>,
    keyring: Option<&Keyring>,
) -> EffectiveAssistant {
    if !config.llm_key.is_empty() {
        return EffectiveAssistant {
            url: config.llm_url.clone(),
            url_source: Source::Server,
            model: config.llm_model.clone(),
            model_source: Source::Server,
            key: SecretValue::new(config.llm_key.clone()),
            key_set: true,
            key_source: Source::Server,
        };
    }
    // `ADM-9 review R4`: the environment's own address and model win on
    // their own, each by itself. The console's saved value of that field is
    // not used, so the operator's file stays the operator's.
    let (url, url_source) = if config.llm_url_from_env {
        (config.llm_url.clone(), Source::Server)
    } else {
        match stored.and_then(|s| s.llm_url.as_deref()) {
            Some(url) => (url.to_owned(), Source::Console),
            None => (config.llm_url.clone(), Source::Default),
        }
    };
    let (model, model_source) = if config.llm_model_from_env {
        (config.llm_model.clone(), Source::Server)
    } else {
        match stored.and_then(|s| s.llm_model.as_deref()) {
            Some(model) => (model.to_owned(), Source::Console),
            None => (config.llm_model.clone(), Source::Default),
        }
    };
    let (key, key_set, key_source) = match open_console_key(stored, keyring) {
        Some(key) => (SecretValue::new(key), true, Source::Console),
        None => (SecretValue::new(""), false, Source::Default),
    };
    EffectiveAssistant {
        url,
        url_source,
        model,
        model_source,
        key,
        key_set,
        key_source,
    }
}

/// Open the saved assistant key, or `None` when it is not set, does not open
/// (the key changed or is missing), or is not UTF-8. The two failure shapes
/// are logged once per process and never returned (plan D2).
fn open_console_key(
    stored: Option<&AssistantSettings>,
    keyring: Option<&Keyring>,
) -> Option<String> {
    let sealed = stored.and_then(|s| s.llm_key.as_deref())?;
    let Some(ring) = keyring else {
        log_once(
            &NO_ASSISTANT_KEY_LOGGED,
            "no installation key is configured; the saved assistant key is treated as not set",
        );
        return None;
    };
    let Ok(opened) = ring.open(ASSISTANT_KEY_SEAL_NAME, sealed) else {
        log_once(
            &UNOPENABLE_ASSISTANT_KEY_LOGGED,
            "the saved assistant key does not open with the configured key; treating it as not set",
        );
        return None;
    };
    let Ok(plain) = String::from_utf8(opened.plaintext) else {
        log_once(
            &UNOPENABLE_ASSISTANT_KEY_LOGGED,
            "the saved assistant key is not UTF-8; treating it as not set",
        );
        return None;
    };
    // An empty key is no key: the console's "Clear" sends `null`, but a save
    // that sent an empty string must not look like a set one.
    if plain.is_empty() {
        return None;
    }
    Some(plain)
}

/// The saved assistant row for a request, or `None` when no Postgres pool is
/// configured (a Phase 1 deployment) or nothing was saved.
///
/// # Errors
///
/// Returns [`StoreError::Database`] when the store read fails.
pub async fn assistant_row(state: &AppState) -> Result<Option<AssistantSettings>, StoreError> {
    match state.pg.as_deref() {
        Some(pool) => state.assistant_settings_cache.row(pool).await,
        None => Ok(None),
    }
}

/// The effective assistant settings: the environment's, else the console's
/// saved ones, else the code's defaults.
///
/// # Errors
///
/// Returns [`StoreError::Database`] when the saved row cannot be read.
pub async fn assistant_or_fail(state: &AppState) -> Result<EffectiveAssistant, StoreError> {
    let stored = assistant_row(state).await?;
    let keyring = state.connector_secret_store.keyring().ok();
    Ok(resolve_assistant(&state.config, stored.as_ref(), keyring))
}

/// The assistant client for the effective settings, rebuilt when they
/// change.
///
/// The client is swappable (`AppState::llm`), so a save in the console takes
/// effect on the next message without a restart. A rebuild checks the
/// address again: a name that resolved publicly when it was saved may
/// resolve inside the deployment's network now, and an address the console
/// chose must never become one (plan D13).
///
/// When the check refuses the address, the client this process already has
/// stays in use and the answer keeps naming it — the console is told what
/// the assistant really uses, not what was asked for.
pub async fn assistant(state: &AppState) -> Arc<AssistantClient> {
    let desired = match assistant_or_fail(state).await {
        Ok(effective) => effective,
        Err(err) => {
            tracing::warn!(
                error = %err,
                "reading saved assistant settings failed; using the environment's values"
            );
            resolve_assistant(&state.config, None, None)
        }
    };
    let current = state.llm.read().await.clone();
    if current.settings == desired {
        return current;
    }
    // The address the console chose is checked again before it is used.
    // The environment's own address is not checked: it is the operator's
    // file, and a deployment may point its assistant at a gateway inside
    // its own network (plan D1: the server's file wins).
    if desired.url_source == Source::Console {
        let internal_hosts = state.config.connector_internal_hosts();
        if let Err(problem) = check_assistant_address(&desired.url, &internal_hosts).await {
            tracing::error!(
                %problem,
                "the assistant's provider address is not one this deployment may reach; \
                 the client was not rebuilt"
            );
            return current;
        }
    }
    let client = Arc::new(LlmClient::new(
        desired.url.clone(),
        desired.model.clone(),
        desired.key.expose_secret().to_owned(),
    ));
    let built = Arc::new(AssistantClient {
        client,
        settings: desired,
    });
    *state.llm.write().await = built.clone();
    built
}

/// The assistant client this process starts with: the effective settings
/// from the environment (there is no saved row at start-up), and the client
/// built from them.
#[must_use]
pub fn initial_assistant_client(
    config: &Config,
    keyring: Option<&Keyring>,
) -> Arc<AssistantClient> {
    let settings = resolve_assistant(config, None, keyring);
    let client = Arc::new(LlmClient::new(
        settings.url.clone(),
        settings.model.clone(),
        settings.key.expose_secret().to_owned(),
    ));
    Arc::new(AssistantClient { client, settings })
}

/// Why a provider address was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddressProblem {
    /// The address is not an `https://` address.
    NotHttps,
    /// The address names no host the check can look up.
    NoHost,
    /// The name does not resolve to any address.
    Unresolved,
    /// The name resolves inside this deployment's network.
    Internal,
}

impl std::fmt::Display for AddressProblem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let text = match self {
            Self::NotHttps => "the address is not an https:// address",
            Self::NoHost => "the address names no host",
            Self::Unresolved => "the address does not resolve to any address",
            Self::Internal => "the address resolves inside this deployment's network",
        };
        f.write_str(text)
    }
}

/// Check an external `https://` address: it must be `https://` and its host
/// must pass the internal-address check (`connector_probe::resolve_checked`).
///
/// Shared by [`check_assistant_address`] and [`check_issuer_address`] (plan D6).
///
/// # Errors
///
/// Returns [`AddressProblem`] if the URL is not `https://`, names no host,
/// does not resolve, or resolves to an internal address.
pub async fn check_external_https_address(
    url: &str,
    internal_hosts: &InternalHosts,
) -> Result<(), AddressProblem> {
    if !url.starts_with("https://") {
        return Err(AddressProblem::NotHttps);
    }
    let Some((host, port)) = crate::connector_probe::parse_endpoint_host_port(url) else {
        return Err(AddressProblem::NoHost);
    };
    // `resolve_checked` answers two failures with one message — a name that
    // does not resolve, and a name that resolves inside the network — so
    // the name is resolved once here to tell them apart. The DECISION (is
    // this address inside the network?) is `resolve_checked`'s, the same
    // check the connector dials use.
    let mut resolved = tokio::net::lookup_host((host, port))
        .await
        .map_err(|_| AddressProblem::Unresolved)?;
    if resolved.next().is_none() {
        return Err(AddressProblem::Unresolved);
    }
    crate::connector_probe::resolve_checked(host, port, internal_hosts)
        .await
        .map(|_| ())
        .map_err(|_| AddressProblem::Internal)
}

/// Check the provider address the console chose: it must be an `https://`
/// address, and its host must pass the same internal-address check the
/// connector dials use (`connector_probe::resolve_checked`), so an
/// administrator's account cannot be used to make the API call a service
/// inside the deployment's network (plan D13).
///
/// The environment's own address is NOT checked here: it is the operator's
/// file, and an on-premises deployment may point its assistant at a gateway
/// inside its own network.
///
/// # Errors
///
/// The [`AddressProblem`] that refused the address. The caller turns it into
/// a fixed sentence; the reason never carries an address the caller did not
/// send.
pub async fn check_assistant_address(
    url: &str,
    internal_hosts: &InternalHosts,
) -> Result<(), AddressProblem> {
    check_external_https_address(url, internal_hosts).await
}

/// Check the OIDC issuer address set in the console: must be `https://` and
/// external (plan D6).
///
/// # Errors
///
/// Returns [`AddressProblem`] describing why the address was refused.
pub async fn check_issuer_address(
    url: &str,
    internal_hosts: &InternalHosts,
) -> Result<(), AddressProblem> {
    check_external_https_address(url, internal_hosts).await
}

// ── Single Sign-On / OIDC (`ADM-12`) ─────────────────────────────────────

/// One row of the group-to-role mapping.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RoleMapEntry {
    /// External `IdP` group name.
    pub group: String,
    /// Local role name.
    pub role: String,
}

/// The resolved single sign-on settings: the effective value of every field
/// plus where it came from (plan D1/D3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveSso {
    /// OIDC provider issuer URL.
    pub issuer: Option<String>,
    /// Where [`Self::issuer`] came from.
    pub issuer_source: Source,
    /// Application client ID.
    pub client_id: Option<String>,
    /// Where [`Self::client_id`] came from.
    pub client_id_source: Source,
    /// Application client secret, as a [`SecretValue`].
    pub client_secret: SecretValue,
    /// Whether a client secret is in effect.
    pub client_secret_set: bool,
    /// Where [`Self::client_secret`] came from.
    pub client_secret_source: Source,
    /// Operator-chosen provider label.
    pub provider_name: String,
    /// Where [`Self::provider_name`] came from.
    pub provider_name_source: Source,
    /// Claim carrying user groups/roles.
    pub groups_claim: String,
    /// Where [`Self::groups_claim`] came from.
    pub groups_claim_source: Source,
    /// Role map as a lookup from external group to local role.
    pub role_map: HashMap<String, String>,
    /// Role map as ordered rows for the console.
    pub role_map_rows: Vec<RoleMapEntry>,
    /// Where the role map came from.
    pub role_map_source: Source,
    /// Whether JIT provisioning is enabled.
    pub jit_provisioning: bool,
    /// Where [`Self::jit_provisioning`] came from.
    pub jit_provisioning_source: Source,
    /// Authorization URL.
    pub authorize_url: Option<String>,
    /// Where [`Self::authorize_url`] came from.
    pub authorize_url_source: Source,
    /// Token endpoint URL.
    pub token_url: Option<String>,
    /// Where [`Self::token_url`] came from.
    pub token_url_source: Source,
    /// JWKS URL override.
    pub jwks_url: Option<String>,
    /// Where [`Self::jwks_url`] came from.
    pub jwks_url_source: Source,
    /// Redirect URI.
    pub redirect_uri: Option<String>,
    /// Where [`Self::redirect_uri`] came from.
    pub redirect_uri_source: Source,
    /// Clock skew in seconds.
    pub clock_skew_seconds: u64,
}

impl EffectiveSso {
    /// Whether `OIDC` is configured (both `issuer` and `client_id` are set and non-empty).
    #[must_use]
    pub fn is_configured(&self) -> bool {
        self.issuer.as_ref().is_some_and(|i| !i.is_empty())
            && self.client_id.as_ref().is_some_and(|c| !c.is_empty())
    }

    /// Build an [`lakehouse_auth::oidc::OidcConfig`] from these effective settings.
    #[must_use]
    pub fn to_oidc_config(&self) -> lakehouse_auth::oidc::OidcConfig {
        let issuer = self.issuer.clone().unwrap_or_default();
        let client_id = self.client_id.clone().unwrap_or_default();
        let mut config = match self.jwks_url.clone() {
            Some(jwks_url) => lakehouse_auth::oidc::OidcConfig::new(
                issuer,
                client_id,
                self.provider_name.clone(),
                jwks_url,
            ),
            None => lakehouse_auth::oidc::OidcConfig::with_discovery(
                issuer,
                client_id,
                self.provider_name.clone(),
            ),
        };
        config.jit_provisioning = self.jit_provisioning;
        config.role_map.clone_from(&self.role_map);
        config.groups_claim.clone_from(&self.groups_claim);
        config.clock_skew_seconds = self.clock_skew_seconds;
        config
    }
}

static NO_KEY_LOGGED_SSO: AtomicBool = AtomicBool::new(false);
static UNOPENABLE_SECRET_LOGGED_SSO: AtomicBool = AtomicBool::new(false);

/// Open the saved SSO client secret, or `None` when it is not set, does not
/// open (the key changed or is missing), or is not UTF-8 (plan D1/D2).
fn open_console_client_secret(
    stored: Option<&SsoSettings>,
    keyring: Option<&Keyring>,
) -> Option<String> {
    let sealed = stored.and_then(|s| s.oidc_client_secret.as_deref())?;
    let Some(ring) = keyring else {
        log_once(
            &NO_KEY_LOGGED_SSO,
            "no installation key is configured; the saved SSO client secret is treated as not set",
        );
        return None;
    };
    let Ok(opened) = ring.open(SSO_CLIENT_SECRET_SEAL_NAME, sealed) else {
        log_once(
            &UNOPENABLE_SECRET_LOGGED_SSO,
            "the saved SSO client secret does not open with the configured key; treating it as not set",
        );
        return None;
    };
    let Ok(plain) = String::from_utf8(opened.plaintext) else {
        log_once(
            &UNOPENABLE_SECRET_LOGGED_SSO,
            "the saved SSO client secret is not UTF-8; treating it as not set",
        );
        return None;
    };
    if plain.is_empty() {
        return None;
    }
    Some(plain)
}

pub(crate) fn parse_role_map_json_entries(val: &serde_json::Value) -> Vec<RoleMapEntry> {
    let mut entries = Vec::new();
    if let serde_json::Value::Array(items) = val {
        for item in items {
            if let (Some(group), Some(role)) = (
                item.get("group").and_then(|v| v.as_str()),
                item.get("role").and_then(|v| v.as_str()),
            ) {
                let group = group.trim();
                let role = role.trim();
                if !group.is_empty() && !role.is_empty() {
                    entries.push(RoleMapEntry {
                        group: group.to_owned(),
                        role: role.to_owned(),
                    });
                }
            }
        }
    }
    entries
}

/// The one merge rule for SSO (plan D1, D3): environment wins as one unit for
/// the connection (`OIDC_ISSUER` or `OIDC_CLIENT_ID` set), then the saved row,
/// then the code default. Pure, so it is unit-tested without a database.
#[must_use]
#[allow(
    clippy::too_many_lines,
    reason = "one flat resolver mapping every SSO setting to its effective value and source, matching resolve_email"
)]
pub fn resolve_sso(
    config: &Config,
    stored: Option<&SsoSettings>,
    keyring: Option<&Keyring>,
) -> EffectiveSso {
    let server_owns_connection = config.oidc_issuer.is_some() || config.oidc_client_id.is_some();

    let (issuer, issuer_source) = if config.oidc_issuer.is_some() {
        (config.oidc_issuer.clone(), Source::Server)
    } else if server_owns_connection {
        (None, Source::Server)
    } else if let Some(iss) = stored.and_then(|s| s.oidc_issuer.as_deref()) {
        (Some(iss.to_owned()), Source::Console)
    } else {
        (None, Source::Default)
    };

    let (client_id, client_id_source) = if config.oidc_client_id.is_some() {
        (config.oidc_client_id.clone(), Source::Server)
    } else if server_owns_connection {
        (None, Source::Server)
    } else if let Some(cid) = stored.and_then(|s| s.oidc_client_id.as_deref()) {
        (Some(cid.to_owned()), Source::Console)
    } else {
        (None, Source::Default)
    };

    let (client_secret, client_secret_set, client_secret_source) =
        if let Some(sec) = config.oidc_client_secret.as_ref() {
            (SecretValue::new(sec.clone()), true, Source::Server)
        } else if server_owns_connection {
            (SecretValue::new(""), false, Source::Server)
        } else if let Some(opened) = open_console_client_secret(stored, keyring) {
            (SecretValue::new(opened), true, Source::Console)
        } else {
            (SecretValue::new(""), false, Source::Default)
        };

    let (authorize_url, authorize_url_source) = if config.oidc_authorize_url.is_some() {
        (config.oidc_authorize_url.clone(), Source::Server)
    } else if server_owns_connection {
        (None, Source::Server)
    } else {
        (None, Source::Default)
    };

    let (token_url, token_url_source) = if config.oidc_token_url.is_some() {
        (config.oidc_token_url.clone(), Source::Server)
    } else if server_owns_connection {
        (None, Source::Server)
    } else {
        (None, Source::Default)
    };

    let (jwks_url, jwks_url_source) = if config.oidc_jwks_url.is_some() {
        (config.oidc_jwks_url.clone(), Source::Server)
    } else if server_owns_connection {
        (None, Source::Server)
    } else {
        (None, Source::Default)
    };

    let (redirect_uri, redirect_uri_source) = if config.oidc_redirect_uri.is_some() {
        (config.oidc_redirect_uri.clone(), Source::Server)
    } else if server_owns_connection {
        (None, Source::Server)
    } else {
        (None, Source::Default)
    };

    let (provider_name, provider_name_source) = if config.oidc_provider_name_from_env {
        (config.oidc_provider_name.clone(), Source::Server)
    } else if let Some(name) = stored.and_then(|s| s.oidc_provider_name.as_deref()) {
        (name.to_owned(), Source::Console)
    } else {
        ("default".to_owned(), Source::Default)
    };

    let (groups_claim, groups_claim_source) = if config.oidc_groups_claim_from_env {
        (config.oidc_groups_claim.clone(), Source::Server)
    } else if let Some(claim) = stored.and_then(|s| s.oidc_groups_claim.as_deref()) {
        (claim.to_owned(), Source::Console)
    } else {
        ("groups".to_owned(), Source::Default)
    };

    let (jit_provisioning, jit_provisioning_source) = if config.oidc_jit_provisioning_from_env {
        (config.oidc_jit_provisioning, Source::Server)
    } else if let Some(jit) = stored.and_then(|s| s.oidc_jit_provisioning) {
        (jit, Source::Console)
    } else {
        (false, Source::Default)
    };

    let (role_map, role_map_rows, role_map_source) = if config.oidc_role_map_from_env {
        let mut entries: Vec<RoleMapEntry> = config
            .oidc_role_map
            .iter()
            .map(|(g, r)| RoleMapEntry {
                group: g.clone(),
                role: r.clone(),
            })
            .collect();
        entries.sort_by(|a, b| a.group.cmp(&b.group));
        (config.oidc_role_map.clone(), entries, Source::Server)
    } else if let Some(val) = stored.and_then(|s| s.oidc_role_map.as_ref()) {
        let entries = parse_role_map_json_entries(val);
        let mut map = HashMap::new();
        for entry in &entries {
            map.insert(entry.group.clone(), entry.role.clone());
        }
        (map, entries, Source::Console)
    } else {
        (HashMap::new(), Vec::new(), Source::Default)
    };

    EffectiveSso {
        issuer,
        issuer_source,
        client_id,
        client_id_source,
        client_secret,
        client_secret_set,
        client_secret_source,
        provider_name,
        provider_name_source,
        groups_claim,
        groups_claim_source,
        role_map,
        role_map_rows,
        role_map_source,
        jit_provisioning,
        jit_provisioning_source,
        authorize_url,
        authorize_url_source,
        token_url,
        token_url_source,
        jwks_url,
        jwks_url_source,
        redirect_uri,
        redirect_uri_source,
        clock_skew_seconds: config.oidc_clock_skew_seconds,
    }
}

/// The saved SSO row for a request, or `None` when no Postgres pool is
/// configured (a Phase 1 deployment) or nothing was saved.
///
/// # Errors
///
/// Returns [`StoreError::Database`] when the store read fails.
pub async fn sso_row(state: &AppState) -> Result<Option<SsoSettings>, StoreError> {
    match state.pg.as_deref() {
        Some(pool) => state.sso_settings_cache.row(pool).await,
        None => Ok(None),
    }
}

/// The effective SSO settings: the environment's, else the console's
/// saved ones, else the code's defaults.
///
/// # Errors
///
/// Returns [`StoreError::Database`] when the saved row cannot be read.
pub async fn sso_or_fail(state: &AppState) -> Result<EffectiveSso, StoreError> {
    let stored = sso_row(state).await?;
    let keyring = state.connector_secret_store.keyring().ok();
    Ok(resolve_sso(&state.config, stored.as_ref(), keyring))
}

/// The path a sign-in returns to on this deployment (`ADM-12` D10): built
/// from the operator's own `CONSOLE_PUBLIC_URL`, never from a request
/// header.
const OIDC_CALLBACK_PATH: &str = "/api/auth/oidc/callback";

/// Why a single sign-on connection cannot be used right now (`ADM-12` D10).
///
/// One fixed sentence each, and each names what an operator can set. The
/// provider's own text never appears in one (principle 4): a discovery
/// document and a token endpoint's answer are remote input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SsoNotReady {
    /// No issuer or client ID is in effect.
    NotConfigured,
    /// Settings need a database, and this deployment has none.
    NoDatabase,
    /// A console-set connection has no return address to send people to.
    NoPublicAddress,
    /// The console-set issuer is not one this deployment may reach.
    IssuerRefused,
    /// The issuer's discovery document could not be read.
    DiscoveryUnreadable,
    /// The discovery document names an address this deployment may not use.
    AddressRefused,
    /// A server-owned connection is missing an address a sign-in needs.
    ServerAddressesMissing,
}

impl SsoNotReady {
    /// The one sentence the Settings form and the log show for this reason.
    #[must_use]
    pub const fn sentence(self) -> &'static str {
        match self {
            Self::NotConfigured => {
                "Single sign-on is not configured: save an issuer and a client ID."
            }
            Self::NoDatabase => "Single sign-on needs a database, and none is configured.",
            Self::NoPublicAddress => {
                "Set CONSOLE_PUBLIC_URL to this deployment's public address: a sign-in has to return people here."
            }
            Self::IssuerRefused => {
                "The issuer address is not one this deployment may reach: it must be an https:// address outside its own network."
            }
            Self::DiscoveryUnreadable => {
                "The issuer's discovery document could not be read: check the issuer address and that the provider is reachable."
            }
            Self::AddressRefused => {
                "The provider's discovery document names an address this deployment may not use: it must be an https:// address outside its own network."
            }
            Self::ServerAddressesMissing => {
                "Set OIDC_AUTHORIZE_URL, OIDC_TOKEN_URL and OIDC_REDIRECT_URI: a sign-in needs all three."
            }
        }
    }
}

/// One built SSO runtime: the addresses a sign-in will use, the
/// authenticator that verifies the token, and the HTTP client every request
/// of this connection goes through (`ADM-12` D4, D10).
///
/// [`Self::base`] is what [`resolve_sso`] returned before discovery, kept
/// so the cache can tell whether the saved row moved; [`Self::settings`] is
/// the same value with the addresses filled in from the provider's
/// discovery document.
///
/// # Two facts, kept apart (`ADM-12` review round 2, R2-B1)
///
/// * An **authenticator** exists whenever the issuer and the client ID are
///   set, as on `main`: it is what the bearer-token path
///   (`crate::auth::AuthenticatedPrincipal`) and the Deployment page
///   (`routes::settings::get_deployment`) mean, and a deployment that sets
///   only `OIDC_ISSUER` and `OIDC_CLIENT_ID` keeps accepting bearer tokens.
///   For a console-set connection it also needs the address checks to have
///   passed, because those are what keep a remote value from pointing this
///   deployment at its own network.
/// * **Ready for sign-in** additionally needs the authorize, token and
///   redirect addresses; [`Self::is_ready`] and [`Self::not_ready_reason`]
///   are this fact, and `providers`, `oidc_start`, the callback and
///   `GET /api/settings/sso` read it.
#[derive(Clone)]
pub struct SwappableOidc {
    /// What [`resolve_sso`] returned, before discovery.
    pub base: EffectiveSso,
    /// The effective settings, with the addresses filled in.
    pub settings: EffectiveSso,
    /// The authenticator: present whenever the issuer and the client ID are
    /// set (and, for a console-set connection, the address checks passed) —
    /// not only when a sign-in can be offered.
    pub authenticator: Option<Arc<lakehouse_auth::oidc::OidcAuthenticator>>,
    /// The HTTP client every request of this connection goes through.
    pub http: reqwest::Client,
    /// Why the connection cannot be used, if it cannot.
    pub not_ready: Option<SsoNotReady>,
    /// When it was built, so a failed discovery is retried after
    /// [`SETTINGS_CACHE_TTL`] without waiting for a save.
    built_at: Instant,
}

impl SwappableOidc {
    /// Whether a sign-in can be offered with this runtime (`ADM-12` D10) —
    /// the second fact in this type's doc comment: the authenticator exists
    /// *and* the authorize, token and redirect addresses do.
    #[must_use]
    pub fn is_ready(&self) -> bool {
        self.not_ready.is_none() && self.authenticator.is_some()
    }

    /// The authenticator of a connection that can sign someone in — what
    /// `providers`, `oidc_start`, the callback and the test callback take
    /// (`ADM-12` review round 2, R2-B1). A connection with an authenticator
    /// but no sign-in addresses is `None` here: it accepts bearer tokens,
    /// not people ([`Self::authenticator`] keeps that one).
    #[must_use]
    pub fn sign_in_authenticator(&self) -> Option<Arc<lakehouse_auth::oidc::OidcAuthenticator>> {
        if self.is_ready() {
            self.authenticator.clone()
        } else {
            None
        }
    }

    /// The fixed sentence for why this runtime cannot be used, or `None`.
    #[must_use]
    pub fn not_ready_reason(&self) -> Option<&'static str> {
        self.not_ready.map(SsoNotReady::sentence)
    }
}

/// The runtime for the effective settings: the environment's, else the
/// console's saved ones, else the defaults (`ADM-12` D4, D10).
///
/// Cached in [`AppState::oidc`] and cleared on save. Rebuilt when the saved
/// row or the environment moves; a ready runtime is kept for
/// [`READY_SSO_RUNTIME_TTL`], and a not-ready one is retried after
/// [`SETTINGS_CACHE_TTL`] so a provider that was briefly unreachable
/// becomes usable without a save (`ADM-12` review round 2, R2-S1).
///
/// # Errors
///
/// Returns [`StoreError::Database`] when the saved row cannot be read — a
/// caller that must not claim a value is unset when the database could not
/// be asked uses this; [`sso`]/[`oidc`] use the infallible wrapper instead.
pub async fn sso_runtime_or_fail(state: &AppState) -> Result<SwappableOidc, StoreError> {
    // D3: when the environment owns the connection, nothing the saved row
    // holds can change where a sign-in goes, so the row is not read at all
    // — the same shape this resolver had before discovery, and what keeps a
    // sign-in on a deployment whose database is briefly down from waiting on
    // it.
    let config_only = resolve_sso(&state.config, None, None);
    if config_only.issuer_source == Source::Server {
        return Ok(cached_or_build(state, config_only).await);
    }
    let base = sso_or_fail(state).await?;
    Ok(cached_or_build(state, base).await)
}

/// The cached runtime for `base`, or a freshly built one.
///
/// `ADM-12` review round 2, R2-S1: the network work — the discovery read,
/// the address look-ups and the authenticator's construction — happens
/// **outside** `state.oidc`'s write lock, so the bearer-token path is never
/// parked behind a provider that is slow to answer. A ready runtime is kept
/// for [`READY_SSO_RUNTIME_TTL`]; a not-ready one is retried after
/// [`SETTINGS_CACHE_TTL`]; a save clears the slot at once (`put_sso`).
async fn cached_or_build(state: &AppState, base: EffectiveSso) -> SwappableOidc {
    if let Some(current) = cached_runtime(state, &base).await {
        return current;
    }

    let pool = state.pg.as_deref().cloned();
    let runtime = build_sso_runtime(state, base.clone(), pool).await;

    // Two requests may build at once now that building is lock-free. The
    // first to finish wins; the other keeps it and drops its own copy
    // rather than replacing a runtime that is at least as new.
    let mut guard = state.oidc.write().await;
    if let Some(current) = guard.as_ref()
        && is_fresh(current, &base)
        && current.built_at > runtime.built_at
    {
        return current.clone();
    }
    *guard = Some(runtime.clone());
    runtime
}

/// The cached runtime for `base` when it is still fresh, read under the
/// read lock only (`ADM-12` review round 2, R2-S1).
async fn cached_runtime(state: &AppState, base: &EffectiveSso) -> Option<SwappableOidc> {
    let guard = state.oidc.read().await;
    guard
        .as_ref()
        .filter(|current| is_fresh(current, base))
        .cloned()
}

/// Whether a cached runtime is `base`'s and young enough to reuse: a ready
/// one for [`READY_SSO_RUNTIME_TTL`], a not-ready one for
/// [`SETTINGS_CACHE_TTL`] (`ADM-12` review round 2, R2-S1).
fn is_fresh(runtime: &SwappableOidc, base: &EffectiveSso) -> bool {
    let ttl = if runtime.is_ready() {
        READY_SSO_RUNTIME_TTL
    } else {
        SETTINGS_CACHE_TTL
    };
    runtime.base == *base && runtime.built_at.elapsed() < ttl
}

/// Build a runtime for one set of settings **without touching the cache** —
/// the test sign-in's draft (`ADM-12` D8): it must use the draft's own
/// discovery document (B2) and must never replace the live connection's
/// runtime. Same builder, same checks as the live path (S5).
pub async fn sso_runtime_for_settings(state: &AppState, settings: EffectiveSso) -> SwappableOidc {
    let pool = state.pg.as_deref().cloned();
    build_sso_runtime(state, settings, pool).await
}

/// The settings a test run's stored draft names (`ADM-12` D8): the fields
/// the draft carries, the deployment's return address, and no secret — a run
/// uses only the secret sealed into it when it was created (S4).
///
/// Every field is `Source::Console`: the draft is what the administrator
/// typed on the form, not the environment's.
#[must_use]
pub fn draft_sso_settings(config: &Config, draft: &serde_json::Value) -> EffectiveSso {
    let text = |key: &str| {
        draft
            .get(key)
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
    };
    let role_map_rows = draft
        .get("roleMap")
        .map(parse_role_map_json_entries)
        .unwrap_or_default();
    let mut role_map = HashMap::new();
    for entry in &role_map_rows {
        role_map.insert(entry.group.clone(), entry.role.clone());
    }
    EffectiveSso {
        issuer: text("issuer"),
        issuer_source: Source::Console,
        client_id: text("clientId"),
        client_id_source: Source::Console,
        client_secret: SecretValue::new(""),
        client_secret_set: false,
        client_secret_source: Source::Default,
        provider_name: text("providerName").unwrap_or_else(|| "default".to_owned()),
        provider_name_source: Source::Console,
        groups_claim: text("groupsClaim").unwrap_or_else(|| "groups".to_owned()),
        groups_claim_source: Source::Console,
        role_map,
        role_map_rows,
        role_map_source: Source::Console,
        jit_provisioning: draft
            .get("jitProvisioning")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
        jit_provisioning_source: Source::Console,
        authorize_url: None,
        authorize_url_source: Source::Default,
        token_url: None,
        token_url_source: Source::Default,
        jwks_url: None,
        jwks_url_source: Source::Default,
        redirect_uri: config.oidc_redirect_uri.clone(),
        redirect_uri_source: if config.oidc_redirect_uri.is_some() {
            Source::Server
        } else {
            Source::Default
        },
        clock_skew_seconds: config.oidc_clock_skew_seconds,
    }
}

/// `sso_runtime_or_fail` with the store failure logged and the
/// environment's values used instead — the shape every request path needs,
/// because a database blip must not take sign-in settings down with it.
pub async fn sso_runtime(state: &AppState) -> SwappableOidc {
    match sso_runtime_or_fail(state).await {
        Ok(runtime) => runtime,
        Err(err) => {
            tracing::warn!(
                error = %err,
                "reading saved SSO settings failed; using the environment's values"
            );
            let base = resolve_sso(&state.config, None, None);
            let pool = state.pg.as_deref().cloned();
            build_sso_runtime(state, base, pool).await
        }
    }
}

/// The effective `SSO` settings for a request: the environment's, else the
/// console's saved ones, else the code's defaults, with the addresses the
/// provider's discovery document names filled in (`ADM-12` D10).
pub async fn sso(state: &AppState) -> EffectiveSso {
    sso_runtime(state).await.settings
}

/// The `OIDC` authenticator for the effective `SSO` settings, rebuilt when
/// they change (plan D4). Present whenever the issuer and the client ID are
/// set — the bearer-token path's fact (`ADM-12` review round 2, R2-B1),
/// not the sign-in readiness [`sso_runtime`]'s callers read.
pub async fn oidc(state: &AppState) -> Option<Arc<lakehouse_auth::oidc::OidcAuthenticator>> {
    sso_runtime(state).await.authenticator
}

/// Build one SSO runtime — the ONE place an authenticator and its pinned
/// HTTP client are assembled for a set of SSO settings (`ADM-12` review
/// round 1, S5). The live path ([`sso_runtime`]) and the test sign-in
/// (`routes::settings::post_sso_test`) both call this, so their checks and
/// their transport cannot drift.
///
/// # What is checked, and when
///
/// A console-set connection is remote configuration: its issuer must be an
/// `https://` address outside this deployment's network (D6), and each
/// address the discovery document names must be too, before anything is
/// sent to it (D10). One that is refused answers [`SsoNotReady`] and builds
/// no authenticator, so no key set, token or userinfo request is ever made
/// to it. A server-owned connection is the operator's own file and is used
/// as it is, which is today's behaviour (D6).
///
/// # Two facts, kept apart (`ADM-12` review round 2, R2-B1)
///
/// The authenticator is built once the issuer and the client ID are set
/// (for a console-set connection, once the checks above passed); that is
/// the bearer-token path's fact, as on `main`. `not_ready` is the separate
/// sign-in fact: the authorize, token and redirect addresses must all be
/// there. A server-owned connection without them keeps its authenticator and
/// says why a sign-in is not offered; a console-set connection without a
/// return address builds nothing, because its addresses are remote
/// configuration and a sign-in is the only thing they are for.
#[allow(
    clippy::too_many_lines,
    reason = "one flat builder: the readiness checks and the authenticator they guard, in the order they must run (ADM-12 D10)"
)]
async fn build_sso_runtime(
    state: &AppState,
    base: EffectiveSso,
    pool: Option<PgPool>,
) -> SwappableOidc {
    let built_at = Instant::now();
    let mut settings = base.clone();
    let default_http = reqwest::Client::new();

    if !base.is_configured() {
        return not_ready_runtime(base, settings, default_http, SsoNotReady::NotConfigured);
    }
    let Some(pool) = pool else {
        return not_ready_runtime(base, settings, default_http, SsoNotReady::NoDatabase);
    };
    if base.issuer_source != Source::Console {
        // D3: the environment owns the whole connection, addresses included.
        // Its values are the operator's file, so they are used as they are
        // and are not checked (D6).
        //
        // `ADM-12` review round 2, R2-B1: the authenticator is built
        // whenever the issuer and the client ID are set, as on `main` — it
        // verifies bearer tokens whether or not this deployment offers a
        // console sign-in. The three sign-in addresses are the separate
        // "ready" fact; without them a sign-in answers `SsoNotReady`, and
        // the bearer path keeps working.
        let missing_sign_in_addresses = settings.authorize_url.is_none()
            || settings.token_url.is_none()
            || settings.redirect_uri.is_none();
        let authenticator = Arc::new(
            lakehouse_auth::oidc::OidcAuthenticator::new_with_http_client(
                settings.to_oidc_config(),
                state.sso_http_client(),
                pool,
            ),
        );
        return SwappableOidc {
            base,
            settings,
            authenticator: Some(authenticator),
            http: default_http,
            not_ready: missing_sign_in_addresses.then_some(SsoNotReady::ServerAddressesMissing),
            built_at,
        };
    }

    let Some(issuer) = base.issuer.clone() else {
        return not_ready_runtime(base, settings, default_http, SsoNotReady::NotConfigured);
    };
    let internal_hosts = state.config.connector_internal_hosts();

    // D6: the issuer itself, before a discovery request is made.
    if check_issuer_address(&issuer, &internal_hosts)
        .await
        .is_err()
    {
        tracing::warn!(
            "the OIDC issuer address is not one this deployment may reach; no authenticator was built"
        );
        return not_ready_runtime(base, settings, default_http, SsoNotReady::IssuerRefused);
    }

    // The console's own return address: `<CONSOLE_PUBLIC_URL>` +
    // `/api/auth/oidc/callback`, never a request header (D10). A set
    // `OIDC_REDIRECT_URI` still wins per value. A console-set connection
    // cannot offer a sign-in without it, so nothing is built (R2-B1 keeps
    // this gate for the console path only; the server-owned path above
    // keeps its authenticator without it).
    let redirect_uri = base
        .redirect_uri
        .clone()
        .or_else(|| state.config.console_public_url.as_deref().map(callback_url));
    let Some(redirect_uri) = redirect_uri else {
        return not_ready_runtime(base, settings, default_http, SsoNotReady::NoPublicAddress);
    };

    // The discovery read is the same one the key set uses (`lakehouse-auth`,
    // `SEC-38`): one parser, one HTTP client, no second fetcher (rule 4).
    // The issuer's host is checked and pinned for it.
    let discovery_http = match state.sso_http_client() {
        Some(client) => client,
        None => match pinned_client(&[issuer.as_str()], &internal_hosts).await {
            Ok(client) => client,
            Err(()) => {
                return not_ready_runtime(base, settings, default_http, SsoNotReady::IssuerRefused);
            }
        },
    };
    let Ok(endpoints) =
        lakehouse_auth::oidc::read_discovery_endpoints(&issuer, discovery_http).await
    else {
        return not_ready_runtime(
            base,
            settings,
            default_http,
            SsoNotReady::DiscoveryUnreadable,
        );
    };

    // D10: each address the document names, before anything is sent there.
    for url in [
        endpoints.authorization_endpoint.as_str(),
        endpoints.token_endpoint.as_str(),
        endpoints.jwks_uri.as_str(),
    ] {
        if check_external_https_address(url, &internal_hosts)
            .await
            .is_err()
        {
            tracing::warn!(
                "the OIDC discovery document names an address this deployment may not use; no authenticator was built"
            );
            return not_ready_runtime(base, settings, default_http, SsoNotReady::AddressRefused);
        }
    }

    // `.env` still wins per value (D10): a set address is used as it is, and
    // only the discovery-named ones are checked above.
    settings.authorize_url = base
        .authorize_url
        .clone()
        .or_else(|| Some(endpoints.authorization_endpoint.clone()));
    settings.token_url = base
        .token_url
        .clone()
        .or_else(|| Some(endpoints.token_endpoint.clone()));
    settings.jwks_url = base
        .jwks_url
        .clone()
        .or_else(|| Some(endpoints.jwks_uri.clone()));
    settings.redirect_uri = Some(redirect_uri);

    // One client, pinned to every address this connection was allowed to
    // reach: the issuer, and the three the document named. A value the
    // environment set is used as it is (D10) and resolves normally — it is
    // the operator's own file, not remote input.
    let http = if let Some(client) = state.sso_http_client() {
        client
    } else {
        let urls = [
            issuer.as_str(),
            endpoints.authorization_endpoint.as_str(),
            endpoints.token_endpoint.as_str(),
            endpoints.jwks_uri.as_str(),
        ];
        let Ok(client) = pinned_client(&urls, &internal_hosts).await else {
            return not_ready_runtime(base, settings, default_http, SsoNotReady::AddressRefused);
        };
        client
    };

    // The key-set address is now explicit, so the authenticator never
    // re-reads a discovery document whose addresses were not checked.
    let authenticator = Arc::new(
        lakehouse_auth::oidc::OidcAuthenticator::new_with_http_client(
            settings.to_oidc_config(),
            Some(http.clone()),
            pool,
        ),
    );
    SwappableOidc {
        base,
        settings,
        authenticator: Some(authenticator),
        http,
        not_ready: None,
        built_at,
    }
}

/// A runtime that says, in one fixed sentence, why it cannot be used, and
/// that exposes no authenticator (`ADM-12` D10: not ready is said, not
/// hidden, and nothing is fetched from a refused address).
fn not_ready_runtime(
    base: EffectiveSso,
    settings: EffectiveSso,
    http: reqwest::Client,
    reason: SsoNotReady,
) -> SwappableOidc {
    SwappableOidc {
        base,
        settings,
        authenticator: None,
        http,
        not_ready: Some(reason),
        built_at: Instant::now(),
    }
}

/// The sign-in return address for a console-set connection: the operator's
/// `CONSOLE_PUBLIC_URL` and this deployment's fixed callback path (`ADM-12`
/// D10). Never taken from a request.
fn callback_url(public_url: &str) -> String {
    format!("{}{OIDC_CALLBACK_PATH}", public_url.trim_end_matches('/'))
}

/// A `reqwest` client pinned to the checked addresses of every host in
/// `urls` (`ADM-12` D6; `SEC-15`'s allowlisted resolver).
///
/// Pinning makes the address that passed [`crate::connector_probe::resolve_checked`]
/// the one the request goes to, so a name that resolves somewhere else
/// between the check and the request cannot redirect it — the same reasoning
/// that module's dials give.
///
/// # Errors
///
/// `Err(())` when a URL names no host, a host does not resolve, a host
/// resolves inside this deployment's network, or the client cannot be
/// built. The caller turns each into a fixed [`SsoNotReady`] sentence.
async fn pinned_client(
    urls: &[&str],
    internal_hosts: &InternalHosts,
) -> Result<reqwest::Client, ()> {
    // A provider that never answers must not hold a request open: discovery
    // and the token exchange are bounded, and a refusal is a plain sentence.
    let mut builder = reqwest::Client::builder().timeout(Duration::from_secs(10));
    for url in urls {
        let Some((host, port)) = crate::connector_probe::parse_endpoint_host_port(url) else {
            return Err(());
        };
        let Ok(checked) = crate::connector_probe::resolve_checked(host, port, internal_hosts).await
        else {
            return Err(());
        };
        builder = builder.resolve_to_addrs(host, checked.all());
    }
    builder.build().map_err(|_| ())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::collections::HashMap;

    use base64::Engine as _;

    use super::*;

    /// base64 of 32 bytes of `0x2a`, the same shape the test harness uses
    /// for `CONNECTOR_SECRETS_KEY` (never a real secret).
    fn test_key() -> String {
        base64::engine::general_purpose::STANDARD.encode([0x2a_u8; 32])
    }

    fn keyring() -> Keyring {
        Keyring::from_values(Some(&test_key()), None).unwrap()
    }

    fn config(env: &[(&str, &str)]) -> Config {
        let map: HashMap<String, String> = env
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        Config::from_map(&map).unwrap()
    }

    fn stored(over: impl Fn(&mut EmailSettings)) -> EmailSettings {
        let mut s = EmailSettings {
            smtp_host: None,
            smtp_port: None,
            smtp_security: None,
            smtp_user: None,
            smtp_pass: None,
            smtp_from: None,
            console_public_url: None,
        };
        over(&mut s);
        s
    }

    #[test]
    fn environment_values_win_over_console_and_default() {
        let cfg = config(&[
            ("SMTP_HOST", "env.example.com"),
            ("SMTP_PORT", "2525"),
            ("SMTP_USER", "env-user"),
            ("SMTP_PASS", "env-pass"),
            ("SMTP_FROM", "env@example.com"),
            ("CONSOLE_PUBLIC_URL", "https://env.example.com"),
        ]);
        let saved = stored(|s| {
            s.smtp_host = Some("console.example.com".to_owned());
            s.smtp_port = Some(465);
            s.smtp_user = Some("console-user".to_owned());
            s.smtp_from = Some("console@example.com".to_owned());
            s.console_public_url = Some("https://console.example.com".to_owned());
        });
        let eff = resolve_email(&cfg, Some(&saved), Some(&keyring()));

        assert_eq!(eff.host.as_deref(), Some("env.example.com"));
        assert_eq!(eff.host_source, Source::Server);
        assert_eq!(eff.port, 2525);
        assert_eq!(eff.port_source, Source::Server);
        assert_eq!(eff.user.as_deref(), Some("env-user"));
        assert_eq!(eff.user_source, Source::Server);
        assert!(eff.password_set);
        assert_eq!(eff.password.expose_secret(), "env-pass");
        assert_eq!(eff.password_source, Source::Server);
        assert_eq!(eff.from, "env@example.com");
        assert_eq!(eff.from_source, Source::Server);
        assert_eq!(
            eff.console_public_url.as_deref(),
            Some("https://env.example.com")
        );
        assert_eq!(eff.console_public_url_source, Source::Server);
    }

    #[test]
    fn console_values_are_used_when_the_environment_is_empty() {
        let cfg = config(&[]);
        let saved = stored(|s| {
            s.smtp_host = Some("console.example.com".to_owned());
            s.smtp_port = Some(465);
            s.smtp_security = Some("tls".to_owned());
            s.smtp_user = Some("console-user".to_owned());
            s.smtp_pass = Some(
                keyring()
                    .seal(EMAIL_PASSWORD_SEAL_NAME, b"console-pass")
                    .unwrap(),
            );
            s.smtp_from = Some("console@example.com".to_owned());
            s.console_public_url = Some("https://console.example.com".to_owned());
        });
        let eff = resolve_email(&cfg, Some(&saved), Some(&keyring()));

        assert_eq!(eff.host.as_deref(), Some("console.example.com"));
        assert_eq!(eff.host_source, Source::Console);
        assert_eq!(eff.port, 465);
        assert_eq!(eff.port_source, Source::Console);
        assert_eq!(eff.security, EmailSecurity::Tls);
        assert_eq!(eff.security_source, Source::Console);
        assert_eq!(eff.user.as_deref(), Some("console-user"));
        assert_eq!(eff.user_source, Source::Console);
        assert!(eff.password_set);
        assert_eq!(eff.password.expose_secret(), "console-pass");
        assert_eq!(eff.password_source, Source::Console);
        assert_eq!(eff.from, "console@example.com");
        assert_eq!(eff.from_source, Source::Console);
        assert_eq!(
            eff.console_public_url.as_deref(),
            Some("https://console.example.com")
        );
        assert_eq!(eff.console_public_url_source, Source::Console);
    }

    #[test]
    fn defaults_apply_when_neither_environment_nor_console_sets_a_field() {
        let cfg = config(&[]);
        let eff = resolve_email(&cfg, None, Some(&keyring()));

        assert_eq!(eff.host, None);
        assert_eq!(eff.host_source, Source::Default);
        assert_eq!(eff.port, 587);
        assert_eq!(eff.port_source, Source::Default);
        assert_eq!(eff.security, EmailSecurity::Starttls);
        assert_eq!(eff.security_source, Source::Default);
        assert_eq!(eff.user, None);
        assert_eq!(eff.user_source, Source::Default);
        assert!(!eff.password_set);
        assert_eq!(eff.password.expose_secret(), "");
        assert_eq!(eff.password_source, Source::Default);
        assert_eq!(eff.from, "rantai-lake@localhost");
        assert_eq!(eff.from_source, Source::Default);
        assert_eq!(eff.console_public_url, None);
        assert_eq!(eff.console_public_url_source, Source::Default);
    }

    #[test]
    fn a_sealed_password_opens_with_the_keyring() {
        let cfg = config(&[]);
        let ring = keyring();
        let saved = stored(|s| {
            s.smtp_pass = Some(ring.seal(EMAIL_PASSWORD_SEAL_NAME, b"open-sesame").unwrap());
        });
        let eff = resolve_email(&cfg, Some(&saved), Some(&ring));
        assert!(eff.password_set);
        assert_eq!(eff.password.expose_secret(), "open-sesame");
        assert_eq!(eff.password_source, Source::Console);
    }

    #[test]
    fn a_row_sealed_with_another_key_reads_as_not_set() {
        let cfg = config(&[]);
        let ring_a = keyring();
        let sealed = ring_a.seal(EMAIL_PASSWORD_SEAL_NAME, b"secret").unwrap();
        let saved = stored(|s| s.smtp_pass = Some(sealed.clone()));

        // A different key (base64 of 0x2b) does not open the row.
        let other = base64::engine::general_purpose::STANDARD.encode([0x2b_u8; 32]);
        let ring_b = Keyring::from_values(Some(&other), None).unwrap();

        let eff = resolve_email(&cfg, Some(&saved), Some(&ring_b));
        assert!(!eff.password_set);
        assert_eq!(eff.password.expose_secret(), "");
        assert_eq!(eff.password_source, Source::Default);
    }

    #[test]
    fn smtp_secure_true_is_the_environment_making_it_implicit_tls() {
        // SMTP_SECURE=true in the environment is a `Server` source for the
        // security field, even with nothing saved in the console.
        let cfg = config(&[("SMTP_SECURE", "true")]);
        let eff = resolve_email(&cfg, None, Some(&keyring()));
        assert_eq!(eff.security, EmailSecurity::Tls);
        assert_eq!(eff.security_source, Source::Server);
    }

    /// `ADM-7 review R1`: the connection fields are one unit. The
    /// environment sets only the password, so it owns the whole connection:
    /// the console's saved host is not used, and every one of the five
    /// fields says where it comes from.
    #[test]
    fn the_environments_connection_is_one_unit() {
        let cfg = config(&[("SMTP_PASS", "env-pass")]);
        let saved = stored(|s| {
            s.smtp_host = Some("console.example.com".to_owned());
            s.smtp_port = Some(465);
            s.smtp_security = Some("tls".to_owned());
            s.smtp_user = Some("console-user".to_owned());
            s.smtp_pass = Some(
                keyring()
                    .seal(EMAIL_PASSWORD_SEAL_NAME, b"console-pass")
                    .unwrap(),
            );
        });
        let eff = resolve_email(&cfg, Some(&saved), Some(&keyring()));

        // The environment's host is none, so email is not set up — the
        // console's host is NOT used for the send.
        assert_eq!(eff.host, None, "the console's host is not used");
        assert_eq!(eff.host_source, Source::Server);
        assert_eq!(eff.port, 587);
        assert_eq!(eff.port_source, Source::Server);
        assert_eq!(eff.security, EmailSecurity::Starttls);
        assert_eq!(eff.security_source, Source::Server);
        assert_eq!(eff.user, None, "the console's account is not used");
        assert_eq!(eff.user_source, Source::Server);
        assert!(eff.password_set);
        assert_eq!(eff.password.expose_secret(), "env-pass");
        assert_eq!(eff.password_source, Source::Server);
    }

    /// `ADM-7 review R1`: `from` and `consolePublicUrl` are not part of the
    /// login, so they stay per field: the console may still set them while
    /// the environment owns the connection.
    #[test]
    fn the_from_address_and_the_console_address_stay_per_field() {
        let cfg = config(&[("SMTP_PASS", "env-pass")]);
        let saved = stored(|s| {
            s.smtp_from = Some("console@example.com".to_owned());
            s.console_public_url = Some("https://console.example.com".to_owned());
        });
        let eff = resolve_email(&cfg, Some(&saved), Some(&keyring()));

        assert_eq!(eff.from, "console@example.com");
        assert_eq!(eff.from_source, Source::Console);
        assert_eq!(
            eff.console_public_url.as_deref(),
            Some("https://console.example.com")
        );
        assert_eq!(eff.console_public_url_source, Source::Console);
    }

    /// `ADM-7 review R1`: the unit rule is about the connection only. With
    /// nothing in the environment, the console's five fields are used, as
    /// before.
    #[test]
    fn the_console_owns_the_connection_when_the_environment_says_nothing() {
        let cfg = config(&[]);
        let saved = stored(|s| {
            s.smtp_host = Some("console.example.com".to_owned());
            s.smtp_port = Some(465);
            s.smtp_security = Some("tls".to_owned());
            s.smtp_user = Some("console-user".to_owned());
        });
        let eff = resolve_email(&cfg, Some(&saved), Some(&keyring()));

        assert_eq!(eff.host.as_deref(), Some("console.example.com"));
        assert_eq!(eff.host_source, Source::Console);
        assert_eq!(eff.port, 465);
        assert_eq!(eff.port_source, Source::Console);
        assert_eq!(eff.security, EmailSecurity::Tls);
        assert_eq!(eff.security_source, Source::Console);
        assert_eq!(eff.user.as_deref(), Some("console-user"));
        assert_eq!(eff.user_source, Source::Console);
    }

    // ── The sign-in limits (`ADM-8`) ──────────────────────────────────────

    fn stored_sign_in(over: impl Fn(&mut SignInSettings)) -> SignInSettings {
        let mut s = SignInSettings {
            max_failures: None,
            failure_window_secs: None,
            lockout_secs: None,
            retention_days: None,
            embed_max_lifetime_secs: None,
        };
        over(&mut s);
        s
    }

    /// The environment wins over the console's saved numbers, and the GET
    /// says so (plan D1).
    #[test]
    fn the_environment_wins_over_the_console_for_the_sign_in_limits() {
        let cfg = config(&[
            ("LOGIN_MAX_FAILURES", "7"),
            ("LOGIN_FAILURE_WINDOW_SECS", "1200"),
            ("LOGIN_LOCKOUT_SECS", "600"),
            ("AUTH_RETENTION_DAYS", "60"),
            ("EMBED_TOKEN_MAX_LIFETIME_SECONDS", "7200"),
        ]);
        let saved = stored_sign_in(|s| {
            s.max_failures = Some(3);
            s.failure_window_secs = Some(600);
            s.lockout_secs = Some(120);
            s.retention_days = Some(14);
            s.embed_max_lifetime_secs = Some(3600);
        });
        let eff = resolve_sign_in(&cfg, Some(&saved));

        assert_eq!(eff.max_failures, 7);
        assert_eq!(eff.max_failures_source, Source::Server);
        assert_eq!(eff.failure_window_secs, 1200);
        assert_eq!(eff.failure_window_secs_source, Source::Server);
        assert_eq!(eff.lockout_secs, 600);
        assert_eq!(eff.lockout_secs_source, Source::Server);
        assert_eq!(eff.retention_days, 60);
        assert_eq!(eff.retention_days_source, Source::Server);
        assert_eq!(eff.embed_max_lifetime_secs, 7200);
        assert_eq!(eff.embed_max_lifetime_secs_source, Source::Server);
    }

    /// With nothing in the environment, the console's saved numbers are used.
    #[test]
    fn the_console_owns_the_sign_in_limits_when_the_environment_says_nothing() {
        let cfg = config(&[]);
        let saved = stored_sign_in(|s| {
            s.max_failures = Some(3);
            s.failure_window_secs = Some(600);
            s.lockout_secs = Some(120);
            s.retention_days = Some(14);
            s.embed_max_lifetime_secs = Some(3600);
        });
        let eff = resolve_sign_in(&cfg, Some(&saved));

        assert_eq!(eff.max_failures, 3);
        assert_eq!(eff.max_failures_source, Source::Console);
        assert_eq!(eff.failure_window_secs, 600);
        assert_eq!(eff.failure_window_secs_source, Source::Console);
        assert_eq!(eff.lockout_secs, 120);
        assert_eq!(eff.lockout_secs_source, Source::Console);
        assert_eq!(eff.retention_days, 14);
        assert_eq!(eff.retention_days_source, Source::Console);
        assert_eq!(eff.embed_max_lifetime_secs, 3600);
        assert_eq!(eff.embed_max_lifetime_secs_source, Source::Console);
    }

    /// Neither the environment nor the console: the code's own numbers, said
    /// as such.
    #[test]
    fn the_defaults_apply_when_neither_sets_a_sign_in_limit() {
        let cfg = config(&[]);
        let eff = resolve_sign_in(&cfg, None);

        assert_eq!(eff.max_failures, 5);
        assert_eq!(eff.max_failures_source, Source::Default);
        assert_eq!(eff.failure_window_secs, 900);
        assert_eq!(eff.failure_window_secs_source, Source::Default);
        assert_eq!(eff.lockout_secs, 300);
        assert_eq!(eff.lockout_secs_source, Source::Default);
        assert_eq!(eff.retention_days, 30);
        assert_eq!(eff.retention_days_source, Source::Default);
        assert_eq!(eff.embed_max_lifetime_secs, 86_400);
        assert_eq!(eff.embed_max_lifetime_secs_source, Source::Default);
    }

    /// An environment value that does not parse is not the environment's
    /// value: the code default is in effect, and the console may set a real
    /// one (the rule the email connection uses for an unparseable port).
    #[test]
    fn an_unparseable_environment_limit_does_not_own_the_setting() {
        let cfg = config(&[("LOGIN_MAX_FAILURES", "many")]);
        let saved = stored_sign_in(|s| s.max_failures = Some(3));
        let eff = resolve_sign_in(&cfg, Some(&saved));

        assert_eq!(eff.max_failures, 3);
        assert_eq!(eff.max_failures_source, Source::Console);
    }

    /// The throttle policy the login handler uses is built from the resolved
    /// numbers, so a console's saved limit is the one that locks a key.
    #[test]
    fn the_throttle_policy_follows_the_resolved_limits() {
        let cfg = config(&[]);
        let saved = stored_sign_in(|s| {
            s.max_failures = Some(3);
            s.failure_window_secs = Some(600);
            s.lockout_secs = Some(120);
        });
        let eff = resolve_sign_in(&cfg, Some(&saved));
        let policy = eff.throttle_policy();

        assert_eq!(policy.max_failures, 3);
        assert_eq!(policy.window, time::Duration::seconds(600));
        assert_eq!(policy.lockout, time::Duration::seconds(120));
    }

    // ── The assistant's provider (`ADM-9`) ───────────────────────────────

    fn stored_assistant(over: impl Fn(&mut AssistantSettings)) -> AssistantSettings {
        let mut s = AssistantSettings {
            llm_url: None,
            llm_model: None,
            llm_key: None,
        };
        over(&mut s);
        s
    }

    /// `ADM-7 review R1`'s lesson, applied to the assistant: the address,
    /// the model and the key are ONE unit, and the environment owns the
    /// whole of it as soon as it holds a key. The console's saved address
    /// is NOT used, so the environment's key is never sent to an address the
    /// console chose.
    #[test]
    fn the_environments_key_owns_the_assistants_whole_unit() {
        let cfg = config(&[
            ("LLM_KEY", "an-environment-key"),
            ("LLM_URL", "https://env.example.com/v1"),
            ("LLM_MODEL", "env-model"),
        ]);
        let saved = stored_assistant(|s| {
            s.llm_url = Some("https://console.example.com/v1".to_owned());
            s.llm_model = Some("console-model".to_owned());
            s.llm_key = Some(
                keyring()
                    .seal(ASSISTANT_KEY_SEAL_NAME, b"console-key")
                    .unwrap(),
            );
        });
        let eff = resolve_assistant(&cfg, Some(&saved), Some(&keyring()));

        assert_eq!(eff.url, "https://env.example.com/v1");
        assert_eq!(eff.url_source, Source::Server);
        assert_eq!(eff.model, "env-model");
        assert_eq!(eff.model_source, Source::Server);
        assert!(eff.key_set);
        assert_eq!(eff.key.expose_secret(), "an-environment-key");
        assert_eq!(eff.key_source, Source::Server);
    }

    /// `MINIMAX_API_KEY` is the other name of the environment's key, and it
    /// owns the unit the same way.
    #[test]
    fn the_environments_minimax_key_owns_the_assistants_whole_unit() {
        let cfg = config(&[("MINIMAX_API_KEY", "an-environment-key")]);
        let saved =
            stored_assistant(|s| s.llm_url = Some("https://console.example.com/v1".to_owned()));
        let eff = resolve_assistant(&cfg, Some(&saved), Some(&keyring()));

        assert_eq!(eff.url_source, Source::Server);
        assert!(eff.key_set);
        assert_eq!(eff.key.expose_secret(), "an-environment-key");
    }

    /// With no key in the environment, the console owns the unit: its saved
    /// address, model and key are used.
    #[test]
    fn the_console_owns_the_assistant_when_the_environment_holds_no_key() {
        let cfg = config(&[]);
        let saved = stored_assistant(|s| {
            s.llm_url = Some("https://console.example.com/v1".to_owned());
            s.llm_model = Some("console-model".to_owned());
            s.llm_key = Some(
                keyring()
                    .seal(ASSISTANT_KEY_SEAL_NAME, b"console-key")
                    .unwrap(),
            );
        });
        let eff = resolve_assistant(&cfg, Some(&saved), Some(&keyring()));

        assert_eq!(eff.url, "https://console.example.com/v1");
        assert_eq!(eff.url_source, Source::Console);
        assert_eq!(eff.model, "console-model");
        assert_eq!(eff.model_source, Source::Console);
        assert!(eff.key_set);
        assert_eq!(eff.key.expose_secret(), "console-key");
        assert_eq!(eff.key_source, Source::Console);
    }

    /// `ADM-9 review R4`: the environment's own address wins on its own,
    /// with no key beside it. The console's saved address is NOT used, and
    /// the address is the server's (`server`), so the console shows it
    /// read-only and refuses a save that sends it.
    #[test]
    fn the_environments_own_address_wins_on_its_own() {
        let cfg = config(&[("LLM_URL", "https://env.example.com/v1")]);
        let saved = stored_assistant(|s| {
            s.llm_url = Some("https://console.example.com/v1".to_owned());
            s.llm_model = Some("console-model".to_owned());
        });
        let eff = resolve_assistant(&cfg, Some(&saved), Some(&keyring()));

        assert_eq!(eff.url, "https://env.example.com/v1");
        assert_eq!(eff.url_source, Source::Server);
        // The model is still the console's: the environment named none.
        assert_eq!(eff.model, "console-model");
        assert_eq!(eff.model_source, Source::Console);
        // And the key is still the console's.
        assert!(!eff.key_set);
        assert_eq!(eff.key_source, Source::Default);
    }

    /// The model half of the same rule (`ADM-9 review R4`).
    #[test]
    fn the_environments_own_model_wins_on_its_own() {
        let cfg = config(&[("LLM_MODEL", "env-model")]);
        let saved = stored_assistant(|s| {
            s.llm_url = Some("https://console.example.com/v1".to_owned());
            s.llm_model = Some("console-model".to_owned());
        });
        let eff = resolve_assistant(&cfg, Some(&saved), Some(&keyring()));

        assert_eq!(eff.model, "env-model");
        assert_eq!(eff.model_source, Source::Server);
        // The address is still the console's.
        assert_eq!(eff.url, "https://console.example.com/v1");
        assert_eq!(eff.url_source, Source::Console);
    }

    /// An empty `LLM_URL` is no address: the console may set one, and the
    /// code's default is used until it does.
    #[test]
    fn an_empty_environments_address_is_not_the_environments_own() {
        let cfg = config(&[("LLM_URL", "")]);
        let saved =
            stored_assistant(|s| s.llm_url = Some("https://console.example.com/v1".to_owned()));
        let eff = resolve_assistant(&cfg, Some(&saved), Some(&keyring()));

        assert_eq!(eff.url, "https://console.example.com/v1");
        assert_eq!(eff.url_source, Source::Console);
    }

    /// Neither the environment's key nor the console's: the code's default
    /// address and model, and no key at all (the assistant is off).
    #[test]
    fn the_assistant_defaults_apply_when_nothing_is_set() {
        let cfg = config(&[]);
        let eff = resolve_assistant(&cfg, None, Some(&keyring()));

        assert_eq!(eff.url, crate::config::DEFAULT_LLM_URL);
        assert_eq!(eff.url_source, Source::Default);
        assert_eq!(eff.model, crate::config::DEFAULT_LLM_MODEL);
        assert_eq!(eff.model_source, Source::Default);
        assert!(!eff.key_set);
        assert_eq!(eff.key.expose_secret(), "");
        assert_eq!(eff.key_source, Source::Default);
    }

    /// A saved key sealed with another installation key reads as not set,
    /// and is never returned.
    #[test]
    fn an_assistant_key_sealed_with_another_key_reads_as_not_set() {
        let cfg = config(&[]);
        let ring_a = keyring();
        let sealed = ring_a.seal(ASSISTANT_KEY_SEAL_NAME, b"secret").unwrap();
        let saved = stored_assistant(|s| s.llm_key = Some(sealed.clone()));

        // A different key (base64 of 0x2b) does not open the row.
        let other = base64::engine::general_purpose::STANDARD.encode([0x2b_u8; 32]);
        let ring_b = Keyring::from_values(Some(&other), None).unwrap();

        let eff = resolve_assistant(&cfg, Some(&saved), Some(&ring_b));
        assert!(!eff.key_set);
        assert_eq!(eff.key.expose_secret(), "");
        assert_eq!(eff.key_source, Source::Default);
    }

    fn stored_sso(over: impl FnOnce(&mut SsoSettings)) -> SsoSettings {
        let mut s = SsoSettings {
            oidc_issuer: None,
            oidc_client_id: None,
            oidc_client_secret: None,
            oidc_provider_name: None,
            oidc_groups_claim: None,
            oidc_role_map: None,
            oidc_jit_provisioning: None,
        };
        over(&mut s);
        s
    }

    #[test]
    fn an_env_issuer_makes_the_connection_server_owned() {
        let cfg = config(&[
            ("OIDC_ISSUER", "https://env-issuer.example.com"),
            ("OIDC_CLIENT_ID", "env-client"),
            ("OIDC_CLIENT_SECRET", "env-secret"),
        ]);
        let ring = keyring();
        let saved = stored_sso(|s| {
            s.oidc_issuer = Some("https://console.example.com".to_owned());
            s.oidc_client_id = Some("console-client".to_owned());
            s.oidc_client_secret = Some(
                ring.seal(SSO_CLIENT_SECRET_SEAL_NAME, b"console-secret")
                    .unwrap(),
            );
        });
        let eff = resolve_sso(&cfg, Some(&saved), Some(&ring));

        assert_eq!(
            eff.issuer.as_deref(),
            Some("https://env-issuer.example.com")
        );
        assert_eq!(eff.issuer_source, Source::Server);
        assert_eq!(eff.client_id.as_deref(), Some("env-client"));
        assert_eq!(eff.client_id_source, Source::Server);
        assert!(eff.client_secret_set);
        assert_eq!(eff.client_secret.expose_secret(), "env-secret");
        assert_eq!(eff.client_secret_source, Source::Server);
        assert!(eff.is_configured());
    }

    #[test]
    fn an_env_client_id_alone_makes_the_connection_server_owned() {
        let cfg = config(&[("OIDC_CLIENT_ID", "env-client")]);
        let ring = keyring();
        let saved = stored_sso(|s| {
            s.oidc_issuer = Some("https://console.example.com".to_owned());
        });
        let eff = resolve_sso(&cfg, Some(&saved), Some(&ring));

        assert_eq!(eff.issuer, None);
        assert_eq!(eff.issuer_source, Source::Server);
        assert_eq!(eff.client_id.as_deref(), Some("env-client"));
        assert_eq!(eff.client_id_source, Source::Server);
    }

    #[test]
    fn a_saved_row_is_used_when_the_environment_is_empty() {
        let cfg = config(&[]);
        let ring = keyring();
        let role_map = serde_json::json!([
            {"group": "admins", "role": "Platform Admin"}
        ]);
        let saved = stored_sso(|s| {
            s.oidc_issuer = Some("https://console.example.com".to_owned());
            s.oidc_client_id = Some("console-client".to_owned());
            s.oidc_client_secret = Some(
                ring.seal(SSO_CLIENT_SECRET_SEAL_NAME, b"console-secret")
                    .unwrap(),
            );
            s.oidc_provider_name = Some("custom-okta".to_owned());
            s.oidc_groups_claim = Some("custom_groups".to_owned());
            s.oidc_role_map = Some(role_map);
            s.oidc_jit_provisioning = Some(true);
        });
        let eff = resolve_sso(&cfg, Some(&saved), Some(&ring));

        assert_eq!(eff.issuer.as_deref(), Some("https://console.example.com"));
        assert_eq!(eff.issuer_source, Source::Console);
        assert_eq!(eff.client_id.as_deref(), Some("console-client"));
        assert_eq!(eff.client_id_source, Source::Console);
        assert!(eff.client_secret_set);
        assert_eq!(eff.client_secret.expose_secret(), "console-secret");
        assert_eq!(eff.client_secret_source, Source::Console);
        assert_eq!(eff.provider_name, "custom-okta");
        assert_eq!(eff.provider_name_source, Source::Console);
        assert_eq!(eff.groups_claim, "custom_groups");
        assert_eq!(eff.groups_claim_source, Source::Console);
        assert_eq!(
            eff.role_map.get("admins").map(String::as_str),
            Some("Platform Admin")
        );
        assert_eq!(eff.role_map_source, Source::Console);
        assert!(eff.jit_provisioning);
        assert_eq!(eff.jit_provisioning_source, Source::Console);
        assert!(eff.is_configured());
    }

    #[test]
    fn the_defaults_apply_when_both_are_empty() {
        let cfg = config(&[]);
        let eff = resolve_sso(&cfg, None, Some(&keyring()));

        assert_eq!(eff.issuer, None);
        assert_eq!(eff.issuer_source, Source::Default);
        assert_eq!(eff.client_id, None);
        assert_eq!(eff.client_id_source, Source::Default);
        assert!(!eff.client_secret_set);
        assert_eq!(eff.client_secret.expose_secret(), "");
        assert_eq!(eff.client_secret_source, Source::Default);
        assert_eq!(eff.provider_name, "default");
        assert_eq!(eff.provider_name_source, Source::Default);
        assert_eq!(eff.groups_claim, "groups");
        assert_eq!(eff.groups_claim_source, Source::Default);
        assert!(eff.role_map.is_empty());
        assert_eq!(eff.role_map_source, Source::Default);
        assert!(!eff.jit_provisioning);
        assert_eq!(eff.jit_provisioning_source, Source::Default);
        assert!(!eff.is_configured());
    }

    #[test]
    fn the_secret_opens_and_fails_closed_when_the_key_is_missing() {
        let cfg = config(&[]);
        let ring_a = keyring();
        let sealed = ring_a
            .seal(SSO_CLIENT_SECRET_SEAL_NAME, b"open-sesame")
            .unwrap();
        let saved = stored_sso(|s| s.oidc_client_secret = Some(sealed.clone()));

        // Opens with correct keyring
        let eff = resolve_sso(&cfg, Some(&saved), Some(&ring_a));
        assert!(eff.client_secret_set);
        assert_eq!(eff.client_secret.expose_secret(), "open-sesame");
        assert_eq!(eff.client_secret_source, Source::Console);

        // Fails closed when keyring is None
        let eff_none = resolve_sso(&cfg, Some(&saved), None);
        assert!(!eff_none.client_secret_set);
        assert_eq!(eff_none.client_secret.expose_secret(), "");
        assert_eq!(eff_none.client_secret_source, Source::Default);

        // Fails closed when keyring is wrong
        let other = base64::engine::general_purpose::STANDARD.encode([0x2b_u8; 32]);
        let ring_b = Keyring::from_values(Some(&other), None).unwrap();
        let eff_wrong = resolve_sso(&cfg, Some(&saved), Some(&ring_b));
        assert!(!eff_wrong.client_secret_set);
        assert_eq!(eff_wrong.client_secret.expose_secret(), "");
        assert_eq!(eff_wrong.client_secret_source, Source::Default);
    }

    // `ADM-12` review round 2, R2-S1: the runtime cache keeps a ready
    // connection for `READY_SSO_RUNTIME_TTL`, retries a not-ready one after
    // `SETTINGS_CACHE_TTL`, and a save clears it at once. The tests age
    // `built_at` instead of sleeping, and use a server-owned connection so a
    // rebuild builds an authenticator without a network read.

    /// An `AppState` whose SSO connection is the environment's, with the
    /// three sign-in addresses when `with_sign_in_addresses` is set. The
    /// database URL is well formed and unreachable: `connect_lazy` performs
    /// no I/O, so the pool is `Some` and nothing dials it.
    fn sso_state(with_sign_in_addresses: bool) -> AppState {
        let mut env = vec![
            (
                "DATABASE_URL",
                "postgres://user:pass@127.0.0.1:1/nonexistent_db_xyz",
            ),
            ("OIDC_ISSUER", "https://idp.example.com"),
            ("OIDC_CLIENT_ID", "lakehouse-console"),
        ];
        if with_sign_in_addresses {
            env.extend([
                ("OIDC_AUTHORIZE_URL", "https://idp.example.com/authorize"),
                ("OIDC_TOKEN_URL", "https://idp.example.com/token"),
                (
                    "OIDC_REDIRECT_URI",
                    "https://console.example.com/api/auth/oidc/callback",
                ),
            ]);
        }
        AppState::new(config(&env))
    }

    /// The `base` `cached_or_build` is asked for on this state.
    fn environment_base(state: &AppState) -> EffectiveSso {
        resolve_sso(&state.config, None, None)
    }

    /// An instant `age` in the past, for a runtime the cache should see as
    /// old. `checked_sub` rather than `-` because clippy forbids the
    /// panicking operator outside the type's own arithmetic.
    fn built_ago(age: Duration) -> Instant {
        Instant::now()
            .checked_sub(age)
            .expect("the test process is older than the age asked for")
    }

    async fn built_runtime(state: &AppState) -> SwappableOidc {
        let pool = state.pg.as_deref().cloned();
        build_sso_runtime(state, environment_base(state), pool).await
    }

    #[tokio::test]
    async fn a_ready_runtime_is_kept_for_ten_minutes() {
        let state = sso_state(true);
        let first = built_runtime(&state).await;
        assert!(first.is_ready(), "{:?}", first.not_ready);

        // Older than `SETTINGS_CACHE_TTL`, younger than
        // `READY_SSO_RUNTIME_TTL`: still reused, so no discovery read and no
        // new key-set cache for the bearer-token path to wait on.
        let mut aged = first.clone();
        aged.built_at = built_ago(SETTINGS_CACHE_TTL + Duration::from_secs(30));
        *state.oidc.write().await = Some(aged.clone());

        let next = cached_or_build(&state, environment_base(&state)).await;
        assert!(
            Arc::ptr_eq(
                next.authenticator.as_ref().unwrap(),
                first.authenticator.as_ref().unwrap()
            ),
            "a ready runtime must not be rebuilt inside its ten minutes"
        );
        assert_eq!(next.built_at, aged.built_at);
    }

    #[tokio::test]
    async fn a_not_ready_runtime_is_retried_after_the_settings_ttl() {
        let state = sso_state(false);
        let first = built_runtime(&state).await;
        assert!(!first.is_ready());
        // The authenticator is there (R2-B1); only a sign-in is missing.
        assert!(first.authenticator.is_some());

        let mut aged = first.clone();
        aged.built_at = built_ago(SETTINGS_CACHE_TTL + Duration::from_secs(30));
        *state.oidc.write().await = Some(aged.clone());

        let next = cached_or_build(&state, environment_base(&state)).await;
        assert!(
            next.built_at > aged.built_at,
            "a not-ready runtime must be rebuilt once the settings TTL passed"
        );
        assert!(!next.is_ready());
    }

    #[tokio::test]
    async fn a_save_clears_a_ready_runtime_at_once() {
        let state = sso_state(true);
        let first = built_runtime(&state).await;
        *state.oidc.write().await = Some(first.clone());

        // The two lines `routes::settings::put_sso` runs after a save.
        state.sso_settings_cache.clear().await;
        *state.oidc.write().await = None;

        let next = cached_or_build(&state, environment_base(&state)).await;
        assert!(
            !Arc::ptr_eq(
                next.authenticator.as_ref().unwrap(),
                first.authenticator.as_ref().unwrap()
            ),
            "a save must not leave the runtime it replaced in the cache, however fresh it is"
        );
    }
}
