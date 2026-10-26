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
\"numberLocale\"?: \"en-US\"|\"id-ID\", \"companyPalette\"?: [\"#rrggbb\", ...], \"fonts\"?: [string, ...], \"contentSites\"?: [\"https://host\", ...], \"downloadDeniedRoles\"?: [string, ...], \"reportAllowedDomains\"?: [string, ...]}";

/// What a deployment that never saved formatting reads back.
///
/// The two lists only an administrator reads (`PR #204` review (SHOULD-FIX) 3)
/// are not part of it: a reader without `settings:write` does not get them.
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

/// The two lists a caller with `settings:write` reads on top of
/// [`formatting_defaults`].
fn administrator_lists(mut body: Value) -> Value {
    body["downloadDeniedRoles"] = json!([]);
    body["reportAllowedDomains"] = json!([]);
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

/// `PR #204` review (SHOULD-FIX) 3: the two lists only an administrator reads
/// are in the answer for a caller with `settings:write` and are not in it for
/// anyone else. The content sites stay: every signed-in page's
/// Content-Security-Policy reads them from this answer.
#[tokio::test]
async fn the_two_administrator_lists_are_given_only_to_a_caller_with_settings_write() {
    let TestApp { router, pool } = spin_up().await;
    let admin = create_principal_with_permissions(&pool, "settings:write").await;
    let admin_cookie = session_cookie_for_user(&pool, admin).await;
    let (status, body) = call(&router, "GET", &admin_cookie, "").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["downloadDeniedRoles"], json!([]));
    assert_eq!(body["reportAllowedDomains"], json!([]));
    assert_eq!(body["contentSites"], json!([]), "the sites are everybody's");

    let reader = create_principal_with_permissions(&pool, "dashboard:read").await;
    let reader_cookie = session_cookie_for_user(&pool, reader).await;
    let (status, body) = call(&router, "GET", &reader_cookie, "").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.get("downloadDeniedRoles").is_none(),
        "the roles that may not download are not for everybody: {body}"
    );
    assert!(
        body.get("reportAllowedDomains").is_none(),
        "the outside report domains are not for everybody: {body}"
    );
    assert_eq!(body["contentSites"], json!([]), "the sites are everybody's");
    assert_eq!(body["timeZone"], json!("Asia/Jakarta"));
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
        administrator_lists(formatting_defaults(
            json!({"timeZone": "Europe/Berlin", "weekStart": "sunday", "saved": true})
        ))
    );
    let reader = create_zero_permission_principal(&pool).await;
    let cookie = session_cookie_for_user(&pool, reader).await;
    let (status, got) = call(&router, "GET", &cookie, "").await;
    assert_eq!(status, StatusCode::OK);
    // The reader gets the same settings, without the two lists only an
    // administrator reads (`PR #204` review (SHOULD-FIX) 3).
    assert_eq!(
        got,
        formatting_defaults(
            json!({"timeZone": "Europe/Berlin", "weekStart": "sunday", "saved": true})
        )
    );
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

/// `BI-1` T1 (D7): the list of roles that may not download is saved by name,
/// kept by a save that omits it, refused when a name is not a role, and needs
/// `settings:write`.
#[tokio::test]
async fn download_denied_roles_are_saved_kept_and_refused_when_not_a_role() {
    let (TestApp { router, pool }, _server) = app_with_engine(1).await;
    let writer = create_principal_with_permissions(&pool, "settings:write").await;
    let cookie = session_cookie_for_user(&pool, writer).await;
    let (status, body) = call(
        &router,
        "PUT",
        &cookie,
        r#"{"timeZone":"UTC","weekStart":"monday","downloadDeniedRoles":["Analyst","Analyst"]}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["downloadDeniedRoles"], json!(["Analyst"]));
    // A save without the field keeps the list.
    let (_, again) = call(
        &router,
        "PUT",
        &cookie,
        r#"{"timeZone":"UTC","weekStart":"sunday"}"#,
    )
    .await;
    assert_eq!(again["downloadDeniedRoles"], json!(["Analyst"]));

    let (status, err) = call(
        &router,
        "PUT",
        &cookie,
        r#"{"timeZone":"UTC","weekStart":"monday","downloadDeniedRoles":["No Such Role"]}"#,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{err}");
    assert_eq!(
        err["error"].as_str(),
        Some("downloadDeniedRoles must name roles that exist.")
    );
    let (_, after) = call(&router, "GET", &cookie, "").await;
    assert_eq!(
        after["downloadDeniedRoles"],
        json!(["Analyst"]),
        "a refused save changes nothing"
    );

    let (_, cleared) = call(
        &router,
        "PUT",
        &cookie,
        r#"{"timeZone":"UTC","weekStart":"monday","downloadDeniedRoles":[]}"#,
    )
    .await;
    assert_eq!(cleared["downloadDeniedRoles"], json!([]));

    let reader = create_principal_with_permissions(&pool, "dashboard:read").await;
    let reader_cookie = session_cookie_for_user(&pool, reader).await;
    let (status, _) = call(
        &router,
        "PUT",
        &reader_cookie,
        r#"{"timeZone":"UTC","weekStart":"monday","downloadDeniedRoles":["Analyst"]}"#,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

async fn post_chart(
    app: &axum::Router,
    cookie: &str,
    verb: &str,
    body: &Value,
) -> (StatusCode, Value) {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method(verb)
                .uri("/api/dashboard/specs")
                .header("cookie", cookie)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
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

#[tokio::test]
async fn an_image_or_web_card_is_saved_only_for_a_site_the_administrator_listed() {
    let (TestApp { router, pool }, _server) = app_with_engine(1).await;
    let admin = create_principal_with_permissions(&pool, "settings:write").await;
    let admin_cookie = session_cookie_for_user(&pool, admin).await;
    let editor = create_principal_with_permissions(&pool, "dashboard:write, dashboard:read").await;
    let cookie = session_cookie_for_user(&pool, editor).await;
    let image = |url: &str| json!({"title": "Logo", "kind": "image", "url": url, "alt": "A logo"});

    // Nothing is listed: nothing can be saved (fail closed).
    let (status, body) = post_chart(
        &router,
        &cookie,
        "POST",
        &image("https://img.example/a.png"),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(
        body["error"],
        json!(lakehouse_bi::types_b::CONTENT_SITE_REFUSED)
    );

    let (status, _) = call(
        &router,
        "PUT",
        &admin_cookie,
        r#"{"timeZone":"UTC","weekStart":"monday","contentSites":["https://img.example"]}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, body) = post_chart(
        &router,
        &cookie,
        "POST",
        &image("https://img.example/a.png"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["chart"]["kind"], json!("image"));

    for bad in [
        "https://other.example/a.png",
        "http://img.example/a.png",
        "https://img.example@evil.example/a.png",
    ] {
        let (status, body) = post_chart(&router, &cookie, "POST", &image(bad)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}: {body}");
        let message = body["error"].as_str().unwrap_or_default();
        assert_eq!(message, lakehouse_bi::types_b::CONTENT_SITE_REFUSED);
        assert!(
            !message.contains(bad),
            "the address is not echoed: {message}"
        );
    }
    // A web card takes the same list.
    let (status, _) = post_chart(
        &router,
        &cookie,
        "POST",
        &json!({"title": "Status", "kind": "web", "url": "https://img.example/status"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = post_chart(
        &router,
        &cookie,
        "POST",
        &json!({"title": "Status", "kind": "web", "url": "https://nope.example/status"}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}
