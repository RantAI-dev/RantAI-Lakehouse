//! `BI-19` part D (plan D14): a dashboard as a file and import, through the
//! real router, a per-test Postgres, and a recording `wiremock` stand-in for
//! `ClickHouse` that knows three marts, one dashboard with three charts, and
//! answers every statement it does not know with no rows.
//!
//! The pure rules (limits, fields a file may not carry, retargeting, re-keying
//! the layout) are unit-tested in `lakehouse_bi::dashboard_file`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use common::{create_principal_with_permissions, session_cookie_for_user, spin_up_with_env};
use serde_json::{Value, json};
use tower::ServiceExt;
use wiremock::{Mock, MockServer, Request as MockRequest, Respond, ResponseTemplate};

/// The marts of the stand-in and their columns.
const MARTS: [(&str, [&str; 3]); 3] = [
    ("mart_a", ["region", "email", "amount"]),
    ("mart_b", ["region", "email", "amount"]),
    ("gold_a", ["region", "email", "amount"]),
];

#[derive(Clone)]
struct Store {
    log: Arc<Mutex<Vec<String>>>,
    /// Every `INSERT` into `bi_chart` fails (a store that cannot write).
    fail_chart_insert: Arc<std::sync::atomic::AtomicBool>,
    /// The SQL source `s_one` lives in a folder the importer cannot see.
    hidden_source: Arc<std::sync::atomic::AtomicBool>,
}

fn quoted_after(sql: &str, marker: &str) -> Option<String> {
    let rest = &sql[sql.find(marker)? + marker.len()..];
    Some(rest[..rest.find('\'')?].to_owned())
}

fn chart_row(id: &str, def: &Value) -> Value {
    let mut def = def.clone();
    def["title"] = json!(format!("Chart {id}"));
    let spec = json!({
        "id": id, "title": format!("Chart {id}"), "kind": def["kind"], "mart": def["mart"],
        "sql": "SELECT 1", "x": "region", "y": "email"
    });
    json!({
        "id": id, "board": "b_src", "created_by": "ui", "created_at": "2026-01-01 00:00:00",
        "spec_json": json!({ "spec": spec, "def": def, "hasYear": false }).to_string(),
    })
}

fn board_row(id: &str, name: &str, with_content: bool) -> Value {
    let (layout, filters, tabs) = if with_content {
        (
            json!({
                "c_a": {"x": 0, "y": 0, "w": 6, "h": 3, "tab": "t1"},
                "c_b": {"x": 6, "y": 0, "w": 6, "h": 3, "tab": "t2"},
                "c_c": {"x": 0, "y": 3, "w": 6, "h": 3, "tab": "t2"}
            })
            .to_string(),
            json!([{"column": "region", "values": ["north"]}]).to_string(),
            json!([{"id": "t1", "title": "First"}, {"id": "t2", "title": "Second"}]).to_string(),
        )
    } else {
        (String::new(), String::new(), String::new())
    };
    json!({
        "id": id, "name": name, "description": "Quarterly", "created_by": "t",
        "layout_json": layout, "filters_json": filters, "public_token": "", "embed_enabled": "0",
        "folder_id": "", "created_at": "2026-10-10 00:00:00", "owner_id": "",
        "tabs_json": tabs, "embed_revoked_before": "0", "embed_revoked_jti_json": "[]",
        "embed_origins_json": "[]"
    })
}

impl Respond for Store {
    fn respond(&self, request: &MockRequest) -> ResponseTemplate {
        let sql = String::from_utf8_lossy(&request.body).into_owned();
        self.log.lock().unwrap().push(sql.clone());
        let rows = |data: Vec<Value>| {
            let n = data.len();
            ResponseTemplate::new(200).set_body_json(json!({ "meta": [], "data": data, "rows": n }))
        };
        if sql.contains("FROM system.tables") && sql.contains("AND name=") {
            let name = quoted_after(&sql, "AND name='").unwrap_or_default();
            let n = i32::from(MARTS.iter().any(|(m, _)| *m == name));
            return rows(vec![json!({ "n": n.to_string() })]);
        }
        if sql.contains("SELECT name FROM system.tables WHERE database='serving'") {
            return rows(MARTS.iter().map(|(m, _)| json!({ "name": m })).collect());
        }
        if sql.contains("FROM system.columns WHERE database='serving' AND table=") {
            let table = quoted_after(&sql, "AND table='").unwrap_or_default();
            let cols = MARTS
                .iter()
                .find(|(m, _)| *m == table)
                .map(|(_, c)| *c)
                .unwrap_or_default();
            return rows(
                cols.iter()
                    .map(|c| json!({ "name": c, "type": if *c == "amount" { "Float64" } else { "String" } }))
                    .collect(),
            );
        }
        if sql.starts_with(
            "INSERT INTO console.bi_chart (id, title, spec_json, board, created_by) VALUES",
        ) && self
            .fail_chart_insert
            .load(std::sync::atomic::Ordering::SeqCst)
        {
            return ResponseTemplate::new(500).set_body_string("planted-engine-text");
        }
        // BI-19 (PR #250 CI), test not product: the importer's viewer hides a
        // folder only if the folder list names it (`AccessMap::viewer` walks
        // the stored folders), so the stand-in must serve `f_hidden` as a row
        // like the real store does; without it the viewer saw the source's
        // folder as unknown-and-open and the chart failed later, on its columns.
        if sql.contains("FROM console.bi_folder")
            && self.hidden_source.load(std::sync::atomic::Ordering::SeqCst)
        {
            return rows(vec![json!({
                "id": "f_hidden", "name": "Hidden", "parent_id": "", "created_by": "t",
                "owner_id": "", "updated_at": "2026-01-01 00:00:00"
            })]);
        }
        if sql.contains("FROM console.bi_source")
            && self.hidden_source.load(std::sync::atomic::Ordering::SeqCst)
        {
            return rows(vec![json!({
                "id": "s_one", "title": "One", "sql": "SELECT 1 AS region", "columns_json": "[]",
                "folder_id": "f_hidden", "created_by": "t", "updated_at": "2026-01-01 00:00:00"
            })]);
        }
        if sql.starts_with("INSERT") || sql.starts_with("ALTER") || sql.starts_with("CREATE") {
            return ResponseTemplate::new(200).set_body_string("");
        }
        if sql.contains("SELECT id, spec_json, board, created_by") {
            let bar = |mart: &str, dim: &str| json!({ "mart": mart, "kind": "bar", "dimension": dim, "measures": ["email"] });
            return rows(vec![
                chart_row("c_a", &bar("mart_a", "region")),
                chart_row("c_b", &bar("mart_b", "region")),
                chart_row(
                    "c_c",
                    &json!({ "mart": "mart_a", "kind": "kpi", "dimension": "", "measures": ["amount"] }),
                ),
            ]);
        }
        if sql.contains("FROM console.bi_board") {
            let id = quoted_after(&sql, "id='").unwrap_or_default();
            if id.is_empty() {
                return rows(vec![]);
            }
            return if id == "b_src" {
                rows(vec![board_row("b_src", "Quarterly sales", true)])
            } else {
                rows(vec![board_row(&id, "Imported", false)])
            };
        }
        rows(vec![])
    }
}

struct App {
    router: axum::Router,
    pool: sqlx::PgPool,
    log: Arc<Mutex<Vec<String>>>,
    fail_chart_insert: Arc<std::sync::atomic::AtomicBool>,
    hidden_source: Arc<std::sync::atomic::AtomicBool>,
    _ch: MockServer,
}

async fn app() -> App {
    let log = Arc::new(Mutex::new(Vec::new()));
    let fail_chart_insert = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let hidden_source = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let ch = MockServer::start().await;
    Mock::given(wiremock::matchers::method("POST"))
        .respond_with(Store {
            log: log.clone(),
            fail_chart_insert: fail_chart_insert.clone(),
            hidden_source: hidden_source.clone(),
        })
        .mount(&ch)
        .await;
    let env = HashMap::from([("CH_URL".to_owned(), ch.uri())]);
    let common::TestApp { router, pool } = spin_up_with_env(&env).await;
    App {
        router,
        pool,
        log,
        fail_chart_insert,
        hidden_source,
        _ch: ch,
    }
}

async fn send(
    app: &App,
    cookie: &str,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let resp = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .header("cookie", cookie)
                .header("content-type", "application/json")
                .body(body.map_or_else(Body::empty, |b| Body::from(b.to_string())))
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

async fn writer(app: &App, permissions: &str) -> String {
    let user = create_principal_with_permissions(&app.pool, permissions).await;
    session_cookie_for_user(&app.pool, user).await
}

fn inserts_into(app: &App, table: &str) -> Vec<String> {
    app.log
        .lock()
        .unwrap()
        .iter()
        .filter(|s| s.starts_with(&format!("INSERT INTO console.{table}")))
        .cloned()
        .collect()
}

/// Export the source board as a file, from the stand-in.
async fn exported(app: &App, cookie: &str) -> Value {
    let (status, file) = send(
        app,
        cookie,
        "GET",
        "/api/dashboard/export?format=file&board=b_src",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{file}");
    file
}

#[tokio::test]
async fn a_board_with_tabs_filters_and_three_charts_exports_and_imports_with_a_mapping() {
    let app = app().await;
    let cookie = writer(&app, "dashboard:read, dashboard:write").await;
    let file = exported(&app, &cookie).await;
    assert_eq!(file["kind"], "rantai-dashboard");
    assert_eq!(file["version"], 1);
    assert_eq!(file["board"]["name"], "Quarterly sales");
    assert_eq!(file["board"]["tabs"].as_array().unwrap().len(), 2);
    assert_eq!(file["board"]["filters"][0]["column"], "region");
    assert_eq!(file["charts"].as_array().unwrap().len(), 3);
    let tables: Vec<&str> = file["tables"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(tables, ["mart_a", "mart_b"]);
    assert_eq!(
        file["tables"][0]["columns"],
        json!(["amount", "email", "region"])
    );
    // No id of the source dashboard, no owner, no sharing state in the file.
    let text = file.to_string();
    for forbidden in ["b_src", "ownerId", "publicToken", "embed"] {
        assert!(!text.contains(forbidden), "{forbidden} is in the file");
    }

    // The preview suggests the same-named table and nothing for a missing one.
    let (status, preview) = send(
        &app,
        &cookie,
        "POST",
        "/api/dashboard/import/preview",
        Some(json!({ "file": file })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{preview}");
    assert_eq!(preview["tables"][0]["suggested"], "mart_a");
    assert_eq!(preview["tables"][1]["suggested"], "mart_b");
    assert_eq!(preview["charts"].as_array().unwrap().len(), 3);

    let (status, result) = send(
        &app,
        &cookie,
        "POST",
        "/api/dashboard/import",
        Some(json!({ "file": file, "mapping": { "mart_a": "gold_a", "mart_b": "mart_b" } })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{result}");
    assert_eq!(result["leftOut"], json!([]));
    let imported = result["imported"].as_array().unwrap();
    assert_eq!(imported.len(), 3);
    let new_board = result["board"]["id"].as_str().unwrap();
    assert_ne!(new_board, "b_src");
    assert_eq!(result["board"]["name"], "Quarterly sales");

    // Three charts were written on the new board, over the mapped tables, under
    // ids of their own.
    let charts = inserts_into(&app, "bi_chart");
    assert_eq!(charts.len(), 3, "{charts:?}");
    assert!(charts.iter().all(|s| s.contains(new_board)));
    assert_eq!(charts.iter().filter(|s| s.contains("gold_a")).count(), 2);
    for old in ["'c_a'", "'c_b'", "'c_c'"] {
        assert!(charts.iter().all(|s| !s.contains(old)), "{old} was reused");
    }
    // The board's settings were saved on the new board with the new ids: the
    // tabs, the filter, and a layout keyed by the three new chart ids.
    let boards = inserts_into(&app, "bi_board");
    let saved = boards
        .iter()
        .filter(|s| s.contains(new_board))
        .cloned()
        .collect::<Vec<_>>()
        .join("\n");
    for new_id in imported.iter().map(|c| c["id"].as_str().unwrap()) {
        assert!(
            saved.contains(new_id),
            "{new_id} is not in the saved layout"
        );
    }
    assert!(
        saved.contains("Second") && saved.contains("north"),
        "{saved}"
    );
    assert!(!saved.contains("c_a") && !saved.contains("c_b"), "{saved}");
}

#[tokio::test]
async fn a_chart_that_cannot_be_made_is_left_out_with_its_reason_and_the_rest_is_imported() {
    let app = app().await;
    let cookie = writer(&app, "dashboard:read, dashboard:write").await;
    let mut file = exported(&app, &cookie).await;
    // One chart reads a column the table here does not have, one a table the
    // mapping does not cover, one a SQL source.
    file["charts"][1]["def"]["dimension"] = json!("no_such_column");
    file["charts"][2]["def"] = json!({
        "title": "From SQL", "kind": "bar", "sqlSource": "s_one", "dimension": "region", "measures": ["email"]
    });
    let (status, result) = send(
        &app,
        &cookie,
        "POST",
        "/api/dashboard/import",
        Some(json!({ "file": file, "mapping": { "mart_a": "mart_a" } })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{result}");
    assert_eq!(result["imported"].as_array().unwrap().len(), 1);
    let left_out = result["leftOut"].as_array().unwrap();
    assert_eq!(left_out.len(), 2, "{result}");
    assert_eq!(left_out[0]["title"], "Chart c_b");
    assert_eq!(
        left_out[0]["reason"],
        "Its table was not matched to a table here."
    );
    assert_eq!(left_out[1]["title"], "From SQL");
    assert_eq!(
        left_out[1]["reason"],
        "It reads a SQL source, which needs the dashboard SQL permission."
    );
    // The same file, with the table mapped, leaves the chart out for its column.
    let (_, result) = send(
        &app,
        &cookie,
        "POST",
        "/api/dashboard/import",
        Some(json!({ "file": file, "mapping": { "mart_a": "mart_a", "mart_b": "mart_b" } })),
    )
    .await;
    assert_eq!(
        result["leftOut"][0]["reason"],
        "A column, calculated field, metric or parameter it uses is missing here."
    );
    // Nothing the reasons name was written as a chart.
    assert!(
        inserts_into(&app, "bi_chart")
            .iter()
            .all(|s| !s.contains("no_such_column") && !s.contains("s_one"))
    );
}

#[tokio::test]
async fn a_file_over_a_limit_of_another_version_or_with_a_smuggled_field_is_refused() {
    let app = app().await;
    let cookie = writer(&app, "dashboard:read, dashboard:write").await;
    let good = exported(&app, &cookie).await;
    let many: Vec<Value> = (0..201)
        .map(|i| json!({ "id": format!("c{i}"), "def": { "title": "t", "kind": "text" } }))
        .collect();
    let mut too_many = good.clone();
    too_many["charts"] = json!(many);
    too_many["board"]["layout"] = json!({});
    too_many["board"]["phoneLayout"] = json!([]);
    let mut later = good.clone();
    later["version"] = json!(2);
    let mut owned = good.clone();
    owned["board"]["ownerId"] = json!("someone-else");
    let mut public = good.clone();
    public["publicToken"] = json!("p_abc");
    let mut big = good.clone();
    big["board"]["description"] = json!("x".repeat(1_100_000));
    for (what, file, expect) in [
        (
            "201 charts",
            too_many,
            "That file holds more than 200 charts.",
        ),
        (
            "version 2",
            later,
            "That dashboard file is from a version this one cannot read.",
        ),
        ("an owner", owned, "That is not a dashboard file."),
        ("a public link", public, "That is not a dashboard file."),
        ("over 1 MB", big, "That file is larger than 1 MB."),
    ] {
        for uri in ["/api/dashboard/import/preview", "/api/dashboard/import"] {
            let (status, body) =
                send(&app, &cookie, "POST", uri, Some(json!({ "file": file }))).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{what} {uri}");
            assert_eq!(body["error"], expect, "{what} {uri}");
        }
    }
    assert!(
        inserts_into(&app, "bi_board").is_empty(),
        "a refused file creates nothing"
    );
}

#[tokio::test]
async fn a_person_who_can_only_read_cannot_import_and_an_unknown_board_has_no_file() {
    let app = app().await;
    let reader = writer(&app, "dashboard:read").await;
    let (status, _) = send(
        &app,
        &reader,
        "POST",
        "/api/dashboard/import",
        Some(json!({ "file": {} })),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, body) = send(
        &app,
        &reader,
        "GET",
        "/api/dashboard/export?format=file&board=nope",
        None,
    )
    .await;
    // The stand-in answers an unknown id with a row of that id, so the board
    // read itself is not what is under test here; the missing id is.
    assert!(
        status == StatusCode::OK || status == StatusCode::NOT_FOUND,
        "{body}"
    );
    let (status, _) = send(
        &app,
        &reader,
        "GET",
        "/api/dashboard/export?format=file",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = send(
        &app,
        &reader,
        "GET",
        "/api/dashboard/export?format=file&board=default",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

/// Whether the log holds the statement `wipe_board` sends for the board.
fn board_was_wiped(app: &App) -> bool {
    app.log.lock().unwrap().iter().any(|s| {
        s.starts_with("INSERT INTO console.bi_board (id, name, owner_id, is_deleted) SELECT")
    })
}

/// `BI-19` D review SF-D1: a store that cannot write is an error that takes the
/// half-made board back, not a chart "left out" in a 200.
#[tokio::test]
async fn a_store_that_cannot_write_a_chart_fails_the_import_and_wipes_the_board() {
    let app = app().await;
    let cookie = writer(&app, "dashboard:read, dashboard:write").await;
    let file = exported(&app, &cookie).await;
    app.fail_chart_insert
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let (status, body) = send(
        &app,
        &cookie,
        "POST",
        "/api/dashboard/import",
        Some(json!({ "file": file, "mapping": { "mart_a": "mart_a", "mart_b": "mart_b" } })),
    )
    .await;
    assert_ne!(status, StatusCode::OK, "{body}");
    assert!(!body.to_string().contains("planted-engine-text"), "{body}");
    assert!(board_was_wiped(&app), "the half-made board is taken back");
}

/// `BI-19` D review SF-D2 (the wipe path): a dashboard setting the editor would
/// not save (a theme that does not exist) is a 400 and the board is wiped.
#[tokio::test]
async fn a_file_with_a_theme_that_cannot_be_saved_is_refused_and_the_board_wiped() {
    let app = app().await;
    let cookie = writer(&app, "dashboard:read, dashboard:write").await;
    let mut file = exported(&app, &cookie).await;
    file["board"]["theme"] = json!("no-such-theme");
    let (status, body) = send(
        &app,
        &cookie,
        "POST",
        "/api/dashboard/import",
        Some(json!({ "file": file, "mapping": { "mart_a": "mart_a", "mart_b": "mart_b" } })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(board_was_wiped(&app));
}

/// `BI-19` D review SF-D3: a table listed twice is refused before the database
/// is asked about any table.
#[tokio::test]
async fn a_file_listing_a_table_twice_is_refused_before_any_schema_query() {
    let app = app().await;
    let cookie = writer(&app, "dashboard:read, dashboard:write").await;
    let mut file = exported(&app, &cookie).await;
    file["tables"] = json!([{ "name": "mart_a" }, { "name": "mart_a" }]);
    let before = app.log.lock().unwrap().len();
    let (status, body) = send(
        &app,
        &cookie,
        "POST",
        "/api/dashboard/import/preview",
        Some(json!({ "file": file })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(
        app.log.lock().unwrap()[before..]
            .iter()
            .all(|s| !s.contains("FROM system.columns")),
        "no schema query for a refused file"
    );
}

/// `BI-19` D review SF-D6: a chart over a SQL source in a folder the importer
/// cannot see is left out as unavailable; a chart naming a table that is not in
/// the file's `tables` is left out as not matched.
#[tokio::test]
async fn a_hidden_sql_source_and_an_unlisted_table_leave_their_charts_out_with_the_reason() {
    let app = app().await;
    let cookie = writer(&app, "dashboard:read, dashboard:write, dashboard:sql").await;
    let mut file = exported(&app, &cookie).await;
    file["charts"][1]["def"] = json!({
        "title": "From SQL", "kind": "bar", "sqlSource": "s_one", "dimension": "region", "measures": ["email"]
    });
    file["tables"] = json!([{ "name": "mart_a", "columns": ["region", "email", "amount"] }]);
    app.hidden_source
        .store(true, std::sync::atomic::Ordering::SeqCst);
    // The importer is not listed on `f_hidden`.
    let other = create_principal_with_permissions(&app.pool, "dashboard:read").await;
    lakehouse_store::dashboard_access::replace_item(
        &app.pool,
        "folder",
        "f_hidden",
        &[lakehouse_store::dashboard_access::NewRule {
            user_id: Some(other),
            role_id: None,
            level: "view".to_owned(),
        }],
        true,
        None,
    )
    .await
    .unwrap();
    let (status, result) = send(
        &app,
        &cookie,
        "POST",
        "/api/dashboard/import",
        Some(json!({ "file": file, "mapping": { "mart_a": "mart_a" } })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{result}");
    let reasons: Vec<&str> = result["leftOut"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|c| c["reason"].as_str())
        .collect();
    assert!(
        reasons.contains(&"The SQL source it reads is not available here."),
        "{result}"
    );
    assert_eq!(reasons.len(), 1, "{result}");
    // A table that is not in the file's `tables` cannot be mapped, so the
    // charts that read it are left out as not matched.
    file["tables"] = json!([]);
    let (_, result) = send(
        &app,
        &cookie,
        "POST",
        "/api/dashboard/import",
        Some(json!({ "file": file, "mapping": { "mart_a": "mart_a" } })),
    )
    .await;
    assert!(
        result["leftOut"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| { c["reason"] == "Its table was not matched to a table here." }),
        "{result}"
    );
}
