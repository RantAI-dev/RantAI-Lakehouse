//! `HashiCorp` Vault, KV version 2 (`SRC-12` part B2): `GET
//! {VAULT_ADDR}/v1/{mount}/data/{path}` with the operator's token, or with
//! one from an `AppRole` sign-in (`VAULT_ROLE_ID` and `VAULT_SECRET_ID`).
//!
//! A small client over `reqwest` instead of a Vault SDK: the interface used
//! is two JSON calls. The token (given or from `AppRole`) is used for one
//! read and dropped; nothing here is cached, so a rotated token or a
//! revoked lease takes effect at the next read.
//!
//! Vault answers 403 both for a token it does not accept and for a policy
//! that does not allow the path; it cannot be told apart from here, so a
//! 403 on a read is "the product may not read it" and a refused `AppRole`
//! sign-in is "may not sign in". A missing key inside an existing secret is
//! "does not exist" like a missing path.

use lakehouse_core::secret::{SecretFailure, SecretValue};
use serde_json::{Value, json};

use super::{
    ManagerBackend, ManagerRef, SecretManagerSettings, http_client, read_stage_failure,
    send_failure,
};

/// Where Vault is and how the product signs in to it.
pub(crate) struct VaultBackend {
    addr: String,
    token: Option<SecretValue>,
    approle: Option<(SecretValue, SecretValue)>,
}

impl std::fmt::Debug for VaultBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VaultBackend").finish_non_exhaustive()
    }
}

impl VaultBackend {
    /// From the operator's settings (checked complete at start-up).
    pub(crate) fn from_settings(settings: &SecretManagerSettings) -> Option<Self> {
        let addr = settings.get("VAULT_ADDR")?.trim_end_matches('/').to_owned();
        let approle = match (
            settings.get("VAULT_ROLE_ID"),
            settings.get("VAULT_SECRET_ID"),
        ) {
            (Some(role), Some(secret)) => Some((SecretValue::new(role), SecretValue::new(secret))),
            _ => None,
        };
        Some(Self {
            addr,
            token: settings.get("VAULT_TOKEN").map(SecretValue::new),
            approle,
        })
    }

    /// The token for this read: the given one, else a fresh `AppRole` login.
    async fn token(&self, client: &reqwest::Client) -> Result<SecretValue, SecretFailure> {
        if let Some(token) = &self.token {
            return Ok(token.clone());
        }
        let Some((role_id, secret_id)) = &self.approle else {
            return Err(SecretFailure::ManagerSignIn);
        };
        let response = client
            .post(format!("{}/v1/auth/approle/login", self.addr))
            .json(&json!({
                "role_id": role_id.expose_secret(),
                "secret_id": secret_id.expose_secret(),
            }))
            .send()
            .await
            .map_err(send_failure)?;
        let status = response.status();
        if !status.is_success() {
            return Err(if matches!(status.as_u16(), 400 | 401 | 403) {
                SecretFailure::ManagerSignIn
            } else {
                SecretFailure::ManagerUnreachable
            });
        }
        let body: Value = response
            .json()
            .await
            .map_err(|_| SecretFailure::ManagerUnreachable)?;
        body.pointer("/auth/client_token")
            .and_then(Value::as_str)
            .filter(|token| !token.is_empty())
            .map(SecretValue::new)
            .ok_or(SecretFailure::ManagerSignIn)
    }
}

#[async_trait::async_trait]
impl ManagerBackend for VaultBackend {
    async fn read(&self, secret_ref: &ManagerRef) -> Result<SecretValue, SecretFailure> {
        let ManagerRef::Vault { mount, path, key } = secret_ref else {
            return Err(SecretFailure::ManagerNotFound);
        };
        let client = http_client()?;
        let token = self.token(&client).await?;
        // `mount` and `path` are name segments (`refs::parse_vault`): no
        // `..`, `?`, `#` or empty segment, so this stays under `/v1/`.
        let response = client
            .get(format!("{}/v1/{mount}/data/{path}", self.addr))
            .header("X-Vault-Token", token.expose_secret())
            .send()
            .await
            .map_err(send_failure)?;
        let status = response.status();
        if !status.is_success() {
            return Err(read_stage_failure(status));
        }
        let body: Value = response
            .json()
            .await
            .map_err(|_| SecretFailure::ManagerUnreachable)?;
        let fields = body
            .pointer("/data/data")
            .and_then(Value::as_object)
            .ok_or(SecretFailure::ManagerNotFound)?;
        match fields.get(key) {
            None => Err(SecretFailure::ManagerNotFound),
            Some(Value::String(text)) => Ok(SecretValue::new(text.clone())),
            Some(_) => Err(SecretFailure::NotSingleText),
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::collections::HashMap;

    use wiremock::matchers::{body_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::secret_manager::SecretManagerKind;

    fn backend(server: &MockServer, extra: &[(&str, &str)]) -> VaultBackend {
        let mut env: HashMap<String, String> = [
            ("SECRET_MANAGER", "vault"),
            ("VAULT_ADDR", server.uri().as_str()),
        ]
        .iter()
        .chain(extra)
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect();
        env.insert("SECRET_MANAGER".to_owned(), "vault".to_owned());
        let settings = SecretManagerSettings::from_map(&env).unwrap();
        VaultBackend::from_settings(&settings).unwrap()
    }

    fn reference() -> ManagerRef {
        crate::secret_manager::refs::parse(
            SecretManagerKind::Vault,
            "vault:secret/connectors/orders#password",
        )
        .unwrap()
    }

    fn kv2(data: &Value) -> ResponseTemplate {
        ResponseTemplate::new(200).set_body_json(json!({
            "data": {"data": data, "metadata": {"version": 3}}
        }))
    }

    #[tokio::test]
    async fn reads_the_key_with_the_given_token() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/secret/data/connectors/orders"))
            .and(header("X-Vault-Token", "tok-1"))
            .respond_with(kv2(&json!({"password": "pw-from-vault", "user": "u"})))
            .expect(1)
            .mount(&server)
            .await;
        let value = backend(&server, &[("VAULT_TOKEN", "tok-1")])
            .read(&reference())
            .await
            .unwrap();
        assert_eq!(value.expose_secret(), "pw-from-vault");
    }

    /// Decision 1: read at every use, no copy. The second read sees the
    /// changed secret.
    #[tokio::test]
    async fn every_read_goes_to_vault_and_a_changed_secret_is_seen_at_once() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(kv2(&json!({"password": "old"})))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .respond_with(kv2(&json!({"password": "new"})))
            .mount(&server)
            .await;
        let backend = backend(&server, &[("VAULT_TOKEN", "t")]);
        assert_eq!(
            backend.read(&reference()).await.unwrap().expose_secret(),
            "old"
        );
        assert_eq!(
            backend.read(&reference()).await.unwrap().expose_secret(),
            "new"
        );
    }

    #[tokio::test]
    async fn signs_in_with_approle_for_each_read_and_uses_the_token_it_gets() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/auth/approle/login"))
            .and(body_json(
                json!({"role_id": "role-1", "secret_id": "sid-1"}),
            ))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"auth": {"client_token": "s.issued"}})),
            )
            .expect(2)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(header("X-Vault-Token", "s.issued"))
            .respond_with(kv2(&json!({"password": "pw"})))
            .expect(2)
            .mount(&server)
            .await;
        let backend = backend(
            &server,
            &[("VAULT_ROLE_ID", "role-1"), ("VAULT_SECRET_ID", "sid-1")],
        );
        backend.read(&reference()).await.unwrap();
        backend.read(&reference()).await.unwrap();
    }

    #[tokio::test]
    async fn a_refused_approle_sign_in_is_may_not_sign_in() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(400)
                    .set_body_json(json!({"errors": ["invalid secret id s3cr3t-detail"]})),
            )
            .mount(&server)
            .await;
        let err = backend(&server, &[("VAULT_ROLE_ID", "r"), ("VAULT_SECRET_ID", "s")])
            .read(&reference())
            .await
            .unwrap_err();
        assert_eq!(err, SecretFailure::ManagerSignIn);
    }

    #[tokio::test]
    async fn each_failure_is_classified_by_status_and_vaults_text_is_dropped() {
        for (status, expected) in [
            (404, SecretFailure::ManagerNotFound),
            (403, SecretFailure::ManagerForbidden),
            (401, SecretFailure::ManagerSignIn),
            (500, SecretFailure::ManagerUnreachable),
            (503, SecretFailure::ManagerUnreachable),
        ] {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .respond_with(
                    ResponseTemplate::new(status)
                        .set_body_json(json!({"errors": ["vault-own-words"]})),
                )
                .mount(&server)
                .await;
            let err = backend(&server, &[("VAULT_TOKEN", "t")])
                .read(&reference())
                .await
                .unwrap_err();
            assert_eq!(err, expected, "{status}");
            assert!(!err.to_string().contains("vault-own-words"));
        }
    }

    #[tokio::test]
    async fn a_missing_key_does_not_exist_and_a_non_text_value_is_not_a_single_text() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(kv2(&json!({"other": "x"})))
            .mount(&server)
            .await;
        assert_eq!(
            backend(&server, &[("VAULT_TOKEN", "t")])
                .read(&reference())
                .await
                .unwrap_err(),
            SecretFailure::ManagerNotFound
        );
        for not_text in [
            json!(7),
            json!({"a": 1}),
            json!(["x"]),
            json!(null),
            json!(true),
        ] {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .respond_with(kv2(&json!({"password": not_text})))
                .mount(&server)
                .await;
            assert_eq!(
                backend(&server, &[("VAULT_TOKEN", "t")])
                    .read(&reference())
                    .await
                    .unwrap_err(),
                SecretFailure::NotSingleText
            );
        }
    }

    #[tokio::test]
    async fn a_redirect_is_not_followed_so_the_token_goes_nowhere_else() {
        let server = MockServer::start().await;
        let elsewhere = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(307).insert_header("location", elsewhere.uri().as_str()),
            )
            .mount(&server)
            .await;
        let err = backend(&server, &[("VAULT_TOKEN", "t")])
            .read(&reference())
            .await
            .unwrap_err();
        assert_eq!(err, SecretFailure::ManagerUnreachable);
        assert!(elsewhere.received_requests().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn an_address_nobody_answers_is_unreachable_and_the_text_shows_no_address() {
        let server = MockServer::start().await;
        let backend = backend(&server, &[("VAULT_TOKEN", "t")]);
        drop(server);
        let err = backend.read(&reference()).await.unwrap_err();
        assert_eq!(err, SecretFailure::ManagerUnreachable);
        assert!(!err.to_string().contains("127.0.0.1"));
    }

    #[test]
    fn debug_shows_neither_address_nor_token() {
        let env: HashMap<String, String> = [
            ("SECRET_MANAGER", "vault"),
            ("VAULT_ADDR", "http://vault.internal:8200"),
            ("VAULT_TOKEN", "s.very-secret"),
        ]
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect();
        let settings = SecretManagerSettings::from_map(&env).unwrap();
        let shown = format!("{:?}", VaultBackend::from_settings(&settings).unwrap());
        assert!(!shown.contains("very-secret") && !shown.contains("vault.internal"));
    }
}
