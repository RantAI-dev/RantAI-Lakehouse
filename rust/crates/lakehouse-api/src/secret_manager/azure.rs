//! Azure Key Vault (`SRC-12` part B5): `GET
//! https://{vault}.vault.azure.net/secrets/{name}[/{version}]?api-version=7.4`
//! with an Entra ID access token for `https://vault.azure.net`, fetched at
//! every read.
//!
//! The token comes from an app registration (`AZURE_TENANT_ID`,
//! `AZURE_CLIENT_ID`, `AZURE_CLIENT_SECRET`: the client-credentials grant at
//! `AZURE_AUTHORITY_HOST`, default `https://login.microsoftonline.com`) or
//! from a managed identity: the App Service / Functions endpoint
//! (`IDENTITY_ENDPOINT` with `IDENTITY_HEADER`) or the instance metadata
//! service, with `AZURE_CLIENT_ID` alone selecting a user-assigned identity.
//! Two small HTTP calls instead of `azure_identity` and
//! `azure_security_keyvault_secrets`. The vault name becomes a host name, so
//! the reference grammar allows only letters, digits and `-` (3 to 24 long)
//! and the host can only be `<that>.vault.azure.net`. Only the public
//! cloud's `vault.azure.net` is built (a limit).
//!
//! Tested against recorded responses of the documented interface; *not run
//! against the real service* (decision 3). `secret_manager_azure.py` is the
//! twin.

use std::time::Duration;

use lakehouse_core::secret::{SecretFailure, SecretValue};
use serde_json::Value;

use super::{
    ManagerBackend, ManagerRef, SecretManagerSettings, http_client, read_stage_failure,
    send_failure,
};

const RESOURCE: &str = "https://vault.azure.net";
const DEFAULT_AUTHORITY: &str = "https://login.microsoftonline.com";
const IMDS: &str = "http://169.254.169.254";
const API_VERSION: &str = "7.4";
/// A host with no identity service must fail fast.
const METADATA_TIMEOUT: Duration = Duration::from_secs(2);

/// Where Key Vault is, and how the product signs in.
pub(crate) struct AzureBackend {
    settings: SecretManagerSettings,
    imds: String,
    /// `https://{vault}.vault.azure.net`, unless a test points elsewhere.
    vault_base: Option<String>,
}

impl std::fmt::Debug for AzureBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AzureBackend").finish_non_exhaustive()
    }
}

impl AzureBackend {
    /// From the operator's settings.
    pub(crate) fn from_settings(settings: &SecretManagerSettings) -> Self {
        Self {
            settings: settings.clone(),
            imds: IMDS.to_owned(),
            vault_base: None,
        }
    }

    /// The same, against stand-ins for the instance metadata service and the
    /// vault (the tests').
    #[cfg(test)]
    pub(crate) fn with_endpoints(mut self, imds: &str, vault_base: &str) -> Self {
        imds.clone_into(&mut self.imds);
        self.vault_base = Some(vault_base.to_owned());
        self
    }

    async fn token(&self, client: &reqwest::Client) -> Result<SecretValue, SecretFailure> {
        let get = |name: &str| self.settings.get(name);
        let response = if let (Some(tenant), Some(client_id), Some(client_secret)) = (
            get("AZURE_TENANT_ID"),
            get("AZURE_CLIENT_ID"),
            get("AZURE_CLIENT_SECRET"),
        ) {
            let authority = get("AZURE_AUTHORITY_HOST")
                .unwrap_or(DEFAULT_AUTHORITY)
                .trim_end_matches('/');
            let response = client
                .post(format!("{authority}/{tenant}/oauth2/v2.0/token"))
                .form(&[
                    ("grant_type", "client_credentials"),
                    ("client_id", client_id),
                    ("client_secret", client_secret),
                    ("scope", &format!("{RESOURCE}/.default")),
                ])
                .send()
                .await
                .map_err(send_failure)?;
            if !response.status().is_success() {
                return Err(if matches!(response.status().as_u16(), 400 | 401 | 403) {
                    SecretFailure::ManagerSignIn
                } else {
                    SecretFailure::ManagerUnreachable
                });
            }
            response
        } else {
            // No app registration: the platform's identity. No answer from
            // the identity service means the platform gave the product none.
            let no_identity = |_: reqwest::Error| SecretFailure::ManagerSignIn;
            let response = if let (Some(endpoint), Some(header)) =
                (get("IDENTITY_ENDPOINT"), get("IDENTITY_HEADER"))
            {
                let mut query = vec![("resource", RESOURCE), ("api-version", "2019-08-01")];
                if let Some(id) = get("AZURE_CLIENT_ID") {
                    query.push(("client_id", id));
                }
                client
                    .get(endpoint)
                    .query(&query)
                    .header("X-IDENTITY-HEADER", header)
                    .timeout(METADATA_TIMEOUT)
                    .send()
                    .await
                    .map_err(no_identity)?
            } else {
                let mut query = vec![("api-version", "2018-02-01"), ("resource", RESOURCE)];
                if let Some(id) = get("AZURE_CLIENT_ID") {
                    query.push(("client_id", id));
                }
                client
                    .get(format!("{}/metadata/identity/oauth2/token", self.imds))
                    .query(&query)
                    .header("Metadata", "true")
                    .timeout(METADATA_TIMEOUT)
                    .send()
                    .await
                    .map_err(no_identity)?
            };
            if !response.status().is_success() {
                return Err(SecretFailure::ManagerSignIn);
            }
            response
        };
        let body: Value = response
            .json()
            .await
            .map_err(|_| SecretFailure::ManagerSignIn)?;
        body.get("access_token")
            .and_then(Value::as_str)
            .filter(|token| !token.is_empty())
            .map(SecretValue::new)
            .ok_or(SecretFailure::ManagerSignIn)
    }
}

#[async_trait::async_trait]
impl ManagerBackend for AzureBackend {
    async fn read(&self, secret_ref: &ManagerRef) -> Result<SecretValue, SecretFailure> {
        let ManagerRef::Azure {
            vault,
            secret,
            version,
        } = secret_ref
        else {
            return Err(SecretFailure::ManagerNotFound);
        };
        let client = http_client()?;
        let token = self.token(&client).await?;
        let base = self
            .vault_base
            .clone()
            .unwrap_or_else(|| format!("https://{vault}.vault.azure.net"));
        let mut url = format!("{base}/secrets/{secret}");
        if let Some(version) = version {
            url.push('/');
            url.push_str(version);
        }
        let response = client
            .get(url)
            .query(&[("api-version", API_VERSION)])
            .bearer_auth(token.expose_secret())
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
        match body.get("value") {
            None | Some(Value::Null) => Err(SecretFailure::ManagerNotFound),
            Some(Value::String(text)) => Ok(SecretValue::new(text.clone())),
            Some(_) => Err(SecretFailure::NotSingleText),
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::collections::HashMap;

    use serde_json::json;
    use wiremock::matchers::{header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::secret_manager::SecretManagerKind;

    const REF: &str = "azure-kv:corp-vault/orders-db";

    fn backend(server: &MockServer, extra: &[(&str, &str)]) -> AzureBackend {
        let mut env: HashMap<String, String> = extra
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        env.insert("SECRET_MANAGER".to_owned(), "azure".to_owned());
        let settings = SecretManagerSettings::from_map(&env).unwrap();
        AzureBackend::from_settings(&settings).with_endpoints(&server.uri(), &server.uri())
    }

    fn registration(server: &MockServer) -> Vec<(&'static str, String)> {
        vec![
            ("AZURE_TENANT_ID", "tenant-1".to_owned()),
            ("AZURE_CLIENT_ID", "client-1".to_owned()),
            ("AZURE_CLIENT_SECRET", "client-secret-test".to_owned()),
            ("AZURE_AUTHORITY_HOST", server.uri()),
        ]
    }

    fn with_registration(server: &MockServer) -> AzureBackend {
        let owned = registration(server);
        let pairs: Vec<(&str, &str)> = owned.iter().map(|(k, v)| (*k, v.as_str())).collect();
        backend(server, &pairs)
    }

    fn reference(text: &str) -> ManagerRef {
        crate::secret_manager::refs::parse(SecretManagerKind::Azure, text).unwrap()
    }

    async fn token_ok(server: &MockServer) {
        Mock::given(method("POST"))
            .and(path("/tenant-1/oauth2/v2.0/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                json!({"access_token": "eyJ.tok", "expires_in": 3599, "token_type": "Bearer"}),
            ))
            .mount(server)
            .await;
    }

    #[tokio::test]
    async fn gets_a_client_credentials_token_then_reads_the_secret() {
        let server = MockServer::start().await;
        token_ok(&server).await;
        Mock::given(method("GET"))
            .and(path("/secrets/orders-db"))
            .and(query_param("api-version", "7.4"))
            .and(header("authorization", "Bearer eyJ.tok"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "value": "pw-from-azure",
                "id": "https://corp-vault.vault.azure.net/secrets/orders-db/abc"
            })))
            .expect(1)
            .mount(&server)
            .await;
        let value = with_registration(&server)
            .read(&reference(REF))
            .await
            .unwrap();
        assert_eq!(value.expose_secret(), "pw-from-azure");
        let requests = server.received_requests().await.unwrap();
        let exchange = requests
            .iter()
            .find(|r| r.method.as_str() == "POST")
            .unwrap();
        let form: HashMap<String, String> = url::form_urlencoded::parse(&exchange.body)
            .into_owned()
            .collect();
        assert_eq!(form["grant_type"], "client_credentials");
        assert_eq!(form["client_id"], "client-1");
        assert_eq!(form["scope"], "https://vault.azure.net/.default");
    }

    #[tokio::test]
    async fn a_version_in_the_reference_is_part_of_the_path() {
        let server = MockServer::start().await;
        token_ok(&server).await;
        Mock::given(method("GET"))
            .and(path("/secrets/orders-db/0123456789abcdef0123456789abcdef"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"value": "pinned"})))
            .expect(1)
            .mount(&server)
            .await;
        let value = with_registration(&server)
            .read(&reference(
                "azure-kv:corp-vault/orders-db/0123456789abcdef0123456789abcdef",
            ))
            .await
            .unwrap();
        assert_eq!(value.expose_secret(), "pinned");
    }

    #[tokio::test]
    async fn every_read_gets_a_token_and_reads_again() {
        let server = MockServer::start().await;
        token_ok(&server).await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"value": "old"})))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"value": "new"})))
            .mount(&server)
            .await;
        let backend = with_registration(&server);
        assert_eq!(
            backend.read(&reference(REF)).await.unwrap().expose_secret(),
            "old"
        );
        assert_eq!(
            backend.read(&reference(REF)).await.unwrap().expose_secret(),
            "new"
        );
        let requests = server.received_requests().await.unwrap();
        assert_eq!(
            requests
                .iter()
                .filter(|r| r.method.as_str() == "POST")
                .count(),
            2
        );
    }

    #[tokio::test]
    async fn each_read_failure_is_classified_by_status_and_azures_text_is_dropped() {
        for (status, expected) in [
            (404, SecretFailure::ManagerNotFound),
            (403, SecretFailure::ManagerForbidden),
            (401, SecretFailure::ManagerSignIn),
            (500, SecretFailure::ManagerUnreachable),
            (307, SecretFailure::ManagerUnreachable),
        ] {
            let server = MockServer::start().await;
            token_ok(&server).await;
            Mock::given(method("GET"))
                .respond_with(ResponseTemplate::new(status).set_body_json(
                    json!({"error": {"code": "Forbidden", "message": "azure-own-words"}}),
                ))
                .mount(&server)
                .await;
            let err = with_registration(&server)
                .read(&reference(REF))
                .await
                .unwrap_err();
            assert_eq!(err, expected, "{status}");
            assert!(!err.to_string().contains("azure-own-words"));
        }
    }

    #[tokio::test]
    async fn a_refused_token_request_is_may_not_sign_in_and_shows_neither_aadsts_text_nor_the_secret()
     {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(401).set_body_json(json!({
                "error": "invalid_client",
                "error_description": "AADSTS7000215 own words"
            })))
            .mount(&server)
            .await;
        let err = with_registration(&server)
            .read(&reference(REF))
            .await
            .unwrap_err();
        assert_eq!(err, SecretFailure::ManagerSignIn);
        let shown = err.to_string();
        assert!(!shown.contains("AADSTS") && !shown.contains("client-secret-test"));
    }

    #[tokio::test]
    async fn a_value_that_is_not_text_is_not_a_single_text_and_a_missing_one_does_not_exist() {
        for (body, expected) in [
            (json!({"value": 7}), SecretFailure::NotSingleText),
            (json!({"id": "x"}), SecretFailure::ManagerNotFound),
        ] {
            let server = MockServer::start().await;
            token_ok(&server).await;
            Mock::given(method("GET"))
                .respond_with(ResponseTemplate::new(200).set_body_json(body))
                .mount(&server)
                .await;
            assert_eq!(
                with_registration(&server)
                    .read(&reference(REF))
                    .await
                    .unwrap_err(),
                expected
            );
        }
    }

    #[tokio::test]
    async fn the_instance_metadata_identity_is_used_without_an_app_registration() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/metadata/identity/oauth2/token"))
            .and(query_param("resource", "https://vault.azure.net"))
            .and(header("metadata", "true"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token": "imds-tok", "resource": "https://vault.azure.net"
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/secrets/orders-db"))
            .and(header("authorization", "Bearer imds-tok"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"value": "via-imds"})))
            .expect(1)
            .mount(&server)
            .await;
        let value = backend(&server, &[]).read(&reference(REF)).await.unwrap();
        assert_eq!(value.expose_secret(), "via-imds");
        let requests = server.received_requests().await.unwrap();
        assert!(!requests[0].url.query().unwrap().contains("client_id"));
    }

    #[tokio::test]
    async fn a_user_assigned_identity_names_its_client_id() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/metadata/identity/oauth2/token"))
            .and(query_param("client_id", "client-1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"access_token": "t"})))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/secrets/orders-db"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"value": "x"})))
            .mount(&server)
            .await;
        backend(&server, &[("AZURE_CLIENT_ID", "client-1")])
            .read(&reference(REF))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn the_app_service_identity_endpoint_is_used_when_the_platform_sets_it() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/msi/token"))
            .and(header("x-identity-header", "hdr-1"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"access_token": "app-tok"})),
            )
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/secrets/orders-db"))
            .and(header("authorization", "Bearer app-tok"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"value": "x"})))
            .expect(1)
            .mount(&server)
            .await;
        let endpoint = format!("{}/msi/token", server.uri());
        backend(
            &server,
            &[
                ("IDENTITY_ENDPOINT", endpoint.as_str()),
                ("IDENTITY_HEADER", "hdr-1"),
            ],
        )
        .read(&reference(REF))
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn no_registration_and_no_identity_service_is_may_not_sign_in() {
        let nobody = MockServer::start().await;
        let backend = backend(&nobody, &[]);
        drop(nobody);
        assert_eq!(
            backend.read(&reference(REF)).await.unwrap_err(),
            SecretFailure::ManagerSignIn
        );
    }
}
