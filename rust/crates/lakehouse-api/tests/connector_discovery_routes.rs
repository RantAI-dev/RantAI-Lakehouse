#![allow(
    clippy::assert_is_empty,
    reason = "test assertions checking collection emptiness are idiomatic and avoid empty array type casts"
)]
//! `SRC-13`: the discovery routes of a connector (`POST` and `GET`
//! `/api/connectors/{id}/discovery/{kind}`), over the real router and a
//! migrated database. Nothing here reaches a network: the REST list is the
//! dial's own endpoints, the internal-host case is refused before any dial, and
//! the stored answers are written through the store.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use lakehouse_store::connector_discovery::{
    ErrorClass, Kind, ListedObject, Listing, ObjectKind, Outcome, begin, finish,
};
use serde_json::{Value, json};
use sqlx::PgPool;
use time::{Duration, OffsetDateTime};
use tower::ServiceExt;

use common::{TestApp, session_cookie_for_seeded_user, spin_up};

const MERIDIAN_GROUP: &str = "11111111-1111-4111-8111-000000000001";

struct Fixture {
    app: TestApp,
    cookie: String,
}

async fn fixture() -> Fixture {
    let app = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&app.pool, "bayu@meridian.example").await;
    Fixture { app, cookie }
}

async fn send(fx: &Fixture, method: &str, path: &str, body: Option<Value>) -> (StatusCode, Value) {
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header("cookie", &fx.cookie);
    let body = match body {
        Some(body) => {
            builder = builder.header("content-type", "application/json");
            Body::from(body.to_string())
        }
        None => Body::empty(),
    };
    let response = fx
        .app
        .router
        .clone()
        .oneshot(builder.body(body).expect("build request"))
        .await
        .expect("router never fails a request outright");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

async fn seed(pool: &PgPool, id: &str, kind: &str, adapter: Option<&str>, dial: Value) {
    sqlx::query(
        "INSERT INTO connector (id, tenant_id, name, type, direction, host, secret_ref, \
         environment, tenant, adapter, ingest_mode, dial) VALUES ($1, $2, 'discovery', $3, \
         'source', 'unused', 'env:CONNECTOR_DISCOVERY_TEST', 'production', 'meridian', $4, \
         CASE WHEN $4::text IS NULL THEN NULL ELSE 'batch' END, $5)",
    )
    .bind(id)
    .bind(MERIDIAN_GROUP.parse::<uuid::Uuid>().unwrap())
    .bind(kind)
    .bind(adapter)
    .bind(dial)
    .execute(pool)
    .await
    .expect("seed a connector");
}

async fn seed_rest(pool: &PgPool, id: &str) {
    seed(
        pool,
        id,
        "REST API",
        Some("rest"),
        json!({
            "baseUrl": "https://api.example",
            "auth": { "type": "bearer" },
            "pagination": { "type": "none" },
            "endpoints": [
                { "path": "/orders", "recordsPath": "data" },
                { "path": "/customers" },
            ],
        }),
    )
    .await;
}

/// `SRC-13` D1, D3: a REST connector's list is its declared endpoints, answered
/// in the same request (no network), and the server-side search narrows it.
#[tokio::test]
async fn a_rest_connector_lists_its_declared_endpoints_and_a_search_narrows_them() {
    let fx = fixture().await;
    seed_rest(&fx.app.pool, "conn-rest-disc").await;
    let url = "/api/connectors/conn-rest-disc/discovery/list";

    let (status, body) = send(&fx, "POST", url, Some(json!({}))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["kind"], "list");
    assert_eq!(body["state"], "done");
    assert_eq!(body["truncated"], false);
    let names: Vec<&str> = body["objects"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["/orders", "/customers"]);
    assert_eq!(body["objects"][0]["kind"], "endpoint");

    let (_, body) = send(&fx, "POST", url, Some(json!({ "search": "CUST" }))).await;
    assert_eq!(body["objects"][0]["name"], "/customers");
    assert_eq!(body["objects"].as_array().unwrap().len(), 1);

    let (_, body) = send(&fx, "POST", url, Some(json!({ "limit": 1 }))).await;
    assert_eq!(body["objects"].as_array().unwrap().len(), 1);
    assert_eq!(body["truncated"], true);
    assert!(body["reason"].as_str().unwrap().contains("10,000"));
}

/// An empty body is the request with every default; a body that is not the
/// documented JSON, a preview with no object, a list with one and a kind that is
/// neither are 400 with this route's own words.
#[tokio::test]
async fn a_malformed_request_is_a_400_before_the_source_is_touched() {
    let fx = fixture().await;
    seed_rest(&fx.app.pool, "conn-rest-disc").await;
    let base = "/api/connectors/conn-rest-disc/discovery";

    let (status, _) = send(&fx, "POST", &format!("{base}/list"), None).await;
    assert_eq!(status, StatusCode::OK, "an empty body is every default");

    let refused = [
        (format!("{base}/preview"), json!({})),
        (format!("{base}/preview"), json!({ "object": "" })),
        (format!("{base}/list"), json!({ "object": "/orders" })),
        (format!("{base}/list"), json!({ "limit": 0 })),
        (format!("{base}/list"), json!({ "limit": 10001 })),
        (format!("{base}/list"), json!({ "surprise": true })),
        (format!("{base}/list"), json!({ "glob": "x".repeat(257) })),
        (format!("{base}/columns"), json!({})),
    ];
    for (url, body) in refused {
        let (status, answer) = send(&fx, "POST", &url, Some(body.clone())).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{url} {body}: {answer}");
    }
}

/// `SRC-13` D10: a connector with no ingest spec has nothing to list, said with
/// a class and its fixed sentence, not the old `unknown adapter` text.
#[tokio::test]
async fn a_connector_without_a_spec_answers_not_supported_with_the_fixed_sentence() {
    let fx = fixture().await;
    seed(
        &fx.app.pool,
        "conn-legacy-disc",
        "PostgreSQL",
        None,
        json!({}),
    )
    .await;
    let (status, body) = send(
        &fx,
        "POST",
        "/api/connectors/conn-legacy-disc/discovery/list",
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["state"], "failed");
    assert_eq!(body["errorClass"], "not_supported");
    assert_eq!(body["reason"], ErrorClass::NotSupported.reason());
    assert!(!body.to_string().contains("unknown adapter"));
}

/// `SRC-13` D10, principle 4, `SEC-15`: a SQL source that resolves to an internal
/// address is refused before any dial or credential, and the answer names the
/// class and the fixed sentence only: neither the host nor the address.
#[tokio::test]
async fn an_internal_host_is_refused_by_class_without_naming_the_host() {
    let fx = fixture().await;
    seed(
        &fx.app.pool,
        "conn-sql-disc",
        "PostgreSQL",
        Some("sql"),
        json!({
            "driver": "postgres", "host": "127.0.0.1", "port": 5432,
            "database": "d", "user": "u",
        }),
    )
    .await;
    for kind_and_body in [
        ("list", json!({})),
        ("list", json!({ "schema": "public" })),
        ("preview", json!({ "object": "public.orders" })),
    ] {
        let (kind, body) = kind_and_body;
        let (status, answer) = send(
            &fx,
            "POST",
            &format!("/api/connectors/conn-sql-disc/discovery/{kind}"),
            Some(body),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{answer}");
        assert_eq!(answer["state"], "failed");
        assert_eq!(answer["errorClass"], "address_refused");
        assert_eq!(answer["reason"], ErrorClass::AddressRefused.reason());
        assert!(!answer.to_string().contains("127.0.0.1"), "{answer}");
    }
}

/// `GET` before any request is a 404; after the orchestrator's answer is stored
/// it reads back in the documented shape; a request whose job never answered
/// reads as a timeout once the window has passed.
#[tokio::test]
async fn the_poll_reads_the_latest_answer_and_a_dead_request_reads_as_a_timeout() {
    let fx = fixture().await;
    seed_rest(&fx.app.pool, "conn-rest-disc").await;
    let url = "/api/connectors/conn-rest-disc/discovery/list";
    let (status, _) = send(&fx, "GET", url, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let now = OffsetDateTime::now_utc();
    begin(
        &fx.app.pool,
        "conn-rest-disc",
        Kind::List,
        "req-1",
        "",
        "topics",
        now,
    )
    .await
    .unwrap();
    let (_, running) = send(&fx, "GET", url, None).await;
    assert_eq!(running["state"], "running");
    assert_eq!(running["scope"], "topics");

    let listing = Listing {
        objects: vec![ListedObject::named("orders-v1", ObjectKind::Topic)],
        truncated: false,
    };
    assert!(
        finish(
            &fx.app.pool,
            "conn-rest-disc",
            Kind::List,
            "req-1",
            &Outcome::Listing(listing),
            now,
        )
        .await
        .unwrap()
    );
    let (status, done) = send(&fx, "GET", url, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(done["state"], "done");
    assert_eq!(done["objects"][0]["name"], "orders-v1");
    assert_eq!(done["objects"][0]["kind"], "topic");

    begin(
        &fx.app.pool,
        "conn-rest-disc",
        Kind::Preview,
        "req-2",
        "orders-v1",
        "",
        now - Duration::minutes(10),
    )
    .await
    .unwrap();
    let (_, dead) = send(
        &fx,
        "GET",
        "/api/connectors/conn-rest-disc/discovery/preview",
        None,
    )
    .await;
    assert_eq!(dead["state"], "failed");
    assert_eq!(dead["errorClass"], "timeout");
    assert_eq!(dead["object"], "orders-v1");
}

/// An unknown connector, and a kind that is not one, are refused the way every
/// other per-connector route refuses them.
#[tokio::test]
async fn an_unknown_connector_is_a_404_and_an_unknown_kind_a_400() {
    let fx = fixture().await;
    let (status, _) = send(
        &fx,
        "POST",
        "/api/connectors/no-such/discovery/list",
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    seed_rest(&fx.app.pool, "conn-rest-disc").await;
    let (status, _) = send(
        &fx,
        "GET",
        "/api/connectors/conn-rest-disc/discovery/columns",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}
