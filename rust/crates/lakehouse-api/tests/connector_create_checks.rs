//! `SRC-41` D11/D12 (QA `LH-SRC-3-11`): the connection settings a console
//! create sends are checked **before** the connector exists, and the refusal
//! names the field.
//!
//! Before this, the wizard created the row (`POST /api/connectors`) and then
//! saved the settings (`PUT .../ingest-spec`), where they were checked only: a
//! refused dial left a connector with no settings. `POST` now takes an optional
//! `connection` and runs the same three checks first.
//!
//! The refusal is the validator's own fixed sentence — one of
//! `ingest_spec.rs`'s, or serde's field text about the caller's own dial — never
//! `dial for adapter …` and never the adapter's name. Nothing is dialled at that
//! point, so it holds no source's text and no resolved address.
//!
//! `CONNECTOR_SECRETS_DIR` is a temporary directory (`/run/secrets` does not
//! exist outside Docker), and the internal-host block is turned on only for the
//! test that needs a loopback dial to be allowed.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::collections::HashMap;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use tower::ServiceExt;

use common::{TestApp, session_cookie_for_seeded_user, spin_up_with_env};

const MERIDIAN_GROUP: &str = "11111111-1111-4111-8111-000000000001";

struct Fixture {
    app: TestApp,
    cookie: String,
    _secrets: tempfile::TempDir,
}

async fn fixture(allow_internal: bool) -> Fixture {
    let secrets = tempfile::tempdir().unwrap();
    let mut env = HashMap::from([(
        "CONNECTOR_SECRETS_DIR".to_owned(),
        secrets.path().to_string_lossy().into_owned(),
    )]);
    if allow_internal {
        env.insert(
            "CONNECTOR_PROBE_ALLOW_INTERNAL_HOSTS".to_owned(),
            "true".to_owned(),
        );
    }
    let app = spin_up_with_env(&env).await;
    let cookie = session_cookie_for_seeded_user(&app.pool, "bayu@meridian.example").await;
    Fixture {
        app,
        cookie,
        _secrets: secrets,
    }
}

async fn send(fx: &Fixture, method: &str, path: &str, body: Option<Value>) -> (StatusCode, Value) {
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header("cookie", &fx.cookie);
    let body = match body {
        Some(body) => {
            builder = builder.header("content-type", "application/json");
            Body::from(body.to_string())
        }
        None => Body::empty(),
    };
    let response = fx
        .app
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

/// A create body that is valid without `connection`: the shape the console
/// sends, with the seeded tenant's id.
fn create_body(name: &str, kind: &str) -> Value {
    json!({
        "name": name, "type": kind, "direction": "source", "host": "h",
        "credential": {"source": "managed", "primary": "password", "values": {"primary": "pw"}},
        "environment": "production", "tenant": "g6", "tenantId": MERIDIAN_GROUP,
        "residency": "", "capabilities": []
    })
}

/// The `saas` connection the vendor's form refuses: `127.0.0.1` is not an
/// account `Dynamics 365` can have.
fn refused_saas_connection() -> Value {
    json!({
        "adapter": "saas",
        "dial": {
            "vendor": "dynamics365", "account": "127.0.0.1",
            "auth": {"type": "oauth2_authorization_code", "clientId": "app"}
        }
    })
}

/// The `sql` connection at an internal address.
fn internal_sql_connection() -> Value {
    json!({
        "adapter": "sql",
        "dial": {"driver": "postgres", "host": "127.0.0.1", "port": 5432, "database": "d", "user": "u"}
    })
}

async fn count(fx: &Fixture, name: &str) -> i64 {
    let (rows,): (i64,) = sqlx::query_as("SELECT count(*) FROM connector WHERE name = $1")
        .bind(name)
        .fetch_one(&fx.app.pool)
        .await
        .unwrap();
    rows
}

#[tokio::test]
async fn a_business_app_account_that_is_refused_creates_nothing_and_the_refusal_names_the_field() {
    let fx = fixture(false).await;
    let mut body = create_body("da-business-app", "Dynamics 365");
    body["connection"] = refused_saas_connection();

    let (status, answer) = send(&fx, "POST", "/api/connectors", Some(body)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{answer}");
    assert_eq!(
        answer["error"],
        json!("The connection settings are not valid: account is not in the form this vendor uses"),
        "{answer}"
    );
    let text = answer.to_string();
    assert!(!text.contains("dial for adapter"), "{text}");
    assert!(!text.contains("saas"), "{text}");
    assert!(!text.contains("127.0.0.1"), "{text}");
    assert_eq!(count(&fx, "da-business-app").await, 0);
}

#[tokio::test]
async fn a_database_at_an_internal_address_creates_nothing() {
    let fx = fixture(false).await;
    let mut body = create_body("da-internal", "PostgreSQL");
    body["connection"] = internal_sql_connection();

    let (status, answer) = send(&fx, "POST", "/api/connectors", Some(body)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{answer}");
    let error = answer["error"].as_str().expect("an error sentence");
    assert!(error.starts_with("refusing to dial"), "{error}");
    assert_eq!(count(&fx, "da-internal").await, 0);
}

#[tokio::test]
async fn good_connection_settings_create_the_connector() {
    let fx = fixture(true).await;
    let mut body = create_body("da-good", "PostgreSQL");
    body["connection"] = internal_sql_connection();

    let (status, answer) = send(&fx, "POST", "/api/connectors", Some(body)).await;
    assert_eq!(status, StatusCode::CREATED, "{answer}");
    assert_eq!(count(&fx, "da-good").await, 1);
}

/// The assistant's path (`create_connector` offers no `connection`) still
/// creates the row: the new field is optional.
#[tokio::test]
async fn a_create_without_connection_settings_still_creates_the_connector() {
    let fx = fixture(false).await;
    let (status, answer) = send(
        &fx,
        "POST",
        "/api/connectors",
        Some(create_body("da-plain", "PostgreSQL")),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{answer}");
    assert_eq!(count(&fx, "da-plain").await, 1);
}

/// The save that stores the settings answers the same sentence for the same
/// dial: one refusal, whichever call runs the check.
#[tokio::test]
async fn saving_refused_connection_settings_answers_the_same_sentence() {
    let fx = fixture(true).await;
    let (status, answer) = send(
        &fx,
        "POST",
        "/api/connectors",
        Some(create_body("da-save", "PostgreSQL")),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{answer}");
    let id = answer["id"].as_str().expect("the created id").to_owned();

    let put = json!({
        "adapter": "saas",
        "ingestMode": "batch",
        "dial": refused_saas_connection()["dial"],
        "sourceObjects": []
    });
    let (status, answer) = send(
        &fx,
        "PUT",
        &format!("/api/connectors/{id}/ingest-spec"),
        Some(put),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{answer}");
    assert_eq!(
        answer["error"],
        json!("The connection settings are not valid: account is not in the form this vendor uses"),
        "{answer}"
    );
}
