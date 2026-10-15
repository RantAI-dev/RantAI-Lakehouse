//! Google Secret Manager (`SRC-12` part B4): `GET
//! https://secretmanager.googleapis.com/v1/{secret version}:access` with an
//! OAuth access token, fetched at every read.
//!
//! The token comes from the service-account key file at
//! `GOOGLE_APPLICATION_CREDENTIALS` (a self-signed RS256 JWT exchanged at the
//! key's `token_uri`, the documented JWT-bearer grant, signed with
//! `jsonwebtoken`, already a workspace dependency) or, with no key file,
//! from the platform's metadata identity (`GCE_METADATA_HOST`, default
//! `metadata.google.internal`). Only `type: service_account` key files are
//! read; anything else is "may not sign in". No Google client library is
//! used. The project is part of the reference, not of the environment.
//!
//! Tested against recorded responses of the documented interface; *not run
//! against the real service* (decision 3, said in the result document).
//! `dagster/dispar_orchestrate/secret_manager_gcp.py` is the twin.

use std::time::Duration;

use base64::Engine as _;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use lakehouse_core::secret::{SecretFailure, SecretValue};
use serde::Serialize;
use serde_json::Value;

use super::{
    ManagerBackend, ManagerRef, SecretManagerSettings, http_client, read_stage_failure,
    send_failure,
};

const API_BASE: &str = "https://secretmanager.googleapis.com";
const DEFAULT_TOKEN_URI: &str = "https://oauth2.googleapis.com/token";
const SCOPE: &str = "https://www.googleapis.com/auth/cloud-platform";
/// A host with no metadata service must fail fast.
const METADATA_TIMEOUT: Duration = Duration::from_secs(2);

/// Where Secret Manager is, and how the product signs in.
pub(crate) struct GcpBackend {
    settings: SecretManagerSettings,
    api_base: String,
}

impl std::fmt::Debug for GcpBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GcpBackend").finish_non_exhaustive()
    }
}

#[derive(Serialize)]
struct Claims<'a> {
    iss: &'a str,
    scope: &'a str,
    aud: &'a str,
    iat: i64,
    exp: i64,
}

impl GcpBackend {
    /// From the operator's settings.
    pub(crate) fn from_settings(settings: &SecretManagerSettings) -> Self {
        Self {
            settings: settings.clone(),
            api_base: API_BASE.to_owned(),
        }
    }

    /// The same, against another API address (the tests' stand-in).
    #[cfg(test)]
    pub(crate) fn with_api_base(mut self, api_base: &str) -> Self {
        api_base.clone_into(&mut self.api_base);
        self
    }

    async fn token(&self, client: &reqwest::Client) -> Result<SecretValue, SecretFailure> {
        let response = if let Some(path) = self.settings.get("GOOGLE_APPLICATION_CREDENTIALS") {
            let (token_uri, assertion) = signed_assertion(path)?;
            client
                .post(token_uri)
                .form(&[
                    ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
                    ("assertion", assertion.as_str()),
                ])
                .send()
                .await
                .map_err(send_failure)?
        } else {
            let host = self
                .settings
                .get("GCE_METADATA_HOST")
                .unwrap_or("metadata.google.internal");
            // No answer from the metadata service means the platform gave
            // the product no identity: "may not sign in".
            client
                .get(format!(
                    "http://{host}/computeMetadata/v1/instance/service-accounts/default/token"
                ))
                .header("Metadata-Flavor", "Google")
                .timeout(METADATA_TIMEOUT)
                .send()
                .await
                .map_err(|_| SecretFailure::ManagerSignIn)?
        };
        if !response.status().is_success() {
            return Err(
                if matches!(response.status().as_u16(), 400 | 401 | 403 | 404) {
                    SecretFailure::ManagerSignIn
                } else {
                    SecretFailure::ManagerUnreachable
                },
            );
        }
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

/// The key file's `token_uri` and the RS256 JWT to present there. Any
/// problem with the file is "may not sign in"; nothing of the file is
/// reported.
fn signed_assertion(path: &str) -> Result<(String, String), SecretFailure> {
    let sign_in = || SecretFailure::ManagerSignIn;
    let file: Value = serde_json::from_slice(&std::fs::read(path).map_err(|_| sign_in())?)
        .map_err(|_| sign_in())?;
    let text = |name: &str| {
        file.get(name)
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty())
    };
    if text("type") != Some("service_account") {
        return Err(sign_in());
    }
    let (Some(email), Some(pem)) = (text("client_email"), text("private_key")) else {
        return Err(sign_in());
    };
    let token_uri = text("token_uri").unwrap_or(DEFAULT_TOKEN_URI);
    let key = EncodingKey::from_rsa_pem(pem.as_bytes()).map_err(|_| sign_in())?;
    let now = time::OffsetDateTime::now_utc().unix_timestamp();
    let assertion = jsonwebtoken::encode(
        &Header::new(Algorithm::RS256),
        &Claims {
            iss: email,
            scope: SCOPE,
            aud: token_uri,
            iat: now,
            exp: now + 3600,
        },
        &key,
    )
    .map_err(|_| sign_in())?;
    Ok((token_uri.to_owned(), assertion))
}

#[async_trait::async_trait]
impl ManagerBackend for GcpBackend {
    async fn read(&self, secret_ref: &ManagerRef) -> Result<SecretValue, SecretFailure> {
        let ManagerRef::Gcp {
            project,
            secret,
            version,
        } = secret_ref
        else {
            return Err(SecretFailure::ManagerNotFound);
        };
        let client = http_client()?;
        let token = self.token(&client).await?;
        // `project`, `secret` and `version` are validated name parts
        // (`refs::parse_gcp`), so the path stays under `/v1/projects/`.
        let response = client
            .get(format!(
                "{}/v1/projects/{project}/secrets/{secret}/versions/{version}:access",
                self.api_base
            ))
            .bearer_auth(token.expose_secret())
            .send()
            .await
            .map_err(send_failure)?;
        let status = response.status();
        if !status.is_success() {
            // A disabled or destroyed version answers 400
            // (FAILED_PRECONDITION): it cannot be read, which to a user is
            // "does not exist".
            return Err(if status.as_u16() == 400 {
                SecretFailure::ManagerNotFound
            } else {
                read_stage_failure(status)
            });
        }
        let body: Value = response
            .json()
            .await
            .map_err(|_| SecretFailure::ManagerUnreachable)?;
        let data = body
            .pointer("/payload/data")
            .and_then(Value::as_str)
            .ok_or(SecretFailure::ManagerNotFound)?;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(data)
            .map_err(|_| SecretFailure::NotSingleText)?;
        String::from_utf8(bytes)
            .map(SecretValue::new)
            .map_err(|_| SecretFailure::NotSingleText)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::collections::HashMap;

    use rsa::pkcs8::{EncodePrivateKey, LineEnding};
    use rsa::traits::PublicKeyParts as _;
    use serde_json::json;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::secret_manager::SecretManagerKind;

    struct Fixture {
        server: MockServer,
        backend: GcpBackend,
        public_key: rsa::RsaPublicKey,
        _dir: tempfile::TempDir,
    }

    /// A service-account key file with a freshly generated key (never a
    /// literal in the repository), whose `token_uri` is the stand-in.
    async fn fixture(overrides: Value) -> Fixture {
        let server = MockServer::start().await;
        // `rsa` 0.9 wants a `rand_core` 0.6 RNG: its own `OsRng`.
        let mut rng = rsa::rand_core::OsRng;
        let private_key = rsa::RsaPrivateKey::new(&mut rng, 2048).unwrap();
        let pem = private_key.to_pkcs8_pem(LineEnding::LF).unwrap();
        let mut file = json!({
            "type": "service_account",
            "client_email": "reader@my-project-1.iam.gserviceaccount.com",
            "private_key": pem.as_str(),
            "token_uri": format!("{}/token", server.uri()),
        });
        if let (Some(file), Some(extra)) = (file.as_object_mut(), overrides.as_object()) {
            file.extend(extra.clone());
        }
        let dir = tempfile::tempdir().unwrap();
        let key_path = dir.path().join("sa.json");
        std::fs::write(&key_path, file.to_string()).unwrap();
        let backend = backend_with(&server, Some(key_path.to_str().unwrap()), None);
        Fixture {
            server,
            backend,
            public_key: private_key.to_public_key(),
            _dir: dir,
        }
    }

    fn backend_with(
        server: &MockServer,
        key_path: Option<&str>,
        metadata: Option<&str>,
    ) -> GcpBackend {
        let mut env: HashMap<String, String> =
            [("SECRET_MANAGER".to_owned(), "gcp".to_owned())].into();
        if let Some(path) = key_path {
            env.insert("GOOGLE_APPLICATION_CREDENTIALS".to_owned(), path.to_owned());
        }
        if let Some(host) = metadata {
            env.insert("GCE_METADATA_HOST".to_owned(), host.to_owned());
        }
        let settings = SecretManagerSettings::from_map(&env).unwrap();
        GcpBackend::from_settings(&settings).with_api_base(&server.uri())
    }

    fn reference(text: &str) -> ManagerRef {
        crate::secret_manager::refs::parse(SecretManagerKind::Gcp, text).unwrap()
    }

    const REF: &str = "gcp-sm:projects/my-project-1/secrets/orders-db/versions/latest";
    const ACCESS: &str = "/v1/projects/my-project-1/secrets/orders-db/versions/latest:access";

    fn payload(text: &str) -> ResponseTemplate {
        ResponseTemplate::new(200).set_body_json(json!({
            "name": "projects/1/secrets/orders-db/versions/3",
            "payload": {"data": base64::engine::general_purpose::STANDARD.encode(text)}
        }))
    }

    async fn token_ok(server: &MockServer) {
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                json!({"access_token": "ya29.tok", "expires_in": 3599, "token_type": "Bearer"}),
            ))
            .mount(server)
            .await;
    }

    #[tokio::test]
    async fn exchanges_a_signed_assertion_then_reads_the_version() {
        let fx = fixture(json!({})).await;
        token_ok(&fx.server).await;
        Mock::given(method("GET"))
            .and(path(ACCESS))
            .and(header("authorization", "Bearer ya29.tok"))
            .respond_with(payload("pw-from-gcp"))
            .expect(1)
            .mount(&fx.server)
            .await;
        let value = fx.backend.read(&reference(REF)).await.unwrap();
        assert_eq!(value.expose_secret(), "pw-from-gcp");

        let requests = fx.server.received_requests().await.unwrap();
        let exchange = requests.iter().find(|r| r.url.path() == "/token").unwrap();
        let form: HashMap<String, String> = url::form_urlencoded::parse(&exchange.body)
            .into_owned()
            .collect();
        assert_eq!(
            form["grant_type"],
            "urn:ietf:params:oauth:grant-type:jwt-bearer"
        );
        // The assertion verifies with the key file's public half and
        // carries the documented claims.
        let b64 = |bytes: Vec<u8>| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes);
        let decoding = jsonwebtoken::DecodingKey::from_rsa_components(
            &b64(fx.public_key.n().to_bytes_be()),
            &b64(fx.public_key.e().to_bytes_be()),
        )
        .unwrap();
        let mut validation = jsonwebtoken::Validation::new(Algorithm::RS256);
        validation.set_audience(&[format!("{}/token", fx.server.uri())]);
        validation.set_required_spec_claims(&["exp", "aud", "iss"]);
        let claims = jsonwebtoken::decode::<Value>(&form["assertion"], &decoding, &validation)
            .unwrap()
            .claims;
        assert_eq!(claims["iss"], "reader@my-project-1.iam.gserviceaccount.com");
        assert_eq!(claims["scope"], SCOPE);
        assert_eq!(
            claims["exp"].as_i64().unwrap() - claims["iat"].as_i64().unwrap(),
            3600
        );
    }

    #[tokio::test]
    async fn every_read_exchanges_and_reads_again() {
        let fx = fixture(json!({})).await;
        token_ok(&fx.server).await;
        Mock::given(method("GET"))
            .and(path(ACCESS))
            .respond_with(payload("first"))
            .up_to_n_times(1)
            .mount(&fx.server)
            .await;
        Mock::given(method("GET"))
            .and(path(ACCESS))
            .respond_with(payload("second"))
            .mount(&fx.server)
            .await;
        assert_eq!(
            fx.backend
                .read(&reference(REF))
                .await
                .unwrap()
                .expose_secret(),
            "first"
        );
        assert_eq!(
            fx.backend
                .read(&reference(REF))
                .await
                .unwrap()
                .expose_secret(),
            "second"
        );
        let requests = fx.server.received_requests().await.unwrap();
        assert_eq!(
            requests.iter().filter(|r| r.url.path() == "/token").count(),
            2
        );
    }

    #[tokio::test]
    async fn each_read_failure_is_classified_by_status_and_googles_text_is_dropped() {
        for (status, expected) in [
            (404, SecretFailure::ManagerNotFound),
            (400, SecretFailure::ManagerNotFound),
            (403, SecretFailure::ManagerForbidden),
            (401, SecretFailure::ManagerSignIn),
            (500, SecretFailure::ManagerUnreachable),
            (307, SecretFailure::ManagerUnreachable),
        ] {
            let fx = fixture(json!({})).await;
            token_ok(&fx.server).await;
            Mock::given(method("GET"))
                .respond_with(
                    ResponseTemplate::new(status)
                        .set_body_json(json!({"error": {"message": "google-own-words"}})),
                )
                .mount(&fx.server)
                .await;
            let err = fx.backend.read(&reference(REF)).await.unwrap_err();
            assert_eq!(err, expected, "{status}");
            assert!(!err.to_string().contains("google-own-words"));
        }
    }

    #[tokio::test]
    async fn a_refused_token_exchange_is_may_not_sign_in() {
        let fx = fixture(json!({})).await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(
                ResponseTemplate::new(400).set_body_json(
                    json!({"error": "invalid_grant", "error_description": "own words"}),
                ),
            )
            .mount(&fx.server)
            .await;
        let err = fx.backend.read(&reference(REF)).await.unwrap_err();
        assert_eq!(err, SecretFailure::ManagerSignIn);
        assert!(!err.to_string().contains("own words"));
    }

    #[tokio::test]
    async fn a_key_file_that_is_not_a_usable_service_account_is_may_not_sign_in() {
        for overrides in [
            json!({"type": "authorized_user"}),
            json!({"private_key": "not a pem"}),
            json!({"client_email": ""}),
        ] {
            let fx = fixture(overrides).await;
            assert_eq!(
                fx.backend.read(&reference(REF)).await.unwrap_err(),
                SecretFailure::ManagerSignIn
            );
            assert!(fx.server.received_requests().await.unwrap().is_empty());
        }
        let server = MockServer::start().await;
        assert_eq!(
            backend_with(&server, Some("/nonexistent/sa.json"), None)
                .read(&reference(REF))
                .await
                .unwrap_err(),
            SecretFailure::ManagerSignIn
        );
    }

    #[tokio::test]
    async fn without_a_key_file_the_metadata_identity_is_used() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(
                "/computeMetadata/v1/instance/service-accounts/default/token",
            ))
            .and(header("metadata-flavor", "Google"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"access_token": "meta-tok", "expires_in": 3000})),
            )
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(ACCESS))
            .and(header("authorization", "Bearer meta-tok"))
            .respond_with(payload("via-metadata"))
            .expect(1)
            .mount(&server)
            .await;
        let host = server.uri().trim_start_matches("http://").to_owned();
        let value = backend_with(&server, None, Some(&host))
            .read(&reference(REF))
            .await
            .unwrap();
        assert_eq!(value.expose_secret(), "via-metadata");
    }

    #[tokio::test]
    async fn no_key_file_and_no_metadata_service_is_may_not_sign_in() {
        let server = MockServer::start().await;
        let nobody = MockServer::start().await;
        let dead = nobody.uri().trim_start_matches("http://").to_owned();
        drop(nobody);
        assert_eq!(
            backend_with(&server, None, Some(&dead))
                .read(&reference(REF))
                .await
                .unwrap_err(),
            SecretFailure::ManagerSignIn
        );
    }

    #[tokio::test]
    async fn a_payload_that_is_not_text_is_not_a_single_text() {
        let fx = fixture(json!({})).await;
        token_ok(&fx.server).await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "payload": {"data": base64::engine::general_purpose::STANDARD.encode([0xff_u8, 0xfe, 0x00])}
            })))
            .mount(&fx.server)
            .await;
        assert_eq!(
            fx.backend.read(&reference(REF)).await.unwrap_err(),
            SecretFailure::NotSingleText
        );
    }
}
