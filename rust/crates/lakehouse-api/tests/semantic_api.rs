//! Route-level tests for the semantic layer's API (`GET /api/semantic`,
//! `GET`/`PUT /api/semantic/{asset}`, AI-16).
//!
//! `ClickHouse` is a `wiremock` server that lists two tables, the way the
//! DATA MAP reads them (`system.tables`, `system.columns`): the `PUT` needs
//! it to know a table is in `serving` or `silver` and has the column, and
//! the per-table `GET` needs it to mark an entry `stale`. The catalog
//! queries are not mounted, so the live read degrades as it does without a
//! catalog.
//!
//! The principals are the seeded Platform Admin (`*:*`, never refused by
//! the shared-catalog rule, `catalog_tenant_refusal`) for the behavior, and
//! one holding `catalog:read` and `catalog:write` but not `*:*` for the
//! refusal: the seed has four tenants and these tests set no
//! `CATALOG_TENANT_ID`, so that rule refuses a caller who is not
//! unrestricted.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::collections::HashMap;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::{
    TestApp, create_principal_with_permissions, session_cookie_for_seeded_user,
    session_cookie_for_user, spin_up_with_env,
};
use lakehouse_store::semantic::{self, SemanticInput};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;
use wiremock::matchers::{body_string_contains, method};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// `serving.orders` (`id`, `city`, `placed_at`) and `silver.events` (`id`).
async fn clickhouse() -> MockServer {
    let server = MockServer::start().await;
    let tables = json!({ "data": [
        { "database": "serving", "name": "orders", "total_rows": "3" },
        { "database": "silver", "name": "events", "total_rows": "5" },
    ]});
    let columns = json!({ "data": [
        { "database": "serving", "table": "orders", "name": "id", "type": "UInt32" },
        { "database": "serving", "table": "orders", "name": "city", "type": "String" },
        { "database": "serving", "table": "orders", "name": "placed_at", "type": "DateTime" },
        { "database": "silver", "table": "events", "name": "id", "type": "UInt32" },
    ]});
    Mock::given(method("POST"))
        .and(body_string_contains("FROM system.tables"))
        .respond_with(ResponseTemplate::new(200).set_body_json(tables))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(body_string_contains("FROM system.columns"))
        .respond_with(ResponseTemplate::new(200).set_body_json(columns))
        .mount(&server)
        .await;
    server
}

async fn app(ch: &MockServer, extra: &[(&str, &str)]) -> TestApp {
    let mut env = HashMap::new();
    env.insert("CH_URL".to_owned(), ch.uri());
    for (k, v) in extra {
        env.insert((*k).to_owned(), (*v).to_owned());
    }
    spin_up_with_env(&env).await
}

async fn admin_cookie(app: &TestApp) -> String {
    session_cookie_for_seeded_user(&app.pool, "fajar@meridian.example").await
}

async fn send(
    app: &TestApp,
    method: &str,
    uri: &str,
    cookie: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let request = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .header("cookie", cookie)
        .body(Body::from(body.map_or_else(String::new, |v| v.to_string())))
        .expect("build request");
    let response = app
        .router
        .clone()
        .oneshot(request)
        .await
        .expect("in-memory request");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read body");
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, body)
}

fn input(asset: &str, column: &str, description: &str) -> SemanticInput {
    SemanticInput {
        asset: asset.to_owned(),
        column_name: column.to_owned(),
        description: description.to_owned(),
        synonyms: Vec::new(),
        role: None,
    }
}

/// The error text of a refused body.
fn error_of(body: &Value) -> &str {
    body["error"].as_str().unwrap_or_default()
}

async fn put(app: &TestApp, cookie: &str, asset: &str, body: Value) -> (StatusCode, Value) {
    send(
        app,
        "PUT",
        &format!("/api/semantic/{asset}"),
        cookie,
        Some(body),
    )
    .await
}

/// A `PUT` that must be refused with a 400 naming `field`, and must write
/// nothing.
async fn assert_bad_request(body: Value, field: &str) {
    let ch = clickhouse().await;
    let app = app(&ch, &[]).await;
    let cookie = admin_cookie(&app).await;
    let (status, reply) = put(&app, &cookie, "serving.orders", body).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{reply}");
    assert!(
        error_of(&reply).contains(field),
        "the 400 must name `{field}`: {reply}"
    );
    assert!(
        semantic::list_all(&app.pool).await.unwrap().is_empty(),
        "a refused write must leave no entry"
    );
}

#[tokio::test]
async fn a_table_description_over_400_characters_is_a_400_naming_the_field() {
    assert_bad_request(
        json!({ "description": "x".repeat(401), "synonyms": [] }),
        "description",
    )
    .await;
}

#[tokio::test]
async fn a_column_description_over_200_characters_is_a_400_naming_the_field() {
    assert_bad_request(
        json!({ "column": "city", "description": "x".repeat(201), "synonyms": [] }),
        "description",
    )
    .await;
}

#[tokio::test]
async fn more_than_six_synonyms_is_a_400_naming_the_field() {
    assert_bad_request(
        json!({ "description": "Orders.", "synonyms": ["a", "b", "c", "d", "e", "f", "g"] }),
        "synonyms",
    )
    .await;
}

#[tokio::test]
async fn a_synonym_of_41_characters_or_none_is_a_400_naming_the_field() {
    assert_bad_request(
        json!({ "description": "Orders.", "synonyms": ["x".repeat(41)] }),
        "synonyms",
    )
    .await;
    assert_bad_request(
        json!({ "description": "Orders.", "synonyms": ["  "] }),
        "synonyms",
    )
    .await;
}

#[tokio::test]
async fn an_unknown_role_is_a_400_naming_the_field() {
    assert_bad_request(
        json!({ "column": "city", "description": "The city.", "synonyms": [], "role": "boss" }),
        "role",
    )
    .await;
}

#[tokio::test]
async fn a_role_on_the_table_itself_is_a_400_naming_the_field() {
    assert_bad_request(
        json!({ "description": "Orders.", "synonyms": [], "role": "measure" }),
        "role",
    )
    .await;
}

#[tokio::test]
async fn a_column_the_table_does_not_have_is_a_400_naming_the_field() {
    assert_bad_request(
        json!({ "column": "no_such_column", "description": "Nothing.", "synonyms": [] }),
        "column",
    )
    .await;
}

#[tokio::test]
async fn a_table_outside_serving_and_silver_is_a_404() {
    let ch = clickhouse().await;
    let app = app(&ch, &[]).await;
    let cookie = admin_cookie(&app).await;
    for asset in ["serving.missing", "bronze.orders", "orders"] {
        let (status, reply) = put(
            &app,
            &cookie,
            asset,
            json!({ "description": "Orders.", "synonyms": [] }),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{asset}: {reply}");
    }
    assert!(semantic::list_all(&app.pool).await.unwrap().is_empty());
}

#[tokio::test]
async fn a_warehouse_that_cannot_be_read_is_a_503_and_not_a_404() {
    let ch = MockServer::start().await; // answers 404 to everything
    let app = app(&ch, &[]).await;
    let cookie = admin_cookie(&app).await;
    let (status, reply) = put(
        &app,
        &cookie,
        "serving.orders",
        json!({ "description": "Orders.", "synonyms": [] }),
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{reply}");
}

#[tokio::test]
async fn a_put_over_a_draft_confirms_it_and_the_get_shows_the_persons_text() {
    let ch = clickhouse().await;
    let app = app(&ch, &[]).await;
    let cookie = admin_cookie(&app).await;
    let draft = SemanticInput {
        synonyms: vec!["town".to_owned()],
        role: Some("dimension".to_owned()),
        ..input("serving.orders", "city", "The model's guess.")
    };
    assert!(
        semantic::insert_draft(&app.pool, &draft, "test-model")
            .await
            .unwrap()
    );

    let (status, reply) = put(
        &app,
        &cookie,
        "serving.orders",
        json!({
            "column": "city",
            "description": "The city the order was placed in.",
            "synonyms": [" town ", "municipality"],
            "role": "dimension",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{reply}");

    let (status, reply) = send(&app, "GET", "/api/semantic/serving.orders", &cookie, None).await;
    assert_eq!(status, StatusCode::OK, "{reply}");
    let entries = reply["entries"].as_array().expect("entries");
    assert_eq!(entries.len(), 1, "{reply}");
    let entry = &entries[0];
    assert_eq!(entry["column"], "city");
    assert_eq!(entry["description"], "The city the order was placed in.");
    assert_eq!(entry["synonyms"], json!(["town", "municipality"]));
    assert_eq!(entry["role"], "dimension");
    assert_eq!(entry["status"], "confirmed");
    assert_eq!(entry["model"], Value::Null);
    let author: Uuid = sqlx::query_scalar("SELECT id FROM app_user WHERE email = $1")
        .bind("fajar@meridian.example")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(entry["writtenBy"], author.to_string());
    assert!(
        entry["updatedAt"]
            .as_str()
            .is_some_and(|t| t.ends_with('Z'))
    );
}

#[tokio::test]
async fn a_put_without_a_column_writes_the_tables_own_entry() {
    let ch = clickhouse().await;
    let app = app(&ch, &[]).await;
    let cookie = admin_cookie(&app).await;
    let (status, reply) = put(
        &app,
        &cookie,
        "silver.events",
        json!({ "description": "One row per event.", "synonyms": ["log"] }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{reply}");
    let (_, reply) = send(&app, "GET", "/api/semantic/silver.events", &cookie, None).await;
    assert_eq!(reply["entries"][0]["column"], Value::Null);
    assert_eq!(reply["entries"][0]["description"], "One row per event.");
}

#[tokio::test]
async fn a_put_records_an_audit_event_with_field_names_and_never_the_text() {
    let ch = clickhouse().await;
    let app = app(&ch, &[]).await;
    let cookie = admin_cookie(&app).await;
    let (status, reply) = put(
        &app,
        &cookie,
        "serving.orders",
        json!({
            "column": "city",
            "description": "Secret-looking words.",
            "synonyms": ["town"],
            "role": "dimension",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{reply}");
    let (resource, args): (Option<String>, Value) = sqlx::query_as(
        "SELECT resource_id, args FROM audit_event WHERE action = 'semantic.confirm'",
    )
    .fetch_one(&app.pool)
    .await
    .expect("one semantic.confirm audit event");
    assert_eq!(resource.as_deref(), Some("serving.orders"));
    assert_eq!(
        args,
        json!({ "fields": ["column", "description", "synonyms", "role"] })
    );
    assert!(!args.to_string().contains("Secret-looking"));
}

#[tokio::test]
async fn a_get_marks_an_entry_stale_when_its_column_is_gone_from_the_live_table() {
    let ch = clickhouse().await;
    let app = app(&ch, &[]).await;
    let cookie = admin_cookie(&app).await;
    for (column, text) in [("", "Orders."), ("city", "The city."), ("old_col", "Gone.")] {
        semantic::insert_draft(&app.pool, &input("serving.orders", column, text), "m")
            .await
            .unwrap();
    }
    let (status, reply) = send(&app, "GET", "/api/semantic/serving.orders", &cookie, None).await;
    assert_eq!(status, StatusCode::OK, "{reply}");
    let stale_of = |column: Value| -> Value {
        reply["entries"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["column"] == column)
            .unwrap_or_else(|| panic!("no entry for {column}: {reply}"))["stale"]
            .clone()
    };
    assert_eq!(stale_of(Value::Null), json!(false));
    assert_eq!(stale_of(json!("city")), json!(false));
    assert_eq!(stale_of(json!("old_col")), json!(true));
}

#[tokio::test]
async fn a_get_of_every_entry_carries_the_status_fields_of_each() {
    let ch = clickhouse().await;
    let app = app(&ch, &[]).await;
    let cookie = admin_cookie(&app).await;
    semantic::insert_draft(&app.pool, &input("serving.orders", "", "Orders."), "m")
        .await
        .unwrap();
    semantic::insert_draft(&app.pool, &input("silver.events", "", "Events."), "m")
        .await
        .unwrap();
    let (status, reply) = send(&app, "GET", "/api/semantic", &cookie, None).await;
    assert_eq!(status, StatusCode::OK, "{reply}");
    let entries = reply["entries"].as_array().expect("entries");
    assert_eq!(entries.len(), 2, "{reply}");
    assert_eq!(entries[0]["asset"], "serving.orders");
    assert_eq!(entries[0]["status"], "draft");
    assert_eq!(entries[0]["model"], "m");
    assert_eq!(entries[0]["writtenBy"], Value::Null);
    assert!(entries[0]["updatedAt"].is_string());
}

#[tokio::test]
async fn the_three_routes_answer_with_the_switch_off() {
    let ch = clickhouse().await;
    let app = app(&ch, &[("AI_SEMANTIC_LAYER", "false")]).await;
    let cookie = admin_cookie(&app).await;
    let (status, reply) = put(
        &app,
        &cookie,
        "serving.orders",
        json!({ "description": "Orders.", "synonyms": [] }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{reply}");
    let (status, reply) = send(&app, "GET", "/api/semantic/serving.orders", &cookie, None).await;
    assert_eq!(status, StatusCode::OK, "{reply}");
    assert_eq!(reply["entries"][0]["description"], "Orders.");
    let (status, reply) = send(&app, "GET", "/api/semantic", &cookie, None).await;
    assert_eq!(status, StatusCode::OK, "{reply}");
    assert_eq!(reply["entries"].as_array().map(Vec::len), Some(1));
}

#[tokio::test]
async fn a_caller_the_shared_catalog_rule_refuses_reads_nothing_and_writes_nothing() {
    let ch = clickhouse().await;
    let app = app(&ch, &[]).await;
    semantic::insert_draft(&app.pool, &input("serving.orders", "", "Orders."), "m")
        .await
        .unwrap();
    let user = create_principal_with_permissions(&app.pool, "catalog:read,catalog:write").await;
    let cookie = session_cookie_for_user(&app.pool, user).await;
    for uri in ["/api/semantic", "/api/semantic/serving.orders"] {
        let (status, reply) = send(&app, "GET", uri, &cookie, None).await;
        assert_eq!(status, StatusCode::OK, "{uri}: {reply}");
        assert_eq!(reply["supported"], json!(false), "{uri}: {reply}");
        assert!(reply.get("entries").is_none(), "{uri}: {reply}");
    }
    let (status, reply) = put(
        &app,
        &cookie,
        "serving.orders",
        json!({ "description": "Mine.", "synonyms": [] }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{reply}");
    assert_eq!(reply["supported"], json!(false), "{reply}");
    let rows = semantic::list_all(&app.pool).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].status, "draft", "the refused write must not land");
}

#[tokio::test]
async fn a_database_error_is_answered_as_database_error() {
    let ch = clickhouse().await;
    let app = app(&ch, &[]).await;
    let cookie = admin_cookie(&app).await;
    sqlx::query("DROP TABLE semantic_entry")
        .execute(&app.pool)
        .await
        .unwrap();
    let (status, reply) = put(
        &app,
        &cookie,
        "serving.orders",
        json!({ "description": "Orders.", "synonyms": [] }),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{reply}");
    assert_eq!(error_of(&reply), "database error");
    let (status, reply) = send(&app, "GET", "/api/semantic", &cookie, None).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{reply}");
    assert_eq!(error_of(&reply), "database error");
}
