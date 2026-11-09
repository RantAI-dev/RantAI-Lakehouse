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
/// `SEC-50` slice 2 M11: another tenant of the same deployment, whose
/// members may not read the shared catalog.
const OTHER_TENANT: &str = "11111111-1111-4111-8111-000000000002";

#[derive(Clone)]
struct FakeCh {
    owner: String,
    target: Arc<Mutex<String>>,
    value: Arc<Mutex<f64>>,
    chart_exists: Arc<Mutex<bool>>,
    /// The stored rule is a mart rule (no chart) instead of a chart rule.
    plain: Arc<Mutex<bool>>,
    /// How many charts `BOARD` holds (`BI-39` T3: a chart alert reads one
    /// tile, whatever the board's size). Default 1.
    charts: Arc<Mutex<usize>>,
    /// The stored rule exists at all (false: an unknown id).
    rule_exists: Arc<Mutex<bool>>,
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

/// One chart of `BOARD` reading `measure` of the mart: `BI-39` D7 counts the
/// tile statements, so a board of several charts must cost one statement each
/// (the cache computes one statement per key, so the charts differ).
fn chart_row_for(id: &str, title: &str, measure: &str) -> Value {
    let spec = json!({
        "id": id, "title": title, "kind": "kpi", "mart": "mart_x",
        "sql": format!("SELECT sum({measure}) AS {measure} FROM serving.mart_x"),
        "x": "", "y": measure
    });
    let def = json!({
        "title": title, "mart": "mart_x", "kind": "kpi", "measures": [measure],
        "aggregate": "sum"
    });
    json!({
        "id": id, "board": BOARD, "created_by": "ui", "created_at": "2026-01-01 00:00:00",
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

/// Whether the viewer clause the product put on a `bi_chart` / `bi_board`
/// read leaves `BOARD` out. `ClickHouse` applies that clause; this stand-in has
/// to apply it too, or a test that removes the owner's access would read the
/// board as if nothing had changed (BI-19 C, PR #250 CI: the test was wrong,
/// the product filters in SQL).
fn clause_hides_board(sql: &str) -> bool {
    if sql.contains("AND 1 = 0") {
        return true;
    }
    let quoted = format!("'{BOARD}'");
    let mut rest = sql;
    let mut consumed = 0;
    while let Some(at) = rest.find(" IN (") {
        let before = &sql[..consumed + at];
        let after = &rest[at + 5..];
        let Some(end) = after.find(')') else { break };
        let list = &after[..end];
        if !list.starts_with("SELECT") {
            let negated = before.ends_with(" NOT");
            if list.contains(&quoted) == negated {
                return true;
            }
        }
        consumed += at + 5 + end;
        rest = &sql[consumed..];
    }
    false
}

/// The `owner_id` a saved rule's `INSERT` wrote: the rule's last value, a
/// single-quoted literal (`SqlLiteral` doubles an inner quote; owner ids never
/// carry one).
fn inserted_owner(sql: &str) -> Option<String> {
    // `save_rule` writes `VALUES ('id','name',...)` with no space after a
    // comma, and `owner_id` is the last column.
    let values = sql.split(") VALUES (").nth(1)?;
    let last = values.rsplit("','").next()?;
    Some(last.strip_suffix("')")?.to_owned())
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
            if !*self.rule_exists.lock().unwrap() {
                return rows(json!([]), vec![]);
            }
            // `ReplacingMergeTree` + `FINAL` sees a just-inserted row on the
            // same connection immediately, and `save_rule` re-reads the rule it
            // wrote; the owner it reads is the one just inserted, not the
            // fixture's (`BI-39` D2: a save sets the owner, so the read after it
            // must show that).
            let owner = self.owner.clone();
            let mut row = rule_row(&owner, &self.target.lock().unwrap());
            if let Some(saved) = self
                .log
                .lock()
                .unwrap()
                .iter()
                .rev()
                .find_map(|previous| inserted_owner(previous))
            {
                row["owner_id"] = json!(saved);
            }
            if *self.plain.lock().unwrap() {
                row["chart"] = json!("");
                row["condition"] = json!("");
                row["mart"] = json!("mart_x");
                row["measure"] = json!("visitors");
            }
            return rows(json!([]), vec![row]);
        }
        if sql.contains("FROM console.bi_chart") {
            let present = *self.chart_exists.lock().unwrap() && !clause_hides_board(&sql);
            let count = *self.charts.lock().unwrap();
            let charts: Vec<Value> = [
                ("c_kpi", "Visitors", "visitors"),
                ("c_two", "Tile c_two", "visitors_two"),
                ("c_three", "Tile c_three", "visitors_three"),
            ]
            .iter()
            .take(count)
            .map(|(id, title, measure)| chart_row_for(id, title, measure))
            .collect();
            return rows(json!([]), if present { charts } else { vec![] });
        }
        if sql.contains("FROM console.bi_board") {
            let hidden = clause_hides_board(&sql);
            return rows(json!([]), if hidden { vec![] } else { vec![board_row()] });
        }
        if sql.contains("system.columns") {
            return rows(
                json!([]),
                [
                    ("region", "String"),
                    ("visitors", "Float64"),
                    ("visitors_two", "Float64"),
                    ("visitors_three", "Float64"),
                ]
                .iter()
                .map(|(n, t)| json!({"table": "mart_x", "name": n, "type": t}))
                .collect(),
            );
        }
        if sql.contains("FROM serving.mart_x") {
            let value = *self.value.lock().unwrap();
            return rows(
                json!([
                    {"name": "visitors", "type": "Float64"},
                    {"name": "visitors_two", "type": "Float64"},
                    {"name": "visitors_three", "type": "Float64"},
                ]),
                vec![json!({ "visitors": value, "visitors_two": value, "visitors_three": value })],
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
    fixture_in(target, common::CATALOG_TENANT).await
}

/// `SEC-50` slice 2 M11: the same fixture, with the rule's owner in
/// `tenant` — the catalog's own tenant, or another one.
async fn fixture_in(target: &str, tenant: &str) -> Fixture {
    let owner = Uuid::new_v4();
    let fake = FakeCh {
        owner: owner.to_string(),
        target: Arc::new(Mutex::new(target.to_owned())),
        value: Arc::new(Mutex::new(150.0)),
        chart_exists: Arc::new(Mutex::new(true)),
        plain: Arc::new(Mutex::new(false)),
        charts: Arc::new(Mutex::new(1)),
        rule_exists: Arc::new(Mutex::new(true)),
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
        // `SEC-50` slice 2 M11: the shared catalog belongs to a tenant, and
        // the rule's owner is in it.
        (
            "CATALOG_TENANT_ID".to_owned(),
            common::CATALOG_TENANT.to_owned(),
        ),
    ]);
    let app = spin_up_with_env(&env).await;
    let role = Uuid::new_v4();
    sqlx::query("INSERT INTO role (id, name, permissions, description) VALUES ($1, 'Chart Owner', 'dashboard:read', 'test')")
        .bind(role)
        .execute(&app.pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO app_user (id, name, username, email, status) VALUES ($1, 'Owner', 'owner', 'owner@test.invalid', 'active')")
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
    // `SEC-50` slice 2 M11: the owner is in the tenant the fixture names.
    sqlx::query("INSERT INTO app_user_tenant (user_id, tenant_id) VALUES ($1, $2)")
        .bind(owner)
        .bind(tenant.parse::<Uuid>().expect("a seeded tenant id"))
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
    run_as(f, f.owner).await
}

/// The same run, triggered by `who` (the rule's owner need not be the caller:
/// the scheduled tick is nobody's session).
async fn run_as(f: &Fixture, who: Uuid) -> Value {
    let cookie = session_cookie_for_user(&f.app.pool, who).await;
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
    sqlx::query("INSERT INTO app_user (id, name, username, email, status) VALUES ($1, 'Other', 'other', 'other@test.invalid', 'active')")
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

// ---- BI-19 C review: the save guard (C-R1, C-R2), reached through PUT/POST ----

fn chart_rule_body(target: &str) -> Value {
    json!({
        "id": RULE, "name": "Visitors in Bali", "type": "alert", "chart": "c_kpi",
        "condition": "above", "threshold": 100.0, "channel": "email", "target": target,
    })
}

async fn send_alert(f: &Fixture, who: Uuid, method: &str, body: Value) -> (u16, Value) {
    let cookie = session_cookie_for_user(&f.app.pool, who).await;
    let resp = f
        .app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri("/api/alerts")
                .header("cookie", cookie)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status().as_u16();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

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

/// C-R1: a person without `alert:write` cannot replace a rule that is not their
/// own chart rule, here a mart rule they can name by id.
#[tokio::test]
async fn a_person_without_alert_write_cannot_take_over_a_mart_rule() {
    let f = fixture("owner@test.invalid").await;
    *f.fake.plain.lock().unwrap() = true;
    let (status, _) = send_alert(&f, f.owner, "PUT", chart_rule_body("owner@test.invalid")).await;
    assert_eq!(status, 403);
    assert!(saved(&f).is_empty(), "the rule must be unchanged");
}

/// C-R1: and an id that does not exist is 403 too, not a way to create a rule
/// with an id of their choosing.
#[tokio::test]
async fn a_person_without_alert_write_cannot_put_an_unknown_id() {
    let f = fixture("owner@test.invalid").await;
    *f.fake.rule_exists.lock().unwrap() = false;
    let (status, _) = send_alert(&f, f.owner, "PUT", chart_rule_body("owner@test.invalid")).await;
    assert_eq!(status, 403);
    assert!(saved(&f).is_empty(), "no rule was written");
}

/// The positive case: the analyst saves their own chart rule, to their own
/// address, and owns it.
#[tokio::test]
async fn an_analyst_saves_their_own_chart_alert_to_their_own_address() {
    let f = fixture("owner@test.invalid").await;
    let (status, body) =
        send_alert(&f, f.owner, "PUT", chart_rule_body("owner@test.invalid")).await;
    assert_eq!(status, 200, "{body}");
    let inserts = saved(&f);
    assert_eq!(inserts.len(), 1, "{inserts:?}");
    assert!(inserts[0].contains(&f.owner.to_string()), "{}", inserts[0]);
    // Another address is the alert permission's to send to.
    let (status, _) = send_alert(&f, f.owner, "PUT", chart_rule_body("someone@example.com")).await;
    assert_eq!(status, 403);
}

/// C-R2: a writer who saves another person's chart rule owns it afterwards, so
/// the next read uses the writer's access and not the first owner's.
#[tokio::test]
async fn a_writer_who_edits_anothers_chart_rule_becomes_its_owner() {
    let f = fixture("owner@test.invalid").await;
    let writer =
        common::create_principal_with_permissions(&f.app.pool, "dashboard:read, alert:write").await;
    let (status, body) =
        send_alert(&f, writer, "PUT", chart_rule_body("someone@example.com")).await;
    assert_eq!(status, 200, "{body}");
    let inserts = saved(&f);
    assert_eq!(inserts.len(), 1, "{inserts:?}");
    assert!(inserts[0].contains(&writer.to_string()), "{}", inserts[0]);
    assert!(!inserts[0].contains(&f.owner.to_string()), "{}", inserts[0]);
}

/// SF-3: an edit of the name alone keeps the state (the unit test
/// `editing_what_a_rule_watches_differs_and_editing_its_name_does_not` covers
/// which edits drop it; the fake here serves one stored row for both reads).
#[tokio::test]
async fn renaming_a_rule_keeps_its_state() {
    let f = fixture("owner@test.invalid").await;
    silence(&f.app.pool).await;
    run(&f).await;
    assert_eq!(state_of(&f.app.pool).await.unwrap().0, "firing");
    let mut renamed = chart_rule_body("owner@test.invalid");
    renamed["name"] = json!("Other name");
    renamed["threshold"] = json!(100.0);
    // The fake serves the same stored row for the "after" read, so the saved
    // rule differs from the stored one in the name only.
    let (status, _) = send_alert(&f, f.owner, "PUT", renamed).await;
    assert_eq!(status, 200);
    assert_eq!(state_of(&f.app.pool).await.unwrap().0, "firing");
}

/// SF-4/SF-5 and C4: a dashboard that is restricted to somebody else stops the
/// owner's alert with the reason (the 403/404 to `stopped` mapping).
#[tokio::test]
async fn an_owner_who_lost_the_dashboard_stops_the_alert() {
    let f = fixture("owner@test.invalid").await;
    let other = Uuid::new_v4();
    sqlx::query("INSERT INTO app_user (id, name, username, email, status) VALUES ($1, 'Other', 'o2', 'o2@test.invalid', 'active')")
        .bind(other)
        .execute(&f.app.pool)
        .await
        .unwrap();
    replace_item(
        &f.app.pool,
        "board",
        BOARD,
        &[NewRule {
            user_id: Some(other),
            role_id: None,
            level: "view".to_owned(),
        }],
        true,
        None,
    )
    .await
    .unwrap();
    let r = run(&f).await;
    assert!(r["skipped"].as_str().is_some(), "{r}");
    let (state, reason) = state_of(&f.app.pool).await.unwrap();
    assert_eq!(state, "stopped");
    assert!(reason.unwrap().contains("can no longer see"));
}

/// `SEC-50` slice 2 M4: a chart alert whose owner is of another tenant
/// reads nothing: the tile is refused, the alert fails, and nothing is
/// sent.
#[tokio::test]
async fn a_chart_alert_of_another_tenant_fails_and_sends_nothing() {
    let f = fixture_in("owner@test.invalid", OTHER_TENANT).await;
    let r = run(&f).await;
    assert_eq!(
        r["skipped"],
        json!("The chart could not be read."),
        "the tile is refused, so the alert cannot be read: {r}"
    );
    assert_eq!(r["fired"], json!(false), "{r}");
    assert!(
        !f.fake
            .log
            .lock()
            .unwrap()
            .iter()
            .any(|sql| sql.contains("FROM serving.mart_x")),
        "nothing is read when the owner may not read the mart"
    );
}

// ---- BI-39: every threshold rule is owned by whoever saved it (D1, D2, D5) ----

/// A mart rule body the save routes accept.
fn mart_rule_body() -> Value {
    json!({
        "name": "Visitors over the line", "type": "alert", "mart": "mart_x",
        "measure": "visitors", "agg": "sum", "op": ">", "threshold": 100.0,
        "channel": "email", "target": "someone@example.com",
    })
}

/// A role named `name` holding `permissions`, and an active person in it.
async fn writer(pool: &PgPool, name: &str, permissions: &str) -> Uuid {
    let role = Uuid::new_v4();
    // Role names are unique, and the seeded deployment already has "Data
    // Engineer"; the suffix keeps the role this test means distinct from it.
    let name = format!("{name} {}", Uuid::new_v4());
    sqlx::query(
        "INSERT INTO role (id, name, permissions, description) VALUES ($1, $2, $3, 'test')",
    )
    .bind(role)
    .bind(&name)
    .bind(permissions)
    .execute(pool)
    .await
    .unwrap();
    let user = Uuid::new_v4();
    sqlx::query("INSERT INTO app_user (id, name, username, email, status) VALUES ($1, $2, $3, $4, 'active')")
        .bind(user)
        .bind(format!("{name} person"))
        .bind(user.to_string())
        .bind(format!("{user}@test.invalid"))
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO app_user_role (user_id, role_id) VALUES ($1, $2)")
        .bind(user)
        .bind(role)
        .execute(pool)
        .await
        .unwrap();
    // `SEC-50` slice 2 M11: a writer of the catalog's own tenant, so the save's
    // tenant gate admits the `serving.*` statement.
    common::in_the_catalog_tenant(pool, user).await;
    user
}

/// `BI-39` D2: a mart rule is owned by whoever saved it — it is not left
/// without an owner as it was before.
#[tokio::test]
async fn a_mart_rule_saved_by_a_writer_stores_that_writer_as_its_owner() {
    let f = fixture("owner@test.invalid").await;
    let engineer = writer(&f.app.pool, "Data Engineer", "dashboard:read, alert:write").await;

    let (status, body) = send_alert(&f, engineer, "POST", mart_rule_body()).await;

    assert_eq!(status, 200, "{body}");
    let inserts = saved(&f);
    assert_eq!(inserts.len(), 1, "{inserts:?}");
    assert!(
        inserts[0].contains(&engineer.to_string()),
        "the saving person's id is the stored owner: {}",
        inserts[0]
    );
}

/// `BI-39` D2: a writer who saves somebody else's mart rule owns it from then
/// on, and `watches_differ`'s `owner_id` starts its state over — the number is
/// read as another person now.
#[tokio::test]
async fn re_saving_a_mart_rule_by_another_writer_makes_them_its_owner_and_resets_its_state() {
    let f = fixture("owner@test.invalid").await;
    *f.fake.plain.lock().unwrap() = true;
    sqlx::query("INSERT INTO alert_rule_state (rule_id, state) VALUES ($1, 'firing')")
        .bind(RULE)
        .execute(&f.app.pool)
        .await
        .unwrap();
    let engineer = writer(&f.app.pool, "Data Engineer", "dashboard:read, alert:write").await;

    let mut body = mart_rule_body();
    body["id"] = json!(RULE);
    let (status, body) = send_alert(&f, engineer, "PUT", body).await;

    assert_eq!(status, 200, "{body}");
    let inserts = saved(&f);
    assert_eq!(inserts.len(), 1, "{inserts:?}");
    assert!(
        inserts[0].contains(&engineer.to_string()),
        "the editing person's id is the stored owner: {}",
        inserts[0]
    );
    let state: Option<(String,)> =
        sqlx::query_as("SELECT state FROM alert_rule_state WHERE rule_id = $1")
            .bind(RULE)
            .fetch_optional(&f.app.pool)
            .await
            .unwrap();
    assert!(state.is_none(), "a change of owner starts the state over");
}

/// `BI-39` D5: a mart rule whose owner is disabled stops with the shared
/// sentence, reads nothing and sends nothing.
#[tokio::test]
async fn a_mart_rules_disabled_owner_stops_it_with_the_sentence_and_sends_nothing() {
    let f = fixture("owner@test.invalid").await;
    *f.fake.plain.lock().unwrap() = true;
    let owner = f.owner;
    let trigger = Uuid::new_v4();
    sqlx::query("INSERT INTO app_user (id, name, username, email, status) VALUES ($1, 'Trigger', 'trigger', 'trigger@test.invalid', 'active')")
        .bind(trigger)
        .execute(&f.app.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE app_user SET status = 'inactive' WHERE id = $1")
        .bind(owner)
        .execute(&f.app.pool)
        .await
        .unwrap();

    let r = run_as(&f, trigger).await;

    assert_eq!(
        r["skipped"],
        json!("The person who set this alert is disabled or no longer exists."),
        "{r}"
    );
    assert_eq!(r["fired"], json!(false), "{r}");
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
        "nothing is read for a stopped rule"
    );
}

// ---- BI-39 T3: one tile (D7) and the chart's name in the list (D8) ----

/// A `GET` of the console's JSON, as `who`.
async fn get_json(f: &Fixture, who: Uuid, uri: &str) -> (u16, Value) {
    let cookie = session_cookie_for_user(&f.app.pool, who).await;
    let resp = f
        .app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .uri(uri)
                .header("cookie", cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status().as_u16();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

/// How many times a tile was computed in `log`: a tile's own statement, not the
/// column or table lookup beside it (those name `serving.mart_x` in a list).
fn tile_statements(log: &[String]) -> usize {
    log.iter()
        .filter(|sql| sql.contains("FROM serving.mart_x"))
        .count()
}

/// `BI-39` D7: on a board of three charts a chart alert's check computes one
/// tile, where the board itself computes three — and the number is the same as
/// before.
#[tokio::test]
async fn a_chart_alert_on_a_board_of_three_charts_reads_exactly_one_tile() {
    let f = fixture("owner@test.invalid").await;
    *f.fake.charts.lock().unwrap() = 3;
    silence(&f.app.pool).await;

    // The board read the browser makes: three tiles. `fresh=1` so none of them
    // is answered from the in-process cache (the three share one statement).
    let before = f.fake.log.lock().unwrap().len();
    let (status, body) = get_json(
        &f,
        f.owner,
        &format!("/api/dashboard?board={BOARD}&fresh=1"),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        body["charts"].as_array().unwrap().len(),
        3,
        "the board has three charts: {body}"
    );
    let board_log = f.fake.log.lock().unwrap()[before..].to_vec();
    assert_eq!(
        tile_statements(&board_log),
        3,
        "one statement per chart of the board"
    );

    // The rule's check: one.
    let before = f.fake.log.lock().unwrap().len();
    let r = run(&f).await;
    let rule_log = f.fake.log.lock().unwrap()[before..].to_vec();
    let rule_tiles = tile_statements(&rule_log);
    assert_eq!(
        rule_tiles, 1,
        "a chart alert reads the tile it watches: {rule_log:#?} / {r}"
    );
    assert_eq!(r["skipped"], Value::Null, "{r}");
    assert_eq!(r["value"].as_f64(), Some(150.0), "{r}");
}

/// `BI-39` D7: `only` is `serde(skip)`, so a request cannot narrow a dashboard
/// read with it — the board still answers every tile.
#[tokio::test]
async fn a_query_string_cannot_narrow_a_dashboard_to_one_chart() {
    let f = fixture("owner@test.invalid").await;
    *f.fake.charts.lock().unwrap() = 3;

    let (status, body) = get_json(
        &f,
        f.owner,
        &format!("/api/dashboard?board={BOARD}&only=c_kpi"),
    )
    .await;

    assert_eq!(status, 200, "{body}");
    let ids: Vec<&str> = body["charts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["c_kpi", "c_two", "c_three"], "{body}");
}

/// `BI-39` D8: the list names a chart rule's chart and dashboard, live.
#[tokio::test]
async fn the_rule_list_names_the_chart_and_the_dashboard() {
    let f = fixture("owner@test.invalid").await;

    let (status, body) = get_json(&f, f.owner, "/api/alerts").await;

    assert_eq!(status, 200, "{body}");
    let rule = body["rules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == json!(RULE))
        .expect("the chart rule is listed");
    assert_eq!(rule["chartTitle"], json!("Visitors"), "{rule}");
    assert_eq!(rule["dashboardId"], json!(BOARD), "{rule}");
    assert_eq!(rule["dashboardTitle"], json!("Public board"), "{rule}");
}

/// `BI-39` D8: a chart rule whose chart the caller cannot see carries none of
/// the three, and the rule itself is still listed.
#[tokio::test]
async fn the_rule_list_names_no_chart_the_caller_cannot_see() {
    let f = fixture("owner@test.invalid").await;
    *f.fake.chart_exists.lock().unwrap() = false;

    let (status, body) = get_json(&f, f.owner, "/api/alerts").await;

    assert_eq!(status, 200, "{body}");
    let rule = body["rules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == json!(RULE))
        .expect("the caller's own rule is still listed");
    assert_eq!(rule["chart"], json!("c_kpi"), "{rule}");
    for field in ["chartTitle", "dashboardId", "dashboardTitle"] {
        assert!(rule.get(field).is_none(), "{field} is absent: {rule}");
    }
}
