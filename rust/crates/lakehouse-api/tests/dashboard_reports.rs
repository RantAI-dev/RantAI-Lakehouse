//! Reports (`BI-18` part D, T1 and T2; feature page
//! `docs/core/features/reports-and-templates.md`, decisions R1 to R4): saved
//! and read back through the real router, a per-test Postgres (the reports,
//! the access rules, the policies) and a recording `wiremock` stand-in for
//! `ClickHouse` that knows three folders (one restricted, one a personal
//! space), two dashboards and their charts, and answers the folder and
//! dashboard reads through the viewer's own `IN` / `NOT IN` lists.
//!
//! The block rules themselves (kinds, limits, links) are unit-tested in
//! `lakehouse_bi::reports`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use crate::common;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use common::{TestApp, session_cookie_for_user, spin_up_with_env};
use lakehouse_embed::{RenderClaims, sign_render, unix_now};
use lakehouse_store::governance::{self, CreatePolicyInput};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request as MockRequest, Respond, ResponseTemplate};

/// The owner of the personal space `p_owner`, fixed so the stand-in can answer
/// the owner clause before the app exists.
const OWNER: &str = "0b0b0b0b-0000-4000-8000-000000000001";

#[derive(Clone)]
struct FakeCh {
    log: Arc<Mutex<Vec<String>>>,
    /// Charts moved to the trash since the fixture was made (PR #528 review
    /// SHOULD-FIX 1): left out of every live read, and `is_deleted = 2` in the
    /// by-id row state.
    trashed: Arc<Mutex<Vec<String>>>,
    /// Map-backed charts `(id, board, map_id)` registered per test (`PR #528
    /// review BLOCKER`).
    map_charts: Arc<Mutex<Vec<(String, String, String)>>>,
}

fn chart_row(id: &str, board: &str, kind: &str, map: Option<&str>) -> Value {
    let mut spec = json!({
        "id": id, "title": format!("Tile {id}"), "kind": kind, "mart": "mart_x",
        "sql": if kind == "text" { String::new() } else {
            format!("SELECT email, region FROM serving.mart_x WHERE '{id}' != ''")
        },
        "x": "region", "y": "email"
    });
    let mut def = json!({
        "title": format!("Tile {id}"), "mart": "mart_x", "kind": kind,
        "dimension": "region", "measures": ["email"]
    });
    if let Some(map_id) = map {
        spec["map"] = json!(map_id);
        def["map"] = json!(map_id);
    }
    json!({
        "id": id, "board": board, "created_by": "ui", "created_at": "2026-01-01 00:00:00",
        "spec_json": json!({ "spec": spec, "def": def, "hasYear": false }).to_string(),
    })
}

fn board_row(id: &str, folder: &str, owner: &str) -> Value {
    json!({
        "id": id, "name": format!("Board {id}"), "description": "", "created_by": "t",
        "layout_json": "", "filters_json": "", "public_token": "", "embed_enabled": "0",
        "folder_id": folder, "created_at": "2026-10-10 00:00:00", "owner_id": owner,
        "tabs_json": "", "embed_revoked_before": "0", "embed_revoked_jti_json": "[]",
        "embed_origins_json": "[]"
    })
}

fn folder_row(id: &str, owner: &str) -> Value {
    json!({
        "id": id, "name": format!("Folder {id}"), "parent_id": "", "created_by": "t",
        "owner_id": owner, "updated_at": "2026-10-10 00:00:00",
    })
}

/// The ids of an `AND <column> IN (...)` or `NOT IN (...)` list fragment, if
/// the statement has one. A sub-select (`IN (SELECT ...)`, the chart clause's
/// owner and trash checks) is not a list and is skipped.
fn id_list(sql: &str, marker: &str) -> Option<Vec<String>> {
    let mut from = 0;
    while let Some(at) = sql[from..].find(marker) {
        let rest = &sql[from + at + marker.len()..];
        from += at + marker.len();
        if rest.starts_with("SELECT") {
            continue;
        }
        let inner = &rest[..rest.find(')')?];
        return Some(
            inner
                .split(',')
                .map(|s| s.trim().trim_matches('\'').to_owned())
                .filter(|s| !s.is_empty())
                .collect(),
        );
    }
    None
}

/// Whether a row passes the statement's viewer fragments: the personal-space
/// clause and the permission list.
fn passes(sql: &str, column: &str, id: &str, owner: &str) -> bool {
    if !owner.is_empty() && sql.contains("(owner_id = '' OR owner_id = ") {
        let mine = format!("owner_id = '{owner}')");
        if !sql.contains(&mine) {
            return false;
        }
    }
    if sql.contains(" AND 1 = 0") {
        return false;
    }
    if let Some(hidden) = id_list(sql, &format!("AND {column} NOT IN (")) {
        return !hidden.iter().any(|h| h == id);
    }
    if let Some(shown) = id_list(sql, &format!("AND {column} IN (")) {
        return shown.iter().any(|s| s == id);
    }
    true
}

impl FakeCh {
    fn json_rows(data: &[Value]) -> ResponseTemplate {
        let n = data.len();
        ResponseTemplate::new(200).set_body_json(json!({ "meta": [], "data": data, "rows": n }))
    }

    fn charts(&self) -> Vec<(String, String, String, Option<String>)> {
        let mut charts: Vec<(String, String, String, Option<String>)> = [
            ("c_one", "b_one", "bar"),
            ("c_two", "b_two", "bar"),
            ("c_text", "b_one", "text"),
            ("c_locked", "b_locked", "bar"),
            ("c_mine", "b_mine", "bar"),
        ]
        .into_iter()
        .map(|(id, board, kind)| (id.to_owned(), board.to_owned(), kind.to_owned(), None))
        .collect();
        for (id, board, map) in self.map_charts.lock().unwrap().iter() {
            charts.push((
                id.clone(),
                board.clone(),
                "geomap".to_owned(),
                Some(map.clone()),
            ));
        }
        charts
    }
}

impl Respond for FakeCh {
    fn respond(&self, request: &MockRequest) -> ResponseTemplate {
        let sql = String::from_utf8_lossy(&request.body).into_owned();
        self.log.lock().unwrap().push(sql.clone());
        if sql.contains("FROM console.bi_folder") {
            let all = [("f_open", ""), ("f_locked", ""), ("p_owner", OWNER)];
            let rows: Vec<Value> = all
                .iter()
                .filter(|(id, owner)| passes(&sql, "id", id, owner))
                .map(|(id, owner)| folder_row(id, owner))
                .collect();
            return Self::json_rows(&rows);
        }
        let boards = [
            ("b_one", "f_open", ""),
            ("b_two", "", ""),
            ("b_locked", "f_locked", ""),
            ("b_mine", "p_owner", OWNER),
        ];
        let charts = self.charts();
        let trashed = self.trashed.lock().unwrap().clone();
        // `store::chart_row_state` and `store::board_row_state`: one row by
        // id, read as the server (no viewer fragment).
        if let Some(id) = sql
            .split("SELECT is_deleted, board FROM console.bi_chart FINAL WHERE id = '")
            .nth(1)
            .and_then(|rest| rest.split('\'').next())
        {
            let rows: Vec<Value> = charts
                .iter()
                .filter(|(chart, _, _, _)| chart.as_str() == id)
                .map(|(chart, board, _, _)| {
                    let gone = trashed.iter().any(|t| t == chart);
                    json!({ "is_deleted": if gone { 2 } else { 0 }, "board": board })
                })
                .collect();
            return Self::json_rows(&rows);
        }
        if let Some(id) = sql
            .split("SELECT is_deleted, owner_id FROM console.bi_board FINAL WHERE id = '")
            .nth(1)
            .and_then(|rest| rest.split('\'').next())
        {
            let rows: Vec<Value> = boards
                .iter()
                .filter(|(board, _, _)| *board == id)
                .map(|(_, _, owner)| json!({ "is_deleted": 0, "owner_id": owner }))
                .collect();
            return Self::json_rows(&rows);
        }
        if sql.contains("FROM console.bi_chart") {
            let rows: Vec<Value> = charts
                .iter()
                .filter(|(id, _, _, _)| !trashed.iter().any(|t| t == id))
                .filter(|(_, board, _, _)| {
                    let owner = boards
                        .iter()
                        .find(|b| b.0 == board.as_str())
                        .map_or("", |b| b.2);
                    // The chart clause hides another owner's board by a
                    // sub-select, and the permission list by board id.
                    let private = !owner.is_empty()
                        && sql.contains("owner_id != ''")
                        && !sql.contains(&format!("owner_id != '{owner}'"));
                    !private && passes(&sql, "board", board, "")
                })
                .map(|(id, board, kind, map)| chart_row(id, board, kind, map.as_deref()))
                .collect();
            return Self::json_rows(&rows);
        }
        if sql.contains("FROM console.bi_board") {
            let wanted =
                |id: &str| sql.contains(&format!("id='{id}'")) || !sql.contains("AND id='");
            let rows: Vec<Value> = boards
                .iter()
                .filter(|(id, _, owner)| wanted(id) && passes(&sql, "id", id, owner))
                .map(|(id, folder, owner)| board_row(id, folder, owner))
                .collect();
            return Self::json_rows(&rows);
        }
        if sql.contains("system.columns") {
            let rows: Vec<Value> = ["email", "region"]
                .iter()
                .map(|n| json!({"table": "mart_x", "name": n, "type": "String"}))
                .collect();
            return Self::json_rows(&rows);
        }
        if sql.contains("FROM serving.mart_x") || sql.contains("replaceRegexpOne") {
            return Self::json_rows(&[json!({"email": "a@x", "region": "north"})]);
        }
        ResponseTemplate::new(200).set_body_string("Ok.\n")
    }
}

struct Fixture {
    app: TestApp,
    fake: FakeCh,
    /// The writer's and the reader's user ids, for a render token.
    writer_id: Uuid,
    reader_id: Uuid,
    /// `dashboard:read` and `dashboard:write`; owns the personal space.
    owner: String,
    /// `dashboard:read` and `dashboard:write`.
    writer: String,
    /// `dashboard:read` only, role `Report Reader`.
    reader: String,
    _ch: MockServer,
}

async fn person(pool: &PgPool, id: Uuid, role: &str, permissions: &str) -> Uuid {
    let role_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO role (id, name, permissions, description) VALUES ($1, $2, $3, 'test')",
    )
    .bind(role_id)
    .bind(role)
    .bind(permissions)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO app_user (id, name, username, email, status) VALUES ($1, $2, $3, $4, 'active')",
    )
    .bind(id)
    .bind(format!("{role} person"))
    .bind(format!("u-{id}"))
    .bind(format!("u-{id}@test.invalid"))
    .execute(pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO app_user_role (user_id, role_id) VALUES ($1, $2)")
        .bind(id)
        .bind(role_id)
        .execute(pool)
        .await
        .unwrap();
    common::in_the_catalog_tenant(pool, id).await;
    id
}

async fn fixture() -> Fixture {
    fixture_with(&[]).await
}

async fn fixture_with(extra: &[(&str, &str)]) -> Fixture {
    let fake = FakeCh {
        log: Arc::new(Mutex::new(Vec::new())),
        trashed: Arc::new(Mutex::new(Vec::new())),
        map_charts: Arc::new(Mutex::new(Vec::new())),
    };
    let ch = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(fake.clone())
        .mount(&ch)
        .await;
    let mut env: HashMap<String, String> = HashMap::from([
        ("CH_URL".to_owned(), ch.uri()),
        (
            "CATALOG_TENANT_ID".to_owned(),
            common::CATALOG_TENANT.to_owned(),
        ),
    ]);
    for (key, value) in extra {
        env.insert((*key).to_owned(), (*value).to_owned());
    }
    let app = spin_up_with_env(&env).await;
    let rw = "dashboard:read,dashboard:write";
    let owner = person(&app.pool, OWNER.parse().unwrap(), "Report Owner", rw).await;
    let writer = person(&app.pool, Uuid::new_v4(), "Report Writer", rw).await;
    let reader = person(&app.pool, Uuid::new_v4(), "Report Reader", "dashboard:read").await;
    // `f_locked` is "only the people listed", and nobody is listed.
    sqlx::query(
        "INSERT INTO dashboard_restriction (item_kind, item_id) VALUES ('folder', 'f_locked')",
    )
    .execute(&app.pool)
    .await
    .unwrap();
    Fixture {
        writer_id: writer,
        reader_id: reader,
        owner: session_cookie_for_user(&app.pool, owner).await,
        writer: session_cookie_for_user(&app.pool, writer).await,
        reader: session_cookie_for_user(&app.pool, reader).await,
        app,
        fake,
        _ch: ch,
    }
}

async fn call(
    app: &TestApp,
    method: &str,
    uri: &str,
    cookie: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut req = Request::builder()
        .method(method)
        .uri(uri)
        .header("cookie", cookie);
    if body.is_some() {
        req = req.header("content-type", "application/json");
    }
    let resp = app
        .router
        .clone()
        .oneshot(
            req.body(body.map_or_else(Body::empty, |b| Body::from(b.to_string())))
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

async fn create(f: &Fixture, cookie: &str, body: Value) -> (StatusCode, Value) {
    call(&f.app, "POST", "/api/dashboard/reports", cookie, Some(body)).await
}

fn chart(id: &str) -> Value {
    json!({ "type": "chart", "chart": id })
}

/// `BI-18D-AC1`: the blocks come back in the order they were saved, each chart
/// block names the dashboard of its chart (filled by the server, not taken from
/// the request), and both charts are read live.
#[tokio::test]
async fn a_report_saves_and_reloads_with_its_blocks_in_order_and_its_charts_live() {
    let f = fixture().await;
    let blocks = json!([
        { "type": "heading", "text": "Visitors this quarter", "level": 1 },
        { "type": "paragraph", "text": "Visits rose in the north." },
        { "type": "chart", "chart": "c_one", "board": "b_somebody_else" },
        { "type": "paragraph", "text": "The south held steady." },
        { "type": "list", "items": ["North up", "South flat"] },
        { "type": "link", "text": "Method", "url": "https://example.com/method" },
        chart("c_two"),
    ]);
    let (status, made) = create(
        &f,
        &f.writer,
        json!({ "title": "  Q3 visitors ", "folderId": "f_open", "blocks": blocks }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{made}");
    assert_eq!(made["report"]["title"], "Q3 visitors");
    let id = made["report"]["id"].as_str().unwrap().to_owned();

    let (status, read) = call(
        &f.app,
        "GET",
        &format!("/api/dashboard/reports/{id}"),
        &f.reader,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{read}");
    let kinds: Vec<&str> = read["report"]["blocks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["type"].as_str().unwrap())
        .collect();
    assert_eq!(
        kinds,
        [
            "heading",
            "paragraph",
            "chart",
            "paragraph",
            "list",
            "link",
            "chart"
        ]
    );
    assert_eq!(read["report"]["blocks"][2]["board"], "b_one");
    assert_eq!(read["report"]["blocks"][6]["board"], "b_two");
    assert_eq!(read["report"]["folderId"], "f_open");
    for chart in ["c_one", "c_two"] {
        let entry = &read["charts"][chart];
        assert_eq!(entry["spec"]["id"], chart, "{read}");
        assert!(entry["cell"].is_object(), "{entry}");
        assert!(entry.get("unavailable").is_none(), "{entry}");
    }
    assert_eq!(read["charts"]["c_one"]["boardName"], "Board b_one");
    // The reader holds `dashboard:read` only: the page offers no editing.
    assert_eq!(read["access"]["level"], "view");

    let (status, list) = call(&f.app, "GET", "/api/dashboard/reports", &f.reader, None).await;
    assert_eq!(status, StatusCode::OK);
    let row = &list["reports"][0];
    assert_eq!(row["id"], id.as_str());
    assert_eq!(row["blockCount"], 7);
    assert_eq!(row["chartCount"], 2);
}

/// `BI-18D-AC4` (decision R4): the 31st chart is refused with the limit named.
#[tokio::test]
async fn a_thirty_first_chart_is_refused_with_a_sentence_that_names_the_limit() {
    let f = fixture().await;
    let blocks: Vec<Value> = (0..31).map(|_| chart("c_one")).collect();
    let (status, body) = create(
        &f,
        &f.writer,
        json!({ "title": "Too many", "blocks": blocks }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "A report holds at most 30 charts.");

    // Thirty is fine, and so is a change that keeps it at thirty.
    let blocks: Vec<Value> = (0..30).map(|_| chart("c_one")).collect();
    let (status, made) = create(
        &f,
        &f.writer,
        json!({ "title": "Thirty", "blocks": blocks }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{made}");
    let id = made["report"]["id"].as_str().unwrap();
    let blocks: Vec<Value> = (0..31).map(|_| chart("c_two")).collect();
    let (status, body) = call(
        &f.app,
        "PUT",
        &format!("/api/dashboard/reports/{id}"),
        &f.writer,
        Some(json!({ "blocks": blocks })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "A report holds at most 30 charts.");
}

/// `BI-18D-AC3` (decision R2): a report in a folder the person cannot read is
/// the one not-found sentence, whether the folder is restricted or somebody
/// else's personal space, and it is not in their list.
#[tokio::test]
async fn a_report_in_a_folder_the_person_cannot_read_is_not_found() {
    let f = fixture().await;
    let (status, mine) = create(
        &f,
        &f.owner,
        json!({ "title": "Draft", "folderId": "p_owner" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{mine}");
    let mine = mine["report"]["id"].as_str().unwrap().to_owned();
    // The owner sees their own draft.
    let (status, _) = call(
        &f.app,
        "GET",
        &format!("/api/dashboard/reports/{mine}"),
        &f.owner,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    // Nobody else does: not by id, not in the list, not to change or delete it.
    for (method, body) in [
        ("GET", None),
        ("PUT", Some(json!({ "title": "Mine now" }))),
        ("DELETE", None),
    ] {
        let (status, said) = call(
            &f.app,
            method,
            &format!("/api/dashboard/reports/{mine}"),
            &f.writer,
            body,
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{method}: {said}");
        assert_eq!(said["error"], "report not found.", "{method}");
    }
    let (_, list) = call(&f.app, "GET", "/api/dashboard/reports", &f.writer, None).await;
    assert!(list["reports"].as_array().unwrap().is_empty(), "{list}");

    // A restricted folder nobody is listed on cannot be filed into either.
    let (status, said) = create(
        &f,
        &f.writer,
        json!({ "title": "Locked", "folderId": "f_locked" }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{said}");
    // An unknown id and a malformed id read the same.
    for id in [Uuid::new_v4().to_string(), "not-a-uuid".to_owned()] {
        let (status, said) = call(
            &f.app,
            "GET",
            &format!("/api/dashboard/reports/{id}"),
            &f.reader,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(said["error"], "report not found.");
    }
}

/// `BI-18D-AC2` (decision R1): each chart block is read as the reader, with the
/// reader's masks and row filters, exactly as on its dashboard; another person
/// whose role no policy names reads it unmasked.
#[tokio::test]
async fn a_chart_block_is_read_with_the_readers_mask_and_row_filter() {
    let f = fixture().await;
    governance::create_policy(
        &f.app.pool,
        &CreatePolicyInput {
            name: "report-mask".to_owned(),
            kind: "Row filter".to_owned(),
            subjects: "Report Reader".to_owned(),
            resources: "serving.mart_x".to_owned(),
            effect: "Permit with obligation".to_owned(),
            conditions: Some(
                r#"{"roles":["Report Reader"],"table":"serving.mart_x","mask":["email"],"rowFilter":"region = 'south'"}"#
                    .to_owned(),
            ),
            activate: true,
            owner: None,
        },
    )
    .await
    .unwrap();
    let (_, made) = create(
        &f,
        &f.writer,
        json!({ "title": "Masked", "blocks": [chart("c_one")] }),
    )
    .await;
    let id = made["report"]["id"].as_str().unwrap().to_owned();

    let tile_statements = |from: usize| -> Vec<String> {
        f.fake.log.lock().unwrap()[from..]
            .iter()
            .filter(|s| {
                s.contains("serving.mart_x") && s.contains("'c_one'") && !s.contains("system.")
            })
            .cloned()
            .collect()
    };
    let before = f.fake.log.lock().unwrap().len();
    let (status, read) = call(
        &f.app,
        "GET",
        &format!("/api/dashboard/reports/{id}"),
        &f.reader,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{read}");
    let sent = tile_statements(before);
    assert!(!sent.is_empty(), "the chart was read");
    assert!(
        sent.iter()
            .all(|s| s.contains("replaceRegexpOne(toString(`email`)")),
        "the reader's mask: {sent:?}"
    );
    assert!(
        sent.iter().all(|s| s.contains("region = 'south'")),
        "the reader's row filter: {sent:?}"
    );

    let before = f.fake.log.lock().unwrap().len();
    let (status, _) = call(
        &f.app,
        "GET",
        &format!("/api/dashboard/reports/{id}"),
        &f.writer,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let sent = tile_statements(before);
    assert!(!sent.is_empty(), "the chart was read for the writer too");
    assert!(
        sent.iter()
            .all(|s| !s.contains("replaceRegexpOne") && !s.contains("region = 'south'")),
        "nobody else's policy: {sent:?}"
    );
}

/// A chart the reader may not see is the fixed sentence, with no title and no
/// data; the rest of the report still reads.
#[tokio::test]
async fn a_chart_the_reader_cannot_see_is_not_available_and_shows_nothing_of_itself() {
    let f = fixture().await;
    // The owner places a chart of their personal board next to a public one.
    let (status, made) = create(
        &f,
        &f.owner,
        json!({ "title": "Mixed", "blocks": [chart("c_mine"), chart("c_two")] }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{made}");
    let id = made["report"]["id"].as_str().unwrap().to_owned();
    let (status, read) = call(
        &f.app,
        "GET",
        &format!("/api/dashboard/reports/{id}"),
        &f.reader,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{read}");
    assert_eq!(
        read["charts"]["c_mine"],
        json!({ "unavailable": "This chart is not available." })
    );
    assert!(!read.to_string().contains("Tile c_mine"), "{read}");
    assert_eq!(read["charts"]["c_two"]["spec"]["id"], "c_two");
}

/// Saving checks every chart block for the writer: a chart they cannot see and
/// a tile that is not a chart are refused, each with its own sentence; the
/// sentence names the block (PR #528 review SHOULD-FIX 1).
#[tokio::test]
async fn saving_refuses_a_chart_the_writer_cannot_see_and_a_tile_that_is_not_a_chart() {
    let f = fixture().await;
    for (block, sentence) in [
        (
            chart("c_mine"),
            "The chart in block 1 of this report is not one you can see.",
        ),
        (
            chart("c_unknown"),
            "The chart in block 1 of this report is not one you can see.",
        ),
        (
            chart("c_text"),
            "A text, section, image or web tile is not a chart; only charts can be placed in a report.",
        ),
    ] {
        let (status, said) =
            create(&f, &f.writer, json!({ "title": "X", "blocks": [block] })).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{said}");
        assert_eq!(said["error"], sentence);
    }
}

/// PR #528 review SHOULD-FIX 1: the editor sends every block back on each
/// save, and a chart block already on the report is kept as stored, so a
/// report whose chart was moved to the trash still takes a title fix (it used
/// to be refused for every change). A deleted chart that is new in a save gets
/// its own sentence, naming its block.
#[tokio::test]
async fn a_report_whose_chart_was_deleted_still_saves_a_title_change() {
    let f = fixture().await;
    let blocks = json!([
        chart("c_one"),
        { "type": "paragraph", "text": "Visits rose." },
        chart("c_two"),
    ]);
    let (status, made) = create(
        &f,
        &f.writer,
        json!({ "title": "Q3", "folderId": "f_open", "blocks": blocks }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{made}");
    let id = made["report"]["id"].as_str().unwrap().to_owned();
    f.fake.trashed.lock().unwrap().push("c_one".to_owned());

    let (status, read) = call(
        &f.app,
        "GET",
        &format!("/api/dashboard/reports/{id}"),
        &f.writer,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{read}");
    assert_eq!(
        read["charts"]["c_one"],
        json!({ "unavailable": "This chart is not available." })
    );
    let (status, saved) = call(
        &f.app,
        "PUT",
        &format!("/api/dashboard/reports/{id}"),
        &f.writer,
        Some(json!({ "title": "Q3 visitors", "blocks": read["report"]["blocks"] })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    assert_eq!(saved["report"]["title"], "Q3 visitors");
    assert_eq!(
        saved["report"]["blocks"][0],
        json!({ "type": "chart", "chart": "c_one", "board": "b_one" }),
        "the kept block keeps the dashboard stored with it"
    );

    // Placing the deleted chart in another report is refused, and says why.
    let (status, other) = create(
        &f,
        &f.writer,
        json!({ "title": "Other", "folderId": "f_open", "blocks": [chart("c_two")] }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{other}");
    let other = other["report"]["id"].as_str().unwrap().to_owned();
    let (status, said) = call(
        &f.app,
        "PUT",
        &format!("/api/dashboard/reports/{other}"),
        &f.writer,
        Some(json!({ "blocks": [chart("c_two"), chart("c_one")] })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{said}");
    assert_eq!(
        said["error"],
        "The chart in block 2 of this report has been deleted. Remove that block and save again."
    );
}

/// PR #528 review SHOULD-FIX 1, the second case: an editor who cannot see a
/// chart another editor placed can still change the report, because that block
/// is kept as stored; a chart they cannot see is refused only when their own
/// save adds it, and a deleted chart on a dashboard they cannot see reads the
/// same as a hidden one (an id guess learns nothing).
#[tokio::test]
async fn an_editor_keeps_a_chart_they_cannot_see_and_may_not_add_one() {
    let f = fixture().await;
    // The owner places a chart of their personal dashboard in a shared folder.
    let (status, made) = create(
        &f,
        &f.owner,
        json!({ "title": "Mixed", "folderId": "f_open", "blocks": [chart("c_mine"), chart("c_two")] }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{made}");
    let id = made["report"]["id"].as_str().unwrap().to_owned();
    let (_, read) = call(
        &f.app,
        "GET",
        &format!("/api/dashboard/reports/{id}"),
        &f.writer,
        None,
    )
    .await;
    let mut blocks = read["report"]["blocks"].as_array().unwrap().clone();
    blocks.push(json!({ "type": "paragraph", "text": "Added by the writer." }));
    let (status, saved) = call(
        &f.app,
        "PUT",
        &format!("/api/dashboard/reports/{id}"),
        &f.writer,
        Some(json!({ "title": "Mixed, edited", "blocks": blocks })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    assert_eq!(saved["report"]["blocks"][0]["chart"], "c_mine");
    assert_eq!(saved["report"]["blocks"][0]["board"], "b_mine");
    assert_eq!(saved["report"]["blocks"].as_array().unwrap().len(), 3);

    // A chart the writer cannot see, new in their save: refused, live or deleted.
    for trashed in [false, true] {
        if trashed {
            f.fake.trashed.lock().unwrap().push("c_locked".to_owned());
        }
        let mut more = blocks.clone();
        more.push(chart("c_locked"));
        let (status, said) = call(
            &f.app,
            "PUT",
            &format!("/api/dashboard/reports/{id}"),
            &f.writer,
            Some(json!({ "blocks": more })),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "trashed {trashed}: {said}");
        assert_eq!(
            said["error"], "The chart in block 4 of this report is not one you can see.",
            "trashed {trashed}"
        );
    }
}

/// Text is stored and answered exactly as typed: markup in a paragraph is a
/// string in JSON, never interpreted, and an unknown block kind (`html`) is
/// refused. The console renders it as text (`report-blocks.test.tsx`).
#[tokio::test]
async fn markup_in_a_paragraph_comes_back_as_the_same_plain_string() {
    let f = fixture().await;
    let text = "<script>alert(1)</script> & <b>bold</b>";
    let (status, made) = create(
        &f,
        &f.writer,
        json!({ "title": "<i>T</i>", "blocks": [{ "type": "paragraph", "text": text }] }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{made}");
    let id = made["report"]["id"].as_str().unwrap().to_owned();
    let (_, read) = call(
        &f.app,
        "GET",
        &format!("/api/dashboard/reports/{id}"),
        &f.reader,
        None,
    )
    .await;
    assert_eq!(read["report"]["blocks"][0]["text"], text);
    assert_eq!(read["report"]["title"], "<i>T</i>");
    let (status, said) = create(
        &f,
        &f.writer,
        json!({ "title": "X", "blocks": [{ "type": "html", "html": text }] }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        said["error"],
        "blocks must be a list of heading, paragraph, list, link and chart blocks."
    );
}

/// A deleted report is in the trash for its folder's readers, comes back with
/// a restore, and a folder that still holds a report cannot be deleted.
#[tokio::test]
async fn a_deleted_report_goes_to_the_trash_and_comes_back_and_keeps_its_folder() {
    let f = fixture().await;
    let (_, made) = create(
        &f,
        &f.writer,
        json!({ "title": "Old", "folderId": "f_open" }),
    )
    .await;
    let id = made["report"]["id"].as_str().unwrap().to_owned();

    let (status, said) = call(
        &f.app,
        "DELETE",
        "/api/dashboard/folders?id=f_open",
        &f.writer,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{said}");
    assert!(
        said["error"].as_str().unwrap().contains("1 report(s)"),
        "{said}"
    );

    let (status, _) = call(
        &f.app,
        "DELETE",
        &format!("/api/dashboard/reports/{id}"),
        &f.writer,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = call(
        &f.app,
        "GET",
        &format!("/api/dashboard/reports/{id}"),
        &f.writer,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    // Still counted: it comes back into this folder.
    let (status, _) = call(
        &f.app,
        "DELETE",
        "/api/dashboard/folders?id=f_open",
        &f.writer,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);

    let (status, trash) = call(&f.app, "GET", "/api/dashboard/trash", &f.reader, None).await;
    assert_eq!(status, StatusCode::OK, "{trash}");
    let item = trash["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["kind"] == "report")
        .cloned()
        .expect("the report is in the trash");
    assert_eq!(item["id"], id.as_str());
    assert_eq!(item["name"], "Old");
    assert_eq!(item["deletedBy"], "Report Writer person");

    // `dashboard:read` alone cannot restore; the writer can.
    let restore = json!({ "kind": "report", "id": id });
    let (status, _) = call(
        &f.app,
        "POST",
        "/api/dashboard/trash/restore",
        &f.reader,
        Some(restore.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, said) = call(
        &f.app,
        "POST",
        "/api/dashboard/trash/restore",
        &f.writer,
        Some(restore.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{said}");
    assert_eq!(said["restored"]["kind"], "report");
    let (status, read) = call(
        &f.app,
        "GET",
        &format!("/api/dashboard/reports/{id}"),
        &f.writer,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(read["report"]["folderId"], "f_open");
    let (status, _) = call(
        &f.app,
        "POST",
        "/api/dashboard/trash/restore",
        &f.writer,
        Some(restore),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "already restored");
}

/// `BI-18D-AC8`: a person with `dashboard:read` only can read reports and is
/// refused every write with 403, whatever the body says.
#[tokio::test]
async fn a_reader_is_refused_every_report_write() {
    let f = fixture().await;
    let (_, made) = create(&f, &f.writer, json!({ "title": "Theirs" })).await;
    let id = made["report"]["id"].as_str().unwrap().to_owned();
    let uri = format!("/api/dashboard/reports/{id}");
    for (method, uri, body) in [
        (
            "POST",
            "/api/dashboard/reports",
            Some(json!({ "title": "Mine" })),
        ),
        ("PUT", uri.as_str(), Some(json!({ "title": "Mine" }))),
        ("DELETE", uri.as_str(), None),
    ] {
        let (status, _) = call(&f.app, method, uri, &f.reader, body).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{method} {uri}");
    }
    let (status, read) = call(&f.app, "GET", &uri, &f.reader, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(read["report"]["title"], "Theirs");
}

// ── T2: the print path ──────────────────────────────────────────────────

const RENDER_SECRET: &str = "test-render-secret-not-real";

fn report_token(user: Uuid, report: &str) -> String {
    let iat = (unix_now() - 5.0).floor();
    sign_render(
        &RenderClaims {
            aud: "render".to_owned(),
            board: String::new(),
            chart: None,
            report: Some(report.to_owned()),
            principal: user.to_string(),
            filters: None,
            params: None,
            grain: None,
            paper: "a4".to_owned(),
            orientation: "portrait".to_owned(),
            exp: iat + 120.0,
            iat,
            jti: Uuid::new_v4().to_string(),
        },
        RENDER_SECRET,
    )
}

async fn render_data(f: &Fixture, token: &str) -> (StatusCode, Value) {
    call(
        &f.app,
        "POST",
        "/api/render/data",
        "",
        Some(json!({ "token": token })),
    )
    .await
}

/// `BI-18D-AC5` (decision R3): the print view's data is the report with its
/// blocks in order and every chart read as the person the token names, and
/// the first page's facts.
#[tokio::test]
async fn a_report_token_reads_the_report_in_order_as_the_person_it_names() {
    let f = fixture_with(&[("RENDER_TOKEN_SECRET", RENDER_SECRET)]).await;
    let (_, made) = create(
        &f,
        &f.writer,
        json!({ "title": "Printed", "blocks": [
            { "type": "heading", "text": "One", "level": 1 },
            chart("c_two"),
            { "type": "paragraph", "text": "Two" },
            chart("c_one"),
        ] }),
    )
    .await;
    let id = made["report"]["id"].as_str().unwrap().to_owned();
    let (status, body) = render_data(&f, &report_token(f.reader_id, &id)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let kinds: Vec<&str> = body["report"]["blocks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["type"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["heading", "chart", "paragraph", "chart"]);
    assert_eq!(body["charts"]["c_two"]["spec"]["id"], "c_two");
    assert_eq!(body["charts"]["c_one"]["spec"]["id"], "c_one");
    assert_eq!(body["render"]["requestedBy"], "Report Reader person");
    assert_eq!(body["render"]["report"], id.as_str());
    // Nothing of a dashboard: the token names no board.
    assert!(
        body.get("layout").is_none() && body.get("results").is_none(),
        "{body}"
    );
}

/// A report token for a report the person cannot read, or an unknown one, is
/// the one sentence every refused token gets.
#[tokio::test]
async fn a_report_token_for_a_report_the_person_cannot_read_is_refused() {
    let f = fixture_with(&[("RENDER_TOKEN_SECRET", RENDER_SECRET)]).await;
    let (_, mine) = create(
        &f,
        &f.owner,
        json!({ "title": "Draft", "folderId": "p_owner" }),
    )
    .await;
    let mine = mine["report"]["id"].as_str().unwrap().to_owned();
    for report in [mine, Uuid::new_v4().to_string(), "nope".to_owned()] {
        let (status, body) = render_data(&f, &report_token(f.writer_id, &report)).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{report}: {body}");
        assert_eq!(body["error"], "render token is invalid or expired");
    }
}

/// `GET /api/dashboard/reports/{id}/pdf`: the renderer opens the console's
/// report print view with a token for this report and this person, the file
/// comes back named after the report, and the download is audited.
#[tokio::test]
async fn the_report_pdf_opens_the_report_print_view_and_is_audited() {
    let gotenberg = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/forms/chromium/convert/url"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"%PDF-1.7 fake".to_vec()))
        .mount(&gotenberg)
        .await;
    let uri = gotenberg.uri();
    let f = fixture_with(&[
        ("RENDER_TOKEN_SECRET", RENDER_SECRET),
        ("GOTENBERG_URL", uri.as_str()),
        ("CONSOLE_INTERNAL_URL", "http://console.test:3000"),
    ])
    .await;
    let (_, made) = create(
        &f,
        &f.writer,
        json!({ "title": "Q3 / visitors", "blocks": [chart("c_one")] }),
    )
    .await;
    let id = made["report"]["id"].as_str().unwrap().to_owned();

    let resp = f
        .app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/api/dashboard/reports/{id}/pdf?paper=letter&orientation=landscape"
                ))
                .header("cookie", &f.reader)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(resp.headers()["content-type"], "application/pdf");
    let disposition = resp.headers()["content-disposition"]
        .to_str()
        .unwrap()
        .to_owned();
    assert!(
        disposition.starts_with("attachment; filename=\"Q3-visitors-"),
        "{disposition}"
    );
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    assert_eq!(&bytes[..], b"%PDF-1.7 fake");

    let sent = gotenberg.received_requests().await.unwrap();
    assert_eq!(sent.len(), 1);
    let form = String::from_utf8_lossy(&sent[0].body).into_owned();
    let marker = "http://console.test:3000/render/report/";
    let at = form.find(marker).expect("the report print view is opened");
    let token: String = form[at + marker.len()..]
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        .collect();
    let claims = lakehouse_embed::verify_render(&token, RENDER_SECRET, unix_now()).unwrap();
    assert_eq!(claims.report.as_deref(), Some(id.as_str()));
    assert_eq!(claims.board, "");
    assert_eq!(claims.principal, f.reader_id.to_string());
    assert_eq!(claims.paper, "letter");

    let (action, args): (String, Value) = sqlx::query_as(
        "SELECT action, args FROM audit_event WHERE action = 'dashboard.report_pdf'",
    )
    .fetch_one(&f.app.pool)
    .await
    .unwrap();
    assert_eq!(action, "dashboard.report_pdf");
    assert_eq!(args["report"], id.as_str());

    // A report the person cannot read is the report's 404, and nothing is printed.
    let (_, mine) = create(
        &f,
        &f.owner,
        json!({ "title": "Draft", "folderId": "p_owner" }),
    )
    .await;
    let mine = mine["report"]["id"].as_str().unwrap().to_owned();
    let (status, said) = call(
        &f.app,
        "GET",
        &format!("/api/dashboard/reports/{mine}/pdf"),
        &f.reader,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(said["error"], "report not found.");
    assert_eq!(gotenberg.received_requests().await.unwrap().len(), 1);
}

const TEST_GEOJSON: &str = r#"{"type":"FeatureCollection","features":[{"type":"Feature","properties":{"NAME":"North"},"geometry":{"type":"Polygon","coordinates":[[[0,0],[1,0],[1,1],[0,1],[0,0]]]}}]}"#;

async fn upload_test_map(app: &TestApp, cookie: &str, name: &str) -> String {
    let mut body = Vec::new();
    for (field, value) in [("name", name), ("nameProperty", "NAME")] {
        body.extend_from_slice(
            format!(
                "--BOUNDARY\r\nContent-Disposition: form-data; name=\"{field}\"\r\n\r\n{value}\r\n"
            )
            .as_bytes(),
        );
    }
    body.extend_from_slice(
        b"--BOUNDARY\r\nContent-Disposition: form-data; name=\"file\"; filename=\"map.geojson\"\r\nContent-Type: application/geo+json\r\n\r\n",
    );
    body.extend_from_slice(TEST_GEOJSON.as_bytes());
    body.extend_from_slice(b"\r\n--BOUNDARY--\r\n");

    let resp = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/maps")
                .header("cookie", cookie)
                .header("content-type", "multipart/form-data; boundary=BOUNDARY")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let json: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    assert_eq!(status, StatusCode::CREATED, "{json}");
    json["id"].as_str().unwrap().to_owned()
}

/// `PR #528 review BLOCKER`: a report render token has `board: ""` and
/// opens uploaded maps through `POST /api/render/maps/{id}/geojson`, strictly
/// scoped to maps drawn by visible charts in that report.
#[tokio::test]
async fn a_report_token_opens_only_the_maps_used_by_the_reports_visible_charts() {
    let f = fixture_with(&[("RENDER_TOKEN_SECRET", RENDER_SECRET)]).await;
    let admin_id = common::create_principal_with_permissions(&f.app.pool, "settings:write").await;
    let admin_cookie = session_cookie_for_user(&f.app.pool, admin_id).await;
    let m_used = upload_test_map(&f.app, &admin_cookie, "Report Map").await;
    let m_sibling = upload_test_map(&f.app, &admin_cookie, "Sibling Map").await;
    let m_hidden = upload_test_map(&f.app, &admin_cookie, "Private Map").await;

    f.fake.map_charts.lock().unwrap().extend([
        ("c_map".to_owned(), "b_one".to_owned(), m_used.clone()),
        (
            "c_sibling_map".to_owned(),
            "b_one".to_owned(),
            m_sibling.clone(),
        ),
        (
            "c_mine_map".to_owned(),
            "b_mine".to_owned(),
            m_hidden.clone(),
        ),
    ]);

    // Owner places both a visible chart (`c_map`) and their personal-board
    // chart (`c_mine_map`) in a shared report in `f_open`. `c_sibling_map` is
    // on the same board `b_one` as `c_map`, but is NOT in the report.
    let (status, made) = create(
        &f,
        &f.owner,
        json!({
            "title": "Regional summary",
            "folderId": "f_open",
            "blocks": [chart("c_map"), chart("c_mine_map")],
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{made}");
    let report_id = made["report"]["id"].as_str().unwrap().to_owned();
    let reader_tok = report_token(f.reader_id, &report_id);

    // Allowed: map used by `c_map` in the report.
    let (status, body) = call(
        &f.app,
        "POST",
        &format!("/api/render/maps/{m_used}/geojson"),
        "",
        Some(json!({ "token": reader_tok })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["type"], "FeatureCollection");

    // Refused (404): map on the same board not in the report, map of a hidden
    // chart in the report, and an unknown map id.
    for refused_map in [m_sibling.as_str(), m_hidden.as_str(), "m_000000000000"] {
        let (status, body) = call(
            &f.app,
            "POST",
            &format!("/api/render/maps/{refused_map}/geojson"),
            "",
            Some(json!({ "token": reader_tok })),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{refused_map}: {body}");
    }

    // Fail-closed: an unreadable report's token is refused with 401.
    let (status, private_report) = create(
        &f,
        &f.owner,
        json!({ "title": "Private", "folderId": "p_owner", "blocks": [chart("c_map")] }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{private_report}");
    let private_id = private_report["report"]["id"].as_str().unwrap().to_owned();
    let bad_tok = report_token(f.reader_id, &private_id);
    let (status, body) = call(
        &f.app,
        "POST",
        &format!("/api/render/maps/{m_used}/geojson"),
        "",
        Some(json!({ "token": bad_tok })),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    assert_eq!(body["error"], "render token is invalid or expired");
}
