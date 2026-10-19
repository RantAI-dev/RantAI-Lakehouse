//! Quality incidents (`DATA-14`, finding F2), driven through the real router
//! against a per-test Postgres and a `wiremock` stand-in for `ClickHouse`.
//!
//! The stand-in answers a table with an `email` column and 200 rows of which
//! 150 are filled in, so `email not null >= 90%` fails (75.0%) and
//! `email not null >= 70%` passes. The tests cover: a failed run opens one
//! incident (`DATA-14-AC1`), a second failure moves `last_seen_at` instead
//! of opening another, a passing run closes nothing, resolving needs a
//! reason and a rule, and deleting a rule resolves its incidents; and that
//! editing a rule no longer erases its runs while deleting one does.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use lakehouse_store::governance::{CreateQualityRuleInput, create_quality_rule};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use wiremock::matchers::{body_string_contains, method};
use wiremock::{Mock, MockServer, ResponseTemplate};

use common::{
    TestApp, create_principal_with_permissions, session_cookie_for_user, spin_up_with_env,
};

async fn app() -> (axum::Router, PgPool, MockServer) {
    let ch = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_string_contains("system.columns"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "meta": [{"name": "name", "type": "String"}, {"name": "type", "type": "String"}],
            "data": [
                {"name": "id", "type": "String"},
                {"name": "email", "type": "Nullable(String)"},
            ],
            "rows": 2,
        })))
        .mount(&ch)
        .await;
    Mock::given(method("POST"))
        .and(body_string_contains("countIf(isNotNull"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "meta": [{"name": "n", "type": "String"}, {"name": "k", "type": "String"}],
            "data": [{"n": "200", "k": "150"}],
            "rows": 1,
        })))
        .mount(&ch)
        .await;
    // Anything else (table creation, the insert of a run, an alter) answers
    // an empty 200.
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "meta": [], "data": [], "rows": 0,
        })))
        .mount(&ch)
        .await;
    let mut overrides = std::collections::HashMap::new();
    overrides.insert("CH_URL".to_owned(), ch.uri());
    let TestApp { router, pool } = spin_up_with_env(&overrides).await;
    (router, pool, ch)
}

async fn call(
    router: &axum::Router,
    cookie: &str,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let resp = router
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .header("content-type", "application/json")
                .header("cookie", cookie)
                .body(Body::from(body.map(|b| b.to_string()).unwrap_or_default()))
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

async fn rule(pool: &PgPool, name: &str, threshold: &str) -> String {
    create_quality_rule(
        pool,
        &CreateQualityRuleInput {
            name: name.to_owned(),
            asset: "silver.orders".to_owned(),
            dimension: "completeness".to_owned(),
            threshold: threshold.to_owned(),
            severity: "high".to_owned(),
        },
    )
    .await
    .unwrap()
    .id
}

/// A session for a principal who may run a rule, change one and resolve an
/// incident. None of the requests below logs out, so one session serves a test.
async fn admin(pool: &PgPool) -> String {
    // PR #106 review R1 SHOULD-FIX DATA-14 1 / D4: *:* passes catalog tenant gate
    let user = create_principal_with_permissions(pool, "*:*,query:read,governance:write").await;
    session_cookie_for_user(pool, user).await
}

async fn incidents(router: &axum::Router, cookie: &str, status: &str) -> Vec<Value> {
    let (code, body) = call(
        router,
        cookie,
        "GET",
        &format!("/api/governance/quality/incidents?status={status}"),
        None,
    )
    .await;
    assert_eq!(code, StatusCode::OK, "{body}");
    body["incidents"].as_array().unwrap().clone()
}

async fn sent_to_clickhouse(ch: &MockServer) -> Vec<String> {
    ch.received_requests()
        .await
        .unwrap()
        .iter()
        .map(|r| String::from_utf8_lossy(&r.body).into_owned())
        .collect()
}

/// `DATA-14-AC1`: a 'no nulls' style check that fails opens an incident,
/// and a second failure of the same rule does not open another.
#[tokio::test]
async fn a_failed_run_opens_one_incident_and_a_second_failure_moves_last_seen() {
    let (router, pool, _ch) = app().await;
    let cookie = admin(&pool).await;
    let id = rule(&pool, "email_filled", "email not null >= 90%").await;

    let (code, run) = call(
        &router,
        &cookie,
        "POST",
        &format!("/api/governance/quality/{id}/run"),
        None,
    )
    .await;
    assert_eq!(code, StatusCode::OK, "{run}");
    assert_eq!(run["status"], "failed");

    let open = incidents(&router, &cookie, "open").await;
    assert_eq!(open.len(), 1, "{open:?}");
    assert_eq!(open[0]["asset"], "silver.orders");
    assert_eq!(open[0]["ruleId"], id.as_str());
    assert_eq!(open[0]["ruleName"], "email_filled");
    assert_eq!(open[0]["kind"], "check");
    assert_eq!(open[0]["status"], "open");
    let detail = open[0]["detail"].as_str().unwrap();
    assert!(detail.contains("75.0% not null"), "{detail}");
    assert!(detail.contains("email not null >= 90%"), "{detail}");

    let (code, _) = call(
        &router,
        &cookie,
        "POST",
        &format!("/api/governance/quality/{id}/run"),
        None,
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    let again = incidents(&router, &cookie, "open").await;
    assert_eq!(
        again.len(),
        1,
        "a second failure must not open a second incident"
    );
    assert_eq!(again[0]["id"], open[0]["id"]);
    assert_eq!(again[0]["openedAt"], open[0]["openedAt"]);
    assert!(again[0]["lastSeenAt"].as_str() >= open[0]["lastSeenAt"].as_str());
}

/// A run that passes opens nothing, and does not close an incident that is
/// already open: that is a person's call.
#[tokio::test]
async fn a_passing_run_opens_nothing_and_leaves_an_open_incident_open() {
    let (router, pool, _ch) = app().await;
    let cookie = admin(&pool).await;
    let ok = rule(&pool, "email_mostly_filled", "email not null >= 70%").await;
    let (_, run) = call(
        &router,
        &cookie,
        "POST",
        &format!("/api/governance/quality/{ok}/run"),
        None,
    )
    .await;
    assert_eq!(run["status"], "passed");
    assert!(incidents(&router, &cookie, "open").await.is_empty());

    let strict = rule(&pool, "email_strict", "email not null >= 90%").await;
    call(
        &router,
        &cookie,
        "POST",
        &format!("/api/governance/quality/{strict}/run"),
        None,
    )
    .await;
    assert_eq!(incidents(&router, &cookie, "open").await.len(), 1);

    // The rule is loosened; it now passes; the incident stays.
    let (code, _) = call(
        &router,
        &cookie,
        "PUT",
        &format!("/api/governance/quality/{strict}"),
        Some(json!({"asset": "silver.orders", "threshold": "email not null >= 70%", "severity": "high"})),
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    let (_, run) = call(
        &router,
        &cookie,
        "POST",
        &format!("/api/governance/quality/{strict}/run"),
        None,
    )
    .await;
    assert_eq!(run["status"], "passed");
    assert_eq!(incidents(&router, &cookie, "open").await.len(), 1);
}

#[tokio::test]
async fn resolving_needs_a_reason_and_an_open_incident() {
    let (router, pool, _ch) = app().await;
    let cookie = admin(&pool).await;
    let id = rule(&pool, "email_filled", "email not null >= 90%").await;
    call(
        &router,
        &cookie,
        "POST",
        &format!("/api/governance/quality/{id}/run"),
        None,
    )
    .await;
    let incident = incidents(&router, &cookie, "open").await[0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let uri = format!("/api/governance/quality/incidents/{incident}/resolve");

    for body in [
        json!({"resolution": ""}),
        json!({"resolution": "   "}),
        json!({}),
    ] {
        let (code, _) = call(&router, &cookie, "POST", &uri, Some(body)).await;
        assert_eq!(code, StatusCode::BAD_REQUEST);
    }
    assert_eq!(
        incidents(&router, &cookie, "open").await.len(),
        1,
        "a refused resolve changes nothing"
    );

    let (code, resolved) = call(
        &router,
        &cookie,
        "POST",
        &uri,
        Some(json!({"resolution": "Source fixed upstream"})),
    )
    .await;
    assert_eq!(code, StatusCode::OK, "{resolved}");
    assert_eq!(resolved["status"], "resolved");
    assert_eq!(resolved["resolution"], "Source fixed upstream");
    assert_eq!(resolved["resolvedBy"], "Single Perm Test User");

    let (code, _) = call(
        &router,
        &cookie,
        "POST",
        &uri,
        Some(json!({"resolution": "again"})),
    )
    .await;
    assert_eq!(code, StatusCode::CONFLICT);
    let (code, _) = call(
        &router,
        &cookie,
        "POST",
        "/api/governance/quality/incidents/00000000-0000-4000-8000-000000000000/resolve",
        Some(json!({"resolution": "x"})),
    )
    .await;
    assert_eq!(code, StatusCode::NOT_FOUND);
    let (code, _) = call(
        &router,
        &cookie,
        "POST",
        "/api/governance/quality/incidents/not-a-uuid/resolve",
        Some(json!({"resolution": "x"})),
    )
    .await;
    assert_eq!(code, StatusCode::NOT_FOUND);

    assert!(incidents(&router, &cookie, "open").await.is_empty());
    assert_eq!(incidents(&router, &cookie, "resolved").await.len(), 1);
    assert_eq!(incidents(&router, &cookie, "all").await.len(), 1);

    // The failure is still there: a new run opens a NEW incident.
    call(
        &router,
        &cookie,
        "POST",
        &format!("/api/governance/quality/{id}/run"),
        None,
    )
    .await;
    assert_eq!(incidents(&router, &cookie, "open").await.len(), 1);
    assert_eq!(incidents(&router, &cookie, "all").await.len(), 2);

    let (code, _) = call(
        &router,
        &cookie,
        "GET",
        "/api/governance/quality/incidents?status=bogus",
        None,
    )
    .await;
    assert_eq!(code, StatusCode::BAD_REQUEST);
}

/// Resolving is audited on the table, like every other change to its rules.
#[tokio::test]
async fn resolving_an_incident_is_audited_on_its_table() {
    let (router, pool, _ch) = app().await;
    let cookie = admin(&pool).await;
    let id = rule(&pool, "email_filled", "email not null >= 90%").await;
    call(
        &router,
        &cookie,
        "POST",
        &format!("/api/governance/quality/{id}/run"),
        None,
    )
    .await;
    let incident = incidents(&router, &cookie, "open").await[0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    call(
        &router,
        &cookie,
        "POST",
        &format!("/api/governance/quality/incidents/{incident}/resolve"),
        Some(json!({"resolution": "ok"})),
    )
    .await;
    let rows: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_event WHERE action = 'quality.incident_resolve' AND resource_id = 'silver.orders'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(rows, 1);
}

/// Deleting a rule resolves its open incidents with the resolution "rule
/// deleted" and the deleting user; the incidents stay as history.
#[tokio::test]
async fn deleting_a_rule_resolves_its_open_incidents() {
    let (router, pool, ch) = app().await;
    let cookie = admin(&pool).await;
    let id = rule(&pool, "email_filled", "email not null >= 90%").await;
    call(
        &router,
        &cookie,
        "POST",
        &format!("/api/governance/quality/{id}/run"),
        None,
    )
    .await;
    assert_eq!(incidents(&router, &cookie, "open").await.len(), 1);

    let (code, _) = call(
        &router,
        &cookie,
        "DELETE",
        &format!("/api/governance/quality/{id}"),
        None,
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    assert!(incidents(&router, &cookie, "open").await.is_empty());
    let resolved = incidents(&router, &cookie, "resolved").await;
    assert_eq!(resolved.len(), 1);
    assert_eq!(resolved[0]["resolution"], "rule deleted");
    assert_eq!(resolved[0]["resolvedBy"], "Single Perm Test User");
    // The rule is gone, so its name is no longer known; the id stays.
    assert_eq!(resolved[0]["ruleName"], Value::Null);
    assert_eq!(resolved[0]["ruleId"], id.as_str());

    let sent = sent_to_clickhouse(&ch).await;
    assert!(
        sent.iter()
            .any(|b| b.contains("ALTER TABLE console.quality_run DELETE")),
        "deleting a rule still forgets its runs"
    );
}

/// `DATA-14` (F2): editing a rule keeps the runs it has (each now records
/// the threshold it ran with), and the old run is not the edited rule's
/// verdict.
#[tokio::test]
async fn editing_a_rule_keeps_its_runs_and_records_the_threshold_each_ran_with() {
    let (router, pool, ch) = app().await;
    let cookie = admin(&pool).await;
    let id = rule(&pool, "email_filled", "email not null >= 90%").await;
    call(
        &router,
        &cookie,
        "POST",
        &format!("/api/governance/quality/{id}/run"),
        None,
    )
    .await;

    let (code, edited) = call(
        &router,
        &cookie,
        "PUT",
        &format!("/api/governance/quality/{id}"),
        Some(json!({"asset": "silver.orders", "threshold": "email not null >= 70%", "severity": "high"})),
    )
    .await;
    assert_eq!(code, StatusCode::OK, "{edited}");

    let sent = sent_to_clickhouse(&ch).await;
    assert!(
        !sent
            .iter()
            .any(|b| b.contains("ALTER TABLE console.quality_run DELETE")),
        "an edit must not erase the rule's runs"
    );
    let insert = sent
        .iter()
        .find(|b| b.contains("INSERT INTO console.quality_run"))
        .expect("the run was recorded");
    assert!(insert.contains("threshold"), "{insert}");
    assert!(insert.contains("'email not null >= 90%'"), "{insert}");
}

/// A rule's history lists every run, newest first, each with the threshold it
/// ran with, and survives an edit (`DATA-14`, F2 and F7). The stand-in answers
/// the history query with two rows, one from before the threshold was kept.
#[tokio::test]
async fn a_rules_history_lists_its_runs_with_the_threshold_each_ran_with() {
    let (router, pool, ch) = app().await;
    let cookie = admin(&pool).await;
    let id = rule(&pool, "email_filled", "email not null >= 90%").await;
    Mock::given(method("POST"))
        .and(body_string_contains(
            "FROM console.quality_run WHERE rule_id",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "meta": [],
            "rows": 2,
            "data": [
                {"status": "passed", "value": "75.0% not null", "run_by": "schedule",
                 "threshold": "email not null >= 70%", "ran_at": "2026-10-02T09:00:00Z"},
                {"status": "failed", "value": "75.0% not null", "run_by": "Ayu",
                 "threshold": "", "ran_at": "2026-10-01T09:00:00Z"},
            ],
        })))
        .with_priority(1)
        .mount(&ch)
        .await;

    let (code, body) = call(
        &router,
        &cookie,
        "GET",
        &format!("/api/governance/quality/{id}/runs"),
        None,
    )
    .await;
    assert_eq!(code, StatusCode::OK, "{body}");
    let runs = body["runs"].as_array().unwrap();
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0]["status"], "passed");
    assert_eq!(runs[0]["runBy"], "schedule");
    assert_eq!(runs[0]["threshold"], "email not null >= 70%");
    assert_eq!(runs[0]["at"], "2026-10-02T09:00:00Z");
    assert_eq!(
        runs[1]["threshold"],
        Value::Null,
        "a run from before the threshold was kept"
    );

    let sent = sent_to_clickhouse(&ch).await;
    let query = sent
        .iter()
        .find(|b| b.contains("FROM console.quality_run WHERE rule_id"))
        .unwrap();
    assert!(query.contains(&format!("'{id}'")), "{query}");
    assert!(query.contains("ORDER BY at DESC LIMIT 50"), "{query}");

    let (code, _) = call(
        &router,
        &cookie,
        "GET",
        "/api/governance/quality/00000000-0000-4000-8000-000000000000/runs",
        None,
    )
    .await;
    assert_eq!(code, StatusCode::NOT_FOUND);
}
