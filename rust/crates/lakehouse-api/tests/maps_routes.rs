//! `/api/maps/*` at the route level (`BI-16` part C2b, tasks T5 and T6 of
//! `docs/superpowers/plans/2026-10-24-bi-16c-advanced-charts-maps.md`): the real
//! router over a real migrated Postgres and real sessions.
//!
//! `ClickHouse` is the unreachable stand-in of `common::spin_up`, so the one
//! thing these tests cannot show is a chart that uses a map; they show that the
//! delete fails closed when the chart store cannot be read (the map stays).
//! The listing of charts in the 409 is covered by the pure parts of the route.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use tower::ServiceExt;

use std::collections::HashMap;

use wiremock::matchers::{body_string_contains, method};
use wiremock::{Mock, MockServer, ResponseTemplate};

use common::{
    TestApp, create_principal_with_permissions, session_cookie_for_user, spin_up, spin_up_with_env,
};

const FILE: &str = r#"{"type":"FeatureCollection","features":[
 {"type":"Feature","properties":{"NAME":"North"},"geometry":{"type":"Polygon","coordinates":[[[0,0],[1,0],[1,1],[0,1],[0,0]]]}},
 {"type":"Feature","properties":{"NAME":"South"},"geometry":{"type":"Polygon","coordinates":[[[0,-2],[1,-2],[1,-1],[0,-1],[0,-2]]]}}]}"#;

struct Reply {
    status: StatusCode,
    headers: axum::http::HeaderMap,
    body: Vec<u8>,
}

impl Reply {
    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or(Value::Null)
    }
}

fn form(name: &str, property: &str, file: &[u8]) -> (String, Vec<u8>) {
    let mut body = Vec::new();
    for (field, value) in [("name", name), ("nameProperty", property)] {
        body.extend_from_slice(
            format!(
                "--BOUNDARY\r\nContent-Disposition: form-data; name=\"{field}\"\r\n\r\n{value}\r\n"
            )
            .as_bytes(),
        );
    }
    body.extend_from_slice(
        b"--BOUNDARY\r\nContent-Disposition: form-data; name=\"file\"; filename=\"map.geojson\"\r\nContent-Type: application/geo+json\r\n\r\n",
    );
    body.extend_from_slice(file);
    body.extend_from_slice(b"\r\n--BOUNDARY--\r\n");
    ("multipart/form-data; boundary=BOUNDARY".to_owned(), body)
}

async fn send(
    app: &TestApp,
    cookie: &str,
    method: &str,
    uri: &str,
    content: Option<(String, Vec<u8>)>,
    extra: &[(&str, &str)],
) -> Reply {
    let mut request = Request::builder()
        .method(method)
        .uri(uri)
        .header("cookie", cookie);
    for (k, v) in extra {
        request = request.header(*k, *v);
    }
    let body = match content {
        Some((content_type, bytes)) => {
            request = request.header("content-type", content_type);
            Body::from(bytes)
        }
        None => Body::empty(),
    };
    let response = app
        .router
        .clone()
        .oneshot(request.body(body).unwrap())
        .await
        .unwrap();
    let (parts, body) = response.into_parts();
    Reply {
        status: parts.status,
        headers: parts.headers,
        body: to_bytes(body, 64 * 1024 * 1024).await.unwrap().to_vec(),
    }
}

async fn admin(app: &TestApp) -> String {
    let id = create_principal_with_permissions(&app.pool, "settings:write").await;
    session_cookie_for_user(&app.pool, id).await
}

async fn viewer(app: &TestApp) -> String {
    let id = create_principal_with_permissions(&app.pool, "dashboard:read").await;
    session_cookie_for_user(&app.pool, id).await
}

async fn upload(app: &TestApp, cookie: &str, name: &str) -> Reply {
    send(
        app,
        cookie,
        "POST",
        "/api/maps",
        Some(form(name, "NAME", FILE.as_bytes())),
        &[],
    )
    .await
}

#[tokio::test]
async fn an_administrator_uploads_lists_reads_renames_and_the_file_is_tagged() {
    let app = spin_up().await;
    let cookie = admin(&app).await;
    let made = upload(&app, &cookie, "Sales regions").await;
    assert_eq!(
        made.status,
        StatusCode::CREATED,
        "{}",
        String::from_utf8_lossy(&made.body)
    );
    let map = made.json();
    let id = map["id"].as_str().unwrap().to_owned();
    assert!(id.starts_with("m_") && id.len() == 14, "{id}");
    assert_eq!(
        (
            map["name"].as_str(),
            map["nameProperty"].as_str(),
            map["regionCount"].as_i64()
        ),
        (Some("Sales regions"), Some("NAME"), Some(2))
    );

    // Everyone signed in can list and read; the file carries the chosen name as properties.name.
    let reader = viewer(&app).await;
    let listed = send(&app, &reader, "GET", "/api/maps", None, &[]).await;
    assert_eq!(listed.status, StatusCode::OK);
    assert_eq!(listed.json()["maps"][0]["id"], json!(id));
    let uri = format!("/api/maps/{id}/geojson");
    let file = send(&app, &reader, "GET", &uri, None, &[]).await;
    assert_eq!(file.status, StatusCode::OK);
    assert_eq!(file.headers["cache-control"], "private, max-age=3600");
    assert_eq!(file.headers["content-type"], "application/geo+json");
    let drawn = file.json();
    assert_eq!(
        drawn["features"][1]["properties"],
        json!({ "name": "South" })
    );
    let etag = file.headers["etag"].to_str().unwrap().to_owned();
    let again = send(
        &app,
        &reader,
        "GET",
        &uri,
        None,
        &[("if-none-match", &etag)],
    )
    .await;
    assert_eq!(again.status, StatusCode::NOT_MODIFIED);

    // Rename; a second map may not take the name (case ignored).
    let renamed = send(
        &app,
        &cookie,
        "PATCH",
        &format!("/api/maps/{id}"),
        Some((
            "application/json".to_owned(),
            br#"{"name":"Regions"}"#.to_vec(),
        )),
        &[],
    )
    .await;
    assert_eq!(renamed.status, StatusCode::OK);
    assert_eq!(renamed.json()["name"], json!("Regions"));
    // BI-16 review SHOULD-FIX S1: the upload and the rename are audited with the map's id and name, never the file.
    let events: Vec<(String, Option<String>, Value)> = sqlx::query_as(
        "SELECT action, resource_id, args FROM audit_event WHERE resource_kind = 'map' ORDER BY at, id",
    )
    .fetch_all(&app.pool)
    .await
    .unwrap();
    let actions: Vec<&str> = events.iter().map(|e| e.0.as_str()).collect();
    assert_eq!(actions, ["map_upload", "map_rename"], "{events:?}");
    assert_eq!(events[0].1.as_deref(), Some(id.as_str()));
    assert_eq!(events[1].2, json!({ "name": "Regions" }));
    assert!(
        !events
            .iter()
            .any(|e| e.2.to_string().contains("FeatureCollection"))
    );
    let twin = upload(&app, &cookie, "REGIONS").await;
    assert_eq!(twin.status, StatusCode::CONFLICT);
    assert_eq!(
        twin.json()["error"],
        json!("A map with this name already exists.")
    );
}

#[tokio::test]
async fn only_settings_write_changes_maps_and_an_unknown_id_is_a_404() {
    let app = spin_up().await;
    let reader = viewer(&app).await;
    let denied = upload(&app, &reader, "Nope").await;
    assert_eq!(denied.status, StatusCode::FORBIDDEN);
    let cookie = admin(&app).await;
    for uri in [
        "/api/maps/m_aaaaaaaaaaaa/geojson",
        "/api/maps/not-an-id/geojson",
    ] {
        assert_eq!(
            send(&app, &reader, "GET", uri, None, &[]).await.status,
            StatusCode::NOT_FOUND
        );
    }
    let gone = send(
        &app,
        &cookie,
        "DELETE",
        "/api/maps/m_aaaaaaaaaaaa",
        None,
        &[],
    )
    .await;
    assert_eq!(gone.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_refused_file_is_a_400_with_a_plain_sentence_and_nothing_is_stored() {
    let app = spin_up().await;
    let cookie = admin(&app).await;
    let point = br#"{"type":"FeatureCollection","features":[{"type":"Feature","properties":{"NAME":"P"},"geometry":{"type":"Point","coordinates":[1,2]}}]}"#;
    let reply = send(
        &app,
        &cookie,
        "POST",
        "/api/maps",
        Some(form("P", "NAME", point)),
        &[],
    )
    .await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    assert!(
        reply.json()["error"]
            .as_str()
            .unwrap()
            .contains("feature 1 is not a polygon")
    );
    let wrong = send(
        &app,
        &cookie,
        "POST",
        "/api/maps",
        Some(form("P", "TITLE", FILE.as_bytes())),
        &[],
    )
    .await;
    assert!(wrong.json()["error"].as_str().unwrap().contains("'TITLE'"));
    let blank = send(
        &app,
        &cookie,
        "POST",
        "/api/maps",
        Some(form("  ", "NAME", FILE.as_bytes())),
        &[],
    )
    .await;
    assert_eq!(blank.status, StatusCode::BAD_REQUEST);
    let listed = send(&app, &cookie, "GET", "/api/maps", None, &[]).await;
    assert_eq!(listed.json()["maps"], json!([]));
}

#[tokio::test]
async fn a_file_over_50_mb_is_refused_before_it_is_parsed() {
    let app = spin_up().await;
    let cookie = admin(&app).await;
    // Not JSON at all: had it been parsed, the sentence would say so.
    let big = vec![b'a'; 50 * 1024 * 1024 + 1];
    let reply = send(
        &app,
        &cookie,
        "POST",
        "/api/maps",
        Some(form("Big", "NAME", &big)),
        &[],
    )
    .await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        reply.json()["error"],
        json!("This file is over 50 MB. Simplify it or split it, then upload again.")
    );
}

#[tokio::test]
async fn a_delete_that_cannot_check_the_charts_fails_closed_and_keeps_the_map() {
    let app = spin_up().await;
    let cookie = admin(&app).await;
    let id = upload(&app, &cookie, "Keep me").await.json()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let reply = send(
        &app,
        &cookie,
        "DELETE",
        &format!("/api/maps/{id}"),
        None,
        &[],
    )
    .await;
    assert!(!reply.status.is_success(), "{}", reply.status);
    let listed = send(&app, &cookie, "GET", "/api/maps", None, &[]).await;
    assert_eq!(listed.json()["maps"].as_array().unwrap().len(), 1);
}

// ── the region lookup on a mock engine (`BI-16` C3 review S5 a, S3) ──────

/// An app whose engine is a mock that answers every statement with an empty
/// result, except the ones a test mounts first.
async fn with_engine() -> (TestApp, MockServer) {
    let engine = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({ "meta": [], "data": [], "rows": 0 })),
        )
        .mount(&engine)
        .await;
    let env = HashMap::from([("CH_URL".to_owned(), engine.uri())]);
    (spin_up_with_env(&env).await, engine)
}

async fn statements(engine: &MockServer) -> Vec<String> {
    engine
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(|r| String::from_utf8_lossy(&r.body).into_owned())
        .collect()
}

fn position(sent: &[String], needle: &str) -> usize {
    sent.iter()
        .position(|s| s.contains(needle))
        .unwrap_or_else(|| panic!("no statement contains {needle:?}: {sent:?}"))
}

async fn region_lookup_of(app: &TestApp, id: &str) -> Option<bool> {
    sqlx::query_scalar("SELECT region_lookup FROM geo_map WHERE id = $1")
        .bind(id)
        .fetch_optional(&app.pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn an_upload_creates_the_region_lookup_in_order_and_a_delete_drops_it() {
    let (app, engine) = with_engine().await;
    let cookie = admin(&app).await;
    let made = upload(&app, &cookie, "Sales regions").await;
    assert_eq!(made.status, StatusCode::CREATED);
    let id = made.json()["id"].as_str().unwrap().to_owned();
    let sent = statements(&engine).await;
    let order = [
        position(&sent, "CREATE TABLE IF NOT EXISTS console.geo_region"),
        position(&sent, "INSERT INTO console.geo_region"),
        position(
            &sent,
            &format!("CREATE DICTIONARY IF NOT EXISTS console.geo_dict_{id}"),
        ),
        position(
            &sent,
            &format!("SYSTEM RELOAD DICTIONARY console.geo_dict_{id}"),
        ),
    ];
    assert!(order.windows(2).all(|w| w[0] < w[1]), "{order:?}");
    // The full boundaries, both regions, and the dictionary reads as the engine user.
    assert!(sent[order[1]].contains("North") && sent[order[1]].contains("South"));
    assert!(
        sent[order[2]].contains("USER 'default' PASSWORD '' QUERY"),
        "{}",
        sent[order[2]]
    );
    assert_eq!(region_lookup_of(&app, &id).await, Some(true));

    let gone = send(
        &app,
        &cookie,
        "DELETE",
        &format!("/api/maps/{id}"),
        None,
        &[],
    )
    .await;
    assert_eq!(gone.status, StatusCode::OK);
    let sent = statements(&engine).await;
    position(
        &sent,
        &format!("DROP DICTIONARY IF EXISTS console.geo_dict_{id}"),
    );
    position(
        &sent,
        &format!("ALTER TABLE console.geo_region DELETE WHERE map_id = '{id}'"),
    );
    assert_eq!(region_lookup_of(&app, &id).await, None);
}

#[tokio::test]
async fn an_engine_that_refuses_the_dictionary_still_gives_201_with_the_lookup_off_and_undone() {
    let (app, engine) = with_engine().await;
    Mock::given(method("POST"))
        .and(body_string_contains("CREATE DICTIONARY"))
        .respond_with(
            ResponseTemplate::new(500).set_body_string("Code: 36. DB::Exception: refused"),
        )
        .with_priority(1)
        .mount(&engine)
        .await;
    let cookie = admin(&app).await;
    let made = upload(&app, &cookie, "Sales regions").await;
    assert_eq!(made.status, StatusCode::CREATED);
    // Upstream text never reaches the caller (SEC-11).
    assert!(!String::from_utf8_lossy(&made.body).contains("DB::Exception"));
    let id = made.json()["id"].as_str().unwrap().to_owned();
    assert_eq!(region_lookup_of(&app, &id).await, Some(false));
    let sent = statements(&engine).await;
    position(
        &sent,
        &format!("DROP DICTIONARY IF EXISTS console.geo_dict_{id}"),
    );
    position(
        &sent,
        &format!("ALTER TABLE console.geo_region DELETE WHERE map_id = '{id}'"),
    );
}

#[tokio::test]
async fn a_calculated_field_that_names_the_map_blocks_its_delete_by_id_or_by_name() {
    // BI-16 C3 review S3.
    for formula in [
        "RegionOf('Sales regions', lat, lon)",
        "RegionOf('{id}', lat, lon)",
    ] {
        let (app, engine) = with_engine().await;
        let cookie = admin(&app).await;
        let id = upload(&app, &cookie, "Sales regions").await.json()["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let row = json!({
            "id": "f_00000001", "source_kind": "mart", "source_id": "mart_visits",
            "name": "region", "formula": formula.replace("{id}", &id), "level": "row",
            "data_type": "text", "created_by": "", "updated_at": "2026-10-01 10:00:00",
        });
        Mock::given(method("POST"))
            .and(body_string_contains("FROM console.bi_field"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({ "meta": [], "data": [row], "rows": 1 })),
            )
            .with_priority(1)
            .mount(&engine)
            .await;
        let reply = send(
            &app,
            &cookie,
            "DELETE",
            &format!("/api/maps/{id}"),
            None,
            &[],
        )
        .await;
        assert_eq!(reply.status, StatusCode::CONFLICT, "{formula}");
        assert_eq!(reply.json()["fields"], json!(["region"]));
        assert!(reply.json()["error"].as_str().unwrap().contains("RegionOf"));
        assert_eq!(region_lookup_of(&app, &id).await, Some(true));
    }
}

// ── a region chart on an uploaded map (`BI-16` C2b QA fix Q1) ────────────

/// The builder refused every uploaded map with "invalid map id" because the
/// check only knew the bundled shape. The mock engine knows no mart, so the
/// preview stops later with another sentence; what is shown is that the map id
/// is not what stops it, while a malformed id still is.
#[tokio::test]
async fn a_region_chart_on_an_uploaded_map_is_not_refused_for_its_id() {
    let (app, _engine) = with_engine().await;
    let admin_cookie = admin(&app).await;
    let id = upload(&app, &admin_cookie, "Zona Demo").await.json()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let editor = session_cookie_for_user(
        &app.pool,
        create_principal_with_permissions(&app.pool, "dashboard:read,dashboard:write").await,
    )
    .await;
    let preview = |map: String| {
        let (app, editor) = (&app, editor.clone());
        async move {
            let body = json!({
                "title": "Regions", "kind": "geomap", "mart": "mart_visits",
                "dimension": "region", "measures": ["visitors"], "map": map,
            });
            send(
                app,
                &editor,
                "POST",
                "/api/dashboard/specs/preview",
                Some(("application/json".to_owned(), body.to_string().into_bytes())),
                &[],
            )
            .await
        }
    };
    let ok = preview(id.clone()).await;
    assert!(
        !String::from_utf8_lossy(&ok.body).contains("invalid map id"),
        "{}",
        String::from_utf8_lossy(&ok.body)
    );
    let bad = preview("m_XYZ".to_owned()).await;
    assert_eq!(bad.status, StatusCode::BAD_REQUEST);
    assert!(String::from_utf8_lossy(&bad.body).contains("invalid map id"));
}
