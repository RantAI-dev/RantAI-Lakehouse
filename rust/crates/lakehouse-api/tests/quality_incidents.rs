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
    TestApp, create_assistant_principal, create_principal_with_permissions,
    session_cookie_for_user, spin_up_with_env,
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
    // The deployment names the tenant that owns the shared catalog: the
    // seeded identity migration creates four tenants, so without it every
    // caller but an unrestricted one is refused (PR review round 1 R3).
    overrides.insert(
        "CATALOG_TENANT_ID".to_owned(),
        common::CATALOG_TENANT.to_owned(),
    );
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

/// Narrows a person's only role to `silver.orders` — the three statements
/// `tests/table_access.rs` writes inline. Both the form's test and the
/// assistant's use it, so the narrowing is one thing (`AGENTS.md` rule 4).
async fn narrow_to_orders(pool: &PgPool, user: uuid::Uuid) {
    let role: uuid::Uuid =
        sqlx::query_scalar("SELECT role_id FROM app_user_role WHERE user_id = $1")
            .bind(user)
            .fetch_one(pool)
            .await
            .unwrap();
    sqlx::query("UPDATE role SET read_scope = 'listed' WHERE id = $1")
        .bind(role)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM role_read_grant WHERE role_id = $1")
        .bind(role)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO role_read_grant (role_id, schema_name, table_name) VALUES ($1, 'silver', 'orders')",
    )
    .bind(role)
    .execute(pool)
    .await
    .unwrap();
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
    assert_eq!(
        incidents(&router, &cookie, "open").await,
        [] as [serde_json::Value; 0]
    );

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

    assert_eq!(
        incidents(&router, &cookie, "open").await,
        [] as [serde_json::Value; 0]
    );
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
    assert_eq!(
        incidents(&router, &cookie, "open").await,
        [] as [serde_json::Value; 0]
    );
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

/// QA `LH-DATA-14-01` F2 (`DATA-54`): a rule is written for a table that is
/// there, and one that does not exist is refused and stores nothing. "Not
/// there" and "not yours" are one sentence, so the answer says neither.
#[tokio::test]
async fn a_rule_for_a_table_that_is_not_there_is_refused_and_nothing_is_stored() {
    let (router, pool, ch) = app().await;
    let cookie = admin(&pool).await;
    Mock::given(method("POST"))
        .and(body_string_contains("system.columns"))
        .and(body_string_contains("no_such_table"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "meta": [], "data": [], "rows": 0,
        })))
        .with_priority(1)
        .mount(&ch)
        .await;

    let body = |name: &str, asset: &str| {
        json!({
            "name": name, "asset": asset, "dimension": "volume",
            "threshold": "rows >= 1", "severity": "high",
        })
    };
    let (code, refused) = call(
        &router,
        &cookie,
        "POST",
        "/api/governance/quality",
        Some(body("t8_missing", "silver.no_such_table")),
    )
    .await;
    assert_eq!(code, StatusCode::BAD_REQUEST, "{refused}");
    assert!(
        refused["error"]
            .as_str()
            .unwrap()
            .contains("is not a table you can read"),
        "{refused}"
    );
    let stored: i64 =
        sqlx::query_scalar("SELECT count(*) FROM quality_rule WHERE name = 't8_missing'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(stored, 0, "a refused rule is not stored");

    let (code, created) = call(
        &router,
        &cookie,
        "POST",
        "/api/governance/quality",
        Some(body("t8_present", "silver.orders")),
    )
    .await;
    assert_eq!(code, StatusCode::CREATED, "{created}");

    for asset in ["orders", "lake.orders"] {
        let (code, refused) = call(
            &router,
            &cookie,
            "POST",
            "/api/governance/quality",
            Some(body("t8_other", asset)),
        )
        .await;
        assert_eq!(code, StatusCode::BAD_REQUEST, "{asset}: {refused}");
        assert!(
            refused["error"]
                .as_str()
                .unwrap()
                .contains("is not a table you can read"),
            "{asset}: {refused}"
        );
    }
}

/// QA `LH-DATA-14-01` F2 (`DATA-54`): a table the author's roles hide is
/// refused with the same sentence as a missing one, so the answer is not an
/// oracle. The two refusals come from different branches: the narrowed
/// caller's hidden table is refused by the role's access, and an unrestricted
/// caller's missing one by the existence check (review round 1 R6 — comparing
/// two access refusals proved nothing).
#[tokio::test]
async fn a_rule_for_a_table_the_role_may_not_read_is_refused_with_the_same_sentence() {
    let (router, pool, ch) = app().await;
    // The stand-in answers no columns for the table that is not there.
    Mock::given(method("POST"))
        .and(body_string_contains("system.columns"))
        .and(body_string_contains("no_such_table"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "meta": [], "data": [], "rows": 0,
        })))
        .with_priority(1)
        .mount(&ch)
        .await;
    let user = create_principal_with_permissions(&pool, "governance:write,governance:read").await;
    narrow_to_orders(&pool, user).await;
    // The person is a member of the tenant that owns the catalog, so the
    // catalog's tenant rule lets them through to the role's own table access
    // (PR review round 1 R3).
    common::in_the_catalog_tenant(&pool, user).await;
    let cookie = session_cookie_for_user(&pool, user).await;

    let body = |asset: &str| {
        json!({
            "name": "t8_narrowed", "asset": asset, "dimension": "volume",
            "threshold": "rows >= 1", "severity": "high",
        })
    };
    let (code, hidden) = call(
        &router,
        &cookie,
        "POST",
        "/api/governance/quality",
        Some(body("silver.customers")),
    )
    .await;
    assert_eq!(code, StatusCode::BAD_REQUEST, "{hidden}");
    // A caller whose role hides nothing asks for a table that is not there:
    // the existence check's refusal, not the access check's.
    let admin_cookie = admin(&pool).await;
    let (code, missing) = call(
        &router,
        &admin_cookie,
        "POST",
        "/api/governance/quality",
        Some(json!({
            "name": "t8_admin_missing", "asset": "silver.no_such_table",
            "dimension": "volume", "threshold": "rows >= 1", "severity": "high",
        })),
    )
    .await;
    assert_eq!(code, StatusCode::BAD_REQUEST, "{missing}");
    let (hidden, missing) = (
        hidden["error"].as_str().unwrap(),
        missing["error"].as_str().unwrap(),
    );
    assert_eq!(
        hidden.replace("silver.customers", "T"),
        missing.replace("silver.no_such_table", "T"),
        "a hidden table and a missing one read alike"
    );

    let (code, created) = call(
        &router,
        &cookie,
        "POST",
        "/api/governance/quality",
        Some(body("silver.orders")),
    )
    .await;
    assert_eq!(code, StatusCode::CREATED, "{created}");
}

/// QA `LH-DATA-14-01` F2 (`DATA-54`), review round 1 R1: the assistant drafts a
/// rule as the person behind the chat, so a table their role hides is refused
/// with the very sentence the form gives that person, and nothing is stored.
#[tokio::test]
async fn the_assistant_cannot_draft_a_rule_on_a_table_the_person_may_not_read() {
    let (router, pool, _ch) = app().await;
    let user = create_assistant_principal(&pool, "governance:write,governance:read").await;
    narrow_to_orders(&pool, user).await;
    common::in_the_catalog_tenant(&pool, user).await;
    let cookie = session_cookie_for_user(&pool, user).await;

    let draft = |name: &str, asset: &str| {
        json!({
            "tool": "draft_quality_rule",
            "args": {
                "name": name, "asset": asset, "dimension": "volume",
                "threshold": "rows >= 1", "severity": "high", "confirmed": true,
            },
            "mode": "build",
        })
    };

    let (code, tool) = call(
        &router,
        &cookie,
        "POST",
        "/api/ai/tool",
        Some(draft("r1_hidden", "silver.customers")),
    )
    .await;
    assert_eq!(code, StatusCode::OK, "{tool}");
    assert_eq!(tool["outcome"], "failed", "{tool}");

    // The sentence the form gives the same person, for the same table.
    let (code, form) = call(
        &router,
        &cookie,
        "POST",
        "/api/governance/quality",
        Some(json!({
            "name": "r1_hidden_form", "asset": "silver.customers", "dimension": "volume",
            "threshold": "rows >= 1", "severity": "high",
        })),
    )
    .await;
    assert_eq!(code, StatusCode::BAD_REQUEST, "{form}");
    assert_eq!(
        tool["result"]["error"], form["error"],
        "the assistant refuses a hidden table as the form does"
    );

    let stored: i64 =
        sqlx::query_scalar("SELECT count(*) FROM quality_rule WHERE name LIKE 'r1_hidden%'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(stored, 0, "a refused draft stores nothing");

    let (_, tool) = call(
        &router,
        &cookie,
        "POST",
        "/api/ai/tool",
        Some(draft("r1_orders", "silver.orders")),
    )
    .await;
    assert_eq!(tool["outcome"], "executed", "{tool}");
}

/// A caller outside the tenant that owns the shared catalog is refused before
/// the table is asked about (PR review round 1 R3, plan gap in T8): the
/// answer is the catalog's own refusal, the same for a table that is there
/// and one that is not, so it says neither.
#[tokio::test]
async fn a_caller_outside_the_catalog_tenant_is_refused_before_the_table_is_asked_about() {
    let (router, pool, _ch) = app().await;
    let outsider =
        create_principal_with_permissions(&pool, "governance:write,governance:read").await;
    // Seeded tenant 2, not the one the catalog names.
    sqlx::query("INSERT INTO app_user_tenant (user_id, tenant_id) VALUES ($1, $2)")
        .bind(outsider)
        .bind(
            "11111111-1111-4111-8111-000000000002"
                .parse::<uuid::Uuid>()
                .unwrap(),
        )
        .execute(&pool)
        .await
        .unwrap();
    let cookie = session_cookie_for_user(&pool, outsider).await;
    let body = |asset: &str| {
        json!({
            "name": "r3_outside", "asset": asset, "dimension": "volume",
            "threshold": "rows >= 1", "severity": "high",
        })
    };

    let (code, present) = call(
        &router,
        &cookie,
        "POST",
        "/api/governance/quality",
        Some(body("silver.orders")),
    )
    .await;
    assert_eq!(code, StatusCode::FORBIDDEN, "{present}");
    let (code, missing) = call(
        &router,
        &cookie,
        "POST",
        "/api/governance/quality",
        Some(body("silver.no_such_table")),
    )
    .await;
    assert_eq!(code, StatusCode::FORBIDDEN, "{missing}");
    assert_eq!(
        present, missing,
        "the tenant refusal is the same for every table, so it names none"
    );
    let stored: i64 =
        sqlx::query_scalar("SELECT count(*) FROM quality_rule WHERE name = 'r3_outside'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        stored, 0,
        "a caller outside the catalog tenant stores nothing"
    );
}

/// QA `LH-DATA-14-01` F2 (`DATA-54`): a rewrite that changes the table is
/// checked, and a refusal leaves the rule on its old table. A rewrite that
/// keeps the table is not checked, so a rule on a dropped table can still be
/// edited.
#[tokio::test]
async fn changing_a_rule_to_a_table_that_is_not_there_is_refused_and_the_rule_keeps_its_table() {
    let (router, pool, ch) = app().await;
    let cookie = admin(&pool).await;
    let id = rule(&pool, "email_filled", "email not null >= 90%").await;
    Mock::given(method("POST"))
        .and(body_string_contains("system.columns"))
        .and(body_string_contains("no_such_table"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "meta": [], "data": [], "rows": 0,
        })))
        .with_priority(1)
        .mount(&ch)
        .await;

    let (code, refused) = call(
        &router,
        &cookie,
        "PUT",
        &format!("/api/governance/quality/{id}"),
        Some(json!({
            "asset": "silver.no_such_table",
            "threshold": "email not null >= 90%", "severity": "high",
        })),
    )
    .await;
    assert_eq!(code, StatusCode::BAD_REQUEST, "{refused}");
    let asset: String = sqlx::query_scalar("SELECT asset FROM quality_rule WHERE name = $1")
        .bind("email_filled")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(asset, "silver.orders", "a refused rewrite changes nothing");

    // The threshold change with the table kept is not checked.
    let (code, edited) = call(
        &router,
        &cookie,
        "PUT",
        &format!("/api/governance/quality/{id}"),
        Some(json!({
            "asset": "silver.orders",
            "threshold": "email not null >= 70%", "severity": "high",
        })),
    )
    .await;
    assert_eq!(code, StatusCode::OK, "{edited}");
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
