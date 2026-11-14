//! `GET`/`PUT /api/settings/sign-in` (`ADM-8`, slice 2 of
//! `docs/superpowers/plans/2026-11-06-adm-6-9-console-settings.md`).
//!
//! What this proves:
//!
//! * every route needs `settings:write`, reading included (plan D4);
//! * the answer says where every value comes from (plan D1);
//! * a field the server's environment owns is refused with a 409 and
//!   nothing is written (plan D1);
//! * a value outside the range plan D12 names is refused with a 400 and the
//!   sentence that names the field;
//! * every save writes one audit row, naming the fields that changed.

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

/// What the person is told when the environment owns a field (plan D1).
const SET_BY_THE_SERVER: &str =
    "This setting is set in the server's environment and cannot be changed here.";

/// The sign-in settings route.
const SIGN_IN: &str = "/api/settings/sign-in";

/// Call one of the routes: its status and its body.
async fn call(
    app: &axum::Router,
    verb: &str,
    path: &str,
    cookie: &str,
    body: &str,
) -> (StatusCode, Value) {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method(verb)
                .uri(path)
                .header("cookie", cookie)
                .header("content-type", "application/json")
                .body(Body::from(body.to_owned()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

/// A deployment with nothing saved answers every limit with its default and
/// says where the default comes from.
#[tokio::test]
async fn a_fresh_deployment_answers_every_limit_with_its_source() {
    let TestApp { router, pool } = spin_up_with_env(&HashMap::new()).await;
    let admin = create_principal_with_permissions(&pool, "settings:write").await;
    let cookie = session_cookie_for_user(&pool, admin).await;
    let (status, body) = call(&router, "GET", SIGN_IN, &cookie, "").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["maxFailures"],
        json!({"value": 5, "source": "default"})
    );
    assert_eq!(
        body["failureWindowSecs"],
        json!({"value": 900, "source": "default"})
    );
    assert_eq!(
        body["lockoutSecs"],
        json!({"value": 300, "source": "default"})
    );
    assert_eq!(
        body["retentionDays"],
        json!({"value": 30, "source": "default"})
    );
    assert_eq!(
        body["embedMaxLifetimeSecs"],
        json!({"value": 86400, "source": "default"})
    );
}

/// Plan D4: reading the limits needs `settings:write` like changing them.
#[tokio::test]
async fn a_caller_without_settings_write_is_refused() {
    let TestApp { router, pool } = spin_up_with_env(&HashMap::new()).await;
    let reader = create_zero_permission_principal(&pool).await;
    let cookie = session_cookie_for_user(&pool, reader).await;
    let (status, _) = call(&router, "GET", SIGN_IN, &cookie, "").await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = call(&router, "PUT", SIGN_IN, &cookie, r#"{"maxFailures":3}"#).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    // Nothing was written by the refused save.
    assert!(
        lakehouse_store::sign_in_settings::get(&pool)
            .await
            .unwrap()
            .is_none()
    );
}

/// Plan D1: a field the server's environment owns is refused with a 409, and
/// nothing is written — not even the field the environment does not own.
#[tokio::test]
async fn a_field_the_environment_owns_is_refused_and_nothing_is_written() {
    let mut env = HashMap::new();
    env.insert("LOGIN_MAX_FAILURES".to_owned(), "7".to_owned());
    let TestApp { router, pool } = spin_up_with_env(&env).await;
    let admin = create_principal_with_permissions(&pool, "settings:write").await;
    let cookie = session_cookie_for_user(&pool, admin).await;

    let (status, body) = call(
        &router,
        "PUT",
        SIGN_IN,
        &cookie,
        r#"{"maxFailures":3,"lockoutSecs":120}"#,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"].as_str(), Some(SET_BY_THE_SERVER), "{body}");
    assert!(
        lakehouse_store::sign_in_settings::get(&pool)
            .await
            .unwrap()
            .is_none(),
        "nothing is written by a refused save"
    );

    // The environment's value is still what is in effect, and it says so.
    let (status, read) = call(&router, "GET", SIGN_IN, &cookie, "").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(read["maxFailures"], json!({"value": 7, "source": "server"}));
    // The field the environment does not own is still the console's.
    assert_eq!(
        read["lockoutSecs"],
        json!({"value": 300, "source": "default"})
    );
}

/// Plan D12: a value outside the range is refused with a fixed sentence that
/// names the field.
#[tokio::test]
async fn a_value_outside_the_range_is_refused_with_a_sentence_that_names_the_field() {
    let TestApp { router, pool } = spin_up_with_env(&HashMap::new()).await;
    let admin = create_principal_with_permissions(&pool, "settings:write").await;
    let cookie = session_cookie_for_user(&pool, admin).await;
    for (body, sentence) in [
        (
            r#"{"maxFailures":2}"#,
            "maxFailures must be a whole number from 3 to 20.",
        ),
        (
            r#"{"maxFailures":21}"#,
            "maxFailures must be a whole number from 3 to 20.",
        ),
        (
            r#"{"failureWindowSecs":59}"#,
            "failureWindowSecs must be a whole number from 60 to 86400.",
        ),
        (
            r#"{"lockoutSecs":86401}"#,
            "lockoutSecs must be a whole number from 60 to 86400.",
        ),
        (
            r#"{"retentionDays":6}"#,
            "retentionDays must be a whole number from 7 to 3650.",
        ),
        (
            r#"{"embedMaxLifetimeSecs":299}"#,
            "embedMaxLifetimeSecs must be a whole number from 300 to 86400.",
        ),
        (
            r#"{"embedMaxLifetimeSecs":86401}"#,
            "embedMaxLifetimeSecs must be a whole number from 300 to 86400.",
        ),
    ] {
        let (status, got) = call(&router, "PUT", SIGN_IN, &cookie, body).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}: {got}");
        assert_eq!(got["error"].as_str(), Some(sentence), "{body}: {got}");
    }
    // A body that is not the sign-in fields at all.
    let (status, got) = call(&router, "PUT", SIGN_IN, &cookie, r#"{"maxFailure":3}"#).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{got}");
    assert!(
        got["error"]
            .as_str()
            .is_some_and(|m| m.starts_with("body must be")),
        "{got}"
    );
    // Nothing was written by any of the refused saves.
    assert!(
        lakehouse_store::sign_in_settings::get(&pool)
            .await
            .unwrap()
            .is_none()
    );
}

/// Plan D5: a field that is not sent keeps what is stored, and an explicit
/// `null` clears it.
#[tokio::test]
async fn a_field_that_is_not_sent_keeps_its_value_and_null_clears_it() {
    let TestApp { router, pool } = spin_up_with_env(&HashMap::new()).await;
    let admin = create_principal_with_permissions(&pool, "settings:write").await;
    let cookie = session_cookie_for_user(&pool, admin).await;
    let (status, body) = call(
        &router,
        "PUT",
        SIGN_IN,
        &cookie,
        r#"{"maxFailures":3,"lockoutSecs":120,"retentionDays":14}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["maxFailures"],
        json!({"value": 3, "source": "console"})
    );
    assert_eq!(
        body["lockoutSecs"],
        json!({"value": 120, "source": "console"})
    );

    // A save that does not send the limit keeps it.
    let (status, again) = call(&router, "PUT", SIGN_IN, &cookie, r#"{"retentionDays":30}"#).await;
    assert_eq!(status, StatusCode::OK, "{again}");
    assert_eq!(
        again["maxFailures"],
        json!({"value": 3, "source": "console"})
    );

    // `null` clears it.
    let (status, cleared) = call(&router, "PUT", SIGN_IN, &cookie, r#"{"maxFailures":null}"#).await;
    assert_eq!(status, StatusCode::OK, "{cleared}");
    assert_eq!(
        cleared["maxFailures"],
        json!({"value": 5, "source": "default"})
    );
}

/// Plan D6: one audit row per save, naming the fields that changed.
#[tokio::test]
async fn a_save_writes_one_audit_row_naming_the_fields() {
    let TestApp { router, pool } = spin_up_with_env(&HashMap::new()).await;
    let admin = create_principal_with_permissions(&pool, "settings:write").await;
    let cookie = session_cookie_for_user(&pool, admin).await;
    let (status, body) = call(
        &router,
        "PUT",
        SIGN_IN,
        &cookie,
        r#"{"maxFailures":3,"retentionDays":14}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let rows: Vec<(String, Value)> = sqlx::query_as(
        "SELECT action, args FROM audit_event WHERE action = 'settings.sign_in.update'",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(rows.len(), 1, "one save, one audit row");
    let (action, args) = &rows[0];
    assert_eq!(action, "settings.sign_in.update");
    let fields: Vec<&str> = args["fields"]
        .as_array()
        .expect("the row names the fields")
        .iter()
        .filter_map(|value| value.as_str())
        .collect();
    assert!(fields.contains(&"maxFailures"), "{fields:?}");
    assert!(fields.contains(&"retentionDays"), "{fields:?}");
}
