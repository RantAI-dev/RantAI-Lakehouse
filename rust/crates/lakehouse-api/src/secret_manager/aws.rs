//! AWS Secrets Manager (`SRC-12` part B3): one signed `GetSecretValue`
//! call.
//!
//! `SigV4` is written here with `hmac` and `sha2` (both already in the
//! workspace) instead of pulling in the AWS SDK: the interface is one JSON
//! POST, and the signer is checked in the tests against AWS's published
//! `get-vanilla` vector and against a signature `botocore` produced for the
//! same Secrets Manager request. `dagster/dispar_orchestrate/secret_manager_aws.py`
//! is the twin and asserts the same two constants.
//!
//! Credentials are fetched at each read and not kept, in this order: a task
//! or pod role when the platform's environment names its endpoint
//! (`AWS_CONTAINER_CREDENTIALS_RELATIVE_URI` or `..._FULL_URI`); access keys
//! in the environment; the instance role through `IMDSv2`. The standard chain
//! tries keys first, but a task role is signalled by a variable only the
//! platform sets, so trying it first costs no probe, and the instance-role
//! probe (short timeout) comes last so a host with no metadata service fails
//! fast. Web-identity (IRSA) and profile files are not built (a limit, in
//! the result document). `AWS_ENDPOINT_URL_SECRETS_MANAGER` points the call at
//! a stand-in (the gate's `moto`); it is the operator's setting, never a
//! reference's.
//!
//! A secret's `SecretString` is returned whole, or, with `#key`, the named
//! string field of its JSON. A binary secret, a `#key` on text that is not a
//! JSON object, or a field that is not a string is "not a single text".
//! Errors are classified by AWS's `__type` code and dropped.

use std::fmt::Write as _;
use std::time::Duration;

use hmac::{Hmac, KeyInit, Mac};
use lakehouse_core::secret::{SecretFailure, SecretValue};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use time::OffsetDateTime;
use time::macros::format_description;

use super::{ManagerBackend, ManagerRef, SecretManagerSettings, http_client, send_failure};

type HmacSha256 = Hmac<Sha256>;

const IMDS_DEFAULT: &str = "http://169.254.169.254";
const ECS_HOST: &str = "http://169.254.170.2";
/// A host with no metadata service must fail fast.
const IMDS_TIMEOUT: Duration = Duration::from_secs(2);

/// AWS credentials for one read. `Debug` prints nothing of them.
struct Credentials {
    access_key: String,
    secret_key: SecretValue,
    session_token: Option<SecretValue>,
}

/// Where Secrets Manager is, and the operator's settings for signing in.
pub(crate) struct AwsBackend {
    settings: SecretManagerSettings,
}

impl std::fmt::Debug for AwsBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AwsBackend").finish_non_exhaustive()
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut out, byte| {
        let _ = write!(out, "{byte:02x}");
        out
    })
}

fn sha256_hex(data: &[u8]) -> String {
    hex(&Sha256::digest(data))
}

fn hmac_bytes(key: &[u8], message: &str) -> Vec<u8> {
    // HMAC accepts a key of any length; `new_from_slice` cannot fail here.
    let Ok(mut mac) = <HmacSha256 as KeyInit>::new_from_slice(key) else {
        return Vec::new();
    };
    mac.update(message.as_bytes());
    mac.finalize().into_bytes().to_vec()
}

/// What [`sign`] signs.
pub(crate) struct SignInput<'a> {
    pub(crate) method: &'a str,
    pub(crate) host: &'a str,
    pub(crate) path: &'a str,
    pub(crate) query: &'a str,
    /// Headers to sign besides `host` and `x-amz-date`.
    pub(crate) extra_headers: &'a [(&'a str, &'a str)],
    pub(crate) payload: &'a [u8],
    pub(crate) region: &'a str,
    pub(crate) service: &'a str,
    pub(crate) access_key: &'a str,
    pub(crate) secret_key: &'a str,
    pub(crate) session_token: Option<&'a str>,
    pub(crate) now: OffsetDateTime,
}

/// The headers (lower-case names, `Authorization` included, `host`
/// excluded: the HTTP client sets it) that SigV4-sign `input`.
pub(crate) fn sign(input: &SignInput<'_>) -> Vec<(String, String)> {
    let stamp = format_description!("[year][month][day]T[hour][minute][second]Z");
    let amz_date = input.now.format(&stamp).unwrap_or_default();
    let date = amz_date.get(..8).unwrap_or_default().to_owned();

    let mut signed: Vec<(String, String)> = vec![
        ("host".to_owned(), input.host.to_owned()),
        ("x-amz-date".to_owned(), amz_date.clone()),
    ];
    for (name, value) in input.extra_headers {
        signed.push((
            name.to_ascii_lowercase(),
            value.split_whitespace().collect::<Vec<_>>().join(" "),
        ));
    }
    if let Some(token) = input.session_token {
        signed.push(("x-amz-security-token".to_owned(), token.to_owned()));
    }
    signed.sort();
    let canonical_headers = signed.iter().fold(String::new(), |mut out, (name, value)| {
        let _ = writeln!(out, "{name}:{value}");
        out
    });
    let signed_headers = signed
        .iter()
        .map(|(name, _)| name.as_str())
        .collect::<Vec<_>>()
        .join(";");
    let canonical_request = format!(
        "{}\n{}\n{}\n{canonical_headers}\n{signed_headers}\n{}",
        input.method,
        input.path,
        input.query,
        sha256_hex(input.payload)
    );
    let scope = format!("{date}/{}/{}/aws4_request", input.region, input.service);
    let string_to_sign = format!(
        "AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{}",
        sha256_hex(canonical_request.as_bytes())
    );
    let mut key = hmac_bytes(format!("AWS4{}", input.secret_key).as_bytes(), &date);
    for part in [input.region, input.service, "aws4_request"] {
        key = hmac_bytes(&key, part);
    }
    let signature = hex(&hmac_bytes(&key, &string_to_sign));

    let mut headers: Vec<(String, String)> = signed
        .into_iter()
        .filter(|(name, _)| name != "host")
        .collect();
    headers.push((
        "Authorization".to_owned(),
        format!(
            "AWS4-HMAC-SHA256 Credential={}/{scope}, SignedHeaders={signed_headers}, Signature={signature}",
            input.access_key
        ),
    ));
    headers
}

impl AwsBackend {
    /// From the operator's settings (checked complete at start-up).
    pub(crate) fn from_settings(settings: &SecretManagerSettings) -> Self {
        Self {
            settings: settings.clone(),
        }
    }

    fn region(&self) -> Option<&str> {
        self.settings
            .get("AWS_REGION")
            .or_else(|| self.settings.get("AWS_DEFAULT_REGION"))
    }

    async fn credentials(&self, client: &reqwest::Client) -> Result<Credentials, SecretFailure> {
        let relative = self.settings.get("AWS_CONTAINER_CREDENTIALS_RELATIVE_URI");
        let full = self.settings.get("AWS_CONTAINER_CREDENTIALS_FULL_URI");
        if relative.is_some() || full.is_some() {
            let url = full.map_or_else(
                || format!("{ECS_HOST}{}", relative.unwrap_or_default()),
                str::to_owned,
            );
            let mut request = client.get(url);
            if let Some(token) = self.settings.get("AWS_CONTAINER_AUTHORIZATION_TOKEN") {
                request = request.header("Authorization", token);
            }
            let response = request.send().await.map_err(send_failure)?;
            return credentials_from(response).await;
        }
        if let (Some(key), Some(secret)) = (
            self.settings.get("AWS_ACCESS_KEY_ID"),
            self.settings.get("AWS_SECRET_ACCESS_KEY"),
        ) {
            return Ok(Credentials {
                access_key: key.to_owned(),
                secret_key: SecretValue::new(secret),
                session_token: self.settings.get("AWS_SESSION_TOKEN").map(SecretValue::new),
            });
        }
        self.instance_role(
            self.settings
                .get("AWS_EC2_METADATA_SERVICE_ENDPOINT")
                .unwrap_or(IMDS_DEFAULT)
                .trim_end_matches('/'),
        )
        .await
    }

    /// `IMDSv2`: a session token, the role name, then the role's credentials.
    /// Any failure is "no role to sign in with".
    async fn instance_role(&self, base: &str) -> Result<Credentials, SecretFailure> {
        let no_role = |_: reqwest::Error| SecretFailure::ManagerSignIn;
        let client = http_client()?;
        let text = |response: reqwest::Response| async move {
            if response.status().is_success() {
                response
                    .text()
                    .await
                    .map_err(|_| SecretFailure::ManagerSignIn)
            } else {
                Err(SecretFailure::ManagerSignIn)
            }
        };
        let token = text(
            client
                .put(format!("{base}/latest/api/token"))
                .header("X-aws-ec2-metadata-token-ttl-seconds", "60")
                .timeout(IMDS_TIMEOUT)
                .send()
                .await
                .map_err(no_role)?,
        )
        .await?;
        let roles = text(
            client
                .get(format!("{base}/latest/meta-data/iam/security-credentials/"))
                .header("X-aws-ec2-metadata-token", token.trim())
                .timeout(IMDS_TIMEOUT)
                .send()
                .await
                .map_err(no_role)?,
        )
        .await?;
        let role = roles.lines().next().map(str::trim).unwrap_or_default();
        if role.is_empty() {
            return Err(SecretFailure::ManagerSignIn);
        }
        let response = client
            .get(format!(
                "{base}/latest/meta-data/iam/security-credentials/{role}"
            ))
            .header("X-aws-ec2-metadata-token", token.trim())
            .timeout(IMDS_TIMEOUT)
            .send()
            .await
            .map_err(no_role)?;
        credentials_from(response).await
    }
}

/// The credentials in a container-role or instance-role response.
async fn credentials_from(response: reqwest::Response) -> Result<Credentials, SecretFailure> {
    if !response.status().is_success() {
        return Err(SecretFailure::ManagerSignIn);
    }
    let body: Value = response
        .json()
        .await
        .map_err(|_| SecretFailure::ManagerSignIn)?;
    let text = |name: &str| {
        body.get(name)
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty())
    };
    let (Some(key), Some(secret)) = (text("AccessKeyId"), text("SecretAccessKey")) else {
        return Err(SecretFailure::ManagerSignIn);
    };
    Ok(Credentials {
        access_key: key.to_owned(),
        secret_key: SecretValue::new(secret),
        session_token: text("Token").map(SecretValue::new),
    })
}

/// What an AWS error `__type` means to a user.
fn failure_for(status: u16, code: &str) -> SecretFailure {
    match code {
        "UnrecognizedClientException"
        | "InvalidSignatureException"
        | "ExpiredTokenException"
        | "IncompleteSignature"
        | "InvalidClientTokenId"
        | "MissingAuthenticationToken"
        | "SignatureDoesNotMatch" => SecretFailure::ManagerSignIn,
        "AccessDeniedException" | "AccessDenied" => SecretFailure::ManagerForbidden,
        "ResourceNotFoundException" | "InvalidRequestException" => SecretFailure::ManagerNotFound,
        _ => match status {
            401 => SecretFailure::ManagerSignIn,
            403 => SecretFailure::ManagerForbidden,
            404 => SecretFailure::ManagerNotFound,
            _ => SecretFailure::ManagerUnreachable,
        },
    }
}

#[async_trait::async_trait]
impl ManagerBackend for AwsBackend {
    async fn read(&self, secret_ref: &ManagerRef) -> Result<SecretValue, SecretFailure> {
        let ManagerRef::Aws { secret_id, key } = secret_ref else {
            return Err(SecretFailure::ManagerNotFound);
        };
        let region = self.region().ok_or(SecretFailure::ManagerSignIn)?;
        let endpoint = self
            .settings
            .get("AWS_ENDPOINT_URL_SECRETS_MANAGER")
            .map_or_else(
                || format!("https://secretsmanager.{region}.amazonaws.com"),
                |url| url.trim_end_matches('/').to_owned(),
            );
        let host = endpoint
            .split_once("://")
            .map(|(_, host)| host)
            .filter(|_| endpoint.starts_with("http"))
            .ok_or(SecretFailure::ManagerUnreachable)?;
        let client = http_client()?;
        let credentials = self.credentials(&client).await?;

        let payload = json!({ "SecretId": secret_id }).to_string();
        let extra = [
            ("content-type", "application/x-amz-json-1.1"),
            ("x-amz-target", "secretsmanager.GetSecretValue"),
        ];
        let headers = sign(&SignInput {
            method: "POST",
            host,
            path: "/",
            query: "",
            extra_headers: &extra,
            payload: payload.as_bytes(),
            region,
            service: "secretsmanager",
            access_key: &credentials.access_key,
            secret_key: credentials.secret_key.expose_secret(),
            session_token: credentials
                .session_token
                .as_ref()
                .map(SecretValue::expose_secret),
            now: OffsetDateTime::now_utc(),
        });
        let mut request = client.post(format!("{endpoint}/")).body(payload);
        for (name, value) in headers {
            request = request.header(name, value);
        }
        let response = request.send().await.map_err(send_failure)?;
        let status = response.status();
        if !status.is_success() {
            let code = response
                .json::<Value>()
                .await
                .ok()
                .and_then(|body| {
                    body.get("__type")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                })
                .unwrap_or_default();
            // `__type` may be namespaced (`com.amazon...#Name`).
            return Err(failure_for(
                status.as_u16(),
                code.rsplit('#').next().unwrap_or_default(),
            ));
        }
        let body: Value = response
            .json()
            .await
            .map_err(|_| SecretFailure::ManagerUnreachable)?;
        let text = body
            .get("SecretString")
            .and_then(Value::as_str)
            .ok_or(SecretFailure::NotSingleText)?;
        let Some(field) = key else {
            return Ok(SecretValue::new(text));
        };
        let document: Value =
            serde_json::from_str(text).map_err(|_| SecretFailure::NotSingleText)?;
        match document.as_object().and_then(|object| object.get(field)) {
            None => Err(SecretFailure::ManagerNotFound),
            Some(Value::String(value)) => Ok(SecretValue::new(value.clone())),
            Some(_) => Err(SecretFailure::NotSingleText),
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::collections::HashMap;

    use time::macros::datetime;
    use wiremock::matchers::{header, header_exists, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::secret_manager::SecretManagerKind;

    fn authorization(headers: &[(String, String)]) -> &str {
        headers
            .iter()
            .find(|(name, _)| name == "Authorization")
            .map(|(_, value)| value.as_str())
            .unwrap()
    }

    /// AWS's published `SigV4` test-suite case "get-vanilla"; the Python
    /// signer asserts the same signature.
    #[test]
    fn the_signer_matches_the_published_get_vanilla_vector() {
        let headers = sign(&SignInput {
            method: "GET",
            host: "example.amazonaws.com",
            path: "/",
            query: "",
            extra_headers: &[],
            payload: b"",
            region: "us-east-1",
            service: "service",
            access_key: "AKIDEXAMPLE",
            secret_key: "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY",
            session_token: None,
            now: datetime!(2015-08-30 12:36:00 UTC),
        });
        assert_eq!(
            authorization(&headers),
            "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20150830/us-east-1/service/aws4_request, \
             SignedHeaders=host;x-amz-date, \
             Signature=5fa00fa31553b73ebf1942676e86291e8372ff2a2260956d9b8aae1d763fbf31"
        );
    }

    /// Produced once with `botocore.auth.SigV4Auth` for exactly these inputs
    /// (fixed clock, POST, session token); the Python signer asserts it too.
    #[test]
    fn the_signer_matches_what_botocore_signed_for_the_same_secrets_manager_request() {
        let payload = json!({ "SecretId": "prod/x" }).to_string();
        let headers = sign(&SignInput {
            method: "POST",
            host: "secretsmanager.eu-west-1.amazonaws.com",
            path: "/",
            query: "",
            extra_headers: &[
                ("content-type", "application/x-amz-json-1.1"),
                ("x-amz-target", "secretsmanager.GetSecretValue"),
            ],
            payload: payload.as_bytes(),
            region: "eu-west-1",
            service: "secretsmanager",
            access_key: "AKID",
            secret_key: "SECRET",
            session_token: Some("TOK"),
            now: datetime!(2026-10-15 10:00:00 UTC),
        });
        assert!(authorization(&headers).ends_with(
            "Signature=61958663bab4354b5593a4b768239b0c320224a9fd102f4455863ec05cbf1e88"
        ));
    }

    fn backend(server: &MockServer, extra: &[(&str, &str)]) -> AwsBackend {
        let mut env: HashMap<String, String> = extra
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        env.insert("SECRET_MANAGER".to_owned(), "aws".to_owned());
        env.insert("AWS_REGION".to_owned(), "eu-west-1".to_owned());
        env.insert("AWS_ENDPOINT_URL_SECRETS_MANAGER".to_owned(), server.uri());
        let settings = SecretManagerSettings::from_map(&env).unwrap();
        AwsBackend::from_settings(&settings)
    }

    const KEYS: [(&str, &str); 2] = [
        ("AWS_ACCESS_KEY_ID", "AKIDTEST"),
        ("AWS_SECRET_ACCESS_KEY", "secret-access-key-test"),
    ];

    fn reference(text: &str) -> ManagerRef {
        crate::secret_manager::refs::parse(SecretManagerKind::Aws, text).unwrap()
    }

    #[tokio::test]
    async fn reads_a_json_field_with_a_signed_request() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/"))
            .and(header("x-amz-target", "secretsmanager.GetSecretValue"))
            .and(header("content-type", "application/x-amz-json-1.1"))
            .and(header_exists("x-amz-date"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "Name": "prod/orders-db",
                "SecretString": "{\"password\":\"pw-from-aws\",\"user\":\"u\"}"
            })))
            .expect(1)
            .mount(&server)
            .await;
        let value = backend(&server, &KEYS)
            .read(&reference("aws-sm:prod/orders-db#password"))
            .await
            .unwrap();
        assert_eq!(value.expose_secret(), "pw-from-aws");
        let requests = server.received_requests().await.unwrap();
        let request = &requests[0];
        assert_eq!(
            serde_json::from_slice::<Value>(&request.body).unwrap(),
            json!({"SecretId": "prod/orders-db"})
        );
        let auth = request
            .headers
            .get("authorization")
            .unwrap()
            .to_str()
            .unwrap();
        assert!(auth.starts_with("AWS4-HMAC-SHA256 Credential=AKIDTEST/"));
        assert!(auth.contains("/eu-west-1/secretsmanager/aws4_request"));
        assert!(!auth.contains("secret-access-key-test"));
    }

    #[tokio::test]
    async fn without_a_key_the_whole_secret_text_is_the_value_and_every_read_asks_again() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"SecretString": "old"})))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"SecretString": "new"})))
            .mount(&server)
            .await;
        let backend = backend(&server, &KEYS);
        let reference = reference("aws-sm:prod/orders-db");
        assert_eq!(
            backend.read(&reference).await.unwrap().expose_secret(),
            "old"
        );
        assert_eq!(
            backend.read(&reference).await.unwrap().expose_secret(),
            "new"
        );
    }

    #[tokio::test]
    async fn a_session_token_is_signed_and_sent() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(header("x-amz-security-token", "session-tok"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"SecretString": "x"})))
            .expect(1)
            .mount(&server)
            .await;
        let mut env = KEYS.to_vec();
        env.push(("AWS_SESSION_TOKEN", "session-tok"));
        backend(&server, &env)
            .read(&reference("aws-sm:prod/orders-db"))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn each_failure_is_classified_by_its_error_type_and_awss_text_is_dropped() {
        for (status, code, expected) in [
            (
                400,
                "com.amazon.coral.service#ResourceNotFoundException",
                SecretFailure::ManagerNotFound,
            ),
            (
                400,
                "InvalidRequestException",
                SecretFailure::ManagerNotFound,
            ),
            (
                400,
                "AccessDeniedException",
                SecretFailure::ManagerForbidden,
            ),
            (
                403,
                "AccessDeniedException",
                SecretFailure::ManagerForbidden,
            ),
            (
                400,
                "UnrecognizedClientException",
                SecretFailure::ManagerSignIn,
            ),
            (400, "ExpiredTokenException", SecretFailure::ManagerSignIn),
            (
                500,
                "InternalServiceError",
                SecretFailure::ManagerUnreachable,
            ),
        ] {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .respond_with(
                    ResponseTemplate::new(status)
                        .set_body_json(json!({"__type": code, "message": "aws-own-words"})),
                )
                .mount(&server)
                .await;
            let err = backend(&server, &KEYS)
                .read(&reference("aws-sm:prod/orders-db#password"))
                .await
                .unwrap_err();
            assert_eq!(err, expected, "{status} {code}");
            assert!(!err.to_string().contains("aws-own-words"));
        }
    }

    #[tokio::test]
    async fn a_binary_secret_non_json_text_for_a_key_and_a_non_string_field_are_not_a_single_text()
    {
        for body in [
            json!({"SecretBinary": "AAEC"}),
            json!({"SecretString": "not json"}),
            json!({"SecretString": "{\"password\": 7}"}),
        ] {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .respond_with(ResponseTemplate::new(200).set_body_json(body))
                .mount(&server)
                .await;
            assert_eq!(
                backend(&server, &KEYS)
                    .read(&reference("aws-sm:prod/orders-db#password"))
                    .await
                    .unwrap_err(),
                SecretFailure::NotSingleText
            );
        }
    }

    #[tokio::test]
    async fn a_missing_field_does_not_exist() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"SecretString": "{\"user\":\"u\"}"})),
            )
            .mount(&server)
            .await;
        assert_eq!(
            backend(&server, &KEYS)
                .read(&reference("aws-sm:prod/orders-db#password"))
                .await
                .unwrap_err(),
            SecretFailure::ManagerNotFound
        );
    }

    #[tokio::test]
    async fn a_task_role_is_used_before_environment_keys() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/creds/role"))
            .and(header("authorization", "auth-tok"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "AccessKeyId": "AKIDROLE",
                "SecretAccessKey": "role-secret",
                "Token": "role-token"
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(header("x-amz-security-token", "role-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"SecretString": "x"})))
            .expect(1)
            .mount(&server)
            .await;
        let full = format!("{}/creds/role", server.uri());
        let mut env = KEYS.to_vec();
        env.push(("AWS_CONTAINER_CREDENTIALS_FULL_URI", full.as_str()));
        env.push(("AWS_CONTAINER_AUTHORIZATION_TOKEN", "auth-tok"));
        backend(&server, &env)
            .read(&reference("aws-sm:prod/orders-db"))
            .await
            .unwrap();
        let requests = server.received_requests().await.unwrap();
        let read = requests
            .iter()
            .find(|r| r.method.as_str() == "POST")
            .unwrap();
        let auth = read.headers.get("authorization").unwrap().to_str().unwrap();
        assert!(auth.contains("Credential=AKIDROLE/"));
    }

    #[tokio::test]
    async fn the_instance_role_through_imds_when_there_are_no_keys() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path("/latest/api/token"))
            .respond_with(ResponseTemplate::new(200).set_body_string("imds-token"))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/latest/meta-data/iam/security-credentials/"))
            .and(header("x-aws-ec2-metadata-token", "imds-token"))
            .respond_with(ResponseTemplate::new(200).set_body_string("role-name\n"))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/latest/meta-data/iam/security-credentials/role-name"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "AccessKeyId": "AKIDIMDS",
                "SecretAccessKey": "imds-secret",
                "Token": "imds-token-2"
            })))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(header("x-amz-security-token", "imds-token-2"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"SecretString": "x"})))
            .expect(1)
            .mount(&server)
            .await;
        let imds = server.uri();
        let value = backend(
            &server,
            &[("AWS_EC2_METADATA_SERVICE_ENDPOINT", imds.as_str())],
        )
        .read(&reference("aws-sm:prod/orders-db"))
        .await
        .unwrap();
        assert_eq!(value.expose_secret(), "x");
    }

    #[tokio::test]
    async fn no_keys_and_no_metadata_service_is_may_not_sign_in() {
        let server = MockServer::start().await;
        let nobody = MockServer::start().await;
        let dead = nobody.uri();
        drop(nobody);
        assert_eq!(
            backend(
                &server,
                &[("AWS_EC2_METADATA_SERVICE_ENDPOINT", dead.as_str())]
            )
            .read(&reference("aws-sm:prod/orders-db"))
            .await
            .unwrap_err(),
            SecretFailure::ManagerSignIn
        );
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn an_endpoint_nobody_answers_is_unreachable() {
        let server = MockServer::start().await;
        let backend = backend(&server, &KEYS);
        drop(server);
        let err = backend
            .read(&reference("aws-sm:prod/orders-db"))
            .await
            .unwrap_err();
        assert_eq!(err, SecretFailure::ManagerUnreachable);
        assert!(!err.to_string().contains("127.0.0.1"));
    }
}
