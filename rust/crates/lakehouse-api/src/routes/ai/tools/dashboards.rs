//! Dashboard/chart tools: `create_chart`, `update_chart`, `delete_chart`,
//! `create_board`, `list_boards`, `list_charts`, `suggest_dashboard`.
//! Moved out of `ai.rs` unchanged (T0.1 registry refactor).

use lakehouse_auth::Principal;
use lakehouse_bi::access::Viewer;
use lakehouse_bi::specs::ChartSource;
use lakehouse_bi::store::{self, ChartInput};
use lakehouse_clickhouse::ChClient;
use serde_json::{Map, Value, json};

use super::arg_str;
use crate::routes::dashboard_organise::{
    Gate, drop_tile, place_new_chart, require_chart_board, require_editable_chart, snapshot_as,
    viewer_of, visible_live_chart_board,
};
use crate::routes::support::is_numeric_type;
use crate::upstream_error;

/// Who the tools read as: the signed-in person, or nobody in particular when
/// none is attached (fail closed: no personal space, nothing in the trash).
fn viewer_for(principal: Option<&Principal>) -> Viewer {
    principal.map_or_else(Viewer::anonymous, viewer_of)
}

/// The tool's error for a refused dashboard or chart: the route's own
/// sentence, or, for a store that could not be read, the fixed message with
/// its reference id (never the database's text).
pub(super) fn refused(gate: Gate) -> Value {
    match gate {
        Gate::Missing(sentence) => json!({ "error": sentence }),
        Gate::Store(cause) => {
            upstream_error::report_ch(&upstream_error::DATABASE, &cause).to_json()
        }
    }
}

/// Parse a tool call's raw args `Value` into a [`ChartInput`], the same way
/// `args as unknown as ChartInput` casts in `ai-tools.ts` (no validation at
/// this boundary — `spec_from_input` validates for real).
fn parse_chart_input(args: &Map<String, Value>) -> Result<ChartInput, String> {
    serde_json::from_value(Value::Object(args.clone())).map_err(|e| e.to_string())
}

pub(super) async fn create_chart(
    ch: &ChClient,
    principal: Option<&Principal>,
    args: &Map<String, Value>,
    id: Option<String>,
    time: &lakehouse_bi::grain::TimeContext,
) -> Value {
    let input = match parse_chart_input(args) {
        Ok(i) => i,
        Err(e) => return json!({ "error": e }),
    };
    // BI-18 part C, part 2: not onto a dashboard in the trash or in somebody
    // else's personal space.
    if let Err(err) = require_chart_board(ch, input.board.as_deref(), &viewer_for(principal)).await
    {
        return refused(err);
    }
    match store::spec_from_input(ch, &input, ChartSource::Ai, "ai", id, time).await {
        Ok(spec) => match store::insert_chart(ch, &spec).await {
            Ok(()) => {
                place_new_chart(ch, &spec.board, &spec.spec.id).await;
                snapshot_as(ch, &spec.board, principal).await;
                json!({
                "created": true,
                "id": spec.spec.id,
                "title": spec.spec.title,
                "kind": spec.spec.kind,
                "mart": spec.spec.mart,
                "board": spec.board,
                "url": "/dashboards",
                "note": "Chart tersimpan & langsung tampil di halaman Dashboards.",
                })
            }
            Err(err) => upstream_error::report_ch(&upstream_error::DATABASE, &err).to_json(),
        },
        Err(err) => upstream_error::bi(&upstream_error::DATABASE, &err).to_json(),
    }
}

/// Checks a chart draft before the gate offers it: a draft that cannot be
/// built must not be offered as a card the person can confirm. It runs the
/// same validation as the tools (`store::spec_from_input`) and stops there,
/// so it writes nothing.
///
/// `id` is `None` for a create and `Some` for an update; an update with no
/// `id` is refused here, as the tool refuses it. Returns `None` when the
/// draft can be built, and the error the tool would return otherwise.
pub(super) async fn check_chart_draft(
    ch: &ChClient,
    args: &Map<String, Value>,
    id: Option<String>,
    time: &lakehouse_bi::grain::TimeContext,
) -> Option<Value> {
    if id.as_deref().is_some_and(str::is_empty) {
        return Some(json!({ "error": "id is required" }));
    }
    let input = match parse_chart_input(args) {
        Ok(i) => i,
        Err(e) => return Some(json!({ "error": e })),
    };
    match store::spec_from_input(ch, &input, ChartSource::Ai, "ai", id, time).await {
        Ok(_) => None,
        Err(err) => Some(upstream_error::bi(&upstream_error::DATABASE, &err).to_json()),
    }
}

pub(super) async fn update_chart(
    ch: &ChClient,
    principal: Option<&Principal>,
    args: &Map<String, Value>,
    time: &lakehouse_bi::grain::TimeContext,
) -> Value {
    let id = arg_str(args, "id");
    if id.is_empty() {
        return json!({ "error": "id is required" });
    }
    let input = match parse_chart_input(args) {
        Ok(i) => i,
        Err(e) => return json!({ "error": e }),
    };
    // BI-18 part C, part 2: a chart in the trash or on somebody else's
    // personal board cannot be edited, nor moved onto such a board.
    let viewer = viewer_for(principal);
    let old_board = match require_editable_chart(ch, &id, &viewer).await {
        Ok(board) => board,
        Err(err) => return refused(err),
    };
    if let Err(err) = require_chart_board(ch, input.board.as_deref(), &viewer).await {
        return refused(err);
    }
    match store::spec_from_input(ch, &input, ChartSource::Ai, "ai", Some(id), time).await {
        Ok(spec) => match store::insert_chart(ch, &spec).await {
            Ok(()) => {
                if let Some(old) = old_board.filter(|b| *b != spec.board) {
                    snapshot_as(ch, &old, principal).await;
                }
                snapshot_as(ch, &spec.board, principal).await;
                json!({
                    "updated": true,
                    "id": spec.spec.id,
                    "title": spec.spec.title,
                    "kind": spec.spec.kind,
                    "mart": spec.spec.mart,
                })
            }
            Err(err) => upstream_error::report_ch(&upstream_error::DATABASE, &err).to_json(),
        },
        Err(err) => upstream_error::bi(&upstream_error::DATABASE, &err).to_json(),
    }
}

pub(super) async fn create_board(
    ch: &ChClient,
    principal: Option<&Principal>,
    args: &Map<String, Value>,
) -> Value {
    // `created_by` is left empty: the assistant is not a person, and naming
    // it as the author would put a non-account into a column the UI shows
    // as an owner.
    match store::create_board(
        ch,
        &arg_str(args, "name"),
        &arg_str(args, "description"),
        "",
    )
    .await
    {
        Ok(board) => {
            snapshot_as(ch, &board.id, principal).await;
            json!({
                "created": true, "id": board.id, "name": board.name,
                "note": "Pakai id ini di create_chart.board.",
            })
        }
        Err(err) => upstream_error::bi(&upstream_error::DATABASE, &err).to_json(),
    }
}

pub(super) async fn list_boards(ch: &ChClient, principal: Option<&Principal>) -> Value {
    match store::list_boards(ch, &viewer_for(principal)).await {
        Ok(boards) => {
            let mut out = vec![json!({ "id": "default", "name": "Main" })];
            // BI-18 part C: a board's tabs, when it has any, so the assistant can name them.
            out.extend(boards.iter().map(|b| {
                let mut entry = json!({ "id": b.id, "name": b.name });
                if let Some(tabs) = b.tabs.as_ref().filter(|t| !t.is_empty()) {
                    entry["tabs"] = json!(tabs);
                }
                entry
            }));
            json!({ "boards": out })
        }
        Err(err) => upstream_error::report_ch(&upstream_error::DATABASE, &err).to_json(),
    }
}

pub(super) async fn suggest_dashboard(ch: &ChClient) -> Value {
    let marts = match ch
        .rows(
            "SELECT name, toString(total_rows) AS total_rows FROM system.tables \
             WHERE database='serving' AND name NOT LIKE '%\\_baru' ORDER BY name",
            None,
        )
        .await
    {
        Ok(r) => r,
        Err(err) => return upstream_error::report_ch(&upstream_error::DATABASE, &err).to_json(),
    };
    let mut out = Vec::with_capacity(marts.len());
    for m in &marts {
        let name = m.get("name").and_then(Value::as_str).unwrap_or("");
        let cols = match ch
            .rows(
                &format!("SELECT name, type FROM system.columns WHERE database='serving' AND table='{name}' ORDER BY position"),
                None,
            )
            .await
        {
            Ok(r) => r,
            Err(err) => return upstream_error::report_ch(&upstream_error::DATABASE, &err).to_json(),
        };
        let dimensions: Vec<&str> = cols
            .iter()
            .filter(|c| !is_numeric_type(c.get("type").and_then(Value::as_str).unwrap_or("")))
            .filter_map(|c| c.get("name").and_then(Value::as_str))
            .collect();
        let measures: Vec<&str> = cols
            .iter()
            .filter(|c| is_numeric_type(c.get("type").and_then(Value::as_str).unwrap_or("")))
            .filter_map(|c| c.get("name").and_then(Value::as_str))
            .collect();
        let rows_n: i64 = m
            .get("total_rows")
            .and_then(Value::as_str)
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        out.push(
            json!({ "mart": name, "rows": rows_n, "dimensions": dimensions, "measures": measures }),
        );
    }
    json!({ "marts": out })
}

pub(super) async fn list_charts(ch: &ChClient, principal: Option<&Principal>) -> Value {
    match store::list_stored_charts(ch, &viewer_for(principal)).await {
        Ok(charts) => {
            let out: Vec<Value> = charts
                .iter()
                .map(|c| {
                    json!({
                        "id": c.spec.id, "title": c.spec.title, "kind": c.spec.kind,
                        "mart": c.spec.mart, "sqlSource": c.spec.sql_source,
                        "source": c.source,
                    })
                })
                .collect();
            json!({ "total": out.len(), "charts": out })
        }
        Err(err) => upstream_error::report_ch(&upstream_error::DATABASE, &err).to_json(),
    }
}

/// The two read tools that say what a chart can be built on beyond one mart:
/// `list_sql_sources` and `suggest_join` (one dispatch arm, `run_tool` is at
/// clippy's line limit).
pub(super) async fn chart_sources(ch: &ChClient, name: &str, args: &Map<String, Value>) -> Value {
    if name == "suggest_join" {
        suggest_join(ch, args).await
    } else {
        list_sql_sources(ch).await
    }
}

/// `suggest_join` (`BI-7`): the route's answer for two marts. A refusal that is
/// ours (unknown mart, bad name) is told in its words; a database failure is
/// the fixed message the route would give, never the database's text.
pub(super) async fn suggest_join(ch: &ChClient, args: &Map<String, Value>) -> Value {
    let (left, right) = (arg_str(args, "left"), arg_str(args, "right"));
    match crate::routes::dashboard::join_suggestions_value(ch, Some(&left), Some(&right)).await {
        Ok(body) => body,
        Err(
            lakehouse_core::ApiError::BadRequest(sentence)
            | lakehouse_core::ApiError::NotFound(sentence),
        ) => json!({ "error": sentence }),
        Err(_) => json!({ "error": "the tables' columns could not be read right now" }),
    }
}

/// `list_sql_sources`: every dashboard SQL source with its columns split
/// into dimensions and measures, the same split `describe_mart` uses. A
/// `ClickHouse` failure is a fixed message, never its text.
pub(super) async fn list_sql_sources(ch: &ChClient) -> Value {
    match lakehouse_bi::sources::list_sources(ch).await {
        Ok(sources) => {
            let out: Vec<Value> = sources
                .iter()
                .map(|s| {
                    let (measures, dimensions): (Vec<_>, Vec<_>) = s
                        .columns
                        .iter()
                        .partition(|c| crate::routes::support::is_numeric_type(&c.ty));
                    // BI-9: each column's kind, so the assistant can tell which
                    // dimensions take a `grain` (date, datetime).
                    let columns: Vec<Value> = s
                        .columns
                        .iter()
                        .map(|c| {
                            json!({
                                "name": c.name,
                                "kind": lakehouse_bi::filters::ColumnKind::from_clickhouse_type(&c.ty),
                            })
                        })
                        .collect();
                    json!({
                        "id": s.id, "title": s.title,
                        "dimensions": dimensions.iter().map(|c| &c.name).collect::<Vec<_>>(),
                        "measures": measures.iter().map(|c| &c.name).collect::<Vec<_>>(),
                        "columns": columns,
                    })
                })
                .collect();
            json!({ "total": out.len(), "sources": out })
        }
        Err(err) => {
            tracing::warn!(%err, "list_sql_sources failed");
            json!({ "error": "SQL sources tidak bisa dibaca saat ini." })
        }
    }
}

pub(super) async fn delete_chart(
    ch: &ChClient,
    principal: Option<&Principal>,
    args: &Map<String, Value>,
) -> Value {
    let id = arg_str(args, "id");
    if id.is_empty() {
        return json!({ "error": "id is required" });
    }
    // BI-18 part C, part 2: a chart the person cannot see is answered like one
    // that does not exist, and nothing is deleted. A deleted chart goes to the
    // trash, restorable for 30 days.
    let board = match visible_live_chart_board(ch, &id, &viewer_for(principal)).await {
        Ok(Some(board)) => board,
        Ok(None) => return json!({ "deleted": true, "id": id }),
        Err(err) => return refused(err),
    };
    let by = principal.map_or("Assistant", |p| p.display_name.as_str());
    match store::delete_chart(ch, &id, by).await {
        Ok(()) => {
            drop_tile(ch, &board, &id).await;
            snapshot_as(ch, &board, principal).await;
            json!({ "deleted": true, "id": id })
        }
        Err(err) => upstream_error::report_ch(&upstream_error::DATABASE, &err).to_json(),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    /// SEC-11: the assistant's dashboard tools never relay `ClickHouse`'s
    /// text; they return a fixed message and a reference id.
    #[tokio::test]
    async fn a_failing_dashboard_tool_reports_a_fixed_message_and_a_reference() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(500).set_body_string(
                "Code: 60. DB::Exception: Table planted-marker-table-x doesn't exist (version 0.0.0)",
            ))
            .mount(&server)
            .await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());

        for result in [
            list_boards(&ch, None).await,
            list_charts(&ch, None).await,
            suggest_dashboard(&ch).await,
            delete_chart(&ch, None, &{
                let mut a = Map::new();
                a.insert("id".to_owned(), json!("c1"));
                a
            })
            .await,
        ] {
            let text = result.to_string();
            assert!(!text.contains("planted-marker"), "leaked: {text}");
            assert!(!text.contains("version 0.0.0"), "{text}");
            assert!(result["errorId"].is_string(), "{text}");
        }
    }

    // ── BI-18 part C, part 2: the personal space and the trash ──────────────

    use lakehouse_auth::{PermissionSet, PrincipalId};
    use wiremock::matchers::body_string_contains;

    fn person(n: u128) -> Principal {
        Principal {
            id: PrincipalId::User(uuid::Uuid::from_u128(n)),
            tenant_ids: Vec::new(),
            display_name: "Rina".to_owned(),
            permissions: PermissionSet::parse("dashboard:read, dashboard:write"),
            provider: "session".to_owned(),
            must_change_password: false,
            role_names: Vec::new(),
        }
    }

    async fn rows(server: &MockServer, needle: &str, data: Vec<Value>) {
        Mock::given(method("POST"))
            .and(body_string_contains(needle))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({ "meta": [], "data": data, "rows": data.len() })),
            )
            .mount(server)
            .await;
    }

    async fn rest(server: &MockServer) {
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({ "meta": [], "data": [], "rows": 0 })),
            )
            .mount(server)
            .await;
    }

    async fn sent(server: &MockServer) -> Vec<String> {
        server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .map(|r| String::from_utf8_lossy(&r.body).into_owned())
            .collect()
    }

    fn writes(sent: &[String]) -> Vec<&String> {
        sent.iter()
            .filter(|s| s.trim_start().starts_with("INSERT"))
            .collect()
    }

    /// The assistant reads as the person it acts for; with nobody attached it
    /// reads as nobody in particular, which sees no personal space.
    #[tokio::test]
    async fn the_assistant_lists_boards_and_charts_as_the_person_it_acts_for() {
        let server = MockServer::start().await;
        rest(&server).await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let me = person(7);
        let id = uuid::Uuid::from_u128(7).to_string();
        let _ = list_boards(&ch, Some(&me)).await;
        let _ = list_charts(&ch, Some(&me)).await;
        let _ = list_boards(&ch, None).await;
        let all = sent(&server).await;
        let boards: Vec<&String> = all
            .iter()
            .filter(|s| s.contains("FROM console.bi_board FINAL WHERE is_deleted = 0"))
            .collect();
        assert!(
            boards[0].contains(&format!("(owner_id = '' OR owner_id = '{id}')")),
            "{}",
            boards[0]
        );
        assert!(
            boards[1].contains("(owner_id = '' OR owner_id = '')"),
            "{}",
            boards[1]
        );
        let charts: Vec<&String> = all
            .iter()
            .filter(|s| s.contains("FROM console.bi_chart FINAL"))
            .collect();
        assert!(
            charts[0].contains(&format!("owner_id != '' AND owner_id != '{id}'")),
            "{}",
            charts[0]
        );
        assert!(charts[0].contains("is_deleted = 2"), "{}", charts[0]);
    }

    /// A chart the person cannot see is answered like one that does not exist,
    /// and nothing is deleted.
    #[tokio::test]
    async fn the_assistant_cannot_delete_a_chart_on_somebody_elses_private_board() {
        let server = MockServer::start().await;
        rows(
            &server,
            "SELECT is_deleted, board FROM console.bi_chart",
            vec![json!({ "is_deleted": 0, "board": "b_draft" })],
        )
        .await;
        rows(
            &server,
            "SELECT is_deleted, owner_id FROM console.bi_board",
            vec![json!({ "is_deleted": 0, "owner_id": "someone-else" })],
        )
        .await;
        rest(&server).await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let mut args = Map::new();
        args.insert("id".to_owned(), json!("c_private"));
        let _ = delete_chart(&ch, Some(&person(7)), &args).await;
        let all = sent(&server).await;
        assert!(writes(&all).is_empty(), "{:?}", writes(&all));
    }

    #[tokio::test]
    async fn the_assistant_moves_a_chart_to_the_trash_under_the_persons_name() {
        let server = MockServer::start().await;
        rows(
            &server,
            "SELECT is_deleted, board FROM console.bi_chart",
            vec![json!({ "is_deleted": 0, "board": "default" })],
        )
        .await;
        rest(&server).await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let mut args = Map::new();
        args.insert("id".to_owned(), json!("c_mine"));
        let reply = delete_chart(&ch, Some(&person(7)), &args).await;
        assert_eq!(reply["deleted"], json!(true));
        let all = sent(&server).await;
        let copy = writes(&all)
            .into_iter()
            .find(|s| s.contains("INTO console.bi_chart"))
            .unwrap();
        assert!(copy.contains(", 2, now(), 'Rina', ''"), "{copy}");
    }

    #[tokio::test]
    async fn the_assistant_cannot_put_a_chart_on_a_dashboard_in_the_trash() {
        let server = MockServer::start().await;
        rows(
            &server,
            "SELECT is_deleted, owner_id FROM console.bi_board",
            vec![json!({ "is_deleted": 2, "owner_id": "" })],
        )
        .await;
        rest(&server).await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let mut args = Map::new();
        for (k, v) in [
            ("title", json!("t")),
            ("kind", json!("bar")),
            ("mart", json!("m")),
            ("dimension", json!("d")),
            ("measures", json!(["x"])),
            ("board", json!("b_gone")),
        ] {
            args.insert(k.to_owned(), v);
        }
        let reply = create_chart(
            &ch,
            Some(&person(7)),
            &args,
            None,
            &lakehouse_bi::grain::TimeContext::default(),
        )
        .await;
        assert_eq!(reply["error"], json!("dashboard not found."));
        assert!(writes(&sent(&server).await).is_empty());
    }

    // ── BI-16 part B (AI-5): progress, histogram, bullet, image and web ─────

    /// A mart with a text and three number columns, and a console that takes
    /// every write.
    async fn mart_server() -> MockServer {
        let server = MockServer::start().await;
        rows(&server, "FROM system.tables", vec![json!({ "n": "1" })]).await;
        rows(
            &server,
            "FROM system.columns",
            vec![
                json!({ "name": "province", "type": "String" }),
                json!({ "name": "visitors", "type": "UInt64" }),
                json!({ "name": "target", "type": "UInt64" }),
                json!({ "name": "price", "type": "Float64" }),
            ],
        )
        .await;
        rest(&server).await;
        server
    }

    fn arguments(v: Value) -> Map<String, Value> {
        match v {
            Value::Object(map) => map,
            other => panic!("not an object: {other}"),
        }
    }

    fn sites(list: &[&str]) -> lakehouse_bi::grain::TimeContext {
        lakehouse_bi::grain::TimeContext::default()
            .with_content_sites(list.iter().map(|s| (*s).to_owned()).collect())
    }

    /// The statement the tool ran to check the chart it saved.
    fn statements(all: &[String]) -> Vec<&String> {
        all.iter()
            .filter(|s| s.starts_with("SELECT") && s.contains("serving.mart_x"))
            .collect()
    }

    /// AI-5: "Show a progress bar of visitors against a goal of 1,000,000".
    #[tokio::test]
    async fn the_request_for_a_progress_bar_against_a_goal_of_a_million_creates_one() {
        let server = mart_server().await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let reply = create_chart(
            &ch,
            None,
            &arguments(json!({
                "title": "Visitors against 1,000,000", "kind": "progress", "mart": "mart_x",
                "measures": ["visitors"], "aggregate": "sum", "goal": 1_000_000,
            })),
            None,
            &lakehouse_bi::grain::TimeContext::default(),
        )
        .await;
        assert_eq!(reply["created"], json!(true), "{reply}");
        assert_eq!(reply["kind"], json!("progress"));
        let all = sent(&server).await;
        assert_eq!(
            statements(&all)
                .first()
                .map(|s| s.trim_end_matches("\nFORMAT JSON")),
            Some("SELECT round(sum(visitors)) AS v FROM serving.mart_x")
        );
        let stored = writes(&all);
        assert!(
            stored.iter().any(|w| w.contains("goal")),
            "the goal is stored"
        );
        // A goal that is a second value, and a goal that is missing.
        let valued = create_chart(
            &ch,
            None,
            &arguments(json!({
                "title": "Against target", "kind": "progress", "mart": "mart_x",
                "measures": ["visitors", "target"], "goal": { "measure": "target" },
            })),
            None,
            &lakehouse_bi::grain::TimeContext::default(),
        )
        .await;
        assert_eq!(valued["created"], json!(true), "{valued}");
        let none = create_chart(
            &ch,
            None,
            &arguments(json!({
                "title": "No goal", "kind": "progress", "mart": "mart_x", "measures": ["visitors"],
            })),
            None,
            &lakehouse_bi::grain::TimeContext::default(),
        )
        .await;
        assert!(
            none["error"].as_str().unwrap().contains("needs a goal"),
            "{none}"
        );
    }

    /// AI-5: "Show how hotel prices are spread" is a histogram of the price
    /// column; the same call with `histogram: {count: 10}` cuts it in ten.
    #[tokio::test]
    async fn the_request_to_show_how_prices_are_spread_creates_a_histogram() {
        let server = mart_server().await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let time = lakehouse_bi::grain::TimeContext::default();
        let auto = create_chart(
            &ch,
            None,
            &arguments(json!({
                "title": "How hotel prices are spread", "kind": "histogram", "mart": "mart_x",
                "dimension": "price",
            })),
            None,
            &time,
        )
        .await;
        assert_eq!(auto["created"], json!(true), "{auto}");
        let ten = create_chart(
            &ch,
            None,
            &arguments(json!({
                "title": "In ten ranges", "kind": "histogram", "mart": "mart_x",
                "dimension": "price", "histogram": { "count": 10 },
            })),
            None,
            &time,
        )
        .await;
        assert_eq!(ten["created"], json!(true), "{ten}");
        let all = sent(&server).await;
        let ran = statements(&all);
        // BI-18 part C review fix (SHOULD-FIX) R21: automatic aims at the nice width
        // nearest 20 ranges, and a count of 10 is ten ranges, not "at most 10".
        assert!(ran[0].contains("log10((mx - mn) / 20)"), "{}", ran[0]);
        assert!(ran[1].contains("(mx - mn) / 10 AS raw"), "{}", ran[1]);
        assert!(ran.iter().all(|s| s.contains("count() AS __count")));
        // The same refusals as the console, in the same words.
        for (extra, sentence) in [
            (json!({ "histogram": { "count": 101 } }), "1 to 100 ranges"),
            (
                json!({ "histogram": { "count": 5, "width": 2 } }),
                "not both",
            ),
            (json!({ "dimension": "province" }), "not a number column"),
        ] {
            let mut input = json!({
                "title": "t", "kind": "histogram", "mart": "mart_x", "dimension": "price",
            });
            input
                .as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            let reply = create_chart(&ch, None, &arguments(input), None, &time).await;
            assert!(
                reply["error"].as_str().unwrap().contains(sentence),
                "{reply}"
            );
        }
    }

    #[tokio::test]
    async fn a_bullet_graph_per_province_with_bands_is_created_and_a_goal_on_a_bar_chart_is_refused()
     {
        let server = mart_server().await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let time = lakehouse_bi::grain::TimeContext::default();
        let bullet = create_chart(
            &ch,
            None,
            &arguments(json!({
                "title": "Visitors against target", "kind": "bullet", "mart": "mart_x",
                "dimension": "province", "measures": ["visitors", "target"],
                "goal": { "measure": "target" }, "bands": [50, 80, 100],
            })),
            None,
            &time,
        )
        .await;
        assert_eq!(bullet["created"], json!(true), "{bullet}");
        let bar = create_chart(
            &ch,
            None,
            &arguments(json!({
                "title": "t", "kind": "bar", "mart": "mart_x", "dimension": "province",
                "measures": ["visitors"], "goal": 5,
            })),
            None,
            &time,
        )
        .await;
        assert_eq!(
            bar["error"],
            json!("goal is only for a progress bar and a bullet graph.")
        );
    }

    /// The address of an image or a web page is checked against the
    /// administrator's list on the assistant's path as on the console's, with
    /// the console's sentence, and the address is not repeated.
    #[tokio::test]
    async fn an_image_on_a_site_that_is_not_allowed_is_refused_with_the_consoles_sentence() {
        let server = mart_server().await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let image = |url: &str| {
            arguments(json!({ "title": "Logo", "kind": "image", "url": url, "alt": "The logo" }))
        };
        let allowed = sites(&["https://img.example"]);
        let created = create_chart(
            &ch,
            None,
            &image("https://img.example/logo.png"),
            None,
            &allowed,
        )
        .await;
        assert_eq!(created["created"], json!(true), "{created}");
        for url in [
            "https://other.example/logo.png",
            "http://img.example/logo.png",
            "https://img.example@evil.example/logo.png",
        ] {
            let refused = create_chart(&ch, None, &image(url), None, &allowed).await;
            assert_eq!(
                refused["error"],
                json!(lakehouse_bi::types_b::CONTENT_SITE_REFUSED),
                "{url}"
            );
            assert!(!refused.to_string().contains(url));
        }
        // With no site listed nothing can be created; update_chart is checked too.
        let none = create_chart(
            &ch,
            None,
            &image("https://img.example/logo.png"),
            None,
            &sites(&[]),
        )
        .await;
        assert_eq!(
            none["error"],
            json!(lakehouse_bi::types_b::CONTENT_SITE_REFUSED)
        );
        rows(
            &server,
            "FROM console.bi_chart",
            vec![json!({ "board": "default", "owner_id": "", "is_deleted": 0 })],
        )
        .await;
        let mut update = image("https://other.example/logo.png");
        update.insert("id".to_owned(), json!("c_1"));
        let refused = update_chart(&ch, None, &update, &allowed).await;
        assert_eq!(
            refused["error"],
            json!(lakehouse_bi::types_b::CONTENT_SITE_REFUSED)
        );
        let web = create_chart(
            &ch,
            None,
            &arguments(
                json!({ "title": "Status", "kind": "web", "url": "https://img.example/status" }),
            ),
            None,
            &allowed,
        )
        .await;
        assert_eq!(web["created"], json!(true), "{web}");
    }
}
