//! The scheduled-report routes over the real router (`RPT-1` T4, plan D7 and
//! D8): who may subscribe, the recipient rules, who may change what, "Send now"
//! recording a delivery, the recipient picker and the administrator's list of
//! outside domains.
//!
//! Real per-test Postgres; `ClickHouse` is a `wiremock` stand-in with one board
//! and one data chart.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::collections::HashMap;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use common::{
    TestApp, create_principal_with_permissions, session_cookie_for_user, spin_up_with_env,
};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, Request as MockRequest, Respond, ResponseTemplate};

const BOARD: &str = "b_rpt";

struct FakeCh;

impl Respond for FakeCh {
    fn respond(&self, request: &MockRequest) -> ResponseTemplate {
        let sql = String::from_utf8_lossy(&request.body).into_owned();
        let rows = |data: Vec<Value>| {
            let n = data.len();
            ResponseTemplate::new(200).set_body_json(json!({ "meta": [], "data": data, "rows": n }))
        };
        if sql.contains("JSONCompactEachRowWithNamesAndTypes") {
            return ResponseTemplate::new(200).set_body_string(
                "[\"region\",\"email\"]\n[\"String\",\"String\"]\n[\"north\",\"a@x\"]\n",
            );
        }
        if sql.contains("FROM console.bi_chart") {
            let spec = json!({
                "id": "c_bar", "title": "Visits", "kind": "bar", "mart": "mart_x",
                "sql": "SELECT stored", "x": "region", "y": "email"
            });
            let def = json!({
                "title": "Visits", "mart": "mart_x", "kind": "bar",
                "dimension": "region", "measures": ["email"]
            });
            let text = json!({
                "id": "c_text", "title": "Note", "kind": "text", "mart": "", "sql": "", "x": "", "y": ""
            });
            return rows(vec![
                json!({
                    "id": "c_bar", "board": BOARD, "created_by": "ui",
                    "created_at": "2026-01-01 00:00:00",
                    "spec_json": json!({ "spec": spec, "def": def, "hasYear": false }).to_string(),
                }),
                json!({
                    "id": "c_text", "board": BOARD, "created_by": "ui",
                    "created_at": "2026-01-01 00:00:00",
                    "spec_json": json!({ "spec": text, "def": {"kind": "text", "text": "hi"}, "hasYear": false }).to_string(),
                }),
            ]);
        }
        if sql.contains("FROM console.bi_board") {
            if sql.contains("AND id=") && !sql.contains(&format!("AND id='{BOARD}'")) {
                return rows(vec![]);
            }
            return rows(vec![json!({
                "id": BOARD, "name": "Sales", "description": "", "created_by": "t",
                "layout_json": "", "filters_json": "", "public_token": "", "embed_enabled": "0",
                "folder_id": "", "created_at": "2026-10-10 00:00:00", "owner_id": "",
                "embed_revoked_before": "0", "embed_revoked_jti_json": "[]",
                "embed_origins_json": "[]"
            })]);
        }
        if sql.contains("system.time_zones") {
            return rows(vec![json!({ "n": 1 })]);
        }
        if sql.contains("system.columns") {
            return rows(vec![]);
        }
        ResponseTemplate::new(200).set_body_string("Ok.\n")
    }
}

struct F {
    app: TestApp,
    owner: Uuid,
    owner_cookie: String,
    _ch: MockServer,
}

async fn fixture(extra_env: &[(&str, &str)]) -> F {
    let ch = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(FakeCh)
        .mount(&ch)
        .await;
    let mut env: HashMap<String, String> = HashMap::from([
        ("CH_URL".to_owned(), ch.uri()),
        ("GOTENBERG_URL".to_owned(), "http://127.0.0.1:1".to_owned()),
        (
            "CONSOLE_INTERNAL_URL".to_owned(),
            "http://console.test:3000".to_owned(),
        ),
        (
            "RENDER_TOKEN_SECRET".to_owned(),
            "test-render-secret-not-real".to_owned(),
        ),
    ]);
    for (k, v) in extra_env {
        env.insert((*k).to_owned(), (*v).to_owned());
    }
    let app = spin_up_with_env(&env).await;
    let owner = create_principal_with_permissions(&app.pool, "dashboard:read").await;
    let owner_cookie = session_cookie_for_user(&app.pool, owner).await;
    F {
        app,
        owner,
        owner_cookie,
        _ch: ch,
    }
}

async fn call(
    f: &F,
    verb: &str,
    uri: &str,
    cookie: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let resp = f
        .app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method(verb)
                .uri(uri)
                .header("cookie", cookie)
                .header("content-type", "application/json")
                .body(Body::from(body.map(|b| b.to_string()).unwrap_or_default()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

async fn allow_domain(pool: &PgPool, domain: &str) {
    sqlx::query(
        "INSERT INTO reporting_settings (singleton, time_zone, week_start, report_allowed_domains) \
         VALUES (TRUE, 'UTC', 'monday', ARRAY[$1]) \
         ON CONFLICT (singleton) DO UPDATE SET report_allowed_domains = ARRAY[$1]",
    )
    .bind(domain)
    .execute(pool)
    .await
    .unwrap();
}

#[allow(
    clippy::needless_pass_by_value,
    reason = "every caller passes a fresh json! literal; a reference would only add `&` noise"
)]
fn body(recipients: Value) -> Value {
    json!({
        "board": BOARD,
        "schedule": { "kind": "weekly", "weekday": 1, "hour": 8 },
        "timeZone": "Asia/Jakarta",
        "attachments": { "charts": [{ "chart": "c_bar", "format": "xlsx" }] },
        "filters": [{ "column": "region", "values": ["Bali"] }],
        "skipWhenEmpty": true,
        "recipients": recipients,
    })
}

async fn user_id(pool: &PgPool, name: &str, status: &str) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO app_user (id, name, email, status) VALUES ($1, $2, $3, $4)")
        .bind(id)
        .bind(name)
        .bind(format!(
            "{}@company.example",
            name.to_lowercase().replace(' ', ".")
        ))
        .bind(status)
        .execute(pool)
        .await
        .unwrap();
    id
}

const SUBS: &str = "/api/reports/subscriptions";

#[tokio::test]
async fn a_viewer_who_may_download_subscribes_and_sees_it_listed_with_the_email_and_pdf_flags() {
    let f = fixture(&[]).await;
    let colleague = user_id(&f.app.pool, "Colleague", "active").await;
    let (status, made) = call(
        &f,
        "POST",
        SUBS,
        &f.owner_cookie,
        Some(body(json!([{ "userId": colleague.to_string() }]))),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{made}");
    assert_eq!(made["ownerId"], json!(f.owner));
    assert_eq!(
        made["schedule"],
        json!({ "kind": "weekly", "weekday": 1, "hour": 8, "minute": 0 })
    );
    assert_eq!(made["paused"], json!(false));
    assert_eq!(made["recipients"][0]["name"], "Colleague");

    let (status, list) = call(
        &f,
        "GET",
        &format!("{SUBS}?board={BOARD}"),
        &f.owner_cookie,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list["items"].as_array().unwrap().len(), 1);
    assert_eq!(list["items"][0]["canEdit"], json!(true));
    assert_eq!(
        list["emailSetUp"],
        json!(false),
        "no SMTP_HOST in this fixture"
    );
    assert_eq!(list["pdfSupported"], json!(true));
}

#[tokio::test]
async fn creating_needs_the_download_right_and_a_dashboard_the_person_can_see() {
    let f = fixture(&[]).await;
    let colleague = user_id(&f.app.pool, "Colleague", "active").await;
    let recipients = json!([{ "userId": colleague.to_string() }]);

    // A role switched off from downloading may not subscribe.
    let role: String = sqlx::query_scalar(
        "SELECT r.name FROM role r JOIN app_user_role ur ON ur.role_id = r.id WHERE ur.user_id = $1",
    )
    .bind(f.owner)
    .fetch_one(&f.app.pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO reporting_settings (singleton, time_zone, week_start, download_denied_roles) \
         VALUES (TRUE, 'UTC', 'monday', ARRAY[$1])",
    )
    .bind(&role)
    .execute(&f.app.pool)
    .await
    .unwrap();
    let (status, _) = call(
        &f,
        "POST",
        SUBS,
        &f.owner_cookie,
        Some(body(recipients.clone())),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    sqlx::query("UPDATE reporting_settings SET download_denied_roles = '{}'")
        .execute(&f.app.pool)
        .await
        .unwrap();

    // A dashboard that does not exist.
    let mut missing = body(recipients.clone());
    missing["board"] = json!("b_nope");
    let (status, _) = call(&f, "POST", SUBS, &f.owner_cookie, Some(missing)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Somebody without dashboard:read is stopped at the gate; no cookie too.
    let nobody = create_principal_with_permissions(&f.app.pool, "query:read").await;
    let cookie = session_cookie_for_user(&f.app.pool, nobody).await;
    let (status, _) = call(&f, "POST", SUBS, &cookie, Some(body(recipients.clone()))).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = call(&f, "POST", SUBS, "", Some(body(recipients))).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn outside_addresses_need_a_listed_domain_and_the_recipient_rules_hold() {
    let f = fixture(&[]).await;
    let colleague = user_id(&f.app.pool, "Colleague", "active").await;
    let gone = user_id(&f.app.pool, "Gone Person", "inactive").await;
    let post = |recipients: Value| {
        let cookie = f.owner_cookie.clone();
        let f = &f;
        async move { call(f, "POST", SUBS, &cookie, Some(body(recipients))).await }
    };

    // No domain is listed at first: no report leaves the company.
    let (status, err) = post(json!([{ "email": "press@partner.example" }])).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let text = err.to_string();
    assert!(text.contains("has not allowed"), "{text}");
    assert!(
        !text.contains("partner.example"),
        "the address is never echoed: {text}"
    );

    allow_domain(&f.app.pool, "partner.example").await;
    let (status, made) = post(json!([{ "email": "Press@Partner.Example" }])).await;
    assert_eq!(status, StatusCode::OK, "{made}");
    assert_eq!(made["recipients"][0]["email"], "press@partner.example");
    // A subdomain is another domain.
    let (status, _) = post(json!([{ "email": "a@sub.partner.example" }])).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // Not an address, both fields, nobody, an inactive user, an unknown user.
    for bad in [
        json!([{ "email": "not-an-address" }]),
        json!([{ "email": "a@partner.example", "userId": colleague.to_string() }]),
        json!([]),
        json!([{ "userId": gone.to_string() }]),
        json!([{ "userId": Uuid::new_v4().to_string() }]),
        json!([{ "userId": "nope" }]),
    ] {
        let (status, err) = post(bad.clone()).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad} {err}");
    }

    // 50 recipients are fine; 51 are refused.
    let fifty: Vec<Value> = (0..50)
        .map(|i| json!({ "email": format!("p{i}@partner.example") }))
        .collect();
    let (status, err) = post(Value::Array(fifty.clone())).await;
    assert_eq!(status, StatusCode::OK, "{err}");
    let mut fifty_one = fifty;
    fifty_one.push(json!({ "email": "extra@partner.example" }));
    let (status, err) = post(Value::Array(fifty_one)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(err.to_string().contains("at most 50"));
}

#[tokio::test]
async fn the_attachments_schedule_and_values_are_checked() {
    let f = fixture(&[]).await;
    let colleague = user_id(&f.app.pool, "Colleague", "active").await;
    let ok_recipients = json!([{ "userId": colleague.to_string() }]);
    let mutate = |edit: &dyn Fn(&mut Value)| {
        let mut b = body(ok_recipients.clone());
        edit(&mut b);
        b
    };
    for (what, b) in [
        (
            "a chart that is not on the board",
            mutate(&|b| b["attachments"]["charts"][0]["chart"] = json!("c_other")),
        ),
        (
            "a text tile",
            mutate(&|b| b["attachments"]["charts"][0]["chart"] = json!("c_text")),
        ),
        (
            "nothing attached",
            mutate(&|b| b["attachments"] = json!({ "charts": [] })),
        ),
        (
            "a bad format",
            mutate(&|b| b["attachments"]["charts"][0]["format"] = json!("pdf")),
        ),
        (
            "an unknown zone",
            mutate(&|b| b["timeZone"] = json!("Mars/Olympus")),
        ),
        (
            "day 29",
            mutate(&|b| b["schedule"] = json!({ "kind": "monthly", "monthDay": 29, "hour": 1 })),
        ),
        (
            "a filter that is not a list",
            mutate(&|b| b["filters"] = json!({ "a": 1 })),
        ),
        ("a bad grain", mutate(&|b| b["grain"] = json!("decade"))),
        (
            "an unknown field",
            mutate(&|b| b["owner"] = json!("someone")),
        ),
    ] {
        let (status, err) = call(&f, "POST", SUBS, &f.owner_cookie, Some(b)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{what}: {err}");
    }
    // The built-in schedule kinds all save.
    for schedule in [
        json!({ "kind": "hourly", "minute": 5 }),
        json!({ "kind": "daily", "hour": 7, "minute": 30 }),
        json!({ "kind": "monthly", "monthDay": "last", "hour": 9 }),
    ] {
        let (status, err) = call(
            &f,
            "POST",
            SUBS,
            &f.owner_cookie,
            Some(mutate(&|b| b["schedule"] = schedule.clone())),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{schedule}: {err}");
    }
}

#[tokio::test]
async fn without_the_pdf_service_a_pdf_is_refused_and_a_chart_file_is_not() {
    let f = fixture(&[("GOTENBERG_URL", "")]).await;
    let colleague = user_id(&f.app.pool, "Colleague", "active").await;
    let mut with_pdf = body(json!([{ "userId": colleague.to_string() }]));
    with_pdf["attachments"]["pdf"] = json!({ "paper": "a4", "orientation": "portrait" });
    let (status, err) = call(&f, "POST", SUBS, &f.owner_cookie, Some(with_pdf)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(err.to_string().contains("PDF service is not set up"));
    let (status, _) = call(
        &f,
        "POST",
        SUBS,
        &f.owner_cookie,
        Some(body(json!([{ "userId": colleague.to_string() }]))),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn only_the_owner_or_an_administrator_changes_pauses_sends_or_deletes() {
    let f = fixture(&[]).await;
    let colleague = user_id(&f.app.pool, "Colleague", "active").await;
    let recipients = json!([{ "userId": colleague.to_string() }]);
    let (_, made) = call(
        &f,
        "POST",
        SUBS,
        &f.owner_cookie,
        Some(body(recipients.clone())),
    )
    .await;
    let id = made["id"].as_str().unwrap().to_owned();
    let one = format!("{SUBS}/{id}");

    let stranger = create_principal_with_permissions(&f.app.pool, "dashboard:read").await;
    let stranger_cookie = session_cookie_for_user(&f.app.pool, stranger).await;
    let admin =
        create_principal_with_permissions(&f.app.pool, "dashboard:read, settings:write").await;
    let admin_cookie = session_cookie_for_user(&f.app.pool, admin).await;

    // A stranger sees none of it and can do none of it.
    let (_, list) = call(
        &f,
        "GET",
        &format!("{SUBS}?board={BOARD}"),
        &stranger_cookie,
        None,
    )
    .await;
    assert!(list["items"].as_array().unwrap().is_empty());
    for (verb, uri, b) in [
        ("GET", one.clone(), None),
        ("PUT", one.clone(), Some(body(recipients.clone()))),
        ("DELETE", one.clone(), None),
        ("POST", format!("{one}/send"), None),
        ("GET", format!("{one}/deliveries"), None),
    ] {
        let (status, _) = call(&f, verb, &uri, &stranger_cookie, b).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{verb} {uri}");
    }

    // An administrator sees it listed, changes it, pauses and resumes it.
    let (_, list) = call(
        &f,
        "GET",
        &format!("{SUBS}?board={BOARD}"),
        &admin_cookie,
        None,
    )
    .await;
    assert_eq!(list["items"].as_array().unwrap().len(), 1);
    let mut paused = body(recipients.clone());
    paused["paused"] = json!(true);
    paused["schedule"] = json!({ "kind": "daily", "hour": 6 });
    let (status, got) = call(&f, "PUT", &one, &admin_cookie, Some(paused)).await;
    assert_eq!(status, StatusCode::OK, "{got}");
    assert_eq!(got["paused"], json!(true));
    assert_eq!(got["schedule"]["kind"], "daily");
    assert_eq!(got["ownerId"], json!(f.owner), "the owner never changes");
    let mut resumed = body(recipients.clone());
    resumed["paused"] = json!(false);
    let (_, got) = call(&f, "PUT", &one, &f.owner_cookie, Some(resumed)).await;
    assert_eq!(got["paused"], json!(false));
    assert!(got["pausedReason"].is_null());

    // Another dashboard is a new subscription, not an edit.
    let mut moved = body(recipients);
    moved["board"] = json!("b_elsewhere");
    let (status, _) = call(&f, "PUT", &one, &f.owner_cookie, Some(moved)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let (status, _) = call(&f, "DELETE", &one, &f.owner_cookie, None).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = call(&f, "GET", &one, &f.owner_cookie, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn send_now_records_a_delivery_and_says_why_when_it_cannot_send() {
    let f = fixture(&[]).await;
    let colleague = user_id(&f.app.pool, "Colleague", "active").await;
    let (_, made) = call(
        &f,
        "POST",
        SUBS,
        &f.owner_cookie,
        Some(body(json!([{ "userId": colleague.to_string() }]))),
    )
    .await;
    let one = format!("{SUBS}/{}", made["id"].as_str().unwrap());

    // No SMTP_HOST: an honest failure, recorded as a manual send.
    let (status, delivery) = call(&f, "POST", &format!("{one}/send"), &f.owner_cookie, None).await;
    assert_eq!(status, StatusCode::OK, "{delivery}");
    assert_eq!(delivery["status"], "failed");
    assert_eq!(delivery["origin"], "manual");
    assert_eq!(
        delivery["reason"],
        "Email is not set up on this deployment."
    );
    let (_, history) = call(
        &f,
        "GET",
        &format!("{one}/deliveries"),
        &f.owner_cookie,
        None,
    )
    .await;
    assert_eq!(history["items"].as_array().unwrap().len(), 1);
    let (_, list) = call(
        &f,
        "GET",
        &format!("{SUBS}?board={BOARD}"),
        &f.owner_cookie,
        None,
    )
    .await;
    assert_eq!(list["items"][0]["lastDelivery"]["status"], "failed");
    assert_eq!(
        list["items"][0]["consecutiveFailures"],
        json!(0),
        "send now is not counted"
    );
}

#[tokio::test]
async fn send_now_to_an_unreachable_mail_server_gives_the_fixed_reason_only() {
    // Nothing listens on this port; the driver's error must not reach the record.
    let port = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let f = fixture(&[("SMTP_HOST", "127.0.0.1"), ("SMTP_PORT", &port.to_string())]).await;
    // No PDF and no skip-when-empty, so the run reaches the mail step.
    let colleague = user_id(&f.app.pool, "Colleague", "active").await;
    let mut not_skipping = body(json!([{ "userId": colleague.to_string() }]));
    not_skipping["skipWhenEmpty"] = json!(false);
    let (_, made) = call(&f, "POST", SUBS, &f.owner_cookie, Some(not_skipping)).await;
    let one = format!("{SUBS}/{}", made["id"].as_str().unwrap());
    let (_, delivery) = call(&f, "POST", &format!("{one}/send"), &f.owner_cookie, None).await;
    assert_eq!(delivery["status"], "failed", "{delivery}");
    let reason = delivery["reason"].as_str().unwrap();
    assert_eq!(
        reason,
        "The email was not sent: the mail server could not be reached."
    );
    assert!(!reason.contains("127.0.0.1"));
}

#[tokio::test]
async fn the_recipient_picker_lists_only_active_people_at_most_twenty_with_name_and_address() {
    let f = fixture(&[]).await;
    for i in 0..25 {
        user_id(&f.app.pool, &format!("Teammate {i:02}"), "active").await;
    }
    user_id(&f.app.pool, "Teammate Gone", "inactive").await;
    let (status, all) = call(
        &f,
        "GET",
        "/api/reports/recipients?q=teammate",
        &f.owner_cookie,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let items = all["items"].as_array().unwrap();
    assert_eq!(items.len(), 20);
    assert!(
        items
            .iter()
            .all(|i| i["name"].as_str().unwrap().starts_with("Teammate "))
    );
    assert!(items.iter().all(|i| i["name"] != "Teammate Gone"));
    let mut keys: Vec<&String> = items[0].as_object().unwrap().keys().collect();
    keys.sort();
    assert_eq!(keys, ["email", "id", "name"], "no roles, tenants or status");
    let (_, one) = call(
        &f,
        "GET",
        "/api/reports/recipients?q=Teammate%2007",
        &f.owner_cookie,
        None,
    )
    .await;
    assert_eq!(one["items"].as_array().unwrap().len(), 1);
    // The search is literal: a percent sign matches nothing.
    let (_, none) = call(
        &f,
        "GET",
        "/api/reports/recipients?q=%25",
        &f.owner_cookie,
        None,
    )
    .await;
    assert!(none["items"].as_array().unwrap().is_empty());
    // Somebody who may not download cannot browse people.
    let nobody = create_principal_with_permissions(&f.app.pool, "query:read").await;
    let cookie = session_cookie_for_user(&f.app.pool, nobody).await;
    let (status, _) = call(&f, "GET", "/api/reports/recipients?q=a", &cookie, None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn an_administrator_lists_the_outside_domains_and_a_bad_entry_is_refused() {
    let f = fixture(&[]).await;
    let admin =
        create_principal_with_permissions(&f.app.pool, "settings:write, dashboard:read").await;
    let cookie = session_cookie_for_user(&f.app.pool, admin).await;
    let put = |domains: Value| json!({ "timeZone": "Asia/Jakarta", "weekStart": "monday", "reportAllowedDomains": domains });
    let (status, saved) = call(
        &f,
        "PUT",
        "/api/settings/reporting",
        &cookie,
        Some(put(json!(["Partner.Example", "b.example"]))),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    assert_eq!(
        saved["reportAllowedDomains"],
        json!(["b.example", "partner.example"])
    );
    let (status, _) = call(
        &f,
        "PUT",
        "/api/settings/reporting",
        &cookie,
        Some(put(json!(["*.example.com"]))),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (_, read) = call(&f, "GET", "/api/settings/reporting", &cookie, None).await;
    assert_eq!(
        read["reportAllowedDomains"],
        json!(["b.example", "partner.example"])
    );
    // A viewer cannot change it.
    let (status, _) = call(
        &f,
        "PUT",
        "/api/settings/reporting",
        &f.owner_cookie,
        Some(put(json!(["evil.example"]))),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}
