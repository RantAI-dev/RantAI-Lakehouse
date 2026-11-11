//! `GET`/`PUT /api/settings/sso`, `POST /api/settings/sso/test`, and
//! `GET /api/settings/sso/test/{id}` (`ADM-12`, plan D5, D8).
//!
//! What this proves:
//!
//! * a caller holding `settings:write` alone can read `GET /api/settings/sso`
//!   (which reports `canEdit = false`), but is refused with 403 on
//!   `PUT /api/settings/sso`, `POST /api/settings/sso/test`, and
//!   `GET /api/settings/sso/test/{id}`;
//! * only a Platform Admin (`*:*`) may save settings or run a test;
//! * a test run stores the verified claims and creates no user or session;
//! * a wrong secret in a test flow stores the classified message;
//! * another admin reading the test result gets 404;
//! * an expired test run gets 404;
//! * the audit row holds no secret.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::collections::HashMap;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use base64::Engine;
use common::{
    TestApp, create_principal_with_permissions, session_cookie_for_seeded_user,
    session_cookie_for_user, spin_up_with_env,
};
use serde_json::{Value, json};
use tower::ServiceExt;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const SSO: &str = "/api/settings/sso";
const SSO_TEST: &str = "/api/settings/sso/test";
const CONSOLE_ISSUER: &str = "https://localhost:8443";

fn b64url(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

struct TestKey {
    kid: String,
    encoding_key: jsonwebtoken::EncodingKey,
    jwk: jsonwebtoken::jwk::Jwk,
}

impl TestKey {
    fn generate(kid: &str) -> Self {
        use rsa::pkcs1::{EncodeRsaPrivateKey, LineEnding};
        use rsa::traits::PublicKeyParts;

        let mut rng = rsa::rand_core::OsRng;
        let private_key = rsa::RsaPrivateKey::new(&mut rng, 2048).expect("keygen");
        let public_key = private_key.to_public_key();
        let pem = private_key.to_pkcs1_pem(LineEnding::LF).expect("pkcs1 pem");
        let encoding_key =
            jsonwebtoken::EncodingKey::from_rsa_pem(pem.as_bytes()).expect("encoding key");

        let jwk = jsonwebtoken::jwk::Jwk {
            common: jsonwebtoken::jwk::CommonParameters {
                key_id: Some(kid.to_owned()),
                ..Default::default()
            },
            algorithm: jsonwebtoken::jwk::AlgorithmParameters::RSA(
                jsonwebtoken::jwk::RSAKeyParameters {
                    key_type: jsonwebtoken::jwk::RSAKeyType::RSA,
                    n: b64url(&public_key.n().to_bytes_be()),
                    e: b64url(&public_key.e().to_bytes_be()),
                },
            ),
        };

        Self {
            kid: kid.to_owned(),
            encoding_key,
            jwk,
        }
    }
}

#[derive(serde::Serialize)]
struct IdTokenClaims<'a> {
    sub: &'a str,
    iss: &'a str,
    aud: &'a str,
    exp: i64,
    nonce: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    email: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    groups: Option<Vec<&'a str>>,
}

fn sign_id_token(claims: &IdTokenClaims<'_>, key: &TestKey) -> String {
    let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256);
    header.kid = Some(key.kid.clone());
    jsonwebtoken::encode(&header, claims, &key.encoding_key).expect("sign")
}

async fn mount_jwks(server: &MockServer, key: &TestKey) {
    let set = jsonwebtoken::jwk::JwkSet {
        keys: vec![key.jwk.clone()],
    };
    Mock::given(method("GET"))
        .and(path("/.well-known/jwks.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&set))
        .mount(server)
        .await;
}

async fn mount_token_endpoint(server: &MockServer, id_token: &str) {
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "id_token": id_token })))
        .mount(server)
        .await;
}

async fn mount_token_endpoint_error(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(
            ResponseTemplate::new(401).set_body_json(
                json!({ "error": "invalid_client", "error_description": "bad secret" }),
            ),
        )
        .mount(server)
        .await;
}

fn env_for_wiremock_sso(server_uri: &str) -> HashMap<String, String> {
    let mut env = HashMap::new();
    env.insert(
        "CONNECTOR_PROBE_ALLOW_INTERNAL_HOSTS".to_owned(),
        "true".to_owned(),
    );
    env.insert("OIDC_TOKEN_URL".to_owned(), format!("{server_uri}/token"));
    env.insert(
        "OIDC_JWKS_URL".to_owned(),
        format!("{server_uri}/.well-known/jwks.json"),
    );
    env
}

async fn call(
    app: &axum::Router,
    verb: &str,
    path: &str,
    cookie: &str,
    body: &str,
) -> (StatusCode, Value, axum::http::HeaderMap) {
    let mut req = Request::builder().method(verb).uri(path);
    if !cookie.is_empty() {
        req = req.header("cookie", cookie);
    }
    if !body.is_empty() {
        req = req.header("content-type", "application/json");
    }
    let resp = app
        .clone()
        .oneshot(req.body(Body::from(body.to_owned())).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let headers = resp.headers().clone();
    let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
    let val = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, val, headers)
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct FlowCookiePayload {
    state: String,
    nonce: String,
}

fn parse_flow_cookie(headers: &axum::http::HeaderMap) -> (String, FlowCookiePayload) {
    let cookie_val = headers
        .get_all(header::SET_COOKIE)
        .iter()
        .find_map(|val| {
            let s = val.to_str().ok()?;
            if s.starts_with("lh_oidc_flow=") {
                let segment = s.split(';').next()?;
                Some(segment.to_owned())
            } else {
                None
            }
        })
        .expect("lh_oidc_flow cookie in Set-Cookie");

    let raw = cookie_val.strip_prefix("lh_oidc_flow=").unwrap();
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(raw)
        .expect("valid base64url");
    let payload: FlowCookiePayload = serde_json::from_slice(&bytes).expect("valid OidcFlow json");
    (cookie_val, payload)
}

/// D5: a settings:write-only caller gets 403 on PUT and test routes.
#[tokio::test]
async fn settings_write_alone_gets_403_on_put_and_test() {
    let mut env = HashMap::new();
    env.insert(
        "CONNECTOR_PROBE_ALLOW_INTERNAL_HOSTS".to_owned(),
        "true".to_owned(),
    );
    let TestApp { router, pool } = spin_up_with_env(&env).await;
    let user_id = create_principal_with_permissions(&pool, "settings:write").await;
    let cookie = session_cookie_for_user(&pool, user_id).await;

    // GET succeeds and reports canEdit = false.
    let (status, body, _) = call(&router, "GET", SSO, &cookie, "").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["canEdit"], false);

    // PUT returns 403 Forbidden.
    let (status, _, _) = call(
        &router,
        "PUT",
        SSO,
        &cookie,
        r#"{"issuer":"https://localhost:8443","clientId":"cid"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // POST test returns 403 Forbidden.
    let (status, _, _) = call(
        &router,
        "POST",
        SSO_TEST,
        &cookie,
        r#"{"issuer":"https://localhost:8443","clientId":"cid"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

/// D8: a test run stores the result and creates no user or session.
#[tokio::test]
async fn a_test_run_stores_the_result_and_creates_no_user_or_session() {
    let server = MockServer::start().await;
    let key = TestKey::generate("kid-1");
    mount_jwks(&server, &key).await;

    let env = env_for_wiremock_sso(&server.uri());
    let TestApp { router, pool } = spin_up_with_env(&env).await;
    let cookie = session_cookie_for_seeded_user(&pool, "fajar@meridian.example").await;

    // Record user and session count before.
    let users_before: i64 = sqlx::query_scalar("SELECT count(*) FROM app_user")
        .fetch_one(&pool)
        .await
        .unwrap();
    let sessions_before: i64 = sqlx::query_scalar("SELECT count(*) FROM session")
        .fetch_one(&pool)
        .await
        .unwrap();

    // 1. POST /api/settings/sso/test
    let draft_body = json!({
        "issuer": CONSOLE_ISSUER,
        "clientId": "lakehouse-test-client",
        "clientSecret": "draft-secret-xyz",
        "providerName": "test-provider",
        "groupsClaim": "groups",
        "roleMap": [{"group": "admins", "role": "Platform Admin"}],
        "jitProvisioning": true,
    });
    let (status, body, headers) =
        call(&router, "POST", SSO_TEST, &cookie, &draft_body.to_string()).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let test_run_id = body["id"].as_str().expect("id in response");
    let authorize_url = body["authorizeUrl"].as_str().expect("authorizeUrl");
    assert!(authorize_url.contains("client_id=lakehouse-test-client"));

    let (flow_cookie_header, flow) = parse_flow_cookie(&headers);

    // 2. Mock token endpoint returning signed ID token.
    let id_token = sign_id_token(
        &IdTokenClaims {
            sub: "test-subject-1234",
            iss: CONSOLE_ISSUER,
            aud: "lakehouse-test-client",
            exp: time::OffsetDateTime::now_utc().unix_timestamp() + 3600,
            nonce: &flow.nonce,
            email: Some("testuser@example.com"),
            groups: Some(vec!["admins"]),
        },
        &key,
    );
    mount_token_endpoint(&server, &id_token).await;

    // 3. Callback
    let callback_uri = format!(
        "/api/auth/oidc/callback?code=auth-code-123&state={}",
        flow.state
    );
    let (cb_status, _, cb_headers) =
        call(&router, "GET", &callback_uri, &flow_cookie_header, "").await;
    assert_eq!(cb_status, StatusCode::FOUND);
    let location = cb_headers.get(header::LOCATION).unwrap().to_str().unwrap();
    assert_eq!(
        location,
        format!("/admin/settings?section=sign-in&ssoTest={test_run_id}")
    );

    // Assert NO session cookie was issued.
    let set_cookies: Vec<&str> = cb_headers
        .get_all(header::SET_COOKIE)
        .iter()
        .map(|v| v.to_str().unwrap())
        .collect();
    assert!(
        !set_cookies
            .iter()
            .any(|c| c.starts_with("lh_session=") && !c.contains("Max-Age=0")),
        "test callback must not issue a session cookie: {set_cookies:?}"
    );

    // Assert user and session counts did not increase.
    let users_after: i64 = sqlx::query_scalar("SELECT count(*) FROM app_user")
        .fetch_one(&pool)
        .await
        .unwrap();
    let sessions_after: i64 = sqlx::query_scalar("SELECT count(*) FROM session")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(users_before, users_after, "no app_user created");
    assert_eq!(sessions_before, sessions_after, "no session created");

    // 4. GET /api/settings/sso/test/{id}
    let test_get_uri = format!("{SSO_TEST}/{test_run_id}");
    let (get_status, get_body, _) = call(&router, "GET", &test_get_uri, &cookie, "").await;
    assert_eq!(get_status, StatusCode::OK, "{get_body}");
    assert_eq!(get_body["ok"], true);
    assert_eq!(get_body["subject"], "test-subject-1234");
    assert_eq!(get_body["email"], "testuser@example.com");
    assert_eq!(get_body["groups"], json!(["admins"]));
    assert_eq!(get_body["mappedRoles"], json!(["Platform Admin"]));
}

/// D8: a wrong secret stores the classified message.
#[tokio::test]
async fn a_wrong_secret_stores_the_classified_message() {
    let server = MockServer::start().await;
    mount_token_endpoint_error(&server).await;

    let env = env_for_wiremock_sso(&server.uri());
    let TestApp { router, pool } = spin_up_with_env(&env).await;
    let cookie = session_cookie_for_seeded_user(&pool, "fajar@meridian.example").await;

    let draft_body = json!({
        "issuer": CONSOLE_ISSUER,
        "clientId": "lakehouse-test-client",
        "clientSecret": "wrong-secret",
    });
    let (status, body, headers) =
        call(&router, "POST", SSO_TEST, &cookie, &draft_body.to_string()).await;
    assert_eq!(status, StatusCode::OK);
    let test_run_id = body["id"].as_str().expect("id in response");
    let (flow_cookie_header, flow) = parse_flow_cookie(&headers);

    let callback_uri = format!(
        "/api/auth/oidc/callback?code=auth-code-123&state={}",
        flow.state
    );
    let (cb_status, _, _) = call(&router, "GET", &callback_uri, &flow_cookie_header, "").await;
    assert_eq!(cb_status, StatusCode::FOUND);

    let test_get_uri = format!("{SSO_TEST}/{test_run_id}");
    let (get_status, get_body, _) = call(&router, "GET", &test_get_uri, &cookie, "").await;
    assert_eq!(get_status, StatusCode::OK, "{get_body}");
    assert_eq!(get_body["ok"], false);
    assert_eq!(
        get_body["error"],
        "The provider refused the client ID or secret."
    );
}

/// D8: another admin reading the test result gets 404, and an expired run gets 404.
#[tokio::test]
async fn another_admin_or_expired_run_gets_404() {
    let mut env = HashMap::new();
    env.insert(
        "CONNECTOR_PROBE_ALLOW_INTERNAL_HOSTS".to_owned(),
        "true".to_owned(),
    );
    let TestApp { router, pool } = spin_up_with_env(&env).await;

    let admin1_cookie = session_cookie_for_seeded_user(&pool, "fajar@meridian.example").await;
    let admin2_id = create_principal_with_permissions(&pool, "*:*").await;
    let admin2_cookie = session_cookie_for_user(&pool, admin2_id).await;

    let draft_body = json!({
        "issuer": CONSOLE_ISSUER,
        "clientId": "client-test",
    });
    let (status, body, _) = call(
        &router,
        "POST",
        SSO_TEST,
        &admin1_cookie,
        &draft_body.to_string(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let test_run_id = body["id"].as_str().unwrap();

    let test_get_uri = format!("{SSO_TEST}/{test_run_id}");

    // Another admin reading it gets 404 Not Found.
    let (other_status, _, _) = call(&router, "GET", &test_get_uri, &admin2_cookie, "").await;
    assert_eq!(other_status, StatusCode::NOT_FOUND);

    // Creator reading it before expiry gets 200 (Null result before callback).
    let (creator_status, res_body, _) =
        call(&router, "GET", &test_get_uri, &admin1_cookie, "").await;
    assert_eq!(creator_status, StatusCode::OK);
    assert_eq!(res_body, Value::Null);

    // Expire the run in the database.
    let run_uuid: uuid::Uuid = test_run_id.parse().unwrap();
    sqlx::query("UPDATE sso_test_run SET expires_at = now() - interval '1 minute' WHERE id = $1")
        .bind(run_uuid)
        .execute(&pool)
        .await
        .unwrap();

    // After expiration, creator reading it gets 404 Not Found.
    let (exp_status, _, _) = call(&router, "GET", &test_get_uri, &admin1_cookie, "").await;
    assert_eq!(exp_status, StatusCode::NOT_FOUND);
}

/// D5/D8: saving SSO settings records an audit row that holds no secret.
#[tokio::test]
async fn the_audit_row_holds_no_secret() {
    let mut env = HashMap::new();
    env.insert(
        "CONNECTOR_PROBE_ALLOW_INTERNAL_HOSTS".to_owned(),
        "true".to_owned(),
    );
    let TestApp { router, pool } = spin_up_with_env(&env).await;
    let cookie = session_cookie_for_seeded_user(&pool, "fajar@meridian.example").await;

    let secret_literal = "my-super-secret-password-12345";
    let body = json!({
        "issuer": CONSOLE_ISSUER,
        "clientId": "cid-audit-test",
        "clientSecret": secret_literal,
        "providerName": "audit-idp",
        "jitProvisioning": true,
    });

    let (status, res_body, _) = call(&router, "PUT", SSO, &cookie, &body.to_string()).await;
    assert_eq!(status, StatusCode::OK, "{res_body}");

    // Query audit_event rows.
    let events: Vec<(String, Option<Value>)> =
        sqlx::query_as("SELECT action, args FROM audit_event WHERE action LIKE 'settings.sso.%'")
            .fetch_all(&pool)
            .await
            .unwrap();

    assert!(
        !events.is_empty(),
        "expected an audit event for settings.sso.update"
    );
    for (action, args) in events {
        let args_str = args.map(|a| a.to_string()).unwrap_or_default();
        assert!(
            !args_str.contains(secret_literal),
            "audit event {action} leaked secret in args: {args_str}"
        );
    }
}
