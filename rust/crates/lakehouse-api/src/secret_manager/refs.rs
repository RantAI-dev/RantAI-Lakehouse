//! The four reference shapes a secret manager credential is stored as
//! (`SRC-12` part B, decision 1), and the one grammar that admits them.
//!
//! A reference names a place in a secret manager, never a value: a Vault
//! path, an AWS secret name, a Google resource name, an Azure vault and
//! secret name. None of those is secret (a version id is not either), so a
//! reference may be logged and audited; what the manager holds at that
//! place is the secret and never leaves [`lakehouse_core::secret::SecretValue`].
//!
//! The grammar is strict on purpose. A reference is chosen by a
//! `connector:manage` principal, and two of the shapes become part of a URL
//! the API or the orchestrator dials with its own sign-in (the Azure vault
//! name is a host name, the Vault path is a URL path): the character sets
//! below keep a reference from steering that request anywhere but to the
//! one secret it names (no `..`, no `?`, no `@`, no `/` where a name is
//! expected). `dagster/dispar_orchestrate/secret_manager.py` ports the same
//! grammar; `tests/fixtures/secret_manager_refs.json` is read by both test
//! suites, so the two cannot disagree on a reference.

/// Which secret manager an installation uses (`SECRET_MANAGER`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretManagerKind {
    /// `HashiCorp` Vault, KV version 2.
    Vault,
    /// AWS Secrets Manager.
    Aws,
    /// Google Secret Manager.
    Gcp,
    /// Azure Key Vault.
    Azure,
}

impl SecretManagerKind {
    /// Every kind, for tests that must cover them all.
    pub const ALL: [Self; 4] = [Self::Vault, Self::Aws, Self::Gcp, Self::Azure];

    /// The value `SECRET_MANAGER` takes, which is also what the console is
    /// told ("Secret manager: vault").
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Vault => "vault",
            Self::Aws => "aws",
            Self::Gcp => "gcp",
            Self::Azure => "azure",
        }
    }

    /// Parse the `SECRET_MANAGER` value (exact, lower-case).
    #[must_use]
    pub fn from_setting(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.as_str() == value)
    }

    /// The scheme a reference of this manager starts with, colon included.
    #[must_use]
    pub fn scheme(self) -> &'static str {
        match self {
            Self::Vault => "vault:",
            Self::Aws => "aws-sm:",
            Self::Gcp => "gcp-sm:",
            Self::Azure => "azure-kv:",
        }
    }

    /// The one allowlist pattern (see
    /// [`lakehouse_core::secret::pattern_matches`]) that admits this
    /// manager's references. The grammar below, not the pattern, decides
    /// which strings under the scheme are references.
    #[must_use]
    pub fn allowed_pattern(self) -> String {
        format!("{}*", self.scheme())
    }

    /// The manager a reference's scheme belongs to, if any. A pure
    /// predicate over the name.
    #[must_use]
    pub fn of_ref(secret_ref: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|kind| secret_ref.starts_with(kind.scheme()))
    }
}

/// A parsed reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManagerRef {
    /// `vault:<mount>/<path>#<key>`.
    Vault {
        /// The KV v2 mount (first segment).
        mount: String,
        /// The secret's path under the mount.
        path: String,
        /// The field inside the secret.
        key: String,
    },
    /// `aws-sm:<secret id>[#<key>]`: a name or an ARN.
    Aws {
        /// Name or ARN.
        secret_id: String,
        /// The JSON field to take, or the whole text when `None`.
        key: Option<String>,
    },
    /// `gcp-sm:projects/<project>/secrets/<name>/versions/<version|latest>`.
    Gcp {
        /// The project id.
        project: String,
        /// The secret's name.
        secret: String,
        /// A version number or `latest`.
        version: String,
    },
    /// `azure-kv:<vault name>/<secret name>[/<version>]`.
    Azure {
        /// The vault's name (the first label of its host).
        vault: String,
        /// The secret's name.
        secret: String,
        /// A version id, or the current one when `None`.
        version: Option<String>,
    },
}

/// A reference that does not fit its manager's grammar. Carries nothing
/// (the reference is the caller's to name), so it can be shown as it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("not a valid secret manager reference")]
pub struct RefError;

const MAX_REF_LEN: usize = 1024;

fn is_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-')
}

/// A path segment or a key: non-empty, name characters only, and not `.`
/// or `..`.
fn is_segment(s: &str) -> bool {
    !s.is_empty() && s != "." && s != ".." && s.chars().all(is_name_char)
}

fn is_key(s: &str) -> bool {
    is_segment(s) && s.len() <= 128
}

/// Parse `secret_ref` as a reference of `kind`.
///
/// # Errors
///
/// [`RefError`] when the scheme is not `kind`'s or the rest does not fit
/// its grammar.
pub fn parse(kind: SecretManagerKind, secret_ref: &str) -> Result<ManagerRef, RefError> {
    if secret_ref.len() > MAX_REF_LEN {
        return Err(RefError);
    }
    let rest = secret_ref.strip_prefix(kind.scheme()).ok_or(RefError)?;
    match kind {
        SecretManagerKind::Vault => parse_vault(rest),
        SecretManagerKind::Aws => parse_aws(rest),
        SecretManagerKind::Gcp => parse_gcp(rest),
        SecretManagerKind::Azure => parse_azure(rest),
    }
}

fn parse_vault(rest: &str) -> Result<ManagerRef, RefError> {
    let (location, key) = rest.split_once('#').ok_or(RefError)?;
    if !is_key(key) {
        return Err(RefError);
    }
    let (mount, path) = location.split_once('/').ok_or(RefError)?;
    if !is_segment(mount) || !path.split('/').all(is_segment) {
        return Err(RefError);
    }
    Ok(ManagerRef::Vault {
        mount: mount.to_owned(),
        path: path.to_owned(),
        key: key.to_owned(),
    })
}

fn parse_aws(rest: &str) -> Result<ManagerRef, RefError> {
    let (id, key) = match rest.split_once('#') {
        Some((id, key)) => (id, Some(key)),
        None => (rest, None),
    };
    // Secret names allow `/_+=.@-`; an ARN adds `:`.
    let id_ok = !id.is_empty()
        && id.len() <= 2048
        && id.chars().all(|c| {
            c.is_ascii_alphanumeric() || matches!(c, '/' | '_' | '+' | '=' | '.' | '@' | '-' | ':')
        });
    if !id_ok || key.is_some_and(|k| !is_key(k)) {
        return Err(RefError);
    }
    Ok(ManagerRef::Aws {
        secret_id: id.to_owned(),
        key: key.map(str::to_owned),
    })
}

fn parse_gcp(rest: &str) -> Result<ManagerRef, RefError> {
    let parts: Vec<&str> = rest.split('/').collect();
    let ["projects", project, "secrets", secret, "versions", version] = parts.as_slice() else {
        return Err(RefError);
    };
    let project_ok = !project.is_empty()
        && project.len() <= 64
        && project
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | ':' | '-'));
    let secret_ok = !secret.is_empty()
        && secret.len() <= 255
        && secret
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'));
    let version_ok = *version == "latest"
        || (!version.is_empty()
            && version.len() <= 20
            && version.chars().all(|c| c.is_ascii_digit()));
    if !(project_ok && secret_ok && version_ok) {
        return Err(RefError);
    }
    Ok(ManagerRef::Gcp {
        project: (*project).to_owned(),
        secret: (*secret).to_owned(),
        version: (*version).to_owned(),
    })
}

fn parse_azure(rest: &str) -> Result<ManagerRef, RefError> {
    let parts: Vec<&str> = rest.split('/').collect();
    let (vault, secret, version) = match parts.as_slice() {
        [vault, secret] => (*vault, *secret, None),
        [vault, secret, version] => (*vault, *secret, Some(*version)),
        _ => return Err(RefError),
    };
    // The vault name becomes `https://<vault>.vault.azure.net`: letters,
    // digits and `-` only, 3 to 24 long, so it cannot carry a port, a
    // userinfo or a second host.
    let dash_alnum = |s: &str| s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    let vault_ok = (3..=24).contains(&vault.len()) && dash_alnum(vault);
    let secret_ok = (1..=127).contains(&secret.len()) && dash_alnum(secret);
    let version_ok = version.is_none_or(|v| {
        (1..=64).contains(&v.len()) && v.chars().all(|c| c.is_ascii_alphanumeric())
    });
    if !(vault_ok && secret_ok && version_ok) {
        return Err(RefError);
    }
    Ok(ManagerRef::Azure {
        vault: vault.to_owned(),
        secret: secret.to_owned(),
        version: version.map(str::to_owned),
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use serde_json::Value;

    use super::*;

    fn fixture() -> Value {
        serde_json::from_str(include_str!(
            "../../tests/fixtures/secret_manager_refs.json"
        ))
        .unwrap()
    }

    fn kind_named(name: &str) -> SecretManagerKind {
        SecretManagerKind::from_setting(name).unwrap()
    }

    fn text(value: &Value) -> Option<String> {
        value.as_str().map(str::to_owned)
    }

    /// The parts the shared list names, in the shape [`ManagerRef`] holds.
    fn parts_of(parsed: &ManagerRef) -> Value {
        match parsed {
            ManagerRef::Vault { mount, path, key } => {
                serde_json::json!({"mount": mount, "path": path, "key": key})
            }
            ManagerRef::Aws { secret_id, key } => {
                serde_json::json!({"secret_id": secret_id, "key": key})
            }
            ManagerRef::Gcp {
                project,
                secret,
                version,
            } => serde_json::json!({"project": project, "secret": secret, "version": version}),
            ManagerRef::Azure {
                vault,
                secret,
                version,
            } => serde_json::json!({"vault": vault, "secret": secret, "version": version}),
        }
    }

    #[test]
    fn every_valid_reference_of_the_shared_list_parses_to_the_listed_parts() {
        let fixture = fixture();
        let valid = fixture["valid"].as_array().unwrap();
        assert!(valid.len() >= 8);
        for case in valid {
            let secret_ref = text(&case["ref"]).unwrap();
            let kind = kind_named(case["manager"].as_str().unwrap());
            let parsed = parse(kind, &secret_ref).unwrap_or_else(|_| panic!("{secret_ref}"));
            assert_eq!(parts_of(&parsed), case["parts"], "{secret_ref}");
            assert_eq!(SecretManagerKind::of_ref(&secret_ref), Some(kind));
        }
    }

    #[test]
    fn every_refused_reference_of_the_shared_list_is_refused() {
        let fixture = fixture();
        for case in fixture["refused"].as_array().unwrap() {
            let secret_ref = text(&case["ref"]).unwrap();
            let kind = kind_named(case["manager"].as_str().unwrap());
            assert_eq!(parse(kind, &secret_ref), Err(RefError), "{secret_ref}");
        }
    }

    #[test]
    fn a_reference_of_another_managers_scheme_is_refused() {
        assert_eq!(
            parse(SecretManagerKind::Aws, "vault:secret/a#k"),
            Err(RefError)
        );
        assert_eq!(
            parse(SecretManagerKind::Vault, "aws-sm:prod/orders"),
            Err(RefError)
        );
        assert_eq!(
            parse(
                SecretManagerKind::Vault,
                &format!("vault:secret/{}#k", "a".repeat(2000))
            ),
            Err(RefError)
        );
    }

    #[test]
    fn the_allowlist_pattern_is_the_scheme_and_a_star() {
        for kind in SecretManagerKind::ALL {
            let pattern = kind.allowed_pattern();
            assert!(lakehouse_core::secret::pattern_matches(
                &pattern,
                &format!("{}x", kind.scheme())
            ));
            assert_eq!(SecretManagerKind::from_setting(kind.as_str()), Some(kind));
        }
        assert_eq!(SecretManagerKind::from_setting("Vault"), None);
        assert_eq!(SecretManagerKind::from_setting(""), None);
    }
}
