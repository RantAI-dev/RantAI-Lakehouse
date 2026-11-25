//! `GET /api/dashboard/export/pdf` (`BI-1`, T4, plan D5): the request Gotenberg
//! gets, what the renderer may open, the fixed sentences, the slots and the
//! audit record. Gotenberg is a `wiremock` server; the real container is
//! proved by `pdf_real_gotenberg.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use crate::common;

use std::collections::HashMap;
use std::time::{Duration, Instant};

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use common::{
    TestApp, create_principal_with_permissions, session_cookie_for_user, spin_up_with_env,
};
use serde_json::{Value, json};
use tower::ServiceExt;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request as MockRequest, Respond, ResponseTemplate};

/// `RENDER_TOKEN_SECRET`; `EMBED_SECRET` is not set at all (BI-1 review fix
/// (BLOCKER) R1: PDF export does not depend on it).
const SECRET: &str = "test-render-secret-not-real";
const CONSOLE: &str = "http://console.test:3000";
const BOARD: &str = "b_pdf";

/// A `ClickHouse` with one board the viewer may see.
struct BoardOnly;

impl Respond for BoardOnly {
    fn respond(&self, request: &MockRequest) -> ResponseTemplate {
        let sql = String::from_utf8_lossy(&request.body);
        if sql.contains("FROM console.bi_board") && sql.contains(&format!("id='{BOARD}'")) {
            let row = json!({
                "id": BOARD, "name": "Sales / Q3 report", "description": "", "created_by": "t",
                "layout_json": "", "filters_json": "", "public_token": "", "embed_enabled": "0",
                "folder_id": "", "created_at": "2026-10-10 00:00:00", "owner_id": "",
                "embed_revoked_before": "0", "embed_revoked_jti_json": "[]",
                "embed_origins_json": "[]"
            });
            return ResponseTemplate::new(200)
                .set_body_json(json!({ "meta": [], "data": [row], "rows": 1 }));
        }
        // "Today" in the report zone: the engine is asked, and answers a date
        // that is not today's UTC date, so the file name must carry this one.
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
    user: uuid::Uuid,
    _ch: MockServer,
}

async fn fixture(env: &[(&str, Option<&str>)], pdf: ResponseTemplate) -> Fixture {
    let ch = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(BoardOnly)
        .mount(&ch)
        .await;
    let gotenberg = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/forms/chromium/convert/url"))
        .respond_with(pdf)
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
        user,
        _ch: ch,
    }
}

fn ok_pdf() -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_bytes(b"%PDF-1.7 fake".to_vec())
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

fn text(bytes: &[u8]) -> Value {
    serde_json::from_slice(bytes).unwrap_or(Value::Null)
}

async fn get_as(
    f: &Fixture,
    uri: &str,
    tenant: &str,
) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
    let resp = f
        .app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .uri(uri)
                .header("cookie", &f.cookie)
                .header("x-tenant", tenant)
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

/// The minted token of this PDF from the Gotenberg request, for the tests
/// that need to read what the print view will be allowed to do.
async fn token_sent(f: &Fixture) -> String {
    let sent = f.gotenberg.received_requests().await.unwrap();
    let form = String::from_utf8_lossy(&sent[0].body).into_owned();
    let url_line = form
        .split("name=\"url\"\r\n\r\n")
        .nth(1)
        .and_then(|rest| rest.split("\r\n").next())
        .unwrap();
    url_line
        .strip_prefix(&format!("{CONSOLE}/render/dashboard/"))
        .unwrap()
        .to_owned()
}

#[tokio::test]
async fn the_pdf_request_opens_only_the_console_print_view_with_a_token_for_this_person() {
    let f = fixture(&[], ok_pdf()).await;
    let filters = r#"[{"column":"region","values":["north"]}]"#;
    let uri = format!(
        "/api/dashboard/export/pdf?board={BOARD}&paper=letter&orientation=landscape&filters={}&url=http%3A%2F%2Fevil.invalid%2F",
        filters
            .replace('{', "%7B")
            .replace('}', "%7D")
            .replace('[', "%5B")
            .replace(']', "%5D")
            .replace('"', "%22")
    );
    let (status, headers, body) = get(&f, &uri).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    assert_eq!(headers["content-type"], "application/pdf");
    assert!(body.starts_with(b"%PDF"));
    let disposition = headers["content-disposition"].to_str().unwrap();
    assert!(
        disposition.starts_with("attachment; filename=\"Sales-Q3-report-")
            && disposition.ends_with(".pdf\""),
        "{disposition}"
    );

    let sent = f.gotenberg.received_requests().await.unwrap();
    assert_eq!(sent.len(), 1);
    let form = String::from_utf8_lossy(&sent[0].body).into_owned();
    // The page is the configured console plus the print-view path, and nothing
    // from the query string (`url=` was ignored).
    let url_line = form
        .split("name=\"url\"\r\n\r\n")
        .nth(1)
        .and_then(|rest| rest.split("\r\n").next())
        .unwrap();
    let token = url_line
        .strip_prefix(&format!("{CONSOLE}/render/dashboard/"))
        .unwrap_or_else(|| panic!("the page is the print view of the console: {url_line}"));
    assert!(!form.contains("evil.invalid"));
    assert!(form.contains("name=\"paperWidth\"\r\n\r\n8.5\r\n"));
    assert!(form.contains("name=\"landscape\"\r\n\r\ntrue\r\n"));
    assert!(form.contains("window.__lakehouseRenderReady === true"));
    // The token names this board, this person, these filters, two minutes.
    let claims =
        lakehouse_embed::verify_render(token, SECRET, lakehouse_embed::unix_now()).unwrap();
    assert_eq!(claims.board, BOARD);
    assert_eq!(claims.filters.as_deref(), Some(filters));
    assert_eq!(claims.paper, "letter");
    assert_eq!(claims.orientation, "landscape");
    assert!(
        (claims.exp - claims.iat - 120.0).abs() < f64::EPSILON,
        "two minutes"
    );
    let (user_id,): (String,) = sqlx::query_as(
        "SELECT principal_id FROM audit_event WHERE action = 'dashboard.pdf_export'",
    )
    .fetch_one(&f.app.pool)
    .await
    .unwrap();
    assert_eq!(
        claims.principal, user_id,
        "the person who asked, audited under the same id"
    );
}

/// `SEC-54` (SEC-50 slice 2 M5): the token names the tenant the caller worked
/// under, which is the tenant the print view reads with — not the person's
/// first tenant. The seeded ids sort so the catalog tenant is first.
#[tokio::test]
async fn the_pdf_token_carries_the_second_tenant_the_caller_exported_under() {
    let f = fixture(
        &[("CATALOG_TENANT_ID", Some(common::CATALOG_TENANT))],
        ok_pdf(),
    )
    .await;
    let second = "11111111-1111-4111-8111-000000000002";
    common::in_the_catalog_tenant(&f.app.pool, f.user).await;
    sqlx::query(
        "INSERT INTO app_user_tenant (user_id, tenant_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
    )
    .bind(f.user)
    .bind(second.parse::<uuid::Uuid>().unwrap())
    .execute(&f.app.pool)
    .await
    .unwrap();

    let (status, _, body) = get_as(
        &f,
        &format!("/api/dashboard/export/pdf?board={BOARD}"),
        second,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let claims =
        lakehouse_embed::verify_render(&token_sent(&f).await, SECRET, lakehouse_embed::unix_now())
            .unwrap();
    assert_eq!(claims.tenant.as_deref(), Some(second), "{claims:?}");
    assert_eq!(claims.principal, f.user.to_string());
}

/// `SEC-54`: a caller cannot mint a token for a tenant they do not belong
/// to. `tenant_scope::resolve` refuses the forged header before the token is
/// signed, so Gotenberg is never asked to open anything.
#[tokio::test]
async fn a_pdf_request_for_a_tenant_the_caller_is_not_in_is_refused_before_minting() {
    let f = fixture(
        &[("CATALOG_TENANT_ID", Some(common::CATALOG_TENANT))],
        ok_pdf(),
    )
    .await;
    let (status, _, body) = get_as(
        &f,
        &format!("/api/dashboard/export/pdf?board={BOARD}"),
        &uuid::Uuid::new_v4().to_string(),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "{}",
        String::from_utf8_lossy(&body)
    );
    assert!(
        f.gotenberg.received_requests().await.unwrap().is_empty(),
        "no token was minted and no page was sent to the renderer"
    );
}

#[tokio::test]
async fn the_download_is_audited_with_its_paper_and_size() {
    let f = fixture(&[], ok_pdf()).await;
    let (status, _, _) = get(&f, &format!("/api/dashboard/export/pdf?board={BOARD}")).await;
    assert_eq!(status, StatusCode::OK);
    let args: Value =
        sqlx::query_scalar("SELECT args FROM audit_event WHERE action = 'dashboard.pdf_export'")
            .fetch_one(&f.app.pool)
            .await
            .unwrap();
    assert_eq!(args["board"], BOARD);
    assert_eq!(args["paper"], "a4");
    assert_eq!(args["orientation"], "portrait");
    assert_eq!(args["bytes"], 13);
}

#[tokio::test]
async fn without_gotenberg_the_console_url_or_the_secret_pdf_is_not_set_up() {
    for missing in [
        "GOTENBERG_URL",
        "CONSOLE_INTERNAL_URL",
        "RENDER_TOKEN_SECRET",
    ] {
        for blank in [None, Some("")] {
            let f = fixture(&[(missing, blank)], ok_pdf()).await;
            let (status, _, body) =
                get(&f, &format!("/api/dashboard/export/pdf?board={BOARD}")).await;
            assert_eq!(
                status,
                StatusCode::SERVICE_UNAVAILABLE,
                "{missing} {blank:?}"
            );
            assert_eq!(
                text(&body)["error"],
                "PDF export is not set up on this deployment"
            );
            assert!(f.gotenberg.received_requests().await.unwrap().is_empty());
        }
    }
}

#[tokio::test]
async fn a_gotenberg_that_is_down_or_fails_gets_a_fixed_sentence_and_never_its_text() {
    let down = fixture(&[("GOTENBERG_URL", Some("http://127.0.0.1:1"))], ok_pdf()).await;
    let (status, _, body) = get(&down, &format!("/api/dashboard/export/pdf?board={BOARD}")).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    let error = text(&body)["error"].as_str().unwrap().to_owned();
    assert!(
        error.starts_with("The PDF service is not reachable."),
        "{error}"
    );
    assert!(!error.contains("127.0.0.1"));

    let broken = fixture(
        &[],
        ResponseTemplate::new(500).set_body_string("chromium crashed at /srv/internal/path"),
    )
    .await;
    let (status, _, body) = get(&broken, &format!("/api/dashboard/export/pdf?board={BOARD}")).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    let error = text(&body)["error"].as_str().unwrap().to_owned();
    assert!(error.starts_with("The PDF could not be made."), "{error}");
    assert!(!error.contains("chromium") && !error.contains("/srv"));
    // Nothing was audited for a PDF that was not handed over.
    let audited: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_event WHERE action = 'dashboard.pdf_export'",
    )
    .fetch_one(&broken.app.pool)
    .await
    .unwrap();
    assert_eq!(audited, 0);
}

#[tokio::test]
async fn bad_input_a_hidden_board_and_switched_off_roles_are_refused_before_gotenberg_is_asked() {
    let f = fixture(&[], ok_pdf()).await;
    for (uri, status) in [
        (
            "/api/dashboard/export/pdf".to_owned(),
            StatusCode::BAD_REQUEST,
        ),
        (
            format!("/api/dashboard/export/pdf?board={BOARD}&paper=a3"),
            StatusCode::BAD_REQUEST,
        ),
        (
            format!("/api/dashboard/export/pdf?board={BOARD}&orientation=tilted"),
            StatusCode::BAD_REQUEST,
        ),
        (
            format!("/api/dashboard/export/pdf?board={BOARD}&filters=nope"),
            StatusCode::BAD_REQUEST,
        ),
        (
            "/api/dashboard/export/pdf?board=nobody".to_owned(),
            StatusCode::NOT_FOUND,
        ),
    ] {
        let (got, _, body) = get(&f, &uri).await;
        assert_eq!(got, status, "{uri}: {}", String::from_utf8_lossy(&body));
    }
    sqlx::query(
        "INSERT INTO reporting_settings (singleton, time_zone, week_start, download_denied_roles) \
         SELECT TRUE, 'UTC', 'monday', array_agg(r.name) FROM role r",
    )
    .execute(&f.app.pool)
    .await
    .unwrap();
    let (status, _, body) = get(&f, &format!("/api/dashboard/export/pdf?board={BOARD}")).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(String::from_utf8_lossy(&body).contains("Downloads are switched off for your role."));
    assert!(f.gotenberg.received_requests().await.unwrap().is_empty());
}

/// Three requests at once: two run, the third waits for a slot (feature
/// decision 5). With a Gotenberg that takes 600 ms each, three finish in two
/// waves, not one and not three.
#[tokio::test]
async fn at_most_two_pdfs_are_made_at_once_and_a_third_waits() {
    let delay = Duration::from_millis(600);
    let f = fixture(&[], ok_pdf().set_delay(delay)).await;
    let started = Instant::now();
    let uri = format!("/api/dashboard/export/pdf?board={BOARD}");
    let (a, b, c) = tokio::join!(get(&f, &uri), get(&f, &uri), get(&f, &uri));
    let elapsed = started.elapsed();
    for (status, _, body) in [a, b, c] {
        assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    }
    assert_eq!(f.gotenberg.received_requests().await.unwrap().len(), 3);
    assert!(elapsed >= delay * 2, "two waves at least: {elapsed:?}");
    assert!(
        elapsed < delay * 3 + Duration::from_millis(900),
        "not three in a row: {elapsed:?}"
    );
}

/// `BI-1` review fix (SHOULD-FIX) R3: a service identity cannot ask for a PDF; it
/// is told so, and Gotenberg is not asked.
#[tokio::test]
async fn a_service_token_is_refused_in_a_sentence_and_gotenberg_is_not_asked() {
    const TOKEN: &str = "svc-pdf-fixture-token";
    let f = fixture(&[], ok_pdf()).await;
    let identity = lakehouse_store::identity::create_service_identity(
        &f.app.pool,
        &lakehouse_store::identity::CreateServiceIdentityInput {
            name: "test-pdf".to_owned(),
            scopes: vec!["dashboard:read".to_owned()],
            environment: "test".to_owned(),
        },
    )
    .await
    .unwrap();
    lakehouse_auth::service_token::ensure_service_credential(
        &f.app.pool,
        identity.id.parse().unwrap(),
        &lakehouse_auth::Secret::new(TOKEN.to_owned()),
    )
    .await
    .unwrap();
    let resp = f
        .app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/dashboard/export/pdf?board={BOARD}"))
                .header("authorization", format!("Bearer {TOKEN}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    assert_eq!(
        text(&bytes)["error"],
        "A PDF is made for a signed-in person; a service token cannot ask for one."
    );
    assert!(f.gotenberg.received_requests().await.unwrap().is_empty());
}

/// R3: Gotenberg failing the conversion (a print view that raised an exception,
/// HTTP 409) is "The PDF could not be made." and is not audited or sent.
#[tokio::test]
async fn a_conversion_gotenberg_fails_is_not_handed_over() {
    let f = fixture(
        &[],
        ResponseTemplate::new(409).set_body_string("console exceptions: render data refused"),
    )
    .await;
    let (status, _, body) = get(&f, &format!("/api/dashboard/export/pdf?board={BOARD}")).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    let error = text(&body)["error"].as_str().unwrap().to_owned();
    assert!(error.starts_with("The PDF could not be made."), "{error}");
    assert!(!error.contains("console exceptions"));
    let audited: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_event WHERE action = 'dashboard.pdf_export'",
    )
    .fetch_one(&f.app.pool)
    .await
    .unwrap();
    assert_eq!(audited, 0);
}
