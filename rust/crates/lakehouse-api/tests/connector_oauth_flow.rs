//! `SRC-12` part C (decisions 6 to 8, 11, 12): a REST connector signs in with
//! OAuth authorization code and PKCE, keeps a refresh token sealed, survives
//! its rotation, and signs out.
//!
//! The provider is a `wiremock` server standing in for an identity provider
//! (its token endpoint checks the PKCE verifier against the challenge the
//! sign-in address carried, and can be told to refuse, rotate or issue no
//! refresh token); the connector's REST API is the same server, which requires
//! the bearer token. What is proven, each by a request to the real router:
//!
//! - the sign-in address carries `S256`, a random `state`, the operator's
//!   callback address and the connector's scopes; the `state` is stored only as
//!   its hash and the verifier never leaves the table;
//! - the callback refuses an unknown, an expired and a replayed `state`, a
//!   different signed-in user, and a provider error, each with a fixed page,
//!   with the provider's token endpoint never called and nothing stored;
//! - a good callback stores the refresh token sealed (a plain read of the file
//!   does not show it) and no access token anywhere;
//! - a provider that refuses the exchange changes nothing and its words never
//!   reach the page;
//! - the connection test performs the refresh, and a refresh token the
//!   provider rotates during it is stored (the test would otherwise sign the
//!   connector out);
//! - the orchestrator's report replaces the refresh token, and only the
//!   orchestrator may send it, and only for an OAuth connector;
//! - signing out and deleting the connector call the revocation address and
//!   remove the token;
//! - an `http` or private token address, and a moved token address, are
//!   refused at save time.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use base64::Engine as _;
use lakehouse_auth::Secret;
use lakehouse_store::connectors::{CredentialKind, CredentialSource, derive_secret_ref};
use serde_json::{Value, json};
use sha2::Digest as _;
use tower::ServiceExt;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request as WireRequest, ResponseTemplate};

use common::{
    TestApp, create_principal_with_permissions, session_cookie_for_seeded_user,
    session_cookie_for_user, spin_up_with_env, stored_credential,
};

const MERIDIAN_GROUP: &str = "11111111-1111-4111-8111-000000000001";
const ID: &str = "conn-src12-oauth";
const CALLBACK: &str = "https://console.test/api/connectors/oauth/callback";
const CLIENT_SECRET: &str = "client-secret-1";
const PROVIDER_WORDS: &str = "PROVIDER-WORDS-do-not-leak";

/// What the provider's token endpoint does next.
#[derive(Clone)]
enum Mode {
    /// Issue this refresh token on a code exchange; on a refresh, rotate it
    /// to the next one (`refresh-1` -> `refresh-2` -> ...).
    Issue,
    Refuse,
    NoRefreshToken,
}

/// The provider's memory: the challenge it was sent, the refresh token it
/// currently honours, how many times each endpoint was called.
struct Provider {
    mode: Mode,
    challenge: Option<String>,
    current_refresh: Option<String>,
    issued: u32,
    token_calls: u32,
    revoked: Vec<String>,
    last_access: String,
}

struct Fixture {
    app: TestApp,
    cookie: String,
    secrets: tempfile::TempDir,
    server: MockServer,
    provider: Arc<Mutex<Provider>>,
}

fn s256(verifier: &str) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(sha2::Sha256::digest(verifier))
}

fn form(request: &WireRequest) -> HashMap<String, String> {
    url::form_urlencoded::parse(&request.body)
        .into_owned()
        .collect()
}

async fn provider_server(provider: &Arc<Mutex<Provider>>) -> MockServer {
    let server = MockServer::start().await;
    let state = Arc::clone(provider);
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(move |request: &WireRequest| {
            let mut p = state.lock().unwrap();
            p.token_calls += 1;
            let refuse = || {
                ResponseTemplate::new(400).set_body_json(json!({
                    "error": "invalid_grant", "error_description": PROVIDER_WORDS
                }))
            };
            if matches!(p.mode, Mode::Refuse) {
                return refuse();
            }
            let form = form(request);
            if form.get("client_id").map(String::as_str) != Some("app-1")
                || form.get("client_secret").map(String::as_str) != Some(CLIENT_SECRET)
            {
                return refuse();
            }
            match form.get("grant_type").map(String::as_str) {
                Some("authorization_code") => {
                    let verified = form.get("code").map(String::as_str) == Some("CODE-1")
                        && form.get("redirect_uri").map(String::as_str) == Some(CALLBACK)
                        && form
                            .get("code_verifier")
                            .is_some_and(|v| Some(s256(v)) == p.challenge);
                    if !verified {
                        return refuse();
                    }
                    p.issued += 1;
                    p.last_access = format!("access-{}", p.issued);
                    if matches!(p.mode, Mode::NoRefreshToken) {
                        return ResponseTemplate::new(200)
                            .set_body_json(json!({ "access_token": p.last_access }));
                    }
                    p.current_refresh = Some(format!("refresh-{}", p.issued));
                    ResponseTemplate::new(200).set_body_json(json!({
                        "access_token": p.last_access, "refresh_token": p.current_refresh
                    }))
                }
                Some("refresh_token") => {
                    if form.get("refresh_token") != p.current_refresh.as_ref() {
                        return refuse();
                    }
                    p.issued += 1;
                    p.last_access = format!("access-{}", p.issued);
                    p.current_refresh = Some(format!("refresh-{}", p.issued));
                    ResponseTemplate::new(200).set_body_json(json!({
                        "access_token": p.last_access, "refresh_token": p.current_refresh
                    }))
                }
                _ => refuse(),
            }
        })
        .mount(&server)
        .await;
    let state = Arc::clone(provider);
    Mock::given(method("POST"))
        .and(path("/revoke"))
        .respond_with(move |request: &WireRequest| {
            let form = form(request);
            state
                .lock()
                .unwrap()
                .revoked
                .push(form.get("token").cloned().unwrap_or_default());
            ResponseTemplate::new(200)
        })
        .mount(&server)
        .await;
    let state = Arc::clone(provider);
    Mock::given(method("GET"))
        .and(path("/api"))
        .respond_with(move |request: &WireRequest| {
            let want = format!("Bearer {}", state.lock().unwrap().last_access);
            let got = request
                .headers
                .get("authorization")
                .and_then(|v| v.to_str().ok())
                .unwrap_or_default()
                .to_owned();
            if got == want {
                ResponseTemplate::new(200).set_body_json(json!([]))
            } else {
                ResponseTemplate::new(401)
            }
        })
        .mount(&server)
        .await;
    server
}

fn dial(server: &MockServer, with_revocation: bool) -> Value {
    let mut auth = json!({
        "type": "oauth2_authorization_code",
        "clientId": "app-1",
        "authorizeUrl": format!("{}/authorize", server.uri()),
        "tokenUrl": format!("{}/token", server.uri()),
        "scopes": ["read", "offline_access"],
    });
    if with_revocation {
        auth["revocationUrl"] = json!(format!("{}/revoke", server.uri()));
    }
    json!({
        "baseUrl": format!("{}/api", server.uri()),
        "auth": auth,
        "pagination": { "type": "none" },
        "endpoints": [{ "path": "/items" }],
    })
}

async fn fixture_with(extra: &[(&str, &str)], with_revocation: bool) -> Fixture {
    let provider = Arc::new(Mutex::new(Provider {
        mode: Mode::Issue,
        challenge: None,
        current_refresh: None,
        issued: 0,
        token_calls: 0,
        revoked: Vec::new(),
        last_access: String::new(),
    }));
    let server = provider_server(&provider).await;
    let secrets = tempfile::tempdir().unwrap();
    let mut env = HashMap::from([
        (
            "CONNECTOR_SECRETS_DIR".to_owned(),
            secrets.path().to_string_lossy().into_owned(),
        ),
        (
            "CONNECTOR_OAUTH_REDIRECT_URI".to_owned(),
            CALLBACK.to_owned(),
        ),
        ("CONNECTOR_OAUTH_ALLOW_HTTP".to_owned(), "true".to_owned()),
        (
            "CONNECTOR_PROBE_ALLOW_INTERNAL_HOSTS".to_owned(),
            "true".to_owned(),
        ),
    ]);
    for (key, value) in extra {
        env.insert((*key).to_owned(), (*value).to_owned());
    }
    let app = spin_up_with_env(&env).await;
    let cookie = session_cookie_for_seeded_user(&app.pool, "bayu@meridian.example").await;
    let secondary = derive_secret_ref(ID, CredentialSource::Managed, CredentialKind::Token);
    sqlx::query(
        "INSERT INTO connector (id, tenant_id, name, type, direction, host, secret_ref, \
         secret_ref_secondary, environment, tenant, adapter, ingest_mode, dial) VALUES ($1, $2, \
         'oauth rest', 'REST API', 'source', 'unused', 'env:CONNECTOR_SRC12_OAUTH_UNSET', $3, \
         'production', 'meridian', 'rest', 'batch', $4)",
    )
    .bind(ID)
    .bind(MERIDIAN_GROUP.parse::<uuid::Uuid>().unwrap())
    .bind(&secondary)
    .bind(dial(&server, with_revocation))
    .execute(&app.pool)
    .await
    .expect("seed an OAuth rest connector");
    Fixture {
        app,
        cookie,
        secrets,
        server,
        provider,
    }
}

async fn fixture() -> Fixture {
    fixture_with(&[], true).await
}

async fn raw(
    app: &TestApp,
    cookie: &str,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, String) {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("cookie", cookie);
    let body = match body {
        Some(body) => {
            builder = builder.header("content-type", "application/json");
            Body::from(body.to_string())
        }
        None => Body::empty(),
    };
    let response = app
        .router
        .clone()
        .oneshot(builder.body(body).expect("build request"))
        .await
        .expect("router never fails a request outright");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

async fn send(fx: &Fixture, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let (status, text) = raw(&fx.app, &fx.cookie, method, uri, body).await;
    (status, serde_json::from_str(&text).unwrap_or(Value::Null))
}

fn url_of(path: &str) -> String {
    format!("/api/connectors/{ID}/{path}")
}

fn token_file() -> String {
    // The file name of the managed refresh token (`…connector_managed_<id>_token`).
    let secondary = derive_secret_ref(ID, CredentialSource::Managed, CredentialKind::Token);
    secondary.rsplit('/').next().unwrap().to_owned()
}

/// Save the client secret, as a person does before the first sign-in.
async fn save_client_secret(fx: &Fixture) {
    let (status, body) = send(
        fx,
        "PUT",
        &url_of("credential"),
        Some(json!({ "primary": { "kind": "secret_key", "value": CLIENT_SECRET } })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    // Nobody has signed in, so there was nothing to test: honest, not faked.
    assert_eq!(body["verified"], false, "{body}");
}

/// Start a sign-in as `fx`'s user and return `(state, authorize_url)`.
async fn start(fx: &Fixture, cookie: &str) -> (String, url::Url) {
    let (status, text) = raw(&fx.app, cookie, "POST", &url_of("oauth/start"), None).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    let body: Value = serde_json::from_str(&text).unwrap();
    let url = url::Url::parse(body["authorizeUrl"].as_str().unwrap()).unwrap();
    let state = url
        .query_pairs()
        .find(|(k, _)| k == "state")
        .unwrap()
        .1
        .into_owned();
    let challenge = url
        .query_pairs()
        .find(|(k, _)| k == "code_challenge")
        .unwrap()
        .1
        .into_owned();
    fx.provider.lock().unwrap().challenge = Some(challenge);
    (state, url)
}

async fn callback(fx: &Fixture, cookie: &str, query: &str) -> (StatusCode, String) {
    raw(
        &fx.app,
        cookie,
        "GET",
        &format!("/api/connectors/oauth/callback?{query}"),
        None,
    )
    .await
}

fn token_calls(fx: &Fixture) -> u32 {
    fx.provider.lock().unwrap().token_calls
}

fn stored_files(fx: &Fixture) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(fx.secrets.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

async fn sign_in(fx: &Fixture) {
    let (state, _) = start(fx, &fx.cookie).await;
    let (status, page) = callback(fx, &fx.cookie, &format!("code=CODE-1&state={state}")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(page.contains("Signed in"), "{page}");
}

#[tokio::test]
async fn the_sign_in_address_carries_s256_a_random_state_and_the_operators_callback() {
    let fx = fixture().await;
    // No client secret yet: refused before the person is sent anywhere.
    let (status, _) = send(&fx, "POST", &url_of("oauth/start"), None).await;
    assert_eq!(status, StatusCode::CONFLICT);

    save_client_secret(&fx).await;
    let (state, url) = start(&fx, &fx.cookie).await;
    let pairs: HashMap<String, String> = url.query_pairs().into_owned().collect();
    assert_eq!(url.path(), "/authorize");
    assert_eq!(pairs["response_type"], "code");
    assert_eq!(pairs["client_id"], "app-1");
    assert_eq!(pairs["redirect_uri"], CALLBACK);
    assert_eq!(pairs["code_challenge_method"], "S256");
    assert_eq!(pairs["scope"], "read offline_access");
    assert!(state.len() >= 32, "at least 128 bits of state: {state}");
    assert!(!url.as_str().contains("code_verifier"));
    assert!(!url.as_str().contains(CLIENT_SECRET));

    // Stored: the hash of the state, the connector, the user, the verifier that
    // matches the challenge; never the state itself.
    let (hash, connector, verifier): (String, String, String) =
        sqlx::query_as("SELECT state_hash, connector_id, code_verifier FROM connector_oauth_state")
            .fetch_one(&fx.app.pool)
            .await
            .unwrap();
    assert_ne!(hash, state);
    assert_eq!(connector, ID);
    assert_eq!(s256(&verifier), pairs["code_challenge"]);
    assert_ne!(verifier, state);

    // A second start gives a different state.
    let (second, _) = start(&fx, &fx.cookie).await;
    assert_ne!(second, state);
}

#[tokio::test]
async fn the_status_names_the_callback_to_copy_and_whether_the_provider_can_be_told() {
    let with = fixture().await;
    let (status, body) = send(&with, "GET", &url_of("oauth"), None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["signedIn"], false);
    assert_eq!(body["callbackUrl"], CALLBACK);
    assert_eq!(body["revocationConfigured"], true);

    let without = fixture_with(&[], false).await;
    let (_, body) = send(&without, "GET", &url_of("oauth"), None).await;
    assert_eq!(body["revocationConfigured"], false);

    let (status, settings) = send(&with, "GET", "/api/connectors/credential-settings", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(settings["oauthCallbackUrl"], CALLBACK);
}

#[tokio::test]
async fn without_the_operators_callback_address_a_sign_in_is_refused() {
    let fx = fixture_with(&[("CONNECTOR_OAUTH_REDIRECT_URI", "")], true).await;
    save_client_secret(&fx).await;
    let (status, body) = send(&fx, "POST", &url_of("oauth/start"), None).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert!(body.to_string().contains("CONNECTOR_OAUTH_REDIRECT_URI"));
}

#[tokio::test]
async fn a_good_callback_stores_the_refresh_token_sealed_and_nothing_else() {
    let fx = fixture().await;
    save_client_secret(&fx).await;
    let before = stored_files(&fx);

    let (state, _) = start(&fx, &fx.cookie).await;
    let (status, page) = callback(&fx, &fx.cookie, &format!("code=CODE-1&state={state}")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(page.contains("Signed in"), "{page}");
    assert!(
        !page.contains("refresh-1") && !page.contains("access-1"),
        "{page}"
    );
    assert_eq!(token_calls(&fx), 1);

    // Sealed under the installation key, and not readable as text.
    let file = token_file();
    assert_eq!(stored_credential(fx.secrets.path(), &file), "refresh-1");
    let on_disk = std::fs::read(fx.secrets.path().join(&file)).unwrap();
    assert!(
        !String::from_utf8_lossy(&on_disk).contains("refresh-1"),
        "the refresh token must not be on disk as text"
    );
    // The client secret file and the refresh token file, nothing for an access token.
    let after = stored_files(&fx);
    assert_eq!(after.len(), before.len() + 1, "{after:?}");
    assert!(after.contains(&file));
    for name in &after {
        let bytes = std::fs::read(fx.secrets.path().join(name)).unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains("access-1"));
    }

    let (_, status) = send(&fx, "GET", &url_of("oauth"), None).await;
    assert_eq!(status["signedIn"], true);

    // The state is spent: the same link again is refused and calls nothing.
    let (_, again) = callback(&fx, &fx.cookie, &format!("code=CODE-1&state={state}")).await;
    assert!(
        again.contains("unknown, has expired or was already used"),
        "{again}"
    );
    assert_eq!(token_calls(&fx), 1);

    // Audited without a token, a state or a code.
    let (action, args): (String, Value) = sqlx::query_as(
        "SELECT action, args FROM audit_event WHERE action = 'connector.oauth_signed_in' \
         AND resource_id = $1",
    )
    .bind(ID)
    .fetch_one(&fx.app.pool)
    .await
    .unwrap();
    assert_eq!(action, "connector.oauth_signed_in");
    let text = args.to_string();
    assert!(!text.contains("refresh-1") && !text.contains(&state) && !text.contains("CODE-1"));
}

#[tokio::test]
async fn an_unknown_or_missing_state_is_refused_and_calls_the_provider_for_nothing() {
    let fx = fixture().await;
    save_client_secret(&fx).await;
    let before = stored_files(&fx);
    for query in [
        "code=CODE-1&state=not-a-state-we-issued",
        "code=CODE-1",
        "state=",
        "",
    ] {
        let (status, page) = callback(&fx, &fx.cookie, query).await;
        assert_eq!(status, StatusCode::OK, "{query}");
        assert!(page.contains("Sign-in not completed"), "{query}: {page}");
    }
    assert_eq!(token_calls(&fx), 0);
    assert_eq!(stored_files(&fx), before);
}

#[tokio::test]
async fn an_expired_state_is_refused_and_stores_nothing() {
    let fx = fixture().await;
    save_client_secret(&fx).await;
    let before = stored_files(&fx);
    let (state, _) = start(&fx, &fx.cookie).await;
    sqlx::query("UPDATE connector_oauth_state SET expires_at = now() - interval '1 second'")
        .execute(&fx.app.pool)
        .await
        .unwrap();
    let (_, page) = callback(&fx, &fx.cookie, &format!("code=CODE-1&state={state}")).await;
    assert!(
        page.contains("unknown, has expired or was already used"),
        "{page}"
    );
    assert_eq!(token_calls(&fx), 0);
    assert_eq!(stored_files(&fx), before);
}

#[tokio::test]
async fn a_different_signed_in_user_is_refused_and_the_state_is_burned() {
    let fx = fixture().await;
    save_client_secret(&fx).await;
    let before = stored_files(&fx);
    let (state, _) = start(&fx, &fx.cookie).await;

    // Another person of the same tenant, with the same permission.
    let other = create_principal_with_permissions(&fx.app.pool, "connector:manage").await;
    sqlx::query("INSERT INTO app_user_tenant (user_id, tenant_id) VALUES ($1, $2)")
        .bind(other)
        .bind(MERIDIAN_GROUP.parse::<uuid::Uuid>().unwrap())
        .execute(&fx.app.pool)
        .await
        .unwrap();
    let other_cookie = session_cookie_for_user(&fx.app.pool, other).await;
    let (_, page) = callback(&fx, &other_cookie, &format!("code=CODE-1&state={state}")).await;
    assert!(page.contains("started by someone else"), "{page}");
    assert_eq!(token_calls(&fx), 0);
    assert_eq!(stored_files(&fx), before);

    // Having been presented once, the state does not work for its owner either.
    let (_, page) = callback(&fx, &fx.cookie, &format!("code=CODE-1&state={state}")).await;
    assert!(
        page.contains("unknown, has expired or was already used"),
        "{page}"
    );
    assert_eq!(token_calls(&fx), 0);
    assert_eq!(stored_files(&fx), before);
}

#[tokio::test]
async fn a_user_who_lost_the_tenant_or_the_permission_is_refused() {
    let fx = fixture().await;
    save_client_secret(&fx).await;
    let before = stored_files(&fx);
    let (state, _) = start(&fx, &fx.cookie).await;
    sqlx::query(
        "DELETE FROM app_user_tenant WHERE user_id = (SELECT id FROM app_user WHERE email = \
         'bayu@meridian.example')",
    )
    .execute(&fx.app.pool)
    .await
    .unwrap();
    // A new session picks up the changed membership.
    let cookie = session_cookie_for_seeded_user(&fx.app.pool, "bayu@meridian.example").await;
    let (_, page) = callback(&fx, &cookie, &format!("code=CODE-1&state={state}")).await;
    assert!(
        page.contains("can no longer manage this connector"),
        "{page}"
    );
    assert_eq!(token_calls(&fx), 0);
    assert_eq!(stored_files(&fx), before);
}

#[tokio::test]
async fn a_provider_error_is_refused_without_its_words_and_stores_nothing() {
    let fx = fixture().await;
    save_client_secret(&fx).await;
    let before = stored_files(&fx);
    let (state, _) = start(&fx, &fx.cookie).await;
    let (status, page) = callback(
        &fx,
        &fx.cookie,
        &format!("error=access_denied&error_description={PROVIDER_WORDS}&state={state}"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(page.contains("did not complete the sign-in"), "{page}");
    assert!(
        !page.contains(PROVIDER_WORDS) && !page.contains("access_denied"),
        "{page}"
    );
    assert_eq!(token_calls(&fx), 0);
    assert_eq!(stored_files(&fx), before);
}

#[tokio::test]
async fn a_provider_that_refuses_the_exchange_changes_nothing_and_its_words_stay_out() {
    let fx = fixture().await;
    save_client_secret(&fx).await;
    let before = stored_files(&fx);
    fx.provider.lock().unwrap().mode = Mode::Refuse;
    let (state, _) = start(&fx, &fx.cookie).await;
    let (_, page) = callback(&fx, &fx.cookie, &format!("code=CODE-1&state={state}")).await;
    assert!(page.contains("Sign-in not completed"), "{page}");
    assert!(page.contains("Nothing was saved"), "{page}");
    assert!(
        !page.contains(PROVIDER_WORDS) && !page.contains("invalid_grant"),
        "{page}"
    );
    assert_eq!(token_calls(&fx), 1);
    assert_eq!(stored_files(&fx), before);
}

#[tokio::test]
async fn a_provider_that_issues_no_refresh_token_stores_nothing() {
    let fx = fixture().await;
    save_client_secret(&fx).await;
    let before = stored_files(&fx);
    fx.provider.lock().unwrap().mode = Mode::NoRefreshToken;
    let (state, _) = start(&fx, &fx.cookie).await;
    let (_, page) = callback(&fx, &fx.cookie, &format!("code=CODE-1&state={state}")).await;
    assert!(page.contains("did not issue a refresh token"), "{page}");
    assert_eq!(stored_files(&fx), before);
}

#[tokio::test]
async fn the_test_refreshes_and_keeps_a_refresh_token_the_provider_rotated() {
    let fx = fixture().await;
    save_client_secret(&fx).await;

    // Before any sign-in: honest, nothing dialled.
    let (status, body) = send(&fx, "POST", &url_of("test"), None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["supported"], false);
    assert_eq!(token_calls(&fx), 0);

    sign_in(&fx).await;
    assert_eq!(
        stored_credential(fx.secrets.path(), &token_file()),
        "refresh-1"
    );

    // The test refreshes (the provider rotates to refresh-2) and calls the API
    // with the new access token.
    let (status, body) = send(&fx, "POST", &url_of("test"), None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["ok"], true, "{body}");
    assert_eq!(
        stored_credential(fx.secrets.path(), &token_file()),
        "refresh-2",
        "the rotated refresh token must be kept"
    );
    // A second test only works because the new token was kept.
    let (_, body) = send(&fx, "POST", &url_of("test"), None).await;
    assert_eq!(body["ok"], true, "{body}");
    assert_eq!(
        stored_credential(fx.secrets.path(), &token_file()),
        "refresh-3"
    );
    assert!(!body.to_string().contains("refresh-"), "{body}");
}

#[tokio::test]
async fn a_refused_refresh_fails_the_test_with_a_fixed_text() {
    let fx = fixture().await;
    save_client_secret(&fx).await;
    sign_in(&fx).await;
    fx.provider.lock().unwrap().mode = Mode::Refuse;
    let (status, body) = send(&fx, "POST", &url_of("test"), None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["ok"], false);
    let text = body.to_string();
    assert!(text.contains("sign in again"), "{text}");
    assert!(!text.contains(PROVIDER_WORDS));
    // Nothing was thrown away: the stored token is still there to retry.
    assert_eq!(
        stored_credential(fx.secrets.path(), &token_file()),
        "refresh-1"
    );
}

async fn ingest_service_bearer(pool: &sqlx::PgPool) -> String {
    const TOKEN: &str = "ingest-service-fixture-token";
    let identity = lakehouse_store::identity::create_service_identity(
        pool,
        &lakehouse_store::identity::CreateServiceIdentityInput {
            name: "test-ingest-service".to_owned(),
            scopes: vec!["ingest:read".to_owned()],
            environment: "test".to_owned(),
        },
    )
    .await
    .unwrap();
    lakehouse_auth::service_token::ensure_service_credential(
        pool,
        identity.id.parse().unwrap(),
        &Secret::new(TOKEN.to_owned()),
    )
    .await
    .unwrap();
    format!("Bearer {TOKEN}")
}

async fn report(app: &TestApp, bearer: &str, id: &str, body: Value) -> (StatusCode, String) {
    let response = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/connectors/{id}/oauth/refresh-token"))
                .header("authorization", bearer)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

#[tokio::test]
async fn the_orchestrator_replaces_the_refresh_token_and_only_it_and_only_for_oauth() {
    let fx = fixture().await;
    save_client_secret(&fx).await;
    sign_in(&fx).await;
    let bearer = ingest_service_bearer(&fx.app.pool).await;

    let (status, text) = report(
        &fx.app,
        &bearer,
        ID,
        json!({ "refreshToken": "rotated-by-run" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{text}");
    assert!(!text.contains("rotated-by-run"));
    assert_eq!(
        stored_credential(fx.secrets.path(), &token_file()),
        "rotated-by-run"
    );
    let on_disk = std::fs::read(fx.secrets.path().join(token_file())).unwrap();
    assert!(!String::from_utf8_lossy(&on_disk).contains("rotated-by-run"));

    // A connector that is not an OAuth one: refused.
    let (status, _) = report(
        &fx.app,
        &bearer,
        "conn-pg-lakehouse",
        json!({ "refreshToken": "x-token" }),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    // An unknown connector, a body with another field, an unusable token.
    let (status, _) = report(&fx.app, &bearer, "nope", json!({ "refreshToken": "x" })).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = report(
        &fx.app,
        &bearer,
        ID,
        json!({ "refreshToken": "x", "other": 1 }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = report(&fx.app, &bearer, ID, json!({ "refreshToken": "  " })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        stored_credential(fx.secrets.path(), &token_file()),
        "rotated-by-run",
        "a refused report changes nothing"
    );

    // A person, even one with `ingest:read`, is not the orchestrator.
    let (status, text) = raw(
        &fx.app,
        &fx.cookie,
        "POST",
        &url_of("oauth/refresh-token"),
        Some(json!({ "refreshToken": "from-a-person" })),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{text}");
    assert_eq!(
        stored_credential(fx.secrets.path(), &token_file()),
        "rotated-by-run"
    );
}

#[tokio::test]
async fn signing_out_revokes_at_the_provider_and_removes_the_token() {
    let fx = fixture().await;
    save_client_secret(&fx).await;
    sign_in(&fx).await;

    let (status, body) = send(&fx, "POST", &url_of("oauth/sign-out"), None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["signedOut"], true);
    assert_eq!(body["revocation"], "done");
    assert_eq!(
        fx.provider.lock().unwrap().revoked,
        vec!["refresh-1".to_owned()]
    );
    assert!(!stored_files(&fx).contains(&token_file()));
    let (_, status_body) = send(&fx, "GET", &url_of("oauth"), None).await;
    assert_eq!(status_body["signedIn"], false);

    // Nothing left to revoke the second time.
    let (_, body) = send(&fx, "POST", &url_of("oauth/sign-out"), None).await;
    assert_eq!(body["revocation"], "noToken");
}

#[tokio::test]
async fn without_a_revocation_address_signing_out_says_the_provider_was_not_told() {
    let fx = fixture_with(&[], false).await;
    save_client_secret(&fx).await;
    sign_in(&fx).await;
    let (status, body) = send(&fx, "POST", &url_of("oauth/sign-out"), None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["revocation"], "notConfigured");
    assert!(body["message"].as_str().unwrap().contains("stays valid"));
    assert!(fx.provider.lock().unwrap().revoked.is_empty());
    assert!(!stored_files(&fx).contains(&token_file()));
}

#[tokio::test]
async fn deleting_the_connector_revokes_and_removes_every_credential_file() {
    let fx = fixture().await;
    save_client_secret(&fx).await;
    sign_in(&fx).await;
    let (status, _) = raw(
        &fx.app,
        &fx.cookie,
        "DELETE",
        &format!("/api/connectors/{ID}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(
        fx.provider.lock().unwrap().revoked,
        vec!["refresh-1".to_owned()]
    );
    assert!(stored_files(&fx).is_empty(), "{:?}", stored_files(&fx));
    let (details,): (Value,) = sqlx::query_as(
        "SELECT args FROM audit_event WHERE action = 'connector.delete' AND resource_id = $1",
    )
    .bind(ID)
    .fetch_one(&fx.app.pool)
    .await
    .unwrap();
    assert_eq!(details["oauthRevocation"], "done");
}

fn spec_body(dial: &Value) -> Value {
    json!({
        "adapter": "rest", "ingestMode": "batch", "dial": dial, "sourceObjects": [],
    })
}

#[tokio::test]
async fn an_http_or_private_token_address_is_refused_at_save() {
    // Production posture: no gate switch, no internal hosts. The dial uses public
    // address literals, so the check needs no DNS and a good dial is accepted.
    let fx = fixture_with(
        &[
            ("CONNECTOR_OAUTH_ALLOW_HTTP", "false"),
            ("CONNECTOR_PROBE_ALLOW_INTERNAL_HOSTS", "false"),
        ],
        true,
    )
    .await;
    let good = json!({
        "baseUrl": "https://93.184.216.34/api",
        "auth": {
            "type": "oauth2_authorization_code",
            "clientId": "app-1",
            "authorizeUrl": "https://login.example.com/authorize",
            "tokenUrl": "https://93.184.216.35/token",
            "revocationUrl": "https://93.184.216.36/revoke",
            "scopes": ["read"],
        },
        "pagination": { "type": "none" },
        "endpoints": [{ "path": "/items" }],
    });
    sqlx::query("UPDATE connector SET dial = $2 WHERE id = $1")
        .bind(ID)
        .bind(&good)
        .execute(&fx.app.pool)
        .await
        .unwrap();
    // The positive control: the same dial, unchanged, saves.
    let (status, body) = send(&fx, "PUT", &url_of("ingest-spec"), Some(spec_body(&good))).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    for (field, value) in [
        ("tokenUrl", "http://93.184.216.35/token"),
        ("revocationUrl", "http://93.184.216.36/revoke"),
        ("authorizeUrl", "http://login.example.com/authorize"),
        ("tokenUrl", "https://127.0.0.1/token"),
        ("revocationUrl", "https://localhost/revoke"),
        ("tokenUrl", "https://10.0.0.5/token"),
    ] {
        let mut bad = good.clone();
        bad["auth"][field] = json!(value);
        let (status, body) = send(&fx, "PUT", &url_of("ingest-spec"), Some(spec_body(&bad))).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{field} {value}: {body}");
    }
}

#[tokio::test]
async fn moving_the_token_address_to_another_host_needs_the_credentials_again() {
    let fx = fixture().await;
    save_client_secret(&fx).await;
    sign_in(&fx).await;
    let mut moved = dial(&fx.server, true);
    moved["auth"]["tokenUrl"] = json!("http://127.0.0.2:9/token");
    let (status, body) = send(&fx, "PUT", &url_of("ingest-spec"), Some(spec_body(&moved))).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    // The stored refresh token still goes only where it was stored for.
    assert_eq!(
        stored_credential(fx.secrets.path(), &token_file()),
        "refresh-1"
    );

    // The same host with another path is not a move.
    let mut same_host = dial(&fx.server, true);
    same_host["auth"]["tokenUrl"] = json!(format!("{}/v2/token", fx.server.uri()));
    let (status, body) = send(
        &fx,
        "PUT",
        &url_of("ingest-spec"),
        Some(spec_body(&same_host)),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
}
