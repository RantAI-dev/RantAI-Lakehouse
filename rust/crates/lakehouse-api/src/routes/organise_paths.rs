//! `BI-18` part C, part 2: what every read and write path of the dashboards
//! does about the trash and the personal space.
//!
//! A wiremock `ClickHouse` answers the store's reads from rules that look at
//! the statement it was sent, so a path gets a hidden row back only if the
//! statement it sent did NOT restrict the read: the assertions are about what
//! each path asked for and what it then showed. The real engine is checked
//! separately by the ignored tests at the end of this file (a scratch
//! database; see `mod engine`), which is where the clauses are proven to mean
//! what these tests assume.

#![cfg(test)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashMap;

use axum::Extension;
use axum::extract::{Query, State};
use lakehouse_auth::{PermissionSet, Principal, PrincipalId};
use lakehouse_bi::access::Viewer;
use lakehouse_core::ApiError;
use serde_json::{Value, json};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

use super::super::dashboard_organise::trash_restore;
use super::typed_filters::{chart_row, client, statements};
use super::{
    DashboardQuery, GrainRequest, RecordsQuery, TableExportQuery, ValuesQuery, boards_create,
    boards_delete, boards_list, boards_update, export, get_body, records_for_roles, specs_create,
    specs_delete, specs_list, specs_update, table_export, values_for_roles,
};
use crate::config::Config;
use crate::policy_engine::PolicyEngineObligations;
use crate::sql_rewrite::PlaceholderValues;
use crate::state::AppState;

const ME: u128 = 0xA1;
const OTHER: u128 = 0xB2;

fn id_of(n: u128) -> String {
    uuid::Uuid::from_u128(n).to_string()
}

fn person(n: u128, permissions: &str) -> Principal {
    Principal {
        id: PrincipalId::User(uuid::Uuid::from_u128(n)),
        tenant_ids: Vec::new(),
        display_name: format!("Person {n:x}"),
        permissions: PermissionSet::parse(permissions),
        provider: "session".to_owned(),
        must_change_password: false,
        role_names: Vec::new(),
    }
}

/// `Result::expect` for a result whose error type has no `Debug` (a route's
/// rejection): the test says what it expected and nothing is printed of the
/// error.
trait Done<T> {
    fn done(self, what: &str) -> T;
}

impl<T, E> Done<T> for Result<T, E> {
    fn done(self, what: &str) -> T {
        let Ok(value) = self else {
            panic!("expected success: {what}");
        };
        value
    }
}

fn state_for(server: &MockServer) -> AppState {
    // No Postgres: a database URL that does not parse leaves `pg` unset, so the
    // reporting settings read as their defaults.
    let env = HashMap::from([
        ("CH_URL".to_owned(), server.uri()),
        (
            "DATABASE_URL".to_owned(),
            "not a connection string".to_owned(),
        ),
    ]);
    AppState::new(Config::from_map(&env).expect("a valid test config"))
}

/// One rule of the fake engine: when the statement holds every needle, answer
/// with these rows.
struct Rule {
    needles: Vec<String>,
    rows: Vec<Value>,
}

/// A `ClickHouse` that answers by looking at the statement: the first rule
/// whose needles are all in it wins, anything else gets no rows. This is how a
/// test makes a hidden row come back only to a statement that did not exclude
/// it.
struct Engine {
    rules: Vec<Rule>,
}

impl Respond for Engine {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let sql = String::from_utf8_lossy(&request.body);
        let rows = self
            .rules
            .iter()
            .find(|r| r.needles.iter().all(|n| sql.contains(n.as_str())))
            .map_or_else(Vec::new, |r| r.rows.clone());
        ResponseTemplate::new(200)
            .set_body_json(json!({ "meta": [], "data": rows, "rows": rows.len() }))
    }
}

async fn engine(rules: Vec<(&[&str], Vec<Value>)>) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(wiremock::matchers::method("POST"))
        .respond_with(Engine {
            rules: rules
                .into_iter()
                .map(|(needles, rows)| Rule {
                    needles: needles.iter().map(|n| (*n).to_owned()).collect(),
                    rows,
                })
                .collect(),
        })
        .mount(&server)
        .await;
    server
}

/// The statements sent so far that hold `needle`.
async fn sent(server: &MockServer, needle: &str) -> Vec<String> {
    statements(server)
        .await
        .into_iter()
        .filter(|s| s.contains(needle))
        .collect()
}

async fn writes(server: &MockServer) -> Vec<String> {
    statements(server)
        .await
        .into_iter()
        .filter(|s| s.trim_start().starts_with("INSERT"))
        .collect()
}

/// The clause that keeps another user's personal board out of a chart read.
fn private_chart_clause(viewer: u128) -> String {
    format!(
        "board NOT IN (SELECT id FROM console.bi_board FINAL WHERE owner_id != '' AND owner_id != '{}')",
        id_of(viewer)
    )
}

const TRASHED_BOARDS: &str =
    "board NOT IN (SELECT id FROM console.bi_board FINAL WHERE is_deleted = 2)";

fn owner_clause(viewer: u128) -> String {
    format!("(owner_id = '' OR owner_id = '{}')", id_of(viewer))
}

fn board_row(id: &str, owner: &str) -> Value {
    json!({
        "id": id, "name": format!("Board {id}"), "description": "", "created_by": "t",
        "layout_json": "{}", "filters_json": "[]", "public_token": "", "embed_enabled": "0",
        "folder_id": "", "owner_id": owner, "created_at": "2026-10-14 00:00:00",
    })
}

fn visible_and_private_charts() -> (Vec<Value>, Vec<Value>) {
    let public = chart_row("c_public", "b_public", "mart_a", None);
    let private = chart_row("c_private", "b_draft", "mart_a", None);
    (vec![public.clone()], vec![public, private])
}

// ── GET /api/dashboard ──────────────────────────────────────────────────

async fn payload(server: &MockServer, viewer: &Viewer, board: &str) -> Value {
    let ch = client(server);
    let obligations = PolicyEngineObligations::new(None, &ch);
    let q = DashboardQuery {
        board: Some(board.to_owned()),
        year: None,
        filters: None,
        grain: None,
        params: None,
    };
    get_body(
        &ch,
        &q,
        None,
        &[],
        &PlaceholderValues::none(),
        &obligations,
        (
            &lakehouse_bi::grain::TimeContext::default(),
            GrainRequest::Saved,
            viewer,
        ),
    )
    .await
    .expect("the payload builds")
}

/// A dashboard read for `board = "all"` names every chart the caller may see:
/// another user's private chart is in the answer only to a statement that did
/// not exclude it.
#[tokio::test]
async fn the_dashboard_payload_leaves_out_another_users_private_chart() {
    let (visible, everything) = visible_and_private_charts();
    let me = private_chart_clause(ME);
    let server = engine(vec![
        (&["FROM console.bi_chart FINAL", me.as_str()], visible),
        (&["FROM console.bi_chart FINAL"], everything),
    ])
    .await;

    let mine = payload(&server, &Viewer::user(id_of(ME), false), "all").await;
    let ids: Vec<&String> = mine["results"].as_object().unwrap().keys().collect();
    assert!(ids.contains(&&"c_public".to_owned()), "{ids:?}");
    assert!(!ids.contains(&&"c_private".to_owned()), "{ids:?}");

    // The statements asked for live rows, and for the viewer's scope.
    let chart_reads = sent(&server, "FROM console.bi_chart FINAL").await;
    assert!(!chart_reads.is_empty());
    for read in &chart_reads {
        assert!(read.contains("is_deleted = 0"), "{read}");
        assert!(read.contains(&me), "{read}");
        assert!(read.contains(TRASHED_BOARDS), "{read}");
    }
    let board_reads = sent(&server, "FROM console.bi_board FINAL WHERE is_deleted = 0").await;
    assert!(!board_reads.is_empty());
    for read in board_reads {
        assert!(read.contains(&owner_clause(ME)), "{read}");
    }
}

/// The same read by the server itself or by an administrator (who sees every
/// personal space) is not narrowed, so the answer differs: the fake engine
/// proves the difference comes from the statement alone.
#[tokio::test]
async fn an_administrator_sees_the_private_chart_the_owner_does() {
    let (visible, everything) = visible_and_private_charts();
    let other = private_chart_clause(OTHER);
    let server = engine(vec![
        (&["FROM console.bi_chart FINAL", other.as_str()], visible),
        (&["FROM console.bi_chart FINAL"], everything),
    ])
    .await;
    let admin = payload(&server, &Viewer::user(id_of(OTHER), true), "all").await;
    let ids: Vec<&String> = admin["results"].as_object().unwrap().keys().collect();
    assert!(ids.contains(&&"c_private".to_owned()), "{ids:?}");
    let plain = payload(&server, &Viewer::user(id_of(OTHER), false), "all").await;
    let ids: Vec<&String> = plain["results"].as_object().unwrap().keys().collect();
    assert!(!ids.contains(&&"c_private".to_owned()), "{ids:?}");
}

/// A request for a private board by id reads as a board that has no charts and
/// no layout: the same payload as an id that never existed.
#[tokio::test]
async fn a_private_board_asked_for_by_id_reads_like_one_that_does_not_exist() {
    let (visible, everything) = visible_and_private_charts();
    let me = private_chart_clause(ME);
    let server = engine(vec![
        (&["FROM console.bi_chart FINAL", me.as_str()], visible),
        (&["FROM console.bi_chart FINAL"], everything),
        // The board row is returned to a statement with no owner restriction.
        (&["FROM console.bi_board FINAL", "owner_id = ''"], vec![]),
        (
            &["FROM console.bi_board FINAL"],
            vec![board_row("b_draft", &id_of(OTHER))],
        ),
    ])
    .await;
    let body = payload(&server, &Viewer::user(id_of(ME), false), "b_draft").await;
    assert_eq!(body["results"].as_object().unwrap().len(), 0, "{body}");
}

// ── Lists ───────────────────────────────────────────────────────────────

async fn text_of(response: axum::response::Response) -> Value {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn the_board_list_and_the_chart_list_are_scoped_to_the_caller() {
    let (visible, everything) = visible_and_private_charts();
    let me = private_chart_clause(ME);
    let mine = owner_clause(ME);
    let server = engine(vec![
        (&["FROM console.bi_chart FINAL", me.as_str()], visible),
        (&["FROM console.bi_chart FINAL"], everything),
        (
            &["FROM console.bi_board FINAL", mine.as_str()],
            vec![board_row("b_public", "")],
        ),
        (
            &["FROM console.bi_board FINAL"],
            vec![
                board_row("b_public", ""),
                board_row("b_draft", &id_of(OTHER)),
            ],
        ),
    ])
    .await;
    let state = state_for(&server);
    let who = person(ME, "dashboard:read");

    let boards = text_of(boards_list(State(state.clone()), Extension(who.clone())).await).await;
    let listed: Vec<&str> = boards["boards"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["id"].as_str().unwrap())
        .collect();
    assert!(listed.contains(&"b_public"), "{listed:?}");
    assert!(!listed.contains(&"b_draft"), "{listed:?}");

    let charts = text_of(specs_list(State(state), Extension(who)).await).await;
    let ids: Vec<&str> = charts["charts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, vec!["c_public"]);
}

// ── Records, CSV export, filter values, YAML export ────────────────────

fn roles_and_viewer(viewer: &Viewer) -> (Vec<String>, Viewer) {
    (Vec::new(), viewer.clone())
}

#[tokio::test]
async fn a_records_request_for_a_hidden_tile_is_not_found() {
    let (visible, everything) = visible_and_private_charts();
    let me = private_chart_clause(ME);
    let server = engine(vec![
        (&["FROM console.bi_chart FINAL", me.as_str()], visible),
        (&["FROM console.bi_chart FINAL"], everything),
    ])
    .await;
    let ch = client(&server);
    let obligations = PolicyEngineObligations::new(None, &ch);
    let (roles, viewer) = roles_and_viewer(&Viewer::user(id_of(ME), false));
    let query = |chart: &str| RecordsQuery {
        mart: Some("mart_a".to_owned()),
        chart: Some(chart.to_owned()),
        ..RecordsQuery::default()
    };
    let refusal = records_for_roles(
        &ch,
        query("c_private"),
        (&roles, &viewer),
        &PlaceholderValues::none(),
        &obligations,
        &lakehouse_bi::grain::TimeContext::default(),
    )
    .await
    .unwrap_err();
    assert!(
        matches!(&refusal, ApiError::NotFound(m) if m == "this tile does not exist"),
        "{refusal:?}"
    );
    // Nothing of the hidden tile's data was read: only store reads were sent.
    assert!(sent(&server, "FROM serving.mart_a").await.is_empty());
}

#[tokio::test]
async fn the_csv_export_of_a_hidden_tile_is_not_found() {
    let (visible, everything) = visible_and_private_charts();
    let me = private_chart_clause(ME);
    let server = engine(vec![
        (&["FROM console.bi_chart FINAL", me.as_str()], visible),
        (&["FROM console.bi_chart FINAL"], everything),
    ])
    .await;
    let query: TableExportQuery = serde_json::from_value(json!({ "chart": "c_private" })).unwrap();
    let outcome = table_export(
        State(state_for(&server)),
        Extension(person(ME, "dashboard:read")),
        Query(query),
    )
    .await;
    let Err(rejection) = outcome else {
        panic!("a hidden tile must not export");
    };
    let response = axum::response::IntoResponse::into_response(rejection);
    assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);
    assert!(sent(&server, "FROM serving.").await.is_empty());
}

#[tokio::test]
async fn the_filter_values_of_a_board_come_only_from_charts_the_caller_may_see() {
    let (visible, everything) = visible_and_private_charts();
    let me = private_chart_clause(ME);
    let server = engine(vec![
        (&["FROM console.bi_chart FINAL", me.as_str()], visible),
        (&["FROM console.bi_chart FINAL"], everything),
    ])
    .await;
    let ch = client(&server);
    let obligations = PolicyEngineObligations::new(None, &ch);
    let (roles, viewer) = roles_and_viewer(&Viewer::user(id_of(ME), false));
    let query: ValuesQuery =
        serde_json::from_value(json!({ "column": "kab", "board": "all" })).unwrap();
    let _ = values_for_roles(
        &ch,
        query,
        (&roles, &viewer),
        &PlaceholderValues::none(),
        &obligations,
        &lakehouse_bi::grain::TimeContext::default(),
    )
    .await;
    for read in sent(&server, "FROM console.bi_chart FINAL").await {
        assert!(read.contains(&me), "{read}");
        assert!(read.contains("is_deleted = 0"), "{read}");
    }
}

#[tokio::test]
async fn the_yaml_export_holds_only_what_the_caller_may_see() {
    let (visible, everything) = visible_and_private_charts();
    let me = private_chart_clause(ME);
    let mine = owner_clause(ME);
    let server = engine(vec![
        (&["FROM console.bi_chart FINAL", me.as_str()], visible),
        (&["FROM console.bi_chart FINAL"], everything),
        (
            &["FROM console.bi_board FINAL", mine.as_str()],
            vec![board_row("b_public", "")],
        ),
        (
            &["FROM console.bi_board FINAL"],
            vec![
                board_row("b_public", ""),
                board_row("b_draft", &id_of(OTHER)),
            ],
        ),
    ])
    .await;
    let response = export(
        State(state_for(&server)),
        Extension(person(ME, "dashboard:read")),
    )
    .await
    .done("the export builds");
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let yaml = String::from_utf8_lossy(&bytes);
    assert!(yaml.contains("c_public"), "{yaml}");
    assert!(!yaml.contains("c_private"), "{yaml}");
    assert!(!yaml.contains("b_draft"), "{yaml}");
}

// ── Writes ──────────────────────────────────────────────────────────────

/// A board row for the state lookup every write makes first.
fn state_row(is_deleted: u8, owner: &str) -> Value {
    json!({ "is_deleted": is_deleted, "owner_id": owner })
}

#[tokio::test]
async fn a_board_in_the_trash_cannot_be_edited_and_nothing_is_written() {
    let server = engine(vec![(
        &["SELECT is_deleted, owner_id FROM console.bi_board"],
        vec![state_row(2, "")],
    )])
    .await;
    let body = json!({ "id": "b_gone", "layout": {} }).to_string();
    let outcome = boards_update(
        State(state_for(&server)),
        Extension(person(ME, "dashboard:write")),
        body.into(),
    )
    .await;
    let response = axum::response::IntoResponse::into_response(match outcome {
        Err(rejection) => rejection,
        Ok(_) => panic!("refused"),
    });
    assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);
    assert!(
        writes(&server).await.is_empty(),
        "{:?}",
        writes(&server).await
    );
}

#[tokio::test]
async fn another_users_private_board_cannot_be_edited_or_deleted() {
    let server = engine(vec![(
        &["SELECT is_deleted, owner_id FROM console.bi_board"],
        vec![state_row(0, &id_of(OTHER))],
    )])
    .await;
    let state = state_for(&server);
    let me = person(ME, "dashboard:write");
    let update = boards_update(
        State(state.clone()),
        Extension(me.clone()),
        json!({ "id": "b_draft", "name": "Mine now" })
            .to_string()
            .into(),
    )
    .await;
    assert!(update.is_err());
    let delete = boards_delete(
        State(state),
        Extension(me),
        Query(serde_json::from_value(json!({ "id": "b_draft" })).unwrap()),
    )
    .await;
    assert!(delete.is_err());
    assert!(
        writes(&server).await.is_empty(),
        "{:?}",
        writes(&server).await
    );
}

#[tokio::test]
async fn deleting_a_board_copies_its_charts_and_then_the_board_to_the_trash() {
    let server = engine(vec![(
        &["SELECT is_deleted, owner_id FROM console.bi_board"],
        vec![state_row(0, "")],
    )])
    .await;
    boards_delete(
        State(state_for(&server)),
        Extension(person(ME, "dashboard:write")),
        Query(serde_json::from_value(json!({ "id": "b_old" })).unwrap()),
    )
    .await
    .done("deleted");
    let written = writes(&server).await;
    // Charts first, then the board; both keep their content and mark state 2.
    let charts = written
        .iter()
        .position(|s| s.contains("INTO console.bi_chart") && s.contains("SELECT"))
        .expect("the charts are copied");
    let board = written
        .iter()
        .position(|s| s.contains("INTO console.bi_board") && s.contains("SELECT"))
        .expect("the board is copied");
    assert!(charts < board, "{written:?}");
    assert!(written[charts].contains("is_deleted = 0 AND board = 'b_old'"));
    assert!(written[charts].contains(", 2, now(), 'Person a1', 'b_old'"));
    assert!(written[board].contains(", 2, now(), 'Person a1'"));
    // No row was blanked: a delete is no longer a tombstone.
    assert!(written.iter().all(|s| !s.contains("VALUES")), "{written:?}");
}

#[tokio::test]
async fn a_chart_in_the_trash_cannot_be_edited_back_to_life_by_an_edit() {
    let server = engine(vec![(
        &["SELECT is_deleted, board FROM console.bi_chart"],
        vec![json!({ "is_deleted": 2, "board": "b_public" })],
    )])
    .await;
    let outcome = specs_update(
        State(state_for(&server)),
        Extension(person(ME, "dashboard:write")),
        json!({ "id": "c_old", "title": "t", "kind": "bar", "mart": "m", "dimension": "d", "measures": ["x"] })
            .to_string()
            .into(),
    )
    .await;
    assert!(outcome.is_err());
    assert!(writes(&server).await.is_empty());
}

#[tokio::test]
async fn a_chart_cannot_be_put_on_a_deleted_or_private_board() {
    for (is_deleted, owner) in [(2_u8, String::new()), (0, id_of(OTHER))] {
        let server = engine(vec![(
            &["SELECT is_deleted, owner_id FROM console.bi_board"],
            vec![state_row(is_deleted, &owner)],
        )])
        .await;
        let outcome = specs_create(
            State(state_for(&server)),
            Extension(person(ME, "dashboard:write")),
            json!({ "title": "t", "kind": "bar", "mart": "m", "dimension": "d",
                    "measures": ["x"], "board": "b_target" })
            .to_string()
            .into(),
        )
        .await;
        assert!(outcome.is_err(), "{is_deleted} {owner}");
        assert!(writes(&server).await.is_empty());
    }
}

/// `PR #199 review M SHOULD-FIX BI-18 1`: a chart cannot be put on a
/// dashboard that never existed either. It used to be created there, so the
/// status code told a caller that the id is unknown, while a private one was
/// refused. The gate itself is checked, so the refusal cannot come from
/// anything else.
#[tokio::test]
async fn a_dashboard_that_does_not_exist_is_refused_like_a_private_one() {
    let me = Viewer::user(id_of(ME), false);
    // An id that never existed.
    let server = engine(vec![]).await;
    let refused = super::super::dashboard_organise::require_chart_board(
        &client(&server),
        Some("b_never"),
        &me,
    )
    .await;
    assert!(refused.is_err(), "{refused:?}");
    // A dashboard in somebody else's personal space: the same refusal.
    let server = engine(vec![(
        &["SELECT is_deleted, owner_id FROM console.bi_board"],
        vec![state_row(0, &id_of(OTHER))],
    )])
    .await;
    let refused = super::super::dashboard_organise::require_chart_board(
        &client(&server),
        Some("b_never"),
        &me,
    )
    .await;
    assert!(refused.is_err(), "{refused:?}");
}

/// `PR #199 review M SHOULD-FIX BI-18 1`: an edit of a chart that does not
/// exist is refused, like one on a board the caller cannot see, so the status
/// code does not tell the two apart.
#[tokio::test]
async fn an_edit_of_a_chart_that_does_not_exist_is_refused() {
    let me = Viewer::user(id_of(ME), false);
    let server = engine(vec![]).await;
    let refused =
        super::super::dashboard_organise::require_editable_chart(&client(&server), "c_never", &me)
            .await;
    assert!(refused.is_err(), "{refused:?}");
    // A chart on a dashboard the caller cannot see: the same sentence, so the
    // answer does not say that a chart of somebody else exists.
    let server = engine(vec![
        (
            &["SELECT is_deleted, board FROM console.bi_chart"],
            vec![json!({ "is_deleted": 0, "board": "b_draft" })],
        ),
        (
            &["SELECT is_deleted, owner_id FROM console.bi_board"],
            vec![state_row(0, &id_of(OTHER))],
        ),
    ])
    .await;
    let refused = super::super::dashboard_organise::require_editable_chart(
        &client(&server),
        "c_private",
        &me,
    )
    .await;
    let Err(super::super::dashboard_organise::Gate::Missing(sentence)) = refused else {
        panic!("a chart on a private dashboard must be refused with the fixed sentence");
    };
    assert_eq!(sentence, "this tile does not exist");
}

/// `PR #199 review O SHOULD-FIX 2`: a dashboard that was purged for good
/// (state 1) is refused like one in the trash, so a chart is not created on
/// a dashboard that is gone.
#[tokio::test]
async fn a_chart_cannot_be_created_on_a_dashboard_that_was_purged_for_good() {
    let me = Viewer::user(id_of(ME), false);
    let server = engine(vec![(
        &["SELECT is_deleted, owner_id FROM console.bi_board"],
        vec![state_row(1, "")],
    )])
    .await;
    let refused = super::super::dashboard_organise::require_chart_board(
        &client(&server),
        Some("b_purged"),
        &me,
    )
    .await;
    let Err(super::super::dashboard_organise::Gate::Missing(sentence)) = refused else {
        panic!("a purged dashboard must be refused with the fixed sentence");
    };
    assert_eq!(sentence, "dashboard not found.");
}

/// `PR #199 review O SHOULD-FIX 2`: a chart that was purged for good (state
/// 1) is refused like one in the trash, so an edit cannot bring it back to
/// life.
#[tokio::test]
async fn an_edit_of_a_chart_that_was_purged_for_good_is_refused() {
    let me = Viewer::user(id_of(ME), false);
    let server = engine(vec![(
        &["SELECT is_deleted, board FROM console.bi_chart"],
        vec![json!({ "is_deleted": 1, "board": "b_draft" })],
    )])
    .await;
    let refused =
        super::super::dashboard_organise::require_editable_chart(&client(&server), "c_purged", &me)
            .await;
    let Err(super::super::dashboard_organise::Gate::Missing(sentence)) = refused else {
        panic!("a purged chart must be refused with the fixed sentence");
    };
    assert_eq!(sentence, "this tile does not exist");
}

#[tokio::test]
async fn deleting_a_hidden_chart_deletes_nothing_and_says_nothing() {
    let server = engine(vec![
        (
            &["SELECT is_deleted, board FROM console.bi_chart"],
            vec![json!({ "is_deleted": 0, "board": "b_draft" })],
        ),
        (
            &["SELECT is_deleted, owner_id FROM console.bi_board"],
            vec![state_row(0, &id_of(OTHER))],
        ),
    ])
    .await;
    let reply = specs_delete(
        State(state_for(&server)),
        Extension(person(ME, "dashboard:write")),
        Query(serde_json::from_value(json!({ "id": "c_private" })).unwrap()),
    )
    .await
    .done("answered like an unknown id");
    drop(reply);
    assert!(
        writes(&server).await.is_empty(),
        "{:?}",
        writes(&server).await
    );
}

#[tokio::test]
async fn a_visible_chart_goes_to_the_trash_with_the_name_of_who_deleted_it() {
    let server = engine(vec![(
        &["SELECT is_deleted, board FROM console.bi_chart"],
        vec![json!({ "is_deleted": 0, "board": "default" })],
    )])
    .await;
    specs_delete(
        State(state_for(&server)),
        Extension(person(ME, "dashboard:write")),
        Query(serde_json::from_value(json!({ "id": "c_mine" })).unwrap()),
    )
    .await
    .done("deleted");
    let written = writes(&server).await;
    let copy = written
        .iter()
        .find(|s| s.contains("INTO console.bi_chart") && s.contains("SELECT"))
        .expect("the chart is copied");
    assert!(copy.contains("is_deleted = 0 AND id = 'c_mine'"), "{copy}");
    assert!(copy.contains(", 2, now(), 'Person a1', ''"), "{copy}");
}

// ── Embeds and public links, the trash sweep, history, folders ─────────

#[tokio::test]
async fn embeds_and_public_links_read_as_nobody() {
    let server = engine(vec![]).await;
    let ch = client(&server);
    let nobody = Viewer::anonymous();
    let _ = lakehouse_bi::store::get_board_by_token(&ch, "p_token").await;
    let _ = lakehouse_bi::store::get_board(&ch, "b1", &nobody).await;
    let _ = lakehouse_bi::store::list_stored_charts(&ch, &nobody).await;
    let reads = sent(&server, "FROM console.bi_board FINAL WHERE is_deleted = 0").await;
    assert_eq!(reads.len(), 2, "{reads:?}");
    for read in reads {
        // Nobody in particular: only a board with no owner.
        assert!(read.contains("(owner_id = '' OR owner_id = '')"), "{read}");
    }
    let chart_read = sent(&server, "FROM console.bi_chart FINAL").await;
    assert!(
        chart_read[0].contains("owner_id != '' AND owner_id != ''"),
        "{}",
        chart_read[0]
    );
    assert!(chart_read[0].contains(TRASHED_BOARDS), "{}", chart_read[0]);
}

/// A statement that fails when it holds `needle`; every other one succeeds
/// with no rows.
struct FailingOn(&'static str);

impl Respond for FailingOn {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        if String::from_utf8_lossy(&request.body).contains(self.0) {
            return ResponseTemplate::new(500).set_body_string("Code: 999. planted-marker");
        }
        ResponseTemplate::new(200).set_body_json(json!({ "meta": [], "data": [], "rows": 0 }))
    }
}

/// The sweep reads what is due before it writes anything: if the read fails it
/// returns the failure and has deleted nothing.
#[tokio::test]
async fn a_sweep_that_cannot_read_deletes_nothing() {
    for needle in [
        "FROM console.bi_board FINAL \
         WHERE is_deleted = 2 AND deleted_at >",
        "FROM console.bi_chart FINAL \
         WHERE is_deleted = 2 AND deleted_at >",
    ] {
        let server = MockServer::start().await;
        Mock::given(wiremock::matchers::method("POST"))
            .respond_with(FailingOn(needle))
            .mount(&server)
            .await;
        let ch = client(&server);
        let outcome = lakehouse_bi::trash::purge_expired(&ch).await;
        assert!(outcome.is_err(), "{needle}");
        assert!(
            writes(&server).await.is_empty(),
            "{:?}",
            writes(&server).await
        );
    }
}

#[tokio::test]
async fn a_sweep_removes_only_what_is_due_and_keeps_a_boards_owner_and_drops_its_history() {
    let server = engine(vec![
        (
            &["SELECT id AS id FROM console.bi_board", "INTERVAL 30 DAY"],
            vec![json!({ "id": "b_old" })],
        ),
        (
            &["SELECT id AS id FROM console.bi_chart", "INTERVAL 30 DAY"],
            vec![json!({ "id": "c_old" })],
        ),
    ])
    .await;
    let ch = client(&server);
    let purged = lakehouse_bi::trash::purge_expired(&ch).await.unwrap();
    assert_eq!((purged.boards, purged.charts), (1, 1));
    let written = writes(&server).await;
    let chart = written
        .iter()
        .find(|s| s.contains("INTO console.bi_chart"))
        .unwrap();
    assert!(
        chart.contains("is_deleted = 2 AND id IN ('c_old')"),
        "{chart}"
    );
    let board = written
        .iter()
        .find(|s| s.contains("INTO console.bi_board ("))
        .unwrap();
    assert!(
        board.contains("SELECT id, '', owner_id, 1"),
        "owner kept: {board}"
    );
    assert!(
        board.contains("is_deleted = 2 AND id IN ('b_old')"),
        "{board}"
    );
    let history = written
        .iter()
        .find(|s| s.contains("bi_board_version"))
        .unwrap();
    assert!(history.contains("board IN ('b_old')"), "{history}");
    // Nothing else was written: no other id, no live row.
    assert_eq!(written.len(), 3, "{written:?}");
}

#[tokio::test]
async fn the_trash_list_and_a_restore_are_scoped_to_the_caller() {
    let mine = owner_clause(ME);
    let server = engine(vec![]).await;
    let ch = client(&server);
    let who = Viewer::user(id_of(ME), false);
    let _ = lakehouse_bi::trash::list_trash(&ch, &who).await.unwrap();
    let boards = sent(&server, "FROM console.bi_board FINAL WHERE is_deleted = 2").await;
    assert!(boards[0].contains(&mine), "{}", boards[0]);
    let charts = sent(&server, "FROM console.bi_chart FINAL WHERE is_deleted = 2").await;
    assert!(charts[0].contains("deleted_with = ''"), "{}", charts[0]);
    assert!(
        charts[0].contains(&private_chart_clause(ME)),
        "{}",
        charts[0]
    );
    // Nothing found for the caller: "not in the trash", and no write.
    for kind in [items_kind_board(), items_kind_chart()] {
        let refusal = match kind {
            "board" => lakehouse_bi::trash::restore_board(&ch, "x", &who)
                .await
                .unwrap_err(),
            _ => lakehouse_bi::trash::restore_chart(&ch, "x", &who)
                .await
                .unwrap_err(),
        };
        assert!(
            refusal.to_string().contains("not in the trash"),
            "{refusal}"
        );
    }
    assert!(writes(&server).await.is_empty());
}

fn items_kind_board() -> &'static str {
    "board"
}

fn items_kind_chart() -> &'static str {
    "chart"
}

#[tokio::test]
async fn favourites_recent_items_and_search_ask_for_what_the_caller_may_see() {
    let server = engine(vec![]).await;
    let ch = client(&server);
    let who = Viewer::user(id_of(ME), false);
    // A blank search asks nothing; a typed one asks for visible rows only.
    assert!(
        lakehouse_bi::search::search(&ch, &who, "  ")
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        sent(&server, "positionCaseInsensitiveUTF8")
            .await
            .is_empty()
    );
    lakehouse_bi::search::search(&ch, &who, "50%_off")
        .await
        .unwrap();
    let boards = sent(
        &server,
        "FROM console.bi_board FINAL WHERE is_deleted = 0 AND id !=",
    )
    .await;
    assert!(boards[0].contains(&owner_clause(ME)), "{}", boards[0]);
    assert!(
        boards[0].contains("positionCaseInsensitiveUTF8(name, '50%_off') > 0"),
        "{}",
        boards[0]
    );
    let charts = sent(&server, "SELECT id, title, board FROM console.bi_chart").await;
    assert!(
        charts[0].contains(&private_chart_clause(ME)),
        "{}",
        charts[0]
    );
    assert!(
        charts[0].contains("NOT IN ('text', 'heading', 'divider', 'section')"),
        "{}",
        charts[0]
    );
    // A tile that shows only text is not a search result, and the text is far
    // shorter than the limit.
    let long = "x".repeat(101);
    assert!(
        lakehouse_bi::search::search(&ch, &who, &long)
            .await
            .is_err()
    );
    // A favourite of something the caller cannot see is refused and writes nothing.
    let refused = lakehouse_bi::items::set_favourite(
        &ch,
        &who,
        lakehouse_bi::items::Kind::Chart,
        "c_private",
        true,
    )
    .await;
    assert!(refused.is_err());
    assert!(
        lakehouse_bi::items::touch_recent(&ch, &who, lakehouse_bi::items::Kind::Board, "b_draft")
            .await
            .is_err()
    );
    assert!(
        writes(&server).await.is_empty(),
        "{:?}",
        writes(&server).await
    );
}

// ── History ─────────────────────────────────────────────────────────────

fn one_chart_state(dimension: &str, measure: &str) -> lakehouse_bi::versions::BoardState {
    let def = json!({
        "title": "Hotels", "mart": "mart_a", "kind": "bar",
        "dimension": dimension, "measures": [measure], "board": "b1",
    });
    let spec = json!({
        "id": "c1", "title": "Hotels", "kind": "bar", "mart": "mart_a",
        "sql": "SELECT 1", "x": dimension, "y": measure,
    });
    lakehouse_bi::versions::BoardState {
        v: 1,
        name: "Older name".to_owned(),
        description: String::new(),
        layout: lakehouse_bi::store::LayoutMap::new(),
        filters: Vec::new(),
        refresh_seconds: None,
        grain: String::new(),
        params: Vec::new(),
        tabs: Vec::new(),
        phone_layout: Vec::new(),
        theme: String::new(),
        font: String::new(),
        charts: vec![lakehouse_bi::versions::StateChart {
            id: "c1".to_owned(),
            title: "Hotels".to_owned(),
            created_by: "ui".to_owned(),
            spec: json!({ "spec": spec, "def": def, "hasYear": false }),
        }],
    }
}

fn history_rules(
    state: &lakehouse_bi::versions::BoardState,
    mart_cols: Vec<Value>,
) -> Vec<(Vec<&'static str>, Vec<Value>)> {
    vec![
        (
            vec!["SELECT is_deleted, owner_id FROM console.bi_board"],
            vec![state_row(0, "")],
        ),
        (
            vec!["SELECT state_json FROM console.bi_board_version"],
            vec![json!({ "state_json": serde_json::to_string(state).unwrap() })],
        ),
        (
            vec![
                "SELECT id, spec_json, board, created_by",
                "FROM console.bi_chart FINAL",
            ],
            vec![chart_row("c1", "b1", "mart_a", None)],
        ),
        (
            vec!["SELECT toString(count()) AS n FROM system.tables"],
            vec![json!({ "n": "1" })],
        ),
        (vec!["SELECT name, type FROM system.columns"], mart_cols),
        (
            vec!["FROM console.bi_board FINAL WHERE is_deleted = 0 AND id='b1'"],
            vec![{
                let mut b = board_row("b1", "");
                b["public_token"] = json!("p_keep");
                b["embed_enabled"] = json!("1");
                b
            }],
        ),
    ]
}

async fn history_server(rules: Vec<(Vec<&'static str>, Vec<Value>)>) -> MockServer {
    let rules: Vec<(&[&str], Vec<Value>)> = rules
        .iter()
        .map(|(n, r)| (n.as_slice(), r.clone()))
        .collect();
    engine(rules).await
}

async fn revert_to(server: &MockServer, version: u32) -> Result<Value, axum::response::Response> {
    super::super::dashboard_organise::revert(
        State(state_for(server)),
        Extension(person(ME, "dashboard:write")),
        json!({ "board": "b1", "version": version })
            .to_string()
            .into(),
    )
    .await
    .map(|ok| ok.0)
    .map_err(axum::response::IntoResponse::into_response)
}

/// A chart whose column is gone makes the whole revert fail with a sentence
/// that names the chart; nothing is written, not even the charts that were fine.
#[tokio::test]
async fn going_back_to_a_version_that_no_longer_holds_changes_nothing() {
    let state = one_chart_state("provinsi", "hotels");
    // The mart no longer has `hotels`.
    let server = history_server(history_rules(
        &state,
        vec![json!({ "name": "provinsi", "type": "String" })],
    ))
    .await;
    let response = revert_to(&server, 2).await.unwrap_err();
    let status = response.status();
    let body = text_of(response).await;
    assert_eq!(
        status,
        axum::http::StatusCode::UNPROCESSABLE_ENTITY,
        "{body}"
    );
    let message = body["error"].as_str().unwrap();
    assert!(
        message.starts_with("Cannot go back to version 2: the chart \"Hotels\""),
        "{message}"
    );
    assert!(message.ends_with("Nothing was changed."), "{message}");
    assert!(
        writes(&server).await.is_empty(),
        "{:?}",
        writes(&server).await
    );
}

/// A version that still holds is written back in one go: the chart first, then
/// the dashboard's content in a single save that keeps the live sharing state,
/// then a new version. The older name is back; the public link stays as it is.
#[tokio::test]
async fn going_back_writes_the_charts_then_the_board_and_keeps_the_sharing_state() {
    let state = one_chart_state("provinsi", "hotels");
    let server = history_server(history_rules(
        &state,
        vec![
            json!({ "name": "provinsi", "type": "String" }),
            json!({ "name": "hotels", "type": "UInt32" }),
        ],
    ))
    .await;
    let done = match revert_to(&server, 2).await {
        Ok(done) => done,
        Err(response) => panic!("not restored: {}", text_of(response).await),
    };
    assert_eq!(done["restoredFrom"], 2);
    let written = writes(&server).await;
    let chart = written
        .iter()
        .position(|s| s.contains("INTO console.bi_chart"))
        .unwrap();
    let board = written
        .iter()
        .position(|s| s.contains("INTO console.bi_board ("))
        .unwrap();
    let version = written
        .iter()
        .position(|s| s.contains("INTO console.bi_board_version"))
        .unwrap();
    assert!(chart < board && board < version, "{written:?}");
    assert!(
        written[board].contains("'Older name'"),
        "{}",
        written[board]
    );
    // The live public link and the embed switch are carried, not reverted.
    assert!(written[board].contains("'p_keep'"), "{}", written[board]);
    assert_eq!(
        written
            .iter()
            .filter(|s| s.contains("INTO console.bi_board ("))
            .count(),
        1
    );
}

/// `BI-17` part A (D6): going back restores the theme and the font with the
/// rest of the dashboard's content, in the same single save.
#[tokio::test]
async fn going_back_restores_the_theme_and_the_font() {
    let mut state = one_chart_state("provinsi", "hotels");
    state.theme = "dark".to_owned();
    state.font = "jetbrains-mono".to_owned();
    let server = history_server(history_rules(
        &state,
        vec![
            json!({ "name": "provinsi", "type": "String" }),
            json!({ "name": "hotels", "type": "UInt32" }),
        ],
    ))
    .await;
    if let Err(response) = revert_to(&server, 2).await {
        panic!("not restored: {}", text_of(response).await);
    }
    let written = writes(&server).await;
    let board = written
        .iter()
        .find(|s| s.contains("INTO console.bi_board ("))
        .unwrap();
    assert!(board.contains("theme, font"), "{board}");
    assert!(board.contains("'dark', 'jetbrains-mono'"), "{board}");
}

async fn edit_board(server: &MockServer, body: Value) -> Result<Value, axum::response::Response> {
    boards_update(
        State(state_for(server)),
        Extension(person(ME, "dashboard:write")),
        body.to_string().into(),
    )
    .await
    .map(|ok| ok.0)
    .map_err(axum::response::IntoResponse::into_response)
}

fn board_edit_rules() -> Vec<(&'static [&'static str], Vec<Value>)> {
    vec![
        (
            &["SELECT is_deleted, owner_id FROM console.bi_board"],
            vec![state_row(0, "")],
        ),
        (
            &["FROM console.bi_board FINAL WHERE is_deleted = 0 AND id='b1'"],
            vec![board_row("b1", "")],
        ),
    ]
}

/// A theme outside the three, and a font the console does not ship (so no
/// administrator can have allowed it), are refused before anything is written.
#[tokio::test]
async fn a_board_edit_refuses_an_unknown_theme_and_a_font_that_is_not_allowed() {
    let server = engine(board_edit_rules()).await;
    for (body, why) in [
        (json!({ "id": "b1", "theme": "sepia" }), "theme must be"),
        (
            json!({ "id": "b1", "font": "comic-sans" }),
            "not one an administrator allows",
        ),
    ] {
        let response = edit_board(&server, body).await.unwrap_err();
        assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
        let message = text_of(response).await;
        assert!(
            message["error"].as_str().unwrap().contains(why),
            "{message}"
        );
    }
    assert!(
        writes(&server).await.is_empty(),
        "{:?}",
        writes(&server).await
    );
}

#[tokio::test]
async fn a_board_edit_saves_the_theme_and_the_font_in_one_full_row_and_snapshots_a_version() {
    let server = engine(board_edit_rules()).await;
    edit_board(
        &server,
        json!({ "id": "b1", "theme": "dark", "font": "source-serif" }),
    )
    .await
    .unwrap();
    let written = writes(&server).await;
    let board = written
        .iter()
        .find(|s| s.contains("INTO console.bi_board ("))
        .expect("the board row is written");
    assert!(board.contains("'dark', 'source-serif'"), "{board}");
    // `auto` and the console's own font clear both columns.
    edit_board(&server, json!({ "id": "b1", "theme": "auto", "font": "" }))
        .await
        .unwrap();
    let all = writes(&server).await;
    let cleared = all
        .iter()
        .rfind(|s| s.contains("INTO console.bi_board ("))
        .unwrap();
    assert!(cleared.contains("'', '', "), "{cleared}");
    // The built-in dashboard takes neither.
    let refused = edit_board(&server, json!({ "id": "default", "theme": "dark" }))
        .await
        .unwrap_err();
    assert_eq!(refused.status(), axum::http::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn a_version_that_was_pruned_or_never_existed_is_not_found() {
    let server = engine(vec![(
        &["SELECT is_deleted, owner_id FROM console.bi_board"],
        vec![state_row(0, "")],
    )])
    .await;
    let response = revert_to(&server, 7).await.unwrap_err();
    assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);
    assert!(writes(&server).await.is_empty());
}

#[tokio::test]
async fn the_built_in_dashboard_keeps_no_history_and_cannot_be_reverted() {
    let server = engine(vec![]).await;
    let ch = client(&server);
    let author = lakehouse_bi::versions::Author {
        id: "x".into(),
        name: "x".into(),
    };
    assert_eq!(
        lakehouse_bi::versions::record(&ch, "default", &author, None)
            .await
            .unwrap(),
        None
    );
    assert!(sent(&server, "bi_board_version").await.is_empty());
    let body = json!({ "board": "default", "version": 1 });
    let outcome = super::super::dashboard_organise::revert(
        State(state_for(&server)),
        Extension(person(ME, "dashboard:write")),
        body.to_string().into(),
    )
    .await;
    assert!(outcome.is_err());
}

/// Same state as the last version: nothing is written. A new state: version
/// N+1 and the versions beyond the newest 15 are tombstoned.
#[tokio::test]
async fn a_version_is_written_only_for_a_change_and_only_fifteen_are_kept() {
    let author = lakehouse_bi::versions::Author {
        id: "u".into(),
        name: "Ann".into(),
    };
    // The board as the store holds it, twice: once to learn the state, once to compare.
    let live = |extra: Vec<(Vec<&'static str>, Vec<Value>)>| {
        let mut rules = vec![(
            vec!["FROM console.bi_board FINAL WHERE is_deleted = 0 AND id='b1'"],
            vec![board_row("b1", "")],
        )];
        rules.extend(extra);
        rules
    };
    let learn = history_server(live(vec![])).await;
    let state = lakehouse_bi::versions::capture(&client(&learn), "b1")
        .await
        .unwrap()
        .unwrap();
    let same = serde_json::to_string(&state).unwrap();

    let unchanged = history_server(live(vec![(
        vec!["SELECT version, state_json FROM console.bi_board_version"],
        vec![json!({ "version": 4, "state_json": same })],
    )]))
    .await;
    assert_eq!(
        lakehouse_bi::versions::record(&client(&unchanged), "b1", &author, None)
            .await
            .unwrap(),
        None
    );
    assert!(writes(&unchanged).await.is_empty());

    let changed = history_server(live(vec![
        (
            vec!["SELECT version, state_json FROM console.bi_board_version"],
            vec![json!({ "version": 4, "state_json": "{}" })],
        ),
        (vec!["SELECT max(version) AS v"], vec![json!({ "v": 16 })]),
        (
            vec!["OFFSET 15"],
            vec![json!({ "version": 1 }), json!({ "version": 2 })],
        ),
    ]))
    .await;
    assert_eq!(
        lakehouse_bi::versions::record(&client(&changed), "b1", &author, None)
            .await
            .unwrap(),
        Some(17)
    );
    let written = writes(&changed).await;
    assert!(written[0].contains("VALUES ('b1', 17,"), "{}", written[0]);
    assert!(written[1].contains("version IN (1, 2)"), "{}", written[1]);
    assert!(
        written[1].contains("SELECT board, version, '', 1"),
        "{}",
        written[1]
    );
}

// ── Folders and the personal space ─────────────────────────────────────

fn folder_row(id: &str, owner: &str) -> Value {
    json!({
        "id": id, "name": "A folder", "parent_id": "", "created_by": "t", "owner_id": owner,
        "updated_at": "2026-10-14 00:00:00",
    })
}

#[tokio::test]
async fn the_folder_list_leaves_out_another_users_personal_space() {
    let mine = owner_clause(ME);
    let server = engine(vec![
        (
            &["FROM console.bi_folder FINAL", mine.as_str()],
            vec![folder_row("f_shared", ""), folder_row("fp_me", &id_of(ME))],
        ),
        (
            &["FROM console.bi_folder FINAL"],
            vec![
                folder_row("f_shared", ""),
                folder_row("fp_me", &id_of(ME)),
                folder_row("fp_other", &id_of(OTHER)),
            ],
        ),
    ])
    .await;
    let listed = super::super::dashboard_folders::list(
        State(state_for(&server)),
        Extension(person(ME, "dashboard:read")),
    )
    .await
    .done("listed")
    .0;
    let ids: Vec<&str> = listed["folders"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, vec!["f_shared", "fp_me"]);
    // An administrator's request has no owner restriction, so every space comes back.
    let admin = super::super::dashboard_folders::list(
        State(state_for(&server)),
        Extension(person(OTHER, "dashboard:read, identity:write")),
    )
    .await
    .done("listed")
    .0;
    assert_eq!(admin["folders"].as_array().unwrap().len(), 3);
}

#[tokio::test]
async fn a_personal_space_cannot_be_renamed_moved_deleted_or_hold_folders() {
    let server = engine(vec![(
        &["FROM console.bi_folder FINAL"],
        vec![folder_row("fp_me", &id_of(ME))],
    )])
    .await;
    let state = state_for(&server);
    let me = person(ME, "dashboard:write");
    let create = super::super::dashboard_folders::create(
        State(state.clone()),
        Extension(me.clone()),
        json!({ "name": "Sub", "parentId": "fp_me" })
            .to_string()
            .into(),
    )
    .await;
    assert!(create.is_err());
    let rename = super::super::dashboard_folders::update(
        State(state.clone()),
        Extension(me.clone()),
        json!({ "id": "fp_me", "name": "Renamed" })
            .to_string()
            .into(),
    )
    .await;
    assert!(rename.is_err());
    let delete = super::super::dashboard_folders::delete(
        State(state),
        Extension(me),
        Query(serde_json::from_value(json!({ "id": "fp_me" })).unwrap()),
    )
    .await;
    assert!(delete.is_err());
    assert!(
        writes(&server).await.is_empty(),
        "{:?}",
        writes(&server).await
    );
}

#[tokio::test]
async fn a_dashboard_is_filed_only_in_the_callers_own_space_and_a_sql_source_never_in_one() {
    let server = engine(vec![(
        &["FROM console.bi_folder FINAL"],
        vec![folder_row("fp_other", &id_of(OTHER))],
    )])
    .await;
    let ch = client(&server);
    // An administrator sees the space but cannot file a dashboard into it.
    let admin = Viewer::user(id_of(ME), true);
    let refused =
        super::super::dashboard_folders::ensure_folder_usable(&ch, "fp_other", &admin).await;
    assert!(refused.is_err());
    // The owner can.
    let owner = Viewer::user(id_of(OTHER), false);
    assert!(
        super::super::dashboard_folders::ensure_folder_usable(&ch, "fp_other", &owner)
            .await
            .is_ok()
    );
    // SQL sources are shared by every dashboard: never in a personal space.
    assert!(
        super::super::dashboard_folders::ensure_folder_exists(&ch, "fp_other")
            .await
            .is_err()
    );
}

#[tokio::test]
async fn moving_a_shared_board_into_a_personal_space_is_refused() {
    let mut shared = board_row("b1", "");
    shared["public_token"] = json!("p_live");
    let server = engine(vec![
        (
            &["FROM console.bi_folder FINAL"],
            vec![folder_row("fp_me", &id_of(ME))],
        ),
        (
            &["FROM console.bi_board FINAL WHERE is_deleted = 0 AND id='b1'"],
            vec![shared],
        ),
    ])
    .await;
    let ch = client(&server);
    let refused = lakehouse_bi::store::move_board(&ch, "b1", "fp_me")
        .await
        .unwrap_err();
    assert!(
        refused.to_string().contains("Turn off sharing"),
        "{refused}"
    );
    assert!(writes(&server).await.is_empty());
    let private = {
        let mut b = board_row("b2", &id_of(ME));
        b["public_token"] = json!("");
        b
    };
    let server = engine(vec![(
        &["FROM console.bi_board FINAL WHERE is_deleted = 0 AND id='b2'"],
        vec![private],
    )])
    .await;
    let ch = client(&server);
    let refused = lakehouse_bi::store::set_board_public(&ch, "b2", true)
        .await
        .unwrap_err();
    assert!(refused.to_string().contains("personal space"), "{refused}");
    let refused = lakehouse_bi::store::set_board_embed(&ch, "b2", true)
        .await
        .unwrap_err();
    assert!(refused.to_string().contains("personal space"), "{refused}");
    assert!(writes(&server).await.is_empty());
}

// ── What names a use ───────────────────────────────────────────────────

/// A chart on somebody's personal board still counts as a use of a thing (so a
/// delete is blocked) but is neither named nor listed for the caller.
#[tokio::test]
async fn a_use_by_a_chart_the_caller_cannot_see_blocks_a_delete_without_naming_it() {
    let (visible, everything) = visible_and_private_charts();
    let me = private_chart_clause(ME);
    let server = engine(vec![
        (&["FROM console.bi_chart FINAL", me.as_str()], visible),
        (&["FROM console.bi_chart FINAL"], everything),
    ])
    .await;
    let ch = client(&server);
    let seen = super::super::dashboard_organise::Seen::read(&ch, &Viewer::user(id_of(ME), false))
        .await
        .unwrap();
    assert_eq!(seen.all_charts.len(), 2);
    assert_eq!(seen.charts.len(), 1);
    let hidden: Vec<String> = seen
        .hidden_charts()
        .into_iter()
        .map(|c| c.spec.id)
        .collect();
    assert_eq!(hidden, vec!["c_private"]);
    // An administrator hides nothing.
    let seen = super::super::dashboard_organise::Seen::read(&ch, &Viewer::user(id_of(OTHER), true))
        .await
        .unwrap();
    assert!(seen.hidden_charts().is_empty());
}

/// A parameter that a dashboard in somebody's personal space shows still stops
/// the parameter being deleted, but the refusal does not name that dashboard.
#[tokio::test]
async fn a_parameter_shown_on_a_private_dashboard_blocks_its_delete_without_naming_it() {
    let mine = owner_clause(ME);
    let mut private = board_row("b_draft", &id_of(OTHER));
    private["name"] = json!("Secret quarterly plan");
    private["params_json"] = json!(r#"[{"name":"cutoff"}]"#);
    let server = engine(vec![
        (
            &["FROM console.bi_param FINAL"],
            vec![json!({
                "id": "p_1", "name": "cutoff",
                "def_json": r#"{"name":"cutoff","type":"number","default":5}"#,
                "created_by": "t", "updated_at": "2026-10-14 00:00:00",
            })],
        ),
        // Nothing the caller may see uses it; the unrestricted read finds the
        // draft. The built-in board's row is a read by id of its own.
        (&["FROM console.bi_board FINAL", mine.as_str()], vec![]),
        (&["FROM console.bi_board FINAL", "id='default'"], vec![]),
        (&["FROM console.bi_board FINAL"], vec![private]),
    ])
    .await;
    let outcome = super::super::params::delete(
        State(state_for(&server)),
        Some(Extension(person(ME, "dashboard:write"))),
        Query(serde_json::from_value(json!({ "id": "p_1" })).unwrap()),
    )
    .await;
    let response = axum::response::IntoResponse::into_response(match outcome {
        Err(rejection) => rejection,
        Ok(_) => panic!("a parameter on a hidden dashboard must not be deleted"),
    });
    assert_eq!(response.status(), axum::http::StatusCode::CONFLICT);
    let body = text_of(response).await;
    let message = body["error"].as_str().unwrap();
    assert!(message.contains("a dashboard you cannot see"), "{message}");
    assert!(!message.contains("Secret"), "{message}");
    assert!(
        writes(&server).await.is_empty(),
        "{:?}",
        writes(&server).await
    );
}

/// A drill asks for the saved chart by id, so it finds it through the caller's
/// own list: another user's private chart reads as a chart that is not there.
#[tokio::test]
async fn a_drill_of_a_hidden_chart_is_not_found() {
    let (visible, everything) = visible_and_private_charts();
    let me = private_chart_clause(ME);
    let server = engine(vec![
        (&["FROM console.bi_chart FINAL", me.as_str()], visible),
        (&["FROM console.bi_chart FINAL"], everything),
    ])
    .await;
    let refusal = super::super::drill::drill_input(
        &client(&server),
        json!({ "drillOf": "c_private", "to": "city" }),
        &Viewer::user(id_of(ME), false),
    )
    .await
    .unwrap_err();
    assert!(
        matches!(&refusal, ApiError::NotFound(m) if m == "chart not found."),
        "{refusal:?}"
    );
}

/// The Share dialog's read of a private board is a 404, not its sample token.
#[tokio::test]
async fn embed_info_for_a_private_board_is_not_found() {
    let mine = owner_clause(ME);
    let server = engine(vec![
        (&["FROM console.bi_board FINAL", mine.as_str()], vec![]),
        (
            &["FROM console.bi_board FINAL"],
            vec![board_row("b_draft", &id_of(OTHER))],
        ),
    ])
    .await;
    let response = super::embed_info(
        State(state_for(&server)),
        Extension(person(ME, "dashboard:read")),
        Query(serde_json::from_value(json!({ "board": "b_draft" })).unwrap()),
    )
    .await;
    assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);
}

/// A chart on somebody's personal board still blocks deleting the SQL source it
/// reads, but the refusal does not name it.
#[tokio::test]
async fn a_sql_source_used_by_a_private_chart_cannot_be_deleted_and_the_chart_is_not_named() {
    let me = private_chart_clause(ME);
    let server = engine(vec![
        (
            &["FROM console.bi_source FINAL"],
            vec![json!({
                "id": "s_1", "title": "src", "sql": "SELECT 1", "columns_json": "[]",
                "folder_id": "", "created_by": "t", "updated_at": "2026-10-14 00:00:00",
            })],
        ),
        (&["FROM console.bi_chart FINAL", me.as_str()], vec![]),
        (
            &["FROM console.bi_chart FINAL"],
            vec![chart_row("c_secret", "b_draft", "", Some("s_1"))],
        ),
    ])
    .await;
    let outcome = super::super::dashboard_sources::delete(
        State(state_for(&server)),
        Extension(person(ME, "dashboard:write")),
        Query(serde_json::from_value(json!({ "id": "s_1" })).unwrap()),
    )
    .await;
    let response = axum::response::IntoResponse::into_response(match outcome {
        Err(rejection) => rejection,
        Ok(_) => panic!("a source used by a hidden chart must not be deleted"),
    });
    assert_eq!(response.status(), axum::http::StatusCode::CONFLICT);
    let body = text_of(response).await;
    let message = body["error"].as_str().unwrap();
    assert!(message.contains("a dashboard you cannot see"), "{message}");
    assert!(!message.contains("c_secret"), "{message}");
    assert!(
        writes(&server).await.is_empty(),
        "{:?}",
        writes(&server).await
    );
}

// ── BI-18 part C review fix (BLOCKER) R1: no tile is left naming no tab ──

const T_A: &str = r#"{"id":"t_a","title":"A"}"#;
const T_B: &str = r#"{"id":"t_b","title":"B"}"#;

/// Board `b1` as stored: three tiles, two of them naming no tab.
fn untagged_board(tabs: &str) -> Value {
    let mut b = board_row("b1", "");
    b["layout_json"] = json!(
        r#"{"c1":{"x":0,"y":0,"w":6,"h":4},"c2":{"x":6,"y":0,"w":6,"h":4},"c3":{"x":0,"y":4,"w":6,"h":4,"tab":"t_b"}}"#
    );
    b["tabs_json"] = json!(tabs);
    if tabs == "[]" {
        // A board with no tabs has no tile naming one.
        b["layout_json"] = json!(
            r#"{"c1":{"x":0,"y":0,"w":6,"h":4},"c2":{"x":6,"y":0,"w":6,"h":4},"c3":{"x":0,"y":4,"w":6,"h":4}}"#
        );
    }
    b
}

fn tile_of(insert: &str, id: &str, tab: &str) -> bool {
    insert.contains(&format!(r#""{id}":{{"x":"#)) && {
        let at = insert.find(&format!(r#""{id}":{{"#)).unwrap();
        let tile = &insert[at..insert[at..].find('}').unwrap() + at];
        tile.contains(&format!(r#""tab":"{tab}""#))
    }
}

async fn board_server() -> MockServer {
    engine(vec![
        (
            &["SELECT is_deleted, owner_id FROM console.bi_board"],
            vec![state_row(0, "")],
        ),
        (
            &["FROM console.bi_board FINAL WHERE is_deleted = 0 AND id='b1'"],
            vec![untagged_board(&format!("[{T_A},{T_B}]"))],
        ),
    ])
    .await
}

fn last_board_write(written: &[String]) -> &String {
    written
        .iter()
        .rev()
        .find(|s| s.contains("INTO console.bi_board ("))
        .expect("the board was saved")
}

fn assert_tiles_stayed(insert: &str) {
    assert!(tile_of(insert, "c1", "t_a"), "{insert}");
    assert!(tile_of(insert, "c2", "t_a"), "{insert}");
    assert!(tile_of(insert, "c3", "t_b"), "{insert}");
}

/// Console path: Maps moves left; the tiles that named no tab were on the first
/// tab as stored (A), and are written as A before the order changes.
#[tokio::test]
async fn the_console_save_that_reorders_tabs_stamps_untagged_tiles_with_the_stored_first_tab() {
    let server = board_server().await;
    let body = json!({ "id": "b1", "tabs": [
        { "id": "t_b", "title": "B" }, { "id": "t_a", "title": "A" } ] });
    boards_update(
        State(state_for(&server)),
        Extension(person(ME, "dashboard:write")),
        body.to_string().into(),
    )
    .await
    .done("saved");
    let written = writes(&server).await;
    let board = last_board_write(&written);
    assert_tiles_stayed(board);
    // The order itself was written.
    assert!(
        board.find(r#""id":"t_b""#) < board.find(r#""id":"t_a""#),
        "{board}"
    );
}

/// Creating the first tab stamps every tile with it.
#[tokio::test]
async fn creating_the_first_tab_stamps_every_tile() {
    let server = engine(vec![
        (
            &["SELECT is_deleted, owner_id FROM console.bi_board"],
            vec![state_row(0, "")],
        ),
        (
            &["FROM console.bi_board FINAL WHERE is_deleted = 0 AND id='b1'"],
            vec![untagged_board("[]")],
        ),
    ])
    .await;
    let body = json!({ "id": "b1", "tabs": [{ "id": "t_a", "title": "A" }] });
    boards_update(
        State(state_for(&server)),
        Extension(person(ME, "dashboard:write")),
        body.to_string().into(),
    )
    .await
    .done("saved");
    let written = writes(&server).await;
    let board = last_board_write(&written);
    for id in ["c1", "c2", "c3"] {
        assert!(tile_of(board, id, "t_a"), "{id}: {board}");
    }
}

/// A chart added to a board with tabs gets its tab when it is created.
#[tokio::test]
async fn a_chart_added_to_a_tabbed_board_gets_its_tab_at_creation() {
    let server = board_server().await;
    let ch = client(&server);
    lakehouse_bi::store::place_new_chart(&ch, "b1", "u_new")
        .await
        .unwrap();
    let written = writes(&server).await;
    let board = last_board_write(&written);
    assert!(tile_of(board, "u_new", "t_a"), "{board}");
    assert_tiles_stayed(board);
    // A board without tabs, and a chart that has a tile, are left alone.
    let none = engine(vec![(
        &["FROM console.bi_board FINAL WHERE is_deleted = 0 AND id='b1'"],
        vec![untagged_board("[]")],
    )])
    .await;
    lakehouse_bi::store::place_new_chart(&client(&none), "b1", "u_new")
        .await
        .unwrap();
    lakehouse_bi::store::place_new_chart(&ch, "b1", "c1")
        .await
        .unwrap();
    assert!(writes(&none).await.is_empty());
    assert_eq!(writes(&server).await.len(), 1);
}

/// A duplicate of an unstamped tabbed board names its tab on every tile.
#[tokio::test]
async fn a_duplicate_of_a_tabbed_board_names_the_tab_of_every_tile() {
    let server = engine(vec![
        (
            &["FROM console.bi_board FINAL WHERE is_deleted = 0 AND id='b1'"],
            vec![untagged_board(&format!("[{T_A},{T_B}]"))],
        ),
        (
            &[
                "SELECT id, spec_json, board, created_by",
                "FROM console.bi_chart FINAL",
            ],
            vec![
                chart_row("c1", "b1", "mart_a", None),
                chart_row("c2", "b1", "mart_a", None),
                chart_row("c3", "b1", "mart_a", None),
            ],
        ),
    ])
    .await;
    let ch = client(&server);
    let copy = lakehouse_bi::store::duplicate_board(&ch, "b1", "Ann", &Viewer::system())
        .await
        .unwrap();
    let written = writes(&server).await;
    let board = last_board_write(&written);
    let tags = board.matches(r#""tab":"t_a""#).count();
    assert_eq!(tags, 2, "{board}");
    assert_eq!(board.matches(r#""tab":"t_b""#).count(), 1, "{board}");
    assert!(board.contains(&copy.id));
}

/// `PR #199 review M BLOCKER 5`: a new dashboard filed in the personal space
/// is private from its FIRST insert, so it is never public in between.
#[tokio::test]
async fn a_new_dashboard_in_the_personal_space_is_private_from_its_first_insert() {
    let server = engine(vec![(
        &["FROM console.bi_folder FINAL"],
        vec![folder_row("fp_me", &id_of(ME))],
    )])
    .await;
    let body = json!({ "name": "Draft", "folderId": "fp_me" });
    boards_create(
        State(state_for(&server)),
        Some(Extension(person(ME, "dashboard:write"))),
        body.to_string().into(),
    )
    .await
    .done("the dashboard was created");
    let written = writes(&server).await;
    let first = written
        .iter()
        .find(|w| w.contains("INTO console.bi_board ("))
        .expect("the dashboard was created");
    assert!(first.contains(&id_of(ME)), "{first}");
}

/// `PR #199 review M BLOCKER 5`: a copy of a private dashboard is private
/// from its FIRST insert, so it is never visible to another user, not even
/// for the moment between the insert and the write that used to give it its
/// owner back.
#[tokio::test]
async fn a_duplicate_of_a_private_board_is_private_from_its_first_insert() {
    let server = engine(vec![
        (
            &["FROM console.bi_board FINAL WHERE is_deleted = 0 AND id='b1'"],
            vec![board_row("b1", "u_owner")],
        ),
        (
            &[
                "SELECT id, spec_json, board, created_by",
                "FROM console.bi_chart FINAL",
            ],
            vec![chart_row("c1", "b1", "mart_a", None)],
        ),
    ])
    .await;
    let ch = client(&server);
    let copy =
        lakehouse_bi::store::duplicate_board(&ch, "b1", "Ann", &Viewer::user("u_owner", false))
            .await
            .unwrap();
    assert_eq!(copy.owner_id.as_deref(), Some("u_owner"));
    let written = writes(&server).await;
    let first = written
        .iter()
        .find(|w| w.contains("INTO console.bi_board ("))
        .expect("the copy was created");
    // The owner is in the first insert, not only in a later write.
    assert!(first.contains("'u_owner'"), "{first}");
}

/// `PR #199 review M BLOCKER 5`: a copy whose chart could not be written is
/// taken back, so no half-built dashboard is left behind.
#[tokio::test]
async fn a_duplicate_that_cannot_be_finished_leaves_no_copy_behind() {
    use wiremock::matchers::{body_string_contains, method};
    let server = MockServer::start().await;
    // The chart's own statement fails, so the copy cannot be finished. The
    // first mock registered is the one that answers, so this one is mounted
    // before the catch-all.
    Mock::given(method("POST"))
        .and(body_string_contains("SELECT 1"))
        .respond_with(ResponseTemplate::new(500).set_body_string("boom"))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(body_string_contains(
            "FROM console.bi_board FINAL WHERE is_deleted = 0 AND id='b1'",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "meta": [], "data": [board_row("b1", "u_owner")], "rows": 1
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(body_string_contains("FROM console.bi_chart FINAL"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "meta": [], "data": [chart_row("c1", "b1", "mart_a", None)], "rows": 1
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "meta": [], "data": [], "rows": 0
        })))
        .mount(&server)
        .await;
    let ch = client(&server);
    lakehouse_bi::store::duplicate_board(&ch, "b1", "Ann", &Viewer::user("u_owner", false))
        .await
        .expect_err("the chart's statement fails");
    let written = writes(&server).await;
    // The copy was taken back: the board and its charts are wiped.
    assert!(
        written
            .iter()
            .any(|w| w.contains("INTO console.bi_board (id, name, owner_id, is_deleted)")),
        "{written:?}"
    );
    assert!(
        written.iter().any(|w| w.contains(
            "INTO console.bi_chart (id, title, spec_json, board, created_by, is_deleted)"
        )),
        "{written:?}"
    );
}

/// `PR #199 review O SHOULD-FIX 3`: a copy that could not be finished and
/// could not be taken back says so: the cleanup failure is reported and the
/// copy's own error is kept.
#[tokio::test]
async fn a_duplicate_whose_cleanup_fails_reports_the_cleanup_failure() {
    use wiremock::matchers::{body_string_contains, method};
    let server = MockServer::start().await;
    // The half-built copy cannot be taken back: the wipe of its charts fails.
    Mock::given(method("POST"))
        .and(body_string_contains(
            "INSERT INTO console.bi_chart (id, title, spec_json, board, created_by, is_deleted)",
        ))
        .respond_with(ResponseTemplate::new(500).set_body_string("boom"))
        .mount(&server)
        .await;
    // The chart's own statement fails, so the copy cannot be finished.
    Mock::given(method("POST"))
        .and(body_string_contains("SELECT 1"))
        .respond_with(ResponseTemplate::new(500).set_body_string("boom"))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(body_string_contains(
            "FROM console.bi_board FINAL WHERE is_deleted = 0 AND id='b1'",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "meta": [], "data": [board_row("b1", "u_owner")], "rows": 1
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(body_string_contains("FROM console.bi_chart FINAL"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "meta": [], "data": [chart_row("c1", "b1", "mart_a", None)], "rows": 1
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "meta": [], "data": [], "rows": 0
        })))
        .mount(&server)
        .await;
    let ch = client(&server);
    let err =
        lakehouse_bi::store::duplicate_board(&ch, "b1", "Ann", &Viewer::user("u_owner", false))
            .await
            .expect_err("the chart's statement fails");
    match err {
        lakehouse_bi::store::DuplicateError::CopyAndCleanup { cause, cleanup } => {
            assert!(
                matches!(cause, lakehouse_bi::store::BiError::Clickhouse(_)),
                "the copy's own error is kept: {cause:?}"
            );
            assert!(
                matches!(cleanup, lakehouse_clickhouse::ChError::Server(_)),
                "the cleanup failure is carried out: {cleanup:?}"
            );
        }
        other @ lakehouse_bi::store::DuplicateError::Copy(_) => {
            panic!("a cleanup that failed must be reported, not dropped: {other:?}")
        }
    }
}

/// `PR #199 review M SHOULD-FIX BI-18 3`: a save that arrives after the
/// dashboard was deleted must not write a blank row over its trash entry: the
/// dashboard would come back to life as an empty shell.
#[tokio::test]
async fn a_save_that_races_a_delete_writes_no_row() {
    let server = engine(vec![(
        &["FROM console.bi_board FINAL WHERE is_deleted = 0 AND id='b1'"],
        vec![],
    )])
    .await;
    let ch = client(&server);
    let layout: lakehouse_bi::store::LayoutMap =
        serde_json::from_value(json!({ "c1": { "x": 0, "y": 0, "w": 6, "h": 4 } })).unwrap();
    let refused = lakehouse_bi::store::update_board_layout(&ch, "b1", &layout).await;
    assert!(refused.is_err(), "{refused:?}");
    assert!(writes(&server).await.is_empty());
    // The built-in dashboard has no row until something is saved: its shell is
    // still written.
    let written = lakehouse_bi::store::update_board_layout(
        &ch,
        lakehouse_bi::store::DEFAULT_BOARD_ID,
        &layout,
    )
    .await;
    assert!(written.is_ok(), "{written:?}");
    assert!(
        writes(&server)
            .await
            .iter()
            .any(|w| w.contains("INTO console.bi_board (")),
        "the built-in dashboard's row is still written"
    );
}

/// Going back to a version saved before the stamping: its untagged tiles were on
/// ITS first tab (B), not the live board's (A).
#[tokio::test]
async fn going_back_to_an_unstamped_version_keeps_every_tile_on_the_tab_it_showed_on() {
    let mut state = one_chart_state("provinsi", "hotels");
    state.tabs = vec![
        lakehouse_bi::layout::Tab {
            id: "t_b".into(),
            title: "B".into(),
        },
        lakehouse_bi::layout::Tab {
            id: "t_a".into(),
            title: "A".into(),
        },
    ];
    state.layout =
        serde_json::from_value(json!({ "c1": { "x": 0, "y": 0, "w": 6, "h": 4 } })).unwrap();
    let mut rules = history_rules(
        &state,
        vec![
            json!({ "name": "provinsi", "type": "String" }),
            json!({ "name": "hotels", "type": "UInt32" }),
        ],
    );
    // The live board has the tabs in the other order.
    rules.retain(|(n, _)| !n[0].contains("id='b1'"));
    rules.push((
        vec!["FROM console.bi_board FINAL WHERE is_deleted = 0 AND id='b1'"],
        vec![untagged_board(&format!("[{T_A},{T_B}]"))],
    ));
    let server = history_server(rules).await;
    assert!(revert_to(&server, 2).await.is_ok(), "not restored");
    let written = writes(&server).await;
    let board = last_board_write(&written);
    assert!(tile_of(board, "c1", "t_b"), "{board}");
}

/// BI-18 part C review fix (BLOCKER) R13: any write of a board gives every live
/// chart with no stored box one, on the first tab, below that tab's stored
/// bottom; a stored tile does not move.
#[tokio::test]
async fn a_board_write_gives_every_live_chart_a_box_on_the_first_tab() {
    let server = engine(vec![
        (
            &["SELECT is_deleted, owner_id FROM console.bi_board"],
            vec![state_row(0, "")],
        ),
        (
            &["FROM console.bi_board FINAL WHERE is_deleted = 0 AND id='b1'"],
            vec![untagged_board(&format!("[{T_A},{T_B}]"))],
        ),
        (
            &["SELECT id, spec_json FROM console.bi_chart FINAL WHERE is_deleted = 0 AND (board = 'b1')"],
            ["c1", "c2", "c3", "u1", "u2", "u3"]
                .iter()
                .map(|id| chart_row(id, "b1", "mart_a", None))
                .collect(),
        ),
    ])
    .await;
    let body = json!({ "id": "b1", "name": "Renamed" });
    boards_update(
        State(state_for(&server)),
        Extension(person(ME, "dashboard:write")),
        body.to_string().into(),
    )
    .await
    .done("saved");
    let written = writes(&server).await;
    let board = last_board_write(&written);
    assert_tiles_stayed(board);
    // c1 and c2 end at y 4 on t_a; c3 is on t_b and does not count.
    for (id, x, y) in [("u1", 0, 4), ("u2", 6, 4), ("u3", 0, 10)] {
        assert!(tile_of(board, id, "t_a"), "{id}: {board}");
        assert!(
            board.contains(&format!(r#""{id}":{{"x":{x},"y":{y},"w":6,"h":6"#)),
            "{id}: {board}"
        );
    }
}

/// BI-18 part C review fix (SHOULD-FIX) R14, closed by R13: deleting a chart on
/// a board whose other charts had no stored box leaves no hole; they get their
/// boxes in the same write, below what is left of the tab.
#[tokio::test]
async fn deleting_a_chart_leaves_no_hole_for_charts_that_had_no_box() {
    let server = engine(vec![
        (
            &["FROM console.bi_board FINAL WHERE is_deleted = 0 AND id='b1'"],
            vec![untagged_board(&format!("[{T_A},{T_B}]"))],
        ),
        (
            &["SELECT id, spec_json FROM console.bi_chart FINAL WHERE is_deleted = 0 AND (board = 'b1')"],
            // c1 is in the trash already; u1 never had a box.
            ["c2", "c3", "u1"]
                .iter()
                .map(|id| chart_row(id, "b1", "mart_a", None))
                .collect(),
        ),
    ])
    .await;
    lakehouse_bi::store::drop_tile_of_deleted_chart(&client(&server), "b1", "c1")
        .await
        .unwrap();
    let written = writes(&server).await;
    let board = last_board_write(&written);
    assert!(!board.contains(r#""c1":{"#), "{board}");
    assert!(tile_of(board, "u1", "t_a"), "{board}");
    assert!(
        board.contains(r#""u1":{"x":0,"y":4,"w":6,"h":6"#),
        "{board}"
    );
}

// ── BI-18 part C review fixes R7 to R10 ─────────────────────────────────

/// The rules of a revert to version 2 of board `b1` whose one chart `c1` is
/// not live: `trashed` says whether it is in the trash.
fn trash_rules(trashed: bool) -> Vec<(Vec<&'static str>, Vec<Value>)> {
    let mut state = one_chart_state("provinsi", "hotels");
    state.layout =
        serde_json::from_value(json!({ "c1": { "x": 0, "y": 0, "w": 6, "h": 4 } })).unwrap();
    let mut rules = history_rules(
        &state,
        vec![
            json!({ "name": "provinsi", "type": "String" }),
            json!({ "name": "hotels", "type": "UInt32" }),
        ],
    );
    rules.retain(|(n, _)| !n.contains(&"SELECT id, spec_json, board, created_by"));
    rules.insert(
        0,
        (
            vec!["SELECT id FROM console.bi_chart FINAL WHERE is_deleted = 2"],
            if trashed {
                vec![json!({ "id": "c1" })]
            } else {
                vec![]
            },
        ),
    );
    rules
}

/// R7: a chart the version had that is in the trash comes back with the revert:
/// written live, named in the answer, its tile in the written layout.
#[tokio::test]
async fn going_back_brings_back_a_chart_that_is_in_the_trash() {
    let server = history_server(trash_rules(true)).await;
    let done = match revert_to(&server, 2).await {
        Ok(done) => done,
        Err(response) => panic!("not restored: {}", text_of(response).await),
    };
    assert_eq!(done["chartsRestored"], json!(["Hotels"]));
    assert_eq!(done["chartsGone"], json!([]));
    let written = writes(&server).await;
    let chart = written
        .iter()
        .position(|s| s.contains("INTO console.bi_chart"))
        .unwrap();
    let board = written
        .iter()
        .position(|s| s.contains("INTO console.bi_board ("))
        .unwrap();
    assert!(chart < board, "{written:?}");
    // A plain insert: the new row has the defaults, so it is live.
    assert!(!written[chart].contains("is_deleted"), "{}", written[chart]);
    assert!(
        written[board].contains(r#""c1":{"x":0,"y":0,"w":6,"h":4}"#),
        "{}",
        written[board]
    );
    // R8: the version it writes says where it came from.
    let version = written
        .iter()
        .find(|s| s.contains("INTO console.bi_board_version"))
        .unwrap();
    assert!(version.contains("'Restored v2'"), "{version}");
}

/// R7: a chart removed from the trash for good cannot come back; the revert
/// still succeeds, names it, and its tile is not in the layout.
#[tokio::test]
async fn going_back_names_a_chart_that_was_removed_for_good_and_leaves_no_hole() {
    let server = history_server(trash_rules(false)).await;
    let done = match revert_to(&server, 2).await {
        Ok(done) => done,
        Err(response) => panic!("not restored: {}", text_of(response).await),
    };
    assert_eq!(done["chartsGone"], json!(["Hotels"]));
    assert_eq!(done["chartsRestored"], json!([]));
    let written = writes(&server).await;
    assert!(
        written.iter().all(|s| !s.contains("INTO console.bi_chart")),
        "{written:?}"
    );
    let board = last_board_write(&written);
    assert!(!board.contains(r#""c1":"#), "{board}");
}

/// `PR #199 review M SHOULD-FIX BI-18 2`: a chart the version had that was
/// moved to another dashboard is not gone. The revert says where it is, so the
/// answer does not say it was removed from the trash for good.
#[tokio::test]
async fn going_back_says_a_chart_that_moved_to_another_dashboard() {
    let mut rules = trash_rules(false);
    rules.insert(
        0,
        (
            vec![
                "SELECT id, spec_json, board, created_by",
                "FROM console.bi_chart FINAL",
            ],
            vec![chart_row("c1", "b_other", "mart_a", None)],
        ),
    );
    let server = history_server(rules).await;
    let done = match revert_to(&server, 2).await {
        Ok(done) => done,
        Err(response) => panic!("not restored: {}", text_of(response).await),
    };
    assert_eq!(done["chartsMoved"], json!(["Hotels"]));
    assert_eq!(done["chartsGone"], json!([]));
}

/// R8, and the question about dragging: a layout save (what a drag in edit mode
/// sends) records a version.
#[tokio::test]
async fn a_layout_save_by_dragging_records_a_version() {
    let server = engine(vec![
        (
            &["SELECT is_deleted, owner_id FROM console.bi_board"],
            vec![state_row(0, "")],
        ),
        (
            &["SELECT version, state_json FROM console.bi_board_version"],
            vec![json!({ "version": 1, "state_json": "{}" })],
        ),
        (&["SELECT max(version) AS v"], vec![json!({ "v": 1 })]),
        (
            &["FROM console.bi_board FINAL WHERE is_deleted = 0 AND id='b1'"],
            vec![untagged_board("[]")],
        ),
    ])
    .await;
    boards_update(
        State(state_for(&server)),
        Extension(person(ME, "dashboard:write")),
        json!({ "id": "b1", "layout": { "c1": { "x": 3, "y": 0, "w": 6, "h": 4 } } })
            .to_string()
            .into(),
    )
    .await
    .done("saved");
    let written = writes(&server).await;
    let version = written
        .iter()
        .find(|s| s.contains("INTO console.bi_board_version"))
        .expect("a version");
    assert!(version.contains("VALUES ('b1', 2,"), "{version}");
}

/// R9: deleting a chart closes the space its tile leaves.
#[tokio::test]
async fn deleting_a_chart_closes_the_space_its_tile_leaves() {
    let mut board = board_row("b1", "");
    board["layout_json"] =
        json!(r#"{"map":{"x":0,"y":0,"w":12,"h":6},"kpi":{"x":0,"y":6,"w":6,"h":4}}"#);
    let server = engine(vec![
        (
            &["SELECT is_deleted, board FROM console.bi_chart"],
            vec![json!({ "is_deleted": 0, "board": "b1" })],
        ),
        (
            &["SELECT is_deleted, owner_id FROM console.bi_board"],
            vec![state_row(0, "")],
        ),
        (
            &["FROM console.bi_board FINAL WHERE is_deleted = 0 AND id='b1'"],
            vec![board],
        ),
    ])
    .await;
    specs_delete(
        State(state_for(&server)),
        Extension(person(ME, "dashboard:write")),
        Query(serde_json::from_value(json!({ "id": "map" })).unwrap()),
    )
    .await
    .done("deleted");
    let written = writes(&server).await;
    let saved = last_board_write(&written);
    assert!(
        saved.contains(r#""kpi":{"x":0,"y":0,"w":6,"h":4}"#),
        "{saved}"
    );
    assert!(!saved.contains(r#""map":"#), "{saved}");
}

/// R10: a chart deleted on its own while its dashboard is in the trash is
/// refused, with a 409 and the sentence the entry shows; nothing is written.
#[tokio::test]
async fn restoring_a_chart_whose_dashboard_is_in_the_trash_is_a_conflict() {
    let server = engine(vec![
        (
            &["SELECT id, board FROM console.bi_chart FINAL WHERE is_deleted = 2"],
            vec![json!({ "id": "c1", "board": "b1" })],
        ),
        (
            &["SELECT is_deleted FROM console.bi_board"],
            vec![json!({ "is_deleted": 2 })],
        ),
    ])
    .await;
    let outcome = trash_restore(
        State(state_for(&server)),
        Extension(person(ME, "dashboard:write")),
        json!({ "kind": "chart", "id": "c1" }).to_string().into(),
    )
    .await;
    let response = axum::response::IntoResponse::into_response(match outcome {
        Err(rejection) => rejection,
        Ok(_) => panic!("must be refused"),
    });
    assert_eq!(response.status(), axum::http::StatusCode::CONFLICT);
    let body = text_of(response).await;
    assert_eq!(
        body["error"],
        "Its dashboard is in the trash too: restore the dashboard first."
    );
    assert!(writes(&server).await.is_empty());
}

/// R11: the payload's board list says which dashboard is private, so the page
/// can show it; a shared dashboard has no `ownerId` key.
#[tokio::test]
async fn the_payload_names_the_owner_of_a_private_dashboard_and_only_that() {
    let server = engine(vec![(
        &["FROM console.bi_board FINAL WHERE is_deleted = 0"],
        vec![board_row("b_shared", ""), board_row("b_draft", &id_of(ME))],
    )])
    .await;
    let body = payload(&server, &Viewer::user(id_of(ME), false), "b_draft").await;
    let boards = body["boards"].as_array().unwrap();
    let find = |id: &str| boards.iter().find(|b| b["id"] == id).unwrap();
    assert_eq!(find("b_draft")["ownerId"], json!(id_of(ME)));
    assert!(find("b_shared").get("ownerId").is_none());
}

// ── The real engine (scratch database) ─────────────────────────────────
//
// `BI18C2_ENGINE=http://127.0.0.1:8123 cargo test -p lakehouse-api --lib
// the_trash_the_personal_space -- --ignored --nocapture`
//
// Every statement the store sends is forwarded to a real `ClickHouse` with
// `console.` rewritten to a scratch database created for the run and dropped
// at the end, so the clauses above are proven to MEAN what the tests assume
// (`NOT IN (sub-select FINAL)`, `INSERT ... SELECT` from the same table,
// `ReplacingMergeTree` keeping the newest row of an id). Nothing in `console`
// is touched: a statement that still names it after the rewrite, or a write
// that does not name the scratch database, stops the run. It must be run
// alone (the store creates its tables once per process).
mod engine {
    use std::process::{Command, Stdio};
    use std::sync::OnceLock;

    use lakehouse_bi::access::Viewer;
    use lakehouse_bi::specs::ChartSource;
    use lakehouse_bi::store::{self, Board, ChartInput};
    use lakehouse_bi::{folders, items, search, trash, versions};
    use serde_json::json;
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

    use super::*;

    static DB: OnceLock<String> = OnceLock::new();

    fn db() -> &'static str {
        DB.get_or_init(|| format!("bi_p2_check_{:x}", std::process::id()))
    }

    fn base() -> String {
        std::env::var("BI18C2_ENGINE").expect("BI18C2_ENGINE names the dev ClickHouse")
    }

    /// Send `sql` to the real engine as it is; the caller has made it safe.
    fn forward(sql: &str) -> (bool, Vec<u8>) {
        let mut child = Command::new("curl")
            .args(["-s", "-w", "\n%{http_code}", "--data-binary", "@-", &base()])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        std::io::Write::write_all(&mut child.stdin.take().unwrap(), sql.as_bytes()).unwrap();
        let out = child.wait_with_output().unwrap().stdout;
        let split = out.iter().rposition(|b| *b == b'\n').unwrap();
        let ok = &out[split + 1..] == b"200";
        (ok, out[..split].to_vec())
    }

    struct Scratch;

    impl Respond for Scratch {
        fn respond(&self, request: &Request) -> ResponseTemplate {
            let sql = String::from_utf8_lossy(&request.body)
                .replace(
                    "CREATE DATABASE IF NOT EXISTS console",
                    &format!("CREATE DATABASE IF NOT EXISTS {}", db()),
                )
                .replace("console.", &format!("{}.", db()));
            let head = sql.trim_start().to_ascii_uppercase();
            let writes = ["INSERT", "ALTER", "CREATE", "DROP"]
                .iter()
                .any(|w| head.starts_with(w));
            assert!(
                !sql.contains("console."),
                "a statement still names console: {sql}"
            );
            if head.starts_with("CREATE DATABASE") {
                assert!(sql.contains(db()), "{sql}");
            }
            if writes {
                assert!(
                    sql.contains(db()),
                    "a write outside the scratch database: {sql}"
                );
            }
            let (ok, body) = forward(&sql);
            ResponseTemplate::new(if ok { 200 } else { 500 })
                .insert_header("content-type", "application/json")
                .set_body_bytes(body)
        }
    }

    async fn mounted() -> (MockServer, lakehouse_clickhouse::ChClient) {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(Scratch)
            .mount(&server)
            .await;
        let ch = client(&server);
        (server, ch)
    }

    fn chart_input(board: &str, title: &str, dimension: &str) -> ChartInput {
        serde_json::from_value(json!({
            "title": title, "mart": "mart_demo_province_capacity", "kind": "bar",
            "dimension": dimension, "measures": ["hotels"], "board": board,
        }))
        .unwrap()
    }

    async fn add_chart(ch: &lakehouse_clickhouse::ChClient, board: &str, title: &str) -> String {
        let spec = store::spec_from_input(
            ch,
            &chart_input(board, title, "provinsi"),
            ChartSource::Ui,
            "ui",
            None,
            &lakehouse_bi::grain::TimeContext::default(),
        )
        .await
        .expect("a chart on a real mart");
        store::insert_chart(ch, &spec).await.expect("stored");
        spec.spec.id
    }

    async fn board_ids(ch: &lakehouse_clickhouse::ChClient, who: &Viewer) -> Vec<String> {
        store::list_boards(ch, who)
            .await
            .unwrap()
            .into_iter()
            .map(|b: Board| b.id)
            .collect()
    }

    async fn chart_ids(ch: &lakehouse_clickhouse::ChClient, who: &Viewer) -> Vec<String> {
        store::list_stored_charts(ch, who)
            .await
            .unwrap()
            .into_iter()
            .map(|c| c.spec.id)
            .collect()
    }

    async fn trash_names(ch: &lakehouse_clickhouse::ChClient, who: &Viewer) -> Vec<String> {
        trash::list_trash(ch, who)
            .await
            .unwrap()
            .into_iter()
            .map(|t| t.name)
            .collect()
    }

    fn author(name: &str) -> versions::Author {
        versions::Author {
            id: format!("id-{name}"),
            name: name.to_owned(),
        }
    }

    #[tokio::test]
    #[ignore = "needs the dev ClickHouse (BI18C2_ENGINE); see the handoff of part 2"]
    #[allow(
        clippy::too_many_lines,
        reason = "one scripted scenario against a real engine: each stage uses the rows the one before made, so splitting it would repeat the setup"
    )]
    async fn the_trash_the_personal_space_versions_favourites_and_search_on_a_real_engine() {
        let (_server, ch) = mounted().await;
        let (me, you) = (Viewer::user("u_me", false), Viewer::user("u_you", false));
        let (admin, system, nobody) = (
            Viewer::user("u_admin", true),
            Viewer::system(),
            Viewer::anonymous(),
        );

        // ── Setup: a shared dashboard with a chart, a draft in Me's space ──
        let shared = store::create_board(&ch, "Shared overview", "", "Me")
            .await
            .unwrap();
        let c_shared = add_chart(&ch, &shared.id, "Hotels in the shared board").await;
        let mine = folders::ensure_personal_folder(&ch, "u_me", "Me")
            .await
            .unwrap();
        assert_eq!(mine.id, folders::personal_folder_id("u_me"));
        // A second first use is the same folder, not a second one.
        let again = folders::ensure_personal_folder(&ch, "u_me", "Me")
            .await
            .unwrap();
        assert_eq!(again.id, mine.id);
        let draft = store::create_board(&ch, "Secret draft", "", "Me")
            .await
            .unwrap();
        store::move_board(&ch, &draft.id, &mine.id).await.unwrap();
        let c_draft = add_chart(&ch, &draft.id, "Hotels in the draft").await;

        // ── The personal space: who sees what ────────────────────────────
        let ids = |v: Vec<String>| v;
        assert!(ids(board_ids(&ch, &me).await).contains(&draft.id));
        assert!(!board_ids(&ch, &you).await.contains(&draft.id));
        assert!(!board_ids(&ch, &nobody).await.contains(&draft.id));
        assert!(board_ids(&ch, &admin).await.contains(&draft.id));
        assert!(board_ids(&ch, &system).await.contains(&draft.id));
        assert!(
            store::get_board(&ch, &draft.id, &you)
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            store::get_board(&ch, &draft.id, &me)
                .await
                .unwrap()
                .is_some()
        );
        assert_eq!(
            store::get_board(&ch, &draft.id, &me)
                .await
                .unwrap()
                .unwrap()
                .owner_id
                .as_deref(),
            Some("u_me")
        );
        let (cs_you, cs_me) = (chart_ids(&ch, &you).await, chart_ids(&ch, &me).await);
        assert!(
            cs_you.contains(&c_shared) && !cs_you.contains(&c_draft),
            "{cs_you:?}"
        );
        assert!(cs_me.contains(&c_draft) && cs_me.contains(&c_shared));
        assert!(!chart_ids(&ch, &nobody).await.contains(&c_draft));
        assert!(chart_ids(&ch, &admin).await.contains(&c_draft));
        // Folders: another user's space is not even listed.
        let seen = |v: Vec<folders::Folder>| v.into_iter().map(|f| f.id).collect::<Vec<_>>();
        assert!(seen(folders::list_folders(&ch, &me).await.unwrap()).contains(&mine.id));
        assert!(!seen(folders::list_folders(&ch, &you).await.unwrap()).contains(&mine.id));
        assert!(seen(folders::list_folders(&ch, &admin).await.unwrap()).contains(&mine.id));
        // Search follows the same rule, in the statement.
        let hit_ids = |hits: Vec<items::Entry>| hits.into_iter().map(|h| h.id).collect::<Vec<_>>();
        let all_hotels = hit_ids(search::search(&ch, &me, "hotels in the").await.unwrap());
        assert!(
            all_hotels.contains(&c_draft) && all_hotels.contains(&c_shared),
            "{all_hotels:?}"
        );
        let for_you = hit_ids(search::search(&ch, &you, "HOTELS IN THE").await.unwrap());
        assert!(
            for_you.contains(&c_shared) && !for_you.contains(&c_draft),
            "{for_you:?}"
        );
        let boards_you = hit_ids(search::search(&ch, &you, "secret").await.unwrap());
        assert!(boards_you.is_empty(), "{boards_you:?}");
        assert!(hit_ids(search::search(&ch, &me, "secret").await.unwrap()).contains(&draft.id));
        // `%` and `_` are ordinary characters.
        assert!(search::search(&ch, &me, "%").await.unwrap().is_empty());
        assert!(search::search(&ch, &me, "a_c").await.unwrap().is_empty());
        // A draft cannot be shared, and a shared dashboard cannot be hidden by moving it.
        let refused = store::set_board_public(&ch, &draft.id, true)
            .await
            .unwrap_err();
        assert!(refused.to_string().contains("personal space"), "{refused}");
        let token = store::set_board_public(&ch, &shared.id, true)
            .await
            .unwrap();
        assert!(
            store::get_board_by_token(&ch, &token)
                .await
                .unwrap()
                .is_some()
        );
        let refused = store::move_board(&ch, &shared.id, &mine.id)
            .await
            .unwrap_err();
        assert!(
            refused.to_string().contains("Turn off sharing"),
            "{refused}"
        );
        // Filing it out of the space makes it everyone's.
        store::move_board(&ch, &draft.id, "").await.unwrap();
        assert!(board_ids(&ch, &you).await.contains(&draft.id));
        assert!(chart_ids(&ch, &you).await.contains(&c_draft));
        store::move_board(&ch, &draft.id, &mine.id).await.unwrap();
        assert!(!chart_ids(&ch, &you).await.contains(&c_draft));

        // ── The trash: nothing read leaves a deleted item in ─────────────
        store::delete_board(&ch, &shared.id, "Me").await.unwrap();
        assert!(!board_ids(&ch, &you).await.contains(&shared.id));
        assert!(!board_ids(&ch, &system).await.contains(&shared.id));
        assert!(!chart_ids(&ch, &system).await.contains(&c_shared));
        assert!(
            store::get_board(&ch, &shared.id, &system)
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            store::get_board_by_token(&ch, &token)
                .await
                .unwrap()
                .is_none(),
            "a deleted board's public link is dead"
        );
        assert!(
            !hit_ids(search::search(&ch, &you, "Shared overview").await.unwrap())
                .contains(&shared.id)
        );
        assert!(
            !hit_ids(search::search(&ch, &you, "shared board").await.unwrap()).contains(&c_shared)
        );
        assert!(versions::capture(&ch, &shared.id).await.unwrap().is_none());
        let listed = trash::list_trash(&ch, &you).await.unwrap();
        let entry = listed
            .iter()
            .find(|t| t.id == shared.id)
            .expect("in the trash");
        assert_eq!((entry.kind, entry.deleted_by.as_str()), ("board", "Me"));
        assert!(
            !listed.iter().any(|t| t.id == c_shared),
            "its chart is part of its entry"
        );
        // A deleted private chart is the owner's to see (and an administrator's).
        store::delete_chart(&ch, &c_draft, "Me").await.unwrap();
        assert!(
            trash_names(&ch, &you)
                .await
                .iter()
                .all(|n| n != "Hotels in the draft")
        );
        assert!(
            trash_names(&ch, &me)
                .await
                .contains(&"Hotels in the draft".to_owned())
        );
        assert!(
            trash_names(&ch, &admin)
                .await
                .contains(&"Hotels in the draft".to_owned())
        );
        let refusal = trash::restore_chart(&ch, &c_draft, &you).await.unwrap_err();
        assert!(
            refusal.to_string().contains("not in the trash"),
            "{refusal}"
        );
        // Restore: the board with exactly its charts.
        let back = trash::restore_board(&ch, &shared.id, &you).await.unwrap();
        assert_eq!(back.charts, 1);
        assert!(board_ids(&ch, &you).await.contains(&shared.id));
        assert!(chart_ids(&ch, &you).await.contains(&c_shared));
        assert!(
            store::get_board_by_token(&ch, &token)
                .await
                .unwrap()
                .is_some()
        );
        assert!(
            trash::restore_board(&ch, &shared.id, &you).await.is_err(),
            "twice is refused"
        );
        // R10: a chart deleted on its own while its dashboard is in the trash
        // too is not restored before the dashboard, and the dashboard's restore
        // does not bring it back.
        let c_wait = add_chart(&ch, &shared.id, "Waits for the board").await;
        store::delete_chart(&ch, &c_wait, "Me").await.unwrap();
        store::delete_board(&ch, &shared.id, "Me").await.unwrap();
        let refusal = trash::restore_chart(&ch, &c_wait, &you).await.unwrap_err();
        assert_eq!(refusal.to_string(), trash::DASHBOARD_IN_TRASH);
        let listed = trash::list_trash(&ch, &you).await.unwrap();
        let entry = listed
            .iter()
            .find(|t| t.id == c_wait)
            .expect("its own entry");
        assert_eq!(entry.board_state, "trash");
        let back = trash::restore_board(&ch, &shared.id, &you).await.unwrap();
        assert_eq!(back.charts, 1, "only the chart the board's delete took");
        assert!(
            !chart_ids(&ch, &you).await.contains(&c_wait),
            "deleted on its own: still in the trash"
        );
        let restored = trash::restore_chart(&ch, &c_wait, &you).await.unwrap();
        assert_eq!(restored.board_state, "live");
        assert!(chart_ids(&ch, &you).await.contains(&c_wait));
        // Deleting the draft's chart earlier must not come back with the draft.
        store::delete_board(&ch, &draft.id, "Me").await.unwrap();
        assert!(
            !trash_names(&ch, &you)
                .await
                .contains(&"Secret draft".to_owned())
        );
        assert!(
            trash_names(&ch, &me)
                .await
                .contains(&"Secret draft".to_owned())
        );
        assert!(trash::restore_board(&ch, &draft.id, &you).await.is_err());
        trash::restore_board(&ch, &draft.id, &me).await.unwrap();
        assert!(
            !chart_ids(&ch, &me).await.contains(&c_draft),
            "deleted on its own: still in the trash"
        );
        trash::restore_chart(&ch, &c_draft, &me).await.unwrap();
        assert!(chart_ids(&ch, &me).await.contains(&c_draft));
        assert!(
            !chart_ids(&ch, &you).await.contains(&c_draft),
            "restored, still private"
        );

        // ── The sweep ────────────────────────────────────────────────────
        let old = store::create_board(&ch, "Old and gone", "", "Me")
            .await
            .unwrap();
        let c_old = add_chart(&ch, &old.id, "Old chart").await;
        assert!(
            versions::record(&ch, &old.id, &author("Me"), None)
                .await
                .unwrap()
                .is_some()
        );
        store::delete_board(&ch, &old.id, "Me").await.unwrap();
        // Nothing is due yet.
        assert_eq!(
            trash::purge_expired(&ch).await.unwrap(),
            trash::Purged::default()
        );
        // Date the entries 40 days ago.
        for table in ["bi_board", "bi_chart"] {
            let cols = if table == "bi_board" {
                "id, name, description, created_by, layout_json, filters_json, public_token, embed_enabled, folder_id, embed_revoked_before, embed_revoked_jti_json, embed_origins_json, refresh_seconds, grain, params_json, tabs_json, phone_layout_json, owner_id, deleted_by"
            } else {
                "id, title, spec_json, board, created_by, deleted_by, deleted_with"
            };
            let (ok, body) = forward(&format!(
                "INSERT INTO {db}.{table} ({cols}, is_deleted, deleted_at) \
                 SELECT {cols}, 2, now() - INTERVAL 40 DAY FROM {db}.{table} FINAL \
                 WHERE is_deleted = 2 AND (id = '{b}' OR id = '{c}')",
                db = db(),
                b = old.id,
                c = c_old
            ));
            assert!(ok, "{}", String::from_utf8_lossy(&body));
        }
        let purged = trash::purge_expired(&ch).await.unwrap();
        assert_eq!((purged.boards, purged.charts), (1, 1));
        assert!(
            !trash_names(&ch, &system)
                .await
                .contains(&"Old and gone".to_owned())
        );
        assert!(trash::restore_board(&ch, &old.id, &system).await.is_err());
        assert!(
            versions::list(&ch, &old.id).await.unwrap().is_empty(),
            "history goes with the board"
        );
        // What was deleted a day ago is untouched by the sweep.
        store::delete_board(&ch, &draft.id, "Me").await.unwrap();
        assert_eq!(
            trash::purge_expired(&ch).await.unwrap(),
            trash::Purged::default()
        );
        trash::restore_board(&ch, &draft.id, &me).await.unwrap();

        // ── Version history ──────────────────────────────────────────────
        let hist = store::create_board(&ch, "History", "", "Me").await.unwrap();
        assert_eq!(
            versions::record(&ch, &hist.id, &author("Ann"), None)
                .await
                .unwrap(),
            Some(1)
        );
        assert_eq!(
            versions::record(&ch, &hist.id, &author("Ann"), None)
                .await
                .unwrap(),
            None,
            "unchanged: no version"
        );
        store::rename_board(&ch, &hist.id, "History two")
            .await
            .unwrap();
        assert_eq!(
            versions::record(&ch, &hist.id, &author("Bob"), None)
                .await
                .unwrap(),
            Some(2)
        );
        let h_chart = add_chart(&ch, &hist.id, "Hotels history").await;
        assert_eq!(
            versions::record(&ch, &hist.id, &author("Bob"), None)
                .await
                .unwrap(),
            Some(3)
        );
        let listed = versions::list(&ch, &hist.id).await.unwrap();
        assert_eq!(
            listed
                .iter()
                .map(|v| (v.version, v.author.as_str(), v.summary.as_str()))
                .collect::<Vec<_>>(),
            vec![
                (3, "Bob", "1 chart added"),
                (2, "Bob", "Renamed"),
                (1, "Ann", "Created")
            ]
        );
        for n in 4..=19 {
            store::rename_board(&ch, &hist.id, &format!("History {n}"))
                .await
                .unwrap();
            versions::record(&ch, &hist.id, &author("Bob"), None)
                .await
                .unwrap();
        }
        let kept = versions::list(&ch, &hist.id).await.unwrap();
        assert_eq!(kept.len(), versions::KEEP);
        assert_eq!((kept[0].version, kept[kept.len() - 1].version), (19, 5));
        assert!(
            versions::load(&ch, &hist.id, 3).await.unwrap().is_none(),
            "pruned"
        );
        assert!(versions::load(&ch, &hist.id, 19).await.unwrap().is_some());
        // R8: a version written with a label lists under it, not under the diff.
        store::rename_board(&ch, &hist.id, "History 20")
            .await
            .unwrap();
        assert_eq!(
            versions::record(&ch, &hist.id, &author("Bob"), Some("Restored v7"))
                .await
                .unwrap(),
            Some(20)
        );
        assert_eq!(
            versions::list(&ch, &hist.id).await.unwrap()[0].summary,
            "Restored v7"
        );
        // A version holds content, never sharing, folder or owner.
        let state = versions::load(&ch, &hist.id, 19).await.unwrap().unwrap();
        assert_eq!(state.charts.len(), 1);
        assert_eq!(state.charts[0].id, h_chart);

        // ── Favourites and recent ────────────────────────────────────────
        items::set_favourite(&ch, &me, items::Kind::Board, &shared.id, true)
            .await
            .unwrap();
        items::set_favourite(&ch, &me, items::Kind::Chart, &c_draft, true)
            .await
            .unwrap();
        assert!(
            items::set_favourite(&ch, &you, items::Kind::Board, &draft.id, true)
                .await
                .is_err(),
            "no probing for a private id"
        );
        assert!(
            items::set_favourite(&ch, &you, items::Kind::Chart, &c_draft, true)
                .await
                .is_err()
        );
        assert!(
            items::set_favourite(&ch, &you, items::Kind::Board, "b_nope", true)
                .await
                .is_err()
        );
        let favs = items::entries(&ch, &me, items::List::Favourite)
            .await
            .unwrap();
        assert_eq!(favs.len(), 2, "{favs:?}");
        assert!(
            items::entries(&ch, &you, items::List::Favourite)
                .await
                .unwrap()
                .is_empty(),
            "per user"
        );
        items::touch_recent(&ch, &me, items::Kind::Board, &hist.id)
            .await
            .unwrap();
        items::touch_recent(&ch, &me, items::Kind::Board, &shared.id)
            .await
            .unwrap();
        let recent = items::entries(&ch, &me, items::List::Recent).await.unwrap();
        assert_eq!(recent[0].id, shared.id, "newest first");
        // A deleted item drops out of both lists; a restored one is back.
        store::delete_board(&ch, &shared.id, "Me").await.unwrap();
        let favs = items::entries(&ch, &me, items::List::Favourite)
            .await
            .unwrap();
        assert!(favs.iter().all(|f| f.id != shared.id), "{favs:?}");
        assert!(
            items::entries(&ch, &me, items::List::Recent)
                .await
                .unwrap()
                .iter()
                .all(|f| f.id != shared.id)
        );
        trash::restore_board(&ch, &shared.id, &me).await.unwrap();
        assert!(
            items::entries(&ch, &me, items::List::Favourite)
                .await
                .unwrap()
                .iter()
                .any(|f| f.id == shared.id)
        );
        items::set_favourite(&ch, &me, items::Kind::Board, &shared.id, false)
            .await
            .unwrap();
        assert!(
            items::entries(&ch, &me, items::List::Favourite)
                .await
                .unwrap()
                .iter()
                .all(|f| f.id != shared.id)
        );

        let (_, _) = forward(&format!("DROP DATABASE IF EXISTS {}", db()));
        println!("the real engine agrees with every clause");
    }
}
