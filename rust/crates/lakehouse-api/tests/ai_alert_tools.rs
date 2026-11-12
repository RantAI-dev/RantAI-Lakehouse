//! The alert tools through `POST /api/ai/tool` (`BI-37` T2): a chart alert is a
//! chart alert, it goes through the console's one save path, "alert me" fills
//! the caller's own address, and the console's refusals are the assistant's.
//!
//! The `ClickHouse` here is a recording fake: one board, one KPI chart and the
//! stored rule the call acts on. Every statement the tools make is kept, so a
//! test can read what was actually saved rather than what the reply says.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::body::{Body, to_bytes};
use axum::http::Request;
use common::{TestApp, create_assistant_principal, session_cookie_for_user, spin_up_with_env};
use lakehouse_store::dashboard_access::replace_item;
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, Request as MockRequest, Respond, ResponseTemplate};

const BOARD: &str = "b_pub";
const RULE: &str = "al_1";
const CHART: &str = "c_kpi";
/// The console's own sentence for another address without `alert:write`
/// (`routes::chart_alerts::OWN_EMAIL_ONLY`).
const OWN_EMAIL_ONLY: &str =
    "Without the alert permission an alert can only be sent to your own email address.";
/// The console's own sentence for a restricted dashboard with another target
/// (`routes::chart_alerts::RESTRICTED_SAVE`).
const RESTRICTED_SAVE: &str = "This dashboard is restricted to the people listed, so its alert can only be sent to your own email address.";

/// What the fake holds: the row a rule read answers, what an insert makes it
/// answer instead, and whether the chart is one the caller can see.
#[derive(Default)]
struct Held {
    rule: Option<Value>,
    after_insert: Option<Value>,
    chart_visible: bool,
}

#[derive(Clone)]
struct FakeCh {
    held: Arc<Mutex<Held>>,
    log: Arc<Mutex<Vec<String>>>,
}

fn rule_row(chart: &str, owner: &str, target: &str) -> Value {
    json!({
        "id": RULE, "name": "Visitors", "type": "alert", "mart": "", "measure": "",
        "agg": "sum", "op": ">", "threshold": "100", "board": "", "channel": "email",
        "target": target, "enabled": "1", "created_at": "2026-10-26 00:00:00",
        "severity": "", "pipeline": "", "metric": "", "connector": "", "asset": "",
        "chart": chart, "series": "", "condition": "below", "owner_id": owner,
        "filters_json": "",
    })
}

fn chart_row() -> Value {
    let spec = json!({
        "id": CHART, "title": "Visitors", "kind": "kpi", "mart": "mart_x",
        "sql": "SELECT sum(visitors) AS visitors FROM serving.mart_x", "x": "", "y": "visitors"
    });
    let def = json!({
        "title": "Visitors", "mart": "mart_x", "kind": "kpi", "measures": ["visitors"],
        "aggregate": "sum"
    });
    json!({
        "id": CHART, "board": BOARD, "created_by": "ui", "created_at": "2026-01-01 00:00:00",
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
        let rows = |data: Vec<Value>| {
            let n = data.len();
            ResponseTemplate::new(200).set_body_json(json!({ "meta": [], "data": data, "rows": n }))
        };
        if sql.contains("INSERT INTO console.alert_rule") {
            let mut held = self.held.lock().unwrap();
            if let Some(next) = held.after_insert.take() {
                held.rule = Some(next);
            }
            return ResponseTemplate::new(200).set_body_string("Ok.\n");
        }
        if sql.contains("FROM console.alert_rule") {
            let rule = self.held.lock().unwrap().rule.clone();
            return rows(rule.into_iter().collect());
        }
        if sql.contains("FROM console.bi_chart") {
            let visible = self.held.lock().unwrap().chart_visible;
            return rows(if visible { vec![chart_row()] } else { vec![] });
        }
        if sql.contains("FROM console.bi_board") {
            return rows(vec![board_row()]);
        }
        if sql.contains("FROM console.bi_folder") {
            return rows(vec![]);
        }
        if sql.contains("system.columns") {
            return rows(
                [("region", "String"), ("visitors", "Float64")]
                    .iter()
                    .map(|(n, t)| json!({"table": "mart_x", "name": n, "type": t}))
                    .collect(),
            );
        }
        ResponseTemplate::new(200).set_body_string("Ok.\n")
    }
}

struct Fixture {
    app: TestApp,
    fake: FakeCh,
    /// The email `create_assistant_principal` gave the person.
    email: String,
    _ch: MockServer,
}

/// A person with `permissions` besides `assistant:use` (every test here calls
/// `POST /api/ai/tool`), and a chart the caller can see unless `chart_visible`
/// says otherwise.
async fn fixture(permissions: &str, chart_visible: bool) -> (Fixture, Uuid) {
    let fake = FakeCh {
        held: Arc::new(Mutex::new(Held {
            rule: None,
            after_insert: None,
            chart_visible,
        })),
        log: Arc::new(Mutex::new(Vec::new())),
    };
    let ch = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(fake.clone())
        .mount(&ch)
        .await;
    // `SEC-50` slice 2 M11: the save now gates a mart or metric rule's
    // statement as the saving person, and the seeded migration creates
    // several tenants, so the shared catalog belongs to none of them unless
    // the deployment names it. These tests are about the console's own
    // sentences, not the gate: the deployment names the catalog's tenant and
    // the person is in it, so the gate asks the same question the save
    // already answered.
    let env: HashMap<String, String> = HashMap::from([
        ("CH_URL".to_owned(), ch.uri()),
        (
            "CATALOG_TENANT_ID".to_owned(),
            common::CATALOG_TENANT.to_owned(),
        ),
    ]);
    let app = spin_up_with_env(&env).await;
    let user = create_assistant_principal(&app.pool, permissions).await;
    common::in_the_catalog_tenant(&app.pool, user).await;
    let fixture = Fixture {
        app,
        fake,
        email: format!("single-perm-{user}@test.invalid"),
        _ch: ch,
    };
    (fixture, user)
}

async fn call_tool(f: &Fixture, who: Uuid, tool: &str, args: Value, mode: &str) -> Value {
    let cookie = session_cookie_for_user(&f.app.pool, who).await;
    let resp = f
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
                    json!({ "tool": tool, "args": args, "mode": mode }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

/// The statements that saved a rule.
fn saved(f: &Fixture) -> Vec<String> {
    f.fake
        .log
        .lock()
        .unwrap()
        .iter()
        .filter(|sql| sql.contains("INSERT INTO console.alert_rule"))
        .cloned()
        .collect()
}

/// D1/D2/D3: a person without `alert:write` saves a chart alert to their own
/// address through the tool, and the rule is theirs.
#[tokio::test]
async fn a_person_without_alert_write_saves_a_chart_alert_to_their_own_address() {
    let (f, person) = fixture("dashboard:read", true).await;
    let me = person.to_string();
    // `alert me when this chart drops below 1,000`: no target, no channel.
    {
        let mut held = f.fake.held.lock().unwrap();
        held.after_insert = Some(rule_row(CHART, &me, &f.email));
    }
    let body = call_tool(
        &f,
        person,
        "create_alert_rule",
        json!({
            "name": "Visitors below 1,000", "type": "alert",
            "chart": CHART, "condition": "below", "threshold": 1000,
            "confirmed": true,
        }),
        "build",
    )
    .await;
    assert_eq!(body["outcome"], "executed", "{body}");
    assert_eq!(body["result"]["ok"], json!(true), "{body}");
    assert_eq!(
        body["result"]["rule"]["ownerId"].as_str(),
        Some(me.as_str()),
        "the rule is saved as its owner: {body}"
    );
    let inserts = saved(&f);
    assert_eq!(inserts.len(), 1, "one save: {inserts:?}");
    let sql = &inserts[0];
    assert!(sql.contains(&format!("'{me}'")), "no owner in {sql}");
    assert!(sql.contains(&f.email), "no own address in {sql}");
    assert!(sql.contains("'email'"), "no email channel in {sql}");
    assert!(sql.contains("'below'"), "no condition in {sql}");
    assert!(
        sql.contains(&format!("'{CHART}'")),
        "the chart is the watched one: {sql}"
    );
}

/// D2: the console's refusals are the tool's — a mart alert needs
/// `alert:write`, and so does a chart alert to anybody else's address.
#[tokio::test]
async fn a_mart_alert_and_another_address_are_refused_with_the_consoles_sentences() {
    let (f, person) = fixture("dashboard:read", true).await;
    let mart = call_tool(
        &f,
        person,
        "create_alert_rule",
        json!({
            "name": "Kunjungan turun", "type": "alert", "mart": "mart_x",
            "measure": "visitors", "agg": "sum", "op": "<", "threshold": 10,
            "channel": "email", "target": "someone@example.test", "confirmed": true,
        }),
        "build",
    )
    .await;
    let error = mart["result"]["error"].as_str().unwrap_or_default();
    assert!(
        error.contains("permission_denied: alert:write"),
        "a mart alert still needs the permission: {mart}"
    );

    let elsewhere = call_tool(
        &f,
        person,
        "create_alert_rule",
        json!({
            "name": "Visitors below 1,000", "type": "alert", "chart": CHART,
            "condition": "below", "threshold": 1000,
            "channel": "email", "target": "someone@example.test", "confirmed": true,
        }),
        "build",
    )
    .await;
    assert_eq!(
        elsewhere["result"]["error"].as_str(),
        Some(format!("permission_denied: {OWN_EMAIL_ONLY}").as_str()),
        "{elsewhere}"
    );
    assert!(saved(&f).is_empty(), "nothing was saved");
}

/// D1: the chart has to be one the caller can see, or the console's 404.
#[tokio::test]
async fn a_chart_they_cannot_see_gets_the_consoles_not_found() {
    let (f, person) = fixture("dashboard:read", false).await;
    let body = call_tool(
        &f,
        person,
        "create_alert_rule",
        json!({
            "name": "Visitors below 1,000", "type": "alert", "chart": CHART,
            "condition": "below", "threshold": 1000, "confirmed": true,
        }),
        "build",
    )
    .await;
    assert!(
        body["result"]["error"]
            .as_str()
            .is_some_and(|e| e.ends_with("chart not found.")),
        "{body}"
    );
    assert!(saved(&f).is_empty(), "nothing was saved");
}

/// Q-R12 through the tool: a restricted dashboard's alert goes only to the
/// owner's own address, with the console's sentence.
#[tokio::test]
async fn a_restricted_dashboard_refuses_another_target() {
    let (f, person) = fixture("dashboard:read, alert:write", true).await;
    replace_item(&f.app.pool, "board", BOARD, &[], true, None)
        .await
        .unwrap();
    let body = call_tool(
        &f,
        person,
        "create_alert_rule",
        json!({
            "name": "Visitors below 1,000", "type": "alert", "chart": CHART,
            "condition": "below", "threshold": 1000,
            "channel": "email", "target": "someone@example.test", "confirmed": true,
        }),
        "build",
    )
    .await;
    assert_eq!(
        body["result"]["error"].as_str(),
        Some(RESTRICTED_SAVE),
        "{body}"
    );
    assert!(saved(&f).is_empty(), "nothing was saved");
}

/// T1's state reset through the tool: a rule that now watches another chart
/// starts over. `SRC-40` D5 (review round 1, R2) is the same moment for the
/// rule's latest delivery: the assistant's save path goes through
/// `routes::alerts::forget_state`, so its row goes too. Fails if the
/// `alert_delivery::delete` call there is removed. (`delete_alert_rule` cannot
/// cover this here: it is `WriteHigh` and returns a pending approval, never the
/// delete itself — `tests/tier1_writehigh_approval.rs`.)
#[tokio::test]
async fn update_alert_rule_changing_the_chart_resets_the_state() {
    let (f, person) = fixture("dashboard:read, alert:write", true).await;
    let me = person.to_string();
    {
        let mut held = f.fake.held.lock().unwrap();
        held.rule = Some(rule_row("c_old", &me, &f.email));
        held.after_insert = Some(rule_row(CHART, &me, &f.email));
    }
    sqlx::query(
        "INSERT INTO alert_rule_state (rule_id, state, last_value) VALUES ($1, 'firing', 900)",
    )
    .bind(RULE)
    .execute(&f.app.pool)
    .await
    .unwrap();
    lakehouse_store::alert_delivery::record(
        &f.app.pool,
        RULE,
        lakehouse_store::overview::DeliveryRecord::NotDelivered(
            "the webhook server answered with an error",
        ),
        time::OffsetDateTime::now_utc(),
    )
    .await
    .unwrap();

    let body = call_tool(
        &f,
        person,
        "update_alert_rule",
        json!({
            "id": RULE, "name": "Visitors", "type": "alert", "chart": CHART,
            "condition": "below", "threshold": 1000,
            "channel": "email", "target": f.email, "confirmed": true,
        }),
        "build",
    )
    .await;
    assert_eq!(body["outcome"], "executed", "{body}");
    let inserts = saved(&f);
    assert_eq!(inserts.len(), 1, "{inserts:?}");
    assert!(inserts[0].contains(&format!("'{CHART}'")), "{inserts:?}");
    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM alert_rule_state WHERE rule_id = $1")
        .bind(RULE)
        .fetch_one(&f.app.pool)
        .await
        .unwrap();
    assert_eq!(left, 0, "a rule watching a new line starts from ok");
    let deliveries: i64 =
        sqlx::query_scalar("SELECT count(*) FROM alert_rule_delivery WHERE rule_id = $1")
            .bind(RULE)
            .fetch_one(&f.app.pool)
            .await
            .unwrap();
    assert_eq!(
        deliveries, 0,
        "a rule watching a new line starts with no last delivery"
    );
}

/// D2/D5: Ask mode still refuses both tools, and an unconfirmed call returns a
/// Confirm card that names the chart, the condition and the threshold.
#[tokio::test]
async fn ask_mode_refuses_and_an_unconfirmed_call_names_the_chart_and_condition() {
    let perms = "dashboard:read, alert:write";
    let (f, person) = fixture(perms, true).await;
    let args = json!({
        "name": "Visitors below 1,000", "type": "alert", "chart": CHART,
        "condition": "below", "threshold": 1000,
    });
    let asked = call_tool(&f, person, "create_alert_rule", args.clone(), "ask").await;
    assert_eq!(asked["outcome"], "refused", "{asked}");
    // The same refusal for an update, so Ask mode covers both save tools.
    let update = call_tool(
        &f,
        person,
        "update_alert_rule",
        json!({ "id": RULE, "name": "x", "type": "alert" }),
        "ask",
    )
    .await;
    assert_eq!(update["outcome"], "refused", "{update}");

    let card = call_tool(&f, person, "create_alert_rule", args.clone(), "build").await;
    assert_eq!(card["outcome"], "needs_confirmation", "{card}");
    let summary = card["result"]["summary"].as_str().unwrap_or_default();
    assert!(
        summary.contains(CHART),
        "the card names the chart: {summary}"
    );
    assert!(
        summary.contains("di bawah") && summary.contains("1000"),
        "the card names the condition and the threshold: {summary}"
    );
    assert!(saved(&f).is_empty(), "nothing was saved before the Confirm");
}

/// D4 (verified, not changed): `run_alert_rule` on a chart rule surfaces the
/// runner's own reason as it is, and adds no second sentence.
///
/// `BI-40`: a chart rule is evaluated now. `run_alert_rule` goes through
/// `routes::alerts::run_one`, which reads a chart rule as its owner, so the
/// old skip sentence (`CHART_SKIPPED`: "chart alerts are evaluated by
/// /api/alerts/run") is gone and the reason is the one the reading gives.
/// This test's fake chart has no usable result, so that reason is
/// `lakehouse_alerts::chart_number`'s own sentence.
#[tokio::test]
async fn running_a_chart_rule_surfaces_the_runners_own_reason() {
    /// `lakehouse_alerts::chart_number`'s own sentence. It is an inline
    /// literal in that crate, not a `pub` const, so it is copied here: a
    /// change there must fail this test rather than be silently accepted.
    const SERIES_NOT_IN_CHART: &str = "the series is not in the chart's result.";
    let (f, person) = fixture("dashboard:read, alert:write", true).await;
    let me = person.to_string();
    f.fake.held.lock().unwrap().rule = Some(rule_row(CHART, &me, &f.email));
    let body = call_tool(
        &f,
        person,
        "run_alert_rule",
        json!({ "id": RULE, "confirmed": true }),
        "build",
    )
    .await;
    assert_eq!(body["outcome"], "executed", "{body}");
    let item = &body["result"]["results"][0];
    let skipped = item["skipped"].as_str().unwrap_or_default();
    // Equal, not `starts_with`: the whole sentence is the runner's, and the
    // tool appends nothing to it (D4).
    assert_eq!(
        skipped, SERIES_NOT_IN_CHART,
        "the runner's own reason: {body}"
    );
    // No sentence of the assistant's own: the answer is the runner's `ran`
    // count and its results, and each result carries only the runner's fields.
    let keys = |value: &Value| -> Vec<String> {
        value
            .as_object()
            .map(|o| o.keys().cloned().collect())
            .unwrap_or_default()
    };
    let mut top = keys(&body);
    top.sort();
    assert_eq!(
        top,
        ["outcome", "result", "tool"],
        "the tool adds no text of its own to the answer: {body}"
    );
    let mut result = keys(&body["result"]);
    result.sort();
    assert_eq!(
        result,
        ["ran", "results"],
        "the tool adds no note beside the runner's results: {body}"
    );
    let mut fields = keys(item);
    fields.sort();
    assert_eq!(
        fields,
        ["fired", "id", "name", "skipped", "type"],
        "the result is the runner's, with no extra field: {body}"
    );
    assert!(
        !body.to_string().contains("chart alerts are evaluated by"),
        "`BI-40` removed the old skip sentence; it is not repeated here: {body}"
    );
}
