//! Chart alerts end to end through `GET /api/alerts/run` (`BI-19` part C, D11
//! and D12): the number is read as the person who set the alert, the rule's
//! filter is in the statement, the alert fires once and recovers once, an owner
//! who lost access stops it, and a restricted dashboard's alert goes only to
//! its owner's own address (Q-R12).
//!
//! The `ClickHouse` here is a recording fake: one rule, one board, one KPI
//! chart. A rule is silenced so that "delivered" needs no mail server: a
//! silence hides the message, not the fact (the state still advances).

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::body::{Body, to_bytes};
use axum::http::Request;
use common::{TestApp, session_cookie_for_user, spin_up_with_env};
use lakehouse_store::dashboard_access::{NewRule, replace_item};
use lakehouse_store::overview::{FiredRule, insert_from_fired_rule, silence_alert};
use serde_json::{Value, json};
use sqlx::PgPool;
use time::OffsetDateTime;
use tower::ServiceExt;
use uuid::Uuid;
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, Request as MockRequest, Respond, ResponseTemplate};

const BOARD: &str = "b_pub";
const RULE: &str = "al_c1";
const TOKEN: &str = "run-token-not-real";

#[derive(Clone)]
struct FakeCh {
    owner: String,
    target: Arc<Mutex<String>>,
    value: Arc<Mutex<f64>>,
    chart_exists: Arc<Mutex<bool>>,
    log: Arc<Mutex<Vec<String>>>,
}

fn rule_row(owner: &str, target: &str) -> Value {
    json!({
        "id": RULE, "name": "Visitors in Bali", "type": "alert", "mart": "", "measure": "",
        "agg": "sum", "op": ">", "threshold": "100", "board": "", "channel": "email",
        "target": target, "enabled": "1", "created_at": "2026-10-26 00:00:00",
        "severity": "", "pipeline": "", "metric": "", "connector": "", "asset": "",
        "chart": "c_kpi", "series": "", "condition": "above", "owner_id": owner,
        "filters_json": r#"[{"column":"region","values":["Bali"]}]"#,
    })
}

fn chart_row() -> Value {
    let spec = json!({
        "id": "c_kpi", "title": "Visitors", "kind": "kpi", "mart": "mart_x",
        "sql": "SELECT sum(visitors) AS visitors FROM serving.mart_x", "x": "", "y": "visitors"
    });
    let def = json!({
        "title": "Visitors", "mart": "mart_x", "kind": "kpi", "measures": ["visitors"],
        "aggregate": "sum"
    });
    json!({
        "id": "c_kpi", "board": BOARD, "created_by": "ui", "created_at": "2026-01-01 00:00:00",
        "spec_json": json!({ "spec": spec, "def": def, "hasYear": false }).to_string(),
    })
}

fn board_row() -> Value {
    json!({
        "id": BOARD, "name": "Public board", "description": "", "created_by": "t",
        "layout_json": "", "filters_json": "", "public_token": "", "embed_enabled": "0",
        "folder_id": "", "created_at": "2026-10-10 00:00:00", "owner_id": "",
        "tabs_json": "", "embed_revoked_before": "0", "embed_revoked_jti_json": "[]",
        "embed_origins_json": "[]"
    })
}

impl Respond for FakeCh {
    fn respond(&self, request: &MockRequest) -> ResponseTemplate {
        let sql = String::from_utf8_lossy(&request.body).into_owned();
        self.log.lock().unwrap().push(sql.clone());
        let rows = |meta: Value, data: Vec<Value>| {
            let n = data.len();
            ResponseTemplate::new(200)
                .set_body_json(json!({ "meta": meta, "data": data, "rows": n }))
        };
        if sql.contains("FROM console.alert_rule") {
            return rows(
                json!([]),
                vec![rule_row(&self.owner, &self.target.lock().unwrap())],
            );
        }
        if sql.contains("FROM console.bi_chart") {
            let present = *self.chart_exists.lock().unwrap();
            return rows(json!([]), if present { vec![chart_row()] } else { vec![] });
        }
        if sql.contains("FROM console.bi_board") {
            return rows(json!([]), vec![board_row()]);
        }
        if sql.contains("system.columns") {
            return rows(
                json!([]),
                [("region", "String"), ("visitors", "Float64")]
                    .iter()
                    .map(|(n, t)| json!({"table": "mart_x", "name": n, "type": t}))
                    .collect(),
            );
        }
        if sql.contains("FROM serving.mart_x") {
            return rows(
                json!([{"name": "visitors", "type": "Float64"}]),
                vec![json!({ "visitors": *self.value.lock().unwrap() })],
            );
        }
        ResponseTemplate::new(200).set_body_string("Ok.\n")
    }
}

struct Fixture {
    app: TestApp,
    fake: FakeCh,
    owner: Uuid,
    _ch: MockServer,
}

async fn fixture(target: &str) -> Fixture {
    let owner = Uuid::new_v4();
    let fake = FakeCh {
        owner: owner.to_string(),
        target: Arc::new(Mutex::new(target.to_owned())),
        value: Arc::new(Mutex::new(150.0)),
        chart_exists: Arc::new(Mutex::new(true)),
        log: Arc::new(Mutex::new(Vec::new())),
    };
    let ch = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(fake.clone())
        .mount(&ch)
        .await;
    let env: HashMap<String, String> = HashMap::from([
        ("CH_URL".to_owned(), ch.uri()),
        ("ALERTS_RUN_TOKEN".to_owned(), TOKEN.to_owned()),
    ]);
    let app = spin_up_with_env(&env).await;
    let role = Uuid::new_v4();
    sqlx::query("INSERT INTO role (id, name, permissions, description) VALUES ($1, 'Chart Owner', 'dashboard:read', 'test')")
        .bind(role)
        .execute(&app.pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO app_user (id, name, email, status) VALUES ($1, 'Owner', 'owner@test.invalid', 'active')")
        .bind(owner)
        .execute(&app.pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO app_user_role (user_id, role_id) VALUES ($1, $2)")
        .bind(owner)
        .bind(role)
        .execute(&app.pool)
        .await
        .unwrap();
    Fixture {
        app,
        fake,
        owner,
        _ch: ch,
    }
}

/// Silence the rule so that a message needs no mail server.
async fn silence(pool: &PgPool) -> String {
    let now = OffsetDateTime::now_utc();
    let item = insert_from_fired_rule(
        pool,
        &FiredRule {
            rule_id: RULE,
            title: "Visitors in Bali",
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
    silence_alert(pool, &item.id, now + time::Duration::hours(1))
        .await
        .unwrap();
    item.id
}

async fn run(f: &Fixture) -> Value {
    let cookie = session_cookie_for_user(&f.app.pool, f.owner).await;
    let resp = f
        .app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/api/alerts/run?id={RULE}"))
                .header("cookie", cookie)
                .header("x-run-token", TOKEN)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    body["results"][0].clone()
}

async fn state_of(pool: &PgPool) -> Option<(String, Option<String>)> {
    sqlx::query_as("SELECT state, reason FROM alert_rule_state WHERE rule_id = $1")
        .bind(RULE)
        .fetch_optional(pool)
        .await
        .unwrap()
}

/// D11 and D12 together: the statement carries the rule's filter, the crossing
/// is sent once over three checks, the recovery once, and the seeded instance
/// is resolved.
#[tokio::test]
async fn a_chart_alert_reads_with_its_filter_fires_once_and_recovers_once() {
    let f = fixture("owner@test.invalid").await;
    let seeded = silence(&f.app.pool).await;
    let mut fired = 0;
    for _ in 0..3 {
        let r = run(&f).await;
        assert_eq!(r["skipped"], Value::Null, "{r}");
        assert_eq!(r["value"].as_f64(), Some(150.0));
        fired += usize::from(r["fired"] == json!(true));
    }
    assert_eq!(fired, 1, "over the line three times, told once");
    assert_eq!(state_of(&f.app.pool).await.unwrap().0, "firing");
    assert!(
        f.fake
            .log
            .lock()
            .unwrap()
            .iter()
            .any(|sql| sql.contains("FROM serving.mart_x") && sql.contains("'Bali'")),
        "the rule's filter must be in the statement"
    );

    *f.fake.value.lock().unwrap() = 40.0;
    let back = run(&f).await;
    assert_eq!(back["recovered"], json!(true), "{back}");
    assert_eq!(back["fired"], json!(false));
    assert_eq!(state_of(&f.app.pool).await.unwrap().0, "ok");
    let status: String = sqlx::query_scalar("SELECT status FROM alert_instance WHERE id = $1")
        .bind(&seeded)
        .fetch_one(&f.app.pool)
        .await
        .unwrap();
    assert_eq!(status, "resolved");
    let again = run(&f).await;
    assert_eq!(again["recovered"], Value::Null);
}

/// C4: an owner who is disabled stops the alert with the reason, and nothing
/// is read or sent.
#[tokio::test]
async fn an_owner_who_is_disabled_stops_the_alert_and_says_why() {
    let f = fixture("owner@test.invalid").await;
    silence(&f.app.pool).await;
    let cookie_owner = f.owner;
    // A second, active person triggers the run; the owner is then disabled.
    let other = Uuid::new_v4();
    sqlx::query("INSERT INTO app_user (id, name, email, status) VALUES ($1, 'Other', 'other@test.invalid', 'active')")
        .bind(other)
        .execute(&f.app.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE app_user SET status = 'inactive' WHERE id = $1")
        .bind(cookie_owner)
        .execute(&f.app.pool)
        .await
        .unwrap();
    let cookie = session_cookie_for_user(&f.app.pool, other).await;
    let resp = f
        .app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/alerts/run?id={RULE}"))
                .header("cookie", cookie)
                .header("x-run-token", TOKEN)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body: Value =
        serde_json::from_slice(&to_bytes(resp.into_body(), usize::MAX).await.unwrap()).unwrap();
    let r = &body["results"][0];
    assert_eq!(r["fired"], json!(false), "{r}");
    assert!(r["skipped"].as_str().unwrap().contains("disabled"), "{r}");
    let (state, reason) = state_of(&f.app.pool).await.unwrap();
    assert_eq!(state, "stopped");
    assert!(reason.unwrap().contains("disabled"));
    assert!(
        !f.fake
            .log
            .lock()
            .unwrap()
            .iter()
            .any(|sql| sql.contains("FROM serving.mart_x")),
        "no data was read for a stopped alert"
    );
}

/// A chart that was deleted stops the alert and says so.
#[tokio::test]
async fn a_deleted_chart_stops_the_alert_and_says_so() {
    let f = fixture("owner@test.invalid").await;
    *f.fake.chart_exists.lock().unwrap() = false;
    let r = run(&f).await;
    assert!(r["skipped"].as_str().unwrap().contains("deleted"), "{r}");
    assert_eq!(state_of(&f.app.pool).await.unwrap().0, "stopped");
}

/// Q-R12: on a restricted dashboard the owner, who is listed, still may not send
/// the alert to anybody but themselves; the check is recorded and nothing is read.
#[tokio::test]
async fn a_restricted_dashboards_alert_goes_only_to_its_owners_own_address() {
    let f = fixture("someone.else@example.com").await;
    replace_item(
        &f.app.pool,
        "board",
        BOARD,
        &[NewRule {
            user_id: Some(f.owner),
            role_id: None,
            level: "view".to_owned(),
        }],
        true,
        None,
    )
    .await
    .unwrap();
    let r = run(&f).await;
    assert!(r["skipped"].as_str().unwrap().contains("restricted"), "{r}");
    assert!(
        !f.fake
            .log
            .lock()
            .unwrap()
            .iter()
            .any(|sql| sql.contains("FROM serving.mart_x")),
        "nothing is read when the target is not allowed"
    );
    // The same restriction with the owner's own address is read and judged.
    *f.fake.target.lock().unwrap() = "owner@test.invalid".to_owned();
    silence(&f.app.pool).await;
    let r = run(&f).await;
    assert_eq!(r["skipped"], Value::Null, "{r}");
    assert_eq!(r["fired"], json!(true));
}
