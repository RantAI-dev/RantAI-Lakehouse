//! The `get_lineage` tool answers "where does this number come from?" for a
//! chart (`AI-13`, plan `ai-143`): called with `chart`, it returns the
//! chart-lineage route's own answer (`BI-14`'s read, never a second graph
//! builder), plus a compact `chain`; called with `slug`, it also names the
//! charts that read the table. Both arguments together, and neither, are
//! refused with one sentence; a chart the caller cannot see answers exactly
//! like an unknown id, with no hint that it exists.
//!
//! The stack is the one `chart_lineage_routes.rs` uses (`BI-14` T2/T3): the
//! real router over a real migrated Postgres, with a `ClickHouse` stand-in
//! that answers the chart store and the lineage records where a test mounts
//! them and an empty result for everything else. The tool is called the way
//! the chat calls it, through `POST /api/ai/tool`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use crate::common;

use std::collections::HashMap;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;
use wiremock::matchers::{body_string_contains, method};
use wiremock::{Mock, MockServer, ResponseTemplate};

use common::{
    CATALOG_TENANT, TestApp, create_principal_with_permissions, in_the_catalog_tenant,
    session_cookie_for_user, spin_up_with_env,
};

/// What the tool route needs, plus what the lineage and chart reads gate on:
/// the same set `lakehouse_tenant_scope.rs`'s lineage reader holds.
const CHAT_LINEAGE: &str = "assistant:use, assistant:ask, lineage:read, dashboard:read";

/// A stored chart row of `console.bi_chart`: a bar chart over `mart_visits`
/// (the same shape `chart_lineage_routes.rs` mounts).
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

/// The Gold and Silver warehouse tables, and the statement that last built
/// the Gold one from the Silver one.
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

/// The connector ingest and the pipeline that put the Bronze and connector
/// steps behind `silver.visits`: one ingestible connector of `tenant` whose
/// spec targets `bronze.visits_raw`, and one pipeline that builds
/// `silver.visits` from it (the rows `routes::lineage::add_tenant_owned`
/// draws, seeded the way `lineage.rs`'s own tests seed them).
async fn seed_connector_and_pipeline(pool: &sqlx::PgPool, tenant: Uuid) {
    sqlx::query(
        "INSERT INTO connector (id, tenant_id, name, type, direction, host, secret_ref, \
         environment, tenant, adapter, ingest_mode, dial, source_objects) \
         VALUES ('conn-eval', $1, 'Eval Source', 'PostgreSQL', 'source', 'unused', \
         'env:SECRET', 'production', 'meridian', 'sql', 'batch', '{}'::jsonb, \
         '[{\"target\": \"visits_raw\"}]'::jsonb)",
    )
    .bind(tenant)
    .execute(pool)
    .await
    .expect("insert the connector");
    sqlx::query(
        "INSERT INTO pipeline_definition \
            (id, name, kind, status, source, target, schedule, tenant_id, input_tables) \
         VALUES ('pl-eval', 'Eval Pipeline', 'batch', 'draft', 'bronze.visits_raw', \
                 'silver.visits', '@hourly', $1, ARRAY['bronze.visits_raw'])",
    )
    .bind(tenant)
    .execute(pool)
    .await
    .expect("insert the pipeline");
    // The connector's refresh time comes from its last successful run.
    sqlx::query(
        "INSERT INTO connector_run \
            (run_id, connector_id, trigger_kind, status, started_at, ended_at) \
         VALUES ('run-eval', 'conn-eval', 'schedule', 'succeeded', \
                 '2026-11-01 02:00:00', '2026-11-01 02:05:00')",
    )
    .execute(pool)
    .await
    .expect("insert the connector run");
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
        // `SEC-20`, plan 96: the charts belong to the shared catalog, which the
        // seeded deployment (several tenants) opens only to the tenant it names.
        let env = HashMap::from([
            ("CH_URL".to_owned(), ch.uri()),
            ("CATALOG_TENANT_ID".to_owned(), CATALOG_TENANT.to_owned()),
        ]);
        Self {
            app: spin_up_with_env(&env).await,
            ch,
        }
    }

    /// `get_lineage`, called the way the chat calls a tool.
    async fn get_lineage(&self, cookie: &str, args: Value) -> (StatusCode, Value) {
        let request = Request::builder()
            .method("POST")
            .uri("/api/ai/tool")
            .header("cookie", cookie)
            .header("content-type", "application/json")
            .body(Body::from(
                json!({ "tool": "get_lineage", "args": args, "mode": "build" }).to_string(),
            ))
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

    /// A caller of the tenant that owns the shared catalog, which is the
    /// caller the chart and its lineage are shown to.
    async fn catalog_member(&self, permissions: &str) -> String {
        let id = create_principal_with_permissions(&self.app.pool, permissions).await;
        in_the_catalog_tenant(&self.app.pool, id).await;
        session_cookie_for_user(&self.app.pool, id).await
    }

    /// The same, with the caller's role narrowed to the listed tables
    /// (`GOV-7` D6): every other table is hidden from every read.
    async fn catalog_member_listed(&self, permissions: &str, grants: &[(&str, &str)]) -> String {
        let id = create_principal_with_permissions(&self.app.pool, permissions).await;
        in_the_catalog_tenant(&self.app.pool, id).await;
        let role: Uuid = sqlx::query_scalar("SELECT role_id FROM app_user_role WHERE user_id = $1")
            .bind(id)
            .fetch_one(&self.app.pool)
            .await
            .expect("the single-permission role");
        sqlx::query("UPDATE role SET read_scope = 'listed' WHERE id = $1")
            .bind(role)
            .execute(&self.app.pool)
            .await
            .expect("narrow the role");
        for (schema, table) in grants {
            sqlx::query(
                "INSERT INTO role_read_grant (role_id, schema_name, table_name) VALUES ($1, $2, $3)",
            )
            .bind(role)
            .bind(schema)
            .bind(table)
            .execute(&self.app.pool)
            .await
            .expect("insert the grant");
        }
        session_cookie_for_user(&self.app.pool, id).await
    }
}

/// The step ids of a chart answer, in the order the route lists them
/// (nearest the chart first).
fn step_ids(result: &Value) -> Vec<String> {
    result["steps"]
        .as_array()
        .unwrap_or_else(|| panic!("a list of steps: {result}"))
        .iter()
        .map(|s| s["id"].as_str().expect("a step id").to_owned())
        .collect()
}

/// `AI-13` AC1: a `chart` call on a chart the caller can see returns the
/// route's whole path — with a connector step, the recorded refresh times,
/// "not recorded" where the platform has none, and the compact `chain`.
#[tokio::test]
async fn a_chart_call_names_the_whole_path_with_a_connector_and_the_refresh_times() {
    let stack = Stack::start().await;
    rows(
        &stack.ch,
        "FROM console.bi_chart",
        vec![chart_row("c_1", "Visitors", "b_1", "ui")],
    )
    .await;
    let tenant: Uuid = CATALOG_TENANT.parse().expect("a seeded tenant id");
    seed_connector_and_pipeline(&stack.app.pool, tenant).await;
    let cookie = stack.catalog_member(CHAT_LINEAGE).await;

    let (status, body) = stack.get_lineage(&cookie, json!({ "chart": "c_1" })).await;

    assert_eq!(status, StatusCode::OK, "{body}");
    let result = &body["result"];
    assert_eq!(
        result["chart"],
        json!({ "id": "c_1", "title": "Visitors" }),
        "{body}"
    );
    assert_eq!(result["supported"], json!(true), "{body}");
    assert_eq!(
        step_ids(result),
        [
            "chart:c_1",
            "table:serving.mart_visits",
            "table:silver.visits",
            "bronze:visits_raw",
            "connector:conn-eval",
        ],
        "{body}"
    );
    // The Gold table's recorded build, and the connector's last successful run.
    let steps = result["steps"].as_array().unwrap();
    let gold = steps
        .iter()
        .find(|s| s["id"] == json!("table:serving.mart_visits"))
        .unwrap();
    assert_eq!(gold["refreshedAt"], json!("2026-09-22T10:03:00Z"), "{body}");
    assert_eq!(
        gold["refreshSource"],
        json!("last build in the query log"),
        "{body}"
    );
    let connector = steps
        .iter()
        .find(|s| s["id"] == json!("connector:conn-eval"))
        .unwrap();
    assert_eq!(
        connector["refreshSource"],
        json!("last successful connector run"),
        "{body}"
    );
    // No refresh time is on record for the Silver and Bronze tables.
    for id in ["table:silver.visits", "bronze:visits_raw"] {
        let step = steps.iter().find(|s| s["id"] == json!(id)).unwrap();
        assert_eq!(step["notRecorded"], json!(true), "{id}: {body}");
    }
    // The compact chain carries every step, the recorded times and the gaps.
    let chain = result["chain"].as_str().expect("a chain string");
    assert!(chain.contains("Visitors (chart)"), "{chain}");
    assert!(
        chain.contains("serving.mart_visits (gold, refreshed 2026-09-22T10:03:00Z via last build in the query log)"),
        "{chain}"
    );
    assert!(
        chain.contains("silver.visits (silver, refresh not recorded)"),
        "{chain}"
    );
    assert!(
        chain.contains("bronze.visits_raw (bronze, refresh not recorded)"),
        "{chain}"
    );
    assert!(
        chain.contains("Eval Source (connector, refreshed 2026-11-01T02:05:00.000Z via last successful connector run)"),
        "{chain}"
    );
}

/// Decision 3: a chart the caller cannot see answers like an unknown id —
/// the same one refusal, and no step that says the chart exists. Without
/// `dashboard:read` no chart is named at all, so that caller is refused too.
#[tokio::test]
async fn a_chart_the_caller_cannot_see_is_refused_like_an_unknown_id() {
    let stack = Stack::start().await;
    rows(
        &stack.ch,
        "FROM console.bi_chart",
        vec![chart_row("c_1", "Visitors", "b_1", "ui")],
    )
    .await;
    let reader = stack.catalog_member(CHAT_LINEAGE).await;
    let no_dashboard_read = stack
        .catalog_member("assistant:use, assistant:ask, lineage:read")
        .await;

    // An id no chart holds.
    let (unknown_status, unknown) = stack
        .get_lineage(&reader, json!({ "chart": "c_private" }))
        .await;
    // The chart exists, but the caller without `dashboard:read` is refused
    // before any chart is named.
    let (hidden_status, hidden) = stack
        .get_lineage(&no_dashboard_read, json!({ "chart": "c_1" }))
        .await;

    assert_eq!(unknown_status, StatusCode::OK, "{unknown}");
    assert_eq!(hidden_status, StatusCode::OK, "{hidden}");
    assert_eq!(unknown, hidden, "one refusal for every invisible chart");
    let error = unknown["result"]["error"].as_str().expect("a refusal");
    assert!(!error.is_empty(), "{unknown}");
    assert!(unknown["result"].get("steps").is_none(), "{unknown}");
    assert!(
        !unknown.to_string().contains("Visitors"),
        "no hint the chart exists: {unknown}"
    );
    assert!(!hidden.to_string().contains("Visitors"), "{hidden}");
}

/// `GOV-7` D6: a source table the caller's role hides is dropped from the
/// chart's path, exactly as the route drops it.
#[tokio::test]
async fn a_hidden_source_table_is_left_out_of_a_charts_answer() {
    let stack = Stack::start().await;
    rows(
        &stack.ch,
        "FROM console.bi_chart",
        vec![chart_row("c_1", "Visitors", "b_1", "ui")],
    )
    .await;
    // The role may read the Gold mart only: `silver.visits` is hidden, with
    // the Bronze table and connector behind it.
    let cookie = stack
        .catalog_member_listed(CHAT_LINEAGE, &[("serving", "mart_visits")])
        .await;

    let (status, body) = stack.get_lineage(&cookie, json!({ "chart": "c_1" })).await;

    assert_eq!(status, StatusCode::OK, "{body}");
    let ids = step_ids(&body["result"]);
    assert!(
        ids.contains(&"table:serving.mart_visits".to_owned()),
        "the granted Gold table is drawn: {body}"
    );
    assert!(
        !ids.contains(&"table:silver.visits".to_owned()),
        "the hidden Silver table is not drawn: {body}"
    );
    assert!(
        !body.to_string().contains("silver.visits"),
        "the hidden table is named nowhere, chain included: {body}"
    );
}

/// Exactly one of `slug`/`chart` names a subject: both, and neither, are
/// refused with one fixed sentence.
#[tokio::test]
async fn both_or_neither_of_slug_and_chart_is_refused_with_one_sentence() {
    let stack = Stack::start().await;
    let cookie = stack.catalog_member(CHAT_LINEAGE).await;

    let (both_status, both) = stack
        .get_lineage(
            &cookie,
            json!({ "slug": "serving.mart_visits", "chart": "c_1" }),
        )
        .await;
    let (neither_status, neither) = stack.get_lineage(&cookie, json!({})).await;

    assert_eq!(both_status, StatusCode::OK, "{both}");
    assert_eq!(neither_status, StatusCode::OK, "{neither}");
    assert_eq!(
        both["result"]["error"], neither["result"]["error"],
        "one sentence for both shapes: {both} | {neither}"
    );
    let sentence = both["result"]["error"].as_str().expect("a refusal");
    assert!(
        sentence.contains("slug") && sentence.contains("chart"),
        "{both}"
    );
    assert!(neither["result"].get("steps").is_none(), "{neither}");
}

/// Decision 2: with a `slug`, the answer also names the charts that read
/// the table, from the used-by route's own read — and for a focus that is
/// not a table name it says so honestly instead of inventing an empty list.
#[tokio::test]
async fn a_slug_call_also_lists_the_charts_that_use_the_table() {
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
    let cookie = stack.catalog_member(CHAT_LINEAGE).await;

    let (status, body) = stack
        .get_lineage(&cookie, json!({ "slug": "serving.mart_visits" }))
        .await;

    assert_eq!(status, StatusCode::OK, "{body}");
    let result = &body["result"];
    // The slug path's own graph is still the answer's core.
    assert!(
        !result["nodes"].as_array().expect("nodes").is_empty(),
        "{body}"
    );
    assert_eq!(
        result["usedBy"]["charts"],
        json!([{ "chartId": "c_1", "chartTitle": "Visitors", "dashboardId": "b_1",
                 "dashboardTitle": "Tourism", "via": "direct" }]),
        "{body}"
    );
    assert_eq!(result["usedBy"]["hiddenCount"], json!(0), "{body}");

    // A dataset slug is not a table name: the used-by read refuses it, and
    // the answer says so instead of an empty list that would read as "none".
    let (status, body) = stack
        .get_lineage(&cookie, json!({ "slug": "group-sales-ledger" }))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["result"]["usedBy"]["supported"],
        json!(false),
        "{body}"
    );
    let reason = body["result"]["usedBy"]["reason"]
        .as_str()
        .expect("a reason");
    assert!(!reason.is_empty(), "{body}");
}

/// A slug naming a table the caller's role hides (`GOV-7` D6) is a real
/// table name, so the used-by refusal must not claim the focus was not a
/// table name: the honest sentence says the list could not be read for
/// this user, and stays true for every other failure cause too (review
/// SHOULD-FIX on `get_lineage`'s slug path, principle 2).
#[tokio::test]
async fn a_hidden_tables_used_by_refusal_says_the_list_could_not_be_read() {
    let stack = Stack::start().await;
    rows(
        &stack.ch,
        "FROM console.bi_chart",
        vec![chart_row("c_1", "Visitors", "b_1", "ui")],
    )
    .await;
    // The role may read no table at all: `serving.mart_visits` is hidden,
    // and with it the used-by list — `used_by::read` answers the same 404
    // an unknown asset gets, before any chart is read.
    let cookie = stack.catalog_member_listed(CHAT_LINEAGE, &[]).await;

    let (status, body) = stack
        .get_lineage(&cookie, json!({ "slug": "serving.mart_visits" }))
        .await;

    assert_eq!(status, StatusCode::OK, "{body}");
    let result = &body["result"];
    assert!(
        result.get("nodes").is_some(),
        "the graph answer still stands: {body}"
    );
    assert_eq!(result["usedBy"]["supported"], json!(false), "{body}");
    assert_eq!(
        result["usedBy"]["reason"],
        json!("the used-by list could not be read for this user"),
        "the honest sentence, not the not-a-table-name one: {body}"
    );
}

/// The chart path sits beside the slug path, not instead of it: the legacy
/// `focus` argument still works.
#[tokio::test]
async fn the_legacy_focus_argument_still_names_the_table() {
    let stack = Stack::start().await;
    rows(
        &stack.ch,
        "FROM console.bi_chart",
        vec![chart_row("c_1", "Visitors", "b_1", "ui")],
    )
    .await;
    let cookie = stack.catalog_member(CHAT_LINEAGE).await;

    let (status, body) = stack
        .get_lineage(&cookie, json!({ "focus": "serving.mart_visits" }))
        .await;

    assert_eq!(status, StatusCode::OK, "{body}");
    let result = &body["result"];
    assert!(result.get("error").is_none(), "{body}");
    assert!(
        result["nodes"]
            .as_array()
            .expect("nodes")
            .iter()
            .any(|n| n["id"] == json!("table:serving.mart_visits")),
        "{body}"
    );
}
