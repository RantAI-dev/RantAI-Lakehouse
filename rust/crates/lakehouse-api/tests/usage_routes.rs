//! `OPS-8` part B: the Usage routes over two seeded tenants.
//!
//! These tests drive the real router and a per-test database: runs are
//! inserted with the day a test wants (or made through `POST /api/query/run`
//! for the freshness test), and every answer is read back over HTTP. They hold
//! the tenant wall (a caller of one tenant sums none of the other's rows), the
//! permissions, the estimate, the CSV file, the bad input and the audit rows.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use crate::common;

use std::collections::HashMap;

use axum::body::{Body, to_bytes};
use axum::http::{HeaderMap, Request, StatusCode};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;
use wiremock::matchers::{body_string_contains, method};
use wiremock::{Mock, MockServer, ResponseTemplate};

use common::{
    TestApp, audit_rows, create_principal_with_permissions, session_cookie_for_seeded_user,
    session_cookie_for_user, spin_up, spin_up_with_env,
};

/// The tenant of the seeded `rina@meridian.example`, and another one.
const TENANT_A: &str = "11111111-1111-4111-8111-000000000001";
const TENANT_B: &str = "11111111-1111-4111-8111-000000000002";

const PERIOD: &str = "from=2026-03-01&to=2026-03-31";

/// The sentence that refuses a JSON number which is not a whole number: the
/// JSON parser holds such a number as a double, and a double cannot be read
/// back as the digits the caller wrote.
const AMOUNT_NOT_WHOLE: &str = "amount must be written as text, so its digits are kept. A number with a point or an exponent is refused, because its digits cannot be read back as they were written.";

struct Answer {
    status: StatusCode,
    headers: HeaderMap,
    bytes: Vec<u8>,
}

impl Answer {
    fn json(&self) -> Value {
        serde_json::from_slice(&self.bytes)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&self.bytes).into_owned()))
    }

    fn text(&self) -> String {
        String::from_utf8_lossy(&self.bytes).into_owned()
    }
}

async fn call(
    router: &axum::Router,
    cookie: &str,
    method: &str,
    uri: &str,
    extra_header: Option<(&str, &str)>,
    body: Option<Value>,
) -> Answer {
    let text = body.map(|body| body.to_string());
    call_text(router, cookie, method, uri, extra_header, text.as_deref()).await
}

/// The same request, with the body as raw text: a body `serde_json`'s `json!`
/// cannot write, such as a number written with an exponent.
async fn call_text(
    router: &axum::Router,
    cookie: &str,
    method: &str,
    uri: &str,
    extra_header: Option<(&str, &str)>,
    body: Option<&str>,
) -> Answer {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("cookie", cookie);
    if let Some((name, value)) = extra_header {
        builder = builder.header(name, value);
    }
    let request = match body {
        // The central audit writer reads the field names of a body only when
        // the request declares its length, the way a browser's does.
        Some(text) => builder
            .header("content-type", "application/json")
            .header("content-length", text.len())
            .body(Body::from(text.to_owned())),
        None => builder.body(Body::empty()),
    }
    .expect("build request");
    let response = router
        .clone()
        .oneshot(request)
        .await
        .expect("router never fails a request outright");
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body")
        .to_vec();
    Answer {
        status,
        headers,
        bytes,
    }
}

async fn get(router: &axum::Router, cookie: &str, uri: &str) -> Answer {
    call(router, cookie, "GET", uri, None, None).await
}

async fn post_rate(router: &axum::Router, cookie: &str, body: Value) -> Answer {
    call(router, cookie, "POST", "/api/usage/rates", None, Some(body)).await
}

/// A rate posted as the raw text of a body, for a body `serde_json`'s `json!`
/// cannot write, such as a number written with an exponent.
async fn post_rate_text(router: &axum::Router, cookie: &str, body: &str) -> Answer {
    call_text(router, cookie, "POST", "/api/usage/rates", None, Some(body)).await
}

/// A person holding exactly `permissions`, who belongs to `tenants`.
async fn person(pool: &PgPool, permissions: &str, tenants: &[&str]) -> String {
    let id = create_principal_with_permissions(pool, permissions).await;
    for tenant in tenants {
        sqlx::query("INSERT INTO app_user_tenant (user_id, tenant_id) VALUES ($1, $2)")
            .bind(id)
            .bind(tenant.parse::<Uuid>().unwrap())
            .execute(pool)
            .await
            .expect("add the membership");
    }
    session_cookie_for_user(pool, id).await
}

/// One run with the columns a sum reads.
struct Run<'a> {
    at: &'a str,
    tenant: Option<&'a str>,
    owner: Option<Uuid>,
    user: &'a str,
    status: &'a str,
    scanned_bytes: i64,
}

impl<'a> Run<'a> {
    fn new(at: &'a str, tenant: Option<&'a str>, user: &'a str) -> Self {
        Self {
            at,
            tenant,
            owner: None,
            user,
            status: "completed",
            scanned_bytes: 0,
        }
    }
}

async fn insert(pool: &PgPool, run: &Run<'_>) {
    sqlx::query(
        "INSERT INTO query_history \
         (id, sql, user_name, at, status, duration_ms, scanned_bytes, cost_units, \
          workload_class, engine, owner_id, tenant_id) \
         VALUES ($1, 'SELECT 1', $2, $3::timestamptz, $4, 10, $5, 0, 'hot-analytics', \
                 'clickhouse', $6, $7)",
    )
    .bind(format!("q-{}", Uuid::new_v4()))
    .bind(run.user)
    .bind(run.at)
    .bind(run.status)
    .bind(run.scanned_bytes)
    .bind(run.owner)
    .bind(run.tenant.map(|t| t.parse::<Uuid>().unwrap()))
    .execute(pool)
    .await
    .expect("insert a query_history row");
}

/// Three tenants' worth of runs: A's Alice, B's Bob and a run with no tenant.
async fn seed_two_tenants(pool: &PgPool) {
    let mut alice = Run::new("2026-03-02T10:00:00Z", Some(TENANT_A), "Alice of A");
    alice.scanned_bytes = 100;
    insert(pool, &alice).await;
    let mut bob = Run::new("2026-03-02T11:00:00Z", Some(TENANT_B), "Bob of B");
    bob.scanned_bytes = 7;
    insert(pool, &bob).await;
    let mut nobody = Run::new("2026-03-02T12:00:00Z", None, "Nobody");
    nobody.scanned_bytes = 1000;
    insert(pool, &nobody).await;
}

fn rows(answer: &Answer) -> Vec<Value> {
    answer.json()["rows"].as_array().expect("rows").clone()
}

/// The row of the answer with that key.
fn person_row(body: &Value, key: &str) -> Value {
    body["rows"]
        .as_array()
        .expect("rows")
        .iter()
        .find(|row| row["key"] == json!(key))
        .cloned()
        .unwrap_or_else(|| panic!("no row for {key}: {body}"))
}

// ── (a) freshness ────────────────────────────────────────────────────────

fn clickhouse_with_one_row() -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({
        "meta": [{"name": "n", "type": "UInt8"}],
        "data": [{"n": 1}],
        "rows": 1,
    }))
}

#[tokio::test]
async fn a_run_shows_in_the_usage_of_its_tenant_at_once() {
    let ch = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_string_contains("system.columns"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "meta": [
                {"name": "name", "type": "String"},
                {"name": "type", "type": "String"},
                {"name": "default_kind", "type": "String"},
                {"name": "default_expression", "type": "String"},
            ],
            "data": [],
            "rows": 0,
        })))
        .mount(&ch)
        .await;
    Mock::given(method("POST"))
        .respond_with(clickhouse_with_one_row())
        .mount(&ch)
        .await;
    let mut overrides = HashMap::new();
    overrides.insert("CH_URL".to_owned(), ch.uri());
    overrides.insert("CATALOG_TENANT_ID".to_owned(), TENANT_A.to_owned());
    let TestApp { router, pool } = spin_up_with_env(&overrides).await;

    let rina: Uuid = sqlx::query_scalar("SELECT id FROM app_user WHERE email = $1")
        .bind("rina@meridian.example")
        .fetch_one(&pool)
        .await
        .unwrap();
    // Rina may read the usage of her tenant: a role that grants only that.
    let role = Uuid::new_v4();
    sqlx::query("INSERT INTO role (id, name, permissions, description) VALUES ($1, 'Usage Reader', 'usage:read', 'test')")
        .bind(role)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO app_user_role (user_id, role_id) VALUES ($1, $2)")
        .bind(rina)
        .bind(role)
        .execute(&pool)
        .await
        .unwrap();
    let cookie = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;

    let before = get(&router, &cookie, "/api/usage/queries?group=person").await;
    assert_eq!(before.status, StatusCode::OK, "{}", before.text());
    assert_eq!(before.json()["totals"]["queries"], json!(0));

    let ran = call(
        &router,
        &cookie,
        "POST",
        "/api/query/run",
        None,
        Some(json!({ "sql": "SELECT 1" })),
    )
    .await;
    assert_eq!(ran.status, StatusCode::OK, "{}", ran.text());

    // No wait, no job: the run is in the answer of the very next request.
    let after = get(&router, &cookie, "/api/usage/queries?group=person").await;
    assert_eq!(after.status, StatusCode::OK, "{}", after.text());
    let body = after.json();
    assert_eq!(body["totals"]["queries"], json!(1), "{body}");
    assert_eq!(body["totals"]["completed"], json!(1), "{body}");
    assert_eq!(body["rows"][0]["key"], json!(rina.to_string()), "{body}");
    // Rina holds no `identity:read`: the row shows the id, not the name.
    assert_eq!(body["rows"][0]["label"], json!(rina.to_string()), "{body}");
}

// ── (b) the tenant wall ──────────────────────────────────────────────────

#[tokio::test]
async fn a_caller_of_tenant_b_sums_none_of_tenant_as_rows() {
    let TestApp { router, pool } = spin_up().await;
    seed_two_tenants(&pool).await;
    let bob = person(&pool, "usage:read", &[TENANT_B]).await;

    for group in ["day", "person", "engine", "tenant"] {
        let answer = get(
            &router,
            &bob,
            &format!("/api/usage/queries?{PERIOD}&group={group}"),
        )
        .await;
        assert_eq!(answer.status, StatusCode::OK, "{group}: {}", answer.text());
        let body = answer.json();
        assert_eq!(body["totals"]["queries"], json!(1), "{group}: {body}");
        assert_eq!(body["totals"]["scannedBytes"], json!(7), "{group}: {body}");
        let text = answer.text();
        assert!(!text.contains("Alice of A"), "{group}: {text}");
        assert!(!text.contains("Nobody"), "{group}: {text}");
    }

    // Naming A in `X-Tenant` does not widen anything: B's caller is not in A.
    let forged = call(
        &router,
        &bob,
        "GET",
        &format!("/api/usage/queries?{PERIOD}"),
        Some(("x-tenant", TENANT_A)),
        None,
    )
    .await;
    assert_eq!(forged.status, StatusCode::NOT_FOUND, "{}", forged.text());
    assert!(!forged.text().contains("Alice"), "{}", forged.text());

    // The same wall holds for the file.
    let file = get(
        &router,
        &bob,
        &format!("/api/usage/queries/export?{PERIOD}&group=person"),
    )
    .await;
    assert_eq!(file.status, StatusCode::OK);
    assert!(!file.text().contains("Alice of A"), "{}", file.text());
}

/// The tenant wall comes first: a forged `X-Tenant` is the same 404 whatever
/// else the request says, so a bad date cannot turn it into a 400.
#[tokio::test]
async fn a_forged_tenant_is_the_same_404_even_when_the_period_is_bad() {
    let TestApp { router, pool } = spin_up().await;
    let bob = person(&pool, "usage:read", &[TENANT_B]).await;

    for uri in ["/api/usage/queries", "/api/usage/queries/export"] {
        let forged = call(
            &router,
            &bob,
            "GET",
            &format!("{uri}?{PERIOD}"),
            Some(("x-tenant", TENANT_A)),
            None,
        )
        .await;
        assert_eq!(
            forged.status,
            StatusCode::NOT_FOUND,
            "{uri}: {}",
            forged.text()
        );

        // The bad date does not turn the refusal into a 400: the tenant is
        // resolved before the period is read.
        let forged_and_bad = call(
            &router,
            &bob,
            "GET",
            &format!("{uri}?from=yesterday"),
            Some(("x-tenant", TENANT_A)),
            None,
        )
        .await;
        assert_eq!(
            forged_and_bad.status,
            StatusCode::NOT_FOUND,
            "{uri}: {}",
            forged_and_bad.text()
        );
        assert_eq!(
            forged_and_bad.json()["error"],
            forged.json()["error"],
            "{uri}: the same refusal as the forged tenant alone"
        );
    }
}

#[tokio::test]
async fn a_caller_that_belongs_to_no_tenant_gets_an_empty_answer_not_every_tenant() {
    let TestApp { router, pool } = spin_up().await;
    seed_two_tenants(&pool).await;
    let lonely = person(&pool, "usage:read", &[]).await;

    let answer = get(
        &router,
        &lonely,
        &format!("/api/usage/queries?{PERIOD}&group=person"),
    )
    .await;
    assert_eq!(answer.status, StatusCode::OK, "{}", answer.text());
    let body = answer.json();
    assert_eq!(body["rows"], json!([]), "{body}");
    assert_eq!(body["totals"]["queries"], json!(0), "{body}");
    assert_eq!(body["scope"], json!("tenant"), "{body}");
}

// ── (c) everybody, and the rows with no tenant ───────────────────────────

#[tokio::test]
async fn the_unrestricted_caller_sees_every_tenant_and_no_tenant_only_when_it_asks_for_all() {
    let TestApp { router, pool } = spin_up().await;
    seed_two_tenants(&pool).await;
    let admin = person(&pool, "*:*", &[TENANT_A]).await;

    let all = get(
        &router,
        &admin,
        &format!("/api/usage/queries?{PERIOD}&group=tenant&tenant=all"),
    )
    .await;
    assert_eq!(all.status, StatusCode::OK, "{}", all.text());
    let body = all.json();
    assert_eq!(body["scope"], json!("all"), "{body}");
    assert_eq!(body["totals"]["queries"], json!(3), "{body}");
    let none = rows(&all)
        .into_iter()
        .find(|r| r["key"] == json!("none"))
        .expect("the run with no tenant has a row");
    assert_eq!(none["label"], json!("no tenant"), "{body}");
    assert_eq!(none["scannedBytes"], json!(1000), "{body}");

    // Without `tenant=all` it sees the tenant it acts as, like anyone.
    let own = get(
        &router,
        &admin,
        &format!("/api/usage/queries?{PERIOD}&group=tenant"),
    )
    .await;
    let body = own.json();
    assert_eq!(body["scope"], json!("tenant"), "{body}");
    assert_eq!(body["totals"]["queries"], json!(1), "{body}");
    assert_eq!(body["totals"]["scannedBytes"], json!(100), "{body}");
}

#[tokio::test]
async fn a_caller_without_the_unrestricted_grant_may_not_ask_for_all_tenants() {
    let TestApp { router, pool } = spin_up().await;
    seed_two_tenants(&pool).await;
    let reader = person(&pool, "usage:read", &[TENANT_A]).await;

    let answer = get(
        &router,
        &reader,
        &format!("/api/usage/queries?{PERIOD}&tenant=all"),
    )
    .await;
    assert_eq!(answer.status, StatusCode::FORBIDDEN, "{}", answer.text());
    let answer = get(
        &router,
        &reader,
        &format!("/api/usage/queries/export?{PERIOD}&tenant=all"),
    )
    .await;
    assert_eq!(answer.status, StatusCode::FORBIDDEN, "{}", answer.text());
    assert!(
        audit_rows(&pool, "usage.export").await.is_empty(),
        "a refused export records nothing"
    );
}

// ── (d) permissions ──────────────────────────────────────────────────────

#[tokio::test]
async fn without_usage_read_every_usage_route_is_refused_and_a_rate_needs_settings_write() {
    let TestApp { router, pool } = spin_up().await;
    let nobody = person(&pool, "dashboard:read", &[TENANT_A]).await;
    for uri in [
        "/api/usage/queries",
        "/api/usage/queries/export",
        "/api/usage/rates",
    ] {
        let answer = get(&router, &nobody, uri).await;
        assert_eq!(answer.status, StatusCode::FORBIDDEN, "{uri}");
    }
    let body =
        json!({"kind": "scan_tb", "currency": "USD", "amount": "5", "effectiveFrom": "2026-03-01"});
    let answer = post_rate(&router, &nobody, body.clone()).await;
    assert_eq!(answer.status, StatusCode::FORBIDDEN);

    // `usage:read` reads the rates and may not write them.
    let reader = person(&pool, "usage:read", &[TENANT_A]).await;
    assert_eq!(
        get(&router, &reader, "/api/usage/rates").await.status,
        StatusCode::OK
    );
    assert_eq!(
        post_rate(&router, &reader, body.clone()).await.status,
        StatusCode::FORBIDDEN
    );

    // `settings:write` writes them and does not read the usage.
    let writer = person(&pool, "settings:write", &[TENANT_A]).await;
    assert_eq!(
        get(&router, &writer, "/api/usage/queries").await.status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        post_rate(&router, &writer, body).await.status,
        StatusCode::CREATED
    );
}

// ── (e) rates and the estimate ───────────────────────────────────────────

#[tokio::test]
async fn the_estimate_is_null_without_a_rate_and_priced_by_the_rate_in_force() {
    let TestApp { router, pool } = spin_up().await;
    for at in ["2026-03-02T10:00:00Z", "2026-03-04T10:00:00Z"] {
        let mut run = Run::new(at, Some(TENANT_A), "Alice");
        run.scanned_bytes = 1_000_000_000_000;
        insert(&pool, &run).await;
    }
    let reader = person(&pool, "usage:read,settings:write", &[TENANT_A]).await;
    let uri = format!("/api/usage/queries?{PERIOD}&group=day");

    let none = get(&router, &reader, &uri).await.json();
    assert_eq!(none["totals"]["estimate"], Value::Null, "{none}");
    assert_eq!(none["currency"], Value::Null, "{none}");

    let first = post_rate(&router, &reader, json!({"kind": "scan_tb", "currency": "USD", "amount": "5.00", "effectiveFrom": "2026-03-01"})).await;
    assert_eq!(first.status, StatusCode::CREATED, "{}", first.text());
    assert_eq!(first.json()["amount"], json!("5.000000"));
    let second = post_rate(
        &router,
        &reader,
        json!({"kind": "scan_tb", "currency": "USD", "amount": 10, "effectiveFrom": "2026-03-03"}),
    )
    .await;
    assert_eq!(second.status, StatusCode::CREATED, "{}", second.text());

    let priced = get(&router, &reader, &uri).await.json();
    let by_day: HashMap<String, Value> = priced["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| (r["key"].as_str().unwrap().to_owned(), r["estimate"].clone()))
        .collect();
    assert_eq!(by_day["2026-03-02"], json!("5.000000"), "{priced}");
    assert_eq!(by_day["2026-03-04"], json!("10.000000"), "{priced}");
    assert_eq!(priced["totals"]["estimate"], json!("15.000000"), "{priced}");
    assert_eq!(priced["currency"], json!("USD"), "{priced}");

    let listed = get(&router, &reader, "/api/usage/rates").await.json();
    assert_eq!(listed["currency"], json!("USD"));
    assert_eq!(listed["rates"].as_array().unwrap().len(), 2);
    assert_eq!(
        listed["rates"][0]["effectiveFrom"],
        json!("2026-03-03"),
        "newest first"
    );
    // PR review fix (SHOULD-FIX): who entered a rate is a principal id of the
    // deployment's administrator; a tenant administrator reading the rates does
    // not get it, in the list or in the answer to the write.
    for rate in listed["rates"].as_array().unwrap() {
        assert!(rate.get("createdBy").is_none(), "{rate}");
    }
    assert!(first.json().get("createdBy").is_none(), "{}", first.text());
    let stored_by: Vec<String> = sqlx::query_scalar("SELECT created_by FROM usage_rate")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert!(
        stored_by.iter().all(|who| !who.is_empty()),
        "the table still records who entered a rate"
    );
}

#[tokio::test]
async fn a_bad_rate_is_a_400_with_a_fixed_sentence_and_adds_nothing() {
    let TestApp { router, pool } = spin_up().await;
    let admin = person(&pool, "usage:read,settings:write", &[TENANT_A]).await;
    let good =
        json!({"kind": "scan_tb", "currency": "USD", "amount": "5", "effectiveFrom": "2026-03-01"});
    assert_eq!(
        post_rate(&router, &admin, good.clone()).await.status,
        StatusCode::CREATED
    );

    let cases = [
        (
            "a negative amount",
            json!({"kind": "scan_tb", "currency": "USD", "amount": "-1", "effectiveFrom": "2026-03-01"}),
        ),
        (
            "an exponent",
            json!({"kind": "scan_tb", "currency": "USD", "amount": "1e3", "effectiveFrom": "2026-03-01"}),
        ),
        (
            "seven decimals",
            json!({"kind": "scan_tb", "currency": "USD", "amount": "0.1234567", "effectiveFrom": "2026-03-01"}),
        ),
        (
            "no amount",
            json!({"kind": "scan_tb", "currency": "USD", "effectiveFrom": "2026-03-01"}),
        ),
        (
            "a four-letter currency",
            json!({"kind": "scan_tb", "currency": "USDX", "amount": "1", "effectiveFrom": "2026-03-01"}),
        ),
        (
            "a lower-case currency",
            json!({"kind": "scan_tb", "currency": "usd", "amount": "1", "effectiveFrom": "2026-03-01"}),
        ),
        (
            "an unknown kind",
            json!({"kind": "per_query", "currency": "USD", "amount": "1", "effectiveFrom": "2026-03-01"}),
        ),
        (
            "a bad date",
            json!({"kind": "scan_tb", "currency": "USD", "amount": "1", "effectiveFrom": "2026-02-30"}),
        ),
        (
            "an unknown field",
            json!({"kind": "scan_tb", "currency": "USD", "amount": "1", "effectiveFrom": "2026-03-01", "extra": 1}),
        ),
        (
            "another currency than the rates entered",
            json!({"kind": "engine_hour", "currency": "IDR", "amount": "1", "effectiveFrom": "2026-03-01"}),
        ),
    ];
    for (name, body) in cases {
        let answer = post_rate(&router, &admin, body).await;
        assert_eq!(
            answer.status,
            StatusCode::BAD_REQUEST,
            "{name}: {}",
            answer.text()
        );
        assert!(answer.json()["error"].is_string(), "{name}");
    }
    let listed = get(&router, &admin, "/api/usage/rates").await.json();
    assert_eq!(listed["rates"].as_array().unwrap().len(), 1, "{listed}");
}

#[tokio::test]
async fn an_amount_is_read_as_the_digits_the_caller_wrote() {
    let TestApp { router, pool } = spin_up().await;
    let admin = person(&pool, "usage:read,settings:write", &[TENANT_A]).await;

    // A JSON number with digits after the point is held as a double, and a
    // double cannot be read back as the digits the caller wrote.
    let number = post_rate(
        &router,
        &admin,
        json!({"kind": "scan_tb", "currency": "USD", "amount": 5.5, "effectiveFrom": "2026-03-01"}),
    )
    .await;
    assert_eq!(number.status, StatusCode::BAD_REQUEST, "{}", number.text());
    assert_eq!(number.json()["error"], json!(AMOUNT_NOT_WHOLE));

    // The same for a number with more digits than a double holds, and for one
    // written with an exponent: both go out as text, because `json!` cannot
    // write them.
    for (name, body) in [
        (
            "digits a double loses",
            r#"{"kind":"scan_tb","currency":"USD","amount":999999999999.999999,"effectiveFrom":"2026-03-01"}"#,
        ),
        (
            "an exponent",
            r#"{"kind":"scan_tb","currency":"USD","amount":1e3,"effectiveFrom":"2026-03-01"}"#,
        ),
    ] {
        let answer = post_rate_text(&router, &admin, body).await;
        assert_eq!(
            answer.status,
            StatusCode::BAD_REQUEST,
            "{name}: {}",
            answer.text()
        );
        assert_eq!(answer.json()["error"], json!(AMOUNT_NOT_WHOLE), "{name}");
    }
    // Nothing was added by a refused amount.
    let listed = get(&router, &admin, "/api/usage/rates").await.json();
    assert_eq!(listed["rates"].as_array().unwrap().len(), 0, "{listed}");

    // A whole number is exact: the JSON parser holds an integer literal as an
    // integer, not as a double.
    let whole = post_rate(
        &router,
        &admin,
        json!({"kind": "scan_tb", "currency": "USD", "amount": 999_999_999_999_i64, "effectiveFrom": "2026-03-01"}),
    )
    .await;
    assert_eq!(whole.status, StatusCode::CREATED, "{}", whole.text());
    assert_eq!(whole.json()["amount"], json!("999999999999.000000"));

    // Text keeps every digit, more than a double holds.
    let text = post_rate(
        &router,
        &admin,
        json!({"kind": "scan_tb", "currency": "USD", "amount": "999999999999.999999", "effectiveFrom": "2026-03-02"}),
    )
    .await;
    assert_eq!(text.status, StatusCode::CREATED, "{}", text.text());
    assert_eq!(text.json()["amount"], json!("999999999999.999999"));

    // The stored rows hold the same digits.
    let stored: Vec<String> =
        sqlx::query_scalar("SELECT amount::text FROM usage_rate ORDER BY effective_from")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(
        stored,
        vec![
            "999999999999.000000".to_owned(),
            "999999999999.999999".to_owned()
        ],
    );
}

// ── (f) the file ─────────────────────────────────────────────────────────

/// The cells of every line of a CSV file, the byte-order mark removed.
fn csv_cells(file: &str) -> Vec<Vec<String>> {
    file.trim_start_matches('\u{feff}')
        .split("\r\n")
        .filter(|line| !line.is_empty())
        .map(|line| line.split(',').map(str::to_owned).collect())
        .collect()
}

#[tokio::test]
async fn the_file_holds_the_rows_and_totals_of_the_page_a_formula_neutralised_and_is_audited() {
    let TestApp { router, pool } = spin_up().await;
    let owner = Uuid::new_v4();
    let mut a = Run::new("2026-03-02T10:00:00Z", Some(TENANT_A), "Rina");
    a.owner = Some(owner);
    a.scanned_bytes = 40;
    insert(&pool, &a).await;
    let mut b = Run::new("2026-03-03T10:00:00Z", Some(TENANT_A), "=HYPERLINK(1)");
    b.status = "failed";
    b.scanned_bytes = 2;
    insert(&pool, &b).await;
    // The names are the user list's: this caller may read it.
    let admin = person(&pool, "usage:read,identity:read", &[TENANT_A]).await;
    // PR review fix (SHOULD-FIX): a period with a rate, so the file's estimate
    // and currency columns are compared with the page's, not both empty.
    sqlx::query(
        "INSERT INTO usage_rate (kind, currency, amount, effective_from, created_by) \
         VALUES ('scan_tb', 'USD', 5000000000, DATE '2026-03-01', 'test')",
    )
    .execute(&pool)
    .await
    .unwrap();
    let uri = format!("{PERIOD}&group=person");

    let page = get(&router, &admin, &format!("/api/usage/queries?{uri}"))
        .await
        .json();
    let file = get(&router, &admin, &format!("/api/usage/queries/export?{uri}")).await;
    assert_eq!(file.status, StatusCode::OK);
    assert_eq!(file.headers["content-type"], "text/csv; charset=utf-8");
    assert_eq!(
        file.headers["content-disposition"],
        "attachment; filename=\"usage-2026-03-01-2026-03-31.csv\""
    );
    assert_eq!(file.headers["cache-control"], "no-store");

    let cells = csv_cells(&file.text());
    assert_eq!(
        cells[0],
        [
            "person",
            "id",
            "queries",
            "completed",
            "failed",
            "blocked",
            "cancelled",
            "scanned_bytes",
            "engine_ms",
            "estimate",
            "currency"
        ]
    );
    // One line per row of the page, then the totals line.
    let page_rows = page["rows"].as_array().unwrap();
    assert_eq!(cells.len(), page_rows.len() + 2, "{cells:?}");
    for (line, row) in cells[1..=page_rows.len()].iter().zip(page_rows) {
        assert_eq!(
            line[1].trim_start_matches('\''),
            row["key"].as_str().unwrap()
        );
        assert_eq!(line[2], row["queries"].to_string());
        assert_eq!(line[4], row["failed"].to_string());
        assert_eq!(line[7], row["scannedBytes"].to_string());
        assert_eq!(line[9], row["estimate"].as_str().unwrap(), "{line:?}");
        assert_eq!(line[10], "USD");
    }
    let totals = cells.last().unwrap();
    assert_eq!(totals[0], "Total");
    assert_eq!(totals[2], page["totals"]["queries"].to_string());
    assert_eq!(totals[7], page["totals"]["scannedBytes"].to_string());
    // 42 bytes x 5e9 per 10^12 bytes: the page's own sum, not a recomputation.
    assert_eq!(page["totals"]["estimate"], json!("0.210000"), "{page}");
    assert_eq!(totals[9], "0.210000");
    assert_eq!(totals[10], "USD");

    // A name that starts with `=` is text in the file, not a formula.
    let name_cells: Vec<&str> = cells[1..=page_rows.len()]
        .iter()
        .map(|l| l[0].as_str())
        .collect();
    assert!(name_cells.contains(&"'=HYPERLINK(1)"), "{name_cells:?}");
    assert!(
        !name_cells.iter().any(|c| c.starts_with('=')),
        "{name_cells:?}"
    );

    let audit = audit_rows(&pool, "usage.export").await;
    assert_eq!(audit.len(), 1, "one export, one row: {audit:?}");
    assert_eq!(audit[0]["outcome"], json!("executed"));
    assert_eq!(audit[0]["resource_kind"], json!("usage"));
    assert_eq!(audit[0]["args"]["from"], json!("2026-03-01"));
    assert_eq!(audit[0]["args"]["to"], json!("2026-03-31"));
    assert_eq!(audit[0]["args"]["group"], json!("person"));
    // PR review fix (SHOULD-FIX): the row used to say `"tenant"` for every
    // caller; it names the tenant whose runs were taken out.
    assert_eq!(audit[0]["args"]["tenant"], json!(TENANT_A));
}

#[tokio::test]
async fn an_export_of_all_tenants_records_all_and_one_of_no_tenant_records_none() {
    let TestApp { router, pool } = spin_up().await;
    seed_two_tenants(&pool).await;
    let root = person(&pool, "*:*", &[TENANT_A]).await;
    let lonely = person(&pool, "usage:read", &[]).await;

    let all = get(
        &router,
        &root,
        &format!("/api/usage/queries/export?{PERIOD}&tenant=all"),
    )
    .await;
    assert_eq!(all.status, StatusCode::OK, "{}", all.text());
    let own = get(
        &router,
        &root,
        &format!("/api/usage/queries/export?{PERIOD}"),
    )
    .await;
    assert_eq!(own.status, StatusCode::OK, "{}", own.text());
    let none = get(
        &router,
        &lonely,
        &format!("/api/usage/queries/export?{PERIOD}"),
    )
    .await;
    assert_eq!(none.status, StatusCode::OK, "{}", none.text());

    let audit = audit_rows(&pool, "usage.export").await;
    let tenants: Vec<&Value> = audit.iter().map(|row| &row["args"]["tenant"]).collect();
    assert_eq!(
        tenants,
        [&json!("all"), &json!(TENANT_A), &json!("none")],
        "{audit:?}"
    );
}

#[tokio::test]
async fn a_caller_without_identity_read_sees_a_persons_id_in_the_page_and_the_file() {
    let TestApp { router, pool } = spin_up().await;
    let owner = Uuid::new_v4();
    let mut run = Run::new("2026-03-02T10:00:00Z", Some(TENANT_A), "Rina Wijaya");
    run.owner = Some(owner);
    insert(&pool, &run).await;
    let with_names = person(&pool, "usage:read,identity:read", &[TENANT_A]).await;
    let without = person(&pool, "usage:read", &[TENANT_A]).await;
    let uri = format!("{PERIOD}&group=person");

    let named = get(&router, &with_names, &format!("/api/usage/queries?{uri}")).await;
    assert_eq!(named.json()["rows"][0]["label"], json!("Rina Wijaya"));
    let plain = get(&router, &without, &format!("/api/usage/queries?{uri}")).await;
    assert_eq!(plain.json()["rows"][0]["label"], json!(owner.to_string()));
    let file = get(
        &router,
        &without,
        &format!("/api/usage/queries/export?{uri}"),
    )
    .await;
    assert!(!file.text().contains("Rina Wijaya"), "{}", file.text());
}

/// The Person row is labelled with the name the user list holds for the
/// owner. Since the query-ownership fix the run records the principal's uuid
/// string, so the name the run carries is not a name at all.
///
/// A run whose owner is not in the user list any more (the user was deleted)
/// and a run with no owner keep the name their run recorded: the first is
/// that name, the second is its key.
#[tokio::test]
async fn a_person_row_is_labelled_with_the_name_the_user_list_holds() {
    let TestApp { router, pool } = spin_up().await;
    let owner = Uuid::new_v4();
    let username = format!("rina-{}", owner.simple());
    let email = format!("{}@test.invalid", owner.simple());
    sqlx::query("INSERT INTO app_user (id, name, username, email) VALUES ($1, $2, $3, $4)")
        .bind(owner)
        .bind("Rina Wijaya")
        .bind(username)
        .bind(email)
        .execute(&pool)
        .await
        .unwrap();
    let recorded = owner.to_string();
    let mut run = Run::new("2026-03-02T10:00:00Z", Some(TENANT_A), &recorded);
    run.owner = Some(owner);
    insert(&pool, &run).await;

    // An owner who is not in the user list any more: the user was deleted.
    let deleted = Uuid::new_v4();
    let mut gone = Run::new("2026-03-02T11:00:00Z", Some(TENANT_A), "Ghost User");
    gone.owner = Some(deleted);
    insert(&pool, &gone).await;

    // A run with no owner at all: its recorded name is its key.
    let mut nobody = Run::new("2026-03-02T12:00:00Z", Some(TENANT_A), "Legacy User");
    nobody.owner = None;
    insert(&pool, &nobody).await;

    let reader = person(&pool, "usage:read,identity:read", &[TENANT_A]).await;
    let answer = get(
        &router,
        &reader,
        &format!("/api/usage/queries?{PERIOD}&group=person"),
    )
    .await;
    assert_eq!(answer.status, StatusCode::OK, "{}", answer.text());
    let body = answer.json();
    let listed = person_row(&body, &owner.to_string());
    assert_eq!(listed["label"], json!("Rina Wijaya"), "{body}");
    assert_eq!(listed["queries"], json!(1), "{body}");
    let gone_row = person_row(&body, &deleted.to_string());
    assert_eq!(gone_row["label"], json!("Ghost User"), "{body}");
    let legacy = person_row(&body, "Legacy User");
    assert_eq!(legacy["label"], json!("Legacy User"), "{body}");
}

// ── (g) bad input ────────────────────────────────────────────────────────

#[tokio::test]
async fn a_bad_period_group_or_tenant_is_a_400_with_a_fixed_sentence() {
    let TestApp { router, pool } = spin_up().await;
    let reader = person(&pool, "usage:read", &[TENANT_A]).await;
    for uri in ["/api/usage/queries", "/api/usage/queries/export"] {
        let cases = [
            (
                "367 days",
                "from=2025-01-01&to=2026-01-02",
                "A period is at most 366 days.",
            ),
            (
                "from after to",
                "from=2026-03-02&to=2026-03-01",
                "from must not be after to.",
            ),
            (
                "a bad date",
                "from=yesterday",
                "from and to must be days written YYYY-MM-DD.",
            ),
            (
                "a day that does not exist",
                "to=2026-02-30",
                "from and to must be days written YYYY-MM-DD.",
            ),
            (
                "a bad group",
                "group=planet",
                "group must be day, person, engine or tenant.",
            ),
            (
                "a bad tenant",
                "tenant=everyone",
                "tenant must be all, or left out.",
            ),
            (
                "a key sent twice",
                "group=day&group=person",
                "The query string could not be read.",
            ),
        ];
        for (name, query, sentence) in cases {
            let answer = get(&router, &reader, &format!("{uri}?{query}")).await;
            assert_eq!(
                answer.status,
                StatusCode::BAD_REQUEST,
                "{uri} {name}: {}",
                answer.text()
            );
            assert_eq!(answer.json()["error"], json!(sentence), "{uri} {name}");
        }
    }
    // 366 days is the longest that is accepted.
    let longest = get(
        &router,
        &reader,
        "/api/usage/queries?from=2025-01-01&to=2026-01-01",
    )
    .await;
    assert_eq!(longest.status, StatusCode::OK, "{}", longest.text());
    assert!(
        audit_rows(&pool, "usage.export").await.is_empty(),
        "a refused export records nothing"
    );
}

#[tokio::test]
async fn with_no_period_the_last_thirty_days_are_summed() {
    let TestApp { router, pool } = spin_up().await;
    let reader = person(&pool, "usage:read", &[TENANT_A]).await;
    let body = get(&router, &reader, "/api/usage/queries").await.json();
    let from = body["from"].as_str().unwrap();
    let to = body["to"].as_str().unwrap();
    assert_eq!(body["group"], json!("day"));
    // PR review fix (SHOULD-FIX): `from < to` held for a period of two days as
    // well. Thirty UTC days, today included, are 29 days between the dates.
    assert_eq!(
        day_of(to) - day_of(from),
        time::Duration::days(29),
        "{from} {to}"
    );
    assert_eq!(
        to,
        time::OffsetDateTime::now_utc().date().to_string(),
        "the period ends today in UTC"
    );
}

/// `YYYY-MM-DD` as a date.
fn day_of(text: &str) -> time::Date {
    let number = |range: std::ops::Range<usize>| text[range].parse::<u8>().unwrap();
    time::Date::from_calendar_date(
        text[0..4].parse().unwrap(),
        time::Month::try_from(number(5..7)).unwrap(),
        number(8..10),
    )
    .unwrap()
}

// ── (h) audit ────────────────────────────────────────────────────────────

#[tokio::test]
async fn adding_a_rate_writes_one_audit_row_and_a_refused_one_writes_a_refused_row() {
    let TestApp { router, pool } = spin_up().await;
    let admin = person(&pool, "settings:write", &[TENANT_A]).await;
    let reader = person(&pool, "usage:read", &[TENANT_A]).await;
    let body =
        json!({"kind": "scan_tb", "currency": "USD", "amount": "5", "effectiveFrom": "2026-03-01"});

    assert_eq!(
        post_rate(&router, &admin, body.clone()).await.status,
        StatusCode::CREATED
    );
    let rows = audit_rows(&pool, "usage.rate_create").await;
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0]["outcome"], json!("executed"));
    assert_eq!(rows[0]["resource_kind"], json!("settings"));
    // The central writer records the names of the body's fields, never a value.
    assert_eq!(
        rows[0]["args"],
        json!({"fields": ["amount", "currency", "effectiveFrom", "kind"]})
    );

    assert_eq!(
        post_rate(&router, &reader, body).await.status,
        StatusCode::FORBIDDEN
    );
    let rows = audit_rows(&pool, "usage.rate_create").await;
    assert_eq!(rows.len(), 2, "{rows:?}");
    assert_eq!(rows[1]["outcome"], json!("refused"));
}

// ── (i) storage per layer (OPS-8 part C) ────────────────────────────────

/// One measured layer, as the daily storage job wrote it: the day, the
/// layer, its tables, the bytes of the tables' current snapshots and the
/// bytes the object store holds under them.
///
/// `stored_bytes` is `None` for a `ClickHouse` layer: its bytes sit on the
/// engine's own disk, not in the warehouse bucket.
fn layer_row(day: &str, layer: &str, current_bytes: i64, stored_bytes: Option<i64>) -> Value {
    json!({
        "day": day,
        "layer": layer,
        "table_count": "3",
        "total_bytes": current_bytes.to_string(),
        "stored_bytes": stored_bytes.map(|bytes| bytes.to_string()),
        "tables_skipped": "0",
        "skipped_reason": "",
        "measured_at": format!("{day} 01:00:12.000"),
    })
}

/// What the engine answers for the deploy-window re-read (`PR #591 review 1`):
/// the row shape of a table that predates the disk column — the
/// `stored_bytes` key is not in the row at all, the way a `SELECT` that
/// cannot name the column really comes back.
fn layer_row_without_disk_column(day: &str, layer: &str, current_bytes: i64) -> Value {
    json!({
        "day": day,
        "layer": layer,
        "table_count": "3",
        "total_bytes": current_bytes.to_string(),
        "tables_skipped": "0",
        "skipped_reason": "",
        "measured_at": format!("{day} 01:00:12.000"),
    })
}

/// The bucket's own measurement, as the capacity job wrote it.
fn bucket_row(measured_at: &str, name: &str, bytes: i64) -> Value {
    json!({
        "measured_at": measured_at,
        "bucket_name": name,
        "bytes": bytes.to_string(),
        "objects": "12",
    })
}

/// The engine's own answer when the table it was asked about is not there:
/// the shape `ClickHouse` 26.7.3.19 writes, observed over its HTTP
/// interface (the same text `routes/lakehouse.rs` classifies).
fn unknown_table(what: &str) -> ResponseTemplate {
    ResponseTemplate::new(404).set_body_string(format!(
        "Code: 60. DB::Exception: Unknown table expression identifier \
         'lake.bronze_meta.{what}' in scope SELECT 1 FROM \
         lake.`bronze_meta.{what}`. (UNKNOWN_TABLE) \
         (version 26.7.3.19 (official build))"
    ))
}

/// Any other failure of the engine.
fn broken_engine() -> ResponseTemplate {
    ResponseTemplate::new(500).set_body_string(
        "Code: 62. DB::Exception: Syntax error: failed at position 1 (SYNTAX_ERROR)",
    )
}

/// The engine's own answer when the column it was asked about is not in the
/// table: the shape `ClickHouse` writes for code 47, observed over its HTTP
/// interface — the code, the `(UNKNOWN_IDENTIFIER)` spelling, and the
/// column named once in the exception text.
fn unknown_column(column: &str) -> ResponseTemplate {
    ResponseTemplate::new(404).set_body_string(format!(
        "Code: 47. DB::Exception: Missing columns: '{column}' while \
         processing query: SELECT day, layer FROM \
         lake.bronze_meta.storage_layer_snapshot. (UNKNOWN_IDENTIFIER) \
         (version 26.7.3.19 (official build))"
    ))
}

/// A `ClickHouse` that answers the storage read with `layers`, the capacity
/// read with `bucket`, and every other query with no rows.
///
/// The specific mocks are mounted FIRST: among mocks of the same priority
/// `wiremock` keeps the insertion order, so the first one that matches
/// answers.
async fn clickhouse_with_storage(layers: Vec<Value>, bucket: Option<Value>) -> MockServer {
    let ch = MockServer::start().await;
    let count = layers.len();
    Mock::given(method("POST"))
        .and(body_string_contains("storage_layer_snapshot"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "meta": [],
            "data": layers,
            "rows": count,
        })))
        .mount(&ch)
        .await;
    Mock::given(method("POST"))
        .and(body_string_contains("capacity_snapshot"))
        .respond_with(match bucket {
            Some(row) => ResponseTemplate::new(200).set_body_json(json!({
                "meta": [],
                "data": [row],
                "rows": 1,
            })),
            None => ResponseTemplate::new(200).set_body_json(json!({
                "meta": [],
                "data": [],
                "rows": 0,
            })),
        })
        .mount(&ch)
        .await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "meta": [],
            "data": [],
            "rows": 0,
        })))
        .mount(&ch)
        .await;
    ch
}

/// The app of a test about the storage read, with the `ClickHouse` that
/// answers it.
async fn storage_app(ch: MockServer) -> (axum::Router, PgPool) {
    let mut overrides = HashMap::new();
    overrides.insert("CH_URL".to_owned(), ch.uri());
    let TestApp { router, pool } = spin_up_with_env(&overrides).await;
    (router, pool)
}

/// The session of the seeded Platform Admin, the one caller that holds `*:*`.
async fn platform_admin(pool: &PgPool) -> String {
    session_cookie_for_seeded_user(pool, "fajar@meridian.example").await
}

#[tokio::test]
async fn the_platform_operator_sees_the_size_of_every_layer() {
    let ch = clickhouse_with_storage(
        vec![
            layer_row("2026-03-15", "bronze", 1_000_000_000, Some(1_740_446_845)),
            layer_row("2026-03-15", "silver", 2_000_000_000, None),
        ],
        None,
    )
    .await;
    let (router, pool) = storage_app(ch).await;
    let admin = platform_admin(&pool).await;

    let answer = get(
        &router,
        &admin,
        "/api/usage/storage?from=2026-03-01&to=2026-03-31",
    )
    .await;
    assert_eq!(answer.status, StatusCode::OK, "{}", answer.text());
    let body = answer.json();
    assert_eq!(body["from"], json!("2026-03-01"), "{body}");
    assert_eq!(body["to"], json!("2026-03-31"), "{body}");
    assert_eq!(
        body["measuredAt"],
        json!("2026-03-15 01:00:12.000"),
        "{body}"
    );
    let rows = body["rows"].as_array().expect("rows");
    assert_eq!(rows.len(), 2, "{body}");
    assert_eq!(rows[0]["layer"], json!("bronze"), "{body}");
    assert_eq!(rows[0]["day"], json!("2026-03-15"), "{body}");
    assert_eq!(rows[0]["tableCount"], json!(3), "{body}");
    assert_eq!(rows[0]["currentBytes"], json!(1_000_000_000), "{body}");
    assert_eq!(rows[0]["storedBytes"], json!(1_740_446_845_i64), "{body}");
    assert_eq!(rows[0]["tablesSkipped"], json!(0), "{body}");
    assert_eq!(rows[0]["skippedReason"], json!(null), "{body}");
    assert_eq!(rows[1]["layer"], json!("silver"), "{body}");
    assert_eq!(rows[1]["currentBytes"], json!(2_000_000_000), "{body}");
    // A `ClickHouse` layer's bytes are not in the warehouse bucket: the job
    // wrote no number for them.
    assert_eq!(rows[1]["storedBytes"], json!(null), "{body}");
}

#[tokio::test]
async fn a_tenant_administrator_is_refused_the_storage_of_every_layer() {
    let ch = clickhouse_with_storage(
        vec![layer_row("2026-03-15", "bronze", 1_000, Some(1_000))],
        None,
    )
    .await;
    let (router, pool) = storage_app(ch).await;
    // A Governance Admin of one tenant: `usage:read` opens the Usage page,
    // and it does not open the storage of every tenant's tables.
    let admin = person(&pool, "usage:read", &[TENANT_A]).await;

    let answer = get(
        &router,
        &admin,
        "/api/usage/storage?from=2026-03-01&to=2026-03-31",
    )
    .await;
    assert_eq!(answer.status, StatusCode::FORBIDDEN, "{}", answer.text());
    assert_eq!(
        answer.json()["error"],
        json!("permission_denied: Storage per layer is shown to the platform operator only.")
    );
}

#[tokio::test]
async fn the_estimate_follows_the_rate_that_was_in_force_on_the_day() {
    let ch = clickhouse_with_storage(
        vec![
            layer_row("2026-03-05", "bronze", 1_000_000_000, Some(1_000_000_000)),
            layer_row("2026-03-15", "bronze", 1_000_000_000, Some(1_000_000_000)),
        ],
        None,
    )
    .await;
    let (router, pool) = storage_app(ch).await;
    let admin = platform_admin(&pool).await;
    // The price of a gigabyte for a month, from the tenth of March.
    let rate = post_rate(
        &router,
        &admin,
        json!({
            "kind": "storage_gb_month",
            "currency": "USD",
            "amount": "10",
            "effectiveFrom": "2026-03-10",
        }),
    )
    .await;
    assert_eq!(rate.status, StatusCode::CREATED, "{}", rate.text());

    let answer = get(
        &router,
        &admin,
        "/api/usage/storage?from=2026-03-01&to=2026-03-31",
    )
    .await;
    assert_eq!(answer.status, StatusCode::OK, "{}", answer.text());
    let body = answer.json();
    assert_eq!(body["currency"], json!("USD"), "{body}");
    let rows = body["rows"].as_array().expect("rows");
    // The day before the rate starts has no estimate at all: not zero.
    assert_eq!(rows[0]["estimate"], json!(null), "{body}");
    // One gigabyte (10^9 bytes) at 10.00 for a 31-day month: 10 / 31, to six places.
    assert_eq!(rows[1]["estimate"], json!("0.322581"), "{body}");
}

// ── (j) the storage of the layers (OPS-8) ──────────────────────────────

/// The daily job has never run here: the table it writes is not there, and
/// that is a measurement that has not happened yet, not an outage.
#[tokio::test]
async fn with_no_snapshot_table_the_storage_is_an_empty_measurement_not_an_outage() {
    let ch = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_string_contains("storage_layer_snapshot"))
        .respond_with(unknown_table("storage_layer_snapshot"))
        .mount(&ch)
        .await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "meta": [],
            "data": [],
            "rows": 0,
        })))
        .mount(&ch)
        .await;
    let (router, pool) = storage_app(ch).await;
    let admin = platform_admin(&pool).await;

    let answer = get(
        &router,
        &admin,
        "/api/usage/storage?from=2026-03-01&to=2026-03-31",
    )
    .await;
    assert_eq!(answer.status, StatusCode::OK, "{}", answer.text());
    let body = answer.json();
    assert_eq!(body["rows"], json!([]), "{body}");
    assert_eq!(body["measuredAt"], json!(null), "{body}");
    assert_eq!(body["bucket"], json!(null), "{body}");
}

/// The deploy window (`PR #591 review 1`): the daily job ran here before it
/// had the disk column, so the table exists WITHOUT `stored_bytes` and the
/// first read fails with `ClickHouse`'s unknown-column error. That is a
/// measurement that has not happened yet (plan decision 6), not an outage:
/// the route re-reads without the column and answers the current data with
/// `storedBytes: null` on every row, `measuredAt` from the rows, and no
/// bucket — the bucket's remainder is meaningless without disk numbers, so
/// it is not read at all. The capacity mock answers with a real row, so a
/// `bucket: null` here proves the route never asked for it.
#[tokio::test]
async fn a_table_that_predates_the_disk_column_reads_as_disk_not_measured_not_as_an_outage() {
    let ch = MockServer::start().await;
    // The first read names the disk column, so it is the query the engine
    // answers with the unknown-column error. Mounted FIRST: the fallback
    // read does not name the column, so this mock does not match it.
    Mock::given(method("POST"))
        .and(body_string_contains("stored_bytes"))
        .respond_with(unknown_column("stored_bytes"))
        .mount(&ch)
        .await;
    Mock::given(method("POST"))
        .and(body_string_contains("storage_layer_snapshot"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "meta": [],
            "data": [layer_row_without_disk_column("2026-03-15", "bronze", 1_000_000_000)],
            "rows": 1,
        })))
        .mount(&ch)
        .await;
    Mock::given(method("POST"))
        .and(body_string_contains("capacity_snapshot"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "meta": [],
            "data": [bucket_row(
                "2026-03-15 02:01:03.000",
                "lakehouse-warehouse",
                4_140_446_845,
            )],
            "rows": 1,
        })))
        .mount(&ch)
        .await;
    let (router, pool) = storage_app(ch).await;
    let admin = platform_admin(&pool).await;

    let answer = get(
        &router,
        &admin,
        "/api/usage/storage?from=2026-03-01&to=2026-03-31",
    )
    .await;
    assert_eq!(answer.status, StatusCode::OK, "{}", answer.text());
    let body = answer.json();
    let rows = body["rows"].as_array().expect("rows");
    assert_eq!(rows.len(), 1, "{body}");
    assert_eq!(rows[0]["layer"], json!("bronze"), "{body}");
    assert_eq!(rows[0]["currentBytes"], json!(1_000_000_000), "{body}");
    assert_eq!(rows[0]["storedBytes"], json!(null), "{body}");
    assert_eq!(
        body["measuredAt"],
        json!("2026-03-15 01:00:12.000"),
        "{body}"
    );
    assert_eq!(body["bucket"], json!(null), "{body}");
}

/// A code 47 about a column OTHER than the disk one is still the fixed 503:
/// only the deploy window is not an outage, and that window is about
/// `stored_bytes` alone (`PR #591 review 1`).
#[tokio::test]
async fn an_unknown_column_other_than_the_disk_one_is_still_the_fixed_503() {
    let ch = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_string_contains("storage_layer_snapshot"))
        .respond_with(unknown_column("total_bytes"))
        .mount(&ch)
        .await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "meta": [],
            "data": [],
            "rows": 0,
        })))
        .mount(&ch)
        .await;
    let (router, pool) = storage_app(ch).await;
    let admin = platform_admin(&pool).await;

    let answer = get(
        &router,
        &admin,
        "/api/usage/storage?from=2026-03-01&to=2026-03-31",
    )
    .await;
    assert_eq!(
        answer.status,
        StatusCode::SERVICE_UNAVAILABLE,
        "{}",
        answer.text()
    );
    assert_eq!(
        answer.json()["error"],
        json!("storage per layer is unavailable")
    );
}

/// Any other failure of the engine is still the fixed 503: a failure must
/// never look like an empty result.
#[tokio::test]
async fn any_other_failure_of_the_engine_is_the_fixed_503() {
    let ch = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_string_contains("storage_layer_snapshot"))
        .respond_with(broken_engine())
        .mount(&ch)
        .await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "meta": [],
            "data": [],
            "rows": 0,
        })))
        .mount(&ch)
        .await;
    let (router, pool) = storage_app(ch).await;
    let admin = platform_admin(&pool).await;

    let answer = get(
        &router,
        &admin,
        "/api/usage/storage?from=2026-03-01&to=2026-03-31",
    )
    .await;
    assert_eq!(
        answer.status,
        StatusCode::SERVICE_UNAVAILABLE,
        "{}",
        answer.text()
    );
    assert_eq!(
        answer.json()["error"],
        json!("storage per layer is unavailable")
    );
}

/// The bucket holds what no current table accounts for: its own size minus
/// the bytes on disk under the newest day's tables. The `ClickHouse` layers
/// are left out of that sum, because their bytes are not in the bucket.
#[tokio::test]
async fn the_bucket_holds_what_no_current_table_accounts_for() {
    let ch = clickhouse_with_storage(
        vec![
            layer_row("2026-03-14", "bronze", 1_000_000_000, Some(1_000_000_000)),
            layer_row("2026-03-15", "bronze", 1_000_000_000, Some(1_740_446_845)),
            layer_row("2026-03-15", "silver", 4_096, None),
        ],
        Some(bucket_row(
            "2026-03-15 02:01:03.000",
            "lakehouse-warehouse",
            4_140_446_845,
        )),
    )
    .await;
    let (router, pool) = storage_app(ch).await;
    let admin = platform_admin(&pool).await;

    let answer = get(
        &router,
        &admin,
        "/api/usage/storage?from=2026-03-01&to=2026-03-31",
    )
    .await;
    assert_eq!(answer.status, StatusCode::OK, "{}", answer.text());
    let body = answer.json();
    assert_eq!(
        body["bucket"]["name"],
        json!("lakehouse-warehouse"),
        "{body}"
    );
    assert_eq!(body["bucket"]["bytes"], json!(4_140_446_845_i64), "{body}");
    assert_eq!(
        body["bucket"]["measuredAt"],
        json!("2026-03-15 02:01:03.000"),
        "{body}"
    );
    // The newest day's disk sum is the bronze layer's 1_740_446_845 bytes;
    // the `ClickHouse` layer's 4_096 bytes are not in the bucket.
    assert_eq!(
        body["bucket"]["unaccountedBytes"],
        json!(4_140_446_845_i64 - 1_740_446_845_i64),
        "{body}"
    );
}

/// Two measurements that disagree are not comparable: the page says so
/// instead of showing a negative number.
#[tokio::test]
async fn a_bucket_smaller_than_the_tables_is_not_comparable() {
    let ch = clickhouse_with_storage(
        vec![layer_row(
            "2026-03-15",
            "bronze",
            1_000_000_000,
            Some(1_000_000_000),
        )],
        Some(bucket_row(
            "2026-03-15 02:01:03.000",
            "lakehouse-warehouse",
            999_999_999,
        )),
    )
    .await;
    let (router, pool) = storage_app(ch).await;
    let admin = platform_admin(&pool).await;

    let answer = get(
        &router,
        &admin,
        "/api/usage/storage?from=2026-03-01&to=2026-03-31",
    )
    .await;
    assert_eq!(answer.status, StatusCode::OK, "{}", answer.text());
    let body = answer.json();
    assert_eq!(body["bucket"]["bytes"], json!(999_999_999), "{body}");
    assert_eq!(body["bucket"]["unaccountedBytes"], json!(null), "{body}");
}

/// The bucket was never measured: the capacity table is not there, and that
/// is not a failure -- the layer rows are still the answer.
#[tokio::test]
async fn with_no_capacity_table_the_layers_are_still_there() {
    let ch = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_string_contains("storage_layer_snapshot"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "meta": [],
            "data": [layer_row("2026-03-15", "bronze", 1_000_000_000, Some(1_000_000_000))],
            "rows": 1,
        })))
        .mount(&ch)
        .await;
    Mock::given(method("POST"))
        .and(body_string_contains("capacity_snapshot"))
        .respond_with(unknown_table("capacity_snapshot"))
        .mount(&ch)
        .await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "meta": [],
            "data": [],
            "rows": 0,
        })))
        .mount(&ch)
        .await;
    let (router, pool) = storage_app(ch).await;
    let admin = platform_admin(&pool).await;

    let answer = get(
        &router,
        &admin,
        "/api/usage/storage?from=2026-03-01&to=2026-03-31",
    )
    .await;
    assert_eq!(answer.status, StatusCode::OK, "{}", answer.text());
    let body = answer.json();
    assert_eq!(body["rows"].as_array().expect("rows").len(), 1, "{body}");
    assert_eq!(body["bucket"], json!(null), "{body}");
}

/// The estimate is priced on the bytes ON DISK, not on the bytes of the
/// current snapshots: the bucket is what the warehouse costs.
#[tokio::test]
async fn the_estimate_is_priced_on_the_bytes_on_disk() {
    let ch = clickhouse_with_storage(
        vec![layer_row(
            "2026-03-15",
            "bronze",
            1_000_000_000,
            Some(2_000_000_000),
        )],
        None,
    )
    .await;
    let (router, pool) = storage_app(ch).await;
    let admin = platform_admin(&pool).await;
    let rate = post_rate(
        &router,
        &admin,
        json!({
            "kind": "storage_gb_month",
            "currency": "USD",
            "amount": "10",
            "effectiveFrom": "2026-03-01",
        }),
    )
    .await;
    assert_eq!(rate.status, StatusCode::CREATED, "{}", rate.text());

    let answer = get(
        &router,
        &admin,
        "/api/usage/storage?from=2026-03-01&to=2026-03-31",
    )
    .await;
    assert_eq!(answer.status, StatusCode::OK, "{}", answer.text());
    let body = answer.json();
    let rows = body["rows"].as_array().expect("rows");
    assert_eq!(rows[0]["currentBytes"], json!(1_000_000_000), "{body}");
    // Two gigabytes on disk at 10.00 for a 31-day month: 20 / 31, to six
    // places. Priced on the current data it would have been "0.322581".
    assert_eq!(rows[0]["estimate"], json!("0.645161"), "{body}");
}
