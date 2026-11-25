//! HTTP-level integration tests for the T2.5 governance draft tools
//! (`draft_policy`, `draft_classification_rule`, `draft_quality_rule`):
//! every one of them must create a record that is never "active" —
//! `draft_policy` always `status = "draft"` even when the model asks for
//! `activate: true`; `draft_classification_rule`/`draft_quality_rule` have
//! no activation concept at all, so this proves they land in the same
//! unevaluated state a human authoring the same rule through the console
//! would get: `review_status = "needs-review"` for classification, and for
//! quality a `null` `lastStatus`/`lastRunAt` (WS1 finding J18) — the API
//! never surfaces the `quality_rule.last_status` column's NOT NULL
//! `'warning'` default, since no evaluator anywhere in the workspace ever
//! runs a quality rule.
//!
//! `draft_quality_rule` asks whether the rule's table exists before it stores
//! the rule (T8), so [`app`] puts a `ClickHouse` stand-in behind the API: the
//! table the draft names answers `system.columns` with a column, and one test
//! answers empty for a table that is not there (review round 1 R2).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use crate::common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use wiremock::matchers::{body_string_contains, method};
use wiremock::{Mock, MockServer, ResponseTemplate};

use common::{TestApp, session_cookie_for_seeded_user, spin_up_with_env};

/// A router over a fresh test database, with a `ClickHouse` stand-in that
/// answers `system.columns` with one column for every table: a drafted rule's
/// table exists as far as the API can tell. The stand-in is returned, so a
/// test may answer "no such table" for one name at `with_priority(1)`.
async fn app() -> (axum::Router, PgPool, MockServer) {
    let ch = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_string_contains("system.columns"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "meta": [{"name": "name", "type": "String"}, {"name": "type", "type": "String"}],
            "data": [{"name": "id", "type": "String"}],
            "rows": 1,
        })))
        .mount(&ch)
        .await;
    let mut overrides = std::collections::HashMap::new();
    overrides.insert("CH_URL".to_owned(), ch.uri());
    let TestApp { router, pool } = spin_up_with_env(&overrides).await;
    (router, pool, ch)
}

async fn post(
    app: &axum::Router,
    uri: &str,
    cookie: &str,
    body: Value,
) -> axum::http::Response<Body> {
    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header("content-type", "application/json")
                .header("cookie", cookie)
                .body(Body::from(
                    serde_json::to_vec(&body).expect("serialize body"),
                ))
                .expect("build request"),
        )
        .await
        .expect("router never fails a request outright")
}

async fn json_body(resp: axum::http::Response<Body>) -> Value {
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("read response body");
    serde_json::from_slice(&bytes).expect("valid JSON body")
}

/// Calls `POST /api/ai/tool` for `tool`/`args` (with `confirmed: true`
/// merged in, since every draft tool is `WriteLow`) as Fajar (Platform
/// Admin: `*:*`) in build mode, and asserts it actually executed.
async fn run_write_low_tool(
    router: &axum::Router,
    pool: &sqlx::PgPool,
    tool: &str,
    mut args: Value,
) -> Value {
    let cookie = session_cookie_for_seeded_user(pool, "fajar@meridian.example").await;
    args["confirmed"] = json!(true);
    let resp = post(
        router,
        "/api/ai/tool",
        &cookie,
        json!({ "tool": tool, "args": args, "mode": "build" }),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK, "{tool}");
    let body = json_body(resp).await;
    assert_eq!(body["outcome"], "executed", "{tool}: {body}");
    body["result"].clone()
}

/// `draft_policy` forces `status = "draft"` in the stored row even though
/// the call asks for `activate: true` — the tool must never let a copilot
/// caller create an already-`"ready"` policy.
#[tokio::test]
async fn draft_policy_always_creates_a_draft_status_row() {
    let (router, pool, _ch) = app().await;
    let result = run_write_low_tool(
        &router,
        &pool,
        "draft_policy",
        json!({
            "name": "copilot_drafted_row_filter",
            "kind": "Row filter",
            "subjects": "All analysts",
            "resources": "tenant-scoped tables",
            "effect": "Permit with obligation",
            "activate": true,
        }),
    )
    .await;
    assert_eq!(result["status"], "draft", "{result}");

    let (status,): (String,) =
        sqlx::query_as("SELECT status FROM policy WHERE name = 'copilot_drafted_row_filter'")
            .fetch_one(&pool)
            .await
            .expect("row must exist");
    assert_eq!(status, "draft");
}

/// Calls `draft_policy` the way [`run_write_low_tool`] does, for a call the
/// tool must refuse: asserts the tool route reports it as `failed` (a result
/// with an `error` key) and returns that error sentence (AI-27).
async fn run_refused_draft_policy(
    router: &axum::Router,
    pool: &sqlx::PgPool,
    mut args: Value,
) -> String {
    let cookie = session_cookie_for_seeded_user(pool, "fajar@meridian.example").await;
    args["confirmed"] = json!(true);
    let resp = post(
        router,
        "/api/ai/tool",
        &cookie,
        json!({ "tool": "draft_policy", "args": args, "mode": "build" }),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = json_body(resp).await;
    assert_eq!(body["outcome"], "failed", "{body}");
    body["result"]["error"]
        .as_str()
        .unwrap_or_else(|| panic!("a refusal carries an error sentence: {body}"))
        .to_owned()
}

/// The fixed sentence a refused draft answers with (AI-27): it names what an
/// enforceable condition needs, so the model can act on it.
const CONDITIONS_REFUSAL: &str =
    "The conditions need roles, a table, and a mask or a row filter, so the draft was not saved.";

/// The args of a policy draft, with `conditions` left for the test to set.
fn policy_args(name: &str) -> Value {
    json!({
        "name": name,
        "kind": "Row filter",
        "subjects": "Analysts",
        "resources": "serving.mart_customer_segment",
        "effect": "Permit with obligation",
    })
}

async fn policy_rows(pool: &sqlx::PgPool, name: &str) -> i64 {
    let (n,): (i64,) = sqlx::query_as("SELECT count(*) FROM policy WHERE name = $1")
        .bind(name)
        .fetch_one(pool)
        .await
        .expect("count policies");
    n
}

/// AI-27 (a): structured conditions with a mask are saved as a draft that the
/// engine can enforce, and the result says so.
#[tokio::test]
async fn draft_policy_with_structured_conditions_is_an_enforceable_draft() {
    let (router, pool, _ch) = app().await;
    let mut args = policy_args("copilot_drafted_masked_policy");
    args["conditions"] = json!({
        "roles": ["analyst"],
        "table": "serving.mart_customer_segment",
        "mask": ["email"],
    });
    let result = run_write_low_tool(&router, &pool, "draft_policy", args).await;
    assert_eq!(result["status"], "draft", "{result}");
    assert_eq!(result["enforceable"], true, "{result}");

    let (status, conditions): (String, Option<String>) = sqlx::query_as(
        "SELECT status, conditions FROM policy WHERE name = 'copilot_drafted_masked_policy'",
    )
    .fetch_one(&pool)
    .await
    .expect("row must exist");
    assert_eq!(status, "draft");
    let stored = conditions.expect("conditions are stored");
    assert!(
        lakehouse_api::policy_engine::PolicyCondition::parse(&stored).is_some(),
        "the stored conditions must be enforceable: {stored}"
    );
}

/// AI-27 (b): roles and a table with neither a mask nor a row filter enforce
/// nothing, so the tool refuses and saves no row.
#[tokio::test]
async fn draft_policy_conditions_without_an_obligation_are_refused() {
    let (router, pool, _ch) = app().await;
    let mut args = policy_args("copilot_drafted_no_obligation");
    args["conditions"] = json!({
        "roles": ["analyst"],
        "table": "serving.mart_customer_segment",
    });
    let error = run_refused_draft_policy(&router, &pool, args).await;
    assert_eq!(error, CONDITIONS_REFUSAL);
    assert_eq!(policy_rows(&pool, "copilot_drafted_no_obligation").await, 0);
}

/// AI-27 (c): `conditions` as prose is what a model following the old schema
/// sends. The tool refuses it with the same sentence and saves no row.
#[tokio::test]
async fn draft_policy_prose_conditions_are_refused() {
    let (router, pool, _ch) = app().await;
    let mut args = policy_args("copilot_drafted_prose_conditions");
    args["conditions"] = json!("Analysts may not see the email column");
    let error = run_refused_draft_policy(&router, &pool, args).await;
    assert_eq!(error, CONDITIONS_REFUSAL);
    assert_eq!(
        policy_rows(&pool, "copilot_drafted_prose_conditions").await,
        0
    );
}

/// AI-27 (d): a draft with no `conditions` is still saved, and the result says
/// it enforces nothing, next to `enforceable: false`.
#[tokio::test]
async fn draft_policy_without_conditions_says_it_enforces_nothing() {
    let (router, pool, _ch) = app().await;
    let result = run_write_low_tool(
        &router,
        &pool,
        "draft_policy",
        policy_args("copilot_drafted_without_conditions"),
    )
    .await;
    assert_eq!(result["status"], "draft", "{result}");
    assert_eq!(result["enforceable"], false, "{result}");
    assert_eq!(
        result["note"],
        "This draft enforces nothing until a person adds conditions in the console (Governance, then Policies).",
        "{result}"
    );
    assert_eq!(
        policy_rows(&pool, "copilot_drafted_without_conditions").await,
        1
    );
}

/// AI-27 (e): `activate: true` still saves a draft when the conditions are
/// structured, so the new check does not open a way around the draft-only rule.
#[tokio::test]
async fn draft_policy_with_structured_conditions_ignores_activate() {
    let (router, pool, _ch) = app().await;
    let mut args = policy_args("copilot_drafted_activate_true");
    args["activate"] = json!(true);
    args["conditions"] = json!({
        "roles": ["analyst"],
        "table": "serving.mart_customer_segment",
        "rowFilter": "region = 'ID'",
    });
    let result = run_write_low_tool(&router, &pool, "draft_policy", args).await;
    assert_eq!(result["status"], "draft", "{result}");
    let (status,): (String,) =
        sqlx::query_as("SELECT status FROM policy WHERE name = 'copilot_drafted_activate_true'")
            .fetch_one(&pool)
            .await
            .expect("row must exist");
    assert_eq!(status, "draft");
}

/// `draft_classification_rule` always lands `review_status = "needs-review"`
/// — the only value `create_classification_rule` ever inserts.
#[tokio::test]
async fn draft_classification_rule_is_never_reviewed_or_auto() {
    let (router, pool, _ch) = app().await;
    let result = run_write_low_tool(
        &router,
        &pool,
        "draft_classification_rule",
        json!({ "asset": "core.customer.copilot_test", "classification": "confidential" }),
    )
    .await;
    assert_eq!(result["reviewStatus"], "needs-review", "{result}");

    let (review_status,): (String,) = sqlx::query_as(
        "SELECT review_status FROM classification_rule WHERE asset = 'core.customer.copilot_test'",
    )
    .fetch_one(&pool)
    .await
    .expect("row must exist");
    assert_eq!(review_status, "needs-review");
}

/// `draft_quality_rule` is never `"passed"` or `"failed"` — that guarantee
/// still holds, but the honest value is `null`, not `"warning"` (WS1
/// finding J18). No evaluator anywhere in the workspace ever writes
/// `quality_rule.last_status`, so a copilot draft carries no verdict at
/// all; `create_quality_rule` originally asserted the fabricated
/// `"warning"` here, which this test corrected. The stored placeholder
/// (the NOT NULL column's `'warning'` default) is still checked, but only
/// for what it is guaranteed not to be — a real passed/failed verdict —
/// since the API never surfaces it.
#[tokio::test]
async fn draft_quality_rule_is_never_passed_or_failed() {
    let (router, pool, _ch) = app().await;
    let result = run_write_low_tool(
        &router,
        &pool,
        "draft_quality_rule",
        json!({
            "name": "copilot_drafted_quality_rule",
            "asset": "serving.mart_wisman",
            "dimension": "completeness",
            "threshold": ">= 95%",
            "severity": "medium",
        }),
    )
    .await;
    assert!(result["lastStatus"].is_null(), "{result}");

    let (last_status,): (String,) = sqlx::query_as(
        "SELECT last_status FROM quality_rule WHERE name = 'copilot_drafted_quality_rule'",
    )
    .fetch_one(&pool)
    .await
    .expect("row must exist");
    assert_ne!(last_status, "passed");
    assert_ne!(last_status, "failed");
}

/// Review round 1 R2 (`DATA-54`): the drafted rule's table must exist, so a
/// table the stand-in answers no columns for is refused and nothing is stored.
/// The `ClickHouse` stand-in is the one [`app`] mounts for every other test;
/// only the missing table answers empty.
#[tokio::test]
async fn a_drafted_rule_for_a_table_that_is_not_there_is_refused() {
    let (router, pool, ch) = app().await;
    Mock::given(method("POST"))
        .and(body_string_contains("system.columns"))
        .and(body_string_contains("no_such_table"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "meta": [], "data": [], "rows": 0,
        })))
        .with_priority(1)
        .mount(&ch)
        .await;

    let cookie = session_cookie_for_seeded_user(&pool, "fajar@meridian.example").await;
    let resp = post(
        &router,
        "/api/ai/tool",
        &cookie,
        json!({
            "tool": "draft_quality_rule",
            "args": {
                "name": "copilot_drafted_missing_table",
                "asset": "serving.no_such_table",
                "dimension": "completeness",
                "threshold": ">= 95%",
                "severity": "medium",
                "confirmed": true,
            },
            "mode": "build",
        }),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = json_body(resp).await;
    assert_eq!(body["outcome"], "failed", "{body}");
    assert!(
        body["result"]["error"]
            .as_str()
            .unwrap_or_default()
            .contains("is not a table you can read"),
        "{body}"
    );
    let (stored,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM quality_rule WHERE name = 'copilot_drafted_missing_table'",
    )
    .fetch_one(&pool)
    .await
    .expect("count");
    assert_eq!(stored, 0, "a refused draft stores nothing");
}
