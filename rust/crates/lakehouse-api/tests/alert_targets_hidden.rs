//! `DEC-16` (QA `LH-SRC-7-04`, `SRC-42`): a rule's target — a webhook address
//! that can carry a token, or an e-mail address — is returned only to a caller
//! who may manage the rule. `GET /api/alerts` and the assistant's
//! `list_alert_rules` go through the same helper, so both hide it; two pins
//! keep the other alert bodies free of it.
//!
//! The `ClickHouse` here is a recording fake holding one stored mart rule whose
//! target is [`ADDRESS`]. Two callers are minted with exactly `alert:read` and
//! `alert:read, alert:write`, so the difference in the answer is the permission
//! and nothing else.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use crate::common;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use common::{TestApp, create_assistant_principal, session_cookie_for_user, spin_up_with_env};
use lakehouse_store::overview::{DeliveryRecord, FiredRule, insert_from_fired_rule};
use serde_json::{Value, json};
use time::OffsetDateTime;
use tower::ServiceExt;
use uuid::Uuid;
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, Request as MockRequest, Respond, ResponseTemplate};

const RULE: &str = "al_1";
/// The address the rule sends to; no answer to a caller without `alert:write`
/// may hold it.
const ADDRESS: &str = "https://hooks.example.com/secret-token";

/// The one stored rule: a mart alert sent to [`ADDRESS`] over a webhook.
fn rule_row() -> Value {
    json!({
        "id": RULE, "name": "Orders", "type": "alert", "mart": "mart_orders",
        "measure": "orders", "agg": "sum", "op": ">", "threshold": "10", "board": "",
        "channel": "webhook", "target": ADDRESS, "enabled": "1",
        "created_at": "2026-10-26 00:00:00", "severity": "", "pipeline": "", "metric": "",
        "connector": "", "asset": "", "chart": "", "series": "", "condition": "",
        "owner_id": "", "filters_json": "",
    })
}

/// A recording `ClickHouse`: the rule read answers [`rule_row`], everything
/// else an empty result. Every statement is kept so a test can read what was
/// asked.
#[derive(Clone)]
struct FakeCh {
    log: Arc<Mutex<Vec<String>>>,
}

impl Respond for FakeCh {
    fn respond(&self, request: &MockRequest) -> ResponseTemplate {
        let sql = String::from_utf8_lossy(&request.body).into_owned();
        self.log.lock().unwrap().push(sql.clone());
        if sql.contains("FROM console.alert_rule") {
            return ResponseTemplate::new(200).set_body_json(json!({
                "meta": [], "data": [rule_row()], "rows": 1
            }));
        }
        ResponseTemplate::new(200).set_body_json(json!({ "meta": [], "data": [], "rows": 0 }))
    }
}

struct Fixture {
    app: TestApp,
    _ch: MockServer,
    /// Every statement the `ClickHouse` fake was asked, in order.
    log: Arc<Mutex<Vec<String>>>,
    read_only: Uuid,
    writer: Uuid,
}

/// An app whose `ClickHouse` is the fake, and the two callers.
async fn fixture() -> Fixture {
    let log = Arc::new(Mutex::new(Vec::new()));
    let fake = FakeCh {
        log: Arc::clone(&log),
    };
    let ch = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(fake)
        .mount(&ch)
        .await;
    let env: HashMap<String, String> = HashMap::from([("CH_URL".to_owned(), ch.uri())]);
    let app = spin_up_with_env(&env).await;
    let read_only = create_assistant_principal(&app.pool, "alert:read").await;
    let writer = create_assistant_principal(&app.pool, "alert:read, alert:write").await;
    Fixture {
        app,
        _ch: ch,
        log,
        read_only,
        writer,
    }
}

async fn get(f: &Fixture, who: Uuid, uri: &str) -> (StatusCode, Value) {
    send(f, who, "GET", uri).await
}

/// `method` on `uri` as `who`; the answer's status and JSON body.
async fn send(f: &Fixture, who: Uuid, http_method: &str, uri: &str) -> (StatusCode, Value) {
    let cookie = session_cookie_for_user(&f.app.pool, who).await;
    let response = f
        .app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method(http_method)
                .uri(uri)
                .header("cookie", cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

/// `POST /api/ai/tool` with `list_alert_rules`; returns the tool's `result`.
async fn list_alert_rules(f: &Fixture, who: Uuid) -> Value {
    let cookie = session_cookie_for_user(&f.app.pool, who).await;
    let response = f
        .app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/ai/tool")
                .header("content-type", "application/json")
                .header("cookie", cookie)
                .body(Body::from(
                    json!({ "tool": "list_alert_rules", "args": {}, "mode": "ask" }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["outcome"], json!("executed"), "{body}");
    body["result"].clone()
}

#[tokio::test]
async fn get_alerts_hides_the_target_from_a_caller_without_alert_write() {
    let f = fixture().await;
    let (status, body) = get(&f, f.read_only, "/api/alerts").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let rule = &body["rules"][0];
    assert!(rule.get("target").is_none(), "{rule}");
    assert_eq!(rule["targetHidden"], json!(true));
    assert_eq!(rule["channel"], json!("webhook"));
    assert!(!body.to_string().contains(ADDRESS), "{body}");
}

#[tokio::test]
async fn get_alerts_returns_the_target_to_a_caller_with_alert_write() {
    let f = fixture().await;
    let (status, body) = get(&f, f.writer, "/api/alerts").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let rule = &body["rules"][0];
    assert_eq!(rule["target"], json!(ADDRESS));
    assert!(rule.get("targetHidden").is_none(), "{rule}");
}

#[tokio::test]
async fn the_assistants_rule_list_hides_the_target_from_a_caller_without_alert_write() {
    let f = fixture().await;
    let result = list_alert_rules(&f, f.read_only).await;
    let rule = &result["rules"][0];
    assert!(rule.get("target").is_none(), "{rule}");
    assert_eq!(rule["targetHidden"], json!(true));
    assert!(!result.to_string().contains(ADDRESS), "{result}");
}

#[tokio::test]
async fn the_assistants_rule_list_returns_the_target_to_a_caller_with_alert_write() {
    let f = fixture().await;
    let result = list_alert_rules(&f, f.writer).await;
    assert_eq!(result["rules"][0]["target"], json!(ADDRESS));
}

/// `SRC-40` D6 (`TC-1695`): a rule's latest delivery — when it last sent, or
/// that it did not and why — is listed for every caller who may list the rule,
/// here and in the assistant's `list_alert_rules`; and the body of a caller
/// without `alert:write` still holds no target. Fails if the `lastDelivery`
/// insert in `listed_rules` is removed.
#[tokio::test]
async fn the_rule_list_and_the_assistants_list_carry_a_rules_last_delivery() {
    const REASON: &str = "the webhook server answered with an error";
    let f = fixture().await;
    lakehouse_store::alert_delivery::record(
        &f.app.pool,
        RULE,
        DeliveryRecord::NotDelivered(REASON),
        OffsetDateTime::now_utc(),
    )
    .await
    .expect("a delivery row");

    for who in [f.read_only, f.writer] {
        let (status, body) = get(&f, who, "/api/alerts").await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let delivery = &body["rules"][0]["lastDelivery"];
        assert_eq!(delivery["delivered"], json!(false), "{body}");
        assert_eq!(delivery["reason"], json!(REASON), "{body}");
        assert!(delivery["at"].is_string(), "{body}");
        if who == f.read_only {
            assert!(!body.to_string().contains(ADDRESS), "{body}");
        }
    }

    let result = list_alert_rules(&f, f.read_only).await;
    let delivery = &result["rules"][0]["lastDelivery"];
    assert_eq!(delivery["delivered"], json!(false), "{result}");
    assert_eq!(delivery["reason"], json!(REASON), "{result}");
    assert!(delivery["at"].is_string(), "{result}");
}

/// `SRC-40` D6: a rule nothing was ever delivered to carries no
/// `lastDelivery` key, not an empty one (principle 2: no invented outcome).
#[tokio::test]
async fn a_rule_never_delivered_to_has_no_last_delivery_key() {
    let f = fixture().await;
    let (status, body) = get(&f, f.writer, "/api/alerts").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body["rules"][0].get("lastDelivery").is_none(), "{body}");
}

/// `SRC-40` D10 (`TC-1696`): a caller without `alert:write` cannot delete a
/// rule — the server refuses it, not only the console that hides the control
/// (principle 3). A pin: it passes on the code before the QA round; it fails
/// if the `may_manage` check in `routes::alerts::delete` is removed. A delete
/// is the soft-delete `INSERT INTO console.alert_rule … is_deleted = 1`.
///
/// `SRC-40` D5 (review round 1, R2): the rule's latest delivery row goes with
/// it, and only when the delete lands — the refused delete leaves it alone.
/// Fails if the `alert_delivery::delete` call in
/// `routes::alerts::forget_state` is removed: the row would outlive the rule
/// and a later rule reusing the id would show the old "Last sent".
#[tokio::test]
async fn a_caller_without_alert_write_cannot_delete_a_rule() {
    let f = fixture().await;
    let deleted = |log: &[String]| {
        log.iter()
            .filter(|sql| sql.contains("INSERT INTO console.alert_rule"))
            .count()
    };
    record_delivery(&f, "the webhook server answered with an error").await;

    let (status, body) = send(&f, f.read_only, "DELETE", "/api/alerts?id=al_1").await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(
        deleted(&f.log.lock().unwrap()),
        0,
        "the rule was not deleted"
    );
    assert!(
        delivery_row(&f).await.is_some(),
        "a refused delete keeps the rule's last delivery"
    );

    let (status, body) = send(&f, f.writer, "DELETE", "/api/alerts?id=al_1").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(deleted(&f.log.lock().unwrap()), 1, "one soft delete");
    assert!(
        delivery_row(&f).await.is_none(),
        "a deleted rule keeps no last delivery"
    );
}

/// The one delivery row of [`RULE`], or `None` after it was removed.
async fn delivery_row(f: &Fixture) -> Option<lakehouse_store::alert_delivery::LastDelivery> {
    lakehouse_store::alert_delivery::last_for(&f.app.pool, &[RULE.to_owned()])
        .await
        .expect("the delivery rows")
        .remove(RULE)
}

/// Record [`RULE`]'s latest delivery as not delivered, with `reason`.
async fn record_delivery(f: &Fixture, reason: &str) {
    lakehouse_store::alert_delivery::record(
        &f.app.pool,
        RULE,
        DeliveryRecord::NotDelivered(reason),
        OffsetDateTime::now_utc(),
    )
    .await
    .expect("a delivery row");
}

/// A pin: the incidents list is built from names and ids and has no target
/// field, for either caller (`D18`).
#[tokio::test]
async fn the_incident_list_never_carries_a_rule_target() {
    let f = fixture().await;
    insert_from_fired_rule(
        &f.app.pool,
        &FiredRule {
            rule_id: RULE,
            title: "Orders",
            severity: Some("high"),
            source: "Threshold alert",
            affected: "mart_orders",
            detail: "orders went above 10",
        },
        OffsetDateTime::now_utc(),
    )
    .await
    .expect("an instance");

    for who in [f.read_only, f.writer] {
        let (status, body) = get(&f, who, "/api/overview/alerts").await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body.as_array().map(Vec::len), Some(1), "{body}");
        assert!(body[0].get("target").is_none(), "{body}");
        assert!(!body.to_string().contains(ADDRESS), "{body}");
    }
}

/// A pin: the status route holds one boolean and nothing else (`D18`).
#[tokio::test]
async fn the_alert_status_holds_one_boolean_and_nothing_else() {
    let f = fixture().await;
    for who in [f.read_only, f.writer] {
        let (status, body) = get(&f, who, "/api/alerts/status").await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body, json!({ "runEventsConfigured": false }));
    }
}
