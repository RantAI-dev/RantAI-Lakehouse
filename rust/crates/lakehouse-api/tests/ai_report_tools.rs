//! The scheduled-report tools through `POST /api/ai/tool` and the approvals
//! route (`AI-10`, schedule half; `RPT-1` T6, plan decision D10): a report for
//! the person alone needs only the Confirm button, naming anyone else waits for
//! approval, a delete waits for approval, the report is made as the requester
//! even when someone else approves, and the requester's rights are checked
//! before the approval exists.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::collections::HashMap;

use axum::body::{Body, to_bytes};
use axum::http::Request;
use common::{
    TestApp, create_principal_with_permissions, session_cookie_for_user, spin_up_with_env,
};
use lakehouse_store::report_subscriptions as subs;
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, Request as MockRequest, Respond, ResponseTemplate};

const BOARD: &str = "b_ai";

struct Store;

impl Respond for Store {
    fn respond(&self, request: &MockRequest) -> ResponseTemplate {
        let sql = String::from_utf8_lossy(&request.body);
        let rows = |data: Vec<Value>| {
            let n = data.len();
            ResponseTemplate::new(200).set_body_json(json!({ "meta": [], "data": data, "rows": n }))
        };
        if sql.contains("FROM console.bi_chart") {
            return rows(vec![]);
        }
        if sql.contains("FROM console.bi_board") && sql.contains(&format!("id='{BOARD}'")) {
            return rows(vec![json!({
                "id": BOARD, "name": "Sales", "description": "", "created_by": "t",
                "layout_json": "", "filters_json": "", "public_token": "", "embed_enabled": "0",
                "folder_id": "", "created_at": "2026-10-10 00:00:00", "owner_id": "",
                "embed_revoked_before": "0", "embed_revoked_jti_json": "[]",
                "embed_origins_json": "[]"
            })]);
        }
        if sql.contains("FROM console.bi_board") {
            return rows(vec![]);
        }
        ResponseTemplate::new(200).set_body_string("Ok.\n")
    }
}

struct F {
    app: TestApp,
    me: Uuid,
    cookie: String,
    approver: Uuid,
    approver_cookie: String,
    _ch: MockServer,
}

async fn fixture() -> F {
    let ch = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(Store)
        .mount(&ch)
        .await;
    let env: HashMap<String, String> = HashMap::from([
        ("CH_URL".to_owned(), ch.uri()),
        (
            "GOTENBERG_URL".to_owned(),
            "http://gotenberg:3000".to_owned(),
        ),
        (
            "CONSOLE_INTERNAL_URL".to_owned(),
            "http://console:3000".to_owned(),
        ),
        (
            "RENDER_TOKEN_SECRET".to_owned(),
            "test-render-secret-not-real".to_owned(),
        ),
    ]);
    let app = spin_up_with_env(&env).await;
    let me = create_principal_with_permissions(&app.pool, "dashboard:read, dashboard:write").await;
    let cookie = session_cookie_for_user(&app.pool, me).await;
    // `dashboard:write` is not the tool's permission; it is here so the
    // requester could also use other tools. The approver is a different person.
    let approver =
        create_principal_with_permissions(&app.pool, "dashboard:read, agent:approve").await;
    let approver_cookie = session_cookie_for_user(&app.pool, approver).await;
    F {
        app,
        me,
        cookie,
        approver,
        approver_cookie,
        _ch: ch,
    }
}

async fn post(f: &F, uri: &str, cookie: &str, body: Value) -> Value {
    let resp = f
        .app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header("content-type", "application/json")
                .header("cookie", cookie)
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let mut v: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    v["__status"] = json!(status.as_u16());
    v
}

async fn tool(f: &F, name: &str, args: Value) -> Value {
    post(
        f,
        "/api/ai/tool",
        &f.cookie,
        json!({ "tool": name, "args": args, "mode": "build" }),
    )
    .await
}

/// The AI-10-AC1 request: "Send this dashboard to me every Monday as a PDF."
fn monday_pdf() -> Value {
    json!({
        "board": BOARD,
        "schedule": { "kind": "weekly", "weekday": 1, "hour": 8 },
        "pdf": true,
    })
}

async fn all(pool: &PgPool) -> Vec<subs::Subscription> {
    subs::list_for_board(pool, BOARD).await.unwrap()
}

async fn approve(f: &F, approval: &str, cookie: &str) -> Value {
    post(
        f,
        &format!("/api/agents/approvals/{approval}/decide"),
        cookie,
        json!({ "decision": "approved" }),
    )
    .await
}

#[tokio::test]
async fn the_quoted_request_creates_a_monday_pdf_subscription_for_the_person_after_confirm() {
    let f = fixture().await;
    // Without Confirm nothing is created and the summary says what would be.
    let pending = tool(&f, "create_report_subscription", monday_pdf()).await;
    assert_eq!(pending["outcome"], "needs_confirmation", "{pending}");
    assert_eq!(
        all(&f.app.pool).await,
        [] as [lakehouse_store::report_subscriptions::Subscription; 0]
    );

    let mut confirmed = monday_pdf();
    confirmed["confirmed"] = json!(true);
    let done = tool(&f, "create_report_subscription", confirmed).await;
    assert_eq!(done["outcome"], "executed", "{done}");
    assert_eq!(done["result"]["created"], json!(true));

    let made = all(&f.app.pool).await;
    assert_eq!(made.len(), 1);
    assert_eq!(made[0].owner_id, f.me);
    assert_eq!(made[0].kind, "weekly");
    assert_eq!(
        (made[0].weekday, made[0].hour, made[0].minute),
        (Some(1), Some(8), 0)
    );
    assert_eq!(made[0].attachments["pdf"]["paper"], "a4");
    let recipients = subs::recipients(&f.app.pool, made[0].id).await.unwrap();
    assert_eq!(recipients.len(), 1);
    assert_eq!(recipients[0].user_id, Some(f.me), "to the person alone");
    // No approval was created for it.
    let approvals: i64 = sqlx::query_scalar("SELECT count(*) FROM approval_item")
        .fetch_one(&f.app.pool)
        .await
        .unwrap();
    assert_eq!(approvals, 0);
}

#[tokio::test]
async fn naming_another_recipient_waits_for_approval_and_the_report_stays_the_requesters() {
    let f = fixture().await;
    let colleague: Uuid = Uuid::new_v4();
    sqlx::query("INSERT INTO app_user (id, name, email, status) VALUES ($1, 'Colleague', 'colleague@company.example', 'active')")
        .bind(colleague)
        .execute(&f.app.pool)
        .await
        .unwrap();
    let mut args = monday_pdf();
    args["recipients"] = json!([{ "userId": colleague.to_string() }]);
    // Even with Confirm: another recipient is an approval, not a confirmation.
    args["confirmed"] = json!(true);
    let waiting = tool(&f, "create_report_subscription", args).await;
    assert_eq!(waiting["outcome"], "needs_approval", "{waiting}");
    assert!(
        all(&f.app.pool).await.is_empty(),
        "nothing exists before approval"
    );
    let approval = waiting["result"]["approval_id"]
        .as_str()
        .unwrap()
        .to_owned();

    // The stored arguments carry the requester, written by the server.
    let evidence: Vec<String> =
        sqlx::query_scalar("SELECT evidence FROM approval_item WHERE id = $1")
            .bind(&approval)
            .fetch_one(&f.app.pool)
            .await
            .unwrap();
    assert!(
        evidence.join(" ").contains(&f.me.to_string()),
        "{evidence:?}"
    );

    // Somebody else approves; the report is still the requester's.
    let decided = approve(&f, &approval, &f.approver_cookie).await;
    assert_eq!(decided["executed"], json!(true), "{decided}");
    assert!(decided["result"]["created"] == json!(true), "{decided}");
    let made = all(&f.app.pool).await;
    assert_eq!(made.len(), 1);
    assert_eq!(
        made[0].owner_id, f.me,
        "made as the requester, not the approver"
    );
    assert_ne!(made[0].owner_id, f.approver);
    let recipients = subs::recipients(&f.app.pool, made[0].id).await.unwrap();
    let ids: Vec<Option<Uuid>> = recipients.iter().map(|r| r.user_id).collect();
    assert!(ids.contains(&Some(f.me)) && ids.contains(&Some(colleague)));
}

#[tokio::test]
async fn the_requesters_rights_are_checked_before_an_approval_is_created() {
    let f = fixture().await;
    // An outside address on a domain nobody listed: refused now, no approval.
    let mut outside = monday_pdf();
    outside["recipients"] = json!([{ "email": "press@partner.example" }]);
    let refused = tool(&f, "create_report_subscription", outside).await;
    assert_eq!(refused["outcome"], "failed", "{refused}");
    assert!(
        refused["result"]["error"]
            .as_str()
            .unwrap()
            .contains("has not allowed"),
        "{refused}"
    );
    // A role switched off from downloading: refused now.
    let role: String = sqlx::query_scalar(
        "SELECT r.name FROM role r JOIN app_user_role ur ON ur.role_id = r.id WHERE ur.user_id = $1",
    )
    .bind(f.me)
    .fetch_one(&f.app.pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO reporting_settings (singleton, time_zone, week_start, download_denied_roles, report_allowed_domains) \
         VALUES (TRUE, 'UTC', 'monday', ARRAY[$1], ARRAY['partner.example'])",
    )
    .bind(role)
    .execute(&f.app.pool)
    .await
    .unwrap();
    let mut listed = monday_pdf();
    listed["recipients"] = json!([{ "email": "press@partner.example" }]);
    let refused = tool(&f, "create_report_subscription", listed).await;
    assert_eq!(refused["outcome"], "failed", "{refused}");
    assert!(
        refused["result"]["error"]
            .as_str()
            .unwrap()
            .contains("Downloads are switched off"),
        "{refused}"
    );
    let approvals: i64 = sqlx::query_scalar("SELECT count(*) FROM approval_item")
        .fetch_one(&f.app.pool)
        .await
        .unwrap();
    assert_eq!(approvals, 0, "no approval was asked for");
}

#[tokio::test]
async fn an_approver_who_may_not_download_cannot_release_the_report() {
    let f = fixture().await;
    sqlx::query(
        "INSERT INTO reporting_settings (singleton, time_zone, week_start, report_allowed_domains) \
         VALUES (TRUE, 'UTC', 'monday', ARRAY['partner.example'])",
    )
    .execute(&f.app.pool)
    .await
    .unwrap();
    let mut args = monday_pdf();
    args["recipients"] = json!([{ "email": "press@partner.example" }]);
    let waiting = tool(&f, "create_report_subscription", args).await;
    let approval = waiting["result"]["approval_id"]
        .as_str()
        .unwrap()
        .to_owned();
    // The approver's role is switched off from downloading after the request.
    let approver_role: String = sqlx::query_scalar(
        "SELECT r.name FROM role r JOIN app_user_role ur ON ur.role_id = r.id WHERE ur.user_id = $1",
    )
    .bind(f.approver)
    .fetch_one(&f.app.pool)
    .await
    .unwrap();
    sqlx::query("UPDATE reporting_settings SET download_denied_roles = ARRAY[$1]")
        .bind(approver_role)
        .execute(&f.app.pool)
        .await
        .unwrap();
    let decided = approve(&f, &approval, &f.approver_cookie).await;
    assert!(
        decided["result"]["error"]
            .as_str()
            .unwrap_or("")
            .contains("Downloads are switched off"),
        "{decided}"
    );
    assert_eq!(
        all(&f.app.pool).await,
        [] as [lakehouse_store::report_subscriptions::Subscription; 0]
    );
}

#[tokio::test]
async fn a_model_cannot_name_the_owner() {
    let f = fixture().await;
    let someone = create_principal_with_permissions(&f.app.pool, "dashboard:read").await;
    let mut args = monday_pdf();
    args["ownerId"] = json!(someone.to_string());
    args["confirmed"] = json!(true);
    let done = tool(&f, "create_report_subscription", args).await;
    assert_eq!(done["outcome"], "executed", "{done}");
    assert_eq!(all(&f.app.pool).await[0].owner_id, f.me);
}

#[tokio::test]
async fn updating_and_listing_work_on_the_callers_own_and_a_delete_waits_for_approval_then_runs_as_the_requester()
 {
    let f = fixture().await;
    let mut args = monday_pdf();
    args["confirmed"] = json!(true);
    tool(&f, "create_report_subscription", args).await;
    let id = all(&f.app.pool).await[0].id.to_string();

    let listed = tool(&f, "list_report_subscriptions", json!({})).await;
    assert_eq!(listed["result"]["items"].as_array().unwrap().len(), 1);
    assert_eq!(listed["result"]["items"][0]["schedule"]["kind"], "weekly");

    // Change the time only: the rest stays, including the recipient.
    let changed = tool(
        &f,
        "update_report_subscription",
        json!({ "id": id, "schedule": { "kind": "daily", "hour": 6 }, "confirmed": true }),
    )
    .await;
    assert_eq!(changed["outcome"], "executed", "{changed}");
    let now = &all(&f.app.pool).await[0];
    assert_eq!(now.kind, "daily");
    assert_eq!(now.attachments["pdf"]["paper"], "a4");
    assert_eq!(
        subs::recipients(&f.app.pool, now.id).await.unwrap().len(),
        1
    );

    // Pausing is an update too.
    let paused = tool(
        &f,
        "update_report_subscription",
        json!({ "id": id, "paused": true, "confirmed": true }),
    )
    .await;
    assert_eq!(paused["outcome"], "executed", "{paused}");
    assert!(all(&f.app.pool).await[0].paused);

    // A delete is an approval, and nothing is deleted until it is approved.
    let waiting = tool(
        &f,
        "delete_report_subscription",
        json!({ "id": id, "confirmed": true }),
    )
    .await;
    assert_eq!(waiting["outcome"], "needs_approval", "{waiting}");
    assert_eq!(all(&f.app.pool).await.len(), 1);
    let approval = waiting["result"]["approval_id"]
        .as_str()
        .unwrap()
        .to_owned();
    // The approved delete runs as the requester, who owns it: the person who
    // approves does not need to own it.
    let decided = approve(&f, &approval, &f.approver_cookie).await;
    assert_eq!(decided["result"]["deleted"], json!(true), "{decided}");
    assert_eq!(
        all(&f.app.pool).await,
        [] as [lakehouse_store::report_subscriptions::Subscription; 0]
    );
}

#[tokio::test]
async fn an_administrator_approver_deletes_the_requesters_subscription() {
    let f = fixture().await;
    let mut args = monday_pdf();
    args["confirmed"] = json!(true);
    tool(&f, "create_report_subscription", args).await;
    let id = all(&f.app.pool).await[0].id.to_string();
    let waiting = tool(&f, "delete_report_subscription", json!({ "id": id })).await;
    let approval = waiting["result"]["approval_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let admin = create_principal_with_permissions(
        &f.app.pool,
        "dashboard:read, agent:approve, settings:write",
    )
    .await;
    let admin_cookie = session_cookie_for_user(&f.app.pool, admin).await;
    let decided = approve(&f, &approval, &admin_cookie).await;
    assert_eq!(decided["result"]["deleted"], json!(true), "{decided}");
    assert_eq!(
        all(&f.app.pool).await,
        [] as [lakehouse_store::report_subscriptions::Subscription; 0]
    );
}

#[tokio::test]
async fn someone_elses_subscription_is_not_found_to_the_assistant_and_a_stranger_cannot_ask_to_delete_it()
 {
    let f = fixture().await;
    let mut args = monday_pdf();
    args["confirmed"] = json!(true);
    tool(&f, "create_report_subscription", args).await;
    let id = all(&f.app.pool).await[0].id.to_string();
    let stranger = create_principal_with_permissions(&f.app.pool, "dashboard:read").await;
    let stranger_cookie = session_cookie_for_user(&f.app.pool, stranger).await;
    let listed = post(
        &f,
        "/api/ai/tool",
        &stranger_cookie,
        json!({ "tool": "list_report_subscriptions", "args": {}, "mode": "build" }),
    )
    .await;
    assert!(
        listed["result"]["items"].as_array().unwrap().is_empty(),
        "{:?}",
        listed["result"]["items"]
    );
    let refused = post(
        &f,
        "/api/ai/tool",
        &stranger_cookie,
        json!({ "tool": "delete_report_subscription", "args": { "id": id }, "mode": "build" }),
    )
    .await;
    assert_eq!(
        refused["outcome"], "failed",
        "no approval for something they may not do: {refused}"
    );
    assert_eq!(all(&f.app.pool).await.len(), 1);
}

/// A subscription of `f.me` whose stored list holds a colleague and an outside
/// address (the domain is allowed), made through the store.
async fn subscription_with_outsiders(f: &F) -> String {
    sqlx::query(
        "INSERT INTO reporting_settings (singleton, time_zone, week_start, report_allowed_domains) \
         VALUES (TRUE, 'UTC', 'monday', ARRAY['partner.example']) \
         ON CONFLICT (singleton) DO UPDATE SET report_allowed_domains = ARRAY['partner.example']",
    )
    .execute(&f.app.pool)
    .await
    .unwrap();
    let colleague = Uuid::new_v4();
    sqlx::query("INSERT INTO app_user (id, name, email, status) VALUES ($1, 'Colleague', 'colleague@company.example', 'active')")
        .bind(colleague)
        .execute(&f.app.pool)
        .await
        .unwrap();
    let new = subs::NewSubscription {
        board: BOARD.to_owned(),
        owner_id: f.me,
        schedule: lakehouse_store::report_schedule::Schedule::Weekly {
            weekday: 1,
            hour: 8,
            minute: 0,
        },
        time_zone: "UTC".to_owned(),
        attachments: json!({ "pdf": { "paper": "a4", "orientation": "portrait" }, "charts": [] }),
        filters: None,
        params: None,
        grain: None,
        skip_when_empty: false,
        next_run_at: time::OffsetDateTime::now_utc() + time::Duration::days(1),
        recipients: vec![
            subs::Recipient::User(f.me),
            subs::Recipient::User(colleague),
            subs::Recipient::Email("press@partner.example".to_owned()),
        ],
    };
    subs::create(&f.app.pool, &new)
        .await
        .unwrap()
        .id
        .to_string()
}

/// RPT-1 review fix (BLOCKER) R2: a change to a report that goes to other people
/// is judged by who receives it, not by the recipients the call names.
#[tokio::test]
async fn changing_what_a_stored_outside_list_receives_needs_approval_but_narrowing_is_a_confirm() {
    let f = fixture().await;
    let id = subscription_with_outsiders(&f).await;

    // A new schedule for the stored list, with Confirm: still an approval.
    let waiting = tool(
        &f,
        "update_report_subscription",
        json!({ "id": id, "schedule": { "kind": "daily", "hour": 6 }, "confirmed": true }),
    )
    .await;
    assert_eq!(waiting["outcome"], "needs_approval", "{waiting}");
    assert_eq!(
        all(&f.app.pool).await[0].kind,
        "weekly",
        "nothing changed yet"
    );

    // Adding a recipient is an approval too.
    let added = tool(
        &f,
        "update_report_subscription",
        json!({ "id": id, "recipients": [{ "email": "other@partner.example" }], "confirmed": true }),
    )
    .await;
    assert_eq!(added["outcome"], "needs_approval", "{added}");

    // Removing the outside address, and pausing, are Confirms.
    let removed = tool(
        &f,
        "update_report_subscription",
        json!({ "id": id, "sendToMe": true, "recipients": [], "confirmed": true }),
    )
    .await;
    assert_eq!(removed["outcome"], "executed", "{removed}");
    let after = subs::recipients(&f.app.pool, all(&f.app.pool).await[0].id)
        .await
        .unwrap();
    assert_eq!(after.len(), 1);
    assert_eq!(after[0].user_id, Some(f.me));
}

#[tokio::test]
async fn pausing_a_report_with_an_outside_list_is_a_confirm() {
    let f = fixture().await;
    let id = subscription_with_outsiders(&f).await;
    let pending = tool(
        &f,
        "update_report_subscription",
        json!({ "id": id, "paused": true }),
    )
    .await;
    assert_eq!(pending["outcome"], "needs_confirmation", "{pending}");
    assert!(
        pending["result"]["summary"]
            .as_str()
            .unwrap()
            .contains("tanpa penerima baru"),
        "{pending}"
    );
    let done = tool(
        &f,
        "update_report_subscription",
        json!({ "id": id, "paused": true, "confirmed": true }),
    )
    .await;
    assert_eq!(done["outcome"], "executed", "{done}");
    assert!(all(&f.app.pool).await[0].paused);
}

/// RPT-1 review fix (SHOULD-FIX) R4: the assistant pauses a report without
/// re-validating it, so a delisted domain does not keep it running.
#[tokio::test]
async fn the_assistant_can_pause_a_report_whose_domain_is_no_longer_allowed() {
    let f = fixture().await;
    let id = subscription_with_outsiders(&f).await;
    sqlx::query("UPDATE reporting_settings SET report_allowed_domains = '{}'")
        .execute(&f.app.pool)
        .await
        .unwrap();
    let done = tool(
        &f,
        "update_report_subscription",
        json!({ "id": id, "paused": true, "confirmed": true }),
    )
    .await;
    assert_eq!(done["outcome"], "executed", "{done}");
    assert!(all(&f.app.pool).await[0].paused);
    let resumed = tool(
        &f,
        "update_report_subscription",
        json!({ "id": id, "paused": false, "confirmed": true }),
    )
    .await;
    assert_eq!(resumed["outcome"], "executed", "{resumed}");
    assert!(!all(&f.app.pool).await[0].paused);
}
