//! `GET /api/settings/deployment` (`ADM-6`, slice 1 of
//! `docs/superpowers/plans/2026-11-06-adm-6-9-console-settings.md`).
//!
//! What this proves: every feature of the deployment is listed, each with
//! whether it is set up, what is missing for the one that is not, and
//! whether the setting is the server's or the console's (plan D8). It
//! reports configuration, never health.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use crate::common;

use std::collections::HashMap;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use common::{
    TestApp, create_principal_with_permissions, create_zero_permission_principal,
    session_cookie_for_user, spin_up_with_env,
};
use serde_json::{Value, json};
use tower::ServiceExt;

/// The deployment route.
const DEPLOYMENT: &str = "/api/settings/deployment";

/// The features of the deployment, as the answer lists them.
async fn features(app: &axum::Router, cookie: &str) -> Vec<Value> {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(DEPLOYMENT)
                .header("cookie", cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "the deployment route answers"
    );
    let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
    let body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    body["features"]
        .as_array()
        .expect("the answer lists the features")
        .clone()
}

/// The entry of one feature, by its name.
fn feature(features: &[Value], name: &str) -> Value {
    features
        .iter()
        .find(|entry| entry["feature"] == json!(name))
        .unwrap_or_else(|| panic!("the answer lists {name}"))
        .clone()
}

/// With nothing set, every feature is off and names what is missing.
#[tokio::test]
async fn every_feature_is_off_and_names_what_is_missing() {
    let TestApp { router, pool } = spin_up_with_env(&HashMap::new()).await;
    let admin = create_principal_with_permissions(&pool, "settings:write").await;
    let cookie = session_cookie_for_user(&pool, admin).await;
    let features = features(&router, &cookie).await;

    assert_eq!(features.len(), 5, "{features:?}");
    assert_eq!(
        feature(&features, "pdfExport"),
        json!({
            "feature": "pdfExport",
            "on": false,
            "missing": ["GOTENBERG_URL", "CONSOLE_INTERNAL_URL", "RENDER_TOKEN_SECRET"],
            "where": "server",
        })
    );
    assert_eq!(
        feature(&features, "email"),
        json!({
            "feature": "email",
            "on": false,
            "missing": ["host"],
            "where": "console",
        })
    );
    assert_eq!(
        feature(&features, "consoleAddress"),
        json!({
            "feature": "consoleAddress",
            "on": false,
            "missing": ["consolePublicUrl"],
            "where": "console",
        })
    );
    // `ADM-12` review round 2, R2-B2: the connection can now be saved in
    // Settings, so with nothing set the console owns it and names the two
    // fields as the form names them — the same shape `consoleAddress` has.
    // (`D3` still lets the environment own the whole connection once
    // `OIDC_ISSUER` is set, and then the names are the environment's.)
    assert_eq!(
        feature(&features, "singleSignOn"),
        json!({
            "feature": "singleSignOn",
            "on": false,
            "missing": ["issuer", "clientId"],
            "where": "console",
        })
    );
    // `ADM-9`: the assistant's key is the console's to set when the
    // environment holds none, so the missing setting is named the way the
    // console names it ("key"), and the console may fix it here.
    assert_eq!(
        feature(&features, "assistant"),
        json!({
            "feature": "assistant",
            "on": false,
            "missing": ["key"],
            "where": "console",
        })
    );
}

/// `ADM-9`: with the environment holding the key, the assistant is the
/// server's — the whole provider unit is read-only in the console.
#[tokio::test]
async fn an_assistant_the_environment_owns_is_named_the_way_the_operator_sets_it() {
    let mut env = HashMap::new();
    env.insert("LLM_KEY".to_owned(), "a-test-llm-key".to_owned());
    let TestApp { router, pool } = spin_up_with_env(&env).await;
    let admin = create_principal_with_permissions(&pool, "settings:write").await;
    let cookie = session_cookie_for_user(&pool, admin).await;
    let features = features(&router, &cookie).await;

    assert_eq!(
        feature(&features, "assistant"),
        json!({
            "feature": "assistant",
            "on": true,
            "missing": [],
            "where": "server",
        })
    );
}

/// With every setting in place, every feature is on and nothing is missing.
#[tokio::test]
async fn every_feature_is_on_when_its_settings_are_in_place() {
    let mut env = HashMap::new();
    env.insert(
        "GOTENBERG_URL".to_owned(),
        "http://gotenberg:3000".to_owned(),
    );
    env.insert(
        "CONSOLE_INTERNAL_URL".to_owned(),
        "http://lakehouse-frontend:3000".to_owned(),
    );
    env.insert(
        "RENDER_TOKEN_SECRET".to_owned(),
        "a-test-render-secret".to_owned(),
    );
    env.insert("SMTP_HOST".to_owned(), "smtp.example.com".to_owned());
    env.insert(
        "CONSOLE_PUBLIC_URL".to_owned(),
        "https://console.example.com".to_owned(),
    );
    env.insert(
        "OIDC_ISSUER".to_owned(),
        "https://idp.example.com".to_owned(),
    );
    env.insert("OIDC_CLIENT_ID".to_owned(), "lakehouse".to_owned());
    env.insert("LLM_KEY".to_owned(), "a-test-llm-key".to_owned());
    let TestApp { router, pool } = spin_up_with_env(&env).await;
    let admin = create_principal_with_permissions(&pool, "settings:write").await;
    let cookie = session_cookie_for_user(&pool, admin).await;
    let features = features(&router, &cookie).await;

    for name in [
        "pdfExport",
        "email",
        "consoleAddress",
        "singleSignOn",
        "assistant",
    ] {
        let entry = feature(&features, name);
        assert_eq!(entry["on"], json!(true), "{name}: {entry}");
        assert_eq!(entry["missing"], json!([]), "{name}: {entry}");
    }
    // A feature the server owns says so: the console cannot change it.
    assert_eq!(
        feature(&features, "email")["where"],
        json!("server"),
        "SMTP_HOST is set, so the email settings are the server's"
    );
    assert_eq!(
        feature(&features, "consoleAddress")["where"],
        json!("server"),
        "CONSOLE_PUBLIC_URL is set, so the address is the server's"
    );
}

/// A feature the console owns says so too, so the console knows it may be
/// changed there.
#[tokio::test]
async fn a_feature_the_console_owns_says_where_it_is_set() {
    let TestApp { router, pool } = spin_up_with_env(&HashMap::new()).await;
    let admin = create_principal_with_permissions(&pool, "settings:write").await;
    let cookie = session_cookie_for_user(&pool, admin).await;
    // Save the mail server in the console.
    let resp = router
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/settings/email")
                .header("cookie", &cookie)
                .header("content-type", "application/json")
                .body(Body::from(r#"{"host":"smtp.example.com"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let features = features(&router, &cookie).await;
    let email = feature(&features, "email");
    assert_eq!(email["on"], json!(true), "{email}");
    assert_eq!(email["missing"], json!([]), "{email}");
    assert_eq!(email["where"], json!("console"), "{email}");
}

/// `ADM-7 review R1`: when the environment owns the connection, the mail
/// server is the server's to set. The answer names the environment variable
/// the operator sets it with, and says the console may not change it.
#[tokio::test]
async fn a_mail_server_the_environment_owns_is_named_the_way_the_operator_sets_it() {
    let mut env = HashMap::new();
    env.insert("SMTP_PASS".to_owned(), "an-environment-password".to_owned());
    let TestApp { router, pool } = spin_up_with_env(&env).await;
    let admin = create_principal_with_permissions(&pool, "settings:write").await;
    let cookie = session_cookie_for_user(&pool, admin).await;
    let features = features(&router, &cookie).await;

    assert_eq!(
        feature(&features, "email"),
        json!({
            "feature": "email",
            "on": false,
            "missing": ["SMTP_HOST"],
            "where": "server",
        })
    );
}

/// The route needs `settings:write` like the other settings routes (plan
/// D4): it names the deployment's own settings.
#[tokio::test]
async fn the_deployment_route_needs_settings_write() {
    let TestApp { router, pool } = spin_up_with_env(&HashMap::new()).await;
    let reader = create_zero_permission_principal(&pool).await;
    let cookie = session_cookie_for_user(&pool, reader).await;
    let resp = router
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(DEPLOYMENT)
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}
