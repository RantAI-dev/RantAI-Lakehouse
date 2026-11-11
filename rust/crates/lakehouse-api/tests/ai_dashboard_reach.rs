//! The D9 gate through the real router (`BI-37` T9, D10): a person who holds
//! only a rule on a dashboard — no `dashboard:read`, no `dashboard:write` — is
//! judged by their level on the dashboard the call acts on, is offered the
//! listed tools, and is still refused everywhere else. An approved `delete_chart`
//! runs for an approver who holds manage on that dashboard through a rule and
//! not for one who holds nothing.
//!
//! Postgres holds the rules (a per-test database); a `wiremock` stand-in plays
//! `ClickHouse` and answers the dashboard lookups, and another plays the model.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::collections::HashMap;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use common::{
    TestApp, create_assistant_principal, create_principal_with_permissions,
    session_cookie_for_user, spin_up_with_env,
};
use lakehouse_store::dashboard_access::{NewRule, replace_item};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request as MockRequest, Respond, ResponseTemplate};

/// The dashboards the stand-in holds: `(id, folder)`.
const BOARDS: [(&str, &str); 2] = [("b1", ""), ("b2", "")];

struct Store;

fn chart_row(id: &str, board: &str) -> Value {
    // The same shape `tests/ai_export_tools.rs` uses, which the store parses.
    let spec = json!({
        "id": id, "title": format!("Chart {id}"), "kind": "bar", "mart": "mart_x",
        "sql": "SELECT 1", "x": "region", "y": "n"
    });
    let def = json!({ "title": format!("Chart {id}"), "mart": "mart_x", "kind": "bar" });
    json!({
        "id": id, "board": board, "created_by": "ui", "created_at": "2026-01-01 00:00:00",
        "spec_json": json!({ "spec": spec, "def": def, "hasYear": false }).to_string(),
    })
}

impl Respond for Store {
    fn respond(&self, request: &MockRequest) -> ResponseTemplate {
        let sql = String::from_utf8_lossy(&request.body).into_owned();
        let rows = |data: Vec<Value>| {
            let n = data.len();
            ResponseTemplate::new(200).set_body_json(json!({ "meta": [], "data": data, "rows": n }))
        };
        // The statements are told apart by their own SELECT list: a viewer
        // clause may itself name `console.bi_board`, so a bare table name would
        // answer the wrong statement.
        // The chart's own row and the state of the dashboards, which the chart
        // tools read before they change or move a chart: `c1` is on `b1`, `c2`
        // on `b2`, and both dashboards are live and unowned.
        if sql.contains("SELECT is_deleted, board FROM console.bi_chart") {
            let board = if sql.contains("id = 'c2'") {
                "b2"
            } else {
                "b1"
            };
            return rows(vec![json!({ "is_deleted": 0, "board": board })]);
        }
        if sql.contains("SELECT is_deleted, owner_id FROM console.bi_board") {
            return rows(vec![json!({ "is_deleted": 0, "owner_id": "" })]);
        }
        if sql.contains("SELECT id, spec_json, board") {
            return rows(vec![chart_row("c1", "b1"), chart_row("c2", "b2")]);
        }
        if sql.contains("SELECT id, name, description") {
            let wanted = sql.find("AND id='").map(|at| {
                let rest = &sql[at + "AND id='".len()..];
                rest[..rest.find('\'').unwrap_or(rest.len())].to_owned()
            });
            return rows(
                BOARDS
                    .iter()
                    .filter(|(id, _)| wanted.as_ref().is_none_or(|w| w == id))
                    .map(|(id, folder)| {
                        json!({ "id": id, "name": format!("Board {id}"), "folder_id": folder, "owner_id": "" })
                    })
                    .collect(),
            );
        }
        if sql.contains("SELECT id, folder_id, owner_id FROM console.bi_board") {
            return rows(
                BOARDS
                    .iter()
                    .map(|(id, folder)| json!({ "id": id, "folder_id": folder, "owner_id": "" }))
                    .collect(),
            );
        }
        // Folders, a board's embed events and every other read: nothing.
        rows(vec![])
    }
}

async fn app() -> (TestApp, MockServer, MockServer) {
    let ch = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(Store)
        .mount(&ch)
        .await;
    let llm = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{ "message": { "role": "assistant", "content": "Here you go." } }],
        })))
        .mount(&llm)
        .await;
    let overrides: HashMap<String, String> = HashMap::from([
        ("CH_URL".to_owned(), ch.uri()),
        ("LLM_URL".to_owned(), llm.uri()),
        ("LLM_API_KEY".to_owned(), "test-llm-key-not-real".to_owned()),
    ]);
    (spin_up_with_env(&overrides).await, ch, llm)
}

fn view(user: Uuid) -> NewRule {
    NewRule {
        user_id: Some(user),
        role_id: None,
        level: "view".to_owned(),
    }
}

fn edit(user: Uuid) -> NewRule {
    NewRule {
        user_id: Some(user),
        role_id: None,
        level: "edit".to_owned(),
    }
}

fn manage(user: Uuid) -> NewRule {
    NewRule {
        user_id: Some(user),
        role_id: None,
        level: "manage".to_owned(),
    }
}

async fn call(app: &TestApp, cookie: &str, tool: &str, args: Value, mode: &str) -> Value {
    let resp = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/ai/tool")
                .header("content-type", "application/json")
                .header("cookie", cookie)
                .body(Body::from(
                    json!({ "tool": tool, "args": args, "mode": mode }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

/// Whether the gate let the call through: the refusal shape carries
/// `refused: true`, whatever the tool made of the call afterwards.
fn gate_passed(body: &Value) -> bool {
    body["result"]["refused"] != json!(true)
}

/// A person who holds only a rule on dashboard `b1`: view lets them read it,
/// edit lets them change it, and their own level stands in for the global
/// dashboard permission.
#[tokio::test]
async fn a_person_with_only_a_rule_is_judged_by_their_level_on_the_dashboard() {
    let (app, _ch, _llm) = app().await;
    let analyst = create_assistant_principal(&app.pool, "query:read").await;
    replace_item(&app.pool, "board", "b1", &[view(analyst)], false, None)
        .await
        .unwrap();
    let cookie = session_cookie_for_user(&app.pool, analyst).await;

    // A listing with no board argument: their best level anywhere offers it.
    let listed = call(&app, &cookie, "list_charts", json!({}), "ask").await;
    assert_eq!(listed["outcome"], "executed", "{listed}");

    // A chart on the dashboard they hold view on.
    let chart = call(
        &app,
        &cookie,
        "export_chart",
        json!({ "chart": "c1", "format": "csv" }),
        "ask",
    )
    .await;
    assert_eq!(chart["outcome"], "executed", "{chart}");
    assert!(chart["result"]["url"].is_string(), "{chart}");

    // A chart on a dashboard they hold nothing on: the gate refuses it, so the
    // tool never reads it.
    let elsewhere = call(
        &app,
        &cookie,
        "export_chart",
        json!({ "chart": "c2", "format": "csv" }),
        "ask",
    )
    .await;
    assert_eq!(elsewhere["outcome"], "refused", "{elsewhere}");
    assert_eq!(elsewhere["result"]["required"], "dashboard:read");

    // View is not edit: creating a chart on `b1` is refused.
    let write = call(
        &app,
        &cookie,
        "create_chart",
        json!({ "title": "T", "kind": "bar", "mart": "mart_x", "board": "b1", "confirmed": true }),
        "build",
    )
    .await;
    assert_eq!(write["outcome"], "refused", "{write}");
    assert_eq!(write["result"]["required"], "dashboard:write");

    // Nothing else is widened: `run_sql` still needs `query:read`, which this
    // person does not hold (the same principal has it, so a second one).
    let nobody = create_assistant_principal(&app.pool, "").await;
    let nobody_cookie = session_cookie_for_user(&app.pool, nobody).await;
    let sql = call(
        &app,
        &nobody_cookie,
        "run_sql",
        json!({ "sql": "SELECT 1" }),
        "ask",
    )
    .await;
    assert_eq!(sql["outcome"], "refused", "{sql}");
    assert_eq!(sql["result"]["required"], "query:read");
}

/// Edit on the dashboard is what changing it needs, and the handler's own
/// checks still run: the gate is a floor, not the whole guard.
#[tokio::test]
async fn edit_on_the_dashboard_lets_the_write_tools_reach_their_handler() {
    let (app, _ch, _llm) = app().await;
    let editor = create_assistant_principal(&app.pool, "").await;
    replace_item(&app.pool, "board", "b1", &[edit(editor)], false, None)
        .await
        .unwrap();
    let cookie = session_cookie_for_user(&app.pool, editor).await;
    let body = call(
        &app,
        &cookie,
        "create_chart",
        json!({ "title": "T", "kind": "bar", "mart": "mart_x", "board": "b1", "confirmed": true }),
        "build",
    )
    .await;
    assert!(
        gate_passed(&body),
        "edit on b1 is what the call needs, so the gate must let it through: {body}"
    );
    // And a dashboard they hold nothing on is refused.
    let elsewhere = call(
        &app,
        &cookie,
        "create_chart",
        json!({ "title": "T2", "kind": "bar", "mart": "mart_x", "board": "b2", "confirmed": true }),
        "build",
    )
    .await;
    assert_eq!(elsewhere["outcome"], "refused", "{elsewhere}");
}

/// A person who may edit the dashboard a chart is on and only view the one the
/// update moves it to: the draft is refused with the sentence the save gives,
/// and no Confirm card is offered. The dashboard is one the caller sees, so
/// `require_chart_board` lets it pass and only the `edit` check on the target
/// refuses it (`PR #391 review`, `PR #402 review`).
#[tokio::test]
async fn the_draft_of_an_update_that_moves_a_chart_onto_a_view_only_dashboard_is_refused_as_the_save_is()
 {
    let (app, _ch, _llm) = app().await;
    let editor = create_assistant_principal(&app.pool, "").await;
    replace_item(&app.pool, "board", "b1", &[edit(editor)], false, None)
        .await
        .unwrap();
    replace_item(&app.pool, "board", "b2", &[view(editor)], false, None)
        .await
        .unwrap();
    let cookie = session_cookie_for_user(&app.pool, editor).await;
    let mut args = json!({
        "id": "c1", "title": "T", "kind": "bar", "mart": "mart_x",
        "dimension": "region", "measures": ["n"], "board": "b2",
    });

    let draft = call(&app, &cookie, "update_chart", args.clone(), "build").await;
    args["confirmed"] = json!(true);
    let saved = call(&app, &cookie, "update_chart", args, "build").await;

    assert_eq!(saved["outcome"], "failed", "{saved}");
    assert!(
        saved["result"]["error"]
            .as_str()
            .is_some_and(|e| e.contains("edit on this dashboard")),
        "{saved}"
    );
    assert_eq!(draft["outcome"], "failed", "{draft}");
    assert!(
        draft["result"].get("needs_confirmation").is_none(),
        "{draft}"
    );
    assert_eq!(draft["result"], saved["result"], "one reply for both");
}

/// The offered list: a chat with a grant-only person carries the listed tools
/// their level covers, and not a tool their permissions still refuse.
#[tokio::test]
async fn a_chat_offers_the_listed_tools_to_a_person_who_holds_only_a_rule() {
    let (app, _ch, llm) = app().await;
    let analyst = create_assistant_principal(&app.pool, "catalog:read").await;
    replace_item(&app.pool, "board", "b1", &[view(analyst)], false, None)
        .await
        .unwrap();
    let cookie = session_cookie_for_user(&app.pool, analyst).await;
    let resp = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/ai/chat")
                .header("content-type", "application/json")
                .header("cookie", cookie)
                .body(Body::from(
                    json!({
                        "mode": "ask",
                        "stream": false,
                        "messages": [{ "role": "user", "content": "Which charts are on this dashboard?" }],
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let asked = llm
        .received_requests()
        .await
        .unwrap()
        .first()
        .map(|r| String::from_utf8_lossy(&r.body).into_owned())
        .unwrap_or_default();
    assert!(
        asked.contains("\"name\":\"list_charts\""),
        "the listed tool is offered for the level: {asked}"
    );
    assert!(
        !asked.contains("\"name\":\"run_sql\""),
        "a tool the person's permissions refuse is not offered: {asked}"
    );
}

/// An approved `delete_chart`: the approver's own level on the dashboard is
/// what runs it, and an approver who holds nothing is refused and recorded.
#[tokio::test]
async fn an_approved_delete_chart_runs_for_an_approver_with_a_rule_on_that_dashboard() {
    let (app, _ch, _llm) = app().await;
    // The requester holds manage on b1 only, and names no one else.
    let requester = create_assistant_principal(&app.pool, "").await;
    replace_item(&app.pool, "board", "b1", &[manage(requester)], false, None)
        .await
        .unwrap();
    let cookie = session_cookie_for_user(&app.pool, requester).await;
    let asked = call(
        &app,
        &cookie,
        "delete_chart",
        json!({ "id": "c1", "confirmed": true }),
        "build",
    )
    .await;
    let approval_id = asked["result"]["approval_id"]
        .as_str()
        .unwrap_or_else(|| panic!("no approval: {asked}"))
        .to_owned();
    assert_eq!(asked["result"]["needs_approval"], json!(true), "{asked}");

    // An approver with manage on b1 through a rule, and no dashboard:write.
    let approver = create_principal_with_permissions(&app.pool, "agent:approve").await;
    replace_item(&app.pool, "board", "b1", &[manage(approver)], false, None)
        .await
        .unwrap();
    let approver_cookie = session_cookie_for_user(&app.pool, approver).await;
    let decided = decide(&app, &approver_cookie, &approval_id).await;
    assert_eq!(decided.0, StatusCode::OK, "{}", decided.1);
    assert_eq!(decided.1["executed"], json!(true), "{}", decided.1);
    assert!(decided.1.get("result").is_some(), "{}", decided.1);
}

#[tokio::test]
async fn an_approver_with_no_rule_is_refused_and_the_run_records_it() {
    let (app, _ch, _llm) = app().await;
    let requester = create_assistant_principal(&app.pool, "").await;
    replace_item(&app.pool, "board", "b1", &[manage(requester)], false, None)
        .await
        .unwrap();
    let cookie = session_cookie_for_user(&app.pool, requester).await;
    let asked = call(
        &app,
        &cookie,
        "delete_chart",
        json!({ "id": "c1", "confirmed": true }),
        "build",
    )
    .await;
    let approval_id = asked["result"]["approval_id"]
        .as_str()
        .unwrap_or_else(|| panic!("no approval: {asked}"))
        .to_owned();
    let run_id = asked["result"]["run_id"]
        .as_str()
        .unwrap_or_else(|| panic!("no run: {asked}"))
        .to_owned();

    // Only `agent:approve`, and no rule on any dashboard.
    let approver = create_principal_with_permissions(&app.pool, "agent:approve").await;
    let approver_cookie = session_cookie_for_user(&app.pool, approver).await;
    let decided = decide(&app, &approver_cookie, &approval_id).await;
    assert_eq!(decided.0, StatusCode::FORBIDDEN, "{}", decided.1);
    let run = lakehouse_store::agents::get_run(&app.pool, &run_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(run.status, "failed", "the run is recorded, not run");
}

async fn decide(app: &TestApp, cookie: &str, id: &str) -> (StatusCode, Value) {
    let resp = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/agents/approvals/{id}/decide"))
                .header("content-type", "application/json")
                .header("cookie", cookie)
                .body(Body::from(json!({ "decision": "approved" }).to_string()))
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
