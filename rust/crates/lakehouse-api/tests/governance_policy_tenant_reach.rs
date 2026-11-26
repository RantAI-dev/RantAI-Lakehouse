//! The policy writers and the policy preview respect the tenant reach
//! (`SEC-83`), the rule `SEC-80` gave the quality writers.
//!
//! Two tenants share one deployment: Meridian Group owns the shared catalog
//! (`CATALOG_TENANT_ID`) and the upload table `group_orders`; Meridian Retail
//! owns the upload table `retail_orders`. A `policy:write` holder of the
//! catalog's tenant may not bind a policy to, list, enforce, suspend, delete
//! over, or count the rows of the other tenant's table:
//!
//! - creating a policy whose `conditions` bind the other tenant's table is
//!   refused with the one sentence a missing table gets, and nothing is
//!   stored;
//! - enforcing, suspending or deleting the other tenant's policy answers the
//!   same `404` an unknown id gets, and changes nothing;
//! - the policy list leaves the other tenant's policies out;
//! - the preview answers an out-of-reach table exactly as a table
//!   `system.columns` has never heard of, and runs no count query.
//!
//! A Bronze table's presence is answered through the Iceberg query database
//! (`DESCRIBE TABLE icecat_api.bronze.<table>` on the `ClickHouse` stand-in),
//! and a published gold table's presence the same way
//! (`DESCRIBE TABLE icecat_api.gold.<table>`), so "there" and "checked" are
//! both under the test's control.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use crate::common;

use std::collections::HashMap;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use lakehouse_store::uploads::claim_table;
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;
use wiremock::matchers::{body_string_contains, method};
use wiremock::{Mock, MockServer, ResponseTemplate};

use common::{
    CATALOG_TENANT, TestApp, create_principal_with_permissions, session_cookie_for_user,
    spin_up_with_env,
};

/// The tenant of Meridian Retail: not the catalog's owner.
const OTHER_TENANT: &str = "11111111-1111-4111-8111-000000000002";

/// The upload table the catalog's own tenant holds.
const GROUP_TABLE: &str = "group_orders";
/// The upload table the other tenant holds.
const RETAIL_TABLE: &str = "retail_orders";
/// The published gold table a policy may bind, as the G8 gate's Trino leg
/// does (`ops/g8/g8_governance_test.py` publishes its own mart through the
/// Gold export and authors its policy on `gold.g8_gate_subjects`).
const GOLD_TABLE: &str = "g8_gate_subjects";

/// Holds exactly what authoring a policy needs.
const POLICY_WRITER: &str = "policy:write";
/// Holds exactly what reading the policy list needs.
const POLICY_READER: &str = "policy:read";

/// The refusal a policy writer gives for a table the caller may not read
/// (`routes::governance::unreadable_policy_table`): the policy path, unlike
/// the quality path, also binds published gold tables, so its sentence
/// offers the fourth zone.
fn unreadable_sentence(asset: &str) -> String {
    format!(
        "\"{asset}\" is not a table you can read. Pick the table as \
         silver.<table>, serving.<table>, bronze.<table> or gold.<table>."
    )
}

/// The answer of a policy the caller may not reach: the same one an unknown
/// id gets.
const POLICY_NOT_FOUND: &str = "Policy not found";

/// Both tenants claim their upload table, the way an upload does.
async fn claim_the_uploads(pool: &PgPool) {
    for (tenant, table) in [(CATALOG_TENANT, GROUP_TABLE), (OTHER_TENANT, RETAIL_TABLE)] {
        let claimed = claim_table(
            pool,
            tenant.parse::<Uuid>().expect("a seeded tenant id"),
            table,
            &format!("upload-{table}"),
        )
        .await
        .expect("claim the upload table");
        assert!(claimed, "the tenant must hold the claim on {table}");
    }
}

/// A principal that holds exactly `permissions` and belongs to exactly
/// `tenant`, as a session cookie.
async fn cookie_in_tenant(pool: &PgPool, permissions: &str, tenant: &str) -> String {
    let user = create_principal_with_permissions(pool, permissions).await;
    sqlx::query("INSERT INTO app_user_tenant (user_id, tenant_id) VALUES ($1, $2)")
        .bind(user)
        .bind(tenant.parse::<Uuid>().expect("a seeded tenant id"))
        .execute(pool)
        .await
        .expect("add the membership");
    session_cookie_for_user(pool, user).await
}

/// A principal that holds every permission and belongs to no tenant: an
/// operator, whose reach is everything.
async fn operator_cookie(pool: &PgPool) -> String {
    let user = create_principal_with_permissions(pool, "*:*").await;
    session_cookie_for_user(pool, user).await
}

async fn send(
    router: &axum::Router,
    method: &str,
    uri: &str,
    cookie: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("cookie", cookie);
    let body = match body {
        Some(value) => {
            builder = builder.header("content-type", "application/json");
            Body::from(value.to_string())
        }
        None => Body::empty(),
    };
    let resp = router
        .clone()
        .oneshot(builder.body(body).expect("build request"))
        .await
        .expect("router never fails a request outright");
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.expect("body");
    let body = serde_json::from_slice(&bytes)
        .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()));
    (status, body)
}

/// The `POST /api/governance/policies` body for a policy whose conditions
/// bind `table` with a mask, as the console sends them (`conditions` is the
/// stored JSON string).
fn policy_body(name: &str, table: &str) -> Value {
    json!({
        "name": name,
        "kind": "Row filter",
        "subjects": "Analysts",
        "resources": "tenant-scoped tables",
        "effect": "Permit with obligation",
        "conditions": json!({
            "roles": ["analyst"],
            "table": table,
            "mask": ["email"],
        })
        .to_string(),
    })
}

/// The `POST /api/governance/policies` body for a policy that binds no
/// table at all.
fn policy_body_without_conditions(name: &str) -> Value {
    json!({
        "name": name,
        "kind": "Agent autonomy",
        "subjects": "Every digital employee",
        "resources": "Every tool",
        "effect": "Permit with obligation",
    })
}

/// How many policies named `name` are stored: a refused write must add none.
async fn policies_named(pool: &PgPool, name: &str) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM policy WHERE name = $1")
        .bind(name)
        .fetch_one(pool)
        .await
        .expect("count the policies")
}

/// The stored status of the policy named `name`, when it is there.
async fn status_of_policy(pool: &PgPool, name: &str) -> Option<String> {
    sqlx::query_scalar("SELECT status FROM policy WHERE name = $1")
        .bind(name)
        .fetch_optional(pool)
        .await
        .expect("read the policy")
}

/// Seeds a policy of the other tenant on its own upload table, the way a
/// draft of Retail's own would sit, and returns its id.
async fn seed_a_retail_policy(pool: &PgPool, name: &str, status: &str) -> String {
    let (id,): (Uuid,) = sqlx::query_as(
        "INSERT INTO policy (name, status, kind, subjects, resources, effect, conditions, owner) \
         VALUES ($1, $2, 'Row filter', 'Retail analysts', 'retail tables', \
                 'Permit with obligation', $3, 'Retail') RETURNING id",
    )
    .bind(name)
    .bind(status)
    .bind(
        json!({
            "roles": ["analyst"],
            "table": format!("bronze.{RETAIL_TABLE}"),
            "mask": ["email"],
        })
        .to_string(),
    )
    .fetch_one(pool)
    .await
    .expect("seed the retail policy");
    id.to_string()
}

/// Serves one column for the `DESCRIBE` of each claimed upload table, so both
/// are there; any other `DESCRIBE` answers no columns, so that bronze name is
/// not there (`SEC-80`'s model). The preview's `system.columns` lookup
/// answers one column for every table, and its count answers 7 rows.
/// Anything else answers no rows. Returns the stand-in so a test may count
/// what it was asked.
async fn mount_clickhouse(server: &MockServer) {
    for table in [GROUP_TABLE, RETAIL_TABLE] {
        Mock::given(method("POST"))
            .and(body_string_contains("DESCRIBE TABLE"))
            .and(body_string_contains(format!("bronze.{table}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "meta": [{"name": "name", "type": "String"}, {"name": "type", "type": "String"}],
                "data": [{"name": "email", "type": "String"}],
                "rows": 1,
            })))
            .with_priority(1)
            .mount(server)
            .await;
    }
    // Mounted after the claimed tables, so a tie in priority is broken in
    // their favour: a `bronze.` name no upload claimed answers no columns,
    // so the table is not there.
    Mock::given(method("POST"))
        .and(body_string_contains("DESCRIBE TABLE"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "meta": [],
            "data": [],
            "rows": 0,
        })))
        .with_priority(1)
        .mount(server)
        .await;
    // The preview's column lookup, so an in-reach preview has columns to
    // report; a refused preview never asks.
    Mock::given(method("POST"))
        .and(body_string_contains("system.columns"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "meta": [{"name": "name", "type": "String"}],
            "data": [{"name": "email"}],
            "rows": 1,
        })))
        .with_priority(1)
        .mount(server)
        .await;
    // The preview's count, so an in-reach preview measures something real to
    // assert; a refused preview must never add to it.
    Mock::given(method("POST"))
        .and(body_string_contains("count()"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "meta": [{"name": "n", "type": "String"}],
            "data": [{"n": "7"}],
            "rows": 1,
        })))
        .with_priority(1)
        .mount(server)
        .await;
    // Everything else (the list's registered-slug lookup): no rows.
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "data": [] })))
        .mount(server)
        .await;
}

/// How many requests the stand-in has answered that measure rows (a body
/// naming `count()`): a refused preview must add none. A `DESCRIBE` or
/// `system.columns` lookup reads the table's shape and is not counted
/// (the `SEC-80` tests count the same way).
async fn count_queries(ch: &MockServer) -> usize {
    ch.received_requests()
        .await
        .expect("mock records requests")
        .iter()
        .filter(|r| String::from_utf8_lossy(&r.body).contains("count()"))
        .count()
}

/// The deployment of the policy writers: the catalog's tenant named, an
/// Iceberg query database, and the `ClickHouse` stand-in above.
async fn spin_up_for_the_policy_writers(ch: &MockServer) -> TestApp {
    let mut env = HashMap::new();
    env.insert("CH_URL".to_owned(), ch.uri());
    env.insert("CATALOG_TENANT_ID".to_owned(), CATALOG_TENANT.to_owned());
    env.insert("ICEBERG_QUERY_DB".to_owned(), "icecat_api".to_owned());
    spin_up_with_env(&env).await
}

// ── Creating a policy respects the reach (`SEC-83` D1) ──────────────────

/// A `policy:write` holder of the catalog's tenant cannot bind a policy to
/// another tenant's upload table: the refusal is the one sentence a table
/// that is not there gets (so the answer says neither), and nothing is
/// stored. Their own table is allowed.
#[tokio::test]
async fn a_policy_writer_of_the_catalog_owner_cannot_bind_another_tenants_upload_table() {
    let ch = MockServer::start().await;
    mount_clickhouse(&ch).await;
    let TestApp { router, pool } = spin_up_for_the_policy_writers(&ch).await;
    claim_the_uploads(&pool).await;
    let writer = cookie_in_tenant(&pool, POLICY_WRITER, CATALOG_TENANT).await;
    let uri = "/api/governance/policies";

    // The other tenant's table: refused, and nothing stored.
    let (status, body) = send(
        &router,
        "POST",
        uri,
        &writer,
        Some(policy_body(
            "writer on retail",
            &format!("bronze.{RETAIL_TABLE}"),
        )),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(
        body["error"],
        json!(unreadable_sentence(&format!("bronze.{RETAIL_TABLE}"))),
        "{body}"
    );
    assert_eq!(policies_named(&pool, "writer on retail").await, 0);

    // A name no table has: the identical status, and the identical sentence
    // for that name — the answer cannot tell a missing table from another
    // tenant's one.
    let (status, body) = send(
        &router,
        "POST",
        uri,
        &writer,
        Some(policy_body("writer on nobody", "bronze.nobody_orders")),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(
        body["error"],
        json!(unreadable_sentence("bronze.nobody_orders")),
        "{body}"
    );
    assert_eq!(policies_named(&pool, "writer on nobody").await, 0);

    // The writer's own upload table is allowed.
    let (status, body) = send(
        &router,
        "POST",
        uri,
        &writer,
        Some(policy_body(
            "writer on group",
            &format!("bronze.{GROUP_TABLE}"),
        )),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(policies_named(&pool, "writer on group").await, 1);

    // A policy that binds no table is not checked: it is saved as before.
    let (status, body) = send(
        &router,
        "POST",
        uri,
        &writer,
        Some(policy_body_without_conditions("writer without conditions")),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(policies_named(&pool, "writer without conditions").await, 1);
}

/// A table the engine did not answer about is a `503` that says so — for
/// another tenant's name and for a name no table has alike, so the outage
/// cannot be used as an oracle either — and nothing is stored.
#[tokio::test]
async fn a_policy_for_a_table_that_cannot_be_checked_is_a_503_and_nothing_is_stored() {
    let ch = MockServer::start().await;
    // Every presence check fails: an outage answers every name the same way.
    // No other mock is mounted: a `DESCRIBE` is a `503`, everything else no
    // rows.
    Mock::given(method("POST"))
        .and(body_string_contains("DESCRIBE TABLE"))
        .respond_with(ResponseTemplate::new(500).set_body_string("Code: 999. DB::Exception"))
        .with_priority(1)
        .mount(&ch)
        .await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "data": [] })))
        .mount(&ch)
        .await;
    let TestApp { router, pool } = spin_up_for_the_policy_writers(&ch).await;
    claim_the_uploads(&pool).await;
    let writer = cookie_in_tenant(&pool, POLICY_WRITER, CATALOG_TENANT).await;
    let uri = "/api/governance/policies";

    for (name, table) in [
        ("outage on retail", format!("bronze.{RETAIL_TABLE}")),
        ("outage on nobody", "bronze.nobody_orders".to_owned()),
    ] {
        let (status, body) = send(
            &router,
            "POST",
            uri,
            &writer,
            Some(policy_body(name, &table)),
        )
        .await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{table}: {body}");
        assert_eq!(
            body["error"],
            json!("The table could not be checked. Try again."),
            "{table}: a check that could not be made says so, in a fixed sentence"
        );
        assert_eq!(policies_named(&pool, name).await, 0, "{table}");
    }
}

// ── A policy may bind a published gold table ────────────────────────────

/// The G8 gate's Trino leg in miniature (`ops/g8/g8_governance_test.py`
/// publishes its own mart through the Gold export and authors its policy on
/// `gold.g8_gate_subjects`): a policy whose condition binds a published gold
/// table is authored. The catalog's tenant reaches it — a gold name belongs
/// to no upload, so it is shared — when the query database describes the
/// published table; a gold name nothing describes is refused with the one
/// sentence, and so is the same present gold table for the other tenant's
/// writer, whose reach no shared name enters. The sentence cannot say which.
#[tokio::test]
async fn a_policy_may_bind_a_published_gold_table_and_other_gold_names_are_refused_alike() {
    let ch = MockServer::start().await;
    // The published gold table, as the query database describes it: one
    // column. Mounted before `mount_clickhouse`'s catch-all `DESCRIBE`, so
    // this more specific match is the one that answers for `gold.<table>`.
    Mock::given(method("POST"))
        .and(body_string_contains("DESCRIBE TABLE"))
        .and(body_string_contains(format!("gold.{GOLD_TABLE}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "meta": [{"name": "name", "type": "String"}, {"name": "type", "type": "String"}],
            "data": [{"name": "email", "type": "String"}],
            "rows": 1,
        })))
        .with_priority(1)
        .mount(&ch)
        .await;
    mount_clickhouse(&ch).await;
    let TestApp { router, pool } = spin_up_for_the_policy_writers(&ch).await;
    let writer = cookie_in_tenant(&pool, POLICY_WRITER, CATALOG_TENANT).await;
    let uri = "/api/governance/policies";
    let gold_name = format!("gold.{GOLD_TABLE}");

    // The published gold table: authored, as the G8 gate's own request is.
    let (status, body) = send(
        &router,
        "POST",
        uri,
        &writer,
        Some(policy_body("writer on gold", &gold_name)),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(policies_named(&pool, "writer on gold").await, 1);

    // A gold name nothing describes: not there — the identical sentence.
    let (status, body) = send(
        &router,
        "POST",
        uri,
        &writer,
        Some(policy_body("writer on missing gold", "gold.nobody_gold")),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(
        body["error"],
        json!(unreadable_sentence("gold.nobody_gold")),
        "{body}"
    );
    assert_eq!(policies_named(&pool, "writer on missing gold").await, 0);

    // The same present gold table, from the other tenant's writer: a gold
    // name is outside a tenant writer's reach, whatever the query database
    // answers about it. The same sentence as a gold name that is not there.
    let other = cookie_in_tenant(&pool, POLICY_WRITER, OTHER_TENANT).await;
    let (status, body) = send(
        &router,
        "POST",
        uri,
        &other,
        Some(policy_body("other writer on gold", &gold_name)),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(
        body["error"],
        json!(unreadable_sentence(&gold_name)),
        "{body}"
    );
    assert_eq!(policies_named(&pool, "other writer on gold").await, 0);
}

// ── Status and delete answer an out-of-reach policy as unknown (`SEC-83` D3) ──

/// Another tenant's policy is unknown to the status route: the same `404` an
/// unknown id gets, and the policy stays exactly as it was. The holder's own
/// policy switches as before, and an operator reaches every policy.
#[tokio::test]
async fn another_tenants_policy_is_unknown_to_the_status_route_and_stays_enforced() {
    let ch = MockServer::start().await;
    mount_clickhouse(&ch).await;
    let TestApp { router, pool } = spin_up_for_the_policy_writers(&ch).await;
    claim_the_uploads(&pool).await;
    let retail_policy = seed_a_retail_policy(&pool, "retail mask", "ready").await;
    let writer = cookie_in_tenant(&pool, POLICY_WRITER, CATALOG_TENANT).await;
    let own = policy_body("writer on group", &format!("bronze.{GROUP_TABLE}"));

    // The other tenant's enforced policy: unknown, and left enforced.
    let (status, body) = send(
        &router,
        "PUT",
        &format!("/api/governance/policies/{retail_policy}/status"),
        &writer,
        Some(json!({ "status": "draft" })),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["error"], json!(POLICY_NOT_FOUND), "{body}");
    assert_eq!(
        status_of_policy(&pool, "retail mask").await,
        Some("ready".to_owned()),
        "a refused switch changes nothing"
    );

    // The holder's own policy switches as before.
    let (status, created) = send(
        &router,
        "POST",
        "/api/governance/policies",
        &writer,
        Some(own),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let own_id = created["id"].as_str().expect("the policy id").to_owned();
    let (status, body) = send(
        &router,
        "PUT",
        &format!("/api/governance/policies/{own_id}/status"),
        &writer,
        Some(json!({ "status": "ready" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], json!("ready"), "{body}");

    // An operator reaches every policy, so the refusal above is the reach's,
    // not the policy's.
    let operator = operator_cookie(&pool).await;
    let (status, body) = send(
        &router,
        "PUT",
        &format!("/api/governance/policies/{retail_policy}/status"),
        &operator,
        Some(json!({ "status": "draft" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

/// Another tenant's policy is unknown to the delete route: the same `404` an
/// unknown id gets, and the policy is still there. The holder's own policy
/// deletes as before, and an operator reaches every policy.
#[tokio::test]
async fn another_tenants_policy_is_unknown_to_the_delete_route_and_stays() {
    let ch = MockServer::start().await;
    mount_clickhouse(&ch).await;
    let TestApp { router, pool } = spin_up_for_the_policy_writers(&ch).await;
    claim_the_uploads(&pool).await;
    let retail_policy = seed_a_retail_policy(&pool, "retail mask", "draft").await;
    let writer = cookie_in_tenant(&pool, POLICY_WRITER, CATALOG_TENANT).await;

    // The other tenant's policy: unknown, and still there.
    let (status, body) = send(
        &router,
        "DELETE",
        &format!("/api/governance/policies/{retail_policy}"),
        &writer,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["error"], json!(POLICY_NOT_FOUND), "{body}");
    assert_eq!(
        policies_named(&pool, "retail mask").await,
        1,
        "a refused delete deletes nothing"
    );

    // The holder's own policy deletes as before.
    let (status, created) = send(
        &router,
        "POST",
        "/api/governance/policies",
        &writer,
        Some(policy_body(
            "writer on group",
            &format!("bronze.{GROUP_TABLE}"),
        )),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let own_id = created["id"].as_str().expect("the policy id").to_owned();
    let (status, body) = send(
        &router,
        "DELETE",
        &format!("/api/governance/policies/{own_id}"),
        &writer,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // An operator reaches every policy.
    let operator = operator_cookie(&pool).await;
    let (status, body) = send(
        &router,
        "DELETE",
        &format!("/api/governance/policies/{retail_policy}"),
        &operator,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(policies_named(&pool, "retail mask").await, 0);
}

// ── The list leaves out out-of-reach policies (`SEC-83` D4) ─────────────

/// The policy list holds only the policies of the tables the caller may
/// reach: the catalog's tenant reads its own policies and every policy that
/// binds no table, never the other tenant's; the other tenant reads its own
/// and the tableless one; an operator reads everything.
#[tokio::test]
async fn the_policy_list_leaves_out_the_policies_of_tables_the_caller_cannot_reach() {
    let ch = MockServer::start().await;
    mount_clickhouse(&ch).await;
    let TestApp { router, pool } = spin_up_for_the_policy_writers(&ch).await;
    claim_the_uploads(&pool).await;
    seed_a_retail_policy(&pool, "retail mask", "draft").await;
    let owner = cookie_in_tenant(&pool, POLICY_READER, CATALOG_TENANT).await;
    let other = cookie_in_tenant(&pool, POLICY_READER, OTHER_TENANT).await;

    // Two policies the catalog's tenant may author: one on its own upload,
    // one that binds no table.
    let writer = cookie_in_tenant(&pool, POLICY_WRITER, CATALOG_TENANT).await;
    for body in [
        policy_body("writer on group", &format!("bronze.{GROUP_TABLE}")),
        policy_body_without_conditions("writer without conditions"),
    ] {
        let (status, created) = send(
            &router,
            "POST",
            "/api/governance/policies",
            &writer,
            Some(body),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{created}");
    }

    let uri = "/api/governance/policies";

    // The catalog's tenant: its own policy and the tableless one, never the
    // other tenant's.
    let (status, body) = send(&router, "GET", uri, &owner, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let names = |body: &Value| -> Vec<String> {
        body.as_array()
            .expect("a list of policies")
            .iter()
            .map(|p| p["name"].as_str().expect("a policy name").to_owned())
            .collect()
    };
    let seen = names(&body);
    assert!(
        seen.contains(&"writer on group".to_owned())
            && seen.contains(&"writer without conditions".to_owned()),
        "the catalog's tenant reads its own policies: {body}"
    );
    assert!(
        !seen.contains(&"retail mask".to_owned()),
        "another tenant's policy is not on the list: {body}"
    );
    assert!(
        !body.to_string().contains(RETAIL_TABLE),
        "the answer carries no trace of the other tenant's table: {body}"
    );

    // The other tenant: its own policy and the tableless one, never the
    // catalog's tenant's.
    let (status, body) = send(&router, "GET", uri, &other, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let seen = names(&body);
    assert!(
        seen.contains(&"retail mask".to_owned()),
        "the other tenant reads its own policy: {body}"
    );
    assert!(
        seen.contains(&"writer without conditions".to_owned()),
        "a policy that binds no table is on every list: {body}"
    );
    assert!(
        !seen.contains(&"writer on group".to_owned()),
        "the catalog's tenant's policy is not on the other tenant's list: {body}"
    );

    // An operator reads everything: the two tenant-bound policies and the
    // tableless ones — the draft above, and the two seed rows
    // (`tenant_row_filter_default`, `agent_l3_write_gate`, which bind no
    // table and so are on every list).
    let operator = operator_cookie(&pool).await;
    let (status, body) = send(&router, "GET", uri, &operator, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let seen = names(&body);
    assert_eq!(seen.len(), 5, "{body}");
    assert!(
        seen.contains(&"retail mask".to_owned()) && seen.contains(&"writer on group".to_owned()),
        "the operator sees both tenants' policies: {body}"
    );
}

// ── The preview respects the reach (`SEC-83` D5) ────────────────────────

/// The preview of another tenant's table answers exactly what a table
/// `system.columns` has never heard of answers — `supported: false`, no
/// affected rows — and runs no count query at all. The preview of the
/// caller's own table measures as before.
#[tokio::test]
async fn the_preview_of_another_tenants_table_answers_as_a_missing_table_and_runs_no_count() {
    let ch = MockServer::start().await;
    mount_clickhouse(&ch).await;
    let TestApp { router, pool } = spin_up_for_the_policy_writers(&ch).await;
    claim_the_uploads(&pool).await;
    let writer = cookie_in_tenant(&pool, POLICY_WRITER, CATALOG_TENANT).await;
    let uri = "/api/governance/policies/preview";

    // Another tenant's table: the missing-table answer, and no measurement.
    let before = count_queries(&ch).await;
    let (status, body) = send(
        &router,
        "POST",
        uri,
        &writer,
        Some(json!({
            "table": format!("bronze.{RETAIL_TABLE}"),
            "mask": ["email"],
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["supported"], json!(false), "{body}");
    assert_eq!(body["affectedRows"], Value::Null, "{body}");
    assert_eq!(
        count_queries(&ch).await,
        before,
        "a refused preview runs no count query"
    );

    // The caller's own table: measured as before.
    let (status, body) = send(
        &router,
        "POST",
        uri,
        &writer,
        Some(json!({
            "table": format!("bronze.{GROUP_TABLE}"),
            "mask": ["email"],
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["supported"], json!(true), "{body}");
    assert_eq!(body["affectedRows"], json!(7), "{body}");
    assert_eq!(
        count_queries(&ch).await,
        before + 1,
        "an in-reach preview runs its one count query"
    );
}
