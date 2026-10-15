//! `GET /api/dashboard/table-export` — the whole-result CSV of a raw table
//! (`BI-16` part A, T8), driven through the real router, a real per-test
//! Postgres (policies, audit) and a `wiremock` stand-in for `ClickHouse`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use lakehouse_store::governance::{self, CreatePolicyInput};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;
use wiremock::matchers::{body_string_contains, method};
use wiremock::{Mock, MockServer, ResponseTemplate};

use common::{TestApp, session_cookie_for_user, spin_up_with_env};

/// A user whose only role, `Exporter`, holds `dashboard:read`.
async fn exporter(pool: &PgPool) -> Uuid {
    let role_id = Uuid::new_v4();
    sqlx::query("INSERT INTO role (id, name, permissions, description) VALUES ($1, 'Exporter', 'dashboard:read', 'test')")
        .bind(role_id)
        .execute(pool)
        .await
        .unwrap();
    let user_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO app_user (id, name, email, status) VALUES ($1, 'Exporter', $2, 'active')",
    )
    .bind(user_id)
    .bind(format!("exporter-{user_id}@test.invalid"))
    .execute(pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO app_user_role (user_id, role_id) VALUES ($1, $2)")
        .bind(user_id)
        .bind(role_id)
        .execute(pool)
        .await
        .unwrap();
    user_id
}

fn chart_row(id: &str, kind: &str, def: &Value) -> Value {
    let spec = json!({
        "id": id, "title": "Visits by place", "kind": kind, "mart": "mart_x",
        "sql": "SELECT stored", "x": "", "y": "email"
    });
    let mut def = def.clone();
    def["title"] = json!("Visits by place");
    def["mart"] = json!("mart_x");
    def["kind"] = json!(kind);
    json!({
        "id": id, "board": "b1", "created_by": "ui", "created_at": "2026-01-01 00:00:00",
        "spec_json": json!({ "spec": spec, "def": def, "hasYear": false }).to_string(),
    })
}

async fn answer(ch: &MockServer, has: &str, meta: Value, data: Vec<Value>) {
    let rows = data.len();
    Mock::given(method("POST"))
        .and(body_string_contains(has))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({ "meta": meta, "data": data, "rows": rows })),
        )
        .mount(ch)
        .await;
}

/// Store reads, the column list (for the route's own check and the policy
/// engine), then the page of rows `data`.
async fn mount(ch: &MockServer, data: Vec<Value>) {
    let chart_meta = json!([
        {"name": "id", "type": "String"}, {"name": "board", "type": "String"},
        {"name": "created_by", "type": "String"}, {"name": "created_at", "type": "String"},
        {"name": "spec_json", "type": "String"}
    ]);
    answer(
        ch,
        "FROM console.bi_chart FINAL",
        chart_meta,
        vec![
            chart_row(
                "c_rows",
                "table",
                &json!({
                    "tableMode": "rows", "columns": ["email", "region", "secret"],
                    "sortColumn": "region", "sortDir": "desc",
                    "columnSettings": { "email": { "label": "E-mail" }, "secret": { "hidden": true } },
                }),
            ),
            chart_row("c_bar", "bar", &json!({ "dimension": "region", "measures": ["email"] })),
        ],
    )
    .await;
    answer(
        ch,
        "system.columns",
        json!([
            {"name": "name", "type": "String"}, {"name": "type", "type": "String"},
            {"name": "default_kind", "type": "String"}, {"name": "default_expression", "type": "String"}
        ]),
        ["email", "region", "secret"]
            .iter()
            .map(|n| json!({"name": n, "type": "String", "default_kind": "", "default_expression": ""}))
            .collect(),
    )
    .await;
    answer(
        ch,
        "ORDER BY",
        json!([{"name": "email", "type": "String"}]),
        data,
    )
    .await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({ "meta": [], "data": [], "rows": 0 })),
        )
        .mount(ch)
        .await;
}

async fn get(router: &axum::Router, uri: &str, cookie: &str) -> axum::http::Response<Body> {
    router
        .clone()
        .oneshot(
            Request::builder()
                .uri(uri)
                .header("cookie", cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
}

async fn app(ch: &MockServer) -> (axum::Router, PgPool, String) {
    let mut overrides = std::collections::HashMap::new();
    overrides.insert("CH_URL".to_owned(), ch.uri());
    let TestApp { router, pool } = spin_up_with_env(&overrides).await;
    let user = exporter(&pool).await;
    let cookie = session_cookie_for_user(&pool, user).await;
    (router, pool, cookie)
}

async fn statements(ch: &MockServer) -> Vec<String> {
    ch.received_requests()
        .await
        .unwrap()
        .iter()
        .map(|r| String::from_utf8_lossy(&r.body).into_owned())
        .collect()
}

#[tokio::test]
async fn the_export_writes_every_visible_column_masked_filtered_sorted_and_audited() {
    let ch = MockServer::start().await;
    mount(
        &ch,
        vec![
            json!({"email": "***", "region": "north"}),
            json!({"email": "=HYPERLINK(\"http://x\")", "region": "a,b"}),
        ],
    )
    .await;
    let (router, pool, cookie) = app(&ch).await;
    governance::create_policy(
        &pool,
        &CreatePolicyInput {
            name: "export-test".to_owned(),
            kind: "Row filter".to_owned(),
            subjects: "Exporter".to_owned(),
            resources: "serving.mart_x".to_owned(),
            effect: "Permit with obligation".to_owned(),
            conditions: Some(
                r#"{"roles":["Exporter"],"table":"serving.mart_x","mask":["email"]}"#.to_owned(),
            ),
            activate: true,
            owner: None,
        },
    )
    .await
    .unwrap();

    let filters = urlencoding_free(r#"[{"column":"region","values":["north","a,b"]}]"#);
    let resp = get(
        &router,
        &format!("/api/dashboard/table-export?chart=c_rows&filters={filters}"),
        &cookie,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let h = resp.headers().clone();
    assert_eq!(h["content-type"], "text/csv; charset=utf-8");
    assert_eq!(
        h["content-disposition"],
        "attachment; filename=\"Visits-by-place.csv\""
    );
    assert_eq!(h["x-export-rows"], "2");
    assert_eq!(h["x-export-cut"], "false");
    let body = String::from_utf8(
        to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    // BOM, label header (the hidden column is left out), CRLF, quoting, formula neutralised.
    assert_eq!(
        body,
        "\u{feff}E-mail,region\r\n***,north\r\n\"'=HYPERLINK(\"\"http://x\"\")\",\"a,b\"\r\n"
    );

    let sent: Vec<String> = statements(&ch)
        .await
        .into_iter()
        .filter(|s| s.contains("serving.mart_x") && !s.contains("system."))
        .collect();
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert!(
        sent[0].contains("replaceRegexpOne(toString(`email`)"),
        "mask: {}",
        sent[0]
    );
    assert!(sent[0].contains("'north'"), "dashboard filter: {}", sent[0]);
    assert!(
        sent[0].contains("ORDER BY region DESC"),
        "tile sort: {}",
        sent[0]
    );
    assert!(sent[0].contains("LIMIT 100001"), "{}", sent[0]);
    assert!(
        sent[0].starts_with("SELECT email, region FROM"),
        "a hidden column is not selected: {}",
        sent[0]
    );

    let (action, args): (String, Value) = sqlx::query_as(
        "SELECT action, args FROM audit_event WHERE action = 'dashboard.table_export'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(action, "dashboard.table_export");
    assert_eq!(args["tile"], "c_rows");
    assert_eq!(args["board"], "b1");
    assert_eq!(args["rows"], 2);
    assert_eq!(args["cut"], false);
}

/// Query strings need `%`-escapes; this keeps the test free of a dependency.
fn urlencoding_free(raw: &str) -> String {
    raw.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

#[tokio::test]
async fn a_result_beyond_the_cap_is_cut_at_100000_and_says_so() {
    let ch = MockServer::start().await;
    mount(
        &ch,
        (0..100_001)
            .map(|i| json!({"email": format!("e{i}"), "region": "r"}))
            .collect(),
    )
    .await;
    let (router, pool, cookie) = app(&ch).await;
    let resp = get(&router, "/api/dashboard/table-export?chart=c_rows", &cookie).await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(resp.headers()["x-export-rows"], "100000");
    assert_eq!(resp.headers()["x-export-cut"], "true");
    assert_eq!(resp.headers()["x-export-cap"], "100000");
    let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    assert_eq!(
        body.windows(2).filter(|w| w == b"\r\n").count(),
        100_001,
        "header + 100000 rows"
    );
    let args: Value =
        sqlx::query_scalar("SELECT args FROM audit_event WHERE action = 'dashboard.table_export'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(args["cut"], true);
    assert_eq!(args["rows"], 100_000);
}

#[tokio::test]
async fn a_tile_that_is_not_a_raw_table_or_does_not_exist_is_refused_and_nothing_is_audited() {
    let ch = MockServer::start().await;
    mount(&ch, vec![]).await;
    let (router, pool, cookie) = app(&ch).await;
    for (uri, status) in [
        ("/api/dashboard/table-export", StatusCode::BAD_REQUEST),
        (
            "/api/dashboard/table-export?chart=c_bar",
            StatusCode::BAD_REQUEST,
        ),
        (
            "/api/dashboard/table-export?chart=nope",
            StatusCode::NOT_FOUND,
        ),
        (
            "/api/dashboard/table-export?chart=c_rows&sortColumn=nope",
            StatusCode::BAD_REQUEST,
        ),
        (
            "/api/dashboard/table-export?chart=c_rows&filters=%5Bnot-json",
            StatusCode::BAD_REQUEST,
        ),
    ] {
        let resp = get(&router, uri, &cookie).await;
        assert_eq!(resp.status(), status, "{uri}");
    }
    let n: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_event WHERE action = 'dashboard.table_export'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(n, 0);
    assert!(
        !statements(&ch)
            .await
            .iter()
            .any(|s| s.contains("serving.mart_x") && s.contains("LIMIT")),
        "a refused request must not read the data"
    );
}

#[tokio::test]
async fn an_unauthenticated_request_is_401() {
    let ch = MockServer::start().await;
    let mut overrides = std::collections::HashMap::new();
    overrides.insert("CH_URL".to_owned(), ch.uri());
    let TestApp { router, .. } = spin_up_with_env(&overrides).await;
    let resp = router
        .oneshot(
            Request::builder()
                .uri("/api/dashboard/table-export?chart=c_rows")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}
