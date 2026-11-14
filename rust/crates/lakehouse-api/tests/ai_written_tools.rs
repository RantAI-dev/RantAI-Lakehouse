//! The report and template tools through `POST /api/ai/tool` (`BI-18` part D,
//! T4, decisions R5 and T5, with `AI-9`): writing is a confirm, deleting an
//! approval, every tool needs `dashboard:write`, and every check is the
//! route's own (the 31st chart names the limit). Postgres holds the reports,
//! templates and approvals; a small `wiremock` stand-in plays `ClickHouse`
//! with one dashboard and its chart.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use crate::common;

use std::collections::HashMap;

use axum::body::{Body, to_bytes};
use axum::http::Request;
use common::{TestApp, create_assistant_principal, session_cookie_for_user, spin_up_with_env};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;
use wiremock::{Mock, MockServer, Request as MockRequest, Respond, ResponseTemplate};

struct Store;

fn quoted_after(sql: &str, marker: &str) -> Option<String> {
    let rest = &sql[sql.find(marker)? + marker.len()..];
    Some(rest[..rest.find('\'')?].to_owned())
}

impl Respond for Store {
    fn respond(&self, request: &MockRequest) -> ResponseTemplate {
        let sql = String::from_utf8_lossy(&request.body).into_owned();
        let rows = |data: Vec<Value>| {
            let n = data.len();
            ResponseTemplate::new(200).set_body_json(json!({ "meta": [], "data": data, "rows": n }))
        };
        if sql.starts_with("INSERT") || sql.starts_with("ALTER") || sql.starts_with("CREATE") {
            return ResponseTemplate::new(200).set_body_string("");
        }
        if sql.contains("FROM console.bi_chart") {
            let spec = json!({ "id": "c_one", "title": "Events", "kind": "kpi", "mart": "mart_event", "sql": "SELECT 1", "x": "", "y": "n" });
            let def = json!({ "title": "Events", "mart": "mart_event", "kind": "kpi", "dimension": "", "measures": ["n"] });
            return rows(vec![json!({
                "id": "c_one", "board": "b_one", "created_by": "ui", "created_at": "2026-01-01 00:00:00",
                "spec_json": json!({ "spec": spec, "def": def, "hasYear": false }).to_string(),
            })]);
        }
        if sql.contains("FROM console.bi_board") {
            let id = quoted_after(&sql, "id='").unwrap_or_else(|| "b_one".to_owned());
            return rows(vec![json!({
                "id": id, "name": format!("Board {id}"), "description": "", "created_by": "t",
                "layout_json": "", "filters_json": "", "public_token": "", "embed_enabled": "0",
                "folder_id": "", "created_at": "2026-10-10 00:00:00", "owner_id": "",
                "tabs_json": "", "embed_revoked_before": "0", "embed_revoked_jti_json": "[]",
                "embed_origins_json": "[]"
            })]);
        }
        rows(vec![])
    }
}

async fn app() -> (TestApp, MockServer) {
    let ch = MockServer::start().await;
    Mock::given(wiremock::matchers::method("POST"))
        .respond_with(Store)
        .mount(&ch)
        .await;
    let env = HashMap::from([("CH_URL".to_owned(), ch.uri())]);
    (spin_up_with_env(&env).await, ch)
}

async fn person(app: &TestApp, permissions: &str) -> String {
    let user = create_assistant_principal(&app.pool, permissions).await;
    session_cookie_for_user(&app.pool, user).await
}

async fn call(app: &TestApp, cookie: &str, tool: &str, args: Value) -> Value {
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
                    json!({ "tool": tool, "args": args, "mode": "build" }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

async fn reports(app: &TestApp) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM dashboard_report WHERE deleted_at IS NULL")
        .fetch_one(&app.pool)
        .await
        .unwrap()
}

async fn approvals_for(app: &TestApp, action: &str) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM approval_item WHERE action = $1")
        .bind(action)
        .fetch_one(&app.pool)
        .await
        .unwrap()
}

const WRITER: &str = "dashboard:read,dashboard:write";

/// `BI-18D-AC9`, first half: a short report with one chart, confirmed in the
/// chat, and the result names what it made and where it is.
#[tokio::test]
async fn create_report_waits_for_the_confirm_then_writes_through_the_route() {
    let (app, _ch) = app().await;
    let cookie = person(&app, WRITER).await;
    let args = json!({
        "title": "Events this quarter",
        "blocks": [
            { "type": "paragraph", "text": "Events are tracked here." },
            { "type": "chart", "chart": "c_one" },
        ],
    });
    let pending = call(&app, &cookie, "create_report", args.clone()).await;
    assert_eq!(pending["outcome"], "needs_confirmation", "{pending}");
    assert_eq!(
        reports(&app).await,
        0,
        "nothing is written before the confirm"
    );

    let mut confirmed = args;
    confirmed["confirmed"] = json!(true);
    let done = call(&app, &cookie, "create_report", confirmed).await;
    let result = &done["result"];
    assert_eq!(result["ok"], true, "{done}");
    assert_eq!(result["report"]["title"], "Events this quarter");
    assert_eq!(result["report"]["charts"], 1);
    let id = result["report"]["id"].as_str().unwrap().to_owned();
    assert_eq!(result["url"], format!("/dashboards/reports/{id}"));
    let board: String =
        sqlx::query_scalar("SELECT blocks->1->>'board' FROM dashboard_report WHERE id = $1::uuid")
            .bind(&id)
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert_eq!(
        board, "b_one",
        "the chart block names its dashboard, from the chart's row"
    );
}

/// "Add this chart to the report": `chart` appends one chart block, and the
/// route's limit applies to the result, with its sentence.
#[tokio::test]
async fn update_report_adds_a_chart_at_the_end_and_is_held_to_thirty_charts() {
    let (app, _ch) = app().await;
    let cookie = person(&app, WRITER).await;
    let made = call(
        &app,
        &cookie,
        "create_report",
        json!({ "title": "R", "blocks": [{ "type": "heading", "text": "One" }], "confirmed": true }),
    )
    .await;
    let id = made["result"]["report"]["id"].as_str().unwrap().to_owned();
    let added = call(
        &app,
        &cookie,
        "update_report",
        json!({ "id": id, "chart": "c_one", "confirmed": true }),
    )
    .await;
    assert_eq!(added["result"]["report"]["blocks"], 2, "{added}");
    assert_eq!(added["result"]["report"]["charts"], 1);

    let thirty: Vec<Value> = (0..30)
        .map(|_| json!({ "type": "chart", "chart": "c_one" }))
        .collect();
    let refused = call(
        &app,
        &cookie,
        "update_report",
        json!({ "id": id, "append": thirty, "confirmed": true }),
    )
    .await;
    assert_eq!(
        refused["result"]["error"], "A report holds at most 30 charts.",
        "{refused}"
    );
}

/// Deleting a report or a template waits for a human in Approvals; nothing is
/// deleted before.
#[tokio::test]
async fn deleting_a_report_or_a_template_waits_for_approval() {
    let (app, _ch) = app().await;
    let cookie = person(&app, WRITER).await;
    let made = call(
        &app,
        &cookie,
        "create_report",
        json!({ "title": "Keep me", "confirmed": true }),
    )
    .await;
    let id = made["result"]["report"]["id"].as_str().unwrap().to_owned();
    let pending = call(
        &app,
        &cookie,
        "delete_report",
        json!({ "id": id, "confirmed": true }),
    )
    .await;
    assert_eq!(pending["outcome"], "needs_approval", "{pending}");
    assert_eq!(approvals_for(&app, "delete_report").await, 1);
    assert_eq!(reports(&app).await, 1, "still there until approved");

    let pending = call(
        &app,
        &cookie,
        "delete_template",
        json!({ "id": Uuid::new_v4().to_string(), "confirmed": true }),
    )
    .await;
    assert_eq!(pending["outcome"], "needs_approval", "{pending}");
    assert_eq!(approvals_for(&app, "delete_template").await, 1);
}

/// `BI-18D-AC9`, second half: the templates are listed and a dashboard is
/// started from one, confirmed in the chat.
#[tokio::test]
async fn a_dashboard_starts_from_a_listed_template_after_the_confirm() {
    let (app, _ch) = app().await;
    let cookie = person(&app, WRITER).await;
    let definition = json!({
        "file": { "kind": "rantai-dashboard", "version": 1, "board": { "name": "Sales overview" }, "charts": [], "tables": [] },
        "charts": {},
    });
    let saved = lakehouse_store::dashboard_templates::insert(
        &app.pool,
        lakehouse_store::dashboard_templates::TemplateInput {
            name: "Sales overview",
            source_board: "b_one",
            source_board_name: "Board b_one",
            chart_count: 0,
            definition: &definition,
            by: "Rina",
        },
    )
    .await
    .unwrap();

    let listed = call(&app, &cookie, "list_templates", json!({})).await;
    assert_eq!(
        listed["result"]["templates"][0]["name"], "Sales overview",
        "{listed}"
    );

    let args = json!({ "template": saved.id.to_string(), "name": "Q4 sales" });
    let pending = call(
        &app,
        &cookie,
        "create_dashboard_from_template",
        args.clone(),
    )
    .await;
    assert_eq!(pending["outcome"], "needs_confirmation", "{pending}");
    let mut confirmed = args;
    confirmed["confirmed"] = json!(true);
    let done = call(&app, &cookie, "create_dashboard_from_template", confirmed).await;
    let result = &done["result"];
    assert_eq!(result["ok"], true, "{done}");
    assert_eq!(result["board"]["name"], "Q4 sales");
    let board = result["board"]["id"].as_str().unwrap();
    assert_eq!(result["url"], format!("/dashboards/{board}"));
    assert_eq!(result["leftOut"], json!([]));
}

/// Without `dashboard:write` every one of the six is refused, and nothing is
/// written.
#[tokio::test]
async fn every_report_and_template_tool_needs_dashboard_write() {
    let (app, _ch) = app().await;
    let cookie = person(&app, "dashboard:read").await;
    for (tool, args) in [
        ("create_report", json!({ "title": "X", "confirmed": true })),
        (
            "update_report",
            json!({ "id": Uuid::new_v4().to_string(), "confirmed": true }),
        ),
        (
            "delete_report",
            json!({ "id": Uuid::new_v4().to_string(), "confirmed": true }),
        ),
        ("list_templates", json!({})),
        (
            "create_dashboard_from_template",
            json!({ "template": Uuid::new_v4().to_string(), "name": "X", "confirmed": true }),
        ),
        (
            "delete_template",
            json!({ "id": Uuid::new_v4().to_string(), "confirmed": true }),
        ),
    ] {
        let body = call(&app, &cookie, tool, args).await;
        assert_eq!(body["outcome"], "refused", "{tool}: {body}");
        assert!(
            body["result"].to_string().contains("dashboard:write"),
            "{tool}: {body}"
        );
    }
    assert_eq!(reports(&app).await, 0);
    assert_eq!(approvals_for(&app, "delete_report").await, 0);
}

/// Review round 1, SHOULD-FIX 2: `list_reports` is a read, the reports the
/// person may read with the ids `update_report` takes; a person with
/// `dashboard:read` alone may list them, and nobody without it.
#[tokio::test]
async fn list_reports_gives_the_ids_update_report_takes() {
    let (app, _ch) = app().await;
    let writer = person(&app, WRITER).await;
    let made = call(
        &app,
        &writer,
        "create_report",
        json!({ "title": "Weekly", "confirmed": true }),
    )
    .await;
    let id = made["result"]["report"]["id"].as_str().unwrap().to_owned();

    let reader = person(&app, "dashboard:read").await;
    let listed = call(&app, &reader, "list_reports", json!({})).await;
    assert_eq!(listed["outcome"], "executed", "{listed}");
    let row = &listed["result"]["reports"][0];
    assert_eq!(row["id"], id.as_str(), "{listed}");
    assert_eq!(row["title"], "Weekly");
    assert_eq!(row["url"], format!("/dashboards/reports/{id}"));
    assert_eq!(listed["result"]["truncated"], false);

    let nobody = person(&app, "query:read").await;
    let refused = call(&app, &nobody, "list_reports", json!({})).await;
    assert_eq!(refused["outcome"], "refused", "{refused}");
}
