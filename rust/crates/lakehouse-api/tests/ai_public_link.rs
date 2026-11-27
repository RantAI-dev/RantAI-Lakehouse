//! The dashboard-sharing tools through `POST /api/ai/tool` and the approvals
//! route (`AI-11` public-link half): `create_public_link` waits
//! for a human's approval and makes no link before it, a live link keeps its
//! address when the expiry moves, `days` outside 1 to 30, a non-manager, a
//! switched-off install and a personal board are refused before any approval
//! row, `revoke_public_link` ends the link on a chat confirm,
//! `list_dashboard_access` never carries the token, and a `move_chart` that
//! widens who can see the chart waits for approval while the same audience
//! stays a confirm.
//!
//! Postgres holds the rules, the approvals and the links (a per-test
//! database); a `wiremock` stand-in plays `ClickHouse`, keeping the board
//! rows and the chart `c1` on `b1`, so the gate's server-side facts have
//! something to read.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use crate::common;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use common::{
    TestApp, create_assistant_principal, create_principal_with_permissions,
    session_cookie_for_user, spin_up_with_env,
};
use lakehouse_store::dashboard_access::{NewRule, replace_item};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, Request as MockRequest, Respond, ResponseTemplate};

/// The routes' sentence for an administrator's switch.
const SWITCHED_OFF: &str = "Public links are switched off by an administrator.";

/// The store's sentence for a dashboard in the personal space.
const PERSONAL_NOT_SHAREABLE: &str =
    "A dashboard in the personal space cannot be shared. Move it to a folder first.";

/// The sharing tools' own bound for `days` (the route allows 1 hour to a
/// year; the assistant may ask for 1 to 30 days).
const DAYS_REFUSED: &str = "The link can be made for 1 to 30 days.";

/// `AI-11`: the refusal when nobody other than the requester can
/// both decide the approval and run it as its approver.
const NO_OTHER_APPROVER: &str = "Only another manager of this dashboard can approve this, and it has none; make the link yourself in Share.";

/// The dashboards the stand-in holds: `(id, embed_enabled)`; the personal
/// board `b_own` gets its owner per test.
const BOARDS: [&str; 3] = ["b1", "b2", "b_own"];

/// A recording `ClickHouse` stand-in that keeps the board rows and serves the
/// chart `c1` on `b1`, so a move's comparison and the level checks read the
/// same boards a later route call would.
#[derive(Clone)]
struct Boards {
    rows: Arc<Mutex<Vec<Value>>>,
}

impl Boards {
    fn new(b1_embed: &str) -> Self {
        let mut rows: Vec<Value> = BOARDS
            .iter()
            .map(|id| {
                json!({
                    "id": id, "name": format!("Board {id}"), "description": "",
                    "created_by": "test", "layout_json": "", "filters_json": "",
                    "public_token": format!("p_{id}"), "embed_enabled": if *id == "b1" { b1_embed } else { "0" },
                    "folder_id": "", "owner_id": "", "created_at": "2026-10-01 00:00:00",
                })
            })
            .collect();
        // `b_own` is never embedded; only its owner is set per test.
        for row in &mut rows {
            if row["id"] == "b_own" {
                row["embed_enabled"] = json!("0");
            }
        }
        Self {
            rows: Arc::new(Mutex::new(rows)),
        }
    }

    /// Make `id` the personal board of `owner`.
    fn set_owner(&self, id: &str, owner: &str) {
        for row in self.rows.lock().unwrap().iter_mut() {
            if row["id"] == id {
                row["owner_id"] = json!(owner);
            }
        }
    }
}

impl Respond for Boards {
    fn respond(&self, request: &MockRequest) -> ResponseTemplate {
        let sql = String::from_utf8_lossy(&request.body).into_owned();
        let rows = |data: Vec<Value>| {
            let n = data.len();
            ResponseTemplate::new(200).set_body_json(json!({ "meta": [], "data": data, "rows": n }))
        };
        if sql.contains("FROM console.bi_chart FINAL WHERE is_deleted = 0") {
            // `list_stored_charts`: the chart the writer may see, on `b1`.
            return rows(vec![json!({
                "id": "c1",
                "spec_json": json!({
                    "spec": {
                        "id": "c1", "title": "T", "kind": "kpi", "mart": "mart_x",
                        "sql": "SELECT province, sum(visitors) FROM serving.mart_x GROUP BY province",
                        "x": "province", "y": "visitors",
                    },
                    "def": {
                        "title": "T", "kind": "kpi", "mart": "mart_x",
                        "dimension": "province", "measures": ["visitors"],
                    },
                    "hasYear": false,
                })
                .to_string(),
                "board": "b1",
                "created_by": "ai",
                "created_at": "2026-01-01 00:00:00",
            })]);
        }
        if sql.contains("SELECT id, folder_id, owner_id FROM console.bi_board") {
            return rows(
                self.rows
                    .lock()
                    .unwrap()
                    .iter()
                    .map(|b| {
                        json!({
                            "id": b["id"], "folder_id": b["folder_id"], "owner_id": b["owner_id"],
                        })
                    })
                    .collect(),
            );
        }
        if sql.contains("FROM console.bi_board") {
            let wanted = sql.find("id='").map(|at| {
                let rest = &sql[at + "id='".len()..];
                rest[..rest.find('\'').unwrap_or(rest.len())].to_owned()
            });
            return rows(
                self.rows
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|b| wanted.as_ref().is_none_or(|w| b["id"] == w.as_str()))
                    .cloned()
                    .collect(),
            );
        }
        // Folders and every other read: nothing.
        rows(Vec::new())
    }
}

async fn app(b1_embed: &str) -> (TestApp, MockServer) {
    let ch = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(Boards::new(b1_embed))
        .mount(&ch)
        .await;
    let env: HashMap<String, String> = HashMap::from([
        ("CH_URL".to_owned(), ch.uri()),
        (
            "CATALOG_TENANT_ID".to_owned(),
            common::CATALOG_TENANT.to_owned(),
        ),
    ]);
    (spin_up_with_env(&env).await, ch)
}

fn rule(user: Uuid, level: &str) -> NewRule {
    NewRule {
        user_id: Some(user),
        role_id: None,
        level: level.to_owned(),
    }
}

/// A manager through a rule alone: `assistant:use` (every call here is
/// `POST /api/ai/tool`) and no dashboard permission; `manage` on the named
/// dashboards through their own rules.
async fn manager(app: &TestApp, boards: &[&str]) -> (Uuid, String) {
    let user = create_assistant_principal(&app.pool, "").await;
    for board in boards {
        replace_item(
            &app.pool,
            "board",
            board,
            &[rule(user, "manage")],
            false,
            None,
        )
        .await
        .unwrap();
    }
    let cookie = session_cookie_for_user(&app.pool, user).await;
    (user, cookie)
}

/// A writer: `dashboard:write` (the move tool's gate permission) plus rules
/// on the named boards, so the widening comparison has rules to read.
async fn writer(app: &TestApp, boards: &[&str]) -> (Uuid, String) {
    let user = create_assistant_principal(&app.pool, "dashboard:write").await;
    for board in boards {
        replace_item(
            &app.pool,
            "board",
            board,
            &[rule(user, "manage")],
            false,
            None,
        )
        .await
        .unwrap();
    }
    let cookie = session_cookie_for_user(&app.pool, user).await;
    (user, cookie)
}

/// An approver: `agent:approve`, and `manage` on the named dashboards through
/// their own rules. `also` keeps the earlier rules, because `replace_item`
/// replaces an item's whole rule list.
async fn approver(app: &TestApp, boards: &[&str], also: &[Uuid]) -> String {
    let user = create_principal_with_permissions(&app.pool, "agent:approve").await;
    for board in boards {
        let mut rules: Vec<NewRule> = also.iter().map(|u| rule(*u, "manage")).collect();
        rules.push(rule(user, "manage"));
        replace_item(&app.pool, "board", board, &rules, false, None)
            .await
            .unwrap();
    }
    session_cookie_for_user(&app.pool, user).await
}

async fn call(app: &TestApp, cookie: &str, tool: &str, args: Value, mode: &str) -> Value {
    let resp = app
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

async fn decide(app: &TestApp, cookie: &str, id: &str) -> (StatusCode, Value) {
    let resp = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/agents/approvals/{id}/decide"))
                .header("content-type", "application/json")
                .header("cookie", cookie)
                .body(Body::from(json!({ "decision": "approved" }).to_string()))
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

async fn approvals_for(app: &TestApp, action: &str) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM approval_item WHERE action = $1")
        .bind(action)
        .fetch_one(&app.pool)
        .await
        .unwrap()
}

async fn approval_reason(app: &TestApp, id: &str) -> String {
    sqlx::query_scalar("SELECT coalesce(reason, '') FROM approval_item WHERE id = $1")
        .bind(id)
        .fetch_one(&app.pool)
        .await
        .unwrap()
}

async fn approval_resource(app: &TestApp, id: &str) -> Option<String> {
    sqlx::query_scalar("SELECT resource FROM approval_item WHERE id = $1")
        .bind(id)
        .fetch_one(&app.pool)
        .await
        .unwrap()
}

/// The console route the Share dialog reads, so a test sees the link as the
/// person would.
async fn public_link(app: &TestApp, cookie: &str, board: &str) -> Value {
    let resp = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/api/dashboard/public-link?board={board}"))
                .header("cookie", cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

/// Whether a link is live for `board` (a row whose expiry has not passed).
async fn live_link_exists(app: &TestApp, board: &str) -> bool {
    sqlx::query_scalar(
        "SELECT exists(SELECT 1 FROM dashboard_public_link \
         WHERE board_id = $1 AND expires_at > now())",
    )
    .bind(board)
    .fetch_one(&app.pool)
    .await
    .unwrap()
}

/// Whether any key named `wanted` sits anywhere in `value`, recursively.
fn has_key(value: &Value, wanted: &str) -> bool {
    match value {
        Value::Object(map) => {
            map.iter().any(|(k, v)| k == wanted || has_key(v, wanted))
                || map.values().any(|v| has_key(v, wanted))
        }
        Value::Array(items) => items.iter().any(|v| has_key(v, wanted)),
        _ => false,
    }
}

/// Days between now and an RFC 3339 `expiresAt`, as a rough number for the
/// expiry assertions.
fn seconds_until(text: &str) -> i64 {
    let at =
        time::OffsetDateTime::parse(text, &time::format_description::well_known::Rfc3339).unwrap();
    (at - time::OffsetDateTime::now_utc()).whole_seconds()
}

/// Make the link through the tool itself: the requester asks, the approver
/// (who also manages the board) approves, and the tool runs as them.
async fn make_link(app: &TestApp, boards: &[&str], days: Option<i64>) -> Value {
    let (user, cookie) = manager(app, boards).await;
    // `AI-11`: the approval exists only because somebody other
    // than the requester can decide it and run it, so the approver (who also
    // manages the boards) exists before the ask.
    let approver_cookie = approver(app, boards, &[user]).await;
    let mut args = json!({ "board": boards[0] });
    if let Some(days) = days {
        args["days"] = json!(days);
    }
    let asked = call(app, &cookie, "create_public_link", args, "build").await;
    let approval_id = asked["result"]["approval_id"]
        .as_str()
        .unwrap_or_else(|| panic!("no approval: {asked}"))
        .to_owned();
    let (status, decided) = decide(app, &approver_cookie, &approval_id).await;
    assert_eq!(status, StatusCode::OK, "{decided}");
    assert_eq!(decided["executed"], json!(true), "{decided}");
    decided
}

// ── create_public_link ──────────────────────────────────────────────────

#[tokio::test]
async fn a_manager_create_public_link_waits_for_approval_and_makes_no_link_before_it() {
    let (app, _ch) = app("0").await;
    let (user, cookie) = manager(&app, &["b1"]).await;
    // `AI-11`: this is the positive control for the refusal —
    // with a second manager who holds the approval permission, the approval
    // is created as before.
    let _approver_cookie = approver(&app, &["b1"], &[user]).await;
    let asked = call(
        &app,
        &cookie,
        "create_public_link",
        json!({ "board": "b1" }),
        "build",
    )
    .await;
    assert_eq!(asked["result"]["needs_approval"], json!(true), "{asked}");
    assert!(
        asked["result"]["approval_id"].is_string(),
        "an approval id: {asked}"
    );
    assert_eq!(approvals_for(&app, "create_public_link").await, 1);
    assert!(
        !live_link_exists(&app, "b1").await,
        "no link before a human approves"
    );
}

#[tokio::test]
async fn a_create_public_link_nobody_else_can_approve_is_refused_and_makes_no_approval_row() {
    // The requester manages `b1` through their rule alone: nobody else holds
    // a manage rule on it, so the request is refused here, with the fixed
    // sentence, and no approval waits for a person who could never run it
    // (`AI-11`).
    let (app, _ch) = app("0").await;
    let (user, cookie) = manager(&app, &["b1"]).await;
    // The seeded Platform Admin holds `*:*`, so it could decide and run the
    // call; deactivating every other account leaves the requester alone, which
    // is the case this test is about (`AI-11`).
    sqlx::query("UPDATE app_user SET status = 'inactive' WHERE id <> $1")
        .bind(user)
        .execute(&app.pool)
        .await
        .unwrap();
    let body = call(
        &app,
        &cookie,
        "create_public_link",
        json!({ "board": "b1" }),
        "build",
    )
    .await;
    assert_eq!(body["result"]["error"], json!(NO_OTHER_APPROVER), "{body}");
    assert_eq!(body["outcome"], "failed", "{body}");
    assert_eq!(approvals_for(&app, "create_public_link").await, 0);
    assert!(!live_link_exists(&app, "b1").await);
}

#[tokio::test]
async fn an_approver_who_cannot_manage_the_board_does_not_make_a_request_creatable() {
    // `AI-11`: the approval route lets an `agent:approve` holder decide, but
    // the approved call runs through the board's manage guard as that person.
    // An approver without manage on `b1` could never run it, so the request is
    // refused here with no approval row.
    let (app, _ch) = app("0").await;
    let (user, cookie) = manager(&app, &["b1"]).await;
    sqlx::query("UPDATE app_user SET status = 'inactive' WHERE id <> $1")
        .bind(user)
        .execute(&app.pool)
        .await
        .unwrap();
    let _outsider = create_principal_with_permissions(&app.pool, "agent:approve").await;
    let body = call(
        &app,
        &cookie,
        "create_public_link",
        json!({ "board": "b1" }),
        "build",
    )
    .await;
    assert_eq!(body["result"]["error"], json!(NO_OTHER_APPROVER), "{body}");
    assert_eq!(approvals_for(&app, "create_public_link").await, 0);
}

#[tokio::test]
async fn an_approved_create_public_link_opens_a_link_for_thirty_days_and_names_its_address() {
    let (app, _ch) = app("0").await;
    let (user, cookie) = manager(&app, &["b1"]).await;
    // The approver exists before the ask, so the request is creatable
    // (`AI-11`).
    let approver_cookie = approver(&app, &["b1"], &[user]).await;
    let asked = call(
        &app,
        &cookie,
        "create_public_link",
        json!({ "board": "b1" }),
        "build",
    )
    .await;
    let approval_id = asked["result"]["approval_id"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    // The card says it makes a new link, and names the dashboard.
    let reason = approval_reason(&app, &approval_id).await;
    assert!(reason.contains("b1"), "{reason}");
    assert!(reason.contains("new link"), "{reason}");
    // The approval is recorded against the dashboard (audit.rs's board arm).
    assert_eq!(
        approval_resource(&app, &approval_id).await,
        Some("b1".to_owned())
    );

    let (_status, decided) = decide(&app, &approver_cookie, &approval_id).await;
    assert_eq!(decided["executed"], json!(true), "{decided}");

    let shown = decided["result"]["url"].as_str().unwrap_or_default();
    let token = decided["result"]["token"].as_str().unwrap_or_default();
    assert!(
        shown.starts_with("/public/dashboard/") && shown.ends_with(token) && !token.is_empty(),
        "the address the Share dialog builds: {decided}"
    );
    let link = public_link(&app, &cookie, "b1").await;
    let stored_token = link["link"]["token"].as_str().unwrap_or_default();
    assert_eq!(token, stored_token, "the tool names the real token: {link}");
    let secs = seconds_until(link["link"]["expiresAt"].as_str().unwrap_or_default());
    assert!(
        (29 * 86_400..31 * 86_400).contains(&secs),
        "a default expiry of 30 days, got {secs}s"
    );
    assert!(!link["link"]["expired"].as_bool().unwrap_or(true), "{link}");
}

#[tokio::test]
async fn a_live_link_keeps_its_address_and_only_moves_the_expiry_when_days_names_fewer() {
    let (app, _ch) = app("0").await;
    make_link(&app, &["b1"], None).await;
    let (user, cookie) = manager(&app, &["b1"]).await;
    // The approver exists before the ask, so the request is creatable
    // (`AI-11`).
    let approver_cookie = approver(&app, &["b1"], &[user]).await;
    let before = public_link(&app, &cookie, "b1").await;
    let token = before["link"]["token"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    assert!(!token.is_empty(), "{before}");

    let asked = call(
        &app,
        &cookie,
        "create_public_link",
        json!({ "board": "b1", "days": 7 }),
        "build",
    )
    .await;
    let approval_id = asked["result"]["approval_id"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    // The card says it keeps the address this time, not that it makes a new link.
    let reason = approval_reason(&app, &approval_id).await;
    assert!(reason.contains("keeps its address"), "{reason}");
    let (_status, decided) = decide(&app, &approver_cookie, &approval_id).await;
    assert_eq!(decided["executed"], json!(true), "{decided}");

    let after = public_link(&app, &cookie, "b1").await;
    assert_eq!(
        after["link"]["token"].as_str().unwrap_or_default(),
        token,
        "the address survives an expiry change: {after}"
    );
    let secs = seconds_until(after["link"]["expiresAt"].as_str().unwrap_or_default());
    assert!(
        (6 * 86_400..8 * 86_400).contains(&secs),
        "seven days, got {secs}s"
    );
    assert_eq!(
        decided["result"]["token"].as_str().unwrap_or_default(),
        token,
        "{decided}"
    );
}

#[tokio::test]
async fn a_days_outside_one_to_thirty_is_refused_with_no_approval_row() {
    let (app, _ch) = app("0").await;
    let (_user, cookie) = manager(&app, &["b1"]).await;
    for days in [60, 0] {
        let body = call(
            &app,
            &cookie,
            "create_public_link",
            json!({ "board": "b1", "days": days }),
            "build",
        )
        .await;
        assert_eq!(body["result"]["error"], json!(DAYS_REFUSED), "{body}");
        assert_eq!(body["outcome"], "failed", "{body}");
    }
    assert_eq!(approvals_for(&app, "create_public_link").await, 0);
    assert!(!live_link_exists(&app, "b1").await);
}

#[tokio::test]
async fn a_requester_without_manage_is_refused_and_no_approval_is_written() {
    let (app, _ch) = app("0").await;
    let viewer = create_assistant_principal(&app.pool, "dashboard:read").await;
    let cookie = session_cookie_for_user(&app.pool, viewer).await;
    // `revoke_public_link` is a confirm tool, so the gate asks for the
    // confirm first and only the confirmed call reaches the tool's own
    // manage check. `create_public_link`'s check runs before its approval;
    // `list_dashboard_access` is a read and runs at once.
    for (tool, args) in [
        ("create_public_link", json!({ "board": "b1" })),
        (
            "revoke_public_link",
            json!({ "board": "b1", "confirmed": true }),
        ),
        ("list_dashboard_access", json!({ "board": "b1" })),
    ] {
        let body = call(&app, &cookie, tool, args, "build").await;
        let error = body["result"]["error"].as_str().unwrap_or_default();
        assert!(
            error.contains("manage on this dashboard") || error.contains("manage on this item"),
            "{tool}: {body}"
        );
        assert_eq!(body["outcome"], "failed", "{body}");
        assert_eq!(approvals_for(&app, tool).await, 0, "{tool}");
    }
    assert!(!live_link_exists(&app, "b1").await);
}

#[tokio::test]
async fn an_approved_link_whose_replay_is_refused_returns_the_routes_sentence() {
    // `AI-11`: the approved call runs as the approver, the route
    // refuses it (public links were switched off after the request), and the
    // result carries the route's own sentence, not only a generic failure.
    let (app, _ch) = app("0").await;
    let (user, cookie) = manager(&app, &["b1"]).await;
    let approver_cookie = approver(&app, &["b1"], &[user]).await;
    let asked = call(
        &app,
        &cookie,
        "create_public_link",
        json!({ "board": "b1" }),
        "build",
    )
    .await;
    let approval_id = asked["result"]["approval_id"]
        .as_str()
        .unwrap_or_else(|| panic!("no approval: {asked}"))
        .to_owned();
    sqlx::query(
        "INSERT INTO reporting_settings (singleton, time_zone, week_start, public_links_enabled) \
         VALUES (TRUE, 'UTC', 'monday', FALSE) \
         ON CONFLICT (singleton) DO UPDATE SET public_links_enabled = FALSE",
    )
    .execute(&app.pool)
    .await
    .unwrap();

    let (_status, decided) = decide(&app, &approver_cookie, &approval_id).await;
    assert_eq!(decided["executed"], json!(true), "{decided}");
    assert_eq!(decided["result"]["error"], json!(SWITCHED_OFF), "{decided}");
    assert!(!live_link_exists(&app, "b1").await);
}

#[tokio::test]
async fn a_switched_off_install_is_refused_with_the_routes_sentence_and_no_approval_row() {
    let (app, _ch) = app("0").await;
    sqlx::query(
        "INSERT INTO reporting_settings (singleton, time_zone, week_start, public_links_enabled) \
         VALUES (TRUE, 'UTC', 'monday', FALSE) \
         ON CONFLICT (singleton) DO UPDATE SET public_links_enabled = FALSE",
    )
    .execute(&app.pool)
    .await
    .unwrap();
    let (_user, cookie) = manager(&app, &["b1"]).await;
    let body = call(
        &app,
        &cookie,
        "create_public_link",
        json!({ "board": "b1" }),
        "build",
    )
    .await;
    assert_eq!(body["result"]["error"], json!(SWITCHED_OFF), "{body}");
    assert_eq!(approvals_for(&app, "create_public_link").await, 0);
}

#[tokio::test]
async fn a_personal_board_is_refused_with_the_routes_sentence_and_no_approval_row() {
    // The stand-in is built here so the owner can be set on the SAME store
    // the mounted mock serves: `Boards` is a shared `Arc`, so the change is
    // visible to every later request, including the app's.
    let store = Boards::new("0");
    let ch = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(store.clone())
        .mount(&ch)
        .await;
    let env: HashMap<String, String> = HashMap::from([
        ("CH_URL".to_owned(), ch.uri()),
        (
            "CATALOG_TENANT_ID".to_owned(),
            common::CATALOG_TENANT.to_owned(),
        ),
    ]);
    let (app, _ch) = (spin_up_with_env(&env).await, ch);
    let (user, cookie) = manager(&app, &["b_own"]).await;
    store.set_owner("b_own", &user.to_string());
    // The owner manages their own board, so the manage check passes and the
    // route's personal-space sentence is what refuses.
    let body = call(
        &app,
        &cookie,
        "create_public_link",
        json!({ "board": "b_own" }),
        "build",
    )
    .await;
    assert_eq!(
        body["result"]["error"],
        json!(PERSONAL_NOT_SHAREABLE),
        "{body}"
    );
    assert_eq!(approvals_for(&app, "create_public_link").await, 0);
}

/// `AI-11` decision 1: the schema offers no `password` argument. The tool
/// never sets or reads one; the schema is the fixture the console reads.
#[test]
fn the_create_public_link_schema_has_no_password_argument() {
    let fixture: Value = serde_json::from_str(include_str!("fixtures/tool_schemas.json")).unwrap();
    let schema = fixture
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["function"]["name"] == "create_public_link")
        .unwrap_or_else(|| panic!("create_public_link is not in the fixture"));
    let text = schema.to_string();
    assert!(
        !text.contains("password"),
        "the schema must not offer a password: {text}"
    );
    assert!(
        schema["function"]["parameters"]["properties"]["board"].is_object(),
        "{schema}"
    );
}

// ── revoke_public_link ──────────────────────────────────────────────────

#[tokio::test]
async fn revoke_public_link_needs_a_confirm_then_ends_the_link_and_reports_when_there_is_none() {
    let (app, _ch) = app("0").await;
    make_link(&app, &["b1"], None).await;
    assert!(live_link_exists(&app, "b1").await);
    let (_user, cookie) = manager(&app, &["b1"]).await;

    let asked = call(
        &app,
        &cookie,
        "revoke_public_link",
        json!({ "board": "b1" }),
        "build",
    )
    .await;
    assert_eq!(
        asked["result"]["needs_confirmation"],
        json!(true),
        "{asked}"
    );
    assert!(live_link_exists(&app, "b1").await, "a confirm ends nothing");

    let done = call(
        &app,
        &cookie,
        "revoke_public_link",
        json!({ "board": "b1", "confirmed": true }),
        "build",
    )
    .await;
    assert_eq!(done["result"]["ended"], json!(true), "{done}");
    assert!(!live_link_exists(&app, "b1").await, "{done}");

    // With no link there is nothing to end, and the tool says so honestly.
    let again = call(
        &app,
        &cookie,
        "revoke_public_link",
        json!({ "board": "b1", "confirmed": true }),
        "build",
    )
    .await;
    assert_eq!(again["result"]["ended"], json!(false), "{again}");
}

#[tokio::test]
async fn revoke_public_link_waits_for_a_confirm_and_not_for_an_approval() {
    let (app, _ch) = app("0").await;
    let (_user, cookie) = manager(&app, &["b1"]).await;
    let asked = call(
        &app,
        &cookie,
        "revoke_public_link",
        json!({ "board": "b1" }),
        "build",
    )
    .await;
    assert_eq!(
        asked["result"]["needs_confirmation"],
        json!(true),
        "{asked}"
    );
    assert!(
        asked["result"].get("needs_approval").is_none(),
        "ending a link narrows access: it never waits in Approvals: {asked}"
    );
    assert_eq!(approvals_for(&app, "revoke_public_link").await, 0);
}

// ── list_dashboard_access ───────────────────────────────────────────────

#[tokio::test]
async fn list_dashboard_access_shows_rules_level_link_and_embed_and_never_a_token() {
    let (app, _ch) = app("1").await;
    make_link(&app, &["b1"], Some(7)).await;
    let (_user, cookie) = manager(&app, &["b1"]).await;
    let body = call(
        &app,
        &cookie,
        "list_dashboard_access",
        json!({ "board": "b1" }),
        "ask",
    )
    .await;
    let result = &body["result"];
    assert_eq!(result["yourLevel"], json!("manage"), "{body}");
    assert!(result["rules"].is_array(), "{body}");
    assert_eq!(result["embedding"], json!(true), "{body}");
    let link = &result["publicLink"];
    assert!(link["expiresAt"].is_string(), "{body}");
    assert_eq!(link["expired"], json!(false), "{body}");
    assert!(
        !has_key(result, "token"),
        "no token anywhere in the answer: {body}"
    );
    assert!(
        !has_key(result, "password"),
        "no password key anywhere in the answer (`hasPassword`, the flag, is legitimate and stays): {body}"
    );
}

#[tokio::test]
async fn list_dashboard_access_names_no_link_when_there_is_none() {
    let (app, _ch) = app("0").await;
    let (_user, cookie) = manager(&app, &["b1"]).await;
    let body = call(
        &app,
        &cookie,
        "list_dashboard_access",
        json!({ "board": "b1" }),
        "ask",
    )
    .await;
    assert_eq!(body["result"]["publicLink"], Value::Null, "{body}");
    assert!(!has_key(&body["result"], "token"), "{body}");
}

// ── move_chart widening (decision 6) ────────────────────────────────────

#[tokio::test]
async fn a_move_to_a_board_that_lists_somebody_new_waits_for_approval() {
    let (app, _ch) = app("0").await;
    let (writer_id, cookie) = writer(&app, &["b1", "b2"]).await;
    // `b2` lists one person more than `b1` does: moving the chart there
    // widens who can see it.
    let viewer = create_assistant_principal(&app.pool, "dashboard:read").await;
    let mut rules = vec![rule(writer_id, "manage")];
    rules.push(rule(viewer, "view"));
    replace_item(&app.pool, "board", "b2", &rules, false, None)
        .await
        .unwrap();

    let asked = call(
        &app,
        &cookie,
        "move_chart",
        json!({ "id": "c1", "board": "b2" }),
        "build",
    )
    .await;
    assert_eq!(
        asked["result"]["needs_approval"],
        json!(true),
        "a widening move waits for approval: {asked}"
    );
    assert_eq!(asked["outcome"], "needs_approval", "{asked}");
    assert_eq!(approvals_for(&app, "move_chart").await, 1);
}

#[tokio::test]
async fn a_move_between_boards_with_the_same_audience_stays_a_confirm() {
    let (app, _ch) = app("0").await;
    let (_writer, cookie) = writer(&app, &["b1", "b2"]).await;
    let asked = call(
        &app,
        &cookie,
        "move_chart",
        json!({ "id": "c1", "board": "b2" }),
        "build",
    )
    .await;
    assert_eq!(
        asked["result"]["needs_confirmation"],
        json!(true),
        "the same audience is a confirm: {asked}"
    );
    assert_eq!(asked["outcome"], "needs_confirmation", "{asked}");
    assert!(asked["result"].get("needs_approval").is_none(), "{asked}");
}

#[tokio::test]
async fn a_move_whose_comparison_cannot_be_answered_waits_for_approval() {
    let (app, _ch) = app("0").await;
    let (_writer, cookie) = writer(&app, &["b1", "b2"]).await;
    // No chart named `ghost` exists, so the source audience is unknown; the
    // gate fails closed to an approval instead of a bare confirm.
    let asked = call(
        &app,
        &cookie,
        "move_chart",
        json!({ "id": "ghost", "board": "b2" }),
        "build",
    )
    .await;
    assert_eq!(asked["result"]["needs_approval"], json!(true), "{asked}");
}

// ── decision 5: no tool writes access ───────────────────────────────────

/// `AI-11` decision 5: no assistant tool reaches the access-write function
/// (`dashboard_access::put_access`). A lexical scan of the tools modules,
/// in the style of `tests/sec11_guard.rs`: the name on a line there means a
/// call is being written, and no tool may write who a dashboard is shared
/// with — the console's Share > People owns that. The allowlist is empty on
/// purpose and stays empty: an entry needs a reason here, and an entry that
/// no line matches is a failure, so it cannot go stale quietly.
#[test]
fn no_tool_module_reaches_the_access_write_function() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/src/routes/ai/tools");
    // (substring of the line, reason it is not an access write) — empty.
    let allowlist: [(&str, &str); 0] = [];
    let mut offenders: Vec<String> = Vec::new();
    let mut matched: Vec<bool> = vec![false; allowlist.len()];
    let mut entries = std::fs::read_dir(dir)
        .unwrap()
        .collect::<Result<Vec<std::fs::DirEntry>, _>>()
        .unwrap();
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "rs") {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        for (n, line) in text.lines().enumerate() {
            if !line.contains("put_access") {
                continue;
            }
            if let Some(at) = allowlist
                .iter()
                .position(|(needle, _)| line.contains(needle))
            {
                matched[at] = true;
            } else {
                offenders.push(format!(
                    "{}:{n}: {}",
                    path.file_name().unwrap().to_string_lossy(),
                    line.trim()
                ));
            }
        }
    }
    for (allowed, hit) in allowlist.iter().zip(&matched) {
        assert!(hit, "stale allowlist entry {allowed:?}: no line matches it");
    }
    assert!(
        offenders.is_empty(),
        "assistant tool modules reaching the access write: {offenders:#?}"
    );
}
