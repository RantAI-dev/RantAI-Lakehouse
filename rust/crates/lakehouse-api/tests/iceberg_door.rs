//! DATA-5 / ADR 0016: the door's HTTP-level behaviour.
//!
//! A `wiremock` stands in for `Lakekeeper`, so the door's forwarding can
//! be observed without a catalog: what it forwards, under which
//! credential, and what it answers.
//!
//! # What these tests are about
//!
//! * the tool's own key NEVER reaches the catalog — the door forwards
//!   under the `outside-readers` principal's token;
//! * a table load through the door is COUNTED;
//! * a load whose count cannot be written is REFUSED, and the table is
//!   not returned (ADR 0016 point 5: an uncounted read would make the
//!   count a lie);
//! * a write is refused with the Iceberg error shape, and the catalog is
//!   never asked;
//! * a session — not a service identity — is refused;
//! * a deployment that has not published its storage address says so.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::collections::HashMap;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use common::{TestApp, spin_up_with_env};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// The warehouse prefix the stand-in catalog advertises. It is the
/// warehouse's id, not its name — `Lakekeeper` answers 400 for a name in
/// that path position.
const PREFIX: &str = "651eeefa-d506-11f1-b379-970f98828ea7";

/// The `outside-readers` principal's token: what the door forwards
/// under. Synthetic, and never a real credential.
const OUTSIDE_TOKEN: &str = "outside-readers-token-for-the-door-tests";

/// The storage address an operator published, which the door swaps into
/// the vended `s3.endpoint`.
const PUBLISHED_ENDPOINT: &str = "https://storage.example.com";

/// A unique temp file holding the `outside-readers` token, and its path.
fn outside_token_file() -> std::path::PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("the clock is after the epoch")
        .as_nanos();
    let path = std::env::temp_dir().join(format!("lakehouse-outside-token-{nanos}.jwt"));
    std::fs::write(&path, OUTSIDE_TOKEN).expect("write the token file");
    path
}

/// The stand-in catalog: the config, and one Gold table with a vended
/// storage address.
async fn catalog() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/config"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "overrides": {
                "uri": "http://lakekeeper:8181/catalog",
                "idempotency-key-lifetime": "PT30M",
            },
            "defaults": {
                "prefix": PREFIX,
                "rest-page-size": "100",
            },
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/v1/{PREFIX}/namespaces/gold/tables/orders")))
        // The catalog is only ever asked under the door's own principal:
        // a mock that matches THIS header does not match the tool's key,
        // so a forwarded tool key would leave the request unmatched.
        .and(header("authorization", format!("Bearer {OUTSIDE_TOKEN}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "metadata": {
                "format-version": 2,
                "table-uuid": "11111111-2222-3333-4444-555555555555",
            },
            "config": {
                "s3.endpoint": "http://rustfs:9000/",
                "s3.path-style-access": "true",
                "s3.access-key-id": "AKIAEXAMPLE",
            },
            "storage-credentials": [
                {
                    "prefix": "s3://lakehouse-warehouse/gold/orders/",
                    "config": {
                        "s3.endpoint": "http://rustfs:9000/",
                        "s3.access-key-id": "ASIAEXAMPLE",
                        "s3.session-token": "session-token",
                    },
                },
            ],
        })))
        .mount(&server)
        .await;
    server
}

/// The API wired to the stand-in catalog, with outside reading set up.
async fn door_over(server: &MockServer, token_file: &std::path::Path) -> TestApp {
    let mut overrides = HashMap::new();
    overrides.insert("LAKEKEEPER_CATALOG_URI".to_owned(), server.uri());
    overrides.insert(
        "OUTSIDE_S3_ENDPOINT".to_owned(),
        PUBLISHED_ENDPOINT.to_owned(),
    );
    overrides.insert(
        "LAKEKEEPER_OUTSIDE_READ_TOKEN_FILE".to_owned(),
        token_file.display().to_string(),
    );
    spin_up_with_env(&overrides).await
}

/// A service identity with the `iceberg:read` scope and a live
/// credential — an outside tool.
async fn outside_tool(pool: &PgPool) -> (Uuid, String) {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO service_identity (id, name, scopes, environment, expires_at) \
         VALUES ($1, $2, ARRAY['iceberg:read'], 'production', now() + interval '1 day')",
    )
    .bind(id)
    .bind(format!("outside-tool-{id}"))
    .execute(pool)
    .await
    .expect("create the outside tool's service identity");
    let token = lakehouse_auth::service_token::create_service_credential(pool, id)
        .await
        .expect("mint the outside tool's credential");
    (id, token.expose().to_owned())
}

async fn send(router: &axum::Router, request: Request<Body>) -> (StatusCode, Value) {
    let resp = router
        .clone()
        .oneshot(request)
        .await
        .expect("router never fails a request outright");
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("read the body");
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, body)
}

fn get(uri: &str, token: &str) -> Request<Body> {
    Request::builder()
        .method("GET")
        .uri(uri)
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .expect("build request")
}

fn request(method: &str, uri: &str, token: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .expect("build request")
}

/// A table load through the door is forwarded under the door's OWN
/// catalog principal — never the tool's key — and the read is counted.
#[tokio::test]
async fn the_iceberg_door_forwards_a_table_load_under_its_own_principal_and_counts_it() {
    let server = catalog().await;
    let token_file = outside_token_file();
    let TestApp { router, pool } = door_over(&server, &token_file).await;
    let (tool_id, tool_token) = outside_tool(&pool).await;

    let (status, body) = send(
        &router,
        get(
            &format!("/api/iceberg/v1/{PREFIX}/namespaces/gold/tables/orders"),
            &tool_token,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(body["metadata"]["format-version"], json!(2));

    // The vended storage address is the published one, in both places
    // (ADR 0016 point 4). The keys are NOT replaced.
    assert_eq!(body["config"]["s3.endpoint"], json!(PUBLISHED_ENDPOINT));
    assert_eq!(
        body["storage-credentials"][0]["config"]["s3.endpoint"],
        json!(PUBLISHED_ENDPOINT)
    );
    assert_eq!(body["config"]["s3.access-key-id"], json!("AKIAEXAMPLE"));

    // What the catalog was asked: never the tool's key.
    let requests = server.received_requests().await.expect("read the requests");
    let forwarded = requests
        .iter()
        .find(|request| request.url.path().contains("/tables/orders"))
        .expect("the table load reached the catalog");
    let authorization = forwarded
        .headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    assert_eq!(authorization, format!("Bearer {OUTSIDE_TOKEN}"));
    assert!(
        !authorization.contains(&tool_token),
        "the tool's own key must never reach the catalog"
    );

    // And the read was counted, against the tool's id and its name.
    let counted: i64 = sqlx::query_scalar(
        "SELECT reads FROM outside_table_read_day \
         WHERE identity_id = $1 AND namespace = 'gold' AND table_name = 'orders'",
    )
    .bind(tool_id)
    .fetch_one(&pool)
    .await
    .expect("read the counter");
    assert_eq!(counted, 1, "the load through the door was counted once");
}

/// A load whose count cannot be written is REFUSED, and the table is not
/// returned: an uncounted read would make the count a lie (ADR 0016
/// point 5, plan D6).
#[tokio::test]
async fn the_iceberg_door_refuses_a_load_whose_count_cannot_be_written() {
    let server = catalog().await;
    let token_file = outside_token_file();
    let TestApp { router, pool } = door_over(&server, &token_file).await;
    let (_tool_id, tool_token) = outside_tool(&pool).await;

    // The counter cannot be written: its table is gone.
    sqlx::query("DROP TABLE outside_table_read_day")
        .execute(&pool)
        .await
        .expect("drop the counter table");

    let (status, body) = send(
        &router,
        get(
            &format!("/api/iceberg/v1/{PREFIX}/namespaces/gold/tables/orders"),
            &tool_token,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "body: {body}");
    assert!(
        body.get("metadata").is_none(),
        "the table must not be returned uncounted: {body}"
    );
    assert_eq!(body["error"], json!("database error"));
}

/// A write is refused with the Iceberg error shape, and the catalog is
/// never asked (ADR 0016 point 2).
#[tokio::test]
async fn the_iceberg_door_refuses_a_write_and_never_reaches_the_catalog() {
    let server = catalog().await;
    let token_file = outside_token_file();
    let TestApp { router, pool } = door_over(&server, &token_file).await;
    let (_tool_id, tool_token) = outside_tool(&pool).await;

    let before = server
        .received_requests()
        .await
        .expect("read the requests")
        .len();

    let (status, body) = send(
        &router,
        request(
            "POST",
            &format!("/api/iceberg/v1/{PREFIX}/namespaces/gold/tables"),
            &tool_token,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "body: {body}");
    assert_eq!(
        body["error"]["message"],
        json!("Outside readers may only read published Gold tables.")
    );
    assert_eq!(body["error"]["type"], json!("ForbiddenException"));
    assert_eq!(body["error"]["code"], json!(403));

    let after = server
        .received_requests()
        .await
        .expect("read the requests")
        .len();
    assert_eq!(
        after, before,
        "a refused write must not reach the catalog at all"
    );
}

/// A namespace other than `gold` is refused the same way (ADR 0016
/// point 2).
#[tokio::test]
async fn the_iceberg_door_refuses_a_namespace_other_than_gold() {
    let server = catalog().await;
    let token_file = outside_token_file();
    let TestApp { router, pool } = door_over(&server, &token_file).await;
    let (_tool_id, tool_token) = outside_tool(&pool).await;

    let (status, body) = send(
        &router,
        get(
            &format!("/api/iceberg/v1/{PREFIX}/namespaces/bronze/tables/orders"),
            &tool_token,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "body: {body}");
    assert_eq!(
        body["error"]["message"],
        json!("Outside readers may only read published Gold tables.")
    );
}

/// A table name that walks out of the `gold` namespace is refused
/// before anything is forwarded (DATA-5 S1 review BLOCKER 1).
///
/// `axum` percent-decodes the `{table}` path value, so `%2F..%2F..`
/// arrives as `../..`: pasted into a forwarded URL it resolves to
/// another namespace. The catalog is mocked for THAT namespace, so a
/// door that forwards the walk answers 200 instead of refusing it —
/// this test fails then, and passes only when the walk is refused.
#[tokio::test]
async fn the_iceberg_door_refuses_a_table_name_that_walks_out_of_the_gold_namespace() {
    let server = catalog().await;
    // The namespace the walk would land in. A table load there answers
    // 200, so the door's own answer says whether it forwarded.
    Mock::given(method("GET"))
        .and(path(format!("/v1/{PREFIX}/namespaces/silver/tables/x")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "metadata": {"format-version": 2},
        })))
        .mount(&server)
        .await;
    let token_file = outside_token_file();
    let TestApp { router, pool } = door_over(&server, &token_file).await;
    let (tool_id, tool_token) = outside_tool(&pool).await;

    let (status, body) = send(
        &router,
        get(
            &format!(
                "/api/iceberg/v1/{PREFIX}/namespaces/gold/tables/..%2F..%2F..%2Fsilver%2Ftables%2Fx"
            ),
            &tool_token,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "body: {body}");
    assert_eq!(
        body["error"]["message"],
        json!("Outside readers may only read published Gold tables.")
    );

    // The catalog was never asked for the walk. The name is refused
    // before the door even asks the catalog for the warehouse prefix, so
    // the list of requests is empty today; the two assertions below hold
    // either way, and a forwarded walk is what would break them.
    let requests = server.received_requests().await.expect("read the requests");
    for request in &requests {
        let path = request.url.path();
        assert!(
            !path.contains("silver"),
            "the walk out of the namespace never reached the catalog: {path}"
        );
        assert!(
            !path.contains("/tables/"),
            "no table load was forwarded: {path}"
        );
    }

    // And nothing was counted under the made-up name.
    let counted: i64 =
        sqlx::query_scalar("SELECT count(*) FROM outside_table_read_day WHERE identity_id = $1")
            .bind(tool_id)
            .fetch_one(&pool)
            .await
            .expect("read the counter");
    assert_eq!(counted, 0, "a refused load is not counted");
}

/// A warehouse prefix that is not this deployment's is a 404, not a
/// forwarded call (D2, D8).
#[tokio::test]
async fn the_iceberg_door_is_not_found_for_a_foreign_warehouse_prefix() {
    let server = catalog().await;
    let token_file = outside_token_file();
    let TestApp { router, pool } = door_over(&server, &token_file).await;
    let (_tool_id, tool_token) = outside_tool(&pool).await;

    let (status, body) = send(
        &router,
        get(
            "/api/iceberg/v1/11111111-1111-1111-1111-111111111111/namespaces/gold/tables/orders",
            &tool_token,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "body: {body}");
    assert_eq!(body["error"]["type"], json!("NoSuchWarehouseException"));
}

/// A session is not a tool: the door counts reads per TOOL, so a browser
/// session is refused (ADR 0016 point 1).
#[tokio::test]
async fn the_iceberg_door_refuses_a_session_and_says_to_use_a_service_identity() {
    let server = catalog().await;
    let token_file = outside_token_file();
    let TestApp { router, pool } = door_over(&server, &token_file).await;
    let cookie = common::session_cookie_for_seeded_user(&pool, "fajar@meridian.example").await;

    let resp = router
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!(
                    "/api/iceberg/v1/{PREFIX}/namespaces/gold/tables/orders"
                ))
                .header("cookie", cookie)
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("router never fails a request outright");
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    let bytes = to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("read the body");
    let body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    assert_eq!(
        body["error"]["message"],
        json!("Use a service identity's key: the door counts reads per tool.")
    );
}

/// A deployment that has not published its storage address says so, and
/// names the setting (ADR 0016 point 4, plan D4).
#[tokio::test]
async fn the_iceberg_door_says_which_setting_is_missing_when_outside_reading_is_off() {
    let server = catalog().await;
    let token_file = outside_token_file();
    let mut overrides = HashMap::new();
    overrides.insert("LAKEKEEPER_CATALOG_URI".to_owned(), server.uri());
    overrides.insert(
        "LAKEKEEPER_OUTSIDE_READ_TOKEN_FILE".to_owned(),
        token_file.display().to_string(),
    );
    let TestApp { router, pool } = spin_up_with_env(&overrides).await;
    let (_tool_id, tool_token) = outside_tool(&pool).await;

    let (status, body) = send(
        &router,
        get(
            &format!("/api/iceberg/v1/{PREFIX}/namespaces/gold/tables/orders"),
            &tool_token,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "body: {body}");
    let message = body["error"].as_str().unwrap_or_default();
    assert!(
        message.contains("OUTSIDE_S3_ENDPOINT"),
        "the refusal names the setting that is missing: {message}"
    );
    assert!(
        message.contains("not set up"),
        "the refusal says outside reading is not set up: {message}"
    );
}

/// The config the door forwards keeps the warehouse prefix the client
/// needs, and loses the catalog's own `uri` override — a client that
/// followed it would leave the door, and a read that leaves the door is a
/// read nobody counts.
#[tokio::test]
async fn the_iceberg_door_keeps_the_prefix_and_drops_the_catalogs_own_uri_override() {
    let server = catalog().await;
    let token_file = outside_token_file();
    let TestApp { router, pool } = door_over(&server, &token_file).await;
    let (_tool_id, tool_token) = outside_tool(&pool).await;

    let (status, body) = send(&router, get("/api/iceberg/v1/config", &tool_token)).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(body["defaults"]["prefix"], json!(PREFIX));
    assert!(
        body["overrides"].get("uri").is_none(),
        "the catalog's own uri override must not send the client away: {body}"
    );
}

/// The namespace list the door serves holds `gold` and nothing else.
#[tokio::test]
async fn the_iceberg_door_serves_only_the_gold_namespace() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/config"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "defaults": {"prefix": PREFIX},
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/v1/{PREFIX}/namespaces")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "namespaces": [["gold"], ["bronze"]],
        })))
        .mount(&server)
        .await;
    let token_file = outside_token_file();
    let TestApp { router, pool } = door_over(&server, &token_file).await;
    let (_tool_id, tool_token) = outside_tool(&pool).await;

    let (status, body) = send(
        &router,
        get(&format!("/api/iceberg/v1/{PREFIX}/namespaces"), &tool_token),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(body["namespaces"], json!([["gold"]]));
}

/// The consumers route reads the count back: the tool that loaded the
/// published table through the door is listed, with its reads and its
/// last read (DATA-5, plan D7).
#[tokio::test]
async fn the_gold_consumers_route_lists_the_tool_that_read_the_published_table() {
    let server = catalog().await;
    let token_file = outside_token_file();
    let TestApp { router, pool } = door_over(&server, &token_file).await;
    let (tool_id, tool_token) = outside_tool(&pool).await;

    // One read through the door.
    let (status, body) = send(
        &router,
        get(
            &format!("/api/iceberg/v1/{PREFIX}/namespaces/gold/tables/orders"),
            &tool_token,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");

    // The consumers route is a CONSOLE route: it needs a session, not
    // the tool's key.
    let cookie = common::session_cookie_for_seeded_user(&pool, "fajar@meridian.example").await;
    let resp = router
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/gold/export/orders/consumers")
                .header("cookie", cookie)
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("router never fails a request outright");
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("read the body");
    let body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);

    assert_eq!(body["mart"], json!("orders"));
    assert_eq!(body["supported"], json!(true));
    assert_eq!(body["windowDays"], json!(7));
    let consumers = body["consumers"].as_array().expect("an array of readers");
    assert_eq!(consumers.len(), 1, "one tool read the table: {body}");
    assert_eq!(consumers[0]["identityId"], json!(tool_id.to_string()));
    assert_eq!(consumers[0]["reads"], json!(1));
    assert!(
        !consumers[0]["lastReadAt"]
            .as_str()
            .unwrap_or_default()
            .is_empty(),
        "the last read is reported, not a guess"
    );
    assert!(
        consumers[0]["name"]
            .as_str()
            .unwrap_or_default()
            .starts_with("outside-tool-"),
        "the tool is listed under the name it had: {body}"
    );
}

/// Where outside reading is off, the consumers route says "not measured"
/// with the door's own reason — never a count of zero (principle 2).
#[tokio::test]
async fn the_gold_consumers_route_reports_not_measured_when_outside_reading_is_off() {
    let server = catalog().await;
    let token_file = outside_token_file();
    let mut overrides = HashMap::new();
    overrides.insert("LAKEKEEPER_CATALOG_URI".to_owned(), server.uri());
    overrides.insert(
        "LAKEKEEPER_OUTSIDE_READ_TOKEN_FILE".to_owned(),
        token_file.display().to_string(),
    );
    let TestApp { router, pool } = spin_up_with_env(&overrides).await;
    let cookie = common::session_cookie_for_seeded_user(&pool, "fajar@meridian.example").await;

    let resp = router
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/gold/export/orders/consumers")
                .header("cookie", cookie)
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("router never fails a request outright");
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("read the body");
    let body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);

    assert_eq!(body["supported"], json!(false));
    assert!(
        body["consumers"].is_null(),
        "an unmeasured count is null, never an empty list that reads as zero: {body}"
    );
    let reason = body["reason"].as_str().unwrap_or_default();
    assert_eq!(
        reason, "Outside reading is not set up: OUTSIDE_S3_ENDPOINT is not set.",
        "the console says exactly what the door's own 503 says (plan D7)"
    );
}

/// A token file that cannot be read means the door cannot work either,
/// so the count is not measured — and the reason says so (ADR 0016 point
/// 4: the check runs before anything is forwarded).
#[tokio::test]
async fn the_gold_consumers_route_reports_not_measured_when_the_token_file_cannot_be_read() {
    let server = catalog().await;
    let mut overrides = HashMap::new();
    overrides.insert("LAKEKEEPER_CATALOG_URI".to_owned(), server.uri());
    overrides.insert(
        "OUTSIDE_S3_ENDPOINT".to_owned(),
        PUBLISHED_ENDPOINT.to_owned(),
    );
    // A path that is not there: the deployment has no `outside-readers`
    // credential to forward under.
    overrides.insert(
        "LAKEKEEPER_OUTSIDE_READ_TOKEN_FILE".to_owned(),
        "/nonexistent/outside-readers.jwt".to_owned(),
    );
    let TestApp { router, pool } = spin_up_with_env(&overrides).await;
    let cookie = common::session_cookie_for_seeded_user(&pool, "fajar@meridian.example").await;

    let resp = router
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/gold/export/orders/consumers")
                .header("cookie", cookie)
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("router never fails a request outright");
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("read the body");
    let body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);

    assert_eq!(body["supported"], json!(false));
    assert!(body["consumers"].is_null(), "not measured: {body}");
    let reason = body["reason"].as_str().unwrap_or_default();
    assert!(
        reason.contains("LAKEKEEPER_OUTSIDE_READ_TOKEN_FILE"),
        "the reason names the setting that is missing: {reason}"
    );
    assert!(
        !reason.contains("/nonexistent"),
        "the reason never carries the token file's path: {reason}"
    );
}
