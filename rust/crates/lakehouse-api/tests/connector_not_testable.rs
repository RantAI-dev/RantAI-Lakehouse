//! `SRC-6` F3: a connector that connects only from the orchestrator
//! (Oracle here) is "not testable from the console", not "supported and
//! failed".
//!
//! Before the fix the Oracle probe answered `supported: true, ok: false`, so
//! `PUT .../credential` refused every new Oracle credential with a 422 and
//! every `POST .../test` marked the connector unhealthy. Now the probe is
//! `supported: false`: the credential is saved with `verified: false`, and a
//! test leaves the stored `health` as it was.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::collections::HashMap;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use tower::ServiceExt;

use common::{TestApp, session_cookie_for_seeded_user, spin_up_with_env};

const MERIDIAN_GROUP: &str = "11111111-1111-4111-8111-000000000001";
const ORACLE_ID: &str = "conn-src6-oracle";

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

async fn health_of(app: &TestApp) -> String {
    sqlx::query_scalar("SELECT health FROM connector WHERE id = $1")
        .bind(ORACLE_ID)
        .fetch_one(&app.pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn an_oracle_credential_is_saved_unverified_and_its_test_leaves_health_alone() {
    let secrets = tempfile::tempdir().unwrap();
    let env = HashMap::from([(
        "CONNECTOR_SECRETS_DIR".to_owned(),
        secrets.path().to_string_lossy().into_owned(),
    )]);
    let app = spin_up_with_env(&env).await;
    let cookie = session_cookie_for_seeded_user(&app.pool, "bayu@meridian.example").await;

    // `health = 'degraded'` is a value a test run would overwrite, so an
    // unchanged value proves the test did not stamp the row.
    sqlx::query(
        "INSERT INTO connector (id, tenant_id, name, type, direction, health, host, secret_ref, \
         environment, tenant, adapter, ingest_mode, dial) VALUES ($1, $2, 'src6 oracle', \
         'Oracle', 'source', 'degraded', 'unused', 'env:CONNECTOR_SRC6_ORACLE_PASSWORD', \
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

    let (status, body) = send(
        &app,
        &cookie,
        "PUT",
        &format!("/api/connectors/{ORACLE_ID}/credential"),
        Some(json!({ "primary": { "kind": "password", "value": "a-new-oracle-password" } })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["saved"], true);
    assert_eq!(body["verified"], false);
    let stored = std::fs::read_to_string(
        secrets
            .path()
            .join("connector_managed_conn_src6_oracle_password"),
    )
    .unwrap();
    assert_eq!(stored, "a-new-oracle-password");

    let (status, body) = send(
        &app,
        &cookie,
        "POST",
        &format!("/api/connectors/{ORACLE_ID}/test"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["supported"], false);
    assert_eq!(body["ok"], false);
    assert!(body["latencyMs"].is_null(), "{body}");
    assert!(
        body["message"]
            .as_str()
            .unwrap()
            .contains("cannot be tested from the console"),
        "{body}"
    );
    assert_eq!(health_of(&app).await, "degraded");
}
