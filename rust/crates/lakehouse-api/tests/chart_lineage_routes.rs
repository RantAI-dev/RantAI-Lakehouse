//! `GET /api/dashboard/charts/{id}/lineage` and
//! `GET /api/governance/lineage/used-by` at the route level (`BI-14` T2 and T3,
//! `docs/superpowers/plans/2026-10-24-bi-14-chart-lineage.md`): the real router
//! over a real migrated Postgres, with a `ClickHouse` stand-in that answers the
//! chart store and the lineage records where a test mounts them and an empty
//! result for everything else.
//!
//! The permission is `lineage:read` (owner decision Q6); `route_auth` already
//! shows both ways that the table row is enforced, so these tests are about the
//! answers: the path from a chart to its sources, the refresh times and their
//! sources, "not recorded" where there is no record, the chart someone else
//! keeps in a personal space (404, and counted but not named in used-by).

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::collections::HashMap;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use tower::ServiceExt;
use wiremock::matchers::{body_string_contains, method};
use wiremock::{Mock, MockServer, ResponseTemplate};

use common::{
    TestApp, create_principal_with_permissions, session_cookie_for_user, spin_up_with_env,
};

async fn rows(server: &MockServer, needle: &str, data: Vec<Value>) {
    Mock::given(method("POST"))
        .and(body_string_contains(needle))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({ "meta": [], "data": data, "rows": data.len() })),
        )
        .with_priority(1)
        .mount(server)
        .await;
}

/// A stored chart row of `console.bi_chart`: a bar chart over `mart_visits`.
fn chart_row(id: &str, title: &str, board: &str, created_by: &str) -> Value {
    let spec_json = json!({
        "spec": { "id": id, "title": title, "kind": "bar", "mart": "mart_visits", "sql": "SELECT 1",
                  "x": "province", "y": "visitors" },
        "def": { "title": title, "mart": "mart_visits", "kind": "bar", "dimension": "province",
                 "measures": ["visitors"], "board": board },
        "hasYear": false,
    });
    json!({ "id": id, "spec_json": spec_json.to_string(), "board": board, "created_by": created_by,
            "created_at": "2026-10-01 10:00:00" })
}

struct Stack {
    app: TestApp,
    ch: MockServer,
}

impl Stack {
    async fn start() -> Self {
        let ch = MockServer::start().await;
        // Everything not mounted below answers an empty result.
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({ "meta": [], "data": [], "rows": 0 })),
            )
            .mount(&ch)
            .await;
        // The warehouse tables, and the statement that last built the Gold one from a Silver one.
        rows(
            &ch,
            "FROM system.tables WHERE database IN ('silver', 'serving')",
            vec![
                json!({ "database": "serving", "name": "mart_visits", "engine": "MergeTree", "as_select": "" }),
                json!({ "database": "silver", "name": "visits", "engine": "MergeTree", "as_select": "" }),
            ],
        )
        .await;
        rows(
            &ch,
            "FROM system.query_log",
            vec![json!({ "target": "serving.mart_visits", "read": "silver.visits", "at": "2026-09-22 10:03", "kind": "Insert" })],
        )
        .await;
        let env = HashMap::from([("CH_URL".to_owned(), ch.uri())]);
        Self {
            app: spin_up_with_env(&env).await,
            ch,
        }
    }

    async fn get(&self, cookie: &str, uri: &str) -> (StatusCode, Value) {
        let request = Request::builder()
            .uri(uri)
            .header("cookie", cookie)
            .body(Body::empty())
            .unwrap();
        let response = self.app.router.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), 4 * 1024 * 1024)
            .await
            .unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    async fn caller(&self, permissions: &str) -> String {
        let id = create_principal_with_permissions(&self.app.pool, permissions).await;
        session_cookie_for_user(&self.app.pool, id).await
    }
}

#[tokio::test]
async fn the_path_from_a_chart_goes_to_its_sources_with_refresh_times_and_their_sources() {
    let stack = Stack::start().await;
    rows(
        &stack.ch,
        "FROM console.bi_chart",
        vec![chart_row("c_1", "Visitors", "default", "ui")],
    )
    .await;
    // The shared catalog is read for a caller with the platform's `*:*`; a narrower caller still gets the chart's own path.
    let cookie = stack.caller("*:*").await;
    let (status, body) = stack
        .get(&cookie, "/api/dashboard/charts/c_1/lineage")
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["chart"], json!({ "id": "c_1", "title": "Visitors" }));
    assert_eq!(body["supported"], json!(true));
    let steps = body["steps"].as_array().unwrap();
    let ids: Vec<&str> = steps.iter().map(|s| s["id"].as_str().unwrap()).collect();
    assert_eq!(
        ids,
        [
            "chart:c_1",
            "table:serving.mart_visits",
            "table:silver.visits"
        ],
        "{body}"
    );
    assert_eq!(steps[1]["kind"], json!("gold"));
    // The Gold table has a recorded build; the Silver one was never built by a logged statement.
    assert_eq!(steps[1]["refreshedAt"], json!("2026-09-22T10:03:00Z"));
    assert_eq!(
        steps[1]["refreshSource"],
        json!("last build in the query log")
    );
    assert!(steps[1].get("notRecorded").is_none());
    assert_eq!(steps[2]["notRecorded"], json!(true));
    assert!(steps[2].get("refreshedAt").is_none());
    // The chart has no refresh time of its own.
    assert!(steps[0].get("notRecorded").is_none() && steps[0].get("refreshedAt").is_none());
    let edges = body["edges"].as_array().unwrap();
    assert!(edges.iter().any(|e| {
        e["from"] == json!("table:serving.mart_visits")
            && e["to"] == json!("chart:c_1")
            && e["evidence"]
                .as_str()
                .unwrap()
                .contains("stored chart \"Visitors\" (c_1)")
    }));
    // Every field says what it is built from, or nothing: no mapping is recorded in this test.
    let fields = body["fields"].as_array().unwrap();
    assert_eq!(
        fields
            .iter()
            .map(|f| f["field"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["province", "visitors"]
    );
    assert!(fields.iter().all(|f| f["columns"] == json!([])));
    assert!(
        body["coverage"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c.as_str().unwrap().contains("not recorded"))
    );
}

#[tokio::test]
async fn a_chart_in_somebody_elses_personal_space_is_not_found() {
    let stack = Stack::start().await;
    // A chart on a board the caller does not own: the store lists it only to its owner, so the
    // stand-in returns nothing to this caller and the answer is the one an unknown id gets.
    let cookie = stack.caller("lineage:read").await;
    let (status, body) = stack
        .get(&cookie, "/api/dashboard/charts/c_private/lineage")
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["error"], json!("No such chart."));
}

#[tokio::test]
async fn used_by_lists_the_charts_that_read_a_table_and_a_bad_table_is_a_400() {
    let stack = Stack::start().await;
    rows(
        &stack.ch,
        "FROM console.bi_chart",
        vec![chart_row("c_1", "Visitors", "b_1", "ui")],
    )
    .await;
    rows(
        &stack.ch,
        "FROM console.bi_board",
        vec![json!({ "id": "b_1", "name": "Tourism", "description": "", "created_by": "ui", "layout_json": "",
                     "filters_json": "", "public_token": "", "embed_enabled": 0, "folder_id": "",
                     "refresh_seconds": 0, "grain": "", "params_json": "", "tabs_json": "",
                     "phone_layout_json": "", "theme": "", "font": "", "owner_id": "", "created_at": "2026-10-01 10:00:00" })],
    )
    .await;
    let cookie = stack.caller("*:*").await;
    let (status, body) = stack
        .get(
            &cookie,
            "/api/governance/lineage/used-by?table=serving.mart_visits",
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["table"], json!("serving.mart_visits"));
    assert_eq!(body["hiddenCount"], json!(0));
    assert_eq!(
        body["charts"],
        json!([{ "chartId": "c_1", "chartTitle": "Visitors", "dashboardId": "b_1", "dashboardTitle": "Tourism", "via": "direct" }])
    );
    let (none_status, none) = stack
        .get(
            &cookie,
            "/api/governance/lineage/used-by?table=serving.mart_other",
        )
        .await;
    assert_eq!(none_status, StatusCode::OK);
    assert_eq!(none["charts"], json!([]));
    for bad in ["", "mart_visits", "console.bi_chart"] {
        let (status, body) = stack
            .get(
                &cookie,
                &format!("/api/governance/lineage/used-by?table={bad}"),
            )
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}: {body}");
    }
}
