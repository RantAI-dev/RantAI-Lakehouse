//! `POST /api/alerts/test` end to end (`BI-40` D1 to D3, D6, D7): a signed-in
//! person tests one rule they may manage, the run is real (the rule settles, a
//! fired result is saved, a recovery resolves), a rule they cannot see answers
//! exactly as an unknown id, and the assistant's `run_alert_rule` asks the same
//! guard before it runs anything.
//!
//! The `ClickHouse` is a recording fake serving three rules: a threshold rule
//! over `serving.mart_x`, a `pipeline_late` rule and a connector rule of
//! Meridian Group. No mail server is configured for these apps, so a fired
//! email is a recorded delivery failure — the alert still settles and its
//! instance is still saved (`SRC-17` F1: the event happened). Nothing is sent
//! to a real host.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use crate::common;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use common::{TestApp, session_cookie_for_seeded_user, spin_up_with_env};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, Request as MockRequest, Respond, ResponseTemplate};

const PLAIN: &str = "al_plain";
const LATE: &str = "al_late";
const CONNECTOR: &str = "al_conn";
/// A connector of Meridian Group; Andi sees only Meridian Retail.
const GROUP_CONNECTOR: &str = "conn-pg-lakehouse";

struct FakeCh {
    value: Arc<Mutex<f64>>,
    /// The connector [`CONNECTOR`] watches, and what an insert makes it watch
    /// next: `save_rule` re-reads the row it just wrote (`get_rule`), so an
    /// edit only reads as a change of connector when this fake applies it.
    connector: Arc<Mutex<String>>,
    after_insert: Arc<Mutex<Option<String>>>,
}

fn plain_rule() -> Value {
    json!({
        "id": PLAIN, "name": "Visitors over the line", "type": "alert", "mart": "mart_x",
        "measure": "visitors", "agg": "sum", "op": ">", "threshold": "100", "board": "",
        "channel": "email", "target": "bayu@meridian.example", "enabled": "1",
        "created_at": "2026-11-02 00:00:00", "severity": "", "pipeline": "", "metric": "",
        "connector": "", "asset": "", "chart": "", "series": "", "condition": "",
        "owner_id": "", "filters_json": "[]",
    })
}

fn late_rule() -> Value {
    json!({
        "id": LATE, "name": "Nightly load is late", "type": "pipeline_late", "mart": "",
        "measure": "", "agg": "sum", "op": ">", "threshold": "0", "board": "",
        "channel": "email", "target": "bayu@meridian.example", "enabled": "1",
        "created_at": "2026-11-02 00:00:00", "severity": "", "pipeline": "daily_visits",
        "metric": "", "connector": "", "asset": "", "chart": "", "series": "",
        "condition": "", "owner_id": "", "filters_json": "[]",
    })
}

fn connector_rule(connector: &str) -> Value {
    json!({
        "id": CONNECTOR, "name": "Source keeps failing", "type": "connector_failure",
        "mart": "", "measure": "", "agg": "sum", "op": ">", "threshold": "0", "board": "",
        "channel": "email", "target": "bayu@meridian.example", "enabled": "1",
        "created_at": "2026-11-02 00:00:00", "severity": "", "pipeline": "", "metric": "",
        "connector": connector, "asset": "", "chart": "", "series": "",
        "condition": "", "owner_id": "", "filters_json": "[]",
    })
}

impl Respond for FakeCh {
    fn respond(&self, request: &MockRequest) -> ResponseTemplate {
        let sql = String::from_utf8_lossy(&request.body).into_owned();
        let rows = |data: Vec<Value>| {
            let n = data.len();
            ResponseTemplate::new(200).set_body_json(json!({ "meta": [], "data": data, "rows": n }))
        };
        if sql.contains("INSERT INTO console.alert_rule") {
            let mut held = self.after_insert.lock().unwrap();
            if let Some(next) = held.take() {
                *self.connector.lock().unwrap() = next;
            }
            return ResponseTemplate::new(200).set_body_string("Ok.\n");
        }
        if sql.contains("FROM console.alert_rule") {
            // `get_rule` names one id; `list_rules` names none.
            let data = if sql.contains(&format!("id='{PLAIN}'")) {
                vec![plain_rule()]
            } else if sql.contains(&format!("id='{LATE}'")) {
                vec![late_rule()]
            } else if sql.contains(&format!("id='{CONNECTOR}'")) {
                vec![connector_rule(&self.connector.lock().unwrap())]
            } else if sql.contains("id=") {
                Vec::new()
            } else {
                vec![
                    plain_rule(),
                    late_rule(),
                    connector_rule(&self.connector.lock().unwrap()),
                ]
            };
            return rows(data);
        }
        if sql.contains("system.columns") {
            return rows(vec![
                json!({ "table": "mart_x", "name": "visitors", "type": "Float64" }),
            ]);
        }
        if sql.contains("FROM serving.mart_x") {
            return rows(vec![json!({ "v": *self.value.lock().unwrap() })]);
        }
        ResponseTemplate::new(200).set_body_string("Ok.\n")
    }
}

/// The app under test, with `serving.mart_x` in the tenant that owns it
/// (`SEC-50` slice 2 M11).
///
/// A person testing a rule reads as THEMSELVES (the tenant gate runs as the
/// caller, `GateActor::Caller`), not as the scheduler's catalog tenant, so a
/// test over the shared catalog needs a deployment that names the catalog's
/// tenant and callers who are members of it. The seeded identity migration
/// holds four tenants and this app named no `CATALOG_TENANT_ID`, so the gate
/// found no tenant that owns the catalog and refused the statement with
/// `SEC-20`'s 404 ("Table not found or not available to your organisation:
/// `serving.mart_x`") — the fixture, not the route, was wrong. The membership
/// is added here, where the app is built, so every test in this file has the
/// same shape (`common::in_the_catalog_tenant`, as the other gated tests do).
async fn app() -> TestApp {
    app_with_connector_edits().await.0
}

/// [`app`], with the handle that makes the fake apply a saved rule's new
/// connector on the read `save_rule` does right after its insert.
async fn app_with_connector_edits() -> (TestApp, Arc<Mutex<Option<String>>>) {
    let ch = MockServer::start().await;
    let after_insert = Arc::new(Mutex::new(None));
    Mock::given(method("POST"))
        .respond_with(FakeCh {
            value: Arc::new(Mutex::new(150.0)),
            connector: Arc::new(Mutex::new(GROUP_CONNECTOR.to_owned())),
            after_insert: Arc::clone(&after_insert),
        })
        .mount(&ch)
        .await;
    let env: HashMap<String, String> = HashMap::from([
        ("CH_URL".to_owned(), ch.uri()),
        (
            "CATALOG_TENANT_ID".to_owned(),
            common::CATALOG_TENANT.to_owned(),
        ),
    ]);
    let app = spin_up_with_env(&env).await;
    // Only the caller who reads a mart needs the membership: the two refusals
    // (an unknown id, and a connector rule of a tenant the caller does not
    // see) must keep Andi outside Meridian Group, or she would see its
    // connector rule.
    let user: Uuid = sqlx::query_scalar("SELECT id FROM app_user WHERE email = $1")
        .bind("bayu@meridian.example")
        .fetch_one(&app.pool)
        .await
        .expect("a seeded user");
    common::in_the_catalog_tenant(&app.pool, user).await;
    (app, after_insert)
}

async fn test_run(app: &TestApp, cookie: &str, id: &str) -> (StatusCode, Value) {
    let response = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/alerts/test?id={id}"))
                .header("cookie", cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("router never fails a request outright");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

/// D2.1: the route is no oracle. A connector rule of a tenant the caller does
/// not see answers exactly as an id that does not exist.
#[tokio::test]
async fn an_unknown_rule_and_one_the_caller_cannot_see_answer_the_same_not_found() {
    let app = app().await;
    let andi = session_cookie_for_seeded_user(&app.pool, "andi@meridian.example").await;

    let (unknown_status, unknown) = test_run(&app, &andi, "al_does_not_exist").await;
    assert_eq!(unknown_status, StatusCode::NOT_FOUND);
    let (hidden_status, hidden) = test_run(&app, &andi, CONNECTOR).await;
    assert_eq!(hidden_status, StatusCode::NOT_FOUND);
    assert_eq!(hidden, unknown, "the two refusals must read the same");
    assert_eq!(hidden["error"], json!("alert rule not found."));
}

/// D3: a test run is a real run. Over the line it fires, the delivery is
/// attempted, and a fired result is saved as one `alert_instance`. No mail
/// server is configured for this app, so the delivery is a recorded failure:
/// the crossing still counts (`SRC-17` F1) and the state goes back to `ok` so
/// the next check may send it — which is why the once-only case below silences
/// the rule instead.
#[tokio::test]
async fn a_threshold_rule_over_its_line_fires_and_saves_one_instance() {
    let app = app().await;
    let bayu = session_cookie_for_seeded_user(&app.pool, "bayu@meridian.example").await;

    let (status, body) = test_run(&app, &bayu, PLAIN).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["ran"], json!(1), "{body}");
    assert_eq!(body["results"][0]["fired"], json!(true), "{body}");
    assert_eq!(body["results"][0]["value"].as_f64(), Some(150.0), "{body}");

    let saved: i64 = sqlx::query_scalar("SELECT count(*) FROM alert_instance WHERE rule_id = $1")
        .bind(PLAIN)
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(saved, 1, "one crossing, one instance");
}

/// D11 through this route: a silenced rule still settles, so a second test
/// while the value is still over sends nothing and the rule stays firing (a
/// silence hides the message, not the fact).
#[tokio::test]
async fn a_second_test_while_still_over_the_line_sends_nothing() {
    let app = app().await;
    let bayu = session_cookie_for_seeded_user(&app.pool, "bayu@meridian.example").await;
    silence(&app.pool).await;

    let (status, first) = test_run(&app, &bayu, PLAIN).await;
    assert_eq!(status, StatusCode::OK, "{first}");
    assert_eq!(first["results"][0]["fired"], json!(true), "{first}");

    let (status, second) = test_run(&app, &bayu, PLAIN).await;
    assert_eq!(status, StatusCode::OK, "{second}");
    assert_eq!(second["results"][0]["fired"], json!(false), "{second}");

    let state: String = sqlx::query_scalar("SELECT state FROM alert_rule_state WHERE rule_id = $1")
        .bind(PLAIN)
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(state, "firing");
}

/// Seed one open, silenced instance for the threshold rule, so that a fire
/// needs no mail server (the same posture `tests/chart_alerts.rs` takes).
async fn silence(pool: &sqlx::PgPool) {
    let now = time::OffsetDateTime::now_utc();
    let item = lakehouse_store::overview::insert_from_fired_rule(
        pool,
        &lakehouse_store::overview::FiredRule {
            rule_id: PLAIN,
            title: "Visitors over the line",
            severity: Some("low"),
            source: "Alert rules",
            affected: "",
            detail: "seed",
        },
        now - time::Duration::hours(1),
    )
    .await
    .unwrap()
    .unwrap();
    lakehouse_store::overview::silence_alert(pool, &item.id, now + time::Duration::hours(1))
        .await
        .unwrap();
}

/// D3, honest: a `pipeline_late` rule is the 15-minute pass's, and the test run
/// says so instead of pretending to check it.
#[tokio::test]
async fn a_late_pipeline_rule_answers_that_the_fifteen_minute_pass_checks_it() {
    let app = app().await;
    let bayu = session_cookie_for_seeded_user(&app.pool, "bayu@meridian.example").await;
    let (status, body) = test_run(&app, &bayu, LATE).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["results"][0]["skipped"],
        json!("A late-pipeline alert is checked by the 15-minute pass, not by a test run."),
        "{body}"
    );
}

/// D7: the assistant runs through the same guard before `run_one`, and its
/// refusal is D2.1's sentence — tenant A's `alert:write` does not reach tenant
/// B's connector rule.
#[tokio::test]
async fn the_assistant_refuses_another_tenants_rule_with_the_same_sentence() {
    let app = app().await;
    let andi = session_cookie_for_seeded_user(&app.pool, "andi@meridian.example").await;
    let response = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/ai/tool")
                .header("content-type", "application/json")
                .header("cookie", andi)
                .body(Body::from(
                    json!({
                        "tool": "run_alert_rule",
                        "args": { "id": CONNECTOR, "confirmed": true },
                        "mode": "build",
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .expect("router never fails a request outright");
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        body["result"]["error"],
        json!("alert rule not found."),
        "{body}"
    );
}

/// `PUT /api/alerts` with `body` as `bayu`; the answer's status and JSON body.
async fn put_rule(app: &TestApp, cookie: &str, body: Value) -> (StatusCode, Value) {
    let response = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/alerts")
                .header("content-type", "application/json")
                .header("cookie", cookie)
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .expect("router never fails a request outright");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

/// A `connector_failure` rule watching `connector`, named `name`, to
/// `bayu@meridian.example`: the stored rule [`CONNECTOR`], edited.
fn connector_rule_body(name: &str, connector: &str) -> Value {
    json!({
        "id": CONNECTOR, "name": name, "type": "connector_failure", "connector": connector,
        "channel": "email", "target": "bayu@meridian.example",
    })
}

/// Whether [`CONNECTOR`] has a latest-delivery row.
async fn has_delivery(app: &TestApp) -> bool {
    lakehouse_store::alert_delivery::last_for(&app.pool, &[CONNECTOR.to_owned()])
        .await
        .expect("the delivery rows")
        .contains_key(CONNECTOR)
}

/// Record one delivery attempt of [`CONNECTOR`], as a success event would.
async fn record_delivery(app: &TestApp) {
    lakehouse_store::alert_delivery::record(
        &app.pool,
        CONNECTOR,
        lakehouse_store::overview::DeliveryRecord::NotDelivered(
            "the webhook server answered with an error",
        ),
        time::OffsetDateTime::now_utc(),
    )
    .await
    .expect("a delivery row");
}

/// `SRC-40` D5 (review round 1, R2): an edit that changes what a rule watches
/// drops its latest delivery — here a `connector_failure` rule moved to
/// another connector, whose "Last sent" would otherwise describe the old one.
/// Fails if the `alert_delivery::delete` call in
/// `routes::alerts::forget_state` is removed.
#[tokio::test]
async fn an_edit_that_changes_the_connector_drops_the_rules_last_delivery() {
    let (app, after_insert) = app_with_connector_edits().await;
    let bayu = session_cookie_for_seeded_user(&app.pool, "bayu@meridian.example").await;
    record_delivery(&app).await;

    // `conn-s3-warehouse` is another seeded connector of the same tenant, so
    // the edit is one the caller may make; the fake serves it after the insert,
    // the way `ClickHouse` would.
    *after_insert.lock().unwrap() = Some("conn-s3-warehouse".to_owned());
    let (status, body) = put_rule(
        &app,
        &bayu,
        connector_rule_body("Source keeps failing", "conn-s3-warehouse"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["rule"]["connector"],
        json!("conn-s3-warehouse"),
        "{body}"
    );
    assert!(
        !has_delivery(&app).await,
        "a rule that watches another connector starts with no last delivery"
    );
}

/// `SRC-40` D5 (review round 1, R2): an edit of the name alone keeps it — the
/// rule still watches the same connector, and its "Last sent" still holds.
#[tokio::test]
async fn an_edit_of_the_name_alone_keeps_the_rules_last_delivery() {
    let app = app().await;
    let bayu = session_cookie_for_seeded_user(&app.pool, "bayu@meridian.example").await;
    record_delivery(&app).await;

    let (status, body) = put_rule(
        &app,
        &bayu,
        connector_rule_body("Source keeps failing today", GROUP_CONNECTOR),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        has_delivery(&app).await,
        "the same connector keeps the rule's last delivery"
    );
}
