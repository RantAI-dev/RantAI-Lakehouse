//! Reading a connector's credential from the installation's secret manager
//! (`SRC-12` part B, decisions 1 to 3 and 12; ADR 0002 addendum 6).
//!
//! # Shape
//!
//! One manager per installation, chosen by the operator with
//! `SECRET_MANAGER` (`vault`, `aws`, `gcp`, `azure`; unset means none) and
//! signed in to with settings from the operator's environment — never from
//! the console, because the sign-in to the manager is itself a secret.
//! [`ManagerResolver`] is built once at start-up ([`ManagerResolver::from_settings`])
//! and sits behind the same [`SecretResolver`] seam the `env:` and `file:`
//! schemes use, inside `state::ConnectorSecretResolver`, so the allowlist
//! (`state::connector_allowed_patterns`) still decides what may be named.
//!
//! # What is read, and when
//!
//! At every use: each connection test, each listing, each run start. There
//! is no cache and no fallback to an older copy — a manager that cannot be
//! reached fails that use with a fixed text (decision 1). The same goes for
//! the manager's own sign-in tokens: a token is fetched for a read and not
//! kept, because holding one would be a second copy of a credential, and a
//! read happens at most a few times a minute.
//!
//! # What is secret
//!
//! The value the manager returns, the operator's sign-in settings
//! (`VAULT_TOKEN`, `VAULT_SECRET_ID`, access keys, a service-account key,
//! `AZURE_CLIENT_SECRET`) and every token those produce. Their `Debug`
//! prints nothing of them. A reference (a path, a name, a version id) is
//! not secret and may be logged and audited. Errors are
//! [`lakehouse_core::secret::SecretFailure`]'s fixed texts; a manager's own
//! error text is classified by status and error code and then dropped
//! (principle 4, decision 12).
//!
//! # Where the requests go
//!
//! Only to addresses the operator set (`VAULT_ADDR`, the AWS endpoint, the
//! cloud providers' fixed hosts). A reference chooses a path or a name and
//! never a host: the one name that becomes part of a host (the Azure vault)
//! is validated to letters, digits and `-` in [`refs`]. Redirects are not
//! followed, so a manager (or something answering as one) cannot send the
//! sign-in token on to another server.

mod aws;
pub mod refs;
mod vault;

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use lakehouse_core::secret::{SecretError, SecretFailure, SecretResolver, SecretValue};

pub use refs::{ManagerRef, SecretManagerKind};

/// How long one request to a manager may take. Generous for a cloud API,
/// short enough that a test or a run start does not hang on a dead address.
pub(crate) const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

/// What a manager's connection settings looked like when the process
/// started: the operator's environment, read once.
///
/// `Debug` shows which manager and nothing else.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct SecretManagerSettings {
    /// The chosen manager, or `None` when `SECRET_MANAGER` is unset.
    pub kind: Option<SecretManagerKind>,
    /// The operator's environment, restricted to the variables the chosen
    /// manager reads (see [`relevant_variables`]). Held as a map so the
    /// clients read their own settings the way `Config` reads its own, and
    /// so tests pass an explicit map instead of changing the process
    /// environment.
    env: HashMap<String, String>,
}

impl fmt::Debug for SecretManagerSettings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SecretManagerSettings")
            .field("kind", &self.kind)
            .finish_non_exhaustive()
    }
}

/// Why `SECRET_MANAGER` and its settings cannot be used. The text names the
/// variable, never a value (a value may be the secret).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SettingsError {
    /// `SECRET_MANAGER` is set to something that is not a manager.
    #[error("SECRET_MANAGER must be one of vault, aws, gcp, azure (or unset)")]
    UnknownManager,
    /// A setting the chosen manager needs is missing or unusable.
    #[error("SECRET_MANAGER={manager} needs {needs}")]
    Incomplete {
        /// The chosen manager.
        manager: &'static str,
        /// What is missing, in words naming the variables.
        needs: &'static str,
    },
}

/// The environment variables the chosen manager reads. Only these are kept
/// in [`SecretManagerSettings`], so the settings of the other three
/// managers (or any unrelated secret in the environment) are not carried
/// around by this module.
fn relevant_variables(kind: SecretManagerKind) -> &'static [&'static str] {
    match kind {
        SecretManagerKind::Vault => &[
            "VAULT_ADDR",
            "VAULT_TOKEN",
            "VAULT_ROLE_ID",
            "VAULT_SECRET_ID",
        ],
        SecretManagerKind::Aws => &[
            "AWS_REGION",
            "AWS_DEFAULT_REGION",
            "AWS_ACCESS_KEY_ID",
            "AWS_SECRET_ACCESS_KEY",
            "AWS_SESSION_TOKEN",
            "AWS_ENDPOINT_URL_SECRETS_MANAGER",
            "AWS_CONTAINER_CREDENTIALS_RELATIVE_URI",
            "AWS_CONTAINER_CREDENTIALS_FULL_URI",
            "AWS_CONTAINER_AUTHORIZATION_TOKEN",
            "AWS_EC2_METADATA_SERVICE_ENDPOINT",
        ],
        SecretManagerKind::Gcp => &["GOOGLE_APPLICATION_CREDENTIALS", "GCE_METADATA_HOST"],
        SecretManagerKind::Azure => &[
            "AZURE_TENANT_ID",
            "AZURE_CLIENT_ID",
            "AZURE_CLIENT_SECRET",
            "AZURE_AUTHORITY_HOST",
            "IDENTITY_ENDPOINT",
            "IDENTITY_HEADER",
        ],
    }
}

impl SecretManagerSettings {
    /// Read the settings from an environment map. Unset or empty
    /// `SECRET_MANAGER` means no manager.
    ///
    /// # Errors
    ///
    /// [`SettingsError`] for an unknown manager name or a missing setting
    /// the chosen manager cannot do without — a start-up refusal, so a typo
    /// is seen at once instead of at the first run.
    pub fn from_map(env: &HashMap<String, String>) -> Result<Self, SettingsError> {
        let chosen = env.get("SECRET_MANAGER").map_or("", |v| v.trim());
        if chosen.is_empty() {
            return Ok(Self::default());
        }
        let kind = SecretManagerKind::from_setting(chosen).ok_or(SettingsError::UnknownManager)?;
        let kept: HashMap<String, String> = relevant_variables(kind)
            .iter()
            .filter_map(|name| {
                env.get(*name)
                    .map(|v| v.trim())
                    .filter(|v| !v.is_empty())
                    .map(|v| ((*name).to_owned(), v.to_owned()))
            })
            .collect();
        let settings = Self {
            kind: Some(kind),
            env: kept,
        };
        settings.check_complete(kind)?;
        Ok(settings)
    }

    /// One setting, if set and non-empty.
    pub(crate) fn get(&self, name: &str) -> Option<&str> {
        self.env.get(name).map(String::as_str)
    }

    fn check_complete(&self, kind: SecretManagerKind) -> Result<(), SettingsError> {
        let incomplete = |needs: &'static str| SettingsError::Incomplete {
            manager: kind.as_str(),
            needs,
        };
        match kind {
            SecretManagerKind::Vault => {
                if !self
                    .get("VAULT_ADDR")
                    .is_some_and(|addr| addr.starts_with("http://") || addr.starts_with("https://"))
                {
                    return Err(incomplete("VAULT_ADDR (an http:// or https:// address)"));
                }
                let token = self.get("VAULT_TOKEN").is_some();
                let approle =
                    self.get("VAULT_ROLE_ID").is_some() && self.get("VAULT_SECRET_ID").is_some();
                if !token && !approle {
                    return Err(incomplete(
                        "VAULT_TOKEN, or VAULT_ROLE_ID with VAULT_SECRET_ID",
                    ));
                }
            }
            SecretManagerKind::Aws => {
                if self
                    .get("AWS_REGION")
                    .or(self.get("AWS_DEFAULT_REGION"))
                    .is_none()
                {
                    return Err(incomplete("AWS_REGION"));
                }
            }
            // A service-account file or the platform's identity, and for
            // Azure an app registration or a managed identity: which of the
            // two is in use is decided per read, so nothing is required
            // here beyond a half-set app registration being refused.
            SecretManagerKind::Gcp => {}
            SecretManagerKind::Azure => {
                let set = ["AZURE_TENANT_ID", "AZURE_CLIENT_ID", "AZURE_CLIENT_SECRET"]
                    .iter()
                    .filter(|name| self.get(name).is_some())
                    .count();
                let user_assigned_identity = set == 1 && self.get("AZURE_CLIENT_ID").is_some();
                if set != 0 && set != 3 && !user_assigned_identity {
                    return Err(incomplete(
                        "AZURE_TENANT_ID, AZURE_CLIENT_ID and AZURE_CLIENT_SECRET together (or none of them)",
                    ));
                }
            }
        }
        Ok(())
    }
}

/// A client for one request to a manager: bounded in time, and never
/// following a redirect, so a sign-in token cannot be sent on to another
/// server by whatever answers at the operator's address.
pub(crate) fn http_client() -> Result<reqwest::Client, SecretFailure> {
    reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| SecretFailure::ManagerUnreachable)
}

/// A request that got no answer: the manager is unreachable. The transport
/// error is dropped (it names the address and may carry a library's text).
pub(crate) fn send_failure(_err: reqwest::Error) -> SecretFailure {
    SecretFailure::ManagerUnreachable
}

/// What a non-success status means for a read that was signed in already:
/// 401 the sign-in no longer works, 403 not allowed, 404 not there, and
/// anything else (including a redirect, 429 and 5xx) is the manager not
/// answering usefully.
pub(crate) fn read_stage_failure(status: reqwest::StatusCode) -> SecretFailure {
    match status.as_u16() {
        401 => SecretFailure::ManagerSignIn,
        403 => SecretFailure::ManagerForbidden,
        404 => SecretFailure::ManagerNotFound,
        _ => SecretFailure::ManagerUnreachable,
    }
}

/// One manager's way of reading a reference. Returns the secret text or the
/// fixed reason it could not.
#[async_trait::async_trait]
pub(crate) trait ManagerBackend: fmt::Debug + Send + Sync {
    /// Read the secret `secret_ref` names. `secret_ref` is already parsed
    /// for this backend's manager.
    async fn read(&self, secret_ref: &ManagerRef) -> Result<SecretValue, SecretFailure>;
}

/// Resolves the references of the installation's one secret manager.
///
/// Built once at start-up; it holds the settings and nothing the manager
/// returned.
#[derive(Debug, Clone)]
#[allow(
    dead_code,
    reason = "built by the per-manager clients of B2 to B5; until then only tests construct it"
)]
pub struct ManagerResolver {
    kind: SecretManagerKind,
    backend: Arc<dyn ManagerBackend>,
}

impl ManagerResolver {
    /// The resolver for `settings`, or `None` when no manager is set (or,
    /// until its client lands, when the chosen manager has none: a
    /// reference then fails as unsupported at use, never as a value).
    ///
    /// Infallible on purpose: every setting is checked by
    /// [`SecretManagerSettings::from_map`] when the configuration is read,
    /// and anything that can still be wrong at use (an unreadable
    /// service-account file, an unreachable address) is a per-read
    /// [`SecretFailure`], so a manager outage never keeps the API from
    /// starting.
    #[must_use]
    pub fn from_settings(settings: &SecretManagerSettings) -> Option<Self> {
        match settings.kind? {
            SecretManagerKind::Vault => {
                let backend = vault::VaultBackend::from_settings(settings)?;
                Some(Self {
                    kind: SecretManagerKind::Vault,
                    backend: Arc::new(backend),
                })
            }
            SecretManagerKind::Aws => {
                let backend = aws::AwsBackend::from_settings(settings);
                Some(Self {
                    kind: SecretManagerKind::Aws,
                    backend: Arc::new(backend),
                })
            }
            SecretManagerKind::Gcp | SecretManagerKind::Azure => None,
        }
    }

    /// A resolver over a given backend, for tests.
    #[cfg(test)]
    pub(crate) fn with_backend(kind: SecretManagerKind, backend: Arc<dyn ManagerBackend>) -> Self {
        Self { kind, backend }
    }

    /// The manager this resolver reads.
    #[must_use]
    #[allow(dead_code, reason = "used by AppState once a client exists (B2 to B5)")]
    pub fn kind(&self) -> SecretManagerKind {
        self.kind
    }
}

impl SecretResolver for ManagerResolver {
    async fn resolve(&self, secret_ref: &str) -> Result<SecretValue, SecretError> {
        let parsed =
            refs::parse(self.kind, secret_ref).map_err(|_| SecretError::UnsupportedRef {
                secret_ref: secret_ref.to_owned(),
                resolver: "secret-manager",
            })?;
        self.backend
            .read(&parsed)
            .await
            .map_err(|failure| {
                // The reference is a name; the manager's own words were
                // dropped by the backend before this point.
                tracing::warn!(secret_ref, %failure, "secret manager read failed");
                SecretError::Failed {
                    secret_ref: secret_ref.to_owned(),
                    failure,
                }
            })
            .map(|value| SecretValue::new(value.expose_secret().trim().to_owned()))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn env(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    #[test]
    fn unset_or_empty_means_no_manager() {
        assert_eq!(
            SecretManagerSettings::from_map(&env(&[])).unwrap().kind,
            None
        );
        assert_eq!(
            SecretManagerSettings::from_map(&env(&[("SECRET_MANAGER", " ")]))
                .unwrap()
                .kind,
            None
        );
    }

    #[test]
    fn an_unknown_manager_is_refused_at_start_up() {
        assert_eq!(
            SecretManagerSettings::from_map(&env(&[("SECRET_MANAGER", "Vault")])).unwrap_err(),
            SettingsError::UnknownManager
        );
        assert_eq!(
            SecretManagerSettings::from_map(&env(&[("SECRET_MANAGER", "consul")])).unwrap_err(),
            SettingsError::UnknownManager
        );
    }

    #[test]
    fn each_manager_names_what_it_cannot_do_without() {
        let missing = |pairs: &[(&str, &str)]| {
            SecretManagerSettings::from_map(&env(pairs))
                .unwrap_err()
                .to_string()
        };
        assert!(missing(&[("SECRET_MANAGER", "vault")]).contains("VAULT_ADDR"));
        assert!(
            missing(&[("SECRET_MANAGER", "vault"), ("VAULT_ADDR", "http://v:8200")])
                .contains("VAULT_TOKEN")
        );
        assert!(missing(&[("SECRET_MANAGER", "aws")]).contains("AWS_REGION"));
        assert!(
            missing(&[("SECRET_MANAGER", "azure"), ("AZURE_TENANT_ID", "t")]).contains("together")
        );
    }

    #[test]
    fn the_complete_settings_of_each_manager_are_accepted_and_only_its_variables_kept() {
        let vault = SecretManagerSettings::from_map(&env(&[
            ("SECRET_MANAGER", "vault"),
            ("VAULT_ADDR", "http://v:8200"),
            ("VAULT_TOKEN", "t"),
            ("AWS_SECRET_ACCESS_KEY", "other-managers-secret"),
        ]))
        .unwrap();
        assert_eq!(vault.kind, Some(SecretManagerKind::Vault));
        assert_eq!(vault.get("VAULT_ADDR"), Some("http://v:8200"));
        assert_eq!(vault.get("AWS_SECRET_ACCESS_KEY"), None);
        assert!(
            SecretManagerSettings::from_map(&env(&[
                ("SECRET_MANAGER", "vault"),
                ("VAULT_ADDR", "http://v:8200"),
                ("VAULT_ROLE_ID", "r"),
                ("VAULT_SECRET_ID", "s"),
            ]))
            .is_ok()
        );
        assert!(
            SecretManagerSettings::from_map(&env(&[
                ("SECRET_MANAGER", "aws"),
                ("AWS_DEFAULT_REGION", "eu-west-1"),
            ]))
            .is_ok()
        );
        assert!(SecretManagerSettings::from_map(&env(&[("SECRET_MANAGER", "gcp")])).is_ok());
        assert!(SecretManagerSettings::from_map(&env(&[("SECRET_MANAGER", "azure")])).is_ok());
    }

    #[test]
    fn debug_never_prints_a_setting() {
        let settings = SecretManagerSettings::from_map(&env(&[
            ("SECRET_MANAGER", "vault"),
            ("VAULT_ADDR", "http://v:8200"),
            ("VAULT_TOKEN", "s.very-secret"),
        ]))
        .unwrap();
        let shown = format!("{settings:?}");
        assert!(!shown.contains("very-secret"));
        assert!(!shown.contains("v:8200"));
    }

    /// A backend that answers one canned result and counts nothing else.
    #[derive(Debug)]
    struct Canned(Result<&'static str, SecretFailure>);

    #[async_trait::async_trait]
    impl ManagerBackend for Canned {
        async fn read(&self, _secret_ref: &ManagerRef) -> Result<SecretValue, SecretFailure> {
            self.0.map(SecretValue::new)
        }
    }

    #[tokio::test]
    async fn the_resolver_reads_its_own_scheme_and_trims_the_value() {
        let resolver =
            ManagerResolver::with_backend(SecretManagerKind::Vault, Arc::new(Canned(Ok(" pw \n"))));
        let value = resolver.resolve("vault:secret/a#k").await.unwrap();
        assert_eq!(value.expose_secret(), "pw");
    }

    #[tokio::test]
    async fn another_schemes_or_a_malformed_reference_is_unsupported_and_reads_nothing() {
        let resolver = ManagerResolver::with_backend(
            SecretManagerKind::Vault,
            Arc::new(Canned(Ok("never-read"))),
        );
        for secret_ref in ["aws-sm:a", "vault:secret/a", "env:CONNECTOR_X_PASSWORD"] {
            assert!(
                matches!(
                    resolver.resolve(secret_ref).await,
                    Err(SecretError::UnsupportedRef { .. })
                ),
                "{secret_ref}"
            );
        }
    }

    /// Decision 12: each failure is one fixed text, and the text carries
    /// neither the reference nor anything a manager said.
    #[tokio::test]
    async fn every_failure_is_its_fixed_text_from_the_shared_list() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/secret_manager_refs.json"
        ))
        .unwrap();
        let texts = &fixture["texts"];
        let cases = [
            (SecretFailure::WrongKey, "wrong_key"),
            (SecretFailure::KeyUnavailable, "key_unavailable"),
            (SecretFailure::UnreadableFile, "unreadable_file"),
            (SecretFailure::ManagerUnreachable, "manager_unreachable"),
            (SecretFailure::ManagerSignIn, "manager_sign_in"),
            (SecretFailure::ManagerNotFound, "manager_not_found"),
            (SecretFailure::ManagerForbidden, "manager_forbidden"),
            (SecretFailure::NotSingleText, "not_single_text"),
        ];
        assert_eq!(texts.as_object().unwrap().len(), cases.len());
        for (failure, name) in cases {
            assert_eq!(failure.to_string(), texts[name].as_str().unwrap(), "{name}");
            let resolver = ManagerResolver::with_backend(
                SecretManagerKind::Vault,
                Arc::new(Canned(Err(failure))),
            );
            let err = resolver.resolve("vault:secret/a#k").await.unwrap_err();
            assert_eq!(err.to_string(), failure.to_string());
            assert!(!err.to_string().contains("secret/a"));
        }
    }
}
