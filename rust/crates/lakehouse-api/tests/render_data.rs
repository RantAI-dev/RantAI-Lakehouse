//! `POST /api/render/data` — the data behind the PDF's print view (`BI-1`, T3,
//! plan decisions D2 and D3): token-only, read as the person the token names
//! (their masks, their personal space), every tab, and nothing a session
//! cookie or an embed token can change.
//!
//! The `ClickHouse` here is a recording fake: two boards (one private to
//! `OWNER_ID`), their charts, and the column list. The statements the fake saw
//! show whose roles the tiles were read with.

#![allow(clippy::unwrap_used, clippy::expect_used)]
#![allow(
    clippy::cast_precision_loss,
    reason = "test timestamps are Unix seconds, exact in an f64"
)]

mod common;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use common::{TestApp, session_cookie_for_user, spin_up_with_env};
use lakehouse_embed::{
    EmbedClaims, EmbedResource, RenderClaims, sign_embed, sign_render, unix_now,
};
use lakehouse_store::governance::{self, CreatePolicyInput};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, Request as MockRequest, Respond, ResponseTemplate};

const SECRET: &str = "test-render-secret-not-real";
const EMBED_SECRET: &str = "test-embed-secret-not-real";
const PUBLIC_BOARD: &str = "b_public";
const PRIVATE_BOARD: &str = "b_private";

/// A recording `ClickHouse`: boards, charts and columns as the stores ask.
#[derive(Clone)]
struct FakeCh {
    owner: String,
    log: Arc<Mutex<Vec<String>>>,
}

fn chart_row(id: &str, board: &str) -> Value {
    let spec = json!({
        "id": id, "title": format!("Tile {id}"), "kind": "bar", "mart": "mart_x",
        "sql": "SELECT email, region FROM serving.mart_x", "x": "region", "y": "email"
    });
    let def = json!({
        "title": format!("Tile {id}"), "mart": "mart_x", "kind": "bar",
        "dimension": "region", "measures": ["email"]
    });
    json!({
        "id": id, "board": board, "created_by": "ui", "created_at": "2026-01-01 00:00:00",
        "spec_json": json!({ "spec": spec, "def": def, "hasYear": false }).to_string(),
    })
}

fn board_row(id: &str, owner: &str, tabs: &str) -> Value {
    json!({
        "id": id, "name": format!("Board {id}"), "description": "", "created_by": "t",
        "layout_json": "", "filters_json": "", "public_token": "", "embed_enabled": "0",
        "folder_id": "", "created_at": "2026-10-10 00:00:00", "owner_id": owner,
        "tabs_json": tabs, "embed_revoked_before": "0", "embed_revoked_jti_json": "[]",
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
        if sql.contains("FROM console.bi_chart") {
            let mut charts = vec![
                chart_row("c_one", PUBLIC_BOARD),
                chart_row("c_two", PUBLIC_BOARD),
            ];
            // The owner's own personal board's chart is in the list only when
            // the statement does not leave that owner's boards out.
            if sql.contains(&format!("owner_id != '{}'", self.owner)) {
                charts.push(chart_row("c_mine", PRIVATE_BOARD));
            }
            return rows(charts);
        }
        if sql.contains("FROM console.bi_board") {
            let mut boards = vec![board_row(
                PUBLIC_BOARD,
                "",
                r#"[{"id":"t1","title":"First"},{"id":"t2","title":"Second"}]"#,
            )];
            if sql.contains(&format!("owner_id = '{}'", self.owner)) {
                boards.push(board_row(PRIVATE_BOARD, &self.owner, ""));
            }
            let wanted: Vec<Value> = boards
                .into_iter()
                .filter(|b| {
                    sql.contains(&format!("id='{}'", b["id"].as_str().unwrap()))
                        || !sql.contains("AND id='")
                })
                .collect();
            return rows(wanted);
        }
        if sql.contains("system.columns") {
            return rows(
                ["email", "region"]
                    .iter()
                    .map(|n| json!({"table": "mart_x", "name": n, "type": "String"}))
                    .collect(),
            );
        }
        if sql.contains("FROM serving.mart_x") || sql.contains("replaceRegexpOne") {
            return rows(vec![json!({"email": "a@x", "region": "north"})]);
        }
        ResponseTemplate::new(200).set_body_string("Ok.\n")
    }
}

struct Fixture {
    app: TestApp,
    fake: FakeCh,
    owner: Uuid,
    other: Uuid,
    _ch: MockServer,
}

async fn user(pool: &PgPool, role: &str) -> Uuid {
    let role_id = Uuid::new_v4();
    sqlx::query("INSERT INTO role (id, name, permissions, description) VALUES ($1, $2, 'dashboard:read', 'test')")
        .bind(role_id)
        .bind(role)
        .execute(pool)
        .await
        .unwrap();
    let user_id = Uuid::new_v4();
    sqlx::query("INSERT INTO app_user (id, name, email, status) VALUES ($1, $2, $3, 'active')")
        .bind(user_id)
        .bind(format!("User of {role}"))
        .bind(format!("{role}-{user_id}@test.invalid").replace(' ', "-"))
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO app_user_role (user_id, role_id) VALUES ($1, $2)")
        .bind(user_id)
        .bind(role_id)
        .execute(pool)
        .await
        .unwrap();
    user_id
}

async fn fixture(secret: Option<&str>) -> Fixture {
    // The owner's id is needed by the fake before the app exists, so the fake
    // is built first with a placeholder and the id is fixed up through `owner`.
    let owner_id = Uuid::new_v4();
    let fake = FakeCh {
        owner: owner_id.to_string(),
        log: Arc::new(Mutex::new(Vec::new())),
    };
    let ch = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(fake.clone())
        .mount(&ch)
        .await;
    let mut env: HashMap<String, String> = HashMap::from([("CH_URL".to_owned(), ch.uri())]);
    if let Some(secret) = secret {
        env.insert("RENDER_TOKEN_SECRET".to_owned(), secret.to_owned());
        // The embedding customer's secret is a different one (R1).
        env.insert("EMBED_SECRET".to_owned(), EMBED_SECRET.to_owned());
    }
    let app = spin_up_with_env(&env).await;
    // The owner row is created with the id the fake already knows.
    let role_id = Uuid::new_v4();
    sqlx::query("INSERT INTO role (id, name, permissions, description) VALUES ($1, 'Owner Role', 'dashboard:read', 'test')")
        .bind(role_id)
        .execute(&app.pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO app_user (id, name, email, status) VALUES ($1, 'Owner', 'owner@test.invalid', 'active')")
        .bind(owner_id)
        .execute(&app.pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO app_user_role (user_id, role_id) VALUES ($1, $2)")
        .bind(owner_id)
        .bind(role_id)
        .execute(&app.pool)
        .await
        .unwrap();
    let other = user(&app.pool, "Other Role").await;
    Fixture {
        app,
        fake,
        owner: owner_id,
        other,
        _ch: ch,
    }
}

fn render_token(user: Uuid, board: &str, age: f64, lifetime: f64) -> String {
    let iat = (unix_now() - age).floor();
    sign_render(
        &RenderClaims {
            aud: "render".to_owned(),
            board: board.to_owned(),
            principal: user.to_string(),
            filters: Some(r#"[{"column":"region","values":["north"]}]"#.to_owned()),
            params: None,
            grain: None,
            paper: "a4".to_owned(),
            orientation: "portrait".to_owned(),
            exp: iat + lifetime,
            iat,
            jti: Uuid::new_v4().to_string(),
        },
        SECRET,
    )
}

async fn post(app: &TestApp, body: &Value, cookie: Option<&str>) -> (StatusCode, Value) {
    let mut req = Request::builder()
        .method("POST")
        .uri("/api/render/data")
        .header("content-type", "application/json");
    if let Some(cookie) = cookie {
        req = req.header("cookie", cookie);
    }
    let resp = app
        .router
        .clone()
        .oneshot(req.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

#[tokio::test]
async fn a_valid_token_gets_the_dashboard_payload_with_every_tab_and_the_first_page_facts() {
    let f = fixture(Some(SECRET)).await;
    let token = render_token(f.owner, PUBLIC_BOARD, 5.0, 120.0);
    let (status, body) = post(&f.app, &json!({ "token": token }), None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    // Every tab and every tile of the board, in one payload.
    assert_eq!(body["tabs"].as_array().unwrap().len(), 2);
    let ids: Vec<&str> = body["charts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["c_one", "c_two"]);
    assert!(body["results"]["c_one"].is_object(), "{body}");
    // The filters of the token are the filters of the payload.
    assert_eq!(body["filters"][0]["column"], "region");
    assert_eq!(body["render"]["requestedBy"], "Owner");
    assert_eq!(body["render"]["paper"], "a4");
    assert!(body["render"]["issuedAt"].as_f64().is_some());
    assert_eq!(body["canDownload"], true);
}

#[tokio::test]
async fn the_tiles_are_read_with_the_masks_of_the_person_not_the_embed_viewer() {
    let f = fixture(Some(SECRET)).await;
    governance::create_policy(
        &f.app.pool,
        &CreatePolicyInput {
            name: "render-mask".to_owned(),
            kind: "Row filter".to_owned(),
            subjects: "Owner Role".to_owned(),
            resources: "serving.mart_x".to_owned(),
            effect: "Permit with obligation".to_owned(),
            conditions: Some(
                r#"{"roles":["Owner Role"],"table":"serving.mart_x","mask":["email"]}"#.to_owned(),
            ),
            activate: true,
            owner: None,
        },
    )
    .await
    .unwrap();
    let before = f.fake.log.lock().unwrap().len();
    let owner_token = render_token(f.owner, PUBLIC_BOARD, 5.0, 120.0);
    let (status, _) = post(&f.app, &json!({ "token": owner_token }), None).await;
    assert_eq!(status, StatusCode::OK);
    let owner_tiles: Vec<String> = f.fake.log.lock().unwrap()[before..]
        .iter()
        .filter(|s| s.contains("serving.mart_x") && !s.contains("system."))
        .cloned()
        .collect();
    assert!(!owner_tiles.is_empty());
    assert!(
        owner_tiles
            .iter()
            .all(|s| s.contains("replaceRegexpOne(toString(`email`)")),
        "the owner's mask is in every tile statement: {owner_tiles:?}"
    );

    // Another person has a role the policy does not name: no mask for them.
    let before = f.fake.log.lock().unwrap().len();
    let other_token = render_token(f.other, PUBLIC_BOARD, 5.0, 120.0);
    let (status, _) = post(&f.app, &json!({ "token": other_token }), None).await;
    assert_eq!(status, StatusCode::OK);
    let other_tiles: Vec<String> = f.fake.log.lock().unwrap()[before..]
        .iter()
        .filter(|s| s.contains("serving.mart_x") && !s.contains("system."))
        .cloned()
        .collect();
    assert!(!other_tiles.is_empty());
    assert!(
        other_tiles.iter().all(|s| !s.contains("replaceRegexpOne")),
        "{other_tiles:?}"
    );
}

#[tokio::test]
async fn a_personal_board_prints_for_its_owner_only() {
    let f = fixture(Some(SECRET)).await;
    let (status, mine) = post(
        &f.app,
        &json!({ "token": render_token(f.owner, PRIVATE_BOARD, 5.0, 120.0) }),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{mine}");
    assert_eq!(mine["charts"].as_array().unwrap().len(), 1, "{mine}");
    let (status, theirs) = post(
        &f.app,
        &json!({ "token": render_token(f.other, PRIVATE_BOARD, 5.0, 120.0) }),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{theirs}");
    assert!(
        theirs["charts"].as_array().unwrap().is_empty(),
        "another person's personal board has no tiles for them: {theirs}"
    );
}

#[tokio::test]
async fn a_session_cookie_is_never_read_and_the_body_names_nothing_but_the_token() {
    let f = fixture(Some(SECRET)).await;
    let cookie = session_cookie_for_user(&f.app.pool, f.owner).await;
    // A signed-in person with no token gets nothing.
    let (status, body) = post(&f.app, &json!({}), Some(&cookie)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let (status, body) = post(&f.app, &json!({ "token": "nope" }), Some(&cookie)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    // A board named beside the token is refused, not followed.
    let token = render_token(f.owner, PUBLIC_BOARD, 5.0, 120.0);
    let (status, _) = post(
        &f.app,
        &json!({ "token": token, "board": PRIVATE_BOARD }),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn an_embed_token_an_expired_token_and_a_foreign_secret_are_refused_with_one_sentence() {
    let f = fixture(Some(SECRET)).await;
    let embed = sign_embed(
        &EmbedClaims {
            resource: Some(EmbedResource {
                chart: None,
                dashboard: Some(PUBLIC_BOARD.to_owned()),
            }),
            params: None,
            exp: Some(unix_now() + 60.0),
            iat: Some(unix_now() - 5.0),
            jti: None,
        },
        EMBED_SECRET,
    );
    let expired = render_token(f.owner, PUBLIC_BOARD, 600.0, 120.0);
    let too_long = render_token(f.owner, PUBLIC_BOARD, 5.0, 3600.0);
    for token in [embed, expired, too_long] {
        let (status, body) = post(&f.app, &json!({ "token": token }), None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
        assert_eq!(body["error"], "render token is invalid or expired");
    }
}

#[tokio::test]
async fn a_disabled_or_deleted_person_is_refused() {
    let f = fixture(Some(SECRET)).await;
    sqlx::query("UPDATE app_user SET status = 'inactive' WHERE id = $1")
        .bind(f.other)
        .execute(&f.app.pool)
        .await
        .unwrap();
    let (status, _) = post(
        &f.app,
        &json!({ "token": render_token(f.other, PUBLIC_BOARD, 5.0, 120.0) }),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let ghost = Uuid::new_v4();
    let (status, _) = post(
        &f.app,
        &json!({ "token": render_token(ghost, PUBLIC_BOARD, 5.0, 120.0) }),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn without_a_configured_secret_pdf_export_is_not_set_up() {
    let f = fixture(None).await;
    let (status, body) = post(&f.app, &json!({ "token": "x" }), None).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert_eq!(body["error"], "PDF export is not set up on this deployment");
}

// (The test that read `canDownload: false` from a switched-off person's payload is
// replaced by the one at the end of this file: with R9 that person gets no payload.)

/// BI-1 review fix (BLOCKER) R1: an embedding host holds `EMBED_SECRET`; with it
/// it cannot mint a render token naming any person and any board.
#[tokio::test]
async fn a_render_token_signed_with_the_embed_secret_is_refused() {
    let f = fixture(Some(SECRET)).await;
    let claims = lakehouse_embed::RenderClaims {
        aud: "render".to_owned(),
        board: PUBLIC_BOARD.to_owned(),
        principal: f.owner.to_string(),
        filters: None,
        params: None,
        grain: None,
        paper: "a4".to_owned(),
        orientation: "portrait".to_owned(),
        exp: unix_now().floor() + 60.0,
        iat: unix_now().floor(),
        jti: "forged".to_owned(),
    };
    let forged = sign_render(&claims, EMBED_SECRET);
    let (status, body) = post(&f.app, &json!({ "token": forged }), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
}

/// BI-1 review fix (SHOULD-FIX) R9: rights are read on every data call, so a
/// revoke inside the token's two minutes stops the data at once.
#[tokio::test]
async fn a_revoked_permission_or_a_download_switch_inside_the_tokens_life_stops_the_data() {
    let f = fixture(Some(SECRET)).await;
    let token = render_token(f.other, PUBLIC_BOARD, 5.0, 120.0);
    let (status, _) = post(&f.app, &json!({ "token": token }), None).await;
    assert_eq!(status, StatusCode::OK, "the token works before the change");

    sqlx::query(
        "INSERT INTO reporting_settings (singleton, time_zone, week_start, download_denied_roles) \
         VALUES (TRUE, 'UTC', 'monday', ARRAY['Other Role'])",
    )
    .execute(&f.app.pool)
    .await
    .unwrap();
    let (status, _) = post(&f.app, &json!({ "token": token }), None).await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "downloads switched off for the role"
    );

    sqlx::query("UPDATE reporting_settings SET download_denied_roles = '{}'")
        .execute(&f.app.pool)
        .await
        .unwrap();
    let (status, _) = post(&f.app, &json!({ "token": token }), None).await;
    assert_eq!(status, StatusCode::OK, "and back");

    sqlx::query("UPDATE role SET permissions = '' WHERE name = 'Other Role'")
        .execute(&f.app.pool)
        .await
        .unwrap();
    let (status, _) = post(&f.app, &json!({ "token": token }), None).await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "dashboard:read taken away"
    );
}
