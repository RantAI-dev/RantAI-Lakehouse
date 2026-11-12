//! `BI-19` part A (T2): public links over the real router. Real per-test
//! Postgres; `ClickHouse` is a small `wiremock` stand-in that serves board rows
//! by id and records every statement.
//!
//! The first test is the reason links left the dashboard row: a dashboard save
//! writes back the `public_token` it read, so a save that read the board before
//! "switch off" used to revive the link.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use common::{
    TestApp, create_principal_with_permissions, session_cookie_for_user, spin_up_with_env,
};
use lakehouse_store::public_links as links;
use serde_json::{Value, json};
use time::format_description::well_known::Rfc3339;
use time::{Duration, OffsetDateTime};
use tower::ServiceExt;
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, Request as MockRequest, Respond, ResponseTemplate};

const BOARD: &str = "b_pub";
const STALE: &str = "p_stale_token_from_the_old_column";

#[derive(Clone)]
struct FakeCh {
    boards: Arc<Mutex<Vec<Value>>>,
    charts: Arc<Mutex<Vec<Value>>>,
    log: Arc<Mutex<Vec<String>>>,
}

/// A `console.bi_chart` row: one tile on the shared catalog, which is what
/// `SEC-50` slice 2 T6's gate refuses for a link of another tenant.
fn chart_row(board: &str) -> Value {
    let spec = json!({
        "id": "c_one", "title": "Tile", "kind": "bar", "mart": "mart_x",
        "sql": "SELECT region, visitors FROM serving.mart_x", "x": "region", "y": "visitors"
    });
    let def = json!({
        "title": "Tile", "mart": "mart_x", "kind": "bar",
        "dimension": "region", "measures": ["visitors"]
    });
    json!({
        "id": "c_one", "board": board, "created_by": "ui", "created_at": "2026-01-01 00:00:00",
        "spec_json": json!({ "spec": spec, "def": def, "hasYear": false }).to_string(),
    })
}

fn board_row(id: &str, owner: &str) -> Value {
    json!({
        "id": id, "name": "Secret Sales Board", "description": "", "created_by": "t",
        "layout_json": "", "filters_json": "", "public_token": STALE,
        "embed_enabled": "0", "folder_id": "", "created_at": "2026-10-10 00:00:00",
        "owner_id": owner, "embed_revoked_before": "0", "embed_revoked_jti_json": "[]",
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
        let boards = self.boards.lock().unwrap().clone();
        let owner_ok = |b: &Value| {
            let owner = b["owner_id"].as_str().unwrap_or_default();
            // An owner clause names the viewer ("owner_id = ''" for nobody). BI-19
            // B2 (PR #250 CI): a read as the system viewer (the access check
            // loads the board before judging the caller's level) has no such
            // clause and sees every board, so the stand-in must not hide the
            // caller's own personal board from it; the select list names
            // `owner_id` too, so the test is for the clause, not the column.
            owner.is_empty()
                || sql.contains(&format!("owner_id = '{owner}'"))
                || !sql.contains("owner_id = ''")
        };
        if sql.starts_with("SELECT is_deleted, owner_id FROM console.bi_board") {
            return rows(
                boards
                    .iter()
                    .filter(|b| {
                        sql.contains(&format!("WHERE id = '{}'", b["id"].as_str().unwrap()))
                    })
                    .map(|b| json!({ "is_deleted": 0, "owner_id": b["owner_id"] }))
                    .collect(),
            );
        }
        if sql.contains("FROM console.bi_board_embed_event") {
            return rows(vec![]);
        }
        if sql.contains("FROM console.bi_board") && !sql.contains("FROM console.bi_chart") {
            return rows(
                boards
                    .iter()
                    .filter(|b| {
                        // A read by id names it; the list names none.
                        (!sql.contains("AND id='")
                            || sql.contains(&format!("AND id='{}'", b["id"].as_str().unwrap())))
                            && owner_ok(b)
                    })
                    .cloned()
                    .collect(),
            );
        }
        if sql.contains("FROM console.bi_chart") {
            return rows(self.charts.lock().unwrap().clone());
        }
        // The columns of the mart the tile reads.
        if sql.contains("FROM system.columns") {
            return rows(vec![
                json!({ "table": "mart_x", "name": "region", "type": "String" }),
                json!({ "table": "mart_x", "name": "visitors", "type": "UInt32" }),
            ]);
        }
        // The rows of the mart the tile reads.
        if sql.contains("FROM serving.mart_x") {
            return rows(vec![json!({ "region": "north", "visitors": 3 })]);
        }
        ResponseTemplate::new(200).set_body_string("Ok.\n")
    }
}

struct F {
    app: TestApp,
    fake: FakeCh,
    _ch: MockServer,
}

async fn fixture(boards: Vec<Value>) -> F {
    fixture_with_charts(boards, Vec::new()).await
}

/// The same fixture, with the board's tiles.
async fn fixture_with_charts(boards: Vec<Value>, charts: Vec<Value>) -> F {
    fixture_with(boards, charts, Some(common::CATALOG_TENANT)).await
}

/// The same fixture, in a deployment that names no tenant to own the shared
/// catalog (`SEC-50` slice 2 M8).
async fn fixture_without_catalog_tenant(boards: Vec<Value>, charts: Vec<Value>) -> F {
    fixture_with(boards, charts, None).await
}

async fn fixture_with(boards: Vec<Value>, charts: Vec<Value>, catalog_tenant: Option<&str>) -> F {
    let fake = FakeCh {
        boards: Arc::new(Mutex::new(boards)),
        charts: Arc::new(Mutex::new(charts)),
        log: Arc::new(Mutex::new(Vec::new())),
    };
    let ch = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(fake.clone())
        .mount(&ch)
        .await;
    let mut env = HashMap::from([("CH_URL".to_owned(), ch.uri())]);
    if let Some(tenant) = catalog_tenant {
        env.insert("CATALOG_TENANT_ID".to_owned(), tenant.to_owned());
    }
    F {
        app: spin_up_with_env(&env).await,
        fake,
        _ch: ch,
    }
}

async fn send(f: &F, request: Request<Body>) -> (StatusCode, Vec<u8>) {
    let resp = f.app.router.clone().oneshot(request).await.unwrap();
    let status = resp.status();
    (
        status,
        to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec(),
    )
}

fn json_of(bytes: &[u8]) -> Value {
    serde_json::from_slice(bytes).unwrap_or(Value::Null)
}

struct Writer {
    cookie: String,
    id: uuid::Uuid,
}

async fn writer(f: &F) -> Writer {
    let id = create_principal_with_permissions(&f.app.pool, "dashboard:write").await;
    Writer {
        cookie: session_cookie_for_user(&f.app.pool, id).await,
        id,
    }
}

/// A person who may manage dashboards and belongs to `tenant` and to no
/// other (`SEC-50` slice 2 M11).
async fn writer_in(f: &F, tenant: &str) -> Writer {
    let id = create_principal_with_permissions(&f.app.pool, "dashboard:write").await;
    sqlx::query("INSERT INTO app_user_tenant (user_id, tenant_id) VALUES ($1, $2)")
        .bind(id)
        .bind(tenant.parse::<uuid::Uuid>().unwrap())
        .execute(&f.app.pool)
        .await
        .unwrap();
    Writer {
        cookie: session_cookie_for_user(&f.app.pool, id).await,
        id,
    }
}

async fn manage(
    f: &F,
    w: &Writer,
    verb: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut req = Request::builder()
        .method(verb)
        .uri(uri)
        .header("cookie", &w.cookie);
    let body = match body {
        Some(b) => {
            req = req.header("content-type", "application/json");
            Body::from(b.to_string())
        }
        None => Body::empty(),
    };
    let (status, bytes) = send(f, req.body(body).unwrap()).await;
    (status, json_of(&bytes))
}

fn at(d: Duration) -> String {
    (OffsetDateTime::now_utc() + d).format(&Rfc3339).unwrap()
}

async fn put(f: &F, w: &Writer, extra: Value) -> (StatusCode, Value) {
    put_on(f, w, BOARD, extra).await
}

async fn put_on(f: &F, w: &Writer, board: &str, extra: Value) -> (StatusCode, Value) {
    let mut body = json!({ "board": board, "expiresAt": at(Duration::days(30)) });
    for (k, v) in extra.as_object().unwrap() {
        body[k] = v.clone();
    }
    manage(f, w, "PUT", "/api/dashboard/public-link", Some(body)).await
}

async fn open(f: &F, token: &str, ticket: Option<&str>) -> (StatusCode, Vec<u8>) {
    let mut req = Request::builder().uri(format!("/api/public/dashboard/{token}"));
    if let Some(t) = ticket {
        req = req.header("x-public-link-ticket", t);
    }
    send(f, req.body(Body::empty()).unwrap()).await
}

async fn unlock(f: &F, token: &str, password: &str) -> (StatusCode, Vec<u8>) {
    send(
        f,
        Request::builder()
            .method("POST")
            .uri(format!("/api/public/dashboard/{token}/unlock"))
            .header("content-type", "application/json")
            .body(Body::from(json!({ "password": password }).to_string()))
            .unwrap(),
    )
    .await
}

async fn one_board() -> F {
    fixture(vec![board_row(BOARD, "")]).await
}

#[tokio::test]
async fn a_dashboard_save_cannot_revive_a_link_somebody_just_switched_off() {
    let f = one_board().await;
    let w = writer(&f).await;
    let (status, made) = put(&f, &w, json!({})).await;
    assert_eq!(status, StatusCode::OK, "{made}");
    let token = made["link"]["token"].as_str().unwrap().to_owned();
    assert_eq!(open(&f, &token, None).await.0, StatusCode::OK);

    // The save reads the board (whose old column still holds a token), then
    // switches off lands in between: model it as off first, save second, which
    // is the order the old code lost.
    let (status, ended) = manage(
        &f,
        &w,
        "DELETE",
        &format!("/api/dashboard/public-link?board={BOARD}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{ended}");
    let (status, saved) = manage(
        &f,
        &w,
        "PUT",
        "/api/dashboard/boards",
        Some(json!({ "id": BOARD, "name": "Renamed", "layout": {} })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    assert!(
        f.fake
            .log
            .lock()
            .unwrap()
            .iter()
            .any(|s| s.starts_with("INSERT INTO console.bi_board ")
                || s.starts_with("INSERT INTO console.bi_board (")),
        "the save really wrote a board row"
    );

    assert_eq!(open(&f, &token, None).await.0, StatusCode::NOT_FOUND);
    assert_eq!(open(&f, STALE, None).await.0, StatusCode::NOT_FOUND);
    assert!(
        links::get_for_board(&f.app.pool, BOARD)
            .await
            .unwrap()
            .is_none()
    );

    // And the other way: a save does not change a live link either.
    let (_, again) = put(&f, &w, json!({})).await;
    let live = again["link"]["token"].as_str().unwrap().to_owned();
    manage(
        &f,
        &w,
        "PUT",
        "/api/dashboard/boards",
        Some(json!({ "id": BOARD, "name": "Again" })),
    )
    .await;
    assert_eq!(
        links::get_for_board(&f.app.pool, BOARD)
            .await
            .unwrap()
            .unwrap()
            .token,
        live
    );
    assert_eq!(open(&f, &live, None).await.0, StatusCode::OK);
}

#[tokio::test]
async fn an_old_console_asking_for_a_public_link_on_a_save_is_told_not_fooled() {
    let f = one_board().await;
    let w = writer(&f).await;
    let (status, body) = manage(
        &f,
        &w,
        "PUT",
        "/api/dashboard/boards",
        Some(json!({ "id": BOARD, "public": true })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(
        links::get_for_board(&f.app.pool, BOARD)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn unknown_expired_switched_off_trashed_and_personal_links_answer_identically() {
    let f = fixture(vec![
        board_row(BOARD, ""),
        board_row("b_personal", "someone"),
    ])
    .await;
    let w = writer(&f).await;
    let (_, made) = put(&f, &w, json!({})).await;
    let live = made["link"]["token"].as_str().unwrap().to_owned();
    links::replace(
        &f.app.pool,
        "b_gone",
        "p_trashed_board_link",
        OffsetDateTime::now_utc() + Duration::days(5),
        None,
        None,
        None,
    )
    .await
    .unwrap();
    links::replace(
        &f.app.pool,
        "b_personal",
        "p_personal_board_lnk",
        OffsetDateTime::now_utc() + Duration::days(5),
        None,
        None,
        None,
    )
    .await
    .unwrap();
    links::replace(
        &f.app.pool,
        "b_old",
        "p_expired_board_link",
        OffsetDateTime::now_utc() + Duration::days(5),
        None,
        None,
        None,
    )
    .await
    .unwrap();
    sqlx::query("UPDATE dashboard_public_link SET expires_at = now() - interval '1 second' WHERE board_id = 'b_old'")
        .execute(&f.app.pool).await.unwrap();

    let mut answers = Vec::new();
    for token in [
        "p_never_existed_at_all",
        "p_expired_board_link",
        "p_trashed_board_link",
        "p_personal_board_lnk",
    ] {
        answers.push(open(&f, token, None).await);
    }
    // Switched off by the administrator: the live link stops, nothing is deleted.
    sqlx::query("INSERT INTO reporting_settings (singleton, time_zone, week_start, public_links_enabled) VALUES (TRUE, 'UTC', 'monday', FALSE) ON CONFLICT (singleton) DO UPDATE SET public_links_enabled = FALSE")
        .execute(&f.app.pool).await.unwrap();
    answers.push(open(&f, &live, None).await);
    for (status, body) in &answers {
        assert_eq!(*status, StatusCode::NOT_FOUND);
        assert_eq!(body, &answers[0].1, "byte for byte the same");
    }
    assert_eq!(json_of(&answers[0].1), json!({ "error": "not_found" }));
    assert!(
        links::get_for_board(&f.app.pool, BOARD)
            .await
            .unwrap()
            .is_some()
    );
    // Switching it back brings the link back.
    sqlx::query("UPDATE reporting_settings SET public_links_enabled = TRUE")
        .execute(&f.app.pool)
        .await
        .unwrap();
    assert_eq!(open(&f, &live, None).await.0, StatusCode::OK);
}

#[tokio::test]
async fn a_password_link_needs_a_ticket_and_a_new_password_ends_the_old_tickets() {
    let f = one_board().await;
    let w = writer(&f).await;
    let (status, made) = put(&f, &w, json!({ "password": "correct horse" })).await;
    assert_eq!(status, StatusCode::OK, "{made}");
    assert_eq!(made["link"]["hasPassword"], true);
    let text = made.to_string();
    assert!(
        !text.contains("correct horse") && !text.contains("argon2"),
        "{text}"
    );
    let token = made["link"]["token"].as_str().unwrap().to_owned();

    let (status, body) = open(&f, &token, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(json_of(&body), json!({ "error": "password_required" }));
    assert!(!String::from_utf8_lossy(&body).contains("Secret Sales Board"));
    assert_eq!(
        open(&f, &token, Some("not-a-ticket")).await.0,
        StatusCode::UNAUTHORIZED
    );

    let (status, body) = unlock(&f, &token, "wrong password").await;
    assert_eq!(
        (status, json_of(&body)),
        (
            StatusCode::UNAUTHORIZED,
            json!({ "error": "wrong_password" })
        )
    );
    let (status, body) = unlock(&f, &token, "correct horse").await;
    assert_eq!(status, StatusCode::OK);
    let ticket = json_of(&body)["ticket"].as_str().unwrap().to_owned();
    let (status, body) = open(&f, &token, Some(&ticket)).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    // Only the ticket's hash is stored.
    let (stored,): (String,) =
        sqlx::query_as("SELECT ticket_hash FROM dashboard_public_link_ticket")
            .fetch_one(&f.app.pool)
            .await
            .unwrap();
    assert_ne!(stored, ticket);

    // A new password: same token, old ticket refused.
    let (status, changed) = put(&f, &w, json!({ "password": "another one!" })).await;
    assert_eq!(status, StatusCode::OK, "{changed}");
    assert_eq!(changed["link"]["token"], json!(token));
    assert_eq!(
        open(&f, &token, Some(&ticket)).await.0,
        StatusCode::UNAUTHORIZED
    );
    // Removing the password opens the link to everyone.
    let (_, removed) = put(&f, &w, json!({ "password": null })).await;
    assert_eq!(removed["link"]["hasPassword"], false);
    assert_eq!(open(&f, &token, None).await.0, StatusCode::OK);
    // A link without a password has nothing to unlock.
    assert_eq!(unlock(&f, &token, "x").await.0, StatusCode::BAD_REQUEST);
    // Absent keeps what is there.
    put(&f, &w, json!({ "password": "kept password" })).await;
    let (_, kept) = put(&f, &w, json!({})).await;
    assert_eq!(kept["link"]["hasPassword"], true);
}

#[tokio::test]
async fn five_wrong_passwords_lock_the_link_and_a_locked_link_says_so() {
    let f = one_board().await;
    let w = writer(&f).await;
    let (_, made) = put(&f, &w, json!({ "password": "correct horse" })).await;
    let token = made["link"]["token"].as_str().unwrap().to_owned();
    let wrong = json!({ "error": "wrong_password" });
    let locked = json!({ "error": "locked" });
    // Four wrong tries read as wrong passwords.
    for _ in 0..4 {
        let (status, body) = unlock(&f, &token, "nope nope nope").await;
        assert_eq!(
            (status, json_of(&body)),
            (StatusCode::UNAUTHORIZED, wrong.clone())
        );
    }
    // BI-19 part A QA fix P2: the fifth wrong try locks the link and says so,
    // with a fixed code and nothing else (no time left).
    let (status, fifth) = unlock(&f, &token, "nope nope nope").await;
    assert_eq!(
        (status, json_of(&fifth)),
        (StatusCode::UNAUTHORIZED, locked.clone())
    );
    // The sixth try has the right password and is still refused, as locked,
    // byte for byte the same.
    assert_eq!(unlock(&f, &token, "correct horse").await, (status, fifth));
    // The lock passes.
    sqlx::query("UPDATE login_throttle SET locked_until = now() - interval '1 second'")
        .execute(&f.app.pool)
        .await
        .unwrap();
    assert_eq!(unlock(&f, &token, "correct horse").await.0, StatusCode::OK);
    // A5 is unchanged: a made-up link is still the one 404.
    let (status, body) = unlock(&f, "p_made_up_token_x", "x").await;
    assert_eq!(
        (status, json_of(&body)),
        (StatusCode::NOT_FOUND, json!({ "error": "not_found" }))
    );
    let (locks,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM audit_event WHERE action = 'dashboard.public_link_locked'",
    )
    .fetch_one(&f.app.pool)
    .await
    .unwrap();
    assert_eq!(locks, 1);
    // QA LH-SEC-2-06 (SEC-70): a visitor locking a public link is nobody the
    // product signed in, so the row has no principal and names the link.
    let (actor_label, principal_kind, principal_id): (
        Option<String>,
        Option<String>,
        Option<String>,
    ) = sqlx::query_as(
        "SELECT actor_label, principal_kind, principal_id FROM audit_event \
         WHERE action = 'dashboard.public_link_locked'",
    )
    .fetch_one(&f.app.pool)
    .await
    .unwrap();
    assert_eq!(
        actor_label.as_deref(),
        Some(lakehouse_store::audit::ACTOR_PUBLIC_LINK)
    );
    assert_eq!(principal_kind, None);
    assert_eq!(principal_id, None);
}

#[tokio::test]
async fn expiry_and_password_limits_and_boards_that_cannot_be_shared_are_refused() {
    let f = fixture(vec![board_row(BOARD, ""), board_row("b_mine", "")]).await;
    let w = writer(&f).await;
    for bad in [
        at(Duration::minutes(10)),
        at(Duration::days(400)),
        "soon".to_owned(),
    ] {
        let (status, _) = put(&f, &w, json!({ "expiresAt": bad })).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }
    assert_eq!(
        put(&f, &w, json!({ "expiresAt": at(Duration::days(364)) }))
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        put(&f, &w, json!({ "password": "short" })).await.0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        put(&f, &w, json!({ "password": "x".repeat(129) })).await.0,
        StatusCode::BAD_REQUEST
    );
    let (status, _) = manage(
        &f,
        &w,
        "PUT",
        "/api/dashboard/public-link",
        Some(json!({ "board": "default", "expiresAt": at(Duration::days(3)) })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = manage(
        &f,
        &w,
        "PUT",
        "/api/dashboard/public-link",
        Some(json!({ "board": "b_nope", "expiresAt": at(Duration::days(3)) })),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    // The caller's own personal board: visible, but never shareable.
    f.fake
        .boards
        .lock()
        .unwrap()
        .push(board_row("b_draft", &w.id.to_string()));
    let (status, body) = manage(
        &f,
        &w,
        "PUT",
        "/api/dashboard/public-link",
        Some(json!({ "board": "b_draft", "expiresAt": at(Duration::days(3)) })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(body["error"].as_str().unwrap().contains("personal space"));
    // Switched off by an administrator: no new link, but ending one still works.
    sqlx::query("INSERT INTO reporting_settings (singleton, time_zone, week_start, public_links_enabled) VALUES (TRUE, 'UTC', 'monday', FALSE) ON CONFLICT (singleton) DO UPDATE SET public_links_enabled = FALSE")
        .execute(&f.app.pool).await.unwrap();
    assert_eq!(put(&f, &w, json!({})).await.0, StatusCode::BAD_REQUEST);
    let (status, state) = manage(
        &f,
        &w,
        "GET",
        &format!("/api/dashboard/public-link?board={BOARD}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(state["enabled"], false);
    let (status, _) = manage(
        &f,
        &w,
        "DELETE",
        &format!("/api/dashboard/public-link?board={BOARD}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        links::get_for_board(&f.app.pool, BOARD)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn a_new_link_ends_the_old_one_and_every_change_is_audited_without_secrets() {
    let f = one_board().await;
    let w = writer(&f).await;
    let (_, first) = put(&f, &w, json!({ "password": "correct horse" })).await;
    let old = first["link"]["token"].as_str().unwrap().to_owned();
    let (_, second) = put(&f, &w, json!({ "newLink": true })).await;
    let new = second["link"]["token"].as_str().unwrap().to_owned();
    assert_ne!(old, new);
    assert_eq!(open(&f, &old, None).await.0, StatusCode::NOT_FOUND);
    assert_eq!(
        second["link"]["hasPassword"], false,
        "a new link starts without a password"
    );
    manage(
        &f,
        &w,
        "DELETE",
        &format!("/api/dashboard/public-link?board={BOARD}"),
        None,
    )
    .await;
    let rows: Vec<(String, Value)> = sqlx::query_as("SELECT action, args FROM audit_event WHERE action LIKE 'dashboard.public_link_%' ORDER BY at")
        .fetch_all(&f.app.pool).await.unwrap();
    let actions: Vec<&str> = rows.iter().map(|(a, _)| a.as_str()).collect();
    assert_eq!(
        actions,
        [
            "dashboard.public_link_create",
            "dashboard.public_link_create",
            "dashboard.public_link_delete"
        ]
    );
    let all = format!("{rows:?}");
    for secret in [old.as_str(), new.as_str(), "correct horse", "argon2"] {
        assert!(!all.contains(secret), "{secret} is in the audit record");
    }
}

#[tokio::test]
async fn a_change_that_cannot_be_audited_is_not_made() {
    let f = one_board().await;
    let w = writer(&f).await;
    sqlx::query("ALTER TABLE audit_event RENAME TO audit_event_away")
        .execute(&f.app.pool)
        .await
        .unwrap();
    let (status, _) = put(&f, &w, json!({})).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(
        links::get_for_board(&f.app.pool, BOARD)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn the_dashboard_list_marks_only_boards_with_a_live_link() {
    let f = fixture(vec![board_row(BOARD, ""), board_row("b_other", "")]).await;
    let w = writer(&f).await;
    let reader = create_principal_with_permissions(&f.app.pool, "dashboard:read").await;
    let cookie = session_cookie_for_user(&f.app.pool, reader).await;
    let (_, made) = put(&f, &w, json!({})).await;
    let req = Request::builder()
        .uri("/api/dashboard/boards")
        .header("cookie", &cookie)
        .body(Body::empty())
        .unwrap();
    let (status, body) = send(&f, req).await;
    assert_eq!(status, StatusCode::OK);
    let boards = json_of(&body)["boards"].as_array().unwrap().clone();
    let by = |id: &str| boards.iter().find(|b| b["id"] == id).unwrap().clone();
    assert_eq!(by(BOARD)["publicToken"], made["link"]["token"]);
    assert!(
        by("b_other").get("publicToken").is_none(),
        "the stale column is never shown"
    );
}

/// `BI-19` part A review fix (BLOCKER) R1: twenty guesses at once are all
/// counted before any is checked, all answer as a wrong password, and the link
/// is locked afterwards even for the right password.
#[tokio::test]
async fn twenty_simultaneous_guesses_are_all_counted_and_all_refused() {
    let f = one_board().await;
    let w = writer(&f).await;
    let (_, made) = put(&f, &w, json!({ "password": "correct horse" })).await;
    let token = made["link"]["token"].as_str().unwrap().to_owned();
    let guesses = (0..20).map(|_| unlock(&f, &token, "wrong guess number"));
    let answers = futures::future::join_all(guesses).await;
    assert_eq!(answers.len(), 20);
    // Four read as wrong passwords at most (the order they arrive in is not
    // fixed); the rest, the fifth included, say the link is locked.
    let mut locked = 0;
    for (status, body) in &answers {
        assert_eq!(*status, StatusCode::UNAUTHORIZED);
        match json_of(body)["error"].as_str() {
            Some("locked") => locked += 1,
            Some("wrong_password") => {}
            other => panic!("unexpected answer {other:?}"),
        }
    }
    assert!(locked >= 16, "{locked} of 20 said locked");
    let (counted,): (i32,) = sqlx::query_as("SELECT failures FROM login_throttle")
        .fetch_one(&f.app.pool)
        .await
        .unwrap();
    assert_eq!(counted, 20, "every guess was counted before it was checked");
    assert_eq!(
        unlock(&f, &token, "correct horse").await.0,
        StatusCode::UNAUTHORIZED
    );
}

// ── SEC-50 slice 2 T6: the link acts as the tenant that made it ─────────

/// Another tenant of the same deployment, whose members may not read the
/// shared catalog.
const OTHER_TENANT: &str = "11111111-1111-4111-8111-000000000002";
/// The board the second link is made for.
const OTHER_BOARD: &str = "b_pub2";
/// `SEC-20`'s refusal of a table the caller's tenant may not read, byte for
/// byte.
const REFUSAL: &str = "Table not found or not available to your organisation: serving.mart_x";

/// `SEC-50` slice 2 D4: a link made by a member of the tenant that owns the
/// shared catalog reads it; a link made by a member of another tenant does
/// not.
#[tokio::test]
async fn a_link_reads_as_the_tenant_that_made_it() {
    let f = fixture_with_charts(
        vec![board_row(BOARD, ""), board_row(OTHER_BOARD, "")],
        vec![chart_row(BOARD), chart_row(OTHER_BOARD)],
    )
    .await;
    let owner = writer_in(&f, common::CATALOG_TENANT).await;
    let other = writer_in(&f, OTHER_TENANT).await;

    // The catalog's own tenant: the link reads the shared catalog.
    let (status, body) = put(&f, &owner, json!({})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let token = body["link"]["token"].as_str().unwrap().to_owned();
    let (status, bytes) = open(&f, &token, None).await;
    assert_eq!(status, StatusCode::OK);
    let view = json_of(&bytes);
    assert!(
        view["results"]["c_one"].get("error").is_none(),
        "the link reads the catalog: {view}"
    );
    assert_eq!(
        view["results"]["c_one"]["rows"].as_array().map(Vec::len),
        Some(1),
        "{view}"
    );

    // Another tenant: the same board, the same link, refused.
    let (status, body) = put_on(&f, &other, OTHER_BOARD, json!({})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let token = body["link"]["token"].as_str().unwrap().to_owned();
    let (status, bytes) = open(&f, &token, None).await;
    assert_eq!(status, StatusCode::OK);
    let view = json_of(&bytes);
    assert_eq!(view["results"]["c_one"]["error"], REFUSAL, "{view}");
    assert!(
        view["results"]["c_one"].get("rows").is_none(),
        "the refused tile has no rows: {view}"
    );
}

/// `SEC-50` slice 2 D4/M8: a creator who later leaves the tenant the link
/// names may not act as it, so the link is refused.
#[tokio::test]
async fn a_link_whose_creator_left_its_tenant_is_refused() {
    let f = fixture_with_charts(vec![board_row(BOARD, "")], vec![chart_row(BOARD)]).await;
    let other = writer_in(&f, OTHER_TENANT).await;
    let (status, body) = put(&f, &other, json!({})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let token = body["link"]["token"].as_str().unwrap().to_owned();

    // The creator leaves the tenant the link was made in.
    sqlx::query("DELETE FROM app_user_tenant WHERE user_id = $1")
        .bind(other.id)
        .execute(&f.app.pool)
        .await
        .unwrap();

    let (status, bytes) = open(&f, &token, None).await;
    assert_eq!(status, StatusCode::OK);
    let view = json_of(&bytes);
    assert!(
        view["results"]["c_one"].get("error").is_some(),
        "the creator may not act as the tenant any more: {view}"
    );
    assert!(
        view["results"]["c_one"].get("rows").is_none(),
        "the refused tile has no rows: {view}"
    );
}

/// `SEC-50` slice 2 M8: a link with no creator (the links that predate the
/// column, and the links a service identity made) has no owner: it reads
/// everything in a deployment with one tenant, and nothing in a deployment
/// with several.
#[tokio::test]
async fn a_link_without_a_creator_reads_nothing_in_a_deployment_with_several_tenants() {
    let f =
        fixture_without_catalog_tenant(vec![board_row(BOARD, "")], vec![chart_row(BOARD)]).await;
    links::replace(
        &f.app.pool,
        BOARD,
        "p_legacy_link",
        OffsetDateTime::now_utc() + Duration::days(30),
        None,
        None,
        None,
    )
    .await
    .unwrap();

    let (status, bytes) = open(&f, "p_legacy_link", None).await;
    assert_eq!(status, StatusCode::OK);
    let view = json_of(&bytes);
    assert_eq!(view["results"]["c_one"]["error"], REFUSAL, "{view}");
    assert!(
        view["results"]["c_one"].get("rows").is_none(),
        "the refused tile has no rows: {view}"
    );
}

/// `SEC-50` slice 2 M8: the same link in a deployment with one tenant reads
/// everything — there is no other tenant's table to reach.
#[tokio::test]
async fn a_link_without_a_creator_reads_everything_in_a_deployment_with_one_tenant() {
    let f =
        fixture_without_catalog_tenant(vec![board_row(BOARD, "")], vec![chart_row(BOARD)]).await;
    common::leave_no_seeded_tenants(&f.app.pool).await;
    links::replace(
        &f.app.pool,
        BOARD,
        "p_legacy_link",
        OffsetDateTime::now_utc() + Duration::days(30),
        None,
        None,
        None,
    )
    .await
    .unwrap();

    let (status, bytes) = open(&f, "p_legacy_link", None).await;
    assert_eq!(status, StatusCode::OK);
    let view = json_of(&bytes);
    assert!(
        view["results"]["c_one"].get("error").is_none(),
        "one tenant in the deployment: nothing changes: {view}"
    );
    assert_eq!(
        view["results"]["c_one"]["rows"].as_array().map(Vec::len),
        Some(1),
        "{view}"
    );
}

/// `SEC-50` slice 2 D4: a link with no tenant of its own (the links that
/// predate the column) reads as its creator's first tenant.
#[tokio::test]
async fn a_link_without_a_tenant_reads_as_its_creator() {
    let f = fixture_with_charts(vec![board_row(BOARD, "")], vec![chart_row(BOARD)]).await;
    let owner = writer_in(&f, common::CATALOG_TENANT).await;
    links::replace(
        &f.app.pool,
        BOARD,
        "p_no_tenant_link",
        OffsetDateTime::now_utc() + Duration::days(30),
        None,
        Some(owner.id),
        None,
    )
    .await
    .unwrap();

    let (status, bytes) = open(&f, "p_no_tenant_link", None).await;
    assert_eq!(status, StatusCode::OK);
    let view = json_of(&bytes);
    assert!(
        view["results"]["c_one"].get("error").is_none(),
        "the link reads as its creator: {view}"
    );
    assert_eq!(
        view["results"]["c_one"]["rows"].as_array().map(Vec::len),
        Some(1),
        "{view}"
    );
}
