//! `SRC-12` part B (decisions 1, 2, 9, 11, 12): a connector's credential can
//! be a REFERENCE to a secret in the installation's secret manager.
//!
//! The manager here is a `wiremock` stand-in for Vault's KV v2 interface (the
//! API's client is the same code whichever manager is set); the connector is
//! an Oracle one, which the console cannot probe (`SRC-6` F3), so the route
//! reaches the point where the reference has been read and nothing is
//! dialled: `PUT .../credential` saves it with `verified: false`. What is
//! proven:
//!
//! - the reference is read once, before anything is saved, and only the
//!   reference lands in the connector row; no file is written;
//! - a reference to nothing, or one the product may not read, is refused with
//!   the fixed text and none of the manager's words, and changes nothing;
//! - a reference needs a secret manager, a typed value can be turned off, and
//!   a reference of another manager's scheme is refused;
//! - the audit event names the reference and the manager but never the value;
//! - `GET .../credential-settings` says what the form may offer.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::collections::HashMap;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use tower::ServiceExt;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use common::{TestApp, session_cookie_for_seeded_user, spin_up_with_env};

const MERIDIAN_GROUP: &str = "11111111-1111-4111-8111-000000000001";
const ORACLE_ID: &str = "conn-src12-ref";
const REFERENCE: &str = "vault:secret/connectors/orders#password";
/// A value that must appear nowhere but in the stand-in manager.
const SECRET_VALUE: &str = "value-held-only-by-the-manager";

async fn send(
    app: &TestApp,
    cookie: &str,
    method: &str,
    path: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
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
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

async fn vault_with_secret() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/secret/data/connectors/orders"))
        .and(header("X-Vault-Token", "ci-vault-token-not-real"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": {"data": {"password": SECRET_VALUE}, "metadata": {"version": 1}}
        })))
        .mount(&server)
        .await;
    server
}

fn env(
    secrets: &std::path::Path,
    vault: Option<&MockServer>,
    extra: &[(&str, &str)],
) -> HashMap<String, String> {
    let mut env = HashMap::from([(
        "CONNECTOR_SECRETS_DIR".to_owned(),
        secrets.to_string_lossy().into_owned(),
    )]);
    if let Some(vault) = vault {
        env.insert("SECRET_MANAGER".to_owned(), "vault".to_owned());
        env.insert("VAULT_ADDR".to_owned(), vault.uri());
        env.insert(
            "VAULT_TOKEN".to_owned(),
            "ci-vault-token-not-real".to_owned(),
        );
        // `SRC-12 review B4 BLOCKER 1`: references are accepted only under an
        // operator-allowed prefix (fail closed), so the app under test is
        // given the one that covers `REFERENCE`; a test overrides it via `extra`.
        env.insert(
            "SECRET_MANAGER_ALLOWED_PREFIXES".to_owned(),
            "vault:secret/connectors/".to_owned(),
        );
    }
    for (key, value) in extra {
        env.insert((*key).to_owned(), (*value).to_owned());
    }
    env
}

async fn seed_oracle(app: &TestApp) {
    sqlx::query(
        "INSERT INTO connector (id, tenant_id, name, type, direction, health, host, secret_ref, \
         environment, tenant, adapter, ingest_mode, dial) VALUES ($1, $2, 'src12 reference', \
         'Oracle', 'source', 'degraded', 'unused', 'env:CONNECTOR_SRC12_REF_PASSWORD', \
         'production', 'meridian', 'sql', 'batch', $3)",
    )
    .bind(ORACLE_ID)
    .bind(MERIDIAN_GROUP.parse::<uuid::Uuid>().unwrap())
    .bind(json!({
        "driver": "oracle", "host": "oracle.invalid", "port": 1521,
        "database": "ORCL", "user": "u", "sslMode": "disable",
    }))
    .execute(&app.pool)
    .await
    .expect("seed an oracle connector");
}

async fn stored_ref(app: &TestApp) -> String {
    sqlx::query_scalar("SELECT secret_ref FROM connector WHERE id = $1")
        .bind(ORACLE_ID)
        .fetch_one(&app.pool)
        .await
        .unwrap()
}

fn credential_url() -> String {
    format!("/api/connectors/{ORACLE_ID}/credential")
}

#[tokio::test]
async fn a_reference_is_read_saved_as_a_reference_and_leaves_no_copy() {
    let secrets = tempfile::tempdir().unwrap();
    let vault = vault_with_secret().await;
    let app = spin_up_with_env(&env(secrets.path(), Some(&vault), &[])).await;
    let cookie = session_cookie_for_seeded_user(&app.pool, "bayu@meridian.example").await;
    seed_oracle(&app).await;

    let (status, body) = send(
        &app,
        &cookie,
        "PUT",
        &credential_url(),
        Some(json!({ "primary": { "reference": REFERENCE } })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["saved"], true);
    // Oracle cannot be probed from the console: honest, not fabricated.
    assert_eq!(body["verified"], false);
    assert!(!body.to_string().contains(SECRET_VALUE), "{body}");

    assert_eq!(stored_ref(&app).await, REFERENCE);
    assert_eq!(
        std::fs::read_dir(secrets.path()).unwrap().count(),
        0,
        "a reference writes no file"
    );
    let (status, detail) = send(
        &app,
        &cookie,
        "GET",
        &format!("/api/connectors/{ORACLE_ID}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{detail}");
    assert_eq!(detail["credentialSource"], "reference");
    assert_eq!(detail["credentialManaged"], false);
    assert!(!detail.to_string().contains(SECRET_VALUE));

    // Decision 11: the audit event names the reference and the manager, and
    // nothing the manager returned.
    let (action, args): (String, Value) = sqlx::query_as(
        "SELECT action, args FROM audit_event WHERE action = 'connector.credential_set' \
         AND resource_id = $1",
    )
    .bind(ORACLE_ID)
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert_eq!(action, "connector.credential_set");
    assert_eq!(args["sources"], json!(["reference"]));
    assert_eq!(args["references"], json!([REFERENCE]));
    assert_eq!(args["manager"], "vault");
    assert!(!args.to_string().contains(SECRET_VALUE), "{args}");

    // The secret was read from the manager, once, for this request.
    let reads = vault.received_requests().await.unwrap();
    assert_eq!(reads.len(), 1);
}

#[tokio::test]
async fn a_reference_to_nothing_or_one_the_product_may_not_read_is_refused_in_fixed_words() {
    for (status, expected) in [
        (404, "does not exist in the secret manager"),
        (403, "may not read the referenced secret"),
        (500, "secret manager cannot be reached"),
    ] {
        let secrets = tempfile::tempdir().unwrap();
        let vault = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(status)
                    .set_body_json(json!({"errors": ["vault-own-words-about-a-policy"]})),
            )
            .mount(&vault)
            .await;
        let app = spin_up_with_env(&env(secrets.path(), Some(&vault), &[])).await;
        let cookie = session_cookie_for_seeded_user(&app.pool, "bayu@meridian.example").await;
        seed_oracle(&app).await;

        let (code, body) = send(
            &app,
            &cookie,
            "PUT",
            &credential_url(),
            Some(json!({ "primary": { "reference": REFERENCE } })),
        )
        .await;
        assert_eq!(code, StatusCode::UNPROCESSABLE_ENTITY, "{status}: {body}");
        let text = body.to_string();
        assert!(text.contains(expected), "{status}: {text}");
        assert!(!text.contains("vault-own-words"), "{status}: {text}");
        assert_eq!(
            stored_ref(&app).await,
            "env:CONNECTOR_SRC12_REF_PASSWORD",
            "a refused reference changes nothing"
        );
    }
}

#[tokio::test]
async fn a_reference_needs_a_secret_manager_and_its_own_scheme() {
    let secrets = tempfile::tempdir().unwrap();
    // No manager at all.
    let app = spin_up_with_env(&env(secrets.path(), None, &[])).await;
    let cookie = session_cookie_for_seeded_user(&app.pool, "bayu@meridian.example").await;
    seed_oracle(&app).await;
    let (code, body) = send(
        &app,
        &cookie,
        "PUT",
        &credential_url(),
        Some(json!({ "primary": { "reference": REFERENCE } })),
    )
    .await;
    assert_eq!(code, StatusCode::BAD_REQUEST, "{body}");
    assert!(
        body.to_string().contains("No secret manager is set up"),
        "{body}"
    );

    // A manager is set, but the reference is another manager's, or malformed.
    let vault = vault_with_secret().await;
    let app = spin_up_with_env(&env(secrets.path(), Some(&vault), &[])).await;
    let cookie = session_cookie_for_seeded_user(&app.pool, "bayu@meridian.example").await;
    seed_oracle(&app).await;
    for bad in [
        "aws-sm:prod/orders",
        "vault:secret/../etc#password",
        "env:CONNECTOR_X_PASSWORD",
        "vault:secret/orders",
    ] {
        let (code, body) = send(
            &app,
            &cookie,
            "PUT",
            &credential_url(),
            Some(json!({ "primary": { "reference": bad } })),
        )
        .await;
        assert_eq!(code, StatusCode::BAD_REQUEST, "{bad}: {body}");
    }
    assert!(
        vault.received_requests().await.unwrap().is_empty(),
        "a malformed reference makes no call to the manager"
    );
    // Both a value and a reference, or neither.
    for slot in [
        json!({ "kind": "password", "value": "pw", "reference": REFERENCE }),
        json!({ "kind": "password" }),
    ] {
        let (code, body) = send(
            &app,
            &cookie,
            "PUT",
            &credential_url(),
            Some(json!({ "primary": slot })),
        )
        .await;
        assert_eq!(code, StatusCode::BAD_REQUEST, "{body}");
    }
}

/// `SRC-12 review B1 BLOCKER 1`: a user must not be able to make the product
/// read a secret outside the operator's prefixes, and with no prefixes set no
/// reference is accepted. Neither makes a call to the manager.
#[tokio::test]
async fn a_reference_outside_the_operators_prefixes_is_refused_and_never_read() {
    let secrets = tempfile::tempdir().unwrap();
    let vault = vault_with_secret().await;
    let app = spin_up_with_env(&env(secrets.path(), Some(&vault), &[])).await;
    let cookie = session_cookie_for_seeded_user(&app.pool, "bayu@meridian.example").await;
    seed_oracle(&app).await;
    for outside in [
        "vault:secret/other/orders#password",
        "vault:secret/connectors-other/orders#password",
        "vault:secret/connectors/../other/orders#password",
    ] {
        let (code, body) = send(
            &app,
            &cookie,
            "PUT",
            &credential_url(),
            Some(json!({ "primary": { "reference": outside } })),
        )
        .await;
        assert_eq!(code, StatusCode::BAD_REQUEST, "{outside}: {body}");
    }
    let (_, body) = send(
        &app,
        &cookie,
        "PUT",
        &credential_url(),
        Some(json!({ "primary": { "reference": "vault:secret/other/orders#password" } })),
    )
    .await;
    assert_eq!(
        body["error"],
        "That reference is outside the secrets this installation allows connectors to reference"
    );

    let app = spin_up_with_env(&env(
        secrets.path(),
        Some(&vault),
        &[("SECRET_MANAGER_ALLOWED_PREFIXES", "")],
    ))
    .await;
    let cookie = session_cookie_for_seeded_user(&app.pool, "bayu@meridian.example").await;
    seed_oracle(&app).await;
    let (code, body) = send(
        &app,
        &cookie,
        "PUT",
        &credential_url(),
        Some(json!({ "primary": { "reference": REFERENCE } })),
    )
    .await;
    assert_eq!(code, StatusCode::BAD_REQUEST, "{body}");
    assert!(
        body.to_string()
            .contains("allows no secret-manager references"),
        "{body}"
    );
    assert!(vault.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn typing_a_value_can_be_turned_off_and_a_reference_still_works() {
    let secrets = tempfile::tempdir().unwrap();
    let vault = vault_with_secret().await;
    let app = spin_up_with_env(&env(
        secrets.path(),
        Some(&vault),
        &[("CONNECTOR_TYPED_CREDENTIALS", "false")],
    ))
    .await;
    let cookie = session_cookie_for_seeded_user(&app.pool, "bayu@meridian.example").await;
    seed_oracle(&app).await;

    let (code, body) = send(
        &app,
        &cookie,
        "PUT",
        &credential_url(),
        Some(json!({ "primary": { "kind": "password", "value": "typed-value" } })),
    )
    .await;
    assert_eq!(code, StatusCode::BAD_REQUEST, "{body}");
    assert!(body.to_string().contains("turned off"), "{body}");
    assert!(!body.to_string().contains("typed-value"));

    let (code, body) = send(
        &app,
        &cookie,
        "PUT",
        &credential_url(),
        Some(json!({ "primary": { "reference": REFERENCE } })),
    )
    .await;
    assert_eq!(code, StatusCode::OK, "{body}");

    // And creating a connector with a typed value is refused too.
    let (code, body) = send(
        &app,
        &cookie,
        "POST",
        "/api/connectors",
        Some(json!({
            "name": "typed", "type": "Oracle", "direction": "source", "host": "h",
            "credential": {"source": "managed", "primary": "password", "values": {"primary": "pw"}},
            "environment": "production", "tenant": "g6", "tenantId": MERIDIAN_GROUP,
            "residency": "", "capabilities": []
        })),
    )
    .await;
    assert_eq!(code, StatusCode::BAD_REQUEST, "{body}");
    // A reference cannot be a new connector's credential: it is set after.
    let (code, body) = send(
        &app,
        &cookie,
        "POST",
        "/api/connectors",
        Some(json!({
            "name": "by reference", "type": "Oracle", "direction": "source", "host": "h",
            "credential": {"source": "reference", "primary": "password"},
            "environment": "production", "tenant": "g6", "tenantId": MERIDIAN_GROUP,
            "residency": "", "capabilities": []
        })),
    )
    .await;
    assert_eq!(code, StatusCode::BAD_REQUEST, "{body}");
}

#[tokio::test]
async fn the_credential_settings_say_what_the_form_may_offer() {
    let secrets = tempfile::tempdir().unwrap();
    let app = spin_up_with_env(&env(secrets.path(), None, &[])).await;
    let cookie = session_cookie_for_seeded_user(&app.pool, "bayu@meridian.example").await;
    let (code, body) = send(
        &app,
        &cookie,
        "GET",
        "/api/connectors/credential-settings",
        None,
    )
    .await;
    assert_eq!(code, StatusCode::OK, "{body}");
    assert_eq!(
        body,
        json!({"secretManager": null, "typedCredentials": true})
    );

    let vault = vault_with_secret().await;
    let app = spin_up_with_env(&env(
        secrets.path(),
        Some(&vault),
        &[("CONNECTOR_TYPED_CREDENTIALS", "false")],
    ))
    .await;
    let cookie = session_cookie_for_seeded_user(&app.pool, "bayu@meridian.example").await;
    let (code, body) = send(
        &app,
        &cookie,
        "GET",
        "/api/connectors/credential-settings",
        None,
    )
    .await;
    assert_eq!(code, StatusCode::OK, "{body}");
    assert_eq!(
        body,
        json!({"secretManager": "vault", "typedCredentials": false})
    );
    // Names no address and no sign-in.
    assert!(!body.to_string().contains("127.0.0.1"));
    assert!(!body.to_string().contains("ci-vault-token"));
}
