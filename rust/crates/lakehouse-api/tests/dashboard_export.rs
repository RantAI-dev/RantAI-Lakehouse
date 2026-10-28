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
            // BI-6: raw tables with filters of their own.
            chart_row(
                "c_own",
                "table",
                &json!({
                    "tableMode": "rows", "columns": ["email", "region"],
                    "filters": [{ "column": "region", "values": ["south", "east"] }],
                }),
            ),
            // BI-7: a raw table that joins a second table.
            chart_row(
                "c_join",
                "table",
                &json!({
                    "tableMode": "rows", "columns": ["email", "mart_y__email", "mart_y__secret"],
                    "joins": [{
                        "mart": "mart_y", "type": "left",
                        "on": [{ "left": "region", "op": "=", "right": "region" }],
                    }],
                }),
            ),
            // BI-1: a raw table with a numeric column that has a value format.
            chart_row(
                "c_amount",
                "table",
                &json!({
                    "tableMode": "rows", "columns": ["email", "amount"],
                    "valueFormats": {
                        "amount": { "style": "compact", "currency": "IDR", "decimals": 2 }
                    },
                }),
            ),
            chart_row("c_text", "text", &json!({ "text": "hello" })),
            chart_row(
                "c_gone",
                "table",
                &json!({
                    "tableMode": "rows", "columns": ["email", "region"],
                    "filters": [{ "column": "gone", "values": ["x"] }],
                }),
            ),
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
        ["email", "region", "secret", "amount"]
            .iter()
            .map(|n| json!({"name": n, "type": "String", "default_kind": "", "default_expression": ""}))
            .collect(),
    )
    .await;
    // The rows of the file arrive as a stream: names, types, then one array a row.
    let names: Vec<String> = data.first().and_then(Value::as_object).map_or_else(
        || vec!["email".to_owned(), "region".to_owned()],
        |first| first.keys().cloned().collect(),
    );
    let mut body = format!("{}\n{}\n", json!(names), json!(vec!["String"; names.len()]));
    for row in &data {
        let cells: Vec<&Value> = names.iter().map(|n| &row[n]).collect();
        body.push_str(&json!(cells).to_string());
        body.push('\n');
    }
    Mock::given(method("POST"))
        .and(body_string_contains("JSONCompactEachRowWithNamesAndTypes"))
        .respond_with(ResponseTemplate::new(200).set_body_string(body))
        .mount(ch)
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
    assert!(sent[0].contains("LIMIT 1000001"), "{}", sent[0]);
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

/// BI-6: a raw table's own filters narrow its file as they narrow the tile, ahead
/// of the dashboard's, under the same mask and row filter; one that no longer
/// applies refuses the export and nothing is read or audited.
#[tokio::test]
async fn the_export_carries_the_tiles_own_filters_and_refuses_one_that_no_longer_applies() {
    let ch = MockServer::start().await;
    mount(&ch, vec![json!({"email": "***", "region": "south"})]).await;
    let (router, pool, cookie) = app(&ch).await;
    governance::create_policy(
        &pool,
        &CreatePolicyInput {
            name: "export-own-test".to_owned(),
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

    let filters = urlencoding_free(r#"[{"column":"region","values":["south","west"]}]"#);
    let resp = get(
        &router,
        &format!("/api/dashboard/table-export?chart=c_own&filters={filters}"),
        &cookie,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let sent: Vec<String> = statements(&ch)
        .await
        .into_iter()
        .filter(|s| s.contains("serving.mart_x") && !s.contains("system."))
        .collect();
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert!(
        sent[0].contains("region IN ('south', 'east') AND region IN ('south', 'west')"),
        "own filters ahead of the dashboard's: {}",
        sent[0]
    );
    assert!(
        sent[0].contains("replaceRegexpOne(toString(`email`)"),
        "mask: {}",
        sent[0]
    );

    let before = statements(&ch).await.len();
    let refused = get(&router, "/api/dashboard/table-export?chart=c_gone", &cookie).await;
    assert_eq!(refused.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let after = statements(&ch).await;
    assert!(
        !after[before..]
            .iter()
            .any(|s| s.contains("serving.mart_x") && !s.contains("system.")),
        "a refused export reads nothing: {after:?}"
    );
    let audited: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_event WHERE action = 'dashboard.table_export'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        audited, 1,
        "only the export that was handed over is audited"
    );
}

/// BI-7 (`BI-7-AC4`, `BI-7-AC2`): the file of a raw table that joins a second
/// table lists the joined columns, and each table is read through its own
/// policy: a mask on a column of the joined table and a row filter on it, with
/// a different mask on the first table, all in the one statement the engine
/// receives.
#[tokio::test]
async fn the_export_of_a_joined_table_lists_joined_columns_and_governs_each_table() {
    let ch = MockServer::start().await;
    // The all-marts column read comes first so it wins over the generic answer.
    answer(
        &ch,
        "SELECT table, name, type FROM system.columns",
        json!([
            {"name": "table", "type": "String"}, {"name": "name", "type": "String"},
            {"name": "type", "type": "String"}
        ]),
        ["mart_x", "mart_y"]
            .iter()
            .flat_map(|t| {
                ["email", "region", "secret"]
                    .iter()
                    .map(move |n| json!({"table": t, "name": n, "type": "String"}))
            })
            .collect(),
    )
    .await;
    mount(
        &ch,
        vec![json!({"email": "***", "mart_y__email": "b@x", "mart_y__secret": "***"})],
    )
    .await;
    let (router, pool, cookie) = app(&ch).await;
    for (table, mask, filter) in [
        ("mart_x", r#"["email"]"#, ""),
        (
            "mart_y",
            r#"["secret"]"#,
            r#","rowFilter":"region = 'south'""#,
        ),
    ] {
        governance::create_policy(
            &pool,
            &CreatePolicyInput {
                name: format!("join-{table}"),
                kind: "Row filter".to_owned(),
                subjects: "Exporter".to_owned(),
                resources: format!("serving.{table}"),
                effect: "Permit with obligation".to_owned(),
                conditions: Some(format!(
                    r#"{{"roles":["Exporter"],"table":"serving.{table}","mask":{mask}{filter}}}"#
                )),
                activate: true,
                owner: None,
            },
        )
        .await
        .unwrap();
    }

    let resp = get(&router, "/api/dashboard/table-export?chart=c_join", &cookie).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = String::from_utf8(
        to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    // The file starts with a byte-order mark (see the export route).
    assert!(
        body.trim_start_matches('\u{feff}')
            .starts_with("email,mart_y__email,mart_y__secret"),
        "{body}"
    );
    let sent: Vec<String> = statements(&ch)
        .await
        .into_iter()
        .filter(|s| s.contains("JOIN") && !s.contains("system."))
        .collect();
    assert_eq!(sent.len(), 1, "{sent:?}");
    let sql = &sent[0];
    assert!(sql.contains("LEFT JOIN"), "{sql}");
    assert!(
        sql.contains("replaceRegexpOne(toString(`email`)"),
        "first table masked: {sql}"
    );
    assert!(
        sql.contains("replaceRegexpOne(toString(`secret`)"),
        "joined table masked: {sql}"
    );
    assert!(
        sql.contains("WHERE (region = 'south')"),
        "joined table row-filtered: {sql}"
    );
    assert_eq!(
        sql.matches("FROM serving.mart_").count(),
        2,
        "each table is read once, only inside its projection: {sql}"
    );
    assert!(sql.contains("join_use_nulls = 1"), "{sql}");
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

/// `BI-1` T2 (feature decision 8): the cap is a million rows now; a result
/// below it is not cut and says what the cap is. The cut itself (1,000,001
/// generated rows) is proved against the writer in `export.rs`, because a mock
/// engine answering a million lines would only measure the mock.
#[tokio::test]
async fn the_cap_is_a_million_rows_and_a_smaller_result_is_not_cut() {
    let ch = MockServer::start().await;
    mount(
        &ch,
        (0..1_000)
            .map(|i| json!({"email": format!("e{i}"), "region": "r"}))
            .collect(),
    )
    .await;
    let (router, pool, cookie) = app(&ch).await;
    let resp = get(&router, "/api/dashboard/table-export?chart=c_rows", &cookie).await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(resp.headers()["x-export-rows"], "1000");
    assert_eq!(resp.headers()["x-export-cut"], "false");
    assert_eq!(resp.headers()["x-export-cap"], "1000000");
    let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    assert_eq!(
        body.windows(2).filter(|w| w == b"\r\n").count(),
        1_001,
        "header + 1000 rows"
    );
    let args: Value =
        sqlx::query_scalar("SELECT args FROM audit_event WHERE action = 'dashboard.table_export'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(args["cut"], false);
    assert_eq!(args["rows"], 1_000);
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

/// `BI-1` T1 (D7): a person whose every role is switched off cannot download,
/// is told so in a sentence, and nothing is read from the engine for them; a
/// person who holds another role still can.
#[tokio::test]
async fn a_person_whose_every_role_is_switched_off_cannot_export() {
    let ch = MockServer::start().await;
    mount(&ch, vec![json!({"email": "a", "region": "north"})]).await;
    let (router, pool, cookie) = app(&ch).await;
    sqlx::query(
        "INSERT INTO reporting_settings (singleton, time_zone, week_start, download_denied_roles) \
         VALUES (TRUE, 'UTC', 'monday', ARRAY['Exporter'])",
    )
    .execute(&pool)
    .await
    .unwrap();
    let before = ch.received_requests().await.unwrap().len();
    let resp = get(&router, "/api/dashboard/table-export?chart=c_rows", &cookie).await;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    let body: Value =
        serde_json::from_slice(&to_bytes(resp.into_body(), 1 << 20).await.unwrap()).unwrap();
    assert!(
        body["error"]
            .as_str()
            .is_some_and(|e| e.contains("Downloads are switched off for your role.")),
        "{body}"
    );
    assert_eq!(
        ch.received_requests().await.unwrap().len(),
        before,
        "a refused download reads nothing"
    );

    // The same person with a second role that is not switched off may.
    let extra = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO role (id, name, permissions, description) VALUES ($1, 'Second', '', 'test')",
    )
    .bind(extra)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO app_user_role (user_id, role_id) SELECT user_id, $1 FROM app_user_role ur JOIN role r ON r.id = ur.role_id WHERE r.name = 'Exporter'")
        .bind(extra)
        .execute(&pool)
        .await
        .unwrap();
    let (user,): (Uuid,) = sqlx::query_as(
        "SELECT ur.user_id FROM app_user_role ur JOIN role r ON r.id = ur.role_id WHERE r.name = 'Second'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let cookie = session_cookie_for_user(&pool, user).await;
    let resp = get(&router, "/api/dashboard/table-export?chart=c_rows", &cookie).await;
    assert_eq!(resp.status(), StatusCode::OK);
}

/// `BI-1` T2: a masked column is masked in the Excel file, the number column
/// stays a number in the cell with the chart's number format, and the same
/// tile as "formatted" CSV holds the text the chart shows. Audited with the
/// file kind and the value mode.
#[tokio::test]
async fn the_excel_download_is_masked_keeps_numbers_as_numbers_and_is_audited() {
    use std::io::Read;

    let ch = MockServer::start().await;
    mount(&ch, vec![json!({"email": "***", "amount": 1_234_567})]).await;
    let (router, pool, cookie) = app(&ch).await;
    governance::create_policy(
        &pool,
        &CreatePolicyInput {
            name: "xlsx-mask".to_owned(),
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

    let resp = get(
        &router,
        "/api/dashboard/chart-export?chart=c_amount&format=xlsx&values=formatted",
        &cookie,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let h = resp.headers().clone();
    assert_eq!(
        h["content-type"],
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
    );
    assert_eq!(
        h["content-disposition"],
        "attachment; filename=\"Visits-by-place.xlsx\""
    );
    assert_eq!(h["x-export-rows"], "1");
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    assert_eq!(h["content-length"], bytes.len().to_string().as_str());
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes.to_vec())).unwrap();
    let mut sheet = String::new();
    zip.by_name("xl/worksheets/sheet1.xml")
        .unwrap()
        .read_to_string(&mut sheet)
        .unwrap();
    assert!(sheet.contains("<v>1234567</v>"), "a number cell: {sheet}");
    let mut styles = String::new();
    zip.by_name("xl/styles.xml")
        .unwrap()
        .read_to_string(&mut styles)
        .unwrap();
    assert!(
        styles.contains("IDR "),
        "the chart's number format: {styles}"
    );

    let sent: Vec<String> = statements(&ch)
        .await
        .into_iter()
        .filter(|s| s.contains("serving.mart_x") && !s.contains("system."))
        .collect();
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert!(
        sent[0].contains("replaceRegexpOne(toString(`email`)"),
        "the mask is in the statement that fed the file: {}",
        sent[0]
    );
    assert!(
        sent[0].contains("JSONCompactEachRowWithNamesAndTypes"),
        "{}",
        sent[0]
    );

    let rows: Vec<(String, Value)> = sqlx::query_as(
        "SELECT action, args FROM audit_event WHERE action = 'dashboard.chart_export' ORDER BY at",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].1["format"], "xlsx");
    assert_eq!(rows[0].1["values"], "formatted");
    assert_eq!(rows[0].1["rows"], 1);
}

/// `BI-1` T2: the same tile as CSV holds the text the chart shows when
/// "formatted" and the plain digits when "raw".
#[tokio::test]
async fn the_csv_download_is_formatted_or_raw_as_asked() {
    let ch = MockServer::start().await;
    mount(&ch, vec![json!({"email": "***", "amount": 1_234_567})]).await;
    let (router, _pool, cookie) = app(&ch).await;
    let resp = get(
        &router,
        "/api/dashboard/chart-export?chart=c_amount&format=csv&values=formatted",
        &cookie,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let text = String::from_utf8(
        to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    assert_eq!(text, "\u{feff}email,amount\r\n***,IDR 1.23 M\r\n");
    let raw = get(
        &router,
        "/api/dashboard/chart-export?chart=c_amount&format=csv&values=raw",
        &cookie,
    )
    .await;
    let text = String::from_utf8(
        to_bytes(raw.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    assert_eq!(text, "\u{feff}email,amount\r\n***,1234567\r\n");
}

/// `BI-1` T2: a grouped chart downloads its own result through the statement
/// the tile reads; a text tile, an unknown format and a switched-off role are
/// refused and nothing is audited.
#[tokio::test]
async fn a_grouped_chart_downloads_and_the_refusals_read_and_audit_nothing() {
    let ch = MockServer::start().await;
    // The all-marts column read comes first so it wins over the generic answer.
    answer(
        &ch,
        "SELECT table, name, type FROM system.columns",
        json!([
            {"name": "table", "type": "String"}, {"name": "name", "type": "String"},
            {"name": "type", "type": "String"}
        ]),
        ["email", "region", "secret"]
            .iter()
            .map(|n| json!({"table": "mart_x", "name": n, "type": "String"}))
            .collect(),
    )
    .await;
    mount(&ch, vec![json!({"region": "north", "email": 3})]).await;
    let (router, pool, cookie) = app(&ch).await;
    let resp = get(
        &router,
        "/api/dashboard/chart-export?chart=c_bar&format=csv&values=raw",
        &cookie,
    )
    .await;
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    assert_eq!(
        status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&bytes)
    );
    let text = String::from_utf8(bytes.to_vec()).unwrap();
    assert_eq!(text, "\u{feff}region,email\r\nnorth,3\r\n");
    let sent = statements(&ch).await;
    assert!(
        sent.iter()
            .any(|s| s.contains("SELECT stored") && s.contains("JSONCompactEachRow")),
        "the statement the tile stores, streamed: {sent:?}"
    );

    for (uri, status) in [
        (
            "/api/dashboard/chart-export?chart=c_text&format=csv",
            StatusCode::BAD_REQUEST,
        ),
        (
            "/api/dashboard/chart-export?chart=c_bar&format=pdf",
            StatusCode::BAD_REQUEST,
        ),
        (
            "/api/dashboard/chart-export?chart=c_bar&values=rounded",
            StatusCode::BAD_REQUEST,
        ),
        (
            "/api/dashboard/chart-export?chart=nope&format=csv",
            StatusCode::NOT_FOUND,
        ),
        ("/api/dashboard/chart-export", StatusCode::BAD_REQUEST),
    ] {
        let resp = get(&router, uri, &cookie).await;
        assert_eq!(resp.status(), status, "{uri}");
    }
    sqlx::query(
        "INSERT INTO reporting_settings (singleton, time_zone, week_start, download_denied_roles) \
         VALUES (TRUE, 'UTC', 'monday', ARRAY['Exporter'])",
    )
    .execute(&pool)
    .await
    .unwrap();
    let resp = get(
        &router,
        "/api/dashboard/chart-export?chart=c_bar&format=xlsx",
        &cookie,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    let audited: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_event WHERE action = 'dashboard.chart_export'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(audited, 1, "only the download that was handed over");
}
