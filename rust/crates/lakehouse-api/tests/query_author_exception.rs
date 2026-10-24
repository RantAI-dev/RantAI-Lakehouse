//! SEC-11, the query-author exception (product owner decision, 2026-10-10).
//!
//! Query Studio (`POST /api/query/run`, `POST /api/query/estimate`) shows the
//! signed-in author the engine's diagnosis of their own statement, with the
//! trailing version and any address trimmed and a reference id. Nothing else
//! does: a dashboard tile, the public link and a signed embed that hit the
//! very same engine error still answer with the fixed message. These tests
//! pin both halves so the exception cannot spread through a shared function.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::collections::HashMap;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use common::{
    TestApp, create_principal_with_permissions, session_cookie_for_user, spin_up_with_env,
};
use serde_json::{Value, json};
use tower::ServiceExt;
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, Request as MockRequest, Respond, ResponseTemplate};

const HOST: &str = "ch-internal.example.net:8123";
const ADDR: &str = "10.9.8.7";
const ENGINE_ERROR: &str = "Code: 47. DB::Exception: Unknown identifier 'nope' in scope SELECT nope FROM serving.t, reached via ch-internal.example.net:8123 (10.9.8.7). (UNKNOWN_IDENTIFIER) (version 24.8.1.1)";

/// A `ClickHouse` that fails every query with a planted statement error.
async fn failing_clickhouse() -> MockServer {
    let ch = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(404).set_body_string(ENGINE_ERROR))
        .mount(&ch)
        .await;
    ch
}

async fn app_with(env: &[(&str, String)]) -> TestApp {
    let mut overrides: HashMap<String, String> = env
        .iter()
        .map(|(k, v)| ((*k).to_owned(), v.clone()))
        .collect();
    // The shared catalog belongs to one tenant, and the query path is
    // scoped to the tenant: this principal must be the catalog's own.
    overrides.insert(
        "CATALOG_TENANT_ID".to_owned(),
        "11111111-1111-4111-8111-000000000001".to_owned(),
    );
    spin_up_with_env(&overrides).await
}

async fn send(router: &axum::Router, request: Request<Body>) -> (StatusCode, String) {
    let resp = router.clone().oneshot(request).await.expect("a response");
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.expect("body");
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

async fn post_as_author(app: &TestApp, uri: &str, body: &str) -> (StatusCode, String) {
    let user = create_principal_with_permissions(&app.pool, "query:read").await;
    // The shared catalog belongs to one tenant, and the query path is
    // scoped to the tenant: this principal is put in the catalog's own.
    sqlx::query("INSERT INTO app_user_tenant (user_id, tenant_id) VALUES ($1, $2::uuid)")
        .bind(user)
        .bind("11111111-1111-4111-8111-000000000001")
        .execute(&app.pool)
        .await
        .expect("attach the fixture principal to the catalog's tenant");
    let cookie = session_cookie_for_user(&app.pool, user).await;
    send(
        &app.router,
        Request::builder()
            .method("POST")
            .uri(uri)
            .header("cookie", cookie)
            .header("content-type", "application/json")
            .body(Body::from(body.to_owned()))
            .expect("build request"),
    )
    .await
}

fn assert_trimmed_diagnosis(body: &str) {
    assert!(body.contains("Unknown identifier 'nope'"), "{body}");
    assert!(body.contains("UNKNOWN_IDENTIFIER"), "{body}");
    assert!(body.contains("Reference: "), "{body}");
    assert!(!body.contains("version 24.8"), "{body}");
    assert!(
        !body.contains(HOST) && !body.contains("ch-internal"),
        "{body}"
    );
    assert!(!body.contains(ADDR), "{body}");
}

#[tokio::test]
async fn the_author_sees_the_engines_diagnosis_of_their_own_statement() {
    let ch = failing_clickhouse().await;
    let app = app_with(&[("CH_URL", ch.uri())]).await;
    let (status, body) = post_as_author(
        &app,
        "/api/query/run",
        r#"{"sql":"SELECT nope FROM serving.t"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_trimmed_diagnosis(&body);
}

#[tokio::test]
async fn the_cost_estimate_shows_the_author_the_same_trimmed_diagnosis() {
    let ch = failing_clickhouse().await;
    let app = app_with(&[("CH_URL", ch.uri())]).await;
    let (status, body) = post_as_author(
        &app,
        "/api/query/estimate",
        r#"{"sql":"SELECT nope FROM serving.t"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let json: Value = serde_json::from_str(&body).expect("json");
    assert_trimmed_diagnosis(json["error"].as_str().expect("an error message"));
}

#[tokio::test]
async fn an_unreachable_engine_still_answers_the_fixed_message() {
    // Nothing listens here, so this is a transport failure, not a statement error.
    let app = app_with(&[("CH_URL", "http://127.0.0.1:1".to_owned())]).await;
    let (status, body) = post_as_author(&app, "/api/query/run", r#"{"sql":"SELECT 1"}"#).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert!(
        body.contains("The database is unavailable. Reference: "),
        "{body}"
    );
    assert!(!body.contains("127.0.0.1"), "{body}");
}

#[tokio::test]
async fn an_engine_failure_that_is_not_a_query_error_stays_fixed() {
    let ch = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(502)
                .set_body_string("<html>bad gateway ch-internal.example.net</html>"),
        )
        .mount(&ch)
        .await;
    let app = app_with(&[("CH_URL", ch.uri())]).await;
    let (status, body) = post_as_author(&app, "/api/query/run", r#"{"sql":"SELECT 1"}"#).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert!(
        body.contains("The database request failed. Reference: "),
        "{body}"
    );
    assert!(!body.contains("gateway"), "{body}");
}

#[tokio::test]
async fn a_public_link_tile_with_the_same_engine_error_stays_fixed() {
    let ch = failing_clickhouse().await;
    let app = app_with(&[("CH_URL", ch.uri())]).await;
    let (_status, body) = send(
        &app.router,
        Request::builder()
            .uri("/api/public/dashboard/some-token")
            .body(Body::empty())
            .expect("build request"),
    )
    .await;
    assert!(!body.contains("Unknown identifier"), "{body}");
    assert!(!body.contains("UNKNOWN_IDENTIFIER"), "{body}");
    assert!(!body.contains("ch-internal"), "{body}");
}

/// A `ClickHouse` that serves one embed-enabled board holding one chart and
/// fails every other query (the tile's own) with the planted engine error.
struct EmbedClickhouse;

impl Respond for EmbedClickhouse {
    fn respond(&self, request: &MockRequest) -> ResponseTemplate {
        let sql = String::from_utf8_lossy(&request.body).into_owned();
        let rows = |data: Vec<Value>| {
            let n = data.len();
            ResponseTemplate::new(200).set_body_json(json!({
                "meta": [], "data": data, "rows": n
            }))
        };
        if sql.starts_with("CREATE") || sql.starts_with("ALTER") || sql.starts_with("INSERT") {
            return ResponseTemplate::new(200);
        }
        // PR #201 review G BLOCKER 1: the DDL bootstrap asks `system.columns`
        // whether the legacy embed columns are still on `console.bi_board`.
        // This database has none (a fresh one), so the answer is empty and
        // the one-time copy of the legacy state is skipped. Answering the
        // question with an engine error instead made the whole bootstrap
        // fail, and the request answered `500` — but only while this test
        // was the first to reach the bootstrap, because the DDL guard is
        // process-wide.
        if sql.contains("FROM system.columns") {
            return rows(Vec::new());
        }
        // The embed access state is read by aggregating the append-only event
        // table (SEC-12). This board has no events: nothing is withdrawn and
        // no site may frame it.
        if sql.contains("FROM console.bi_board_embed_event") {
            return rows(Vec::new());
        }
        // The board itself. The table name is matched exactly: the event
        // table's name starts with `console.bi_board` too, and a match on
        // the prefix answered the aggregate read with a board row.
        // A chart read names the boards table too (its sub-selects keep another
        // user's personal board and a deleted board out, BI-18 part C part 2),
        // so a statement that reads charts is not the board read.
        if sql.contains("FROM console.bi_board FINAL") && !sql.contains("FROM console.bi_chart") {
            return rows(vec![json!({
                "id": "b_embed", "name": "Embedded", "description": "", "created_by": "t",
                "layout_json": "", "filters_json": "", "public_token": "", "embed_enabled": "1",
                "folder_id": "", "created_at": "2026-10-10 00:00:00"
            })]);
        }
        if sql.contains("FROM console.bi_chart") {
            let spec = r#"{"spec":{"id":"u_1","title":"T","kind":"bar","mart":"mart_x","sql":"SELECT 1","x":"place","y":"visitors"},"def":{"title":"T","mart":"mart_x","kind":"bar","dimension":"place","measures":["visitors"]}}"#;
            return rows(vec![json!({
                "id": "u_1", "spec_json": spec, "board": "b_embed", "created_by": "t",
                "created_at": "2026-10-10 00:00:00"
            })]);
        }
        ResponseTemplate::new(404).set_body_string(ENGINE_ERROR)
    }
}

#[tokio::test]
async fn a_failing_tile_inside_a_signed_embed_is_the_fixed_message_with_a_reference() {
    let ch = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(EmbedClickhouse)
        .mount(&ch)
        .await;
    let secret = "test-embed-secret";
    let app = app_with(&[("CH_URL", ch.uri()), ("EMBED_SECRET", secret.to_owned())]).await;
    let now = lakehouse_embed::unix_now().floor();
    let token = lakehouse_embed::sign_embed(
        &lakehouse_embed::EmbedClaims {
            resource: Some(lakehouse_embed::EmbedResource {
                dashboard: Some("b_embed".to_owned()),
            }),
            params: None,
            // SEC-12: a token must carry `iat` and `exp`.
            exp: Some(now + 3600.0),
            iat: Some(now),
            jti: None,
        },
        secret,
    );

    let (status, body) = send(
        &app.router,
        Request::builder()
            .method("POST")
            .uri("/api/embed/data")
            .header("content-type", "application/json")
            .body(Body::from(json!({ "jwt": token }).to_string()))
            .expect("build request"),
    )
    .await;

    assert_eq!(
        status,
        StatusCode::OK,
        "the token must get past verification: {body}"
    );
    for marker in [
        "Unknown identifier",
        "UNKNOWN_IDENTIFIER",
        "ch-internal",
        ADDR,
        "version 24.8",
    ] {
        assert!(!body.contains(marker), "{marker} leaked: {body}");
    }
    let json: Value = serde_json::from_str(&body).expect("json");
    let tile = &json["results"]["u_1"];
    assert_eq!(tile["error"], "This chart could not be loaded.", "{body}");
    assert!(tile["errorId"].is_string(), "{body}");
}

/// `BI-13`: a `ClickHouse` that serves one embed-enabled board holding one
/// combined chart: the base question answers, and the other question's table
/// fails with the planted engine error (or, with `sources_fail`, the list of
/// SQL sources the other question needs cannot be read).
struct CombinedEmbedClickhouse {
    layer: Value,
}

impl Respond for CombinedEmbedClickhouse {
    fn respond(&self, request: &MockRequest) -> ResponseTemplate {
        let sql = String::from_utf8_lossy(&request.body).into_owned();
        let rows = |data: Vec<Value>| {
            let n = data.len();
            ResponseTemplate::new(200).set_body_json(json!({ "meta": [], "data": data, "rows": n }))
        };
        if sql.starts_with("CREATE") || sql.starts_with("ALTER") || sql.starts_with("INSERT") {
            return ResponseTemplate::new(200);
        }
        // PR #201 review G BLOCKER 1: the board is read from `console.bi_board`
        // exactly; the append-only event table's name starts with the same
        // prefix, and a match on the prefix answered the aggregate read with
        // a board row.
        // A chart read names the boards table too (its sub-selects keep another
        // user's personal board and a deleted board out, BI-18 part C part 2),
        // so a statement that reads charts is not the board read.
        if sql.contains("FROM console.bi_board FINAL") && !sql.contains("FROM console.bi_chart") {
            return rows(vec![json!({
                "id": "b_embed", "name": "Embedded", "description": "", "created_by": "t",
                "layout_json": "", "filters_json": "", "public_token": "", "embed_enabled": "1",
                "folder_id": "", "created_at": "2026-10-10 00:00:00"
            })]);
        }
        // The embed access state: this board has no events.
        if sql.contains("FROM console.bi_board_embed_event") {
            return rows(Vec::new());
        }
        if sql.contains("FROM console.bi_chart") {
            let spec = json!({
                "spec": { "id": "u_1", "title": "T", "kind": "bar", "mart": "mart_x",
                    "sql": "SELECT place, visitors FROM serving.mart_x",
                    "x": "place", "y": "visitors" },
                "def": { "title": "T", "mart": "mart_x", "kind": "bar", "dimension": "place",
                    "measures": ["visitors"], "layers": [self.layer] }
            });
            return rows(vec![json!({
                "id": "u_1", "spec_json": spec.to_string(), "board": "b_embed",
                "created_by": "t", "created_at": "2026-10-10 00:00:00"
            })]);
        }
        // PR #201 review G BLOCKER 1: the DDL bootstrap asks `system.columns`
        // whether the legacy embed columns are still on `console.bi_board`.
        // This database has none (a fresh one), so the answer is empty and
        // the one-time copy of the legacy state is skipped. The other
        // `system.columns` read — a serving mart's columns — is answered
        // below.
        if sql.contains("FROM system.columns") && sql.contains("'console'") {
            return rows(Vec::new());
        }
        if sql.contains("FROM system.columns") {
            return rows(
                [
                    ("mart_x", "place", "String"),
                    ("mart_x", "visitors", "UInt32"),
                    ("mart_y", "place", "String"),
                    ("mart_y", "rooms", "UInt32"),
                ]
                .iter()
                .map(|(t, n, ty)| json!({ "table": t, "name": n, "type": ty }))
                .collect(),
            );
        }
        // The base question's own statement.
        if sql.contains("serving.mart_x") && !sql.contains("serving.mart_y") {
            return rows(vec![json!({ "place": "A", "visitors": 5 })]);
        }
        ResponseTemplate::new(404).set_body_string(ENGINE_ERROR)
    }
}

async fn combined_embed(layer: Value) -> Value {
    let ch = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(CombinedEmbedClickhouse { layer })
        .mount(&ch)
        .await;
    let secret = "test-embed-secret";
    let app = app_with(&[("CH_URL", ch.uri()), ("EMBED_SECRET", secret.to_owned())]).await;
    let now = lakehouse_embed::unix_now().floor();
    let token = lakehouse_embed::sign_embed(
        &lakehouse_embed::EmbedClaims {
            resource: Some(lakehouse_embed::EmbedResource {
                dashboard: Some("b_embed".to_owned()),
            }),
            params: None,
            exp: Some(now + 3600.0),
            iat: Some(now),
            jti: None,
        },
        secret,
    );
    let (status, body) = send(
        &app.router,
        Request::builder()
            .method("POST")
            .uri("/api/embed/data")
            .header("content-type", "application/json")
            .body(Body::from(json!({ "jwt": token }).to_string()))
            .expect("build request"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    for marker in [
        "Unknown identifier",
        "UNKNOWN_IDENTIFIER",
        "ch-internal",
        ADDR,
        "version 24.8",
    ] {
        assert!(!body.contains(marker), "{marker} leaked: {body}");
    }
    serde_json::from_str(&body).expect("json")
}

#[tokio::test]
async fn a_failing_layer_in_a_signed_embed_fails_alone_and_the_board_still_renders() {
    let json = combined_embed(json!({
        "mart": "mart_y", "dimension": "place", "measures": ["rooms"],
        "draw": "line", "axis": "right"
    }))
    .await;
    let tile = &json["results"]["u_1"];
    assert_eq!(
        tile["rows"],
        json!([{ "place": "A", "visitors": 5 }]),
        "{json}"
    );
    let layer = &tile["layers"][0];
    assert_eq!(layer["error"], "This chart could not be loaded.", "{json}");
    assert!(layer["errorId"].is_string(), "{json}");
    assert_eq!(layer["rows"], json!([]));
}

#[tokio::test]
async fn a_layer_whose_sql_sources_cannot_be_read_fails_alone_in_a_signed_embed() {
    // The mock fails the `console.bi_source` read like every other unknown one.
    let json = combined_embed(json!({
        "sqlSource": "s_other", "dimension": "place", "measures": ["rooms"],
        "draw": "bar", "axis": "left"
    }))
    .await;
    let tile = &json["results"]["u_1"];
    assert_eq!(
        tile["rows"],
        json!([{ "place": "A", "visitors": 5 }]),
        "{json}"
    );
    assert_eq!(
        tile["layers"][0]["error"], "this question's SQL source could not be read.",
        "{json}"
    );
}
