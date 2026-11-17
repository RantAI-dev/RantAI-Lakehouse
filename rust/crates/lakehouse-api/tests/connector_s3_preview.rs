//! `SEC-55`: an S3 files connector's Parquet preview is read by the conversion
//! child, from the file's TAIL.
//!
//! A Parquet footer declares how much memory its reader will want, and
//! `parquet` reserves that room from the declared counts before any check runs
//! (`SEC-22` P1). Reading a footer in the API process — which is what the
//! preview used to do, for a whole object of up to 64 MiB — let one crafted
//! file in a bucket a connector reads end the API for every tenant. So the
//! preview now fetches only the object's last
//! `ConversionRunner::parquet_tail_bytes()` bytes and asks the child, exactly
//! as an upload does.
//!
//! # The stub
//!
//! The object store is a `wiremock` server speaking the two requests
//! `object_store` makes here: a `HEAD` (the size) and a ranged `GET` (the
//! tail). Its facts are the same ones `upload_routes`'s fake bucket serves, so
//! the API's S3 client is real and only the bucket is a stand-in. The
//! connector's endpoint is the stub's loopback address, which the SSRF check
//! allows through `CONNECTOR_PROBE_ALLOW_INTERNAL_HOSTS=true` — the same
//! switch the other loopback dial tests set, never a change to the check.
//!
//! `CONNECTOR_SECRETS_KEY`-sealed credentials are not needed: the connector's
//! two secrets resolve from an injected map
//! (`spin_up_with_state`'s `connector_secret_resolver`), so no process-wide
//! environment is touched.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use crate::common;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use lakehouse_core::secret::EnvSecretResolver;
use lakehouse_store::connector_discovery::ErrorClass;
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use wiremock::matchers::{method, path_regex};
use wiremock::{Mock, MockServer, Request as MockRequest, Respond, ResponseTemplate};

use common::{TestApp, session_cookie_for_seeded_user, spin_up_with_state};

const MERIDIAN_GROUP: &str = "11111111-1111-4111-8111-000000000001";
const CONNECTOR: &str = "conn-s3-preview";
const BUCKET: &str = "lakehouse-warehouse";
/// The bucket's own path shape, path-style (`SEC-6`): `/<bucket>/<key>`.
const OBJECT_PATH: &str = r"^/lakehouse-warehouse/.+";
/// The tail the tests whose object is not stored whole ask the runner for.
/// The real bound is 16 MiB; a test proves the tail is read without the
/// fixture holding one (`SEC-22` T1's own test seam).
const TEST_TAIL_BYTES: u64 = 4096;

// ── the fake bucket ──────────────────────────────────────────────────────

/// One object of the fake bucket: the size `HEAD` reports and the bytes the
/// stub can serve.
///
/// The bytes are the object's END, and they stand at `size - bytes.len()`:
/// a 48-byte object holds all of itself, and an 80 MiB object holds only the
/// last [`TEST_TAIL_BYTES`] of it. That is what the preview is allowed to
/// read, and nothing else is served.
#[derive(Clone)]
struct Object {
    size: u64,
    bytes: Vec<u8>,
}

impl Object {
    /// An object whose bytes are all of it.
    fn whole(bytes: Vec<u8>) -> Self {
        Self {
            size: bytes.len() as u64,
            bytes,
        }
    }

    /// An object of `size` bytes that serves only the end of `tail` at its
    /// own end: the fixture's declared size and the bytes that hold its
    /// footer.
    fn ending_at(size: u64, tail: Vec<u8>) -> Self {
        assert!(size >= tail.len() as u64, "the tail is part of the object");
        Self { size, bytes: tail }
    }

    /// The object's first offset the stub can serve.
    fn first(&self) -> u64 {
        self.size - self.bytes.len() as u64
    }
}

#[derive(Clone, Default)]
struct Bucket(Arc<Mutex<HashMap<String, Object>>>);

impl Bucket {
    fn put(&self, key: &str, object: Object) {
        self.0.lock().unwrap().insert(key.to_owned(), object);
    }

    fn get(&self, key: &str) -> Option<Object> {
        self.0.lock().unwrap().get(key).cloned()
    }
}

async fn mount_bucket(server: &MockServer, bucket: &Bucket) {
    Mock::given(method("HEAD"))
        .and(path_regex(OBJECT_PATH))
        .respond_with(HeadObject(bucket.clone()))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path_regex(OBJECT_PATH))
        .respond_with(GetRange(bucket.clone()))
        .mount(server)
        .await;
}

fn key_of(request: &MockRequest) -> String {
    request
        .url
        .path()
        .trim_start_matches(&format!("/{BUCKET}/"))
        .to_owned()
}

fn object_headers(template: ResponseTemplate) -> ResponseTemplate {
    template
        .insert_header("ETag", "\"sec55\"")
        .insert_header("Last-Modified", "Wed, 21 Oct 2015 07:28:00 GMT")
}

struct HeadObject(Bucket);
impl Respond for HeadObject {
    fn respond(&self, request: &MockRequest) -> ResponseTemplate {
        match self.0.get(&key_of(request)) {
            Some(object) => object_headers(
                ResponseTemplate::new(200).insert_header("Content-Length", object.size.to_string()),
            ),
            None => ResponseTemplate::new(404),
        }
    }
}

/// The ranged `GET`: only the part of the object the stub holds is served,
/// which for an object over the tail bound is the tail and nothing before it.
struct GetRange(Bucket);
impl Respond for GetRange {
    fn respond(&self, request: &MockRequest) -> ResponseTemplate {
        let Some(object) = self.0.get(&key_of(request)) else {
            return ResponseTemplate::new(404);
        };
        let range = request
            .headers
            .get("range")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("bytes="))
            .and_then(|value| value.split_once('-'))
            .and_then(|(from, to)| Some((from.parse::<u64>().ok()?, to.parse::<u64>().ok()?)));
        let Some((from, to)) = range else {
            return ResponseTemplate::new(400)
                .set_body_string("<Error><Code>InvalidRequest</Code></Error>");
        };
        let first = object.first();
        let Some(slice) = usize::try_from(from - first)
            .ok()
            .zip(usize::try_from(to - first).ok())
            .and_then(|(start, end)| object.bytes.get(start..=end))
        else {
            return ResponseTemplate::new(416)
                .set_body_string("<Error><Code>InvalidRange</Code></Error>");
        };
        object_headers(
            ResponseTemplate::new(206)
                .insert_header(
                    "Content-Range",
                    format!("bytes {from}-{to}/{}", object.size),
                )
                .set_body_bytes(slice.to_vec()),
        )
    }
}

// ── the fixture ──────────────────────────────────────────────────────────

struct Fixture {
    app: TestApp,
    cookie: String,
    s3: MockServer,
}

/// A Parquet fixture of `ops/fixtures/parquet/`.
fn parquet_file(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/../../../ops/fixtures/parquet/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

/// A hostile upload fixture of `ops/fixtures/uploads/hostile/`.
fn hostile_file(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/../../../ops/fixtures/uploads/hostile/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

/// A value for the two credentials the stub never checks. Not a secret: the
/// process's own package name, so the repository holds no key-shaped literal.
fn fixture_credential() -> String {
    std::env::var("CARGO_PKG_NAME").unwrap_or_else(|_| "fixture".to_owned())
}

/// The app, a stub bucket with `objects` in it, and a `files`/`s3`/`parquet`
/// connector that reads it.
///
/// `tail_bytes` is `Some` only for the tests whose object is not stored whole:
/// the runner's tail bound is a test seam, so a test proves the tail is read
/// without a 16 MiB fixture.
async fn fixture(tail_bytes: Option<u64>, objects: &[(&str, Object)]) -> Fixture {
    let s3 = MockServer::start().await;
    let bucket = Bucket::default();
    for (key, object) in objects {
        bucket.put(key, object.clone());
    }
    mount_bucket(&s3, &bucket).await;

    let overrides = HashMap::from([(
        "CONNECTOR_PROBE_ALLOW_INTERNAL_HOSTS".to_owned(),
        "true".to_owned(),
    )]);
    let resolver = Arc::new(EnvSecretResolver::with_map(HashMap::from([
        ("CONNECTOR_S3_ACCESS_KEY".to_owned(), fixture_credential()),
        ("CONNECTOR_S3_SECRET_KEY".to_owned(), fixture_credential()),
    ])));
    let (app, _state) = spin_up_with_state(&overrides, move |state| {
        state.connector_secret_resolver = resolver;
        if let Some(tail_bytes) = tail_bytes {
            state.conversion_runner = state
                .conversion_runner
                .with_parquet_limits(tail_bytes, 64 * 1024 * 1024);
        }
    })
    .await;
    let cookie = session_cookie_for_seeded_user(&app.pool, "bayu@meridian.example").await;
    seed_connector(&app.pool, &s3.uri()).await;
    Fixture { app, cookie, s3 }
}

/// A `files` connector of Meridian Group whose bucket is the stub.
async fn seed_connector(pool: &PgPool, endpoint: &str) {
    sqlx::query(
        "INSERT INTO connector (id, tenant_id, name, type, direction, host, secret_ref, \
         secret_ref_secondary, environment, tenant, adapter, ingest_mode, dial) VALUES \
         ($1, $2, 's3 preview', 'S3', 'source', 'unused', 'env:CONNECTOR_S3_ACCESS_KEY', \
         'env:CONNECTOR_S3_SECRET_KEY', 'production', 'meridian', 'files', 'batch', $3)",
    )
    .bind(CONNECTOR)
    .bind(MERIDIAN_GROUP.parse::<uuid::Uuid>().unwrap())
    .bind(json!({
        "protocol": "s3",
        "endpoint": endpoint,
        "bucket": BUCKET,
        "prefix": "landing",
        "format": "parquet",
    }))
    .execute(pool)
    .await
    .expect("seed a files connector");
}

impl Fixture {
    /// Post one discovery request, and answer with its status and body.
    async fn send(&self, kind: &str, body: Option<Value>) -> (StatusCode, Value) {
        let mut builder = Request::builder()
            .method("POST")
            .uri(format!("/api/connectors/{CONNECTOR}/discovery/{kind}"))
            .header("cookie", &self.cookie);
        let payload = match body {
            Some(body) => {
                builder = builder.header("content-type", "application/json");
                Body::from(body.to_string())
            }
            None => Body::empty(),
        };
        let response = self
            .app
            .router
            .clone()
            .oneshot(builder.body(payload).expect("build request"))
            .await
            .expect("the router never fails a request outright");
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    /// Preview the object at `key`.
    async fn preview(&self, key: &str) -> (StatusCode, Value) {
        self.send("preview", Some(json!({ "object": key }))).await
    }

    /// `GET /health`, the process-level liveness route: a request that is not
    /// this connector's, made after a preview, to show the API is still there.
    async fn health(&self) -> (StatusCode, Value) {
        let response = self
            .app
            .router
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("the router never fails a request outright");
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    /// The `Range` header of every ranged read the stub was sent, in the order
    /// it saw them.
    async fn ranges_sent(&self) -> Vec<String> {
        self.s3
            .received_requests()
            .await
            .unwrap()
            .into_iter()
            .filter(|request| request.method.as_str() == "GET")
            .map(|request| {
                request
                    .headers
                    .get("range")
                    .and_then(|value| value.to_str().ok())
                    .unwrap_or("<no range>")
                    .to_owned()
            })
            .collect()
    }
}

// ── the tests ────────────────────────────────────────────────────────────

/// `SEC-55` D1, D4: a crafted footer served by a bucket the connector reads
/// ends the CHILD, the preview answers the listing's fixed `failed` sentence,
/// and the API answers the next request. Before `SEC-55` the API itself read
/// that footer, and this process would be gone.
#[tokio::test]
async fn a_crafted_parquet_footer_in_a_bucket_ends_only_the_child() {
    let hostile = hostile_file("huge_list_footer.parquet");
    let fx = fixture(None, &[("landing/hostile.parquet", Object::whole(hostile))]).await;

    let (status, body) = fx.preview("landing/hostile.parquet").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["state"], "failed", "{body}");
    assert_eq!(body["errorClass"], "failed", "{body}");
    assert_eq!(body["reason"], ErrorClass::Failed.reason(), "{body}");
    assert!(
        !body.to_string().contains("memory"),
        "the child's own reason does not reach the answer: {body}"
    );

    // The API is still there: a request that is not this connector's answers.
    let (status, health) = fx.health().await;
    assert_eq!(status, StatusCode::OK, "{health}");
    assert_eq!(health["status"], "ok");
}

/// `SEC-55` D1: a valid Parquet object previews its columns and the types the
/// file declares, marked as declared and not inferred (`SRC-13` D6).
#[tokio::test]
async fn a_valid_parquet_object_previews_its_columns_and_types() {
    let orders = parquet_file("orders.parquet");
    let fx = fixture(None, &[("landing/orders.parquet", Object::whole(orders))]).await;

    let (status, body) = fx.preview("landing/orders.parquet").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["state"], "done", "{body}");
    assert_eq!(
        body["columns"],
        json!([
            { "name": "sku", "typeName": "string", "inferred": false },
            { "name": "name", "typeName": "string", "inferred": false },
            { "name": "qty", "typeName": "int64", "inferred": false },
            { "name": "price", "typeName": "decimal(10,2)", "inferred": false },
            { "name": "received", "typeName": "date", "inferred": false },
        ]),
        "{body}"
    );
}

/// `SEC-55` D1: the preview reads the file's TAIL and never the whole object.
/// The stub holds only the last [`TEST_TAIL_BYTES`] of an object it reports as
/// 1 MiB, so a read of `0..size` would be unanswered; the answer is the
/// footer's, and the only ranged read is the tail's.
#[tokio::test]
async fn the_preview_reads_the_tail_and_never_the_whole_object() {
    let orders = parquet_file("orders.parquet");
    let size = 1024 * 1024;
    // The tail bytes end with the real file: the footer is the file's own,
    // and the bytes before it are padding the child never asks for.
    let mut tail = vec![0_u8; usize::try_from(TEST_TAIL_BYTES).unwrap() - orders.len()];
    tail.extend_from_slice(&orders);
    let fx = fixture(
        Some(TEST_TAIL_BYTES),
        &[("landing/big.parquet", Object::ending_at(size, tail))],
    )
    .await;

    let (status, body) = fx.preview("landing/big.parquet").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["state"], "done", "{body}");
    assert_eq!(body["columns"][0]["name"], "sku", "{body}");

    let sent = fx.ranges_sent().await;
    assert_eq!(sent.len(), 1, "one ranged read: {sent:?}");
    assert_eq!(
        sent[0],
        format!("bytes={}-{}", size - TEST_TAIL_BYTES, size - 1),
        "the tail and nothing before it"
    );
    assert!(
        !sent
            .iter()
            .any(|range| range == &format!("bytes=0-{}", size - 1)),
        "the whole object was never asked for: {sent:?}"
    );
}

/// `SEC-55` D2: the 64 MiB refusal is gone. A Parquet object of 80 MiB
/// previews, because only its tail moves; the stub reports the size and
/// serves the tail.
#[tokio::test]
async fn a_parquet_object_over_64_mib_previews_because_only_its_tail_moves() {
    let orders = parquet_file("orders.parquet");
    let size = 80 * 1024 * 1024;
    // The tail bytes end with the real file: the footer is the file's own,
    // and the bytes before it are padding the child never asks for.
    let mut tail = vec![0_u8; usize::try_from(TEST_TAIL_BYTES).unwrap() - orders.len()];
    tail.extend_from_slice(&orders);
    let fx = fixture(
        Some(TEST_TAIL_BYTES),
        &[("landing/large.parquet", Object::ending_at(size, tail))],
    )
    .await;

    let (status, body) = fx.preview("landing/large.parquet").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["state"], "done", "{body}");
    let names: Vec<&str> = body["columns"]
        .as_array()
        .unwrap()
        .iter()
        .map(|column| column["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["sku", "name", "qty", "price", "received"], "{body}");

    let sent = fx.ranges_sent().await;
    assert_eq!(sent.len(), 1, "one ranged read: {sent:?}");
    assert_eq!(
        sent[0],
        format!("bytes={}-{}", size - TEST_TAIL_BYTES, size - 1),
        "an 80 MiB object moves its tail only"
    );
}
