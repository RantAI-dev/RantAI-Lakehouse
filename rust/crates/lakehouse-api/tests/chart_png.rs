//! `GET /api/dashboard/chart-export?format=png` (`BI-19` part D, plan D13): the
//! request Gotenberg's screenshot route gets, the token that names one chart,
//! the fixed sentences, the downloads switch, and the audit record. Gotenberg
//! is a `wiremock` server; the real container is proved by the ignored test in
//! `pdf_real_gotenberg.rs`.

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
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request as MockRequest, Respond, ResponseTemplate};

const SECRET: &str = "test-render-secret-not-real";
const CONSOLE: &str = "http://console.test:3000";
const PNG: &[u8] = b"\x89PNG\r\n\x1a\n fake";

/// A `ClickHouse` with one data chart and one text tile on a board.
struct Charts;

impl Respond for Charts {
    fn respond(&self, request: &MockRequest) -> ResponseTemplate {
        let sql = String::from_utf8_lossy(&request.body);
        let row = |id: &str, kind: &str| {
            let spec = json!({ "id": id, "title": "Visits / by place", "kind": kind, "mart": "mart_x", "sql": "SELECT 1", "x": "", "y": "" });
            let def = json!({ "title": "Visits / by place", "mart": "mart_x", "kind": kind });
            json!({
                "id": id, "board": "b1", "created_by": "ui", "created_at": "2026-01-01 00:00:00",
                "spec_json": json!({ "spec": spec, "def": def, "hasYear": false }).to_string(),
            })
        };
        if sql.contains("FROM console.bi_chart") {
            let data = vec![row("c_bar", "bar"), row("c_text", "text")];
            return ResponseTemplate::new(200)
                .set_body_json(json!({ "meta": [], "data": data, "rows": 2 }));
        }
        if sql.contains("toDate(now(") {
            return ResponseTemplate::new(200)
                .set_body_json(json!({ "meta": [], "data": [{ "d": "2031-02-03" }], "rows": 1 }));
        }
        ResponseTemplate::new(200).set_body_string("Ok.\n")
    }
}

struct Fixture {
    app: TestApp,
    gotenberg: MockServer,
    cookie: String,
    _ch: MockServer,
}

async fn fixture(env: &[(&str, Option<&str>)], shot: ResponseTemplate) -> Fixture {
    let ch = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(Charts)
        .mount(&ch)
        .await;
    let gotenberg = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/forms/chromium/screenshot/url"))
        .respond_with(shot)
        .mount(&gotenberg)
        .await;
    let mut vars: HashMap<String, String> = HashMap::from([
        ("CH_URL".to_owned(), ch.uri()),
        ("GOTENBERG_URL".to_owned(), gotenberg.uri()),
        ("CONSOLE_INTERNAL_URL".to_owned(), CONSOLE.to_owned()),
        ("RENDER_TOKEN_SECRET".to_owned(), SECRET.to_owned()),
    ]);
    for (key, value) in env {
        match value {
            Some(v) => vars.insert((*key).to_owned(), (*v).to_owned()),
            None => vars.remove(*key),
        };
    }
    let app = spin_up_with_env(&vars).await;
    let user = create_principal_with_permissions(&app.pool, "dashboard:read").await;
    let cookie = session_cookie_for_user(&app.pool, user).await;
    Fixture {
        app,
        gotenberg,
        cookie,
        _ch: ch,
    }
}

fn ok_png() -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_bytes(PNG.to_vec())
}

async fn get(f: &Fixture, uri: &str) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
    let resp = f
        .app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .uri(uri)
                .header("cookie", &f.cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status();
    let headers = resp.headers().clone();
    let bytes = to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap()
        .to_vec();
    (status, headers, bytes)
}

fn error_of(bytes: &[u8]) -> String {
    serde_json::from_slice::<Value>(bytes).unwrap_or(Value::Null)["error"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

#[tokio::test]
async fn the_png_request_opens_only_the_chart_view_with_a_token_for_that_chart_and_is_audited() {
    let f = fixture(&[], ok_png()).await;
    let (status, headers, body) = get(
        &f,
        "/api/dashboard/chart-export?chart=c_bar&format=png&url=http%3A%2F%2Fevil.invalid%2F",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    assert_eq!(headers["content-type"], "image/png");
    assert!(body.starts_with(b"\x89PNG"));
    let disposition = headers["content-disposition"].to_str().unwrap();
    assert!(
        disposition.starts_with("attachment; filename=\"Visits-by-place-2031-02-03.png"),
        "{disposition}"
    );

    let sent = f.gotenberg.received_requests().await.unwrap();
    assert_eq!(sent.len(), 1);
    let form = String::from_utf8_lossy(&sent[0].body).into_owned();
    assert!(!form.contains("evil.invalid"));
    for field in [
        "name=\"width\"\r\n\r\n1200\r\n",
        "name=\"deviceScaleFactor\"\r\n\r\n2\r\n",
        "name=\"format\"\r\n\r\npng\r\n",
    ] {
        assert!(form.contains(field), "{field}");
    }
    let url_line = form
        .split("name=\"url\"\r\n\r\n")
        .nth(1)
        .and_then(|rest| rest.split("\r\n").next())
        .unwrap();
    let token = url_line
        .strip_prefix(&format!("{CONSOLE}/render/chart/"))
        .unwrap_or_else(|| panic!("the page is the chart view of the console: {url_line}"));
    let claims =
        lakehouse_embed::verify_render(token, SECRET, lakehouse_embed::unix_now()).unwrap();
    assert_eq!(claims.board, "b1");
    assert_eq!(claims.chart.as_deref(), Some("c_bar"));
    let (user_id, args): (String, Value) = sqlx::query_as(
        "SELECT principal_id, args FROM audit_event WHERE action = 'dashboard.chart_png'",
    )
    .fetch_one(&f.app.pool)
    .await
    .unwrap();
    assert_eq!(claims.principal, user_id);
    assert_eq!(args["tile"], "c_bar");
    assert_eq!(args["scale"], 2);
}

#[tokio::test]
async fn a_chart_the_person_cannot_see_or_that_shows_no_data_is_refused_before_the_renderer() {
    let f = fixture(&[], ok_png()).await;
    for (uri, status) in [
        (
            "/api/dashboard/chart-export?format=png",
            StatusCode::BAD_REQUEST,
        ),
        (
            "/api/dashboard/chart-export?chart=c_nobody&format=png",
            StatusCode::NOT_FOUND,
        ),
        (
            "/api/dashboard/chart-export?chart=c_text&format=png",
            StatusCode::BAD_REQUEST,
        ),
        (
            "/api/dashboard/chart-export?chart=c_bar&format=png&filters=nope",
            StatusCode::BAD_REQUEST,
        ),
    ] {
        let (got, _, body) = get(&f, uri).await;
        assert_eq!(got, status, "{uri}: {}", String::from_utf8_lossy(&body));
    }
    assert!(f.gotenberg.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn with_downloads_switched_off_the_png_is_403_and_nothing_is_rendered_or_audited() {
    let f = fixture(&[], ok_png()).await;
    sqlx::query(
        "INSERT INTO reporting_settings (singleton, time_zone, week_start, download_denied_roles) \
         SELECT TRUE, 'UTC', 'monday', array_agg(r.name) FROM role r",
    )
    .execute(&f.app.pool)
    .await
    .unwrap();
    let (status, _, body) = get(&f, "/api/dashboard/chart-export?chart=c_bar&format=png").await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(error_of(&body).contains("Downloads are switched off for your role."));
    assert!(f.gotenberg.received_requests().await.unwrap().is_empty());
    let audited: i64 =
        sqlx::query_scalar("SELECT count(*) FROM audit_event WHERE action = 'dashboard.chart_png'")
            .fetch_one(&f.app.pool)
            .await
            .unwrap();
    assert_eq!(audited, 0);
}

#[tokio::test]
async fn without_the_renderer_settings_images_are_not_set_up_and_failures_are_fixed_sentences() {
    for missing in [
        "GOTENBERG_URL",
        "CONSOLE_INTERNAL_URL",
        "RENDER_TOKEN_SECRET",
    ] {
        let f = fixture(&[(missing, None)], ok_png()).await;
        let (status, _, body) = get(&f, "/api/dashboard/chart-export?chart=c_bar&format=png").await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{missing}");
        assert_eq!(error_of(&body), "Images are not set up on this deployment.");
    }
    let broken = fixture(
        &[],
        ResponseTemplate::new(500).set_body_string("chromium crashed at /srv/internal/path"),
    )
    .await;
    let (status, _, body) = get(
        &broken,
        "/api/dashboard/chart-export?chart=c_bar&format=png",
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    let error = error_of(&body);
    assert!(error.starts_with("The PDF could not be made."), "{error}");
    assert!(!error.contains("chromium") && !error.contains("/srv"));
    let audited: i64 =
        sqlx::query_scalar("SELECT count(*) FROM audit_event WHERE action = 'dashboard.chart_png'")
            .fetch_one(&broken.app.pool)
            .await
            .unwrap();
    assert_eq!(audited, 0);
}
