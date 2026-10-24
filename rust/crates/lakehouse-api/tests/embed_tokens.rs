//! `SEC-12`: signed embed tokens always expire, can be withdrawn, are signed
//! with a configured secret, and the embed pages say who may frame them.
//!
//! The `ClickHouse` here is a recording fake ([`FakeCh`]) that serves one
//! board row, so a test can see both what the API read and every statement
//! it wrote. No test sleeps: expiry is exercised by the `iat` and `exp` a
//! token is minted with, relative to the real clock at the moment of the
//! test (the unit tests in `lakehouse-embed` pin the exact boundaries with
//! an injected clock).

#![allow(clippy::unwrap_used, clippy::expect_used)]
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    reason = "test timestamps are Unix seconds, a few billion, exact in an f64"
)]

mod common;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use common::{
    TestApp, create_principal_with_permissions, session_cookie_for_user, spin_up_with_env,
};
use lakehouse_embed::{EmbedClaims, EmbedResource, sign_embed, unix_now};
use serde_json::{Value, json};
use tower::ServiceExt;
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, Request as MockRequest, Respond, ResponseTemplate};

const SECRET: &str = "test-embed-secret-not-real";
const BOARD: &str = "b_embed";

/// A recording `ClickHouse` serving board rows, found by `id` or by
/// `public_token` in the statement, as the real store asks for them.
#[derive(Clone)]
struct FakeCh {
    boards: Arc<Mutex<Vec<Value>>>,
    events: Arc<Mutex<Vec<Value>>>,
    log: Arc<Mutex<Vec<String>>>,
    /// Stored chart rows (`BI-26`); empty unless a test adds some.
    charts: Arc<Mutex<Vec<Value>>>,
}

fn seed_events_from_board(b: &Value) -> Vec<Value> {
    let mut events = Vec::new();
    let id = b["id"].as_str().unwrap_or_default();

    let revoked_before = match &b["embed_revoked_before"] {
        Value::String(s) => s.parse::<u64>().unwrap_or(0),
        Value::Number(n) => n.as_u64().unwrap_or(0),
        _ => 0,
    };
    if revoked_before > 0 {
        events.push(json!({
            "board_id": id,
            "kind": "revoke_all",
            "revoked_before": revoked_before.to_string(),
            "jti": "",
            "exp": "0",
            "origins_json": "[]",
        }));
    }

    let origins_str = match &b["embed_origins_json"] {
        Value::String(s) => s.as_str(),
        _ => "[]",
    };
    if origins_str != "[]" && !origins_str.is_empty() {
        events.push(json!({
            "board_id": id,
            "kind": "origins",
            "revoked_before": "0",
            "jti": "",
            "exp": "0",
            "origins_json": origins_str,
        }));
    }

    let jti_str = match &b["embed_revoked_jti_json"] {
        Value::String(s) => s.as_str(),
        _ => "[]",
    };
    if let Ok(parsed) = serde_json::from_str::<Value>(jti_str)
        && let Some(arr) = parsed.as_array()
    {
        for item in arr {
            let jti = item["jti"].as_str().unwrap_or_default();
            let exp = item["exp"].as_u64().unwrap_or(0);
            events.push(json!({
                "board_id": id,
                "kind": "withdraw",
                "revoked_before": "0",
                "jti": jti,
                "exp": exp.to_string(),
                "origins_json": "[]",
            }));
        }
    }

    events
}

fn board_row(id: &str, overrides: &Value) -> Value {
    let mut board = json!({
        "id": id, "name": "Embedded", "description": "", "created_by": "t",
        "layout_json": "", "filters_json": "", "public_token": format!("p_{id}"),
        "embed_enabled": "1", "folder_id": "", "created_at": "2026-10-10 00:00:00",
        "embed_revoked_before": "0", "embed_revoked_jti_json": "[]",
        "embed_origins_json": "[]"
    });
    for (k, v) in overrides.as_object().expect("an object") {
        board[k] = v.clone();
    }
    board
}

impl FakeCh {
    /// One board, `b_embed`.
    fn new(board_overrides: &Value) -> Self {
        Self::with_boards(vec![board_row(BOARD, board_overrides)])
    }

    fn with_boards(boards: Vec<Value>) -> Self {
        let mut events = Vec::new();
        for b in &boards {
            events.extend(seed_events_from_board(b));
        }
        Self {
            boards: Arc::new(Mutex::new(boards)),
            events: Arc::new(Mutex::new(events)),
            log: Arc::new(Mutex::new(Vec::new())),
            charts: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Change a field of a board between two requests, as a write by another
    /// session would.
    fn set(&self, id: &str, field: &str, value: &Value) {
        if field == "embed_revoked_before" {
            let val = match value {
                Value::String(s) => s.clone(),
                Value::Number(n) => n.to_string(),
                _ => "0".to_string(),
            };
            self.events.lock().unwrap().push(json!({
                "board_id": id,
                "kind": "revoke_all",
                "revoked_before": val,
                "jti": "",
                "exp": "0",
                "origins_json": "[]",
            }));
        } else if field == "embed_origins_json" {
            let val = match value {
                Value::String(s) => s.clone(),
                _ => "[]".to_string(),
            };
            self.events.lock().unwrap().push(json!({
                "board_id": id,
                "kind": "origins",
                "revoked_before": "0",
                "jti": "",
                "exp": "0",
                "origins_json": val,
            }));
        }
        for b in self.boards.lock().unwrap().iter_mut() {
            if b["id"] == id {
                b[field] = value.clone();
            }
        }
    }

    /// A `filter_modes` event for `board` (`BI-26`): the newest one is pushed last.
    fn add_modes(&self, board: &str, modes: &Value) {
        self.events.lock().unwrap().push(json!({
            "board_id": board, "kind": "filter_modes", "revoked_before": "0",
            "jti": "", "exp": "0", "origins_json": modes.to_string(),
        }));
    }

    /// The statements that read the test mart's data.
    fn tile_statements(&self) -> Vec<String> {
        self.statements()
            .into_iter()
            .filter(|s| {
                s.contains("mart_x") && s.contains("visitors") && !s.contains("system.columns")
            })
            .collect()
    }

    /// A stored chart row (`BI-26`): `id` titled `title` on `board`.
    fn add_chart(&self, id: &str, board: &str, title: &str) {
        let spec = json!({
            "id": id, "title": title, "kind": "bar", "mart": "mart_x",
            "sql": "SELECT stored", "x": "region", "y": "visitors"
        });
        let def = json!({
            "title": title, "mart": "mart_x", "kind": "bar",
            "dimension": "region", "measures": ["visitors"]
        });
        self.charts.lock().unwrap().push(json!({
            "id": id, "board": board, "created_by": "ui",
            "created_at": "2026-01-01 00:00:00",
            "spec_json": json!({ "spec": spec, "def": def, "hasYear": false }).to_string(),
        }));
    }

    fn statements(&self) -> Vec<String> {
        self.log.lock().unwrap().clone()
    }

    /// SEC-12 (PR #105 review SHOULD-FIX 1): record the embed event an
    /// `INSERT INTO console.bi_board_embed_event` carries, so a later read
    /// of the aggregate sees it. Split out of [`Respond::respond`] for the
    /// line limit.
    fn record_embed_event(&self, sql: &str) {
        let Some(pos) = sql.find("VALUES (") else {
            return;
        };
        let rest = &sql[pos + "VALUES (".len()..];
        let rest = rest.trim_end().trim_end_matches(')');
        let parts: Vec<&str> = rest.splitn(4, ", ").collect();
        if sql.contains("'origins'") && parts.len() >= 3 {
            let board_id = parts[0].trim_matches('\'');
            let origins_json = parts[2].trim_matches('\'').replace("\\\"", "\"");
            self.events.lock().unwrap().push(json!({
                "board_id": board_id,
                "kind": "origins",
                "revoked_before": "0",
                "jti": "",
                "exp": "0",
                "origins_json": origins_json,
            }));
        } else if sql.contains("'revoke_all'") && parts.len() >= 3 {
            let board_id = parts[0].trim_matches('\'');
            let revoked_before = parts[2].trim();
            self.events.lock().unwrap().push(json!({
                "board_id": board_id,
                "kind": "revoke_all",
                "revoked_before": revoked_before,
                "jti": "",
                "exp": "0",
                "origins_json": "[]",
            }));
        } else if sql.contains("'withdraw'") && parts.len() >= 4 {
            let board_id = parts[0].trim_matches('\'');
            let jti = parts[2].trim_matches('\'');
            let exp = parts[3].trim();
            self.events.lock().unwrap().push(json!({
                "board_id": board_id,
                "kind": "withdraw",
                "revoked_before": "0",
                "jti": jti,
                "exp": exp,
                "origins_json": "[]",
            }));
        }
    }

    fn inserts_into_board(&self) -> Vec<String> {
        self.statements()
            .into_iter()
            // The board table only: a content edit also writes a row of
            // `bi_board_version` (BI-18 part C, part 2), whose name starts the same.
            .filter(|s| {
                s.starts_with("INSERT INTO console.bi_board (")
                    || s.starts_with("INSERT INTO console.bi_board ")
            })
            .collect()
    }

    /// SEC-12 (PR #105 review SHOULD-FIX 1): the embed access state is
    /// written to the append-only event table and never to the board row.
    fn inserts_into_embed_events(&self) -> Vec<String> {
        self.statements()
            .into_iter()
            .filter(|s| s.starts_with("INSERT INTO console.bi_board_embed_event"))
            .collect()
    }

    async fn serve(&self) -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(self.clone())
            .mount(&server)
            .await;
        server
    }
}

impl Respond for FakeCh {
    fn respond(&self, request: &MockRequest) -> ResponseTemplate {
        let sql = String::from_utf8_lossy(&request.body).into_owned();
        self.log.lock().unwrap().push(sql.clone());
        let rows = |data: Vec<Value>| {
            let n = data.len();
            ResponseTemplate::new(200).set_body_json(json!({ "meta": [], "data": data, "rows": n }))
        };
        // The state lookup every board edit makes first (BI-18 part C, part 2):
        // live, and nobody's personal board.
        if sql.starts_with("SELECT is_deleted, owner_id FROM console.bi_board") {
            let found: Vec<Value> = self
                .boards
                .lock()
                .unwrap()
                .iter()
                .filter(|b| {
                    let id = b["id"].as_str().unwrap_or_default();
                    sql.contains(&format!("WHERE id = '{id}'"))
                })
                .map(|b| json!({ "is_deleted": 0, "owner_id": b["owner_id"].as_str().unwrap_or("") }))
                .collect();
            return rows(found);
        }
        // SEC-12 (PR #105 review SHOULD-FIX 1): the embed access state is read
        // by aggregating the append-only event table, never from the row.
        if sql.contains("FROM console.bi_board_embed_event") {
            let found: Vec<Value> = self
                .events
                .lock()
                .unwrap()
                .iter()
                .filter(|e| {
                    let id = e["board_id"].as_str().unwrap_or_default();
                    sql.contains(&format!("board_id='{id}'"))
                })
                .cloned()
                .collect();
            return rows(found);
        }
        // And it is written there too: the event table is the only writer of
        // the embed access state.
        if sql.starts_with("INSERT INTO console.bi_board_embed_event") {
            self.record_embed_event(&sql);
            return ResponseTemplate::new(200).set_body_string(
                "Ok.
",
            );
        }
        // A chart read names the boards table too (its sub-selects), so it is
        // not the board read.
        if sql.contains("FROM console.bi_board") && !sql.contains("FROM console.bi_chart") {
            let found: Vec<Value> = self
                .boards
                .lock()
                .unwrap()
                .iter()
                .filter(|b| {
                    let id = b["id"].as_str().unwrap_or_default();
                    let token = b["public_token"].as_str().unwrap_or_default();
                    sql.contains(&format!("AND id='{id}'"))
                        || (!token.is_empty() && sql.contains(&format!("public_token='{token}'")))
                })
                .cloned()
                .collect();
            return rows(found);
        }
        if sql.contains("FROM console.bi_chart") {
            return rows(self.charts.lock().unwrap().clone());
        }
        // BI-26: the columns of the one mart the test charts read, so a filter
        // on `region` reaches the tile's statement.
        if sql.contains("FROM system.columns") {
            return rows(vec![
                json!({ "table": "mart_x", "name": "region", "type": "String" }),
                json!({ "table": "mart_x", "name": "visitors", "type": "Int64" }),
            ]);
        }
        ResponseTemplate::new(200).set_body_string(
            "Ok.
",
        )
    }
}

async fn app(ch: &MockServer, secret: Option<&str>) -> TestApp {
    let mut env: HashMap<String, String> = HashMap::from([("CH_URL".to_owned(), ch.uri())]);
    if let Some(secret) = secret {
        env.insert("EMBED_SECRET".to_owned(), secret.to_owned());
    }
    spin_up_with_env(&env).await
}

async fn send(router: &axum::Router, request: Request<Body>) -> (StatusCode, Value) {
    let resp = router.clone().oneshot(request).await.expect("a response");
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.expect("body");
    let text = String::from_utf8_lossy(&bytes).into_owned();
    (
        status,
        serde_json::from_str(&text).unwrap_or(Value::String(text)),
    )
}

fn token(board: &str, iat: f64, exp: f64, jti: Option<&str>) -> String {
    sign_embed(
        &EmbedClaims {
            resource: Some(EmbedResource {
                chart: None,
                dashboard: Some(board.to_owned()),
            }),
            params: None,
            exp: Some(exp),
            iat: Some(iat),
            jti: jti.map(str::to_owned),
        },
        SECRET,
    )
}

/// A token issued `age` seconds ago that lives for an hour from then.
fn token_issued(age: f64, jti: Option<&str>) -> String {
    let iat = (unix_now() - age).floor();
    token(BOARD, iat, iat + 3600.0, jti)
}

async fn embed_data(app: &TestApp, jwt: &str) -> (StatusCode, Value) {
    send(
        &app.router,
        Request::builder()
            .method("POST")
            .uri("/api/embed/data")
            .header("content-type", "application/json")
            .body(Body::from(json!({ "jwt": jwt }).to_string()))
            .expect("build request"),
    )
    .await
}

async fn as_user(
    app: &TestApp,
    permissions: &str,
    request: axum::http::request::Builder,
    body: Body,
) -> (StatusCode, Value) {
    let user = create_principal_with_permissions(&app.pool, permissions).await;
    let cookie = session_cookie_for_user(&app.pool, user).await;
    send(
        &app.router,
        request
            .header("cookie", cookie)
            .body(body)
            .expect("build request"),
    )
    .await
}

// ── T2: the secret ─────────────────────────────────────────────────────

#[tokio::test]
async fn with_no_secret_the_embed_endpoint_says_embedding_is_not_configured_and_touches_nothing() {
    let fake = FakeCh::new(&json!({}));
    let ch = fake.serve().await;
    let app = app(&ch, None).await;

    let (status, body) = embed_data(&app, &token_issued(5.0, None)).await;

    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert_eq!(body["error"], "embedding is not configured");
    // SEC-12: nothing reads or writes `console.app_kv` for the secret any
    // more. The fake records every statement; none was sent at all.
    assert!(
        fake.statements().iter().all(|s| !s.contains("app_kv")),
        "{:?}",
        fake.statements()
    );
    assert!(fake.statements().is_empty(), "{:?}", fake.statements());
}

#[tokio::test]
async fn an_empty_secret_is_the_same_as_none() {
    let fake = FakeCh::new(&json!({}));
    let ch = fake.serve().await;
    let app = app(&ch, Some("")).await;
    let (status, body) = embed_data(&app, &token_issued(5.0, None)).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
}

#[tokio::test]
async fn embed_info_without_a_secret_is_unsupported_with_the_reason_and_no_sample_token() {
    let fake = FakeCh::new(&json!({}));
    let ch = fake.serve().await;
    let app = app(&ch, None).await;
    let (status, body) = as_user(
        &app,
        "dashboard:read",
        Request::builder().uri(format!("/api/dashboard/embed-info?board={BOARD}")),
        Body::empty(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["supported"], false);
    assert_eq!(body["reason"], "embedding is not configured");
    assert!(body.get("sampleToken").is_none(), "{body}");
    assert_eq!(body["maxLifetimeSeconds"], 86_400);
    assert!(
        fake.statements().iter().all(|s| !s.contains("app_kv")),
        "{:?}",
        fake.statements()
    );
}

#[tokio::test]
async fn embed_info_with_a_secret_returns_a_sample_token_that_has_iat_exp_and_jti() {
    let fake = FakeCh::new(&json!({}));
    let ch = fake.serve().await;
    let app = app(&ch, Some(SECRET)).await;
    let (status, body) = as_user(
        &app,
        "dashboard:read",
        Request::builder().uri(format!("/api/dashboard/embed-info?board={BOARD}")),
        Body::empty(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["supported"], true);
    let sample = body["sampleToken"].as_str().expect("a sample token");
    let claims = lakehouse_embed::verify_embed(sample, SECRET, 86_400, unix_now())
        .expect("the sample token passes the same verification the endpoint applies");
    assert!(claims.jti.is_some());
    assert!(claims.iat.is_some() && claims.exp.is_some());
    assert!(!body.to_string().contains(SECRET));
    // And the endpoint accepts it.
    let (status, data) = embed_data(&app, sample).await;
    assert_eq!(status, StatusCode::OK, "{data}");
}

// ── T1/T3: what a refused token looks like, and withdrawal ─────────────

const REFUSED: &str = "embed token is invalid or expired";

fn now_secs() -> u64 {
    unix_now().floor() as u64
}

#[tokio::test]
async fn every_kind_of_refused_token_gets_the_same_status_and_body() {
    let now = unix_now().floor();
    let gone = format!(r#"[{{"jti":"gone","exp":{}}}]"#, now_secs() + 3600);
    let fake = FakeCh::new(&json!({ "embed_revoked_jti_json": gone }));
    let ch = fake.serve().await;
    let app = app(&ch, Some(SECRET)).await;

    let no_exp = sign_embed(
        &EmbedClaims {
            resource: Some(EmbedResource {
                chart: None,
                dashboard: Some(BOARD.to_owned()),
            }),
            iat: Some(now),
            ..EmbedClaims::default()
        },
        SECRET,
    );
    let no_iat = sign_embed(
        &EmbedClaims {
            resource: Some(EmbedResource {
                chart: None,
                dashboard: Some(BOARD.to_owned()),
            }),
            exp: Some(now + 60.0),
            ..EmbedClaims::default()
        },
        SECRET,
    );
    let refused = [
        ("no exp", no_exp),
        ("no iat", no_iat),
        ("25 hours", token(BOARD, now, now + 25.0 * 3600.0, None)),
        ("expired", token(BOARD, now - 7200.0, now - 3600.0, None)),
        (
            "issued in the future",
            token(BOARD, now + 3600.0, now + 7200.0, None),
        ),
        (
            "wrong signature",
            token_issued(5.0, None).replace('.', "x."),
        ),
        ("withdrawn jti", token_issued(5.0, Some("gone"))),
        (
            "no dashboard",
            sign_embed(
                &EmbedClaims {
                    exp: Some(now + 60.0),
                    iat: Some(now),
                    ..EmbedClaims::default()
                },
                SECRET,
            ),
        ),
        ("empty dashboard", token("", now, now + 60.0, None)),
    ];
    for (what, jwt) in refused {
        let (status, body) = embed_data(&app, &jwt).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{what}: {body}");
        assert_eq!(body, json!({ "error": REFUSED }), "{what}");
    }
    let (status, _) = embed_data(&app, &token_issued(5.0, Some("fine"))).await;
    assert_eq!(status, StatusCode::OK, "a good token still works");
}

#[tokio::test]
async fn a_token_issued_before_withdraw_all_is_refused_and_one_issued_after_is_accepted() {
    let withdrawn_at = now_secs() - 100;
    let fake = FakeCh::new(&json!({ "embed_revoked_before": withdrawn_at.to_string() }));
    let ch = fake.serve().await;
    let app = app(&ch, Some(SECRET)).await;

    let (before, body) = embed_data(&app, &token_issued(500.0, None)).await;
    assert_eq!(before, StatusCode::UNAUTHORIZED, "{body}");
    assert_eq!(body["error"], REFUSED);
    // Issued at the very instant of the withdrawal: at-or-before is refused.
    let at = token(BOARD, withdrawn_at as f64, withdrawn_at as f64 + 60.0, None);
    assert_eq!(embed_data(&app, &at).await.0, StatusCode::UNAUTHORIZED);
    let (after, body) = embed_data(&app, &token_issued(5.0, None)).await;
    assert_eq!(after, StatusCode::OK, "{body}");
}

#[tokio::test]
async fn a_withdrawal_written_between_two_requests_is_honoured_by_the_second() {
    // No cache sits between the board and the answer: the board is read on
    // every request, so a withdrawal made elsewhere shows up at once (the
    // one-minute limit is met with no wait at all).
    let fake = FakeCh::new(&json!({}));
    let ch = fake.serve().await;
    let app = app(&ch, Some(SECRET)).await;
    let jwt = token_issued(30.0, Some("j1"));
    assert_eq!(embed_data(&app, &jwt).await.0, StatusCode::OK);

    fake.set(
        BOARD,
        "embed_revoked_before",
        &json!(now_secs().to_string()),
    );
    assert_eq!(embed_data(&app, &jwt).await.0, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_withdrawn_jti_is_refused_while_another_token_of_the_same_board_works() {
    let list = format!(r#"[{{"jti":"gone","exp":{}}}]"#, now_secs() + 3600);
    let fake = FakeCh::new(&json!({ "embed_revoked_jti_json": list }));
    let ch = fake.serve().await;
    let app = app(&ch, Some(SECRET)).await;
    assert_eq!(
        embed_data(&app, &token_issued(5.0, Some("gone"))).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        embed_data(&app, &token_issued(5.0, Some("kept"))).await.0,
        StatusCode::OK
    );
    assert_eq!(
        embed_data(&app, &token_issued(5.0, None)).await.0,
        StatusCode::OK
    );
}

#[tokio::test]
async fn a_withdrawn_jti_for_board_a_does_not_affect_board_b() {
    let list = format!(r#"[{{"jti":"shared-id","exp":{}}}]"#, now_secs() + 3600);
    let fake = FakeCh::with_boards(vec![
        board_row("b_a", &json!({ "embed_revoked_jti_json": list })),
        board_row("b_b", &json!({})),
    ]);
    let ch = fake.serve().await;
    let app = app(&ch, Some(SECRET)).await;
    let iat = unix_now().floor() - 5.0;
    let for_board = |b: &str| token(b, iat, iat + 600.0, Some("shared-id"));
    assert_eq!(
        embed_data(&app, &for_board("b_a")).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(embed_data(&app, &for_board("b_b")).await.0, StatusCode::OK);
}

#[tokio::test]
async fn the_public_link_ignores_every_new_check() {
    // Out of scope for SEC-12 and must not change: a board with everything
    // withdrawn and no site allowed still serves its public link.
    let fake = FakeCh::new(&json!({
        "embed_revoked_before": u64::MAX.to_string(),
        "embed_enabled": "0",
    }));
    let ch = fake.serve().await;
    let app = app(&ch, None).await;
    // BI-19 part A: the link is a row in Postgres, not the board's column.
    lakehouse_store::public_links::replace(
        &app.pool,
        BOARD,
        &format!("p_{BOARD}"),
        time::OffsetDateTime::now_utc() + time::Duration::days(30),
        None,
        None,
    )
    .await
    .unwrap();
    let (status, body) = send(
        &app.router,
        Request::builder()
            .uri(format!("/api/public/dashboard/p_{BOARD}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["board"]["id"], BOARD);
}

async fn revoke_one(app: &TestApp, perms: &str, board: &str, token: &str) -> (StatusCode, Value) {
    as_user(
        app,
        perms,
        Request::builder()
            .method("POST")
            .uri("/api/dashboard/embed-revoke")
            .header("content-type", "application/json"),
        Body::from(json!({ "board": board, "token": token }).to_string()),
    )
    .await
}

#[tokio::test]
async fn withdrawing_one_token_stores_its_jti_and_keeps_the_rest_of_the_board_state() {
    let fake = FakeCh::new(&json!({
        "embed_revoked_before": "1234",
        "embed_origins_json": r#"["https://app.customer.example"]"#,
    }));
    let ch = fake.serve().await;
    let app = app(&ch, Some(SECRET)).await;
    let jwt = token_issued(5.0, Some("to-withdraw"));

    let (status, body) = revoke_one(&app, "dashboard:write", BOARD, &jwt).await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["withdrawn"], "to-withdraw");
    let inserts = fake.inserts_into_embed_events();
    assert_eq!(inserts.len(), 1, "{inserts:?}");
    let insert = &inserts[0];
    assert!(insert.contains("to-withdraw"), "{insert}");
    // SEC-12 (PR #105 review SHOULD-FIX 1): withdrawing does not touch the
    // board row.
    assert!(fake.inserts_into_board().is_empty());
}

#[tokio::test]
async fn withdrawing_needs_dashboard_write() {
    let fake = FakeCh::new(&json!({}));
    let ch = fake.serve().await;
    let app = app(&ch, Some(SECRET)).await;
    let jwt = token_issued(5.0, Some("j"));

    let (status, _) = revoke_one(&app, "dashboard:read", BOARD, &jwt).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let (status, _) = as_user(
        &app,
        "dashboard:read",
        Request::builder()
            .method("PUT")
            .uri("/api/dashboard/boards")
            .header("content-type", "application/json"),
        Body::from(json!({ "id": BOARD, "embedRevokeAll": true }).to_string()),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(fake.inserts_into_board().is_empty(), "nothing was written");
}

#[tokio::test]
async fn a_token_without_a_jti_cannot_be_withdrawn_singly_and_the_answer_says_so() {
    let fake = FakeCh::new(&json!({}));
    let ch = fake.serve().await;
    let app = app(&ch, Some(SECRET)).await;
    let (status, body) = revoke_one(&app, "dashboard:write", BOARD, &token_issued(5.0, None)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let message = body["error"].as_str().unwrap();
    assert!(message.contains("no id (jti)"), "{message}");
    assert!(message.contains("withdraw all"), "{message}");
    assert!(fake.inserts_into_board().is_empty());
}

#[tokio::test]
async fn withdrawing_refuses_a_bad_token_a_token_for_another_board_and_a_bare_jti() {
    let fake = FakeCh::new(&json!({}));
    let ch = fake.serve().await;
    let app = app(&ch, Some(SECRET)).await;

    let (status, body) = revoke_one(&app, "dashboard:write", BOARD, "not.a.token").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["error"], REFUSED);

    let iat = unix_now().floor();
    let other = token("b_other", iat, iat + 60.0, Some("j"));
    let (status, body) = revoke_one(&app, "dashboard:write", BOARD, &other).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .contains("different dashboard")
    );

    // A bare `jti` is not accepted in place of a token.
    let (status, _) = as_user(
        &app,
        "dashboard:write",
        Request::builder()
            .method("POST")
            .uri("/api/dashboard/embed-revoke")
            .header("content-type", "application/json"),
        Body::from(json!({ "board": BOARD, "jti": "guess" }).to_string()),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(fake.inserts_into_board().is_empty());
}

#[tokio::test]
async fn withdraw_all_writes_the_instant_and_an_unrelated_edit_does_not_reset_it() {
    let fake = FakeCh::new(&json!({}));
    let ch = fake.serve().await;
    let app = app(&ch, Some(SECRET)).await;
    let before = now_secs();
    let (status, body) = as_user(
        &app,
        "dashboard:write",
        Request::builder()
            .method("PUT")
            .uri("/api/dashboard/boards")
            .header("content-type", "application/json"),
        Body::from(json!({ "id": BOARD, "embedRevokeAll": true }).to_string()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let stored = body["embedRevokedBefore"].as_u64().expect("the instant");
    assert!((before..=now_secs()).contains(&stored), "{stored}");
    let event_inserts = fake.inserts_into_embed_events();
    assert_eq!(event_inserts.len(), 1);
    assert!(
        event_inserts[0].contains(&format!(", {stored}")),
        "{event_inserts:?}"
    );
    // SEC-12 (PR #105 review SHOULD-FIX 1): revoke_all inserts into the
    // event table and does not touch bi_board.
    assert!(fake.inserts_into_board().is_empty());

    // A rename afterwards writes the board row, but contains none of the embed columns.
    fake.set(BOARD, "embed_revoked_before", &json!(stored.to_string()));
    let (status, _) = as_user(
        &app,
        "dashboard:write",
        Request::builder()
            .method("PUT")
            .uri("/api/dashboard/boards")
            .header("content-type", "application/json"),
        Body::from(json!({ "id": BOARD, "name": "Renamed" }).to_string()),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let last = fake.inserts_into_board().pop().unwrap();
    assert!(!last.contains("embed_revoked_before"), "{last}");
    assert!(!last.contains("embed_revoked_jti_json"), "{last}");
    assert!(!last.contains("embed_origins_json"), "{last}");
}

#[tokio::test]
async fn withdraw_all_can_only_be_true() {
    let fake = FakeCh::new(&json!({}));
    let ch = fake.serve().await;
    let app = app(&ch, Some(SECRET)).await;
    let (status, _) = as_user(
        &app,
        "dashboard:write",
        Request::builder()
            .method("PUT")
            .uri("/api/dashboard/boards")
            .header("content-type", "application/json"),
        Body::from(json!({ "id": BOARD, "embedRevokeAll": false }).to_string()),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(fake.inserts_into_board().is_empty());
}

// ── T4: allowed sites and the frame answer ─────────────────────────────

async fn put_origins(app: &TestApp, perms: &str, origins: Value) -> (StatusCode, Value) {
    as_user(
        app,
        perms,
        Request::builder()
            .method("PUT")
            .uri("/api/dashboard/boards")
            .header("content-type", "application/json"),
        Body::from(json!({ "id": BOARD, "embedOrigins": origins }).to_string()),
    )
    .await
}

#[tokio::test]
async fn allowed_sites_are_validated_normalised_and_stored() {
    let fake = FakeCh::new(&json!({}));
    let ch = fake.serve().await;
    let app = app(&ch, Some(SECRET)).await;

    let (status, body) = put_origins(
        &app,
        "dashboard:write",
        json!(["https://App.Customer.example", "http://localhost:3000"]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["embedOrigins"],
        json!(["https://app.customer.example", "http://localhost:3000"])
    );
    assert!(fake.inserts_into_embed_events()[0].contains("https://app.customer.example"));
    assert!(fake.inserts_into_board().is_empty());

    for bad in [
        json!(["https://*.customer.example"]),
        json!(["http://app.customer.example"]),
        json!(["https://a.example/path"]),
        json!(
            (0..21)
                .map(|i| format!("https://s{i}.example"))
                .collect::<Vec<_>>()
        ),
    ] {
        let writes = fake.inserts_into_embed_events().len();
        let (status, body) = put_origins(&app, "dashboard:write", bad).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert!(body["error"].is_string());
        assert_eq!(
            fake.inserts_into_embed_events().len(),
            writes,
            "nothing written"
        );
    }
}

#[tokio::test]
async fn editing_allowed_sites_needs_dashboard_write() {
    let fake = FakeCh::new(&json!({}));
    let ch = fake.serve().await;
    let app = app(&ch, Some(SECRET)).await;
    let (status, _) = put_origins(&app, "dashboard:read", json!(["https://a.example"])).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(fake.inserts_into_board().is_empty());
}

#[tokio::test]
async fn embed_info_reports_the_allowed_sites_the_instant_and_the_limit() {
    let fake = FakeCh::new(&json!({
        "embed_revoked_before": "1700000000",
        "embed_origins_json": r#"["https://a.example"]"#,
    }));
    let ch = fake.serve().await;
    let app = app(&ch, Some(SECRET)).await;
    let (status, body) = as_user(
        &app,
        "dashboard:read",
        Request::builder().uri(format!("/api/dashboard/embed-info?board={BOARD}")),
        Body::empty(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["allowedOrigins"], json!(["https://a.example"]));
    assert_eq!(body["revokedBefore"], 1_700_000_000);
    assert_eq!(body["maxLifetimeSeconds"], 86_400);
    // The withdrawn ids themselves are not for a dashboard reader.
    assert!(!body.to_string().contains("jti"), "{body}");
}

async fn frame(app: &TestApp, body: Value) -> (StatusCode, Value) {
    send(
        &app.router,
        Request::builder()
            .method("POST")
            .uri("/api/embed/frame")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap(),
    )
    .await
}

#[tokio::test]
async fn the_frame_answer_is_the_boards_sites_for_a_good_token_and_empty_for_every_other() {
    let list = format!(r#"[{{"jti":"gone","exp":{}}}]"#, now_secs() + 3600);
    let fake = FakeCh::with_boards(vec![
        board_row(
            BOARD,
            &json!({
                "embed_origins_json": r#"["https://app.customer.example"]"#,
                "embed_revoked_jti_json": list,
            }),
        ),
        board_row(
            "b_off",
            &json!({ "embed_enabled": "0", "embed_origins_json": r#"["https://x.example"]"# }),
        ),
    ]);
    let ch = fake.serve().await;
    let app = app(&ch, Some(SECRET)).await;
    lakehouse_store::public_links::replace(
        &app.pool,
        BOARD,
        &format!("p_{BOARD}"),
        time::OffsetDateTime::now_utc() + time::Duration::days(30),
        None,
        None,
    )
    .await
    .unwrap();
    let listed = json!({ "origins": ["https://app.customer.example"] });
    let none = json!({ "origins": [] });

    // Signed token, public link token.
    assert_eq!(
        frame(&app, json!({ "jwt": token_issued(5.0, Some("ok")) })).await,
        (StatusCode::OK, listed.clone())
    );
    assert_eq!(
        frame(&app, json!({ "token": format!("p_{BOARD}") })).await,
        (StatusCode::OK, listed)
    );

    // Everything else is the same answer as a board with no sites listed.
    let iat = unix_now().floor();
    let others = [
        json!({ "jwt": token_issued(5.0, Some("gone")) }),
        json!({ "jwt": token_issued(5.0, None).replace('.', "x.") }),
        json!({ "jwt": token(BOARD, iat, iat + 86_400.0 * 2.0, None) }),
        json!({ "jwt": token("b_off", iat, iat + 60.0, None) }),
        json!({ "jwt": token("b_missing", iat, iat + 60.0, None) }),
        json!({ "token": "p_does_not_exist" }),
        json!({ "token": "" }),
        json!({}),
    ];
    for body in others {
        assert_eq!(
            frame(&app, body.clone()).await,
            (StatusCode::OK, none.clone()),
            "{body}"
        );
    }
}

#[tokio::test]
async fn the_frame_answer_without_a_secret_is_empty_not_an_error() {
    let fake = FakeCh::new(&json!({}));
    let ch = fake.serve().await;
    let app = app(&ch, None).await;
    assert_eq!(
        frame(&app, json!({ "jwt": token_issued(5.0, None) })).await,
        (StatusCode::OK, json!({ "origins": [] }))
    );
}

// ── BI-26 T1: chart tokens ─────────────────────────────────────────────

fn chart_token(chart: &str, jti: Option<&str>) -> String {
    let iat = (unix_now() - 5.0).floor();
    sign_embed(
        &EmbedClaims {
            resource: Some(EmbedResource {
                chart: Some(chart.to_owned()),
                dashboard: None,
            }),
            params: None,
            exp: Some(iat + 3600.0),
            iat: Some(iat),
            jti: jti.map(str::to_owned),
        },
        SECRET,
    )
}

fn two_charts(fake: &FakeCh) {
    fake.add_chart("u_one", BOARD, "Visitors by region");
    fake.add_chart("u_two", BOARD, "Secret revenue");
}

#[tokio::test]
async fn a_chart_token_reads_that_chart_and_nothing_that_names_another() {
    let fake = FakeCh::new(&json!({ "name": "Whole board name" }));
    two_charts(&fake);
    let ch = fake.serve().await;
    let app = app(&ch, Some(SECRET)).await;

    let (status, body) = embed_data(&app, &chart_token("u_one", None)).await;

    assert_eq!(status, StatusCode::OK, "{body}");
    let charts = body["charts"].as_array().unwrap();
    assert_eq!(charts.len(), 1, "{body}");
    assert_eq!(charts[0]["id"], "u_one");
    assert_eq!(body["onlyChart"], "u_one");
    assert_eq!(body["layout"], json!({}));
    assert!(body.get("tabs").is_none() && body.get("phoneLayout").is_none());
    let text = body.to_string();
    for other in ["u_two", "Secret revenue", "Whole board name"] {
        assert!(
            !text.contains(other),
            "{other} is in the chart payload: {text}"
        );
    }
    // A dashboard token for the same board still reads both.
    let (_, whole) = embed_data(&app, &token_issued(5.0, None)).await;
    assert_eq!(whole["charts"].as_array().unwrap().len(), 2, "{whole}");
}

#[tokio::test]
async fn a_token_naming_both_neither_or_an_unknown_chart_is_refused_alike() {
    let fake = FakeCh::new(&json!({}));
    two_charts(&fake);
    let ch = fake.serve().await;
    let app = app(&ch, Some(SECRET)).await;
    let iat = (unix_now() - 5.0).floor();
    let signed = |dashboard: Option<&str>, chart: Option<&str>| {
        sign_embed(
            &EmbedClaims {
                resource: Some(EmbedResource {
                    chart: chart.map(str::to_owned),
                    dashboard: dashboard.map(str::to_owned),
                }),
                params: None,
                exp: Some(iat + 3600.0),
                iat: Some(iat),
                jti: None,
            },
            SECRET,
        )
    };
    let refused = [
        signed(Some(BOARD), Some("u_one")),
        signed(None, None),
        signed(None, Some("u_not_a_chart")),
    ];
    for jwt in refused {
        let (status, body) = embed_data(&app, &jwt).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
        assert_eq!(body["error"], "embed token is invalid or expired");
    }
}

#[tokio::test]
async fn a_chart_token_obeys_the_boards_embed_switch_and_withdrawal() {
    let off = FakeCh::with_boards(vec![board_row(BOARD, &json!({ "embed_enabled": "0" }))]);
    two_charts(&off);
    let ch = off.serve().await;
    let app_off = app(&ch, Some(SECRET)).await;
    let (status, _) = embed_data(&app_off, &chart_token("u_one", None)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let withdrawn = FakeCh::new(&json!({ "embed_revoked_before": u64::MAX.to_string() }));
    two_charts(&withdrawn);
    let ch2 = withdrawn.serve().await;
    let app_w = app(&ch2, Some(SECRET)).await;
    let (status, _) = embed_data(&app_w, &chart_token("u_one", None)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_chart_token_is_framed_and_withdrawn_through_its_charts_board() {
    let fake = FakeCh::new(&json!({ "embed_origins_json": r#"["https://app.customer.example"]"# }));
    two_charts(&fake);
    let ch = fake.serve().await;
    let app = app(&ch, Some(SECRET)).await;
    let jwt = chart_token("u_one", Some("one-id"));
    assert_eq!(
        frame(&app, json!({ "jwt": jwt })).await,
        (
            StatusCode::OK,
            json!({ "origins": ["https://app.customer.example"] })
        )
    );
    let (status, body) = revoke_one(&app, "dashboard:write", BOARD, &jwt).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["withdrawn"], "one-id");
    // The same token cannot be withdrawn through another board.
    let (status, _) = revoke_one(&app, "dashboard:write", "b_other", &jwt).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

// ── BI-26 T2: filter modes ─────────────────────────────────────────────

const SAVED_BALI: &str = r#"[{"column":"region","op":"in","values":["Bali"]}]"#;

/// A board with a saved `region = Bali` filter, one chart, and `modes` chosen.
fn modal_board(modes: &Value) -> FakeCh {
    let fake = FakeCh::new(&json!({ "filters_json": SAVED_BALI }));
    fake.add_chart("u_one", BOARD, "Visitors by region");
    if !modes.is_null() {
        fake.add_modes(BOARD, modes);
    }
    fake
}

async fn embed_with(app: &TestApp, jwt: &str, filters: Option<Value>) -> (StatusCode, Value) {
    let mut body = json!({ "jwt": jwt });
    if let Some(f) = filters {
        body["filters"] = f;
    }
    send(
        &app.router,
        Request::builder()
            .method("POST")
            .uri("/api/embed/data")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap(),
    )
    .await
}

fn token_with(params: Option<Value>) -> String {
    let iat = (unix_now() - 5.0).floor();
    sign_embed(
        &EmbedClaims {
            resource: Some(EmbedResource {
                chart: None,
                dashboard: Some(BOARD.to_owned()),
            }),
            params: params.map(|p| serde_json::from_value(p).unwrap()),
            exp: Some(iat + 3600.0),
            iat: Some(iat),
            jti: None,
        },
        SECRET,
    )
}

fn sql_has(fake: &FakeCh, needle: &str) -> bool {
    fake.tile_statements().iter().any(|s| s.contains(needle))
}

#[tokio::test]
async fn a_locked_filter_needs_its_value_in_the_token_and_the_refusal_is_the_fixed_one() {
    let fake = modal_board(&json!({ "region": "locked" }));
    let ch = fake.serve().await;
    let app = app(&ch, Some(SECRET)).await;

    let (status, body) = embed_with(&app, &token_with(None), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    assert_eq!(body["error"], "embed token is invalid or expired");
    assert!(
        !body.to_string().contains("region"),
        "the column is not named: {body}"
    );
    assert!(fake.tile_statements().is_empty(), "nothing was read");

    let (status, body) =
        embed_with(&app, &token_with(Some(json!({ "region": "Bali" }))), None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(sql_has(&fake, "Bali"));
    assert!(
        body.get("embedFilters").is_none(),
        "a locked filter is not described"
    );
}

#[tokio::test]
async fn an_editable_filter_takes_the_viewers_value_and_is_described() {
    let fake = modal_board(&json!({ "region": "editable" }));
    let ch = fake.serve().await;
    let first = app(&ch, Some(SECRET)).await;

    let (status, body) = embed_with(
        &first,
        &token_with(None),
        Some(json!([{ "column": "region", "op": "in", "values": ["Java"] }])),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        sql_has(&fake, "Java") && !sql_has(&fake, "Bali"),
        "{:?}",
        fake.tile_statements()
    );
    let described = body["embedFilters"].as_array().unwrap();
    assert_eq!(described.len(), 1);
    assert_eq!(described[0]["column"], "region");
    assert_eq!(described[0]["filter"]["values"], json!(["Java"]));

    // Without a viewer value the saved one applies.
    let fake2 = modal_board(&json!({ "region": "editable" }));
    let ch2 = fake2.serve().await;
    let app2 = app(&ch2, Some(SECRET)).await;
    let (status, _) = embed_with(&app2, &token_with(None), None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(sql_has(&fake2, "Bali"));
}

#[tokio::test]
async fn a_viewer_value_for_a_hidden_or_locked_column_is_dropped_and_a_token_value_wins() {
    let java = json!([{ "column": "region", "op": "in", "values": ["Java"] }]);
    for (modes, params) in [
        (json!({ "region": "hidden" }), None),
        (Value::Null, None), // no mode chosen: hidden
        (
            json!({ "region": "locked" }),
            Some(json!({ "region": "Bali" })),
        ),
        // editable, but the token set it: decision 7, the token wins.
        (
            json!({ "region": "editable" }),
            Some(json!({ "region": "Bali" })),
        ),
    ] {
        let fake = modal_board(&modes);
        let ch = fake.serve().await;
        let app = app(&ch, Some(SECRET)).await;
        let (status, body) = embed_with(&app, &token_with(params), Some(java.clone())).await;
        assert_eq!(status, StatusCode::OK, "{modes}: {body}");
        assert!(sql_has(&fake, "Bali"), "{modes}");
        assert!(
            !sql_has(&fake, "Java"),
            "{modes}: the viewer's value was applied"
        );
        assert!(body.get("embedFilters").is_none(), "{modes}: {body}");
    }
}

#[tokio::test]
async fn an_old_token_that_locks_a_column_that_is_no_filter_still_narrows() {
    // BI-26-AC7: no saved filter, no mode; the token's params apply as before.
    let fake = FakeCh::new(&json!({}));
    fake.add_chart("u_one", BOARD, "Visitors by region");
    let ch = fake.serve().await;
    let app = app(&ch, Some(SECRET)).await;
    let (status, body) =
        embed_with(&app, &token_with(Some(json!({ "region": "Bali" }))), None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(sql_has(&fake, "Bali"));
}

#[tokio::test]
async fn an_invalid_viewer_filter_is_a_400() {
    let fake = modal_board(&json!({ "region": "editable" }));
    let ch = fake.serve().await;
    let app = app(&ch, Some(SECRET)).await;
    let (status, body) = embed_with(
        &app,
        &token_with(None),
        Some(json!([{ "column": "bad column!", "op": "in", "values": ["x"] }])),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(fake.tile_statements().is_empty());
}
