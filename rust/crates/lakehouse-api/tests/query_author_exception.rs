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
    let overrides: HashMap<String, String> = env
        .iter()
        .map(|(k, v)| ((*k).to_owned(), v.clone()))
        .collect();
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
        let rows = |row: Value| {
            ResponseTemplate::new(200).set_body_json(json!({
                "meta": [], "data": [row], "rows": 1
            }))
        };
        if sql.starts_with("CREATE") || sql.starts_with("ALTER") {
            return ResponseTemplate::new(200);
        }
        if sql.contains("FROM console.bi_board") {
            return rows(json!({
                "id": "b_embed", "name": "Embedded", "description": "", "created_by": "t",
                "layout_json": "", "filters_json": "", "public_token": "", "embed_enabled": "1",
                "folder_id": "", "created_at": "2026-10-10 00:00:00"
            }));
        }
        if sql.contains("FROM console.bi_chart") {
            let spec = r#"{"spec":{"id":"u_1","title":"T","kind":"bar","mart":"mart_x","sql":"SELECT 1","x":"place","y":"visitors"},"def":{"title":"T","mart":"mart_x","kind":"bar","dimension":"place","measures":["visitors"]}}"#;
            return rows(json!({
                "id": "u_1", "spec_json": spec, "board": "b_embed", "created_by": "t",
                "created_at": "2026-10-10 00:00:00"
            }));
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
    let token = lakehouse_embed::sign_embed(
        &lakehouse_embed::EmbedClaims {
            resource: Some(lakehouse_embed::EmbedResource {
                dashboard: Some("b_embed".to_owned()),
            }),
            params: None,
            exp: None,
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
