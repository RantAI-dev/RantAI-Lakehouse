//! Column lineage for dashboard SQL sources (`DATA-13`, finding F4): saving
//! a source records which `serving` columns feed each of its output columns,
//! and deleting the source removes them. Driven through the real router
//! against a per-test Postgres and a `wiremock` stand-in for `ClickHouse`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use lakehouse_store::column_lineage::{self, Scope};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use wiremock::matchers::{body_string_contains, method};
use wiremock::{Mock, MockServer, ResponseTemplate};

use common::{TestApp, session_cookie_for_seeded_user, spin_up_with_env};

const SQL: &str = "SELECT place, visitors AS v, visitors * 2 AS doubled FROM serving.mart_x";

async fn app() -> (axum::Router, PgPool, String, MockServer) {
    let ch = MockServer::start().await;
    // DESCRIBE answers the probed output columns of the source.
    Mock::given(method("POST"))
        .and(body_string_contains("DESCRIBE"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "meta": [{"name": "name", "type": "String"}, {"name": "type", "type": "String"}],
            "data": [
                {"name": "place", "type": "String"},
                {"name": "v", "type": "UInt64"},
                {"name": "doubled", "type": "UInt64"},
            ],
            "rows": 3,
        })))
        .mount(&ch)
        .await;
    // The columns of `serving.mart_x`, for the mapping.
    Mock::given(method("POST"))
        .and(body_string_contains("system.columns"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "meta": [
                {"name": "name", "type": "String"}, {"name": "default_kind", "type": "String"},
                {"name": "default_expression", "type": "String"}
            ],
            "data": [
                {"name": "place", "default_kind": "", "default_expression": ""},
                {"name": "visitors", "default_kind": "", "default_expression": ""},
            ],
            "rows": 2,
        })))
        .mount(&ch)
        .await;
    // The source row `delete` looks up; the id is filled in by the test that
    // needs it through a second, more specific mock mounted first.
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "meta": [], "data": [], "rows": 0,
        })))
        .mount(&ch)
        .await;
    let mut overrides = std::collections::HashMap::new();
    overrides.insert("CH_URL".to_owned(), ch.uri());
    let TestApp { router, pool } = spin_up_with_env(&overrides).await;
    let cookie = session_cookie_for_seeded_user(&pool, "fajar@meridian.example").await;
    (router, pool, cookie, ch)
}

async fn send(
    router: &axum::Router,
    cookie: &str,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let resp = router
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .header("content-type", "application/json")
                .header("cookie", cookie)
                .body(match body {
                    Some(b) => Body::from(b.to_string()),
                    None => Body::empty(),
                })
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

fn everything() -> Scope {
    Scope {
        tenant: None,
        all_tenants: true,
        shared: true,
    }
}

#[tokio::test]
async fn saving_a_sql_source_records_its_column_mappings() {
    let (router, pool, cookie, _ch) = app().await;

    let (status, body) = send(
        &router,
        &cookie,
        "POST",
        "/api/dashboard/sources",
        Some(json!({ "title": "Visitors", "sql": SQL })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let id = body["source"]["id"].as_str().unwrap().to_owned();
    let key = format!("sql_source:{id}");

    let rows = column_lineage::touching_table(&pool, &key, everything())
        .await
        .unwrap();
    let mut got: Vec<(String, String, String, String)> = rows
        .iter()
        .map(|r| {
            (
                r.to_column.clone(),
                r.from_table.clone().unwrap(),
                r.from_column.clone().unwrap(),
                r.transform.clone(),
            )
        })
        .collect();
    got.sort();
    let mart = "serving.mart_x".to_owned();
    assert_eq!(
        got,
        vec![
            (
                "doubled".to_owned(),
                mart.clone(),
                "visitors".to_owned(),
                "expression".to_owned()
            ),
            (
                "place".to_owned(),
                mart.clone(),
                "place".to_owned(),
                "copy".to_owned()
            ),
            (
                "v".to_owned(),
                mart,
                "visitors".to_owned(),
                "copy".to_owned()
            ),
        ]
    );
    assert!(
        rows.iter()
            .all(|r| r.producer_kind == "sql_source" && r.producer_ref == id)
    );
}

#[tokio::test]
async fn saving_again_replaces_the_mappings_and_deleting_removes_them() {
    let (router, pool, cookie, ch) = app().await;
    let (_, body) = send(
        &router,
        &cookie,
        "POST",
        "/api/dashboard/sources",
        Some(json!({ "title": "Visitors", "sql": SQL })),
    )
    .await;
    let id = body["source"]["id"].as_str().unwrap().to_owned();
    let key = format!("sql_source:{id}");

    // `update` and `delete` look the source up first; answer with the row.
    // Priority 1 (wiremock: lower wins) puts it ahead of the catch-all.
    Mock::given(method("POST"))
        .and(body_string_contains("FROM console.bi_source FINAL"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "meta": [], "rows": 1,
            "data": [{
                "id": id, "title": "Visitors", "sql": SQL, "columns_json": "[]",
                "folder_id": "", "created_by": "x", "updated_at": ""
            }],
        })))
        .with_priority(1)
        .mount(&ch)
        .await;

    // The same source saved with a narrower SELECT: the earlier set is gone.
    // (The DESCRIBE mock still answers three columns, so two of them are
    // now ones the statement does not account for: partial, not dropped.)
    let (status, body) = send(
        &router,
        &cookie,
        "PUT",
        "/api/dashboard/sources",
        Some(json!({ "id": id, "title": "Visitors", "sql": "SELECT place FROM serving.mart_x" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let rows = column_lineage::touching_table(&pool, &key, everything())
        .await
        .unwrap();
    assert_eq!(rows.len(), 3);
    let by_col = |name: &str| rows.iter().find(|r| r.to_column == name).unwrap();
    assert!(!by_col("place").partial);
    assert!(by_col("v").partial && by_col("doubled").partial);
    assert!(by_col("v").reason.as_deref().is_some_and(|r| !r.is_empty()));

    let (status, body) = send(
        &router,
        &cookie,
        "DELETE",
        &format!("/api/dashboard/sources?id={id}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        column_lineage::touching_table(&pool, &key, everything())
            .await
            .unwrap()
            .is_empty()
    );
}
