//! `GET`/`PUT /api/settings/reporting` over the real router (`BI-9`, T1):
//! who may read and write, what is refused with a fixed message, that the
//! engine's zone list decides, and that an upstream failure never puts its
//! own text in the response.
//!
//! `ClickHouse` is a `wiremock` server answering the one `system.time_zones`
//! statement; Postgres is the per-test database from `tests/common`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::collections::HashMap;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use common::{
    TestApp, create_principal_with_permissions, create_zero_permission_principal,
    session_cookie_for_user, spin_up, spin_up_with_env,
};
use serde_json::{Value, json};
use tower::ServiceExt;
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

const BODY_SHAPE: &str = "body must be {\"timeZone\": string, \"weekStart\": \"monday\"|\"sunday\", \
\"numberLocale\"?: \"en-US\"|\"id-ID\", \"companyPalette\"?: [\"#rrggbb\", ...], \"fonts\"?: [string, ...], \"contentSites\"?: [\"https://host\", ...]}";

/// What a deployment that never saved formatting reads back.
fn formatting_defaults(mut body: Value) -> Value {
    body["numberLocale"] = json!("en-US");
    body["companyPalette"] = json!([]);
    body["fonts"] = json!([
        "inter",
        "ibm-plex-sans",
        "source-serif",
        "jetbrains-mono",
        "system"
    ]);
    body["contentSites"] = json!([]);
    body
}

async fn call(app: &axum::Router, verb: &str, cookie: &str, body: &str) -> (StatusCode, Value) {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method(verb)
                .uri("/api/settings/reporting")
                .header("cookie", cookie)
                .header("content-type", "application/json")
                .body(Body::from(body.to_owned()))
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

/// An app whose `ClickHouse` says the engine lists `known` zones (1) or not (0).
async fn app_with_engine(zone_count: u32) -> (TestApp, MockServer) {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_string(format!(
            r#"{{"meta":[{{"name":"n","type":"UInt64"}}],"data":[{{"n":"{zone_count}"}}],"rows":1}}"#
        )))
        .mount(&server)
        .await;
    let mut env = HashMap::new();
    env.insert("CH_URL".to_owned(), server.uri());
    (spin_up_with_env(&env).await, server)
}

#[tokio::test]
async fn with_nothing_saved_a_signed_in_reader_gets_the_defaults() {
    let TestApp { router, pool } = spin_up().await;
    let user = create_zero_permission_principal(&pool).await;
    let cookie = session_cookie_for_user(&pool, user).await;
    let (status, body) = call(&router, "GET", &cookie, "").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body,
        formatting_defaults(
            json!({"timeZone": "Asia/Jakarta", "weekStart": "monday", "saved": false})
        )
    );
}

#[tokio::test]
async fn a_dashboard_editor_without_settings_write_is_refused() {
    let TestApp { router, pool } = spin_up().await;
    let user = create_principal_with_permissions(&pool, "dashboard:write, dashboard:read").await;
    let cookie = session_cookie_for_user(&pool, user).await;
    let (status, _) = call(
        &router,
        "PUT",
        &cookie,
        r#"{"timeZone":"UTC","weekStart":"monday"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = call(&router, "GET", &cookie, "").await;
    assert_eq!(status, StatusCode::OK, "reading needs only a sign-in");
}

#[tokio::test]
async fn a_bad_body_a_third_week_start_and_a_malformed_zone_are_400_with_a_fixed_message() {
    let TestApp { router, pool } = spin_up().await;
    let user = create_principal_with_permissions(&pool, "settings:write").await;
    let cookie = session_cookie_for_user(&pool, user).await;
    let cases = [
        ("not json", BODY_SHAPE),
        (r#"{"timeZone":"UTC"}"#, BODY_SHAPE),
        (
            r#"{"timeZone":"UTC","weekStart":"friday"}"#,
            "weekStart must be monday or sunday.",
        ),
        (
            r#"{"timeZone":"UTC' OR 1=1 --","weekStart":"monday"}"#,
            "Unknown time zone.",
        ),
        (
            r#"{"timeZone":"UTC","weekStart":"monday","numberLocale":"fr-FR"}"#,
            "numberLocale must be en-US or id-ID.",
        ),
        (
            r##"{"timeZone":"UTC","weekStart":"monday","companyPalette":["#112233"]}"##,
            "The company palette needs 2 to 12 colours, or none.",
        ),
        (
            r##"{"timeZone":"UTC","weekStart":"monday","companyPalette":["#112233","blue"]}"##,
            "Every company palette colour must look like #1a2b3c.",
        ),
        (
            r#"{"timeZone":"UTC","weekStart":"monday","fonts":["comic-sans"]}"#,
            "fonts must be chosen from the fonts the console ships.",
        ),
    ];
    for (body, message) in cases {
        let (status, got) = call(&router, "PUT", &cookie, body).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert_eq!(got["error"].as_str(), Some(message), "{body} -> {got}");
    }
}

#[tokio::test]
async fn a_zone_the_engine_does_not_list_is_refused_and_nothing_is_saved() {
    let (TestApp { router, pool }, _server) = app_with_engine(0).await;
    let user = create_principal_with_permissions(&pool, "settings:write").await;
    let cookie = session_cookie_for_user(&pool, user).await;
    let (status, body) = call(
        &router,
        "PUT",
        &cookie,
        r#"{"timeZone":"Mars/Olympus","weekStart":"sunday"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.to_string().contains("Unknown time zone."), "{body}");
    let (_, got) = call(&router, "GET", &cookie, "").await;
    assert_eq!(got["saved"], json!(false));
}

#[tokio::test]
async fn a_known_zone_is_saved_and_read_back_with_the_saved_flag() {
    let (TestApp { router, pool }, _server) = app_with_engine(1).await;
    let writer = create_principal_with_permissions(&pool, "settings:write").await;
    let cookie = session_cookie_for_user(&pool, writer).await;
    let (status, body) = call(
        &router,
        "PUT",
        &cookie,
        r#"{"timeZone":"Europe/Berlin","weekStart":"sunday"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body,
        formatting_defaults(
            json!({"timeZone": "Europe/Berlin", "weekStart": "sunday", "saved": true})
        )
    );
    let reader = create_zero_permission_principal(&pool).await;
    let cookie = session_cookie_for_user(&pool, reader).await;
    let (status, got) = call(&router, "GET", &cookie, "").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(got, body);
}

#[tokio::test]
async fn an_engine_failure_is_reported_without_its_own_text() {
    // The default harness points the engine at a refused connection.
    let TestApp { router, pool } = spin_up().await;
    let user = create_principal_with_permissions(&pool, "settings:write").await;
    let cookie = session_cookie_for_user(&pool, user).await;
    let (status, body) = call(
        &router,
        "PUT",
        &cookie,
        r#"{"timeZone":"UTC","weekStart":"monday"}"#,
    )
    .await;
    assert!(status.is_server_error(), "{status}");
    let text = body.to_string();
    assert!(!text.contains("127.0.0.1"), "{text}");
    assert!(text.contains("Reference:"), "{text}");
}

#[tokio::test]
async fn the_number_locale_palette_and_fonts_are_saved_and_a_later_save_without_them_keeps_them() {
    let (TestApp { router, pool }, _server) = app_with_engine(1).await;
    let writer = create_principal_with_permissions(&pool, "settings:write").await;
    let cookie = session_cookie_for_user(&pool, writer).await;
    let (status, body) = call(
        &router,
        "PUT",
        &cookie,
        r##"{"timeZone":"UTC","weekStart":"monday","numberLocale":"id-ID",
            "companyPalette":["#AABBCC","#112233"],"fonts":["system","inter"]}"##,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["numberLocale"], json!("id-ID"));
    assert_eq!(body["companyPalette"], json!(["#aabbcc", "#112233"]));
    assert_eq!(body["fonts"], json!(["inter", "system"]));

    // The BI-9 form sends only the zone and the first day.
    let (status, again) = call(
        &router,
        "PUT",
        &cookie,
        r#"{"timeZone":"Europe/Berlin","weekStart":"sunday"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{again}");
    assert_eq!(again["numberLocale"], body["numberLocale"]);
    assert_eq!(again["companyPalette"], body["companyPalette"]);
    assert_eq!(again["fonts"], body["fonts"]);

    // An empty palette clears the company palette.
    let (_, cleared) = call(
        &router,
        "PUT",
        &cookie,
        r#"{"timeZone":"UTC","weekStart":"monday","companyPalette":[]}"#,
    )
    .await;
    assert_eq!(cleared["companyPalette"], json!([]));
}

#[tokio::test]
async fn every_shipped_font_and_locale_passes_the_tables_checks() {
    let (TestApp { router, pool }, _server) = app_with_engine(1).await;
    let writer = create_principal_with_permissions(&pool, "settings:write").await;
    let cookie = session_cookie_for_user(&pool, writer).await;
    for locale in lakehouse_bi::formatting::NUMBER_LOCALES {
        let fonts = json!(lakehouse_bi::formatting::SHIPPED_FONTS);
        let body = json!({"timeZone": "UTC", "weekStart": "monday", "numberLocale": locale, "fonts": fonts});
        let (status, got) = call(&router, "PUT", &cookie, &body.to_string()).await;
        assert_eq!(status, StatusCode::OK, "{locale}: {got}");
        assert_eq!(got["fonts"], fonts);
    }
}

#[tokio::test]
async fn content_sites_are_saved_normalised_kept_by_a_later_save_and_refused_when_unfit() {
    let (TestApp { router, pool }, _server) = app_with_engine(1).await;
    let writer = create_principal_with_permissions(&pool, "settings:write").await;
    let cookie = session_cookie_for_user(&pool, writer).await;
    let (status, body) = call(
        &router,
        "PUT",
        &cookie,
        r#"{"timeZone":"UTC","weekStart":"monday",
            "contentSites":["https://Img.Example","https://cdn.example:8443","https://img.example"]}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["contentSites"],
        json!(["https://img.example", "https://cdn.example:8443"])
    );
    // A save without the field keeps the list.
    let (_, again) = call(
        &router,
        "PUT",
        &cookie,
        r#"{"timeZone":"UTC","weekStart":"sunday"}"#,
    )
    .await;
    assert_eq!(again["contentSites"], body["contentSites"]);
    let (_, read) = call(&router, "GET", &cookie, "").await;
    assert_eq!(read["contentSites"], body["contentSites"]);

    let fifty_one: Vec<String> = (0..51).map(|i| format!("https://s{i}.example")).collect();
    for bad in [
        json!(["http://img.example"]),
        json!(["https://*.example"]),
        json!(["https://img.example/path"]),
        json!(["img.example"]),
        json!(fifty_one),
    ] {
        let payload = json!({"timeZone": "UTC", "weekStart": "monday", "contentSites": bad});
        let (status, err) = call(&router, "PUT", &cookie, &payload.to_string()).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}: {err}");
    }
    let (_, after) = call(&router, "GET", &cookie, "").await;
    assert_eq!(
        after["contentSites"], body["contentSites"],
        "a refused save changes nothing"
    );
    // Clearing the list is allowed.
    let (_, cleared) = call(
        &router,
        "PUT",
        &cookie,
        r#"{"timeZone":"UTC","weekStart":"monday","contentSites":[]}"#,
    )
    .await;
    assert_eq!(cleared["contentSites"], json!([]));
}
