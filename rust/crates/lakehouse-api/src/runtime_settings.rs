//! Effective deployment settings, resolved in exactly one place and read
//! through a short cache (`ADM-6`/`ADM-7`, slice 1 of
//! `docs/superpowers/plans/2026-11-06-adm-6-9-console-settings.md`).
//!
//! # One merge rule
//!
//! For each field the effective value is the environment's when it is set
//! and non-empty, else the console's saved value, else the code default
//! (plan D1). That precedence is implemented ONLY in [`resolve_email`].
//! The pre-`ADM-7` merge (`routes::alerts::smtp_config`, six fields read
//! straight from [`Config`]) is gone: every sender builds its
//! [`SmtpConfig`] from this resolver, and every "is email set up" check
//! reads this resolver's answer, so there is no second place where
//! environment and saved values are combined (rule 4: no duplicated
//! guard).
//!
//! # The cache
//!
//! [`EmailSettingsCache`] holds the saved row for at most
//! [`EMAIL_CACHE_TTL`] per process (a `tokio::sync::RwLock` with the read
//! time). The email `PUT` (the console's email screen) drops the cached row
//! at once, so a save on this process is visible to the next send without
//! waiting for the TTL. Other processes re-read within the TTL; pushing a
//! change across processes faster than 30 s is out of scope (plan §3).
//!
//! # Secrets
//!
//! The resolved password is held as a [`SecretValue`] (redacting `Debug`, no
//! `Serialize`). It never appears in a log, an error or a response; a GET
//! answers `set` only, and the audit row records only the field NAME. The
//! saved password is sealed with the installation keyring under
//! [`EMAIL_PASSWORD_SEAL_NAME`]; a row that does not open (the key changed)
//! is read as "not set" and logged, never returned.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use lakehouse_core::secret::SecretValue;
use lakehouse_notify::{SmtpCaBundle, SmtpConfig};
use lakehouse_store::email_settings::EmailSettings;
use lakehouse_store::{PgPool, StoreError};
use tokio::sync::{Mutex, RwLock};
use uuid::Uuid;

use crate::config::Config;
use crate::secret_crypto::Keyring;
use crate::state::AppState;

/// How long one read of the saved email row is reused before the store is
/// asked again (plan D3).
pub const EMAIL_CACHE_TTL: Duration = Duration::from_secs(30);

/// The associated-data name the email password is sealed under (plan D2).
pub const EMAIL_PASSWORD_SEAL_NAME: &str = "settings/email/password";

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

/// The saved email row, cached for [`EMAIL_CACHE_TTL`] (plan D3). Cheap to
/// clone: one `Arc`-shared [`RwLock`].
#[derive(Debug)]
pub struct EmailSettingsCache {
    inner: RwLock<Option<(Instant, Option<EmailSettings>)>>,
}

impl EmailSettingsCache {
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
    pub async fn row(&self, pool: &PgPool) -> Result<Option<EmailSettings>, StoreError> {
        if let Some(row) = self.fresh_row().await {
            return Ok(row);
        }
        let mut guard = self.inner.write().await;
        if let Some((at, row)) = guard.as_ref()
            && at.elapsed() < EMAIL_CACHE_TTL
        {
            return Ok(row.clone());
        }
        let row = lakehouse_store::email_settings::get(pool).await?;
        *guard = Some((Instant::now(), row.clone()));
        Ok(row)
    }

    /// The cached row when it is still within [`EMAIL_CACHE_TTL`].
    async fn fresh_row(&self) -> Option<Option<EmailSettings>> {
        let guard = self.inner.read().await;
        let (at, row) = guard.as_ref()?;
        (at.elapsed() < EMAIL_CACHE_TTL).then(|| row.clone())
    }

    /// Drop the cached row, so the next read asks the store again. The email
    /// `PUT` calls this on the process that saved, so a change is visible to
    /// the next send there without waiting for the TTL (plan D3).
    pub async fn clear(&self) {
        *self.inner.write().await = None;
    }
}

impl Default for EmailSettingsCache {
    fn default() -> Self {
        Self::new()
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
    let (host, host_source) = if config.smtp_host.is_some() {
        (config.smtp_host.clone(), Source::Server)
    } else if let Some(host) = stored.and_then(|s| s.smtp_host.as_deref()) {
        (Some(host.to_owned()), Source::Console)
    } else {
        (None, Source::Default)
    };

    let (port, port_source) = if config.smtp_port_from_env {
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
}
