//! `export_dashboard` and `export_chart` through `POST /api/ai/tool` (`AI-10`,
//! export half; `BI-1` T6, plan decision D9): the link carries the arguments,
//! nothing is generated, and the console's refusals are the assistant's.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::collections::HashMap;

use axum::body::{Body, to_bytes};
use axum::http::Request;
use common::{
    TestApp, create_principal_with_permissions, session_cookie_for_user, spin_up_with_env,
};
use serde_json::{Value, json};
use tower::ServiceExt;
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, Request as MockRequest, Respond, ResponseTemplate};

const BOARD: &str = "b_ai";

/// A `ClickHouse` with one board and one chart the viewer may see.
struct Store;

impl Respond for Store {
    fn respond(&self, request: &MockRequest) -> ResponseTemplate {
        let sql = String::from_utf8_lossy(&request.body);
        let rows = |data: Vec<Value>| {
            let n = data.len();
            ResponseTemplate::new(200).set_body_json(json!({ "meta": [], "data": data, "rows": n }))
        };
        if sql.contains("FROM console.bi_chart") {
            let spec = json!({
                "id": "c_visits", "title": "Visits by place", "kind": "bar", "mart": "mart_x",
                "sql": "SELECT 1", "x": "region", "y": "n"
            });
            let note = json!({
                "id": "c_note", "title": "Note", "kind": "text", "mart": "mart_x",
                "sql": "SELECT stored", "x": "", "y": "email"
            });
            let row = |id: &str, spec: &Value, def: Value| {
                json!({
                    "id": id, "board": BOARD, "created_by": "ui", "created_at": "2026-01-01 00:00:00",
                    "spec_json": json!({ "spec": spec, "def": def, "hasYear": false }).to_string(),
                })
            };
            return rows(vec![
                row(
                    "c_visits",
                    &spec,
                    json!({ "title": "Visits by place", "mart": "mart_x", "kind": "bar" }),
                ),
                row(
                    "c_note",
                    &note,
                    json!({ "title": "Note", "mart": "", "kind": "text" }),
                ),
            ]);
        }
        if sql.contains("FROM console.bi_board") && sql.contains(&format!("id='{BOARD}'")) {
            return rows(vec![json!({
                "id": BOARD, "name": "Sales / Q3 report", "description": "", "created_by": "t",
                "layout_json": "", "filters_json": "", "public_token": "", "embed_enabled": "0",
                "folder_id": "", "created_at": "2026-10-10 00:00:00", "owner_id": "",
                "embed_revoked_before": "0", "embed_revoked_jti_json": "[]",
                "embed_origins_json": "[]"
            })]);
        }
        ResponseTemplate::new(200).set_body_string("Ok.\n")
    }
}

async fn app(pdf: bool) -> (TestApp, MockServer, String) {
    let ch = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(Store)
        .mount(&ch)
        .await;
    let mut env: HashMap<String, String> = HashMap::from([("CH_URL".to_owned(), ch.uri())]);
    if pdf {
        env.insert(
            "GOTENBERG_URL".to_owned(),
            "http://gotenberg:3000".to_owned(),
        );
        env.insert(
            "CONSOLE_INTERNAL_URL".to_owned(),
            "http://console:3000".to_owned(),
        );
        env.insert(
            "RENDER_TOKEN_SECRET".to_owned(),
            "test-embed-secret-not-real".to_owned(),
        );
    }
    let app = spin_up_with_env(&env).await;
    let user = create_principal_with_permissions(&app.pool, "dashboard:read").await;
    let cookie = session_cookie_for_user(&app.pool, user).await;
    (app, ch, cookie)
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

#[tokio::test]
async fn export_dashboard_returns_a_link_with_the_arguments_and_makes_nothing() {
    let (app, ch, cookie) = app(true).await;
    let body = call(
        &app,
        &cookie,
        "export_dashboard",
        json!({ "board": BOARD, "paper": "letter", "orientation": "landscape" }),
        "ask",
    )
    .await;
    assert_eq!(body["outcome"], "executed", "{body}");
    let result = &body["result"];
    assert_eq!(
        result["url"],
        "/api/dashboard/export/pdf?board=b_ai&paper=letter&orientation=landscape"
    );
    let name = result["fileName"].as_str().unwrap();
    assert!(
        name.starts_with("Sales-Q3-report-")
            && std::path::Path::new(name)
                .extension()
                .is_some_and(|e| e == "pdf"),
        "{name}"
    );
    // Nothing was generated: no statement but the board lookup reached the engine.
    let audited: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_event WHERE action IN ('dashboard.pdf_export', 'dashboard.chart_export')",
    )
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert_eq!(
        audited, 0,
        "the download is audited when the link is opened, not now"
    );
    assert!(
        ch.received_requests()
            .await
            .unwrap()
            .iter()
            .all(|r| !String::from_utf8_lossy(&r.body).contains("serving.")),
        "no data was read"
    );
}

#[tokio::test]
async fn export_dashboard_defaults_to_a4_portrait_and_refuses_a_bad_word_or_a_missing_board() {
    let (app, _ch, cookie) = app(true).await;
    let body = call(
        &app,
        &cookie,
        "export_dashboard",
        json!({ "board": BOARD }),
        "ask",
    )
    .await;
    assert_eq!(
        body["result"]["url"],
        "/api/dashboard/export/pdf?board=b_ai&paper=a4&orientation=portrait"
    );
    for (args, sentence) in [
        (
            json!({ "board": BOARD, "paper": "a3" }),
            "paper must be a4 or letter.",
        ),
        (
            json!({ "board": BOARD, "orientation": "tilted" }),
            "orientation must be portrait or landscape.",
        ),
        (
            json!({ "board": BOARD, "format": "pptx" }),
            "A dashboard exports as a PDF: format must be pdf.",
        ),
        (
            json!({ "board": "nobody" }),
            "this dashboard does not exist",
        ),
        (json!({}), "board is required (list_boards gives the ids)."),
    ] {
        let body = call(&app, &cookie, "export_dashboard", args.clone(), "ask").await;
        assert_eq!(body["result"]["error"], sentence, "{args}: {body}");
    }
}

#[tokio::test]
async fn without_a_renderer_the_assistant_says_what_the_console_says() {
    let (app, _ch, cookie) = app(false).await;
    let body = call(
        &app,
        &cookie,
        "export_dashboard",
        json!({ "board": BOARD }),
        "ask",
    )
    .await;
    assert_eq!(
        body["result"]["error"],
        "PDF export is not set up on this deployment"
    );
}

#[tokio::test]
async fn export_chart_returns_a_link_for_excel_or_csv_and_refuses_what_has_no_data() {
    let (app, _ch, cookie) = app(false).await;
    let body = call(
        &app,
        &cookie,
        "export_chart",
        json!({ "chart": "c_visits", "format": "xlsx", "values": "raw" }),
        "ask",
    )
    .await;
    assert_eq!(
        body["result"]["url"], "/api/dashboard/chart-export?chart=c_visits&format=xlsx&values=raw",
        "{body}"
    );
    assert_eq!(body["result"]["fileName"], "Visits-by-place.xlsx");
    let formatted = call(
        &app,
        &cookie,
        "export_chart",
        json!({ "chart": "c_visits", "format": "csv" }),
        "ask",
    )
    .await;
    assert_eq!(
        formatted["result"]["url"],
        "/api/dashboard/chart-export?chart=c_visits&format=csv&values=formatted"
    );
    for (args, sentence) in [
        (
            json!({ "chart": "c_note", "format": "csv" }),
            "this tile shows no data, so there is nothing to download",
        ),
        (
            json!({ "chart": "nope", "format": "csv" }),
            "this tile does not exist",
        ),
        (
            json!({ "chart": "c_visits", "format": "pdf" }),
            "format must be xlsx or csv.",
        ),
        (
            json!({ "chart": "c_visits", "format": "csv", "values": "rounded" }),
            "values must be formatted or raw.",
        ),
    ] {
        let body = call(&app, &cookie, "export_chart", args.clone(), "ask").await;
        assert_eq!(body["result"]["error"], sentence, "{args}: {body}");
    }
}

#[tokio::test]
async fn a_person_switched_off_from_downloading_gets_the_consoles_sentence() {
    let (app, _ch, cookie) = app(true).await;
    sqlx::query(
        "INSERT INTO reporting_settings (singleton, time_zone, week_start, download_denied_roles) \
         SELECT TRUE, 'UTC', 'monday', array_agg(r.name) FROM role r",
    )
    .execute(&app.pool)
    .await
    .unwrap();
    for (tool, args) in [
        ("export_dashboard", json!({ "board": BOARD })),
        (
            "export_chart",
            json!({ "chart": "c_visits", "format": "csv" }),
        ),
    ] {
        let body = call(&app, &cookie, tool, args, "ask").await;
        assert_eq!(
            body["result"]["error"], "Downloads are switched off for your role.",
            "{tool}: {body}"
        );
    }
}

#[tokio::test]
async fn without_dashboard_read_the_tools_are_refused_by_the_gate() {
    let (app, _ch, _cookie) = app(true).await;
    let user = common::create_zero_permission_principal(&app.pool).await;
    let cookie = session_cookie_for_user(&app.pool, user).await;
    let body = call(
        &app,
        &cookie,
        "export_dashboard",
        json!({ "board": BOARD }),
        "ask",
    )
    .await;
    assert_eq!(body["outcome"], "refused", "{body}");
    assert_eq!(body["result"]["required"], "dashboard:read");
}

/// `BI-1` review fix (SHOULD-FIX) R7: the link carries the page's filters,
/// parameters and grain, in the encoding the menus use, and a value the console
/// would refuse is refused with its sentence.
#[tokio::test]
async fn the_links_carry_the_pages_filters_parameters_and_grain() {
    let (app, _ch, cookie) = app(true).await;
    let filters = json!([{ "column": "region", "values": ["north"] }]);
    let body = call(
        &app,
        &cookie,
        "export_dashboard",
        json!({ "board": BOARD, "filters": filters, "params": { "top": 5 }, "grain": "month" }),
        "ask",
    )
    .await;
    let url = body["result"]["url"].as_str().unwrap();
    let parsed = url::Url::parse(&format!("http://x{url}")).unwrap();
    let pairs: std::collections::HashMap<_, _> = parsed.query_pairs().into_owned().collect();
    assert_eq!(pairs["filters"], filters.to_string());
    assert_eq!(pairs["params"], r#"{"top":5}"#);
    assert_eq!(pairs["grain"], "month");
    assert_eq!(pairs["board"], BOARD);

    let chart = call(
        &app,
        &cookie,
        "export_chart",
        json!({ "chart": "c_visits", "format": "csv", "filters": filters }),
        "ask",
    )
    .await;
    assert!(
        chart["result"]["url"]
            .as_str()
            .unwrap()
            .contains("filters=%5B%7B%22column%22"),
        "{chart}"
    );

    // Empty lists add nothing; a bad grain or a bad filter is refused.
    let plain = call(
        &app,
        &cookie,
        "export_dashboard",
        json!({ "board": BOARD, "filters": [], "params": {} }),
        "ask",
    )
    .await;
    assert_eq!(
        plain["result"]["url"],
        "/api/dashboard/export/pdf?board=b_ai&paper=a4&orientation=portrait"
    );
    let bad = call(
        &app,
        &cookie,
        "export_dashboard",
        json!({ "board": BOARD, "grain": "fortnight" }),
        "ask",
    )
    .await;
    assert_eq!(
        bad["result"]["error"],
        "grain must be one of minute, hour, day, week, month, quarter, year."
    );
    let bad = call(
        &app,
        &cookie,
        "export_chart",
        json!({ "chart": "c_visits", "format": "csv", "filters": [{"column": 5}] }),
        "ask",
    )
    .await;
    assert!(bad["result"]["error"].is_string(), "{bad}");
}
