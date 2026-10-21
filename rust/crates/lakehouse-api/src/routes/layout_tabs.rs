//! `BI-18` part C: tabs on the dashboard payload, the tab ties of a filter,
//! the tab rules of a board save, and a section tile that never reaches the
//! engine. A wiremock `ClickHouse` answers the store reads and records every
//! statement it is sent (the helpers are `typed_filters`'), so the assertions
//! are about the statements a tile was really sent.

#![cfg(test)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use lakehouse_bi::access::Viewer;
use lakehouse_bi::store;
use lakehouse_core::ApiError;
use serde_json::{Value, json};
use wiremock::matchers::{body_string_contains, method};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::typed_filters::{answer, answer_rest, chart_row, client, mart_column_rows, statements};
use super::{
    BoardEditBody, DashboardQuery, GrainRequest, check_tab_rules, filters_for_chart_tab, get_body,
    parse_filters_param,
};
use crate::policy_engine::PolicyEngineObligations;
use crate::sql_rewrite::PlaceholderValues;

const BOARD_COLUMNS: [&str; 5] = [
    "id",
    "name",
    "layout_json",
    "tabs_json",
    "phone_layout_json",
];

fn board_row(layout: &Value, tabs: &Value, phone: &Value) -> Value {
    json!({
        "id": "b1", "name": "Tabbed",
        "layout_json": layout.to_string(),
        "tabs_json": tabs.to_string(),
        "phone_layout_json": phone.to_string(),
    })
}

fn tab_list() -> Value {
    json!([{ "id": "t_a", "title": "Overview" }, { "id": "t_b", "title": "Maps" }])
}

/// A bar tile on `mart_a` (tab `t_a`), one on `mart_b` (tab `t_b`).
fn two_tab_layout() -> Value {
    json!({
        "c_a": { "x": 0, "y": 0, "w": 6, "h": 4, "tab": "t_a" },
        "c_b": { "x": 0, "y": 0, "w": 6, "h": 4, "tab": "t_b" },
    })
}

fn heading_row(id: &str, smuggled_sql: &str) -> Value {
    // A heading written by hand: its stored statement is not empty.
    let spec = json!({
        "id": id, "title": "Hostile", "kind": "heading", "mart": "mart_a",
        "sql": smuggled_sql, "x": "kab", "y": "visitors", "text": "Section"
    });
    let def = json!({
        "title": "Hostile", "kind": "heading", "mart": "mart_a",
        "dimension": "kab", "measures": ["visitors"], "text": "Section"
    });
    json!({
        "id": id, "board": "b1", "created_by": "ui", "created_at": "2026-01-01 00:00:00",
        "spec_json": json!({ "spec": spec, "def": def, "hasYear": false }).to_string(),
    })
}

async fn mount(server: &MockServer, board: Value, extra_charts: Vec<Value>) {
    let mut charts = vec![
        chart_row("c_a", "b1", "mart_a", None),
        chart_row("c_b", "b1", "mart_b", None),
    ];
    charts.extend(extra_charts);
    answer(
        server,
        "FROM console.bi_chart FINAL",
        &["id", "board", "created_by", "created_at", "spec_json"],
        charts,
    )
    .await;
    answer(
        server,
        "FROM console.bi_board FINAL",
        &BOARD_COLUMNS,
        vec![board],
    )
    .await;
    let mut cols = mart_column_rows(
        "mart_a",
        &[("day", "Date"), ("visitors", "UInt32"), ("kab", "String")],
    );
    cols.extend(mart_column_rows(
        "mart_b",
        &[("visitors", "UInt32"), ("kab", "String")],
    ));
    answer(
        server,
        "SELECT table, name, type FROM system.columns",
        &["table", "name", "type"],
        cols,
    )
    .await;
    answer_rest(server).await;
}

async fn dashboard(server: &MockServer, filters: &str) -> Value {
    let ch = client(server);
    let obligations = PolicyEngineObligations::new(None, &ch);
    let q = DashboardQuery {
        board: Some("b1".to_owned()),
        year: None,
        filters: Some(filters.to_owned()),
        grain: None,
        params: None,
    };
    let parsed = parse_filters_param(q.filters.as_deref()).expect("valid filters");
    get_body(
        &ch,
        &q,
        parsed,
        &[],
        &PlaceholderValues::none(),
        &obligations,
        (
            &lakehouse_bi::grain::TimeContext::default(),
            GrainRequest::Saved,
            &Viewer::system(),
        ),
    )
    .await
    .expect("the payload builds")
}

async fn statement_for(server: &MockServer, needle: &str) -> String {
    let sent = statements(server).await;
    sent.iter()
        .find(|s| s.contains(needle))
        .cloned()
        .unwrap_or_else(|| panic!("no statement for {needle}: {sent:?}"))
}

const DAY_RANGE: &str =
    r#"{"column":"day","op":"relative","anchor":"last","n":30,"unit":"day","tabs":["t_a"]}"#;

/// `BI-18C-AC1`: a filter tied to one tab leaves the other tab's tiles
/// unchanged; one that is not tied changes both.
#[tokio::test]
async fn a_filter_tied_to_one_tab_is_not_given_to_the_tiles_of_the_other() {
    let server = MockServer::start().await;
    mount(
        &server,
        board_row(&two_tab_layout(), &tab_list(), &json!([])),
        Vec::new(),
    )
    .await;
    let body = dashboard(
        &server,
        &format!(r#"[{DAY_RANGE},{{"column":"visitors","op":"between","min":"100"}}]"#),
    )
    .await;

    let on_a = statement_for(&server, "FROM serving.mart_a WHERE").await;
    assert!(
        on_a.contains("subtractDays"),
        "tied filter on its tab: {on_a}"
    );
    assert!(on_a.contains("visitors >= 100"), "{on_a}");
    let on_b = statement_for(&server, "FROM serving.mart_b WHERE").await;
    assert!(!on_b.contains("subtractDays"), "tied elsewhere: {on_b}");
    assert!(
        on_b.contains("visitors >= 100"),
        "the untied one applies: {on_b}"
    );
    // `mart_b` has no `day`; the filter is not tied to its tab, so the tile is
    // not reported as having skipped it.
    assert!(body["results"]["c_b"].get("filtersSkipped").is_none());
    assert!(body["results"]["c_a"].get("filtersSkipped").is_none());
    // The filters in force are reported whole.
    assert_eq!(body["filters"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn a_tile_with_no_tab_sits_on_the_first_and_gets_its_tied_filters() {
    let server = MockServer::start().await;
    let layout = json!({ "c_a": { "x": 0, "y": 0, "w": 6, "h": 4 } });
    mount(
        &server,
        board_row(&layout, &tab_list(), &json!([])),
        Vec::new(),
    )
    .await;
    let _ = dashboard(&server, &format!("[{DAY_RANGE}]")).await;
    // c_a has no tile tab and c_b no tile at all: both are on the first tab.
    let on_a = statement_for(&server, "FROM serving.mart_a WHERE").await;
    assert!(on_a.contains("subtractDays"), "{on_a}");
}

#[tokio::test]
async fn on_a_board_without_tabs_a_tie_is_ignored_and_the_skip_is_still_reported() {
    let server = MockServer::start().await;
    mount(
        &server,
        board_row(&json!({}), &json!([]), &json!([])),
        Vec::new(),
    )
    .await;
    let body = dashboard(&server, &format!("[{DAY_RANGE}]")).await;
    // `mart_a` takes the range; `mart_b` has no `day` and runs its stored SQL.
    let on_a = statement_for(&server, "FROM serving.mart_a WHERE").await;
    assert!(on_a.contains("subtractDays"), "{on_a}");
    assert_eq!(
        body["results"]["c_b"]["filtersSkipped"],
        json!([{ "column": "day", "reason": "no_column" }]),
        "untied behaviour is unchanged"
    );
    assert!(body.get("tabs").is_none() && body.get("phoneLayout").is_none());
}

#[tokio::test]
async fn the_payload_carries_the_tabs_the_tile_tabs_and_the_phone_layout() {
    let server = MockServer::start().await;
    let phone = json!([{ "id": "c_b", "h": 5 }, { "id": "c_a", "h": 8 }]);
    mount(
        &server,
        board_row(&two_tab_layout(), &tab_list(), &phone),
        Vec::new(),
    )
    .await;
    let body = dashboard(&server, "[]").await;
    assert_eq!(body["tabs"], tab_list());
    assert_eq!(body["phoneLayout"], phone);
    assert_eq!(body["layout"]["c_b"]["tab"], "t_b");
    assert_eq!(
        body["layout"]["c_a"],
        json!({ "x": 0, "y": 0, "w": 6, "h": 4, "tab": "t_a" })
    );
}

#[tokio::test]
async fn the_payload_carries_the_theme_and_font_only_when_the_board_has_them() {
    let server = MockServer::start().await;
    let mut row = board_row(&json!({}), &json!([]), &json!([]));
    row["theme"] = json!("dark");
    row["font"] = json!("source-serif");
    mount(&server, row, Vec::new()).await;
    let body = dashboard(&server, "[]").await;
    assert_eq!(
        (body["theme"].as_str(), body["font"].as_str()),
        (Some("dark"), Some("source-serif"))
    );
    let plain = MockServer::start().await;
    mount(
        &plain,
        board_row(&json!({}), &json!([]), &json!([])),
        Vec::new(),
    )
    .await;
    let body = dashboard(&plain, "[]").await;
    assert!(body.get("theme").is_none() && body.get("font").is_none());
}

#[tokio::test]
async fn a_layout_saved_before_tabs_is_served_without_a_tab_key() {
    let server = MockServer::start().await;
    let layout = json!({ "c_a": { "x": 0, "y": 0, "w": 6, "h": 4 } });
    mount(
        &server,
        board_row(&layout, &json!([]), &json!([])),
        Vec::new(),
    )
    .await;
    let body = dashboard(&server, "[]").await;
    assert_eq!(body["layout"], layout);
}

/// The section kinds have no statement: a heading whose stored definition holds
/// a mart and SQL (a row written by hand) is read, and the engine is never
/// sent that SQL.
#[tokio::test]
async fn a_heading_with_sql_in_its_stored_definition_never_reaches_the_engine() {
    let server = MockServer::start().await;
    mount(
        &server,
        board_row(&two_tab_layout(), &tab_list(), &json!([])),
        vec![heading_row("c_h", "SELECT secret FROM serving.mart_a")],
    )
    .await;
    let body = dashboard(
        &server,
        r#"[{"column":"visitors","op":"between","min":"1"}]"#,
    )
    .await;
    let sent = statements(&server).await;
    assert!(
        sent.iter().all(|s| !s.contains("secret")),
        "the heading's SQL was sent: {sent:?}"
    );
    assert_eq!(body["results"]["c_h"], json!({ "columns": [], "rows": [] }));
    // And it is not counted as a tile that reads `kab`, nor reported as skipped.
    assert!(body["results"]["c_h"].get("filtersSkipped").is_none());
}

fn edit(value: Value) -> BoardEditBody {
    serde_json::from_value(value).unwrap()
}

fn rejected(result: Result<(), ApiError>) -> String {
    match result {
        Err(ApiError::BadRequest(message)) => message,
        other => panic!("expected a 400, got {other:?}"),
    }
}

#[tokio::test]
async fn a_save_that_leaves_a_tile_on_a_tab_that_does_not_exist_is_refused_before_any_write() {
    let server = MockServer::start().await;
    mount(
        &server,
        board_row(&json!({}), &tab_list(), &json!([])),
        Vec::new(),
    )
    .await;
    let ch = client(&server);
    let bad_tile = edit(json!({
        "id": "b1", "layout": { "c_a": { "x": 0, "y": 0, "w": 6, "h": 4, "tab": "t_gone" } }
    }));
    assert!(
        rejected(check_tab_rules(&ch, "b1", &bad_tile).await).contains("tab that does not exist")
    );
    let good_tile = edit(json!({
        "id": "b1", "layout": { "c_a": { "x": 0, "y": 0, "w": 6, "h": 4, "tab": "t_b" } }
    }));
    check_tab_rules(&ch, "b1", &good_tile).await.unwrap();
    // Nothing was written by a check.
    assert!(
        statements(&server)
            .await
            .iter()
            .all(|s| !s.contains("INSERT"))
    );
}

#[tokio::test]
async fn a_tie_to_a_missing_tab_and_a_delete_that_strands_a_tile_or_a_tie_are_refused() {
    let server = MockServer::start().await;
    let stored_tied = json!([
        { "column": "day", "values": [], "op": "relative", "unit": "day", "anchor": "last", "n": 3, "tabs": ["t_b"] }
    ]);
    mount(
        &server,
        {
            let mut row = board_row(&two_tab_layout(), &tab_list(), &json!([]));
            row["filters_json"] = json!(stored_tied.to_string());
            row
        },
        Vec::new(),
    )
    .await;
    let ch = client(&server);

    let tie_missing = edit(json!({
        "id": "b1", "filters": [{ "column": "kab", "values": ["a"], "tabs": ["nope"] }]
    }));
    assert!(rejected(check_tab_rules(&ch, "b1", &tie_missing).await).contains("filter is tied"));

    // Deleting `t_b` while the stored layout still has a tile on it...
    let drops_tab = edit(json!({ "id": "b1", "tabs": [{ "id": "t_a", "title": "Overview" }] }));
    assert!(rejected(check_tab_rules(&ch, "b1", &drops_tab).await).contains("tile is on a tab"));

    // ...and with the tile moved, the stored filter tied to `t_b` is what stops it.
    let moved = edit(json!({
        "id": "b1",
        "tabs": [{ "id": "t_a", "title": "Overview" }],
        "layout": {
            "c_a": { "x": 0, "y": 0, "w": 6, "h": 4, "tab": "t_a" },
            "c_b": { "x": 0, "y": 4, "w": 6, "h": 4, "tab": "t_a" },
        }
    }));
    assert!(rejected(check_tab_rules(&ch, "b1", &moved).await).contains("filter is tied"));

    // Moving the tiles and the tie in the one save is accepted.
    let whole = edit(json!({
        "id": "b1",
        "tabs": [{ "id": "t_a", "title": "Overview" }],
        "layout": { "c_a": { "x": 0, "y": 0, "w": 6, "h": 4, "tab": "t_a" } },
        "filters": [{ "column": "kab", "values": ["a"] }],
    }));
    check_tab_rules(&ch, "b1", &whole).await.unwrap();

    // Tab shape is checked with our sentences, before any row is read.
    for tabs in [
        json!([{ "id": "a", "title": " " }]),
        json!([{ "id": "a b", "title": "x" }]),
        json!([{ "id": "a", "title": "x" }, { "id": "a", "title": "y" }]),
    ] {
        let bad = edit(json!({ "id": "b1", "tabs": tabs }));
        assert!(!rejected(check_tab_rules(&ch, "b1", &bad).await).is_empty());
    }
    let long_phone = edit(json!({ "id": "b1", "phoneLayout": [{ "id": "c_a", "h": 0 }] }));
    assert!(rejected(check_tab_rules(&ch, "b1", &long_phone).await).contains("rows high"));
}

#[tokio::test]
async fn a_plain_layout_or_name_save_reads_no_board_row() {
    let server = MockServer::start().await;
    answer_rest(&server).await;
    let ch = client(&server);
    let plain = edit(json!({
        "id": "b1", "name": "New name",
        "layout": { "c_a": { "x": 0, "y": 0, "w": 6, "h": 4 } },
        "filters": [{ "column": "kab", "values": ["a"] }],
    }));
    check_tab_rules(&ch, "b1", &plain).await.unwrap();
    assert!(
        statements(&server).await.is_empty(),
        "no read for an untabbed save"
    );
}

#[tokio::test]
async fn the_built_in_board_takes_a_phone_layout_but_not_tabs() {
    let allowed = |body: Value| super::board_edit_allowed(&edit(body));
    assert!(allowed(json!({ "id": "default", "layout": {} })));
    assert!(allowed(
        json!({ "id": "default", "phoneLayout": [{ "id": "k", "h": 3 }] })
    ));
    assert!(!allowed(
        json!({ "id": "default", "tabs": [{ "id": "a", "title": "A" }] })
    ));
}

#[tokio::test]
async fn a_records_list_or_export_of_a_tile_drops_the_filters_tied_to_other_tabs() {
    let server = MockServer::start().await;
    mount(
        &server,
        board_row(&two_tab_layout(), &tab_list(), &json!([])),
        Vec::new(),
    )
    .await;
    let ch = client(&server);
    let stored = store::list_stored_charts(&ch, &Viewer::system())
        .await
        .unwrap();
    let chart = |id: &str| stored.iter().find(|c| c.spec.id == id).unwrap();
    let tied: Vec<_> = parse_filters_param(Some(&format!(
        r#"[{DAY_RANGE},{{"column":"visitors","op":"between","min":"1"}}]"#
    )))
    .unwrap()
    .unwrap();
    // c_b is on t_b: the t_a-tied filter goes, the untied one stays.
    let for_b = filters_for_chart_tab(&ch, chart("c_b"), tied.clone()).await;
    assert_eq!(for_b.len(), 1);
    assert_eq!(for_b[0].column, "visitors");
    assert_eq!(
        filters_for_chart_tab(&ch, chart("c_a"), tied.clone())
            .await
            .len(),
        2
    );
    // No tie, no read: the same call on untied filters asks the store nothing.
    let before = statements(&server).await.len();
    let untied = vec![tied[1].clone()];
    assert_eq!(
        filters_for_chart_tab(&ch, chart("c_b"), untied).await.len(),
        1
    );
    assert_eq!(statements(&server).await.len(), before);
}

#[tokio::test]
async fn a_board_that_cannot_be_read_leaves_every_filter_on_the_tile() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        // The board read; the chart read's sub-selects ask `is_deleted = 2` and
        // `owner_id != ''` of the same table and must still be answered.
        .and(body_string_contains(
            "FROM console.bi_board FINAL WHERE is_deleted = 0",
        ))
        .respond_with(ResponseTemplate::new(500).set_body_string("boom: secret detail"))
        .mount(&server)
        .await;
    answer(
        &server,
        "FROM console.bi_chart FINAL",
        &["id", "board", "created_by", "created_at", "spec_json"],
        vec![chart_row("c_b", "b1", "mart_b", None)],
    )
    .await;
    answer_rest(&server).await;
    let ch = client(&server);
    let stored = store::list_stored_charts(&ch, &Viewer::system())
        .await
        .unwrap();
    let chart = &stored[0];
    let filters = parse_filters_param(Some(&format!("[{DAY_RANGE}]")))
        .unwrap()
        .unwrap();
    assert_eq!(filters_for_chart_tab(&ch, chart, filters).await.len(), 1);
}

// ── Verification against the real engine (plan section 5) ─────────────────
//
// `cargo test -p lakehouse-api --lib duplicate_of_a_real_board -- --ignored
// --nocapture` with `BI18C_DUMP` naming a JSON file `{ "charts": [...],
// "board": {...} }` dumped from the dev `ClickHouse` (see the plan's handoff)
// and `BI18C_ENGINE` its HTTP address. Every `SELECT` is forwarded to that
// engine; the store's own reads of `console.bi_chart` and `console.bi_board`
// are answered from the dump plus whatever `duplicate_board` wrote, and every
// write is captured instead of sent, so nothing in the engine changes.

mod verify {
    use std::process::{Command, Stdio};
    use std::sync::{Arc, Mutex};

    use serde_json::{Value, json};
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

    use super::*;

    #[derive(Default)]
    struct Store {
        charts: Vec<Value>,
        boards: Vec<Value>,
        writes: Vec<String>,
    }

    struct Engine {
        base: String,
        store: Arc<Mutex<Store>>,
    }

    fn table(rows: &[Value], columns: &[&str]) -> ResponseTemplate {
        let meta: Vec<Value> = columns
            .iter()
            .map(|c| json!({ "name": c, "type": "String" }))
            .collect();
        ResponseTemplate::new(200)
            .set_body_json(json!({ "meta": meta, "data": rows, "rows": rows.len() }))
    }

    impl Respond for Engine {
        fn respond(&self, request: &Request) -> ResponseTemplate {
            let sql = String::from_utf8_lossy(&request.body).into_owned();
            let head = sql.trim_start().to_ascii_uppercase();
            let mut store = self.store.lock().unwrap();
            if head.starts_with("CREATE") || head.starts_with("ALTER") {
                return ResponseTemplate::new(200);
            }
            if head.starts_with("INSERT") {
                store.writes.push(sql);
                return ResponseTemplate::new(200);
            }
            if sql.contains("FROM console.bi_chart FINAL") {
                return table(
                    &store.charts,
                    &["id", "spec_json", "board", "created_by", "created_at"],
                );
            }
            if sql.contains("FROM console.bi_board FINAL") {
                let wanted = sql.split("id=").nth(1).map(|rest| {
                    rest.trim_start_matches('\'')
                        .split('\'')
                        .next()
                        .unwrap_or("")
                });
                let rows: Vec<Value> = store
                    .boards
                    .iter()
                    .filter(|b| wanted.is_none_or(|id| b["id"] == id))
                    .cloned()
                    .collect();
                return table(&rows, &["id"]);
            }
            assert!(
                head.starts_with("SELECT") || head.starts_with("WITH"),
                "only reads are forwarded: {sql}"
            );
            let mut child = Command::new("curl")
                .args(["-s", "--data-binary", "@-", &self.base])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()
                .unwrap();
            std::io::Write::write_all(&mut child.stdin.take().unwrap(), sql.as_bytes()).unwrap();
            let out = child.wait_with_output().unwrap();
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/json")
                .set_body_bytes(out.stdout)
        }
    }

    /// The values of one `INSERT ... VALUES (...)` as text, in order, undoing
    /// `SqlLiteral`'s escapes (`\\` and `''`).
    fn values_of(insert: &str) -> Vec<String> {
        let start = insert.find("VALUES").unwrap() + "VALUES".len();
        let mut out = Vec::new();
        let mut chars = insert[start..].chars().peekable();
        let mut current = String::new();
        let mut in_string = false;
        while let Some(c) = chars.next() {
            if in_string {
                match c {
                    '\\' => current.push(chars.next().unwrap_or('\\')),
                    // `SqlLiteral` doubles a quote and a backslash.
                    '\'' if chars.peek() == Some(&'\'') => {
                        chars.next();
                        current.push('\'');
                    }
                    '\'' => {
                        in_string = false;
                        out.push(std::mem::take(&mut current));
                    }
                    other => current.push(other),
                }
            } else if c == '\'' {
                in_string = true;
            } else if c.is_ascii_digit() || c == '-' {
                current.push(c);
                while let Some(d) = chars.peek().copied().filter(char::is_ascii_digit) {
                    current.push(d);
                    chars.next();
                }
                out.push(std::mem::take(&mut current));
            }
        }
        out
    }

    fn columns_of(insert: &str) -> Vec<String> {
        let open = insert.find('(').unwrap() + 1;
        let close = insert[open..].find(')').unwrap() + open;
        insert[open..close]
            .split(',')
            .map(|c| c.trim().to_owned())
            .collect()
    }

    async fn read_board(ch: &lakehouse_clickhouse::ChClient, id: &str) -> Value {
        let q = DashboardQuery {
            board: Some(id.to_owned()),
            year: None,
            filters: None,
            grain: None,
            params: None,
        };
        let obligations = PolicyEngineObligations::new(None, ch);
        get_body(
            ch,
            &q,
            None,
            &[],
            &PlaceholderValues::none(),
            &obligations,
            (
                &lakehouse_bi::grain::TimeContext::default(),
                GrainRequest::Saved,
                &Viewer::system(),
            ),
        )
        .await
        .expect("the board reads")
    }

    /// Put what a duplicate wrote where the next read finds it; the new
    /// charts, in the order they were written.
    fn register_writes(store: &Arc<Mutex<Store>>) -> Vec<Value> {
        let writes = store.lock().unwrap().writes.clone();
        let mut copies = Vec::new();
        let mut last_board = None;
        for sql in &writes {
            let values = values_of(sql);
            if sql.contains("INSERT INTO console.bi_chart") {
                copies.push(json!({
                    "id": values[0], "spec_json": values[2], "board": values[3],
                    "created_by": values[4], "created_at": "2026-10-14 00:00:00",
                }));
            } else if sql.contains("INSERT INTO console.bi_board") {
                let mut row = serde_json::Map::new();
                for (k, v) in columns_of(sql).into_iter().zip(values) {
                    row.insert(k, json!(v));
                }
                row.insert("created_at".to_owned(), json!("2026-10-14 00:00:00"));
                last_board = Some(Value::Object(row));
            }
        }
        let mut store = store.lock().unwrap();
        store.charts.extend(copies.clone());
        store.boards.push(last_board.expect("the copy's board row"));
        copies
    }

    async fn mounted(store: &Arc<Mutex<Store>>) -> (MockServer, lakehouse_clickhouse::ChClient) {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(Engine {
                base: std::env::var("BI18C_ENGINE").unwrap(),
                store: store.clone(),
            })
            .mount(&server)
            .await;
        let ch = client(&server);
        (server, ch)
    }

    /// The built-in board (`BUILTIN_DASHBOARD_SPEC` set to the shipped
    /// catalogue): its copy returns the same numbers tile by tile.
    #[tokio::test]
    #[ignore = "needs the dev ClickHouse (BI18C_ENGINE) and BUILTIN_DASHBOARD_SPEC; see the plan's handoff"]
    async fn duplicate_of_the_built_in_board_returns_the_same_numbers() {
        let store = Arc::new(Mutex::new(Store::default()));
        let (_server, ch) = mounted(&store).await;
        let copy =
            store::duplicate_board(&ch, store::DEFAULT_BOARD_ID, "verify", &Viewer::system())
                .await
                .unwrap();
        let copies = register_writes(&store);
        let builtin: Vec<String> = lakehouse_bi::specs::kpis()
            .iter()
            .map(|k| k.id.clone())
            .chain(lakehouse_bi::specs::charts().iter().map(|c| c.id.clone()))
            .collect();
        assert!(!builtin.is_empty(), "set BUILTIN_DASHBOARD_SPEC");
        assert_eq!(copies.len(), builtin.len());
        let before = read_board(&ch, store::DEFAULT_BOARD_ID).await;
        let after = read_board(&ch, &copy.id).await;
        for (old, new) in builtin
            .iter()
            .zip(copies.iter().map(|c| c["id"].as_str().unwrap()))
        {
            let (a, b) = (&before["results"][old], &after["results"][new]);
            assert!(a["rows"].is_array(), "built-in tile {old}: {a}");
            assert_eq!(a, b, "built-in tile {old} vs its copy {new}");
            println!(
                "{old} -> {new}: {} rows, first {}",
                a["rows"].as_array().unwrap().len(),
                a["rows"][0]
            );
        }
        println!(
            "copy {} named {:?}; {} built-in tiles compared",
            copy.id,
            copy.name,
            builtin.len()
        );
    }

    #[tokio::test]
    #[ignore = "needs the dev ClickHouse (BI18C_ENGINE) and a dump (BI18C_DUMP); see the plan's handoff"]
    async fn duplicate_of_a_real_board_returns_the_same_numbers() {
        let dump: Value = serde_json::from_str(
            &std::fs::read_to_string(std::env::var("BI18C_DUMP").unwrap()).unwrap(),
        )
        .unwrap();
        let source = dump["board"]["id"].as_str().unwrap().to_owned();
        let store = Arc::new(Mutex::new(Store {
            charts: dump["charts"].as_array().unwrap().clone(),
            boards: vec![dump["board"].clone()],
            writes: Vec::new(),
        }));
        let (_server, ch) = mounted(&store).await;
        let originals: Vec<String> = store::list_stored_charts(&ch, &Viewer::system())
            .await
            .unwrap()
            .into_iter()
            .filter(|c| c.board == source)
            .map(|c| c.spec.id)
            .collect();
        let copy = store::duplicate_board(&ch, &source, "verify", &Viewer::system())
            .await
            .unwrap();

        let copies = register_writes(&store);
        assert_eq!(copies.len(), originals.len(), "one new chart per chart");
        println!("copy {} named {:?}", copy.id, copy.name);

        let before = read_board(&ch, &source).await;
        let after = read_board(&ch, &copy.id).await;
        let mut compared = 0;
        for (old, new) in originals
            .iter()
            .zip(copies.iter().map(|c| c["id"].as_str().unwrap()))
        {
            let (a, b) = (&before["results"][old], &after["results"][new]);
            assert_eq!(a, b, "tile {old} vs its copy {new}");
            let rows = a["rows"].as_array().map_or(0, Vec::len);
            let first = a["rows"]
                .get(0)
                .map(ToString::to_string)
                .unwrap_or_default();
            println!("{old} -> {new}: {rows} rows, first {first}");
            compared += 1;
        }
        println!("{compared} tiles compared; tabs {:?}", after.get("tabs"));

        // The same board split in two tabs, with a filter on `provinsi`
        // tied to the first: the first tab's tiles that have the column are
        // narrowed, the second tab's are what they were.
        let new_ids: Vec<&str> = copies.iter().map(|c| c["id"].as_str().unwrap()).collect();
        let half = new_ids.len() / 2;
        let mut layout = serde_json::Map::new();
        for (i, id) in new_ids.iter().enumerate() {
            let tab = if i < half { "t_a" } else { "t_b" };
            layout.insert(
                (*id).to_owned(),
                json!({ "x": 0, "y": i64::try_from(i).unwrap() * 6, "w": 12, "h": 6, "tab": tab }),
            );
        }
        {
            let mut store = store.lock().unwrap();
            let row = store
                .boards
                .iter_mut()
                .find(|b| b["id"] == copy.id.as_str())
                .unwrap();
            row["layout_json"] = json!(Value::Object(layout).to_string());
            row["tabs_json"] = json!(r#"[{"id":"t_a","title":"A"},{"id":"t_b","title":"B"}]"#);
            row["filters_json"] =
                json!(r#"[{"column":"provinsi","values":["Bali"],"tabs":["t_a"]}]"#);
        }
        let tied = read_board(&ch, &copy.id).await;
        let (mut narrowed, mut untouched) = (0, 0);
        for (i, id) in new_ids.iter().enumerate() {
            let rows = tied["results"][id]["rows"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            if i < half {
                // A pivot's subtotal rows have an empty cell where the column is rolled up.
                let provinces: Vec<&Value> = rows
                    .iter()
                    .filter_map(|r| r.get("provinsi"))
                    .filter(|p| p.as_str() != Some(""))
                    .collect();
                if !provinces.is_empty() {
                    assert!(
                        provinces.iter().all(|p| *p == "Bali"),
                        "tile {id} on the tied tab: {provinces:?}"
                    );
                    narrowed += 1;
                }
            } else {
                assert_eq!(
                    tied["results"][id], after["results"][id],
                    "tile {id} on the other tab"
                );
                untouched += 1;
            }
        }
        println!(
            "{narrowed} tiles of tab A narrowed to Bali, {untouched} tiles of tab B unchanged"
        );
        assert!(narrowed > 0 && untouched > 0);
        assert_eq!(tied["tabs"][1]["id"], "t_b");
    }
}
