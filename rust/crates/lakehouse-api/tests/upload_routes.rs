//! `/api/uploads/*` at the route level (T6 of
//! `docs/superpowers/plans/2026-10-02-upload-file.md`, ADR 0014): the real
//! router over a real migrated Postgres, real authenticated sessions, and
//! stand-ins for the three things the routes talk to.
//!
//! # The stand-ins
//!
//! - **The warehouse bucket** is a stateful `wiremock` fake ([`Bucket`]): it
//!   keeps what a `PUT` stores and answers `HEAD`, ranged `GET` and the
//!   multi-object delete `object_store` sends, so a test can upload a file and
//!   then read its preview back, or check that nothing was stored. The two
//!   credential refs name variables Cargo sets for every test binary
//!   (`CARGO_PKG_NAME`, `CARGO_PKG_VERSION`): `UploadStore::connect` reads the
//!   process environment, and a test cannot set one without racing the others.
//!   The fake never checks a signature.
//! - **`Dagster`** answers `launchRun` and `pipelineRunOrError` where a test
//!   mounts them; anything else is wiremock's 404, which the client reads as a
//!   failure. Nothing is mounted by default, so a launch a test did not expect
//!   is visible in `received_requests`.
//! - **`ClickHouse`** answers the registry lookup, `DESCRIBE` and the
//!   `ingest_run` read where a test mounts them. A request nobody mounted is
//!   a 404 with no body: an error that is not "unknown table", which is what
//!   makes the fail-closed paths easy to reach.
//!
//! The seeded users are those of `0002_seed_identity.sql`: Bayu (Data
//! Engineer, Meridian Group and Meridian Logistics), Andi (Data Engineer,
//! Meridian Retail) and Sari (Analyst, Meridian Retail).
//!
//! Every message asserted here is the fixed text the plan gives, or text of the
//! same plain kind; where a test injects an upstream error it asserts that the
//! injected text is absent from the response (principle 4).
//!
//! Who owns a raw table name is the claim table's to say (`upload_table_claim`,
//! T6a, review finding B4), so the helpers that put an upload in a loading or
//! loaded state make the claim first, as the ingest route does, and the tests
//! that refuse a load check that the refusal claimed nothing.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use lakehouse_store::uploads::{self, LoadMode, NewUpload, Upload};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use tower::ServiceExt;
use uuid::Uuid;
use wiremock::matchers::{any, body_string_contains, method, path, path_regex, query_param};
use wiremock::{Mock, MockServer, Request as MockRequest, Respond, ResponseTemplate};

use common::{
    TestApp, create_principal_with_permissions, session_cookie_for_seeded_user,
    session_cookie_for_user, spin_up_with_env,
};

const GROUP: &str = "11111111-1111-4111-8111-000000000001";
const RETAIL: &str = "11111111-1111-4111-8111-000000000002";
const LOGISTICS: &str = "11111111-1111-4111-8111-000000000003";

/// A string no handler of this crate could produce, injected into an upstream
/// error, so that finding it in a response proves upstream text got out.
const MARKER: &str = "UPSTREAM-DETAIL-9f3a1c";

/// `MAX_UPLOAD_BYTES` of `routes::uploads`: 50 MiB. Not importable (the
/// module is private), so repeated here and pinned by that module's own test.
const CAP: usize = 50 * 1024 * 1024;

/// `MAX_SESSION_BYTES` of `routes::uploads::sessions`: 2 GiB, exactly
/// 2,147,483,648 (`DATA-17-RESULT.md`, plan T6.2). Also private, so repeated
/// here and pinned by that module's own test.
const SESSION_CAP: usize = 2 * 1024 * 1024 * 1024;

/// `MAX_UNPACKED_BYTES` of `routes::uploads`: 2 GiB, the same number as
/// [`SESSION_CAP`] and a constant of its own. It is the `max_bytes` the job is
/// told (it was `MAX_UPLOAD_BYTES`, 52,428,800, so the job refused a gzip above
/// 50 MiB unpacked) and the most a gzip, a zip or a grouped load may add up to.
const UNPACKED_CAP: u64 = 2 * 1024 * 1024 * 1024;

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/../../../ops/fixtures/uploads/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

/// A workbook fixture of `ops/fixtures/workbooks/`.
fn workbook(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/../../../ops/fixtures/workbooks/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

/// A fixture of `ops/fixtures/uploads/formats/` (`DATA-17` slice 4).
fn format_file(name: &str) -> Vec<u8> {
    fixture(&format!("formats/{name}"))
}

/// A Parquet fixture of `ops/fixtures/parquet/`.
fn parquet_file(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/../../../ops/fixtures/parquet/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .fold(String::new(), |mut hex, byte| {
            let _ = write!(hex, "{byte:02x}");
            hex
        })
}

// ── the warehouse bucket ─────────────────────────────────────────────────

const OBJECT_PATH: &str = r"^/lakehouse-warehouse/uploads/.+";

/// The multipart uploads a fake bucket has open: S3's `uploadId` to the key
/// and the pieces received so far, by part number (`DATA-17`).
type OpenMultipart = (String, std::collections::BTreeMap<u32, Vec<u8>>);

#[derive(Clone, Default)]
struct Multiparts(Arc<Mutex<HashMap<String, OpenMultipart>>>);

/// What the fake bucket holds, by object key, and the multipart uploads it has
/// open.
#[derive(Clone, Default)]
struct Bucket(Arc<Mutex<HashMap<String, Vec<u8>>>>, Multiparts);

impl Bucket {
    fn keys(&self) -> Vec<String> {
        let mut keys: Vec<String> = self.0.lock().unwrap().keys().cloned().collect();
        keys.sort();
        keys
    }

    fn get(&self, key: &str) -> Option<Vec<u8>> {
        self.0.lock().unwrap().get(key).cloned()
    }

    /// The keys of the multipart uploads that are open (created, neither
    /// completed nor aborted).
    fn open_multiparts(&self) -> Vec<String> {
        let mut keys: Vec<String> = self
            .1
            .0
            .lock()
            .unwrap()
            .values()
            .map(|(key, _)| key.clone())
            .collect();
        keys.sort();
        keys
    }

    fn put(&self, key: &str, bytes: &[u8]) {
        self.0
            .lock()
            .unwrap()
            .insert(key.to_owned(), bytes.to_vec());
    }
}

fn key_of(request: &MockRequest) -> String {
    request
        .url
        .path()
        .trim_start_matches("/lakehouse-warehouse/")
        .to_owned()
}

fn object_headers(template: ResponseTemplate) -> ResponseTemplate {
    template
        .insert_header("ETag", "\"abc\"")
        .insert_header("Last-Modified", "Wed, 21 Oct 2015 07:28:00 GMT")
}

struct PutObject(Bucket);
impl Respond for PutObject {
    fn respond(&self, request: &MockRequest) -> ResponseTemplate {
        self.0.put(&key_of(request), &request.body);
        object_headers(ResponseTemplate::new(200))
    }
}

struct HeadObject(Bucket);
impl Respond for HeadObject {
    fn respond(&self, request: &MockRequest) -> ResponseTemplate {
        match self.0.get(&key_of(request)) {
            Some(bytes) => object_headers(
                ResponseTemplate::new(200).insert_header("Content-Length", bytes.len().to_string()),
            ),
            None => ResponseTemplate::new(404),
        }
    }
}

struct GetRange(Bucket);
impl Respond for GetRange {
    fn respond(&self, request: &MockRequest) -> ResponseTemplate {
        let Some(bytes) = self.0.get(&key_of(request)) else {
            return ResponseTemplate::new(404);
        };
        let range = request
            .headers
            .get("range")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("bytes="))
            .and_then(|v| v.split_once('-'))
            .and_then(|(a, b)| Some((a.parse::<usize>().ok()?, b.parse::<usize>().ok()?)));
        let Some((start, end)) = range else {
            return object_headers(ResponseTemplate::new(200).set_body_bytes(bytes));
        };
        let end = end.min(bytes.len().saturating_sub(1));
        object_headers(
            ResponseTemplate::new(206)
                .insert_header(
                    "Content-Range",
                    format!("bytes {start}-{end}/{}", bytes.len()),
                )
                .set_body_bytes(bytes[start..=end].to_vec()),
        )
    }
}

/// `object_store` deletes through S3's multi-object delete (`POST
/// /<bucket>?delete`), whose body names the keys.
struct BulkDelete(Bucket);
impl Respond for BulkDelete {
    fn respond(&self, request: &MockRequest) -> ResponseTemplate {
        let body = String::from_utf8_lossy(&request.body).into_owned();
        let mut deleted = String::new();
        for key in body
            .split("<Key>")
            .skip(1)
            .filter_map(|s| s.split("</Key>").next())
        {
            self.0.0.lock().unwrap().remove(key);
            let _ = write!(deleted, "<Deleted><Key>{key}</Key></Deleted>");
        }
        ResponseTemplate::new(200).set_body_string(format!(
            r#"<DeleteResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/">{deleted}</DeleteResult>"#
        ))
    }
}

/// S3's multipart calls on an object key, as `object_store` sends them:
/// `POST ?uploads` creates, `PUT ?partNumber&uploadId` stores a piece (the
/// `ETag` is a hash of its bytes, so a piece sent again with other bytes gets
/// another one), `POST ?uploadId` assembles the pieces the body lists, and
/// `DELETE ?uploadId` aborts. Like the real store, completion refuses a piece
/// whose `ETag` is not the one it holds, and a piece never sent.
struct CreateMultipart(Bucket);
impl Respond for CreateMultipart {
    fn respond(&self, request: &MockRequest) -> ResponseTemplate {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let key = key_of(request);
        let mut open = self.0.1.0.lock().unwrap();
        let upload_id = format!(
            "mp-{}",
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        );
        open.insert(
            upload_id.clone(),
            (key.clone(), std::collections::BTreeMap::new()),
        );
        ResponseTemplate::new(200).set_body_string(format!(
            r#"<InitiateMultipartUploadResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/"><Bucket>lakehouse-warehouse</Bucket><Key>{key}</Key><UploadId>{upload_id}</UploadId></InitiateMultipartUploadResult>"#
        ))
    }
}

/// A matcher for a request that carries the query parameter `name`, whatever
/// its value (`wiremock` has `query_param` for a value and no "exists").
fn has_query(name: &'static str) -> impl Fn(&MockRequest) -> bool + Send + Sync + 'static {
    move |request: &MockRequest| request.url.query_pairs().any(|(key, _)| key == name)
}

fn query_of(request: &MockRequest, name: &str) -> Option<String> {
    request
        .url
        .query_pairs()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.into_owned())
}

struct PutPart(Bucket);
impl Respond for PutPart {
    fn respond(&self, request: &MockRequest) -> ResponseTemplate {
        let (Some(id), Some(number)) = (
            query_of(request, "uploadId"),
            query_of(request, "partNumber").and_then(|n| n.parse::<u32>().ok()),
        ) else {
            return ResponseTemplate::new(400);
        };
        let mut open = self.0.1.0.lock().unwrap();
        let Some((_, parts)) = open.get_mut(&id) else {
            return ResponseTemplate::new(404)
                .set_body_string("<Error><Code>NoSuchUpload</Code></Error>");
        };
        parts.insert(number, request.body.clone());
        ResponseTemplate::new(200)
            .insert_header("ETag", format!("\"{}\"", &sha256_hex(&request.body)[..32]))
    }
}

struct CompleteMultipart(Bucket);
impl Respond for CompleteMultipart {
    fn respond(&self, request: &MockRequest) -> ResponseTemplate {
        let Some(id) = query_of(request, "uploadId") else {
            return ResponseTemplate::new(400);
        };
        let body = String::from_utf8_lossy(&request.body).into_owned();
        let mut listed = Vec::new();
        for part in body.split("<Part>").skip(1) {
            let number = part
                .split("<PartNumber>")
                .nth(1)
                .and_then(|s| s.split("</PartNumber>").next())
                .and_then(|s| s.parse::<u32>().ok());
            let etag = part
                .split("<ETag>")
                .nth(1)
                .and_then(|s| s.split("</ETag>").next())
                .map(|s| s.replace("&quot;", "\"").trim_matches('"').to_owned());
            if let (Some(number), Some(etag)) = (number, etag) {
                listed.push((number, etag));
            }
        }
        let mut open = self.0.1.0.lock().unwrap();
        let Some((key, parts)) = open.get(&id).cloned() else {
            return ResponseTemplate::new(404)
                .set_body_string("<Error><Code>NoSuchUpload</Code></Error>");
        };
        let mut object = Vec::new();
        for (number, etag) in &listed {
            match parts.get(number) {
                Some(bytes) if &sha256_hex(bytes)[..32] == etag => object.extend_from_slice(bytes),
                _ => {
                    return ResponseTemplate::new(400)
                        .set_body_string("<Error><Code>InvalidPart</Code></Error>");
                }
            }
        }
        open.remove(&id);
        self.0.put(&key, &object);
        ResponseTemplate::new(200).set_body_string(format!(
            r#"<CompleteMultipartUploadResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/"><Location>x</Location><Bucket>lakehouse-warehouse</Bucket><Key>{key}</Key><ETag>"done"</ETag></CompleteMultipartUploadResult>"#
        ))
    }
}

struct AbortMultipart(Bucket);
impl Respond for AbortMultipart {
    fn respond(&self, request: &MockRequest) -> ResponseTemplate {
        let removed =
            query_of(request, "uploadId").and_then(|id| self.0.1.0.lock().unwrap().remove(&id));
        match removed {
            Some(_) => ResponseTemplate::new(204),
            None => ResponseTemplate::new(404)
                .set_body_string("<Error><Code>NoSuchUpload</Code></Error>"),
        }
    }
}

async fn mount_bucket(server: &MockServer, bucket: &Bucket) {
    // Priority 2 beats the whole-object mocks below (default 5) for a request
    // that carries the multipart query.
    Mock::given(method("POST"))
        .and(path_regex(OBJECT_PATH))
        .and(has_query("uploads"))
        .respond_with(CreateMultipart(bucket.clone()))
        .with_priority(2)
        .mount(server)
        .await;
    Mock::given(method("PUT"))
        .and(path_regex(OBJECT_PATH))
        .and(has_query("uploadId"))
        .respond_with(PutPart(bucket.clone()))
        .with_priority(2)
        .mount(server)
        .await;
    Mock::given(method("POST"))
        .and(path_regex(OBJECT_PATH))
        .and(has_query("uploadId"))
        .respond_with(CompleteMultipart(bucket.clone()))
        .with_priority(2)
        .mount(server)
        .await;
    Mock::given(method("DELETE"))
        .and(path_regex(OBJECT_PATH))
        .and(has_query("uploadId"))
        .respond_with(AbortMultipart(bucket.clone()))
        .with_priority(2)
        .mount(server)
        .await;
    Mock::given(method("PUT"))
        .and(path_regex(OBJECT_PATH))
        .respond_with(PutObject(bucket.clone()))
        .mount(server)
        .await;
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
    Mock::given(method("POST"))
        .and(path("/lakehouse-warehouse"))
        .and(query_param("delete", ""))
        .respond_with(BulkDelete(bucket.clone()))
        .mount(server)
        .await;
}

/// Every request to the bucket is refused, with `MARKER` in the body the way a
/// real store names its internals. A 403, not a 500: `object_store` retries a
/// 5xx for the better part of a minute, and a refusal is not retried. Priority
/// 1: it wins over the working mocks.
async fn break_bucket(server: &MockServer) {
    Mock::given(any())
        .respond_with(ResponseTemplate::new(403).set_body_string(format!("{MARKER} <Error/>")))
        .with_priority(1)
        .mount(server)
        .await;
}

// ── Dagster and ClickHouse ───────────────────────────────────────────────

async fn mount_launch(dagster: &MockServer, run_id: &str) {
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .and(body_string_contains("launchRun"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": { "launchRun": { "__typename": "LaunchRunSuccess", "run": { "runId": run_id } } }
        })))
        .mount(dagster)
        .await;
}

async fn mount_launch_refused(dagster: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .and(body_string_contains("launchRun"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": { "launchRun": { "__typename": "PythonError", "message": format!("{MARKER} traceback") } }
        })))
        .mount(dagster)
        .await;
}

async fn mount_run_status(dagster: &MockServer, status: &str) {
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .and(body_string_contains("pipelineRunOrError"))
        .respond_with(ResponseTemplate::new(200).set_body_json(run_status_body(status)))
        .mount(dagster)
        .await;
}

/// The orchestrator does not know the run: the shape its API answers for a run
/// id it never had or has lost (`RunNotFoundError`), with `MARKER` in the
/// message the API must not repeat.
async fn mount_unknown_run(dagster: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .and(body_string_contains("pipelineRunOrError"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": { "pipelineRunOrError": {
                "__typename": "RunNotFoundError",
                "message": format!("Run run-1 could not be found {MARKER}")
            } }
        })))
        .mount(dagster)
        .await;
}

fn run_status_body(status: &str) -> Value {
    json!({ "data": { "pipelineRunOrError": {
        "__typename": "Run", "status": status,
        "startTime": 1_790_670_111.0, "endTime": null, "stepStats": []
    } } })
}

fn rows_body(data: &Value) -> Value {
    json!({ "meta": [], "data": data, "rows": data.as_array().map_or(0, Vec::len) })
}

/// The table name is free: no registry row, and `DESCRIBE` says what
/// `ClickHouse` says for a table that is not there.
async fn mount_free_table(ch: &MockServer) {
    Mock::given(method("POST"))
        .and(body_string_contains("dataset_catalog"))
        .respond_with(ResponseTemplate::new(200).set_body_json(rows_body(&json!([]))))
        .mount(ch)
        .await;
    Mock::given(method("POST"))
        .and(body_string_contains("DESCRIBE TABLE"))
        .respond_with(ResponseTemplate::new(404).set_body_string(
            "Code: 60. DB::Exception: Table icecat_api.`bronze.x` does not exist. \
             (UNKNOWN_TABLE) (version 26.8.1.1 (official build))",
        ))
        .mount(ch)
        .await;
}

async fn mount_registered(ch: &MockServer) {
    Mock::given(method("POST"))
        .and(body_string_contains("dataset_catalog"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(rows_body(&json!([{ "slug": "stock-raw" }]))),
        )
        .mount(ch)
        .await;
}

async fn mount_results(ch: &MockServer, results: &Value) {
    Mock::given(method("POST"))
        .and(body_string_contains("bronze_meta.ingest_run"))
        .respond_with(ResponseTemplate::new(200).set_body_json(rows_body(results)))
        .mount(ch)
        .await;
}

fn result_row(
    upload_id: &str,
    status: &str,
    rows: Option<u64>,
    error: &str,
    ended_ago: i64,
) -> Value {
    let ended = OffsetDateTime::now_utc() - time::Duration::minutes(ended_ago);
    json!({
        "connector_id": format!("upload:{upload_id}"), "job": "file_ingest_job",
        "object": "stock_raw", "rows": rows,
        "started_at": ended.format(&Rfc3339).unwrap(), "ended_at": ended.format(&Rfc3339).unwrap(),
        "status": status, "error": error,
    })
}

/// How many requests had `needle` in their body (`ClickHouse`'s are plain SQL).
async fn count_requests_containing(server: &MockServer, needle: &str) -> usize {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| String::from_utf8_lossy(&r.body).contains(needle))
        .count()
}

/// The JSON bodies of the requests that had `needle` in them (Dagster's).
async fn requests_containing(server: &MockServer, needle: &str) -> Vec<Value> {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| String::from_utf8_lossy(&r.body).contains(needle))
        .map(|r| serde_json::from_slice(&r.body).unwrap_or(Value::Null))
        .collect()
}

// ── the stack ────────────────────────────────────────────────────────────

/// What differs between stacks: the Iceberg query database, and where
/// `Dagster` and `ClickHouse` are when a test wants them not to be at the
/// mock's address.
struct Options {
    iceberg_query_db: Option<&'static str>,
    dagster_url: Option<&'static str>,
    ch_url: Option<&'static str>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            iceberg_query_db: Some("icecat_api"),
            dagster_url: None,
            ch_url: None,
        }
    }
}

struct Stack {
    app: TestApp,
    bucket: Bucket,
    s3: MockServer,
    dagster: MockServer,
    ch: MockServer,
}

struct Reply {
    status: StatusCode,
    json: Value,
    text: String,
    /// The `Retry-After` header, as sent.
    retry_after: Option<String>,
}

enum Payload {
    None,
    Json(Value),
    Raw(&'static str, Vec<u8>),
}

/// A multipart form with one `file` part.
fn file_form(filename: &str, content_type: &str, bytes: &[u8]) -> Payload {
    form(&[("file", Some(filename), content_type, bytes)])
}

fn form(parts: &[(&str, Option<&str>, &str, &[u8])]) -> Payload {
    let mut body = Vec::new();
    for (name, filename, content_type, bytes) in parts {
        body.extend_from_slice(b"--BOUNDARY\r\n");
        let disposition = filename.map_or_else(
            || format!("Content-Disposition: form-data; name=\"{name}\"\r\n"),
            |f| format!("Content-Disposition: form-data; name=\"{name}\"; filename=\"{f}\"\r\n"),
        );
        body.extend_from_slice(disposition.as_bytes());
        body.extend_from_slice(format!("Content-Type: {content_type}\r\n\r\n").as_bytes());
        body.extend_from_slice(bytes);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(b"--BOUNDARY--\r\n");
    Payload::Raw("multipart/form-data; boundary=BOUNDARY", body)
}

/// A signed-in caller: the session cookie and the tenant sent as `X-Tenant`.
struct Caller {
    cookie: String,
    tenant: Option<&'static str>,
}

impl Stack {
    async fn start() -> Self {
        Self::with(Options::default()).await
    }

    async fn with(options: Options) -> Self {
        let s3 = MockServer::start().await;
        let dagster = MockServer::start().await;
        let ch = MockServer::start().await;
        let bucket = Bucket::default();
        mount_bucket(&s3, &bucket).await;
        let mut env: HashMap<String, String> = HashMap::from([
            ("CH_URL".to_owned(), ch.uri()),
            (
                "DAGSTER_URL".to_owned(),
                format!("{}/graphql", dagster.uri()),
            ),
            ("RUSTFS_S3_ENDPOINT".to_owned(), s3.uri()),
            (
                "RUSTFS_ACCESS_KEY_SECRET_REF".to_owned(),
                "env:CARGO_PKG_NAME".to_owned(),
            ),
            (
                "RUSTFS_SECRET_KEY_SECRET_REF".to_owned(),
                "env:CARGO_PKG_VERSION".to_owned(),
            ),
        ]);
        if let Some(db) = options.iceberg_query_db {
            env.insert("ICEBERG_QUERY_DB".to_owned(), db.to_owned());
        }
        if let Some(url) = options.dagster_url {
            env.insert("DAGSTER_URL".to_owned(), url.to_owned());
        }
        if let Some(url) = options.ch_url {
            env.insert("CH_URL".to_owned(), url.to_owned());
        }
        let app = spin_up_with_env(&env).await;
        Self {
            app,
            bucket,
            s3,
            dagster,
            ch,
        }
    }

    async fn caller(&self, email: &str, tenant: Option<&'static str>) -> Caller {
        Caller {
            cookie: session_cookie_for_seeded_user(&self.app.pool, email).await,
            tenant,
        }
    }

    async fn bayu(&self) -> Caller {
        self.caller("bayu@meridian.example", Some(GROUP)).await
    }

    async fn andi(&self) -> Caller {
        self.caller("andi@meridian.example", Some(RETAIL)).await
    }

    async fn send(&self, who: &Caller, http_method: &str, uri: &str, payload: Payload) -> Reply {
        let mut request = Request::builder()
            .method(http_method)
            .uri(uri)
            .header("cookie", &who.cookie);
        if let Some(tenant) = who.tenant {
            request = request.header("x-tenant", tenant);
        }
        let body = match payload {
            Payload::None => Body::empty(),
            Payload::Json(value) => {
                request = request.header("content-type", "application/json");
                Body::from(value.to_string())
            }
            Payload::Raw(content_type, bytes) => {
                request = request.header("content-type", content_type);
                Body::from(bytes)
            }
        };
        let response = self
            .app
            .router
            .clone()
            .oneshot(request.body(body).unwrap())
            .await
            .expect("router never fails a request outright");
        let status = response.status();
        let retry_after = response
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let text = String::from_utf8_lossy(&bytes).into_owned();
        Reply {
            status,
            json: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
            text,
            retry_after,
        }
    }

    async fn get(&self, who: &Caller, uri: &str) -> Reply {
        self.send(who, "GET", uri, Payload::None).await
    }

    /// Upload `bytes` through the route; the answer must be 201.
    async fn upload(&self, who: &Caller, filename: &str, bytes: &[u8]) -> Value {
        let reply = self
            .send(
                who,
                "POST",
                "/api/uploads",
                file_form(filename, "text/csv", bytes),
            )
            .await;
        assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.text);
        reply.json
    }

    async fn ingest(&self, who: &Caller, id: &str, body: Value) -> Reply {
        self.send(
            who,
            "POST",
            &format!("/api/uploads/{id}/ingest"),
            Payload::Json(body),
        )
        .await
    }

    /// Put an upload of `tenant` in the store and its bytes in the bucket,
    /// without going through the route.
    async fn seed(&self, tenant: &str, id: &str, bytes: &[u8]) -> Upload {
        let tenant_id: Uuid = tenant.parse().unwrap();
        let key = format!("uploads/{tenant_id}/{id}.csv");
        self.bucket.put(&key, bytes);
        uploads::insert(
            &self.app.pool,
            &NewUpload {
                id,
                original_filename: "stock.csv",
                storage_key: &key,
                content_type: "text/csv",
                size_bytes: i64::try_from(bytes.len()).unwrap(),
                sha256: &sha256_hex(bytes),
                uploaded_by: "Seeded Uploader",
                tenant_id,
            },
        )
        .await
        .unwrap()
    }

    /// Claim `table` for `tenant` on behalf of upload `id`, as the ingest route
    /// does just before it marks the upload as loading. The claim must be
    /// allowed.
    async fn claim(&self, tenant: &str, table: &str, id: &str) {
        let tenant_id: Uuid = tenant.parse().unwrap();
        assert!(
            uploads::claim_table(&self.app.pool, tenant_id, table, id)
                .await
                .unwrap(),
            "{table} could not be claimed for {tenant}"
        );
    }

    /// The rows of `upload_table_claim`: `(table, tenant, upload that asked
    /// first)`, by table name.
    async fn claims(&self) -> Vec<(String, Option<Uuid>, String)> {
        sqlx::query_as(
            "SELECT bronze_table, tenant_id, upload_id FROM upload_table_claim \
             ORDER BY bronze_table",
        )
        .fetch_all(&self.app.pool)
        .await
        .unwrap()
    }

    /// Put an upload in the state a launched load leaves it in, claimed
    /// `age` ago, with the table claimed for its tenant.
    async fn seed_loading(
        &self,
        tenant: &str,
        id: &str,
        table: &str,
        run_id: Option<&str>,
        age: &str,
    ) {
        self.seed(tenant, id, b"id,name\n1,a\n").await;
        self.claim(tenant, table, id).await;
        let options = json!({ "encoding": "utf-8", "delimiter": ",", "headerRow": 0 });
        uploads::mark_ingesting(&self.app.pool, id, &options, table, LoadMode::Replace, None)
            .await
            .unwrap()
            .unwrap();
        if let Some(run_id) = run_id {
            uploads::attach_run(&self.app.pool, id, run_id)
                .await
                .unwrap()
                .unwrap();
        }
        sqlx::query(&format!(
            "UPDATE file_upload SET updated_at = now() - interval '{age}' WHERE id = $1"
        ))
        .bind(id)
        .execute(&self.app.pool)
        .await
        .unwrap();
    }

    /// Put an upload in the state of a load that succeeded into `table`, with
    /// the table claimed for its tenant.
    async fn seed_ingested(&self, tenant: &str, id: &str, table: &str) {
        self.seed(tenant, id, b"id,name\n1,a\n").await;
        self.claim(tenant, table, id).await;
        let options = json!({ "encoding": "utf-8", "delimiter": ",", "headerRow": 0 });
        uploads::mark_ingesting(
            &self.app.pool,
            id,
            &options,
            table,
            LoadMode::Replace,
            Some("run-0"),
        )
        .await
        .unwrap()
        .unwrap();
        uploads::mark_finished(&self.app.pool, id, Some("run-0"), None, Some(1))
            .await
            .unwrap()
            .unwrap();
    }

    async fn status_of(&self, id: &str) -> (String, Option<String>, Option<String>) {
        sqlx::query_as("SELECT status, run_id, error FROM file_upload WHERE id = $1")
            .bind(id)
            .fetch_one(&self.app.pool)
            .await
            .unwrap()
    }

    async fn audit_actions(&self) -> Vec<(String, Option<String>, Option<String>, Value)> {
        sqlx::query_as(
            "SELECT action, resource_kind, resource_id, args FROM audit_event \
             WHERE action LIKE 'upload.%' ORDER BY at",
        )
        .fetch_all(&self.app.pool)
        .await
        .unwrap()
    }
}

fn ingest_body(table: &str) -> Value {
    json!({ "bronzeTable": table, "encoding": "utf-8", "delimiter": ",", "headerRow": 0 })
}

fn id_of(upload: &Value) -> String {
    upload["id"].as_str().unwrap().to_owned()
}

/// A CSV of about `target` bytes: a header and numbered rows.
fn csv_of_size(target: usize) -> Vec<u8> {
    let mut text = String::from("id,name\n");
    let mut n = 0_u64;
    while text.len() < target {
        let _ = writeln!(text, "{n},name_{n}");
        n += 1;
    }
    text.into_bytes()
}

// ════════════════════════════════════════════════════════════════════════
// POST /api/uploads
// ════════════════════════════════════════════════════════════════════════

/// The upload goes to the bucket under a key made of server-generated parts,
/// the row records the caller's tenant, and the response carries neither.
#[tokio::test]
async fn an_upload_is_stored_under_a_server_made_key_and_answered_without_internals() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    let bytes = fixture("quoted_comma_newline.csv");

    let upload = stack.upload(&bayu, "../../quoted comma.csv", &bytes).await;

    let id = id_of(&upload);
    assert!(id.starts_with("up-"), "{id}");
    assert_eq!(
        upload["originalFilename"], "quoted comma.csv",
        "the last path segment"
    );
    assert_eq!(upload["sizeBytes"], bytes.len());
    assert_eq!(upload["status"], "uploaded");
    assert_eq!(upload["uploadedBy"], "Bayu Pratama");
    assert!(upload["createdAt"].as_str().unwrap().ends_with('Z'));
    for hidden in [
        "storageKey",
        "tenantId",
        "tenant",
        "contentType",
        "sha256",
        "duplicateOf",
        "rows",
        "runId",
        "assetId",
        "error",
        "bronzeTable",
        "loadMode",
        "parseOptions",
    ] {
        assert!(
            upload.get(hidden).is_none(),
            "{hidden} is not on the wire: {upload}"
        );
    }
    let raw = upload.to_string();
    assert!(!raw.contains("uploads/") && !raw.contains(GROUP), "{raw}");

    let key = format!("uploads/{GROUP}/{id}.csv");
    assert_eq!(stack.bucket.keys(), std::slice::from_ref(&key));
    assert_eq!(stack.bucket.get(&key).unwrap(), bytes);
    let (tenant, sha, kind): (Uuid, String, String) =
        sqlx::query_as("SELECT tenant_id, sha256, content_type FROM file_upload WHERE id = $1")
            .bind(&id)
            .fetch_one(&stack.app.pool)
            .await
            .unwrap();
    assert_eq!(tenant.to_string(), GROUP);
    assert_eq!(sha, sha256_hex(&bytes));
    assert_eq!(kind, "text/csv");

    let audit = stack.audit_actions().await;
    assert_eq!(audit.len(), 1, "{audit:?}");
    assert_eq!(audit[0].0, "upload.create");
    assert_eq!(audit[0].1.as_deref(), Some("upload"));
    assert_eq!(audit[0].2.as_deref(), Some(id.as_str()));
    assert_eq!(
        audit[0].3,
        json!({ "fileName": "quoted comma.csv", "sizeBytes": bytes.len() }),
        "the file's name and size, never a row of it"
    );
}

/// The same bytes from the same tenant are reported, not refused; another
/// tenant's identical upload is not reported (it would tell the caller what
/// other tenants hold).
#[tokio::test]
async fn a_repeated_file_names_the_earlier_upload_of_the_same_tenant_only() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    let andi = stack.andi().await;
    let bytes = fixture("semicolon.csv");

    let first = stack.upload(&bayu, "first.csv", &bytes).await;
    assert!(first.get("duplicateOf").is_none());

    let second = stack.upload(&bayu, "second.csv", &bytes).await;
    let earlier = &second["duplicateOf"];
    assert_eq!(earlier["id"], first["id"]);
    assert_eq!(earlier["originalFilename"], "first.csv");
    assert_eq!(earlier["status"], "uploaded");
    assert!(earlier["createdAt"].as_str().is_some());
    for hidden in ["sha256", "storageKey", "tenantId", "contentType"] {
        assert!(earlier.get(hidden).is_none(), "{hidden}");
    }

    let other_tenant = stack.upload(&andi, "same-bytes.csv", &bytes).await;
    assert!(
        other_tenant.get("duplicateOf").is_none(),
        "tenant B must not learn that tenant A holds these bytes: {other_tenant}"
    );
}

/// axum's default body limit is 2 MB; this route takes up to the cap.
#[tokio::test]
async fn a_3_mb_upload_is_accepted() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    let bytes = csv_of_size(3 * 1024 * 1024);

    let upload = stack.upload(&bayu, "big.csv", &bytes).await;

    assert_eq!(upload["sizeBytes"], bytes.len());
    let key = format!("uploads/{GROUP}/{}.csv", id_of(&upload));
    assert_eq!(stack.bucket.get(&key).unwrap().len(), bytes.len());
}

/// The raised body limit belongs to `POST /api/uploads` alone: a body of the
/// same size sent to another route keeps axum's 2 MB default.
#[tokio::test]
async fn the_raised_body_limit_belongs_to_the_upload_route_alone() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    let upload = stack.upload(&bayu, "a.csv", b"a,b\n1,2\n").await;
    let mut body = ingest_body("stock_raw");
    body["padding"] = json!("x".repeat(3 * 1024 * 1024));

    let reply = stack.ingest(&bayu, &id_of(&upload), body).await;

    assert_eq!(
        reply.status,
        StatusCode::PAYLOAD_TOO_LARGE,
        "{}",
        reply.text
    );
    assert!(stack.dagster.received_requests().await.unwrap().is_empty());
}

/// A body past the cap plus the form's framing is cut off by the body limit;
/// the answer is the fixed sentence, and nothing is stored or recorded.
#[tokio::test]
async fn a_body_past_the_limit_is_refused_with_the_fixed_text_and_stores_nothing() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    let bytes = vec![b'a'; CAP + 2 * 1024 * 1024];

    let reply = stack
        .send(
            &bayu,
            "POST",
            "/api/uploads",
            file_form("huge.csv", "text/csv", &bytes),
        )
        .await;

    assert_eq!(reply.status, StatusCode::BAD_REQUEST, "{}", reply.text);
    assert_eq!(
        reply.json["error"],
        "The file is larger than the 50 MB limit."
    );
    assert!(stack.bucket.keys().is_empty());
    assert!(stack.audit_actions().await.is_empty());
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM file_upload")
        .fetch_one(&stack.app.pool)
        .await
        .unwrap();
    assert_eq!(rows, 0);
}

/// A file over the cap but inside the body limit (the cap plus 1 MiB) gets
/// past the limit and is refused by size, with the same sentence.
#[tokio::test]
async fn a_file_one_byte_over_the_cap_is_refused_by_size() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    let bytes = vec![b'a'; CAP + 1];

    let reply = stack
        .send(
            &bayu,
            "POST",
            "/api/uploads",
            file_form("over.csv", "text/csv", &bytes),
        )
        .await;

    assert_eq!(reply.status, StatusCode::BAD_REQUEST, "{}", reply.text);
    assert_eq!(
        reply.json["error"],
        "The file is larger than the 50 MB limit."
    );
    assert!(stack.bucket.keys().is_empty());
}

/// The first bytes decide, never the name or the type the browser claimed.
#[tokio::test]
async fn a_file_is_judged_by_its_first_bytes_not_by_its_name() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    let refuse = |name: &'static str, bytes: Vec<u8>| {
        let stack = &stack;
        let bayu = &bayu;
        async move {
            let reply = stack
                .send(
                    bayu,
                    "POST",
                    "/api/uploads",
                    file_form(name, "text/csv", &bytes),
                )
                .await;
            assert_eq!(
                reply.status,
                StatusCode::BAD_REQUEST,
                "{name}: {}",
                reply.text
            );
            reply.json["error"].as_str().unwrap().to_owned()
        }
    };

    // X2 of the Excel plan: a zip or OLE file is refused with the sentence of
    // the workbook kinds that are not accepted unless it is named `.xls` or
    // `.xlsx` (then it must open: see the workbook tests below).
    let workbook = refuse(
        "data.csv",
        b"PK\x03\x04\x14\x00\x06\x00rest of a zip".to_vec(),
    )
    .await;
    assert_eq!(
        workbook,
        "This looks like a workbook or a zip archive that is not named like one. Excel workbooks must end in .xls or .xlsx and zip archives in .zip; or save the sheet as .xlsx or CSV first."
    );
    assert!(
        refuse(
            "old.dat",
            vec![0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1]
        )
        .await
        .contains("not named like one")
    );
    // Q2 of the Parquet plan: a Parquet-shaped file is refused unless it is
    // named `.parquet` (then it must open: see the Parquet tests below).
    assert_eq!(
        refuse("t.csv", b"PAR1\x15\x04\x15\x30rest".to_vec()).await,
        "This looks like a Parquet file, but its name does not end in .parquet. Rename it to end in .parquet and upload it again."
    );
    assert!(
        refuse("report.pdf", b"%PDF-1.7 ...".to_vec())
            .await
            .contains("not a file this upload reads")
    );
    assert!(
        refuse("x.csv", vec![0x1F, 0x8B, 8, 0, 0, 0])
            .await
            .contains("does not end in .gz")
    );
    assert!(
        stack.bucket.keys().is_empty(),
        "nothing was stored for a refused file"
    );

    // And the other way: text called a workbook is text.
    let upload = stack.upload(&bayu, "stock.xlsx", b"id,name\n1,a\n").await;
    assert_eq!(upload["originalFilename"], "stock.xlsx");
    let key = format!("uploads/{GROUP}/{}.xlsx", id_of(&upload));
    assert_eq!(stack.bucket.keys(), [key]);
}

#[tokio::test]
async fn an_empty_file_a_missing_part_and_a_request_that_is_no_form_are_each_400() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    let post = |payload| stack.send(&bayu, "POST", "/api/uploads", payload);

    let empty = post(file_form("empty.csv", "text/csv", b"")).await;
    assert_eq!(empty.status, StatusCode::BAD_REQUEST);
    assert_eq!(empty.json["error"], "The file is empty.");

    let wrong_name = post(form(&[(
        "attachment",
        Some("a.csv"),
        "text/csv",
        b"a,b\n1,2\n",
    )]))
    .await;
    assert_eq!(wrong_name.status, StatusCode::BAD_REQUEST);
    assert_eq!(wrong_name.json["error"], "The form has no part named file.");

    let no_parts = post(Payload::Raw(
        "multipart/form-data; boundary=BOUNDARY",
        b"--BOUNDARY--\r\n".to_vec(),
    ))
    .await;
    assert_eq!(no_parts.status, StatusCode::BAD_REQUEST);
    assert_eq!(no_parts.json["error"], "The form has no part named file.");

    for payload in [
        Payload::None,
        Payload::Json(json!({ "file": "a,b" })),
        Payload::Raw("multipart/form-data", b"no boundary".to_vec()),
    ] {
        let reply = post(payload).await;
        assert_eq!(reply.status, StatusCode::BAD_REQUEST, "{}", reply.text);
        assert_eq!(
            reply.json["error"],
            "The request must be a multipart form with one part named file."
        );
    }
    assert!(stack.bucket.keys().is_empty());
}

/// A part that is not the file does not stand in for it, and a form read
/// partway (a cut-off body) says so in one fixed sentence.
#[tokio::test]
async fn a_form_cut_off_mid_part_is_unreadable_in_one_fixed_sentence() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    let reply = stack
        .send(
            &bayu,
            "POST",
            "/api/uploads",
            Payload::Raw(
                "multipart/form-data; boundary=BOUNDARY",
                b"--BOUNDARY\r\nContent-Disposition: form-data; name=\"file\"; filename=\"a.csv\"\r\n\r\na,b\n1,2".to_vec(),
            ),
        )
        .await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST, "{}", reply.text);
    assert_eq!(reply.json["error"], "The upload could not be read.");
    assert!(stack.bucket.keys().is_empty());
}

/// A caller in no tenant has nowhere to put a file, and sees an empty list,
/// never "every tenant's uploads".
#[tokio::test]
async fn a_caller_in_no_tenant_cannot_upload_and_lists_nothing() {
    let stack = Stack::start().await;
    let user = create_principal_with_permissions(&stack.app.pool, "connector:manage").await;
    let nobody = Caller {
        cookie: session_cookie_for_user(&stack.app.pool, user).await,
        tenant: None,
    };

    let reply = stack
        .send(
            &nobody,
            "POST",
            "/api/uploads",
            file_form("a.csv", "text/csv", b"a,b\n1,2\n"),
        )
        .await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        reply.json["error"],
        "Your account belongs to no tenant, so a file cannot be uploaded."
    );

    stack.seed(GROUP, "up-someone-elses", b"id\n1\n").await;
    let list = stack.get(&nobody, "/api/uploads").await;
    assert_eq!(list.status, StatusCode::OK);
    assert_eq!(list.json, json!([]));
}

/// `X-Tenant` names a tenant the caller is not in: the same 404 as
/// everywhere else, and nothing is stored.
#[tokio::test]
async fn an_x_tenant_the_caller_does_not_belong_to_is_404_and_stores_nothing() {
    let stack = Stack::start().await;
    let andi = stack.caller("andi@meridian.example", Some(GROUP)).await;
    let reply = stack
        .send(
            &andi,
            "POST",
            "/api/uploads",
            file_form("a.csv", "text/csv", b"a,b\n1,2\n"),
        )
        .await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    assert!(stack.bucket.keys().is_empty());
}

/// A deployment that has no storage credentials answers 503 with the fixed
/// sentence, before the body is read.
#[tokio::test]
async fn an_unconfigured_store_is_a_fixed_503() {
    let app = common::spin_up().await;
    let cookie = session_cookie_for_seeded_user(&app.pool, "bayu@meridian.example").await;
    let Payload::Raw(content_type, bytes) = file_form("a.csv", "text/csv", b"a,b\n1,2\n") else {
        unreachable!("file_form builds a raw payload")
    };
    let response = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/uploads")
                .header("cookie", cookie)
                .header("x-tenant", GROUP)
                .header("content-type", content_type)
                .body(Body::from(bytes))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap()).unwrap();
    assert_eq!(body["error"], "Upload storage is not configured.");
}

/// A storage failure is a fixed 503 sentence (the classified word, not the
/// response body), writes no row, and on preview and delete leaves the
/// upload as it was.
#[tokio::test]
async fn a_storage_failure_never_puts_the_stores_own_text_in_a_response() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    stack.seed(GROUP, "up-existing", b"id,name\n1,a\n").await;
    break_bucket(&stack.s3).await;

    let create = stack
        .send(
            &bayu,
            "POST",
            "/api/uploads",
            file_form("a.csv", "text/csv", b"a,b\n1,2\n"),
        )
        .await;
    let preview = stack.get(&bayu, "/api/uploads/up-existing/preview").await;
    let delete = stack
        .send(&bayu, "DELETE", "/api/uploads/up-existing", Payload::None)
        .await;

    for (what, reply) in [
        ("create", &create),
        ("preview", &preview),
        ("delete", &delete),
    ] {
        assert_eq!(
            reply.status,
            StatusCode::SERVICE_UNAVAILABLE,
            "{what}: {}",
            reply.text
        );
        assert!(
            reply.json["error"]
                .as_str()
                .unwrap()
                .starts_with("Upload storage is unavailable ("),
            "{what}: {}",
            reply.text
        );
        assert!(
            !reply.text.contains(MARKER),
            "{what} leaked the store's text: {}",
            reply.text
        );
    }
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM file_upload")
        .fetch_one(&stack.app.pool)
        .await
        .unwrap();
    assert_eq!(
        rows, 1,
        "the failed upload wrote no row, and a delete that could not remove the object kept the one there was"
    );
    assert_eq!(stack.status_of("up-existing").await.0, "uploaded");
}

/// The object goes in first so that a row always has its bytes; when the row
/// cannot be written the object is taken back out, and the database's own
/// text does not reach the response.
#[tokio::test]
async fn a_row_that_cannot_be_written_takes_its_object_back_out_and_leaks_nothing() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    sqlx::query("ALTER TABLE file_upload ADD CONSTRAINT never_ok CHECK (id = 'x') NOT VALID")
        .execute(&stack.app.pool)
        .await
        .unwrap();

    let reply = stack
        .send(
            &bayu,
            "POST",
            "/api/uploads",
            file_form("a.csv", "text/csv", b"a,b\n1,2\n"),
        )
        .await;

    assert_eq!(
        reply.status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "{}",
        reply.text
    );
    assert_eq!(reply.json["error"], "database error");
    for leaked in ["never_ok", "file_upload", "violates", "constraint"] {
        assert!(!reply.text.contains(leaked), "{leaked}: {}", reply.text);
    }
    assert!(
        stack.bucket.keys().is_empty(),
        "the stored object was removed again"
    );
}

/// The audit trail is best effort: an upload whose audit row cannot be
/// written is still an upload.
#[tokio::test]
async fn a_failed_audit_write_never_fails_the_request() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    sqlx::query("ALTER TABLE audit_event RENAME TO audit_event_gone")
        .execute(&stack.app.pool)
        .await
        .unwrap();

    let upload = stack.upload(&bayu, "a.csv", b"a,b\n1,2\n").await;

    assert_eq!(upload["status"], "uploaded");
    assert_eq!(stack.bucket.keys().len(), 1);
}

// ════════════════════════════════════════════════════════════════════════
// Reading: list, get, and the tenant rule
// ════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn the_list_is_the_active_tenants_uploads_newest_first() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    let andi = stack.andi().await;
    let first = stack.upload(&bayu, "a.csv", b"a,b\n1,2\n").await;
    let second = stack.upload(&bayu, "b.csv", b"a,b\n3,4\n").await;
    let retail = stack.upload(&andi, "c.csv", b"a,b\n5,6\n").await;
    sqlx::query("UPDATE file_upload SET created_at = now() - interval '1 hour' WHERE id = $1")
        .bind(id_of(&first))
        .execute(&stack.app.pool)
        .await
        .unwrap();

    let list = stack.get(&bayu, "/api/uploads").await;
    assert_eq!(list.status, StatusCode::OK, "{}", list.text);
    let ids: Vec<&str> = list
        .json
        .as_array()
        .unwrap()
        .iter()
        .map(|u| u["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        ids,
        [
            second["id"].as_str().unwrap(),
            first["id"].as_str().unwrap()
        ]
    );
    assert!(
        !list.text.contains(&id_of(&retail)),
        "another tenant's upload is not listed"
    );
    for hidden in ["storageKey", "tenantId", "sha256", "contentType"] {
        assert!(!list.text.contains(hidden), "{hidden}");
    }

    let retail_list = stack.get(&andi, "/api/uploads").await;
    assert_eq!(retail_list.json.as_array().unwrap().len(), 1);
    assert_eq!(retail_list.json[0]["id"], retail["id"]);

    // Bayu is also in Meridian Logistics, where he has uploaded nothing.
    let logistics = stack.caller("bayu@meridian.example", Some(LOGISTICS)).await;
    assert_eq!(stack.get(&logistics, "/api/uploads").await.json, json!([]));
}

/// Another tenant's upload answers exactly like one that does not exist, on
/// every per-id route, and nothing is changed or launched.
#[tokio::test]
async fn another_tenants_upload_is_not_found_on_every_per_id_route() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    let andi = stack.andi().await;
    let upload = stack.upload(&bayu, "a.csv", b"id,name\n1,a\n").await;
    let id = id_of(&upload);
    let unknown = stack.get(&andi, "/api/uploads/up-does-not-exist").await;
    assert_eq!(unknown.status, StatusCode::NOT_FOUND);
    assert_eq!(unknown.json["error"], "Upload not found.");

    for (http_method, suffix, payload) in [
        ("GET", "", Payload::None),
        ("GET", "/preview", Payload::None),
        ("POST", "/ingest", Payload::Json(ingest_body("stock_raw"))),
        ("DELETE", "", Payload::None),
    ] {
        let reply = stack
            .send(
                &andi,
                http_method,
                &format!("/api/uploads/{id}{suffix}"),
                payload,
            )
            .await;
        assert_eq!(
            reply.status,
            StatusCode::NOT_FOUND,
            "{http_method} {suffix}: {}",
            reply.text
        );
        assert_eq!(
            reply.json, unknown.json,
            "{http_method} {suffix}: no oracle for which ids exist"
        );
    }

    assert_eq!(stack.status_of(&id).await.0, "uploaded", "untouched");
    assert_eq!(stack.bucket.keys().len(), 1, "the object was not deleted");
    assert!(
        stack.dagster.received_requests().await.unwrap().is_empty(),
        "nothing launched"
    );
    assert!(stack.ch.received_requests().await.unwrap().is_empty());

    assert_eq!(
        stack.get(&bayu, &format!("/api/uploads/{id}")).await.status,
        StatusCode::OK
    );
}

/// The per-id rule is membership of the upload's tenant, as for connectors,
/// so a caller in two tenants reaches an upload of either by id; the LIST is
/// the active tenant's.
#[tokio::test]
async fn a_caller_in_two_tenants_reaches_an_upload_of_either_by_id() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    let upload = stack.upload(&bayu, "a.csv", b"id,name\n1,a\n").await;
    let in_logistics = stack.caller("bayu@meridian.example", Some(LOGISTICS)).await;

    let by_id = stack
        .get(&in_logistics, &format!("/api/uploads/{}", id_of(&upload)))
        .await;
    assert_eq!(by_id.status, StatusCode::OK, "{}", by_id.text);
    assert_eq!(
        stack.get(&in_logistics, "/api/uploads").await.json,
        json!([])
    );
}

#[tokio::test]
async fn a_database_failure_is_the_fixed_text_and_names_no_table() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    sqlx::query("ALTER TABLE file_upload RENAME TO file_upload_gone")
        .execute(&stack.app.pool)
        .await
        .unwrap();

    let list = stack.get(&bayu, "/api/uploads").await;

    assert_eq!(
        list.status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "{}",
        list.text
    );
    assert_eq!(list.json["error"], "database error");
    assert!(
        !list.text.contains("file_upload") && !list.text.contains("relation"),
        "{}",
        list.text
    );
}

// ════════════════════════════════════════════════════════════════════════
// GET /api/uploads/{id}/preview
// ════════════════════════════════════════════════════════════════════════

/// The file that motivated the feature: a UTF-16 tab-separated export called
/// `.xls`, six lines of report above its header.
#[tokio::test]
async fn the_preview_reports_what_was_detected_and_what_is_used_and_applies_overrides() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    let upload = stack
        .upload(&bayu, "SAP stock.xls", &fixture("sap_report_utf16.xls"))
        .await;
    let uri = format!("/api/uploads/{}/preview", id_of(&upload));

    let plain = stack.get(&bayu, &uri).await;
    assert_eq!(plain.status, StatusCode::OK, "{}", plain.text);
    let reading = json!({ "encoding": "utf-16", "delimiter": "\t", "headerRow": 4 });
    assert_eq!(plain.json["detected"], reading);
    assert_eq!(plain.json["using"], reading);
    assert_eq!(
        plain.json["columns"],
        json!(["Plnt", "Material", "Description", "Qty"])
    );
    assert_eq!(
        plain.json["rows"],
        json!([
            ["8250", "0000100", "ADHESIVE", "10"],
            ["8250", "0000200", "PP BAND", "4"]
        ])
    );
    assert_eq!(plain.json["truncated"], false);

    let overridden = stack
        .get(
            &bayu,
            &format!("{uri}?encoding=utf-16&delimiter=%3B&headerRow=2"),
        )
        .await;
    assert_eq!(overridden.status, StatusCode::OK, "{}", overridden.text);
    assert_eq!(
        overridden.json["using"],
        json!({ "encoding": "utf-16", "delimiter": ";", "headerRow": 2 })
    );
    assert_eq!(
        overridden.json["detected"]["encoding"], "utf-16",
        "what was detected is reported beside what is used"
    );
    assert_eq!(
        overridden.json["columns"],
        json!(["Generated 24.09.2025"]),
        "one column under the wrong delimiter"
    );

    let quoted = stack
        .upload(&bayu, "q.csv", &fixture("quoted_comma_newline.csv"))
        .await;
    let quoted_preview = stack
        .get(&bayu, &format!("/api/uploads/{}/preview", id_of(&quoted)))
        .await;
    assert_eq!(
        quoted_preview.json["columns"],
        json!(["id", "note", "amount"])
    );
    assert_eq!(
        quoted_preview.json["rows"][1],
        json!(["2", "First line\nsecond line", "4"])
    );
}

/// A preview reads one range of the object, so the cost does not grow with
/// the file, and says that the file goes on.
#[tokio::test]
async fn the_preview_of_a_large_file_reads_one_range_and_says_it_is_truncated() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    let bytes = csv_of_size(600 * 1024);
    let upload = stack.upload(&bayu, "large.csv", &bytes).await;

    let preview = stack
        .get(&bayu, &format!("/api/uploads/{}/preview", id_of(&upload)))
        .await;

    assert_eq!(preview.status, StatusCode::OK, "{}", preview.text);
    assert_eq!(preview.json["truncated"], true);
    assert_eq!(preview.json["columns"], json!(["id", "name"]));
    assert_eq!(preview.json["rows"].as_array().unwrap().len(), 20);
    assert_eq!(preview.json["rows"][0], json!(["0", "name_0"]));
    let ranges: Vec<String> = stack
        .s3
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.method.as_str() == "GET")
        .map(|r| {
            r.headers
                .get("range")
                .map_or_else(String::new, |v| v.to_str().unwrap().to_owned())
        })
        .collect();
    assert_eq!(
        ranges,
        ["bytes=0-262143", "bytes=0-614403"],
        "the first 256 KiB for the preview, then a second ranged read, bounded at 16 MiB, for the guess of the column types (DATA-18 decision 1)"
    );
}

#[tokio::test]
async fn a_preview_override_that_is_given_must_be_valid_and_costs_no_storage_read() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    let upload = stack.upload(&bayu, "a.csv", b"a,b\n1,2\n").await;
    let uri = format!("/api/uploads/{}/preview", id_of(&upload));
    let reads_before = stack.s3.received_requests().await.unwrap().len();

    for (query, sentence) in [
        ("encoding=latin-1", "encoding must be utf-8 or utf-16."),
        ("encoding=", "encoding must be utf-8 or utf-16."),
        (
            "delimiter=%3B%3B",
            "delimiter must be a comma, a semicolon, a tab or a pipe.",
        ),
        (
            "delimiter=x",
            "delimiter must be a comma, a semicolon, a tab or a pipe.",
        ),
        (
            "headerRow=-1",
            "headerRow must be a whole number, 0 or more.",
        ),
        (
            "headerRow=abc",
            "headerRow must be a whole number, 0 or more.",
        ),
        ("headerRow=", "headerRow must be a whole number, 0 or more."),
    ] {
        let reply = stack.get(&bayu, &format!("{uri}?{query}")).await;
        assert_eq!(
            reply.status,
            StatusCode::BAD_REQUEST,
            "{query}: {}",
            reply.text
        );
        assert_eq!(reply.json["error"], sentence, "{query}");
    }
    assert_eq!(
        stack.s3.received_requests().await.unwrap().len(),
        reads_before
    );
}

// ════════════════════════════════════════════════════════════════════════
// POST /api/uploads/{id}/ingest
// ════════════════════════════════════════════════════════════════════════

/// The job is launched with what the user confirmed, under the plan's run
/// config, and the row is claimed before and given its run after.
#[tokio::test]
async fn an_ingest_launches_the_job_with_the_confirmed_options_and_records_the_run() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    mount_free_table(&stack.ch).await;
    mount_launch(&stack.dagster, "run-1").await;
    let upload = stack
        .upload(&bayu, "SAP.xls", &fixture("sap_report_utf16.xls"))
        .await;
    let id = id_of(&upload);

    let reply = stack
        .ingest(
            &bayu,
            &id,
            json!({ "bronzeTable": "sap_stock", "mode": "append", "encoding": "utf-16", "delimiter": "\t", "headerRow": 4 }),
        )
        .await;

    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text);
    assert_eq!(reply.json["runId"], "run-1");
    let loading = &reply.json["upload"];
    assert_eq!(loading["id"], id.as_str());
    assert_eq!(loading["status"], "ingesting");
    assert_eq!(loading["bronzeTable"], "sap_stock");
    assert_eq!(loading["loadMode"], "append");
    assert_eq!(loading["runId"], "run-1");
    assert_eq!(
        loading["parseOptions"],
        json!({ "encoding": "utf-16", "delimiter": "\t", "headerRow": 4 })
    );
    assert!(
        loading.get("rows").is_none()
            && loading.get("error").is_none()
            && loading.get("assetId").is_none()
    );

    let launches = requests_containing(&stack.dagster, "launchRun").await;
    assert_eq!(launches.len(), 1);
    assert_eq!(
        launches[0]["variables"]["sel"]["pipelineName"],
        "file_ingest_job"
    );
    assert_eq!(
        launches[0]["variables"]["cfg"],
        json!({ "ops": { "ingest_uploaded_file": { "config": {
            "upload_id": id,
            "storage_keys": [format!("uploads/{GROUP}/{id}.xls")],
            "format": "delimited",
            "compression": "none",
            "max_bytes": UNPACKED_CAP,
            "bronze_table_name": "sap_stock",
            "load_mode": "append",
            "encoding": "utf-16",
            "delimiter": "\t",
            "header_row": 4,
        "columns": [],
        "on_bad_value": "stop",
        } } } })
    );
    let row = stack.status_of(&id).await;
    assert_eq!(
        (row.0.as_str(), row.1.as_deref(), row.2),
        ("ingesting", Some("run-1"), None)
    );
    assert_eq!(
        stack.claims().await,
        [(
            "sap_stock".to_owned(),
            Some(GROUP.parse::<Uuid>().unwrap()),
            id.clone()
        )],
        "the name was claimed for the caller's tenant, by this upload (finding B4)"
    );

    let audit = stack.audit_actions().await;
    let ingest = audit
        .iter()
        .find(|a| a.0 == "upload.ingest")
        .expect("an upload.ingest event");
    assert_eq!(ingest.1.as_deref(), Some("upload"));
    assert_eq!(ingest.2.as_deref(), Some(id.as_str()));
    assert_eq!(
        ingest.3,
        json!({ "fileName": "SAP.xls", "sizeBytes": upload["sizeBytes"], "table": "sap_stock", "mode": "append" })
    );
}

#[tokio::test]
async fn the_mode_defaults_to_replace() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    mount_free_table(&stack.ch).await;
    mount_launch(&stack.dagster, "run-1").await;
    let upload = stack.upload(&bayu, "a.csv", b"a,b\n1,2\n").await;

    let reply = stack
        .ingest(&bayu, &id_of(&upload), ingest_body("stock_raw"))
        .await;

    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text);
    assert_eq!(reply.json["upload"]["loadMode"], "replace");
    let launches = requests_containing(&stack.dagster, "launchRun").await;
    assert_eq!(
        launches[0]["variables"]["cfg"]["ops"]["ingest_uploaded_file"]["config"]["load_mode"],
        "replace"
    );
}

/// `SEC-17`: a CSV whose header has `columns` cells, and one row.
fn csv_of_width(columns: usize) -> Vec<u8> {
    let names: Vec<String> = (0..columns).map(|n| format!("c{n}")).collect();
    format!("{}\n1\n", names.join(",")).into_bytes()
}

const TOO_MANY_COLUMNS: &str = "The file has more than 1,000 columns.";

/// `SEC-17`: 1,000 columns is the largest width that previews; one more is a
/// 400 with the fixed sentence, and the same file read under another
/// delimiter previews again.
#[tokio::test]
async fn a_preview_over_1000_columns_is_400_and_a_preview_of_1000_is_not() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    let fits = stack.upload(&bayu, "fits.csv", &csv_of_width(1_000)).await;
    let wide = stack.upload(&bayu, "wide.csv", &csv_of_width(1_001)).await;

    let ok = stack
        .get(&bayu, &format!("/api/uploads/{}/preview", id_of(&fits)))
        .await;
    assert_eq!(ok.status, StatusCode::OK, "{}", ok.text);
    assert_eq!(ok.json["columns"].as_array().unwrap().len(), 1_000);

    let uri = format!("/api/uploads/{}/preview", id_of(&wide));
    let refused = stack.get(&bayu, &uri).await;
    assert_eq!(refused.status, StatusCode::BAD_REQUEST, "{}", refused.text);
    assert_eq!(refused.json["error"], TOO_MANY_COLUMNS);

    // A wrong delimiter must not lock a good file out: under a semicolon the
    // same bytes are one column.
    let again = stack.get(&bayu, &format!("{uri}?delimiter=%3B")).await;
    assert_eq!(again.status, StatusCode::OK, "{}", again.text);
    assert_eq!(again.json["columns"].as_array().unwrap().len(), 1);
}

/// `SEC-17`: a header of nothing but commas, longer than the 256 KiB the
/// preview reads, is refused (the cut record is counted, not dropped).
#[tokio::test]
async fn a_header_of_commas_longer_than_the_preview_is_refused() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    let upload = stack
        .upload(&bayu, "commas.csv", &vec![b','; 300 * 1024])
        .await;

    let reply = stack
        .get(&bayu, &format!("/api/uploads/{}/preview", id_of(&upload)))
        .await;

    assert_eq!(reply.status, StatusCode::BAD_REQUEST, "{}", reply.text);
    assert_eq!(reply.json["error"], TOO_MANY_COLUMNS);
}

/// `SEC-17`: an ingest over the cap is a 400 before any claim, mark or
/// launch: no table claim, the row still `uploaded`, no run launched.
#[tokio::test]
async fn an_ingest_over_1000_columns_is_400_and_claims_marks_and_launches_nothing() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    mount_free_table(&stack.ch).await;
    mount_launch(&stack.dagster, "run-1").await;
    let upload = stack.upload(&bayu, "wide.csv", &csv_of_width(1_001)).await;
    let id = id_of(&upload);

    let reply = stack.ingest(&bayu, &id, ingest_body("wide_raw")).await;

    assert_eq!(reply.status, StatusCode::BAD_REQUEST, "{}", reply.text);
    assert_eq!(reply.json["error"], TOO_MANY_COLUMNS);
    assert!(stack.claims().await.is_empty(), "no table was claimed");
    let row = stack.status_of(&id).await;
    assert_eq!((row.0.as_str(), row.1, row.2), ("uploaded", None, None));
    assert_eq!(
        count_requests_containing(&stack.dagster, "launchRun").await,
        0,
        "no run was launched"
    );
}

/// `SEC-17`: the cap is not a reason to refuse the largest file allowed.
#[tokio::test]
async fn an_ingest_of_1000_columns_is_launched() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    mount_free_table(&stack.ch).await;
    mount_launch(&stack.dagster, "run-1").await;
    let upload = stack.upload(&bayu, "fits.csv", &csv_of_width(1_000)).await;

    let reply = stack
        .ingest(&bayu, &id_of(&upload), ingest_body("fits_raw"))
        .await;

    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text);
    assert_eq!(reply.json["runId"], "run-1");
}

const TOO_MANY_UPLOADS: &str = "Too many uploads are in progress. Try again in a moment.";

/// A `PUT` that takes a second, so requests sent together overlap.
struct SlowPut(Bucket);
impl Respond for SlowPut {
    fn respond(&self, request: &MockRequest) -> ResponseTemplate {
        PutObject(self.0.clone())
            .respond(request)
            .set_delay(Duration::from_millis(1_500))
    }
}

/// `SEC-17`: five files sent by one user at the same moment, four are
/// accepted and one is a 429 with `Retry-After`; nothing of the refused one
/// is stored. (Each accepted file is held in memory for the second its `PUT`
/// takes, which is what keeps the five overlapping.)
#[tokio::test]
async fn the_fifth_simultaneous_upload_of_one_user_is_429_with_retry_after() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    Mock::given(method("PUT"))
        .and(path_regex(OBJECT_PATH))
        .respond_with(SlowPut(stack.bucket.clone()))
        .with_priority(1)
        .mount(&stack.s3)
        .await;
    let send = |name: &'static str| {
        stack.send(
            &bayu,
            "POST",
            "/api/uploads",
            file_form(name, "text/csv", format!("a,b\n{name},2\n").as_bytes()),
        )
    };

    let replies = tokio::join!(
        send("one.csv"),
        send("two.csv"),
        send("three.csv"),
        send("four.csv"),
        send("five.csv"),
    );
    let replies = [replies.0, replies.1, replies.2, replies.3, replies.4];

    let accepted = replies
        .iter()
        .filter(|r| r.status == StatusCode::CREATED)
        .count();
    let refused: Vec<&Reply> = replies
        .iter()
        .filter(|r| r.status == StatusCode::TOO_MANY_REQUESTS)
        .collect();
    assert_eq!(
        accepted,
        4,
        "{:?}",
        replies.iter().map(|r| &r.text).collect::<Vec<_>>()
    );
    assert_eq!(refused.len(), 1);
    assert_eq!(refused[0].json["error"], TOO_MANY_UPLOADS);
    assert_eq!(refused[0].retry_after.as_deref(), Some("5"));
    assert_eq!(
        stack.bucket.keys().len(),
        4,
        "the refused file stored nothing"
    );

    // The places were given back: a sixth sent afterwards is accepted.
    let again = stack.upload(&bayu, "six.csv", b"a,b\n6,2\n").await;
    assert!(again["id"].is_string());
}

/// `SEC-17`: with four loads of the uploader running, a fifth ingest is a 429
/// with `Retry-After`, the upload stays `uploaded`, no table is claimed and
/// nothing is launched.
#[tokio::test]
async fn a_fifth_load_of_one_uploader_is_429_and_claims_and_launches_nothing() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    mount_free_table(&stack.ch).await;
    mount_launch(&stack.dagster, "run-1").await;
    for n in 0..4 {
        stack
            .seed_loading(
                GROUP,
                &format!("up-busy-{n}"),
                &format!("busy_{n}"),
                None,
                "10 seconds",
            )
            .await;
    }
    stack.seed(GROUP, "up-fifth", b"id,name\n1,a\n").await;
    let claims_before = stack.claims().await.len();

    let reply = stack
        .ingest(&bayu, "up-fifth", ingest_body("fifth_raw"))
        .await;

    assert_eq!(
        reply.status,
        StatusCode::TOO_MANY_REQUESTS,
        "{}",
        reply.text
    );
    assert_eq!(reply.json["error"], TOO_MANY_UPLOADS);
    assert_eq!(reply.retry_after.as_deref(), Some("5"));
    let row = stack.status_of("up-fifth").await;
    assert_eq!((row.0.as_str(), row.1, row.2), ("uploaded", None, None));
    assert_eq!(stack.claims().await.len(), claims_before, "no table claim");
    assert_eq!(
        count_requests_containing(&stack.dagster, "launchRun").await,
        0
    );
}

/// `SEC-17`: the number is per uploader (the person the upload records), so
/// loads of another person do not use it up.
#[tokio::test]
async fn loads_of_another_uploader_do_not_count_against_this_one() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    mount_free_table(&stack.ch).await;
    mount_launch(&stack.dagster, "run-1").await;
    for n in 0..4 {
        let id = format!("up-other-{n}");
        stack
            .seed_loading(GROUP, &id, &format!("other_{n}"), None, "10 seconds")
            .await;
        sqlx::query("UPDATE file_upload SET uploaded_by = 'Someone Else' WHERE id = $1")
            .bind(&id)
            .execute(&stack.app.pool)
            .await
            .unwrap();
    }
    stack.seed(GROUP, "up-mine", b"id,name\n1,a\n").await;

    let reply = stack
        .ingest(&bayu, "up-mine", ingest_body("mine_raw"))
        .await;

    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text);
}

/// Review finding B2: the row is claimed before the launch, so two requests
/// sent at the same moment cannot both launch.
///
/// Both requests must be past the "is it loading?" check before either claims,
/// or the early check would refuse the second and the claim would never be
/// tested. The registry lookup in between is slowed down for that: it is the
/// call both make after the early checks and before the claim.
#[tokio::test]
async fn two_simultaneous_ingests_of_one_upload_launch_once() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    Mock::given(method("POST"))
        .and(body_string_contains("dataset_catalog"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(Duration::from_millis(500))
                .set_body_json(rows_body(&json!([]))),
        )
        .mount(&stack.ch)
        .await;
    Mock::given(method("POST"))
        .and(body_string_contains("DESCRIBE TABLE"))
        .respond_with(ResponseTemplate::new(404).set_body_string("Code: 60. (UNKNOWN_TABLE)"))
        .mount(&stack.ch)
        .await;
    mount_launch(&stack.dagster, "run-1").await;
    let upload = stack.upload(&bayu, "a.csv", b"a,b\n1,2\n").await;
    let id = id_of(&upload);

    let (first, second) = tokio::join!(
        stack.ingest(&bayu, &id, ingest_body("stock_raw")),
        stack.ingest(&bayu, &id, ingest_body("stock_raw")),
    );

    let mut statuses = [first.status, second.status];
    statuses.sort();
    assert_eq!(
        statuses,
        [StatusCode::OK, StatusCode::CONFLICT],
        "{} / {}",
        first.text,
        second.text
    );
    let loser = if first.status == StatusCode::CONFLICT {
        &first
    } else {
        &second
    };
    assert_eq!(loser.json["error"], "This upload is already being loaded.");
    assert_eq!(
        count_requests_containing(&stack.ch, "dataset_catalog").await,
        2,
        "both requests were past the early checks when the claim was made"
    );
    assert_eq!(
        requests_containing(&stack.dagster, "launchRun").await.len(),
        1,
        "launched once"
    );
}

#[tokio::test]
async fn a_second_ingest_while_the_first_is_loading_is_409_and_so_is_a_delete() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    mount_free_table(&stack.ch).await;
    mount_launch(&stack.dagster, "run-1").await;
    mount_run_status(&stack.dagster, "STARTED").await;
    let upload = stack.upload(&bayu, "a.csv", b"a,b\n1,2\n").await;
    let id = id_of(&upload);
    assert_eq!(
        stack
            .ingest(&bayu, &id, ingest_body("stock_raw"))
            .await
            .status,
        StatusCode::OK
    );

    let again = stack.ingest(&bayu, &id, ingest_body("other_raw")).await;
    assert_eq!(again.status, StatusCode::CONFLICT, "{}", again.text);
    assert_eq!(again.json["error"], "This upload is already being loaded.");

    let delete = stack
        .send(
            &bayu,
            "DELETE",
            &format!("/api/uploads/{id}"),
            Payload::None,
        )
        .await;
    assert_eq!(delete.status, StatusCode::CONFLICT, "{}", delete.text);
    assert_eq!(
        delete.json["error"],
        "This upload is being loaded, so it cannot be deleted yet."
    );

    assert_eq!(
        requests_containing(&stack.dagster, "launchRun").await.len(),
        1
    );
    let row = stack.status_of(&id).await;
    assert_eq!(
        (row.0.as_str(), row.1.as_deref()),
        ("ingesting", Some("run-1")),
        "the first load is untouched"
    );
    let table: Option<String> =
        sqlx::query_scalar("SELECT bronze_table FROM file_upload WHERE id = $1")
            .bind(&id)
            .fetch_one(&stack.app.pool)
            .await
            .unwrap();
    assert_eq!(table.as_deref(), Some("stock_raw"));
    assert_eq!(
        stack.bucket.keys().len(),
        1,
        "a refused delete removes nothing"
    );
}

/// All five fields are validated, each with its own sentence, before anything
/// is asked of the catalog or the orchestrator.
#[tokio::test]
async fn an_invalid_ingest_body_is_400_and_asks_nobody() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    let upload = stack.upload(&bayu, "a.csv", b"a,b\n1,2\n").await;
    let id = id_of(&upload);
    // Review finding C1: the sentence of the rule a person can follow, and
    // the names the plan says must be refused with it.
    let table_rule = "Table names start with a lower-case letter and use lower-case letters and digits joined by single underscores, with at most 128 characters.";
    let with = |field: &str, value: Value| {
        let mut body = ingest_body("stock_raw");
        body[field] = value;
        body
    };

    for (body, sentence) in [
        (with("bronzeTable", json!("Orders 2025")), table_rule),
        (with("bronzeTable", json!("Orders")), table_rule),
        (with("bronzeTable", json!("2025_orders")), table_rule),
        (with("bronzeTable", json!("")), table_rule),
        (with("bronzeTable", json!("x_")), table_rule),
        (with("bronzeTable", json!("_x")), table_rule),
        (with("bronzeTable", json!("a__b")), table_rule),
        (with("bronzeTable", json!("1a")), table_rule),
        (with("bronzeTable", json!("a".repeat(129))), table_rule),
        (
            with("mode", json!("merge")),
            "mode must be replace or append.",
        ),
        (
            with("encoding", json!("latin-1")),
            "encoding must be utf-8 or utf-16.",
        ),
        (
            with("delimiter", json!("ab")),
            "delimiter must be a comma, a semicolon, a tab or a pipe.",
        ),
        (
            with("delimiter", json!("")),
            "delimiter must be a comma, a semicolon, a tab or a pipe.",
        ),
        (
            with("headerRow", json!(-1)),
            "headerRow must be a whole number, 0 or more.",
        ),
        (
            with("headerRow", json!("0")),
            "headerRow must be a whole number, 0 or more.",
        ),
        (
            json!({ "encoding": "utf-8", "delimiter": ",", "headerRow": 0 }),
            "bronzeTable is required.",
        ),
    ] {
        let reply = stack.ingest(&bayu, &id, body.clone()).await;
        assert_eq!(
            reply.status,
            StatusCode::BAD_REQUEST,
            "{body}: {}",
            reply.text
        );
        assert_eq!(reply.json["error"], sentence, "{body}");
    }
    let not_json = stack
        .send(
            &bayu,
            "POST",
            &format!("/api/uploads/{id}/ingest"),
            Payload::Raw("application/json", b"{nope".to_vec()),
        )
        .await;
    assert_eq!(not_json.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        not_json.json["error"],
        "The request body must be a JSON object."
    );

    assert!(stack.dagster.received_requests().await.unwrap().is_empty());
    assert!(stack.ch.received_requests().await.unwrap().is_empty());
    assert_eq!(
        stack.status_of(&id).await.0,
        "uploaded",
        "no claim was made"
    );
    assert!(stack.claims().await.is_empty(), "no table was claimed");
}

#[tokio::test]
async fn a_table_a_connector_loads_is_409_and_nothing_is_claimed() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    sqlx::query(
        "UPDATE connector SET source_objects = '[{\"name\":\"public.orders\",\"target\":\"orders_raw\"}]'::jsonb \
         WHERE id = 'conn-pg-lakehouse'",
    )
    .execute(&stack.app.pool)
    .await
    .unwrap();
    let upload = stack.upload(&bayu, "a.csv", b"a,b\n1,2\n").await;

    let reply = stack
        .ingest(&bayu, &id_of(&upload), ingest_body("orders_raw"))
        .await;

    assert_eq!(reply.status, StatusCode::CONFLICT, "{}", reply.text);
    assert_eq!(
        reply.json["error"],
        "A connector loads that table, so a file cannot be loaded into it."
    );
    assert_eq!(stack.status_of(&id_of(&upload)).await.0, "uploaded");
    assert!(stack.dagster.received_requests().await.unwrap().is_empty());
    assert!(stack.claims().await.is_empty(), "no table was claimed");
}

/// A connector's table is refused before the claim table is asked: the
/// connector's sentence even for a name this tenant's own upload holds (a
/// spec saved before the name was claimed is the way to such a state).
#[tokio::test]
async fn a_connector_that_loads_the_table_is_refused_even_when_the_tenant_holds_the_claim() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    stack.seed_ingested(GROUP, "up-first", "orders_raw").await;
    sqlx::query(
        "UPDATE connector SET source_objects = '[{\"name\":\"public.orders\",\"target\":\"orders_raw\"}]'::jsonb \
         WHERE id = 'conn-pg-lakehouse'",
    )
    .execute(&stack.app.pool)
    .await
    .unwrap();
    let upload = stack.upload(&bayu, "a.csv", b"a,b\n1,2\n").await;

    let reply = stack
        .ingest(&bayu, &id_of(&upload), ingest_body("orders_raw"))
        .await;

    assert_eq!(reply.status, StatusCode::CONFLICT, "{}", reply.text);
    assert_eq!(
        reply.json["error"],
        "A connector loads that table, so a file cannot be loaded into it."
    );
    assert_eq!(stack.status_of(&id_of(&upload)).await.0, "uploaded");
    assert!(stack.dagster.received_requests().await.unwrap().is_empty());
}

/// `SEC-16` regression: a connector of ANOTHER tenant that loads the table
/// closes the name, but the answer is the one sentence every other reason
/// gets, not "A connector loads that table", which told a person that another
/// organisation has a connector writing a table of that name. It fails before
/// the fix, which answered with the connector's sentence.
#[tokio::test]
async fn another_tenants_connector_closes_the_table_with_the_one_sentence() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    sqlx::query(
        "INSERT INTO connector (id, tenant_id, \
         name, type, direction, host, secret_ref, environment, tenant, \
         adapter, ingest_mode, dial, source_objects) VALUES \
         ('conn-retail-orders', '11111111-1111-4111-8111-000000000002', \
         'retail orders', 'PostgreSQL', 'source', 'unused', 'env:CONNECTOR_PG_PASSWORD', \
         'production', 'meridian', 'sql', 'batch', \
         '{\"driver\":\"postgres\",\"host\":\"127.0.0.1\",\"port\":5432,\"database\":\"d\",\"user\":\"u\"}'::jsonb, \
         '[{\"name\":\"public.orders\",\"target\":\"retail_orders_raw\"}]'::jsonb)",
    )
    .execute(&stack.app.pool)
    .await
    .unwrap();
    let upload = stack.upload(&bayu, "a.csv", b"a,b\n1,2\n").await;

    let reply = stack
        .ingest(&bayu, &id_of(&upload), ingest_body("retail_orders_raw"))
        .await;

    assert_eq!(reply.status, StatusCode::CONFLICT, "{}", reply.text);
    assert_eq!(reply.json["error"], NOT_FREE);
    assert!(
        !reply.text.to_lowercase().contains("connector"),
        "the answer must not mention a connector: {}",
        reply.text
    );
    assert_eq!(stack.status_of(&id_of(&upload)).await.0, "uploaded");
    assert!(stack.dagster.received_requests().await.unwrap().is_empty());
    assert!(stack.claims().await.is_empty(), "no table was claimed");
}

/// The one sentence for a table name that is not this tenant's to load into
/// (review finding B4): another tenant's claim, and an existing table nobody
/// claimed, are told apart by nothing a caller can see.
const NOT_FREE: &str = "That table name cannot be used. Choose another name.";

/// An existing table is only loaded into again by the tenant that holds its
/// claim. Registered in the catalog and claimed by nobody: refused, and no
/// claim is made for the refused name.
#[tokio::test]
async fn a_registered_table_that_no_upload_of_the_tenant_claimed_is_409() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    mount_registered(&stack.ch).await;
    let upload = stack.upload(&bayu, "a.csv", b"a,b\n1,2\n").await;

    let reply = stack
        .ingest(&bayu, &id_of(&upload), ingest_body("stock_raw"))
        .await;

    assert_eq!(reply.status, StatusCode::CONFLICT, "{}", reply.text);
    assert_eq!(reply.json["error"], NOT_FREE);
    assert_eq!(
        stack.status_of(&id_of(&upload)).await.0,
        "uploaded",
        "nothing was claimed"
    );
    assert!(stack.claims().await.is_empty(), "no table was claimed");
    assert!(stack.dagster.received_requests().await.unwrap().is_empty());
}

/// Not registered, but there as an Iceberg table: refused too.
#[tokio::test]
async fn an_iceberg_table_that_is_not_registered_and_not_claimed_is_409() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    Mock::given(method("POST"))
        .and(body_string_contains("dataset_catalog"))
        .respond_with(ResponseTemplate::new(200).set_body_json(rows_body(&json!([]))))
        .mount(&stack.ch)
        .await;
    Mock::given(method("POST"))
        .and(body_string_contains(
            "DESCRIBE TABLE icecat_api.`bronze.stock_raw`",
        ))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(rows_body(&json!([{ "name": "id", "type": "String" }]))),
        )
        .mount(&stack.ch)
        .await;
    let upload = stack.upload(&bayu, "a.csv", b"a,b\n1,2\n").await;

    let reply = stack
        .ingest(&bayu, &id_of(&upload), ingest_body("stock_raw"))
        .await;

    assert_eq!(reply.status, StatusCode::CONFLICT, "{}", reply.text);
    assert_eq!(reply.json["error"], NOT_FREE);
    assert!(stack.dagster.received_requests().await.unwrap().is_empty());
    assert!(stack.claims().await.is_empty(), "no table was claimed");
}

/// Review finding B4: another tenant's claim keeps this tenant out, and the
/// answer is the same sentence whether the table exists or not, so a caller
/// cannot tell a name another tenant holds from one that is merely taken. The
/// claim is read first: the registry and the Iceberg query database are not
/// asked at all.
#[tokio::test]
async fn another_tenants_claim_closes_a_table_whether_or_not_it_exists() {
    for (what, exists) in [
        ("a table that exists", true),
        ("a name not yet used", false),
    ] {
        let stack = Stack::start().await;
        let bayu = stack.bayu().await;
        if exists {
            mount_registered(&stack.ch).await;
        } else {
            mount_free_table(&stack.ch).await;
        }
        stack.seed_ingested(RETAIL, "up-retail", "stock_raw").await;
        let upload = stack.upload(&bayu, "a.csv", b"a,b\n1,2\n").await;

        let reply = stack
            .ingest(&bayu, &id_of(&upload), ingest_body("stock_raw"))
            .await;

        assert_eq!(reply.status, StatusCode::CONFLICT, "{what}: {}", reply.text);
        assert_eq!(reply.json["error"], NOT_FREE, "{what}");
        assert_eq!(
            stack.status_of(&id_of(&upload)).await.0,
            "uploaded",
            "{what}"
        );
        assert!(
            stack.ch.received_requests().await.unwrap().is_empty(),
            "{what}: the claim answers before the table is looked up"
        );
        assert!(stack.dagster.received_requests().await.unwrap().is_empty());
        assert_eq!(
            stack.claims().await,
            [(
                "stock_raw".to_owned(),
                Some(RETAIL.parse::<Uuid>().unwrap()),
                "up-retail".to_owned()
            )],
            "{what}: the claim is still the other tenant's alone"
        );
    }
}

/// A claim whose tenant is gone belongs to nobody, and is closed to every
/// tenant, with the same sentence (a claim's tenant is `ON DELETE SET NULL`).
#[tokio::test]
async fn a_claim_whose_tenant_is_gone_is_closed_to_every_tenant() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    mount_free_table(&stack.ch).await;
    stack.claim(RETAIL, "stock_raw", "up-gone").await;
    sqlx::query("UPDATE upload_table_claim SET tenant_id = NULL WHERE bronze_table = 'stock_raw'")
        .execute(&stack.app.pool)
        .await
        .unwrap();
    let upload = stack.upload(&bayu, "a.csv", b"a,b\n1,2\n").await;

    let reply = stack
        .ingest(&bayu, &id_of(&upload), ingest_body("stock_raw"))
        .await;

    assert_eq!(reply.status, StatusCode::CONFLICT, "{}", reply.text);
    assert_eq!(reply.json["error"], NOT_FREE);
    assert_eq!(stack.status_of(&id_of(&upload)).await.0, "uploaded");
    assert_eq!(
        stack.claims().await,
        [("stock_raw".to_owned(), None, "up-gone".to_owned())]
    );
}

/// Review findings B1 and B4, end to end: a table the tenant's own upload
/// claimed can be loaded into again, replacing or adding, even though it
/// exists, and even after the upload that made it was deleted. The row is
/// really gone; the claim is what remains.
#[tokio::test]
async fn after_a_delete_the_claim_remains_and_a_new_upload_of_the_tenant_loads_into_the_table() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    mount_registered(&stack.ch).await;
    mount_launch(&stack.dagster, "run-2").await;
    stack.seed_ingested(GROUP, "up-first", "stock_raw").await;
    let second = stack.upload(&bayu, "again.csv", b"id,name\n1,a\n").await;

    let delete = stack
        .send(&bayu, "DELETE", "/api/uploads/up-first", Payload::None)
        .await;
    assert_eq!(delete.status, StatusCode::NO_CONTENT, "{}", delete.text);
    let first_rows: i64 =
        sqlx::query_scalar("SELECT count(*) FROM file_upload WHERE id = 'up-first'")
            .fetch_one(&stack.app.pool)
            .await
            .unwrap();
    assert_eq!(first_rows, 0, "a delete is a real delete");
    let append = stack
        .ingest(
            &bayu,
            &id_of(&second),
            json!({ "bronzeTable": "stock_raw", "mode": "append", "encoding": "utf-8", "delimiter": ",", "headerRow": 0 }),
        )
        .await;

    assert_eq!(append.status, StatusCode::OK, "{}", append.text);
    assert_eq!(append.json["upload"]["loadMode"], "append");
    assert_eq!(
        requests_containing(&stack.dagster, "launchRun").await.len(),
        1
    );
    assert_eq!(
        stack.claims().await,
        [(
            "stock_raw".to_owned(),
            Some(GROUP.parse::<Uuid>().unwrap()),
            "up-first".to_owned()
        )],
        "still the claim the first upload made"
    );
}

/// A failed load that named a table still holds it: "Try again" must be able
/// to succeed.
#[tokio::test]
async fn a_failed_load_holds_its_table_so_try_again_can_succeed() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    mount_registered(&stack.ch).await;
    mount_launch(&stack.dagster, "run-2").await;
    stack
        .seed_loading(GROUP, "up-1", "stock_raw", Some("run-1"), "10 minutes")
        .await;
    uploads::mark_finished(
        &stack.app.pool,
        "up-1",
        Some("run-1"),
        Some("The load into the table failed."),
        None,
    )
    .await
    .unwrap()
    .unwrap();

    let again = stack.ingest(&bayu, "up-1", ingest_body("stock_raw")).await;

    assert_eq!(again.status, StatusCode::OK, "{}", again.text);
    assert_eq!(again.json["upload"]["status"], "ingesting");
    assert!(
        again.json["upload"].get("error").is_none(),
        "the last attempt's reason is cleared"
    );
}

/// Another upload of the same tenant is loading into the table: the tenant's
/// own claim lets it through the ownership check and this is what stops it. (An
/// upload of another tenant loading into it holds the claim, and gets the
/// ownership sentence first: `another_tenants_claim_closes_a_table_whether_or_not_it_exists`.)
#[tokio::test]
async fn another_upload_of_the_tenant_loading_into_the_table_is_409() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    mount_free_table(&stack.ch).await;
    stack
        .seed_loading(GROUP, "up-other", "stock_raw", Some("run-9"), "10 seconds")
        .await;
    let upload = stack.upload(&bayu, "a.csv", b"a,b\n1,2\n").await;

    let reply = stack
        .ingest(&bayu, &id_of(&upload), ingest_body("stock_raw"))
        .await;

    assert_eq!(reply.status, StatusCode::CONFLICT, "{}", reply.text);
    assert_eq!(
        reply.json["error"],
        "Another upload is loading into that table."
    );
    assert_eq!(stack.status_of(&id_of(&upload)).await.0, "uploaded");
}

/// Review finding B4, the failure the reviewer found, through the routes:
/// tenant B's load into `t_raw` fails before it writes anything; tenant A then
/// asks for `t_raw`, a table that does not exist, and is refused; B's retry goes
/// through and the name is still B's. With ownership read from upload rows, A
/// took the name and B's retry replaced A's rows.
#[tokio::test]
async fn a_failed_claim_keeps_another_tenant_out_of_the_table_and_lets_its_own_tenant_retry() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    let andi = stack.andi().await;
    mount_free_table(&stack.ch).await;
    mount_launch(&stack.dagster, "run-1").await;
    mount_run_status(&stack.dagster, "FAILURE").await;
    let b_upload = stack.upload(&andi, "b.csv", b"a,b\n1,2\n").await;
    let b_id = id_of(&b_upload);

    let first = stack.ingest(&andi, &b_id, ingest_body("t_raw")).await;
    assert_eq!(first.status, StatusCode::OK, "{}", first.text);

    // B's load fails before it writes: the job recorded its reason and ended.
    sqlx::query("UPDATE file_upload SET updated_at = now() - interval '1 hour' WHERE id = $1")
        .bind(&b_id)
        .execute(&stack.app.pool)
        .await
        .unwrap();
    mount_results(
        &stack.ch,
        &json!([result_row(
            &b_id,
            "failed",
            None,
            "The stored file could not be read.",
            5
        )]),
    )
    .await;
    let failed = stack.get(&andi, &format!("/api/uploads/{b_id}")).await;
    assert_eq!(failed.json["status"], "failed", "{}", failed.text);

    // A asks for the same name. Nothing was ever written to it, and it is
    // still not A's.
    let a_upload = stack.upload(&bayu, "a.csv", b"x,y\n3,4\n").await;
    let refused = stack
        .ingest(&bayu, &id_of(&a_upload), ingest_body("t_raw"))
        .await;
    assert_eq!(refused.status, StatusCode::CONFLICT, "{}", refused.text);
    assert_eq!(refused.json["error"], NOT_FREE);
    assert_eq!(stack.status_of(&id_of(&a_upload)).await.0, "uploaded");
    assert_eq!(
        requests_containing(&stack.dagster, "launchRun").await.len(),
        1,
        "A's refused request launched nothing"
    );

    // B retries: the claim is B's own.
    let retry = stack.ingest(&andi, &b_id, ingest_body("t_raw")).await;
    assert_eq!(retry.status, StatusCode::OK, "{}", retry.text);
    assert_eq!(retry.json["upload"]["status"], "ingesting");
    assert_eq!(
        requests_containing(&stack.dagster, "launchRun").await.len(),
        2
    );
    assert_eq!(
        stack.claims().await,
        [(
            "t_raw".to_owned(),
            Some(RETAIL.parse::<Uuid>().unwrap()),
            b_id.clone()
        )],
        "the name was B's throughout"
    );
}

/// Review finding B4: two tenants asking for one new name at the same moment,
/// one loads and the other is refused, and one run is launched. The registry
/// lookup both make after the ownership check is slowed down so that both are
/// past that check before either claims, as
/// `two_simultaneous_ingests_of_one_upload_launch_once` does.
///
/// What this pins is the outcome under a real race: one launch, one claim, and
/// the loser refused with a 409 and left untouched. Which sentence the loser
/// gets depends on how far the winner had got: the ownership sentence when it
/// loses `claim_table`, or "Another upload is loading into that table." when
/// the winner had already marked its upload by the time the loser checked.
/// `a_tenant_that_loses_the_name_between_the_check_and_the_claim_is_refused_and_launches_nothing`
/// is the one that fixes the first path.
#[tokio::test]
async fn two_tenants_asking_for_one_new_name_at_the_same_moment_one_launches() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    let andi = stack.andi().await;
    Mock::given(method("POST"))
        .and(body_string_contains("dataset_catalog"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(Duration::from_millis(500))
                .set_body_json(rows_body(&json!([]))),
        )
        .mount(&stack.ch)
        .await;
    Mock::given(method("POST"))
        .and(body_string_contains("DESCRIBE TABLE"))
        .respond_with(ResponseTemplate::new(404).set_body_string("Code: 60. (UNKNOWN_TABLE)"))
        .mount(&stack.ch)
        .await;
    mount_launch(&stack.dagster, "run-1").await;
    let a_id = id_of(&stack.upload(&bayu, "a.csv", b"a,b\n1,2\n").await);
    let b_id = id_of(&stack.upload(&andi, "b.csv", b"a,b\n3,4\n").await);

    let (a, b) = tokio::join!(
        stack.ingest(&bayu, &a_id, ingest_body("shared_raw")),
        stack.ingest(&andi, &b_id, ingest_body("shared_raw")),
    );

    let mut statuses = [a.status, b.status];
    statuses.sort();
    assert_eq!(
        statuses,
        [StatusCode::OK, StatusCode::CONFLICT],
        "{} / {}",
        a.text,
        b.text
    );
    let (loser, winner_tenant, winner_upload, loser_upload) = if a.status == StatusCode::OK {
        (&b, GROUP, &a_id, &b_id)
    } else {
        (&a, RETAIL, &b_id, &a_id)
    };
    let refusal = loser.json["error"].as_str().unwrap();
    assert!(
        refusal == NOT_FREE || refusal == "Another upload is loading into that table.",
        "{refusal}"
    );
    assert_eq!(
        count_requests_containing(&stack.ch, "dataset_catalog").await,
        2,
        "both requests were past the ownership check when the claim was made"
    );
    assert_eq!(
        requests_containing(&stack.dagster, "launchRun").await.len(),
        1,
        "launched once"
    );
    assert_eq!(
        stack.claims().await,
        [(
            "shared_raw".to_owned(),
            Some(winner_tenant.parse::<Uuid>().unwrap()),
            winner_upload.clone()
        )],
        "the name is the winner's alone"
    );
    assert_eq!(stack.status_of(loser_upload).await.0, "uploaded");
}

/// Review finding B4: the ownership check only reads, so a tenant can pass it
/// and lose the name before it claims. Here tenant A's request has read "nobody
/// holds `shared_raw`" and is waiting on the slowed registry lookup when tenant
/// B's claim lands. A's `claim_table` is then false: the same 409, A's upload
/// is never marked, nothing is launched, and A makes no claim.
#[tokio::test]
async fn a_tenant_that_loses_the_name_between_the_check_and_the_claim_is_refused_and_launches_nothing()
 {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    Mock::given(method("POST"))
        .and(body_string_contains("dataset_catalog"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(Duration::from_millis(500))
                .set_body_json(rows_body(&json!([]))),
        )
        .mount(&stack.ch)
        .await;
    Mock::given(method("POST"))
        .and(body_string_contains("DESCRIBE TABLE"))
        .respond_with(ResponseTemplate::new(404).set_body_string("Code: 60. (UNKNOWN_TABLE)"))
        .mount(&stack.ch)
        .await;
    mount_launch(&stack.dagster, "run-1").await;
    let a_id = id_of(&stack.upload(&bayu, "a.csv", b"a,b\n1,2\n").await);

    let asking = stack.ingest(&bayu, &a_id, ingest_body("shared_raw"));
    let rival = async {
        // The registry is asked after the ownership check and before the
        // claim, so once it has been asked, A has read "nobody's". The wait
        // is bounded: if the request never comes, the rival claims anyway and
        // the assertions below say what happened.
        for _ in 0..500 {
            if count_requests_containing(&stack.ch, "dataset_catalog").await > 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        stack.claim(RETAIL, "shared_raw", "up-rival").await;
    };
    let (reply, ()) = tokio::join!(asking, rival);

    assert_eq!(reply.status, StatusCode::CONFLICT, "{}", reply.text);
    assert_eq!(reply.json["error"], NOT_FREE);
    assert_eq!(
        stack.status_of(&a_id).await.0,
        "uploaded",
        "A's upload was never marked as loading"
    );
    assert!(
        stack.dagster.received_requests().await.unwrap().is_empty(),
        "nothing was launched"
    );
    assert_eq!(
        stack.claims().await,
        [(
            "shared_raw".to_owned(),
            Some(RETAIL.parse::<Uuid>().unwrap()),
            "up-rival".to_owned()
        )],
        "the name is the rival's, and A made no claim"
    );
}

/// Review finding B4: an upload loaded into `x_raw` and then into `y_raw`
/// leaves `x_raw` claimed. Another upload of the tenant may load into it (the
/// row of the first upload now names only `y_raw`), and another tenant may not.
#[tokio::test]
async fn an_upload_loaded_into_one_table_and_then_another_leaves_the_first_claimed() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    let andi = stack.andi().await;
    mount_free_table(&stack.ch).await;
    mount_launch(&stack.dagster, "run-1").await;
    let first = id_of(&stack.upload(&bayu, "first.csv", b"a,b\n1,2\n").await);

    let into_x = stack.ingest(&bayu, &first, ingest_body("x_raw")).await;
    assert_eq!(into_x.status, StatusCode::OK, "{}", into_x.text);
    uploads::mark_finished(&stack.app.pool, &first, Some("run-1"), None, Some(1))
        .await
        .unwrap()
        .unwrap();
    let into_y = stack.ingest(&bayu, &first, ingest_body("y_raw")).await;
    assert_eq!(into_y.status, StatusCode::OK, "{}", into_y.text);
    assert_eq!(into_y.json["upload"]["bronzeTable"], "y_raw");

    // Another upload of the tenant loads into the first table, which no upload
    // row names any more.
    let second = id_of(&stack.upload(&bayu, "second.csv", b"a,b\n3,4\n").await);
    let append = stack
        .ingest(
            &bayu,
            &second,
            json!({ "bronzeTable": "x_raw", "mode": "append", "encoding": "utf-8", "delimiter": ",", "headerRow": 0 }),
        )
        .await;
    assert_eq!(append.status, StatusCode::OK, "{}", append.text);

    // Another tenant may not.
    let other = id_of(&stack.upload(&andi, "other.csv", b"a,b\n5,6\n").await);
    let refused = stack.ingest(&andi, &other, ingest_body("x_raw")).await;
    assert_eq!(refused.status, StatusCode::CONFLICT, "{}", refused.text);
    assert_eq!(refused.json["error"], NOT_FREE);

    let group = GROUP.parse::<Uuid>().unwrap();
    assert_eq!(
        stack.claims().await,
        [
            ("x_raw".to_owned(), Some(group), first.clone()),
            ("y_raw".to_owned(), Some(group), first.clone()),
        ]
    );
    assert_eq!(
        requests_containing(&stack.dagster, "launchRun").await.len(),
        3,
        "the refused request launched nothing"
    );
}

const UNCHECKED: &str = "Could not check whether that table already exists, so nothing was loaded.";

/// Never "free" on a doubt. When the check cannot be made the answer is 503
/// with a fixed sentence, nothing is claimed, nothing is launched, and no
/// upstream text escapes. First: the registry answers with an error.
#[tokio::test]
async fn a_registry_that_cannot_be_read_is_503_and_assumes_the_name_is_not_free() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    Mock::given(method("POST"))
        .and(body_string_contains("dataset_catalog"))
        .respond_with(
            ResponseTemplate::new(500)
                .set_body_string(format!("Code: 999. {MARKER} (KEEPER_EXCEPTION)")),
        )
        .mount(&stack.ch)
        .await;
    let upload = stack.upload(&bayu, "a.csv", b"a,b\n1,2\n").await;

    let reply = stack
        .ingest(&bayu, &id_of(&upload), ingest_body("stock_raw"))
        .await;

    assert_eq!(
        reply.status,
        StatusCode::SERVICE_UNAVAILABLE,
        "{}",
        reply.text
    );
    assert_eq!(reply.json["error"], UNCHECKED);
    assert!(!reply.text.contains(MARKER), "{}", reply.text);
    assert_eq!(stack.status_of(&id_of(&upload)).await.0, "uploaded");
    assert!(stack.dagster.received_requests().await.unwrap().is_empty());
    assert!(stack.claims().await.is_empty(), "a doubt claims nothing");
}

/// SRC-9 finding G4: on a fresh install the `lake` database does not exist
/// until the orchestrator's first registration, so an upload can be the first
/// thing a tenant does. `Code: 81` (unknown database) reads as "no registry
/// row"; the Iceberg half of the check still runs and says the name is free.
#[tokio::test]
async fn a_registry_database_that_does_not_exist_yet_reads_as_no_claim_and_the_load_proceeds() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    Mock::given(method("POST"))
        .and(body_string_contains("dataset_catalog"))
        .respond_with(ResponseTemplate::new(404).set_body_string(
            "Code: 81. DB::Exception: Database lake does not exist. (UNKNOWN_DATABASE) \
             (version 26.8.1.1 (official build))",
        ))
        .mount(&stack.ch)
        .await;
    Mock::given(method("POST"))
        .and(body_string_contains("DESCRIBE TABLE"))
        .respond_with(ResponseTemplate::new(404).set_body_string("Code: 60. (UNKNOWN_TABLE)"))
        .mount(&stack.ch)
        .await;
    mount_launch(&stack.dagster, "run-1").await;
    let upload = stack.upload(&bayu, "a.csv", b"a,b\n1,2\n").await;

    let reply = stack
        .ingest(&bayu, &id_of(&upload), ingest_body("stock_raw"))
        .await;

    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text);
    assert_eq!(
        requests_containing(&stack.dagster, "launchRun").await.len(),
        1
    );
}

/// The other half still counts: a missing registry database does not make a
/// name free when the Iceberg table already exists.
#[tokio::test]
async fn a_missing_registry_database_does_not_hide_an_existing_iceberg_table() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    Mock::given(method("POST"))
        .and(body_string_contains("dataset_catalog"))
        .respond_with(ResponseTemplate::new(404).set_body_string(
            "Code: 81. DB::Exception: Database lake does not exist. (UNKNOWN_DATABASE)",
        ))
        .mount(&stack.ch)
        .await;
    Mock::given(method("POST"))
        .and(body_string_contains("DESCRIBE TABLE"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(rows_body(&json!([{ "name": "a", "type": "String" }]))),
        )
        .mount(&stack.ch)
        .await;
    let upload = stack.upload(&bayu, "a.csv", b"a,b\n1,2\n").await;

    let reply = stack
        .ingest(&bayu, &id_of(&upload), ingest_body("stock_raw"))
        .await;

    assert_eq!(reply.status, StatusCode::CONFLICT, "{}", reply.text);
    assert!(stack.dagster.received_requests().await.unwrap().is_empty());
}

/// ... nothing listens where `ClickHouse` should be.
#[tokio::test]
async fn a_clickhouse_that_is_not_there_is_503_and_assumes_the_name_is_not_free() {
    let stack = Stack::with(Options {
        ch_url: Some("http://127.0.0.1:1"),
        ..Options::default()
    })
    .await;
    let bayu = stack.bayu().await;
    let upload = stack.upload(&bayu, "a.csv", b"a,b\n1,2\n").await;

    let reply = stack
        .ingest(&bayu, &id_of(&upload), ingest_body("stock_raw"))
        .await;

    assert_eq!(
        reply.status,
        StatusCode::SERVICE_UNAVAILABLE,
        "{}",
        reply.text
    );
    assert_eq!(reply.json["error"], UNCHECKED);
    assert!(!reply.text.contains("127.0.0.1"), "{}", reply.text);
    assert!(stack.dagster.received_requests().await.unwrap().is_empty());
    assert_eq!(stack.status_of(&id_of(&upload)).await.0, "uploaded");
    assert!(stack.claims().await.is_empty(), "a doubt claims nothing");
}

/// ... the registry says nothing, and the Iceberg query database answers with
/// something that is not "no such table".
#[tokio::test]
async fn an_iceberg_answer_that_is_not_no_such_table_is_503_and_assumes_nothing() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    Mock::given(method("POST"))
        .and(body_string_contains("dataset_catalog"))
        .respond_with(ResponseTemplate::new(200).set_body_json(rows_body(&json!([]))))
        .mount(&stack.ch)
        .await;
    Mock::given(method("POST"))
        .and(body_string_contains("DESCRIBE TABLE"))
        .respond_with(
            ResponseTemplate::new(500)
                .set_body_string(format!("Code: 1000. {MARKER} (POCO_EXCEPTION)")),
        )
        .mount(&stack.ch)
        .await;
    let upload = stack.upload(&bayu, "a.csv", b"a,b\n1,2\n").await;

    let reply = stack
        .ingest(&bayu, &id_of(&upload), ingest_body("stock_raw"))
        .await;

    assert_eq!(
        reply.status,
        StatusCode::SERVICE_UNAVAILABLE,
        "{}",
        reply.text
    );
    assert_eq!(reply.json["error"], UNCHECKED);
    assert!(!reply.text.contains(MARKER), "{}", reply.text);
    assert!(stack.dagster.received_requests().await.unwrap().is_empty());
    assert!(stack.claims().await.is_empty(), "a doubt claims nothing");
}

#[tokio::test]
async fn without_an_iceberg_query_database_a_new_name_cannot_be_checked_and_is_503() {
    let stack = Stack::with(Options {
        iceberg_query_db: None,
        ..Options::default()
    })
    .await;
    let bayu = stack.bayu().await;
    Mock::given(method("POST"))
        .and(body_string_contains("dataset_catalog"))
        .respond_with(ResponseTemplate::new(200).set_body_json(rows_body(&json!([]))))
        .mount(&stack.ch)
        .await;
    let upload = stack.upload(&bayu, "a.csv", b"a,b\n1,2\n").await;

    let reply = stack
        .ingest(&bayu, &id_of(&upload), ingest_body("stock_raw"))
        .await;

    assert_eq!(
        reply.status,
        StatusCode::SERVICE_UNAVAILABLE,
        "{}",
        reply.text
    );
    assert!(
        reply.json["error"]
            .as_str()
            .unwrap()
            .contains("ICEBERG_QUERY_DB"),
        "{}",
        reply.text
    );
    assert!(stack.dagster.received_requests().await.unwrap().is_empty());
    assert_eq!(stack.status_of(&id_of(&upload)).await.0, "uploaded");
    assert!(stack.claims().await.is_empty(), "a doubt claims nothing");
}

/// The orchestrator refuses the launch with a message naming its internals:
/// the answer is a fixed 422, the claim is settled so the upload is not left
/// loading, and a retry is possible at once.
#[tokio::test]
async fn a_refused_launch_is_a_fixed_422_and_settles_the_claim() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    mount_free_table(&stack.ch).await;
    mount_launch_refused(&stack.dagster).await;
    let upload = stack.upload(&bayu, "a.csv", b"a,b\n1,2\n").await;
    let id = id_of(&upload);

    let reply = stack.ingest(&bayu, &id, ingest_body("stock_raw")).await;

    assert_eq!(
        reply.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        reply.text
    );
    assert_eq!(
        reply.json["error"],
        "The orchestrator refused to start the load."
    );
    assert!(!reply.text.contains(MARKER), "{}", reply.text);
    let row = stack.status_of(&id).await;
    assert_eq!(row.0, "failed");
    assert_eq!(row.1, None, "no run");
    assert_eq!(row.2.as_deref(), Some("The load could not be started."));
    let shown = stack.get(&bayu, &format!("/api/uploads/{id}")).await;
    assert_eq!(shown.json["status"], "failed");
    assert_eq!(shown.json["error"], "The load could not be started.");
    assert!(!shown.text.contains(MARKER));
    assert_eq!(
        stack.claims().await,
        [(
            "stock_raw".to_owned(),
            Some(GROUP.parse::<Uuid>().unwrap()),
            id.clone()
        )],
        "a claim is never released, a refused launch included (finding B4)"
    );
}

/// The orchestrator cannot be reached (nothing listens, and a server that
/// answers with an error): a fixed 503, the claim settled, nothing leaked.
#[tokio::test]
async fn an_unreachable_orchestrator_is_a_fixed_503_and_settles_the_claim() {
    for dead in [Some("http://127.0.0.1:1/graphql"), None] {
        let stack = Stack::with(Options {
            dagster_url: dead,
            ..Options::default()
        })
        .await;
        let bayu = stack.bayu().await;
        mount_free_table(&stack.ch).await;
        if dead.is_none() {
            Mock::given(method("POST"))
                .respond_with(
                    ResponseTemplate::new(500).set_body_string(format!("{MARKER} internal error")),
                )
                .mount(&stack.dagster)
                .await;
        }
        let upload = stack.upload(&bayu, "a.csv", b"a,b\n1,2\n").await;
        let id = id_of(&upload);

        let reply = stack.ingest(&bayu, &id, ingest_body("stock_raw")).await;

        assert_eq!(
            reply.status,
            StatusCode::SERVICE_UNAVAILABLE,
            "{dead:?}: {}",
            reply.text
        );
        assert_eq!(
            reply.json["error"],
            "The orchestrator could not be reached, so the load was not started."
        );
        assert!(
            !reply.text.contains(MARKER) && !reply.text.contains("127.0.0.1"),
            "{}",
            reply.text
        );
        let row = stack.status_of(&id).await;
        assert_eq!(
            (row.0.as_str(), row.2.as_deref()),
            ("failed", Some("The load could not be started."))
        );
        assert_eq!(
            stack.claims().await.len(),
            1,
            "the name stays claimed: a claim is never released"
        );
    }
}

/// A load that has ended but was never settled (nobody read the upload since)
/// must not block the next one.
#[tokio::test]
async fn an_ingest_settles_a_load_that_has_ended_before_it_decides_the_upload_is_busy() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    mount_run_status(&stack.dagster, "SUCCESS").await;
    mount_launch(&stack.dagster, "run-2").await;
    stack
        .seed_loading(GROUP, "up-1", "stock_raw", Some("run-1"), "1 hour")
        .await;
    mount_results(
        &stack.ch,
        &json!([result_row("up-1", "succeeded", Some(7), "", 30)]),
    )
    .await;

    let reply = stack.ingest(&bayu, "up-1", ingest_body("stock_raw")).await;

    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text);
    assert_eq!(reply.json["runId"], "run-2");
    assert_eq!(reply.json["upload"]["status"], "ingesting");
}

// ════════════════════════════════════════════════════════════════════════
// Settling a load when it is read
// ════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn a_loading_upload_whose_run_succeeded_becomes_ingested_with_the_rows_the_job_recorded() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    mount_run_status(&stack.dagster, "SUCCESS").await;
    mount_results(
        &stack.ch,
        &json!([result_row("up-1", "succeeded", Some(35_000), "", 30)]),
    )
    .await;
    stack
        .seed_loading(GROUP, "up-1", "stock_raw", Some("run-1"), "1 hour")
        .await;

    let shown = stack.get(&bayu, "/api/uploads/up-1").await;

    assert_eq!(shown.status, StatusCode::OK, "{}", shown.text);
    assert_eq!(shown.json["status"], "ingested");
    assert_eq!(shown.json["rows"], 35_000);
    assert_eq!(
        shown.json["assetId"], "stock-raw",
        "the table name with _ as -"
    );
    assert!(shown.json.get("error").is_none());
    assert_eq!(shown.json["bronzeTable"], "stock_raw");
    // It is written, not recomputed: the list says the same and asks nobody.
    let before = stack.dagster.received_requests().await.unwrap().len();
    let list = stack.get(&bayu, "/api/uploads").await;
    assert_eq!(list.json[0]["status"], "ingested");
    assert_eq!(list.json[0]["rows"], 35_000);
    assert_eq!(
        stack.dagster.received_requests().await.unwrap().len(),
        before
    );
    assert_eq!(
        count_requests_containing(&stack.ch, "bronze_meta.ingest_run").await,
        1,
        "the result was read once, when the row was settled"
    );
}

/// A load whose sink reported no total is `ingested` with no `rows`: not
/// measured, never 0.
#[tokio::test]
async fn a_load_with_no_recorded_total_is_ingested_with_no_row_count_not_zero() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    mount_run_status(&stack.dagster, "SUCCESS").await;
    mount_results(
        &stack.ch,
        &json!([result_row("up-1", "succeeded", None, "", 5)]),
    )
    .await;
    stack
        .seed_loading(GROUP, "up-1", "stock_raw", Some("run-1"), "1 hour")
        .await;

    let shown = stack.get(&bayu, "/api/uploads/up-1").await;

    assert_eq!(shown.json["status"], "ingested");
    assert!(shown.json.get("rows").is_none(), "{}", shown.text);
}

#[tokio::test]
async fn a_failed_run_is_settled_with_the_reason_the_job_recorded() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    mount_run_status(&stack.dagster, "FAILURE").await;
    mount_results(
        &stack.ch,
        &json!([result_row(
            "up-1",
            "failed",
            None,
            "The file has no rows below the header row.",
            5
        )]),
    )
    .await;
    stack
        .seed_loading(GROUP, "up-1", "stock_raw", Some("run-1"), "1 hour")
        .await;

    let shown = stack.get(&bayu, "/api/uploads/up-1").await;

    assert_eq!(shown.json["status"], "failed");
    assert_eq!(
        shown.json["error"],
        "The file has no rows below the header row."
    );
    assert!(shown.json.get("rows").is_none() && shown.json.get("assetId").is_none());
}

/// An old result (an earlier load of the same upload) is not this load's, and
/// a run that ended with no result says so.
#[tokio::test]
async fn a_run_that_ended_without_a_result_of_its_own_is_failed_not_matched_to_an_old_one() {
    for (results, why) in [
        (json!([]), "no result at all"),
        (
            json!([result_row("up-1", "succeeded", Some(99), "", 120)]),
            "only an older load's result",
        ),
    ] {
        let stack = Stack::start().await;
        let bayu = stack.bayu().await;
        mount_run_status(&stack.dagster, "SUCCESS").await;
        mount_results(&stack.ch, &results).await;
        stack
            .seed_loading(GROUP, "up-1", "stock_raw", Some("run-1"), "1 hour")
            .await;

        let shown = stack.get(&bayu, "/api/uploads/up-1").await;

        assert_eq!(shown.json["status"], "failed", "{why}: {}", shown.text);
        assert_eq!(
            shown.json["error"], "The load stopped before it recorded a result.",
            "{why}"
        );
        assert!(shown.json.get("rows").is_none(), "{why}");
    }
}

/// A recorded failure with no reason gets a fixed one; the job's reason, when
/// it gave one, is what is shown.
#[tokio::test]
async fn a_recorded_failure_with_no_reason_is_the_fixed_one() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    mount_run_status(&stack.dagster, "FAILURE").await;
    mount_results(
        &stack.ch,
        &json!([result_row("up-1", "failed", None, "", 5)]),
    )
    .await;
    stack
        .seed_loading(GROUP, "up-1", "stock_raw", Some("run-1"), "1 hour")
        .await;

    let shown = stack.get(&bayu, "/api/uploads/up-1").await;

    assert_eq!(shown.json["error"], "The load failed.");
}

#[tokio::test]
async fn a_queued_or_running_load_is_left_alone_and_the_results_are_not_read() {
    for status in ["QUEUED", "STARTING", "STARTED", "NOT_STARTED"] {
        let stack = Stack::start().await;
        let bayu = stack.bayu().await;
        mount_run_status(&stack.dagster, status).await;
        stack
            .seed_loading(GROUP, "up-1", "stock_raw", Some("run-1"), "1 hour")
            .await;

        let shown = stack.get(&bayu, "/api/uploads/up-1").await;

        assert_eq!(shown.json["status"], "ingesting", "{status}");
        assert!(
            stack.ch.received_requests().await.unwrap().is_empty(),
            "{status}"
        );
    }
}

/// When the orchestrator cannot be asked the row is returned as it is, and a
/// list does not ask again for every loading row.
#[tokio::test]
async fn an_unreachable_orchestrator_leaves_a_loading_upload_as_it_was_and_the_list_asks_once() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(500).set_body_string(format!("{MARKER} down")))
        .mount(&stack.dagster)
        .await;
    // Three hours: past the bound of finding B5, so an orchestrator that
    // cannot be asked, read as one that does not know the run, would fail
    // these two. It is not: the doubt leaves them loading.
    stack
        .seed_loading(GROUP, "up-1", "stock_raw", Some("run-1"), "3 hours")
        .await;
    stack
        .seed_loading(GROUP, "up-2", "other_raw", Some("run-2"), "3 hours")
        .await;

    let list = stack.get(&bayu, "/api/uploads").await;

    assert_eq!(list.status, StatusCode::OK, "{}", list.text);
    let statuses: Vec<&str> = list
        .json
        .as_array()
        .unwrap()
        .iter()
        .map(|u| u["status"].as_str().unwrap())
        .collect();
    assert_eq!(statuses, ["ingesting", "ingesting"]);
    assert!(!list.text.contains(MARKER));
    assert_eq!(
        stack.dagster.received_requests().await.unwrap().len(),
        1,
        "stopped asking after the first failure"
    );
    let one = stack.get(&bayu, "/api/uploads/up-1").await;
    assert_eq!(one.json["status"], "ingesting");
}

/// A hung orchestrator is also "cannot be reached": a read is not held for
/// the request deadline.
#[tokio::test]
async fn a_hung_orchestrator_does_not_hold_a_read() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    Mock::given(method("POST"))
        .and(body_string_contains("pipelineRunOrError"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(Duration::from_secs(8))
                .set_body_json(run_status_body("SUCCESS")),
        )
        .mount(&stack.dagster)
        .await;
    mount_results(
        &stack.ch,
        &json!([result_row("up-1", "succeeded", Some(1), "", 5)]),
    )
    .await;
    // Three hours, for the reason given at the test above (finding B5).
    stack
        .seed_loading(GROUP, "up-1", "stock_raw", Some("run-1"), "3 hours")
        .await;

    let started = Instant::now();
    let shown = stack.get(&bayu, "/api/uploads/up-1").await;

    assert_eq!(shown.status, StatusCode::OK, "{}", shown.text);
    assert_eq!(
        shown.json["status"], "ingesting",
        "the late SUCCESS was not waited for"
    );
    assert!(
        started.elapsed() < Duration::from_secs(7),
        "{:?}",
        started.elapsed()
    );
}

#[tokio::test]
async fn results_that_cannot_be_read_leave_an_ended_load_as_it_was() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    mount_run_status(&stack.dagster, "SUCCESS").await;
    Mock::given(method("POST"))
        .and(body_string_contains("bronze_meta.ingest_run"))
        .respond_with(
            ResponseTemplate::new(500).set_body_string(format!("{MARKER} (KEEPER_EXCEPTION)")),
        )
        .mount(&stack.ch)
        .await;
    stack
        .seed_loading(GROUP, "up-1", "stock_raw", Some("run-1"), "1 hour")
        .await;

    let shown = stack.get(&bayu, "/api/uploads/up-1").await;

    assert_eq!(
        shown.json["status"], "ingesting",
        "a doubt is not an outcome"
    );
    assert!(!shown.text.contains(MARKER));
}

/// When the results cannot be read, a list does not try again for every
/// ended load.
#[tokio::test]
async fn a_list_stops_reading_results_after_the_first_failure() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    mount_run_status(&stack.dagster, "SUCCESS").await;
    Mock::given(method("POST"))
        .and(body_string_contains("bronze_meta.ingest_run"))
        .respond_with(ResponseTemplate::new(500).set_body_string(MARKER))
        .mount(&stack.ch)
        .await;
    stack
        .seed_loading(GROUP, "up-1", "stock_raw", Some("run-1"), "1 hour")
        .await;
    stack
        .seed_loading(GROUP, "up-2", "other_raw", Some("run-2"), "1 hour")
        .await;

    let list = stack.get(&bayu, "/api/uploads").await;

    assert_eq!(list.status, StatusCode::OK, "{}", list.text);
    let statuses: Vec<&str> = list
        .json
        .as_array()
        .unwrap()
        .iter()
        .map(|u| u["status"].as_str().unwrap())
        .collect();
    assert_eq!(statuses, ["ingesting", "ingesting"]);
    assert_eq!(
        count_requests_containing(&stack.ch, "bronze_meta.ingest_run").await,
        1
    );
    assert!(!list.text.contains(MARKER));
}

/// A claim that never got a run is abandoned after two minutes, and not
/// before: the request that made it may still be about to attach one.
#[tokio::test]
async fn a_claim_with_no_run_is_failed_after_two_minutes_and_left_alone_before() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    stack
        .seed_loading(GROUP, "up-fresh", "fresh_raw", None, "30 seconds")
        .await;
    stack
        .seed_loading(GROUP, "up-stale", "stale_raw", None, "3 minutes")
        .await;

    let fresh = stack.get(&bayu, "/api/uploads/up-fresh").await;
    let stale = stack.get(&bayu, "/api/uploads/up-stale").await;

    assert_eq!(fresh.json["status"], "ingesting");
    assert_eq!(stale.json["status"], "failed");
    assert_eq!(stale.json["error"], "The load was not started.");
    assert!(
        stack.dagster.received_requests().await.unwrap().is_empty(),
        "there is no run to ask about"
    );
    // and it can be loaded again
    mount_free_table(&stack.ch).await;
    mount_launch(&stack.dagster, "run-2").await;
    let again = stack
        .ingest(&bayu, "up-stale", ingest_body("stale_raw"))
        .await;
    assert_eq!(again.status, StatusCode::OK, "{}", again.text);
}

/// Review finding B5: a run the orchestrator does not know is not left loading
/// for ever. When the job recorded a result, that settles the upload whatever
/// the claim's age.
#[tokio::test]
async fn a_run_the_orchestrator_does_not_know_is_settled_by_the_result_the_job_recorded() {
    for (recorded, status, rows, error) in [
        (
            result_row("up-1", "succeeded", Some(12), "", 5),
            "ingested",
            Some(12),
            None,
        ),
        (
            result_row(
                "up-1",
                "failed",
                None,
                "The file has no rows below the header row.",
                5,
            ),
            "failed",
            None,
            Some("The file has no rows below the header row."),
        ),
    ] {
        let stack = Stack::start().await;
        let bayu = stack.bayu().await;
        mount_unknown_run(&stack.dagster).await;
        mount_results(&stack.ch, &json!([recorded])).await;
        stack
            .seed_loading(GROUP, "up-1", "stock_raw", Some("run-1"), "20 minutes")
            .await;

        let shown = stack.get(&bayu, "/api/uploads/up-1").await;

        assert_eq!(shown.status, StatusCode::OK, "{}", shown.text);
        assert_eq!(shown.json["status"], status, "{}", shown.text);
        assert_eq!(shown.json["rows"], json!(rows));
        assert_eq!(shown.json.get("error").and_then(Value::as_str), error);
        assert!(!shown.text.contains(MARKER), "{}", shown.text);
    }
}

/// Review finding B5: no result either, and the claim more than an hour old:
/// the upload is failed, and so is no longer stuck (it could be neither deleted
/// nor loaded again). A younger claim is left alone, because the run may be
/// about to start or to record its result. An older load's result is nobody's
/// answer.
#[tokio::test]
async fn a_lost_run_with_no_result_is_failed_after_an_hour_and_left_alone_before() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    mount_unknown_run(&stack.dagster).await;
    mount_results(
        &stack.ch,
        &json!([result_row("up-older", "succeeded", Some(99), "", 300)]),
    )
    .await;
    mount_launch(&stack.dagster, "run-2").await;
    stack
        .seed_loading(GROUP, "up-old", "old_raw", Some("run-1"), "2 hours")
        .await;
    stack
        .seed_loading(GROUP, "up-old-b", "old_b_raw", Some("run-1b"), "2 hours")
        .await;
    stack
        .seed_loading(GROUP, "up-young", "young_raw", Some("run-3"), "30 minutes")
        .await;

    let old = stack.get(&bayu, "/api/uploads/up-old").await;
    let young = stack.get(&bayu, "/api/uploads/up-young").await;

    assert_eq!(old.json["status"], "failed", "{}", old.text);
    assert_eq!(
        old.json["error"],
        "The orchestrator no longer knows this load."
    );
    assert!(old.json.get("rows").is_none() && old.json.get("assetId").is_none());
    assert!(!old.text.contains(MARKER), "{}", old.text);
    assert_eq!(
        stack.status_of("up-old").await.1.as_deref(),
        Some("run-1"),
        "settled under its run"
    );
    assert_eq!(young.json["status"], "ingesting", "{}", young.text);
    assert_eq!(stack.status_of("up-young").await.0, "ingesting");

    // No longer stuck: a failed upload can be loaded again and deleted, and
    // the young one still cannot.
    let again = stack
        .ingest(&bayu, "up-old-b", ingest_body("old_b_raw"))
        .await;
    assert_eq!(again.status, StatusCode::OK, "{}", again.text);
    assert_eq!(again.json["runId"], "run-2");
    let delete = stack
        .send(&bayu, "DELETE", "/api/uploads/up-old", Payload::None)
        .await;
    assert_eq!(delete.status, StatusCode::NO_CONTENT, "{}", delete.text);
    let young_delete = stack
        .send(&bayu, "DELETE", "/api/uploads/up-young", Payload::None)
        .await;
    assert_eq!(
        young_delete.status,
        StatusCode::CONFLICT,
        "{}",
        young_delete.text
    );
}

/// Review finding B5: when the results cannot be read the upload is left as it
/// is, however old the claim, and a list does not try again for every upload.
#[tokio::test]
async fn a_lost_run_whose_results_cannot_be_read_is_left_alone_however_old() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    mount_unknown_run(&stack.dagster).await;
    Mock::given(method("POST"))
        .and(body_string_contains("bronze_meta.ingest_run"))
        .respond_with(
            ResponseTemplate::new(500).set_body_string(format!("{MARKER} (KEEPER_EXCEPTION)")),
        )
        .mount(&stack.ch)
        .await;
    stack
        .seed_loading(GROUP, "up-1", "stock_raw", Some("run-1"), "3 hours")
        .await;
    stack
        .seed_loading(GROUP, "up-2", "other_raw", Some("run-2"), "3 hours")
        .await;

    let list = stack.get(&bayu, "/api/uploads").await;

    assert_eq!(list.status, StatusCode::OK, "{}", list.text);
    let statuses: Vec<&str> = list
        .json
        .as_array()
        .unwrap()
        .iter()
        .map(|u| u["status"].as_str().unwrap())
        .collect();
    assert_eq!(
        statuses,
        ["ingesting", "ingesting"],
        "a doubt is not an outcome"
    );
    assert_eq!(
        count_requests_containing(&stack.ch, "bronze_meta.ingest_run").await,
        1,
        "stopped reading after the first failure"
    );
    assert!(!list.text.contains(MARKER));
}

/// Review finding B6: the reason a job recorded reaches a response, and the
/// row, only when it is one of the seven the API knows. Exception text, a path
/// and a marker the job might have recorded are the fixed `The load failed.`.
#[tokio::test]
async fn a_reason_the_job_recorded_that_the_api_does_not_know_reaches_neither_a_response_nor_the_row()
 {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    mount_run_status(&stack.dagster, "FAILURE").await;
    mount_results(
        &stack.ch,
        &json!([result_row(
            "up-1",
            "failed",
            None,
            &format!("KeyError: {MARKER} at /app/dagster/dispar_orchestrate/file_ingest.py:88"),
            5
        )]),
    )
    .await;
    stack
        .seed_loading(GROUP, "up-1", "stock_raw", Some("run-1"), "1 hour")
        .await;

    let shown = stack.get(&bayu, "/api/uploads/up-1").await;

    assert_eq!(shown.json["status"], "failed", "{}", shown.text);
    assert_eq!(shown.json["error"], "The load failed.");
    assert!(!shown.text.contains(MARKER), "{}", shown.text);
    assert_eq!(
        stack.status_of("up-1").await.2.as_deref(),
        Some("The load failed."),
        "the recorded text is not written to the row either"
    );
    let list = stack.get(&bayu, "/api/uploads").await;
    assert!(!list.text.contains(MARKER), "{}", list.text);
}

// ════════════════════════════════════════════════════════════════════════
// DELETE /api/uploads/{id}
// ════════════════════════════════════════════════════════════════════════

/// Deleting removes the file and the row; the table, and the claim on its
/// name, stay (review finding B4: the row is no longer the record of that).
#[tokio::test]
async fn a_delete_removes_the_object_and_the_row() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    let upload = stack.upload(&bayu, "a.csv", b"a,b\n1,2\n").await;
    let id = id_of(&upload);
    assert_eq!(stack.bucket.keys().len(), 1);

    let delete = stack
        .send(
            &bayu,
            "DELETE",
            &format!("/api/uploads/{id}"),
            Payload::None,
        )
        .await;

    assert_eq!(delete.status, StatusCode::NO_CONTENT, "{}", delete.text);
    assert!(delete.text.is_empty());
    assert!(stack.bucket.keys().is_empty(), "the object is gone");
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM file_upload WHERE id = $1")
        .bind(&id)
        .fetch_one(&stack.app.pool)
        .await
        .unwrap();
    assert_eq!(rows, 0, "the row is gone");
    assert_eq!(
        stack.get(&bayu, &format!("/api/uploads/{id}")).await.status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(stack.get(&bayu, "/api/uploads").await.json, json!([]));
    let again = stack
        .send(
            &bayu,
            "DELETE",
            &format!("/api/uploads/{id}"),
            Payload::None,
        )
        .await;
    assert_eq!(again.status, StatusCode::NOT_FOUND, "{}", again.text);
    let audit = stack.audit_actions().await;
    let event = audit
        .iter()
        .find(|a| a.0 == "upload.delete")
        .expect("an upload.delete event");
    assert_eq!(event.2.as_deref(), Some(id.as_str()));
    assert_eq!(event.3, json!({ "fileName": "a.csv", "sizeBytes": 8 }));
}

#[tokio::test]
async fn deleting_an_upload_that_loaded_a_table_records_the_table_and_leaves_the_claim() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    stack.seed_ingested(GROUP, "up-1", "stock_raw").await;

    let delete = stack
        .send(&bayu, "DELETE", "/api/uploads/up-1", Payload::None)
        .await;

    assert_eq!(delete.status, StatusCode::NO_CONTENT, "{}", delete.text);
    let audit = stack.audit_actions().await;
    assert_eq!(audit.last().unwrap().3["table"], "stock_raw");
    let tenant_id: Uuid = GROUP.parse().unwrap();
    assert_eq!(
        uploads::table_claim(&stack.app.pool, tenant_id, "stock_raw")
            .await
            .unwrap(),
        uploads::TableClaim::Ours
    );
    assert!(
        uploads::table_claimed(&stack.app.pool, "stock_raw")
            .await
            .unwrap(),
        "a connector still cannot take it"
    );
}

/// The routes' own refusals and the guard that keeps a loaded upload's
/// object: a delete of an upload that is loading touches nothing, whichever
/// way it arrives.
#[tokio::test]
async fn a_delete_of_a_loading_upload_whose_run_ended_is_settled_first() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    mount_run_status(&stack.dagster, "FAILURE").await;
    mount_results(
        &stack.ch,
        &json!([result_row(
            "up-1",
            "failed",
            None,
            "The load into the table failed.",
            5
        )]),
    )
    .await;
    stack
        .seed_loading(GROUP, "up-1", "stock_raw", Some("run-1"), "1 hour")
        .await;

    let delete = stack
        .send(&bayu, "DELETE", "/api/uploads/up-1", Payload::None)
        .await;

    assert_eq!(delete.status, StatusCode::NO_CONTENT, "{}", delete.text);
    assert!(stack.bucket.keys().is_empty());
}

// ════════════════════════════════════════════════════════════════════════
// What a response never carries
// ════════════════════════════════════════════════════════════════════════

/// Over a whole session of reads and writes, no response names an object key,
/// a tenant id, a checksum or the content type.
#[tokio::test]
async fn no_response_of_a_whole_session_carries_the_key_the_tenant_the_checksum_or_the_type() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    mount_free_table(&stack.ch).await;
    mount_launch(&stack.dagster, "run-1").await;
    let bytes = fixture("semicolon.csv");
    let upload = stack.upload(&bayu, "a.csv", &bytes).await;
    let id = id_of(&upload);
    let duplicate = stack.upload(&bayu, "b.csv", &bytes).await;
    let responses = [
        upload.to_string(),
        duplicate.to_string(),
        stack.get(&bayu, "/api/uploads").await.text,
        stack.get(&bayu, &format!("/api/uploads/{id}")).await.text,
        stack
            .get(&bayu, &format!("/api/uploads/{id}/preview"))
            .await
            .text,
        stack
            .ingest(&bayu, &id, ingest_body("stock_raw"))
            .await
            .text,
    ];
    for response in &responses {
        for leaked in [
            "uploads/",
            GROUP,
            &sha256_hex(&bytes),
            "text/csv",
            "storageKey",
            "tenantId",
        ] {
            assert!(!response.contains(leaked), "{leaked} in {response}");
        }
    }
}

// ════════════════════════════════════════════════════════════════════════
// Excel workbooks (X2 of docs/superpowers/plans/2026-10-07-upload-excel.md)
// ════════════════════════════════════════════════════════════════════════

const UNREADABLE_WORKBOOK: &str =
    "This file could not be opened as an Excel workbook (.xls or .xlsx).";
const UNKNOWN_SHEET: &str = "The workbook has no sheet with that name.";
const EMPTY_SHEET: &str = "That sheet is empty, so there is nothing to load.";

/// The sheets `stock.xlsx` and `stock.xls` hold, as the API lists them.
fn stock_sheets() -> Value {
    json!([
        { "name": "Stock", "visible": true },
        { "name": "Quirks", "visible": true },
        { "name": "Offset", "visible": true },
        { "name": "Hidden notes", "visible": false },
        { "name": "Empty", "visible": true },
    ])
}

async fn refused_upload(stack: &Stack, who: &Caller, name: &str, bytes: &[u8]) -> String {
    let reply = stack
        .send(
            who,
            "POST",
            "/api/uploads",
            file_form(name, "application/octet-stream", bytes),
        )
        .await;
    assert_eq!(
        reply.status,
        StatusCode::BAD_REQUEST,
        "{name}: {}",
        reply.text
    );
    reply.json["error"].as_str().unwrap().to_owned()
}

#[tokio::test]
async fn a_workbook_is_stored_as_it_arrived_and_the_answer_lists_its_sheets() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    for (name, bytes) in [
        ("stock.xlsx", workbook("stock.xlsx")),
        ("old stock.xls", workbook("stock.xls")),
    ] {
        let upload = stack.upload(&bayu, name, &bytes).await;
        let id = id_of(&upload);
        assert_eq!(upload["status"], "uploaded", "{name}");
        assert_eq!(upload["workbook"]["sheets"], stock_sheets(), "{name}");
        assert_eq!(upload["workbook"]["defaultSheet"], "Stock", "{name}");
        let extension = name.rsplit('.').next().unwrap();
        let key = format!("uploads/{GROUP}/{id}.{extension}");
        assert_eq!(
            stack.bucket.get(&key).unwrap(),
            bytes,
            "{name}: byte for byte"
        );
    }
    let text = stack.upload(&bayu, "a.csv", b"a,b\n1,2\n").await;
    assert!(text.get("workbook").is_none(), "a text file has no sheets");
}

#[tokio::test]
async fn a_workbook_that_cannot_be_read_or_is_not_named_like_one_is_refused_with_its_reason() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    let xlsx = workbook("stock.xlsx");
    let not_accepted = "This looks like a workbook or a zip archive that is not named like one. Excel workbooks must end in .xls or .xlsx and zip archives in .zip; or save the sheet as .xlsx or CSV first.";
    // A workbook-shaped file under a name that is not `.xls` or `.xlsx`.
    for name in ["macros.xlsm", "binary.xlsb", "sheet.ods", "noext"] {
        assert_eq!(
            refused_upload(&stack, &bayu, name, &xlsx).await,
            not_accepted,
            "{name}"
        );
    }
    // `DATA-17` slice 4: named `.zip` it is an archive, and a workbook's dozen parts are
    // not the up to ten files of one supported format a zip may hold.
    let as_zip = refused_upload(&stack, &bayu, "archive.zip", &xlsx).await;
    assert!(as_zip.contains("at most 10 files"), "{as_zip}");
    // Named like a workbook, and not one.
    let mut encrypted = vec![0xD0, 0xCF, 0x11, 0xE0];
    encrypted.extend_from_slice(&[0_u8; 600]);
    encrypted.extend("EncryptedPackage".encode_utf16().flat_map(u16::to_le_bytes));
    encrypted.extend_from_slice(&[0_u8; 600]);
    let stock_xls = workbook("stock.xls");
    for (name, bytes) in [
        ("plain.xlsx", workbook("plain.zip")),
        ("cut.xlsx", xlsx[..xlsx.len() / 2].to_vec()),
        ("cut.xls", stock_xls[..600].to_vec()),
        ("garbage.xls", vec![0xD0, 0xCF, 0x11, 0xE0, 1, 2, 3, 4]),
    ] {
        assert_eq!(
            refused_upload(&stack, &bayu, name, &bytes).await,
            UNREADABLE_WORKBOOK,
            "{name}"
        );
    }
    assert_eq!(
        refused_upload(&stack, &bayu, "locked.xlsx", &encrypted).await,
        "This workbook is password protected, so it cannot be read. Remove the password and upload it again."
    );
    assert!(
        stack.bucket.keys().is_empty(),
        "nothing was stored for a refused file"
    );
    assert!(stack.audit_actions().await.is_empty());
}

#[tokio::test]
async fn a_workbook_preview_shows_the_default_sheet_as_text_under_the_fixed_dialect() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    let expected_rows = json!([
        ["A-100", "Bolt, hex M6", "10", "1.5", "2025-09-24"],
        ["A-200", "Washer", "250", "0.05", "2025-09-25"],
        [
            "A-300",
            "Flange DN50 \u{2013} steel",
            "4",
            "12",
            "2025-10-01"
        ]
    ]);
    for (name, bytes) in [
        ("stock.xlsx", workbook("stock.xlsx")),
        ("stock.xls", workbook("stock.xls")),
    ] {
        let upload = stack.upload(&bayu, name, &bytes).await;
        let uri = format!("/api/uploads/{}/preview", id_of(&upload));
        let reply = stack.get(&bayu, &uri).await;
        assert_eq!(reply.status, StatusCode::OK, "{name}: {}", reply.text);
        assert_eq!(
            reply.json["columns"],
            json!(["sku", "name", "qty", "price", "received"]),
            "{name}"
        );
        assert_eq!(reply.json["rows"], expected_rows, "{name}");
        assert_eq!(reply.json["truncated"], false, "{name}");
        let reading = json!({ "encoding": "utf-8", "delimiter": ",", "headerRow": 0 });
        assert_eq!(reply.json["using"], reading, "{name}");
        assert_eq!(reply.json["workbook"]["sheet"], "Stock", "{name}");
        assert_eq!(reply.json["workbook"]["sheets"], stock_sheets(), "{name}");
        assert_eq!(reply.json["workbook"]["defaultSheet"], "Stock", "{name}");

        // A header row of the user's choosing, counted in the sheet's records.
        let moved = stack.get(&bayu, &format!("{uri}?headerRow=1")).await;
        assert_eq!(moved.status, StatusCode::OK, "{name}: {}", moved.text);
        assert_eq!(
            moved.json["columns"],
            json!(["A-100", "Bolt, hex M6", "10", "1.5", "2025-09-24"]),
            "{name}"
        );
        assert_eq!(moved.json["rows"].as_array().unwrap().len(), 2, "{name}");
        assert_eq!(moved.json["using"]["headerRow"], 1, "{name}");

        // The encoding and the delimiter mean nothing for a workbook: a valid
        // value is ignored, an invalid one is refused as it is for text.
        let ignored = stack
            .get(&bayu, &format!("{uri}?encoding=utf-16&delimiter=%3B"))
            .await;
        assert_eq!(ignored.status, StatusCode::OK, "{name}: {}", ignored.text);
        assert_eq!(ignored.json["using"], reading, "{name}");
        assert_eq!(ignored.json["columns"], reply.json["columns"], "{name}");
        let refused = stack.get(&bayu, &format!("{uri}?encoding=latin-1")).await;
        assert_eq!(refused.status, StatusCode::BAD_REQUEST, "{name}");
        assert_eq!(
            refused.json["error"], "encoding must be utf-8 or utf-16.",
            "{name}"
        );
    }
}

#[tokio::test]
async fn a_workbook_preview_reads_the_sheet_that_was_asked_for_hidden_and_empty_ones_included() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    let upload = stack
        .upload(&bayu, "stock.xlsx", &workbook("stock.xlsx"))
        .await;
    let uri = format!("/api/uploads/{}/preview", id_of(&upload));

    let quirks = stack.get(&bayu, &format!("{uri}?sheet=Quirks")).await;
    assert_eq!(quirks.status, StatusCode::OK, "{}", quirks.text);
    assert_eq!(quirks.json["workbook"]["sheet"], "Quirks");
    assert_eq!(quirks.json["columns"], json!(["case", "value"]));
    assert_eq!(quirks.json["rows"][0], json!(["whole number", "1"]));
    assert_eq!(quirks.json["rows"][8], json!(["date", "2025-09-24"]));
    assert_eq!(
        quirks.json["rows"].as_array().unwrap().len(),
        20,
        "the preview returns 20 rows"
    );
    assert_eq!(quirks.json["truncated"], true, "the sheet has 27");

    let hidden = stack
        .get(&bayu, &format!("{uri}?sheet=Hidden%20notes"))
        .await;
    assert_eq!(hidden.status, StatusCode::OK, "{}", hidden.text);
    assert_eq!(hidden.json["columns"], json!(["note"]));
    assert_eq!(hidden.json["rows"], json!([["internal"]]));

    let empty = stack.get(&bayu, &format!("{uri}?sheet=Empty")).await;
    assert_eq!(empty.status, StatusCode::OK, "{}", empty.text);
    assert_eq!(empty.json["columns"], json!([]));
    assert_eq!(empty.json["rows"], json!([]));
    assert_eq!(empty.json["workbook"]["sheet"], "Empty");

    for name in ["stock", "", "Quirks%20"] {
        let unknown = stack.get(&bayu, &format!("{uri}?sheet={name}")).await;
        assert_eq!(unknown.status, StatusCode::BAD_REQUEST, "{name:?}");
        assert_eq!(unknown.json["error"], UNKNOWN_SHEET, "{name:?}");
    }
}

#[tokio::test]
async fn a_text_file_named_like_a_workbook_previews_as_text_and_a_csv_preview_has_no_workbook() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    let text = stack.upload(&bayu, "stock.xlsx", b"id,name\n1,a\n").await;
    let reply = stack
        .get(
            &bayu,
            &format!("/api/uploads/{}/preview?sheet=Nope", id_of(&text)),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text);
    assert_eq!(reply.json["columns"], json!(["id", "name"]));
    assert!(reply.json.get("workbook").is_none());
}

/// The ingest body of a workbook: no encoding, no delimiter.
fn workbook_ingest_body(table: &str, sheet: Option<&str>) -> Value {
    let mut body = json!({ "bronzeTable": table, "headerRow": 0 });
    if let Some(sheet) = sheet {
        body["sheet"] = json!(sheet);
    }
    body
}

#[tokio::test]
async fn a_workbook_ingest_stores_the_converted_sheet_beside_the_original_and_launches_on_it() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    mount_free_table(&stack.ch).await;
    mount_launch(&stack.dagster, "run-1").await;
    let bytes = workbook("stock.xlsx");
    let upload = stack.upload(&bayu, "stock.xlsx", &bytes).await;
    let id = id_of(&upload);

    let reply = stack
        .ingest(
            &bayu,
            &id,
            workbook_ingest_body("quirks_raw", Some("Quirks")),
        )
        .await;

    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text);
    assert_eq!(reply.json["runId"], "run-1");
    assert_eq!(
        reply.json["upload"]["parseOptions"],
        json!({ "encoding": "utf-8", "delimiter": ",", "headerRow": 0, "sheet": "Quirks" })
    );
    let original = format!("uploads/{GROUP}/{id}.xlsx");
    let converted = format!("{original}.converted.csv");
    assert_eq!(stack.bucket.keys(), [original.clone(), converted.clone()]);
    assert_eq!(
        stack.bucket.get(&original).unwrap(),
        bytes,
        "the original is untouched"
    );
    assert_eq!(
        stack.bucket.get(&converted).unwrap(),
        fixture("converted_sheet.csv"),
        "the pinned conversion of the Quirks sheet"
    );

    let launches = requests_containing(&stack.dagster, "launchRun").await;
    assert_eq!(launches.len(), 1);
    assert_eq!(
        launches[0]["variables"]["cfg"],
        json!({ "ops": { "ingest_uploaded_file": { "config": {
            "upload_id": id,
            "storage_keys": [converted],
            "format": "delimited",
            "compression": "none",
            "max_bytes": UNPACKED_CAP,
            "bronze_table_name": "quirks_raw",
            "load_mode": "replace",
            "encoding": "utf-8",
            "delimiter": ",",
            "header_row": 0,
        "columns": [],
        "on_bad_value": "stop",
        } } } }),
        "the job's own configuration, the converted object and the fixed dialect"
    );
    let audit = stack.audit_actions().await;
    let ingest = audit.iter().find(|a| a.0 == "upload.ingest").unwrap();
    assert_eq!(
        ingest.3,
        json!({
            "fileName": "stock.xlsx", "sizeBytes": bytes.len(),
            "table": "quirks_raw", "mode": "replace", "sheet": "Quirks"
        })
    );
}

#[tokio::test]
async fn a_workbook_ingest_without_a_sheet_loads_the_default_sheet_of_an_xls_too() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    mount_free_table(&stack.ch).await;
    mount_launch(&stack.dagster, "run-1").await;
    let upload = stack
        .upload(&bayu, "stock.xls", &workbook("stock.xls"))
        .await;
    let id = id_of(&upload);

    let reply = stack
        .ingest(&bayu, &id, workbook_ingest_body("stock_raw", None))
        .await;

    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text);
    assert_eq!(reply.json["upload"]["parseOptions"]["sheet"], "Stock");
    let converted = stack
        .bucket
        .get(&format!("uploads/{GROUP}/{id}.xls.converted.csv"))
        .unwrap();
    assert_eq!(
        String::from_utf8(converted).unwrap(),
        "sku,name,qty,price,received\n\
         A-100,\"Bolt, hex M6\",10,1.5,2025-09-24\n\
         A-200,Washer,250,0.05,2025-09-25\n\
         A-300,Flange DN50 \u{2013} steel,4,12,2025-10-01\n"
    );
}

#[tokio::test]
async fn a_workbook_ingest_that_cannot_convert_claims_nothing_and_launches_nothing() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    mount_free_table(&stack.ch).await;
    mount_launch(&stack.dagster, "run-1").await;
    let upload = stack
        .upload(&bayu, "stock.xlsx", &workbook("stock.xlsx"))
        .await;
    let id = id_of(&upload);

    for (body, sentence) in [
        (workbook_ingest_body("t_raw", Some("Nope")), UNKNOWN_SHEET),
        (workbook_ingest_body("t_raw", Some("Empty")), EMPTY_SHEET),
        (
            json!({ "bronzeTable": "t_raw", "headerRow": 0, "sheet": 3 }),
            "sheet must be text.",
        ),
    ] {
        let reply = stack.ingest(&bayu, &id, body.clone()).await;
        assert_eq!(
            reply.status,
            StatusCode::BAD_REQUEST,
            "{body}: {}",
            reply.text
        );
        assert_eq!(reply.json["error"], sentence, "{body}");
    }
    assert!(stack.dagster.received_requests().await.unwrap().is_empty());
    assert_eq!(stack.status_of(&id).await.0, "uploaded");
    assert!(stack.claims().await.is_empty());
    assert_eq!(
        stack.bucket.keys(),
        [format!("uploads/{GROUP}/{id}.xlsx")],
        "no converted object for a sheet that could not be converted"
    );
}

#[tokio::test]
async fn a_text_file_named_like_a_workbook_is_loaded_as_text() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    mount_free_table(&stack.ch).await;
    mount_launch(&stack.dagster, "run-1").await;
    let upload = stack.upload(&bayu, "stock.xlsx", b"id,name\n1,a\n").await;
    let id = id_of(&upload);

    let reply = stack.ingest(&bayu, &id, ingest_body("stock_raw")).await;

    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text);
    assert_eq!(
        reply.json["upload"]["parseOptions"],
        json!({ "encoding": "utf-8", "delimiter": ",", "headerRow": 0 }),
        "no sheet"
    );
    let launches = requests_containing(&stack.dagster, "launchRun").await;
    assert_eq!(
        launches[0]["variables"]["cfg"]["ops"]["ingest_uploaded_file"]["config"]["storage_keys"],
        json!([format!("uploads/{GROUP}/{id}.xlsx")])
    );
    assert_eq!(stack.bucket.keys().len(), 1, "nothing was converted");
    // And a text upload still needs its dialect.
    let missing = stack
        .ingest(&bayu, &id, workbook_ingest_body("stock_raw", None))
        .await;
    assert_eq!(missing.status, StatusCode::BAD_REQUEST);
    assert_eq!(missing.json["error"], "encoding is required.");
}

#[tokio::test]
async fn deleting_a_workbook_upload_removes_the_original_and_the_converted_sheet() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    mount_free_table(&stack.ch).await;
    // The launch is refused, which settles the claim as failed and leaves the
    // converted object in place: the state a delete must clean up.
    mount_launch_refused(&stack.dagster).await;
    let upload = stack
        .upload(&bayu, "stock.xlsx", &workbook("stock.xlsx"))
        .await;
    let id = id_of(&upload);
    let reply = stack
        .ingest(&bayu, &id, workbook_ingest_body("stock_raw", None))
        .await;
    assert_eq!(
        reply.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        reply.text
    );
    assert_eq!(stack.bucket.keys().len(), 2);

    let gone = stack
        .send(
            &bayu,
            "DELETE",
            &format!("/api/uploads/{id}"),
            Payload::None,
        )
        .await;

    assert_eq!(gone.status, StatusCode::NO_CONTENT, "{}", gone.text);
    assert!(stack.bucket.keys().is_empty(), "{:?}", stack.bucket.keys());
}

// ════════════════════════════════════════════════════════════════════════
// Parquet files (Q2 of section 11 of
// docs/superpowers/plans/2026-10-07-upload-excel.md)
// ════════════════════════════════════════════════════════════════════════

const UNREADABLE_PARQUET: &str = "This file could not be opened as a Parquet file.";
const EMPTY_PARQUET: &str = "That Parquet file has no rows, so there is nothing to load.";

/// The columns `orders.parquet` declares, as the API lists them.
fn orders_columns() -> Value {
    json!([
        { "name": "sku", "type": "string" },
        { "name": "name", "type": "string" },
        { "name": "qty", "type": "int64" },
        { "name": "price", "type": "decimal(10,2)" },
        { "name": "received", "type": "date" },
    ])
}

/// The ingest body of a Parquet file: no encoding, no delimiter, no header
/// row, no sheet.
fn parquet_ingest_body(table: &str) -> Value {
    json!({ "bronzeTable": table })
}

#[tokio::test]
async fn a_parquet_file_is_stored_as_it_arrived_and_the_answer_lists_its_columns() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    let bytes = parquet_file("orders.parquet");
    let upload = stack.upload(&bayu, "orders.parquet", &bytes).await;
    let id = id_of(&upload);
    assert_eq!(upload["status"], "uploaded");
    assert_eq!(
        upload["parquet"],
        json!({ "columns": orders_columns(), "rows": 3 })
    );
    assert!(upload.get("workbook").is_none());
    assert_eq!(
        stack
            .bucket
            .get(&format!("uploads/{GROUP}/{id}.parquet"))
            .unwrap(),
        bytes,
        "byte for byte"
    );
    let text = stack.upload(&bayu, "a.csv", b"a,b\n1,2\n").await;
    assert!(
        text.get("parquet").is_none(),
        "a text file has no columns list"
    );
}

#[tokio::test]
async fn a_parquet_file_that_cannot_be_read_or_is_not_named_like_one_is_refused_with_its_reason() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    let orders = parquet_file("orders.parquet");
    assert_eq!(
        refused_upload(&stack, &bayu, "orders.csv", &orders).await,
        "This looks like a Parquet file, but its name does not end in .parquet. Rename it to end in .parquet and upload it again."
    );
    for (name, bytes) in [
        ("cut.parquet", orders[..orders.len() / 2].to_vec()),
        ("garbage.parquet", b"PAR1\x15\x04\x15\x30rest".to_vec()),
    ] {
        assert_eq!(
            refused_upload(&stack, &bayu, name, &bytes).await,
            UNREADABLE_PARQUET,
            "{name}"
        );
    }
    assert_eq!(
        refused_upload(
            &stack,
            &bayu,
            "locked.parquet",
            b"PARE\x00\x01\x02\x03 an encrypted footer PARE"
        )
        .await,
        "This Parquet file is encrypted, so it cannot be read. Upload an unencrypted copy."
    );
    assert_eq!(
        refused_upload(
            &stack,
            &bayu,
            "blob.parquet",
            &parquet_file("binary_column.parquet")
        )
        .await,
        "Column \"blob\" holds binary values, which cannot be loaded as text. Remove that column and upload the file again."
    );
    assert_eq!(
        refused_upload(
            &stack,
            &bayu,
            "tags.parquet",
            &parquet_file("nested_column.parquet")
        )
        .await,
        "Column \"tags\" holds list values, which cannot be loaded as text. Remove that column and upload the file again."
    );
    assert!(
        stack.bucket.keys().is_empty(),
        "nothing was stored for a refused file"
    );
    assert!(stack.audit_actions().await.is_empty());
}

#[tokio::test]
async fn a_parquet_preview_shows_its_rows_as_text_and_its_declared_types() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    let upload = stack
        .upload(&bayu, "orders.parquet", &parquet_file("orders.parquet"))
        .await;
    let uri = format!("/api/uploads/{}/preview", id_of(&upload));

    let reply = stack.get(&bayu, &uri).await;

    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text);
    assert_eq!(
        reply.json["columns"],
        json!(["sku", "name", "qty", "price", "received"])
    );
    assert_eq!(
        reply.json["rows"],
        json!([
            ["A-100", "Bolt, hex M6", "10", "1.50", "2025-09-24"],
            ["A-200", "Washer", "250", "0.05", "2025-09-25"],
            [
                "A-300",
                "Flange DN50 \u{2013} steel",
                "4",
                "12.00",
                "2025-10-01"
            ],
        ])
    );
    assert_eq!(reply.json["truncated"], false);
    assert_eq!(
        reply.json["using"],
        json!({ "encoding": "utf-8", "delimiter": ",", "headerRow": 0 })
    );
    assert_eq!(
        reply.json["parquet"],
        json!({ "columns": orders_columns(), "rows": 3 })
    );
    assert!(reply.json.get("workbook").is_none());

    // The dialect, the header row and the sheet mean nothing for a Parquet
    // file: a valid value is ignored, an invalid one is refused as for text.
    let ignored = stack
        .get(
            &bayu,
            &format!("{uri}?encoding=utf-16&delimiter=%3B&headerRow=2&sheet=Nope"),
        )
        .await;
    assert_eq!(ignored.status, StatusCode::OK, "{}", ignored.text);
    assert_eq!(ignored.json["columns"], reply.json["columns"]);
    assert_eq!(ignored.json["using"], reply.json["using"]);
    let refused = stack.get(&bayu, &format!("{uri}?encoding=latin-1")).await;
    assert_eq!(refused.status, StatusCode::BAD_REQUEST);
    assert_eq!(refused.json["error"], "encoding must be utf-8 or utf-16.");
}

#[tokio::test]
async fn a_parquet_preview_of_a_file_with_no_rows_shows_the_columns_alone() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    let upload = stack
        .upload(&bayu, "empty.parquet", &parquet_file("empty.parquet"))
        .await;

    let reply = stack
        .get(&bayu, &format!("/api/uploads/{}/preview", id_of(&upload)))
        .await;

    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text);
    assert_eq!(
        reply.json["columns"],
        json!(["sku", "name", "qty", "price", "received"])
    );
    assert_eq!(reply.json["rows"], json!([]));
    assert_eq!(reply.json["parquet"]["rows"], 0);
}

#[tokio::test]
async fn a_text_file_named_like_a_parquet_file_previews_and_loads_as_text() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    mount_free_table(&stack.ch).await;
    mount_launch(&stack.dagster, "run-1").await;
    let text = stack
        .upload(&bayu, "orders.parquet", b"id,name\n1,a\n")
        .await;
    let id = id_of(&text);
    assert!(text.get("parquet").is_none());

    let preview = stack
        .get(&bayu, &format!("/api/uploads/{id}/preview"))
        .await;
    assert_eq!(preview.status, StatusCode::OK, "{}", preview.text);
    assert_eq!(preview.json["columns"], json!(["id", "name"]));
    assert!(preview.json.get("parquet").is_none());

    let reply = stack.ingest(&bayu, &id, ingest_body("orders_raw")).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text);
    assert_eq!(stack.bucket.keys().len(), 1, "nothing was converted");
    // And a text upload still needs its dialect.
    let missing = stack
        .ingest(&bayu, &id, parquet_ingest_body("orders_raw"))
        .await;
    assert_eq!(missing.status, StatusCode::BAD_REQUEST);
    assert_eq!(missing.json["error"], "encoding is required.");
}

#[tokio::test]
async fn a_parquet_ingest_converts_nothing_and_launches_the_job_on_the_original_as_parquet() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    mount_free_table(&stack.ch).await;
    mount_launch(&stack.dagster, "run-1").await;
    let bytes = parquet_file("orders.parquet");
    let upload = stack.upload(&bayu, "orders.parquet", &bytes).await;
    let id = id_of(&upload);

    // A retry may send back the dialect and a header row it recorded: none of
    // it is read for a Parquet file.
    let mut body = parquet_ingest_body("orders_raw");
    body["headerRow"] = json!(4);
    body["encoding"] = json!("utf-16");
    let reply = stack.ingest(&bayu, &id, body).await;

    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text);
    assert_eq!(reply.json["runId"], "run-1");
    assert_eq!(
        reply.json["upload"]["parseOptions"],
        json!({ "encoding": "utf-8", "delimiter": ",", "headerRow": 0 }),
        "no sheet"
    );
    // `DATA-17` T2.2: the API converts nothing; the job reads the file itself.
    let original = format!("uploads/{GROUP}/{id}.parquet");
    assert_eq!(
        stack.bucket.keys(),
        std::slice::from_ref(&original),
        "no converted text"
    );
    assert_eq!(
        stack.bucket.get(&original).unwrap(),
        bytes,
        "the original is untouched"
    );

    let launches = requests_containing(&stack.dagster, "launchRun").await;
    assert_eq!(launches.len(), 1);
    assert_eq!(
        launches[0]["variables"]["cfg"],
        json!({ "ops": { "ingest_uploaded_file": { "config": {
            "upload_id": id,
            "storage_keys": [original],
            "format": "parquet",
            "compression": "none",
            "max_bytes": UNPACKED_CAP,
            "bronze_table_name": "orders_raw",
            "load_mode": "replace",
            "encoding": "utf-8",
            "delimiter": ",",
            "header_row": 0,
        "columns": [],
        "on_bad_value": "stop",
        } } } }),
        "the job's own configuration: the original object, read as Parquet"
    );
    let audit = stack.audit_actions().await;
    let ingest = audit.iter().find(|a| a.0 == "upload.ingest").unwrap();
    assert_eq!(
        ingest.3,
        json!({
            "fileName": "orders.parquet", "sizeBytes": bytes.len(),
            "table": "orders_raw", "mode": "replace", "format": "parquet"
        })
    );
}

#[tokio::test]
async fn a_parquet_ingest_that_cannot_convert_claims_nothing_and_launches_nothing() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    mount_free_table(&stack.ch).await;
    mount_launch(&stack.dagster, "run-1").await;
    let upload = stack
        .upload(&bayu, "empty.parquet", &parquet_file("empty.parquet"))
        .await;
    let id = id_of(&upload);

    let reply = stack
        .ingest(&bayu, &id, parquet_ingest_body("orders_raw"))
        .await;

    assert_eq!(reply.status, StatusCode::BAD_REQUEST, "{}", reply.text);
    assert_eq!(reply.json["error"], EMPTY_PARQUET);
    assert!(stack.dagster.received_requests().await.unwrap().is_empty());
    assert_eq!(stack.status_of(&id).await.0, "uploaded");
    assert!(stack.claims().await.is_empty());
    assert_eq!(
        stack.bucket.keys(),
        [format!("uploads/{GROUP}/{id}.parquet")],
        "no converted object for a file that could not be converted"
    );
}

#[tokio::test]
async fn deleting_a_parquet_upload_removes_the_original() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    mount_free_table(&stack.ch).await;
    // The launch is refused, which settles the claim as failed: the state a
    // delete must clean up. (Nothing is converted any more, so only the
    // original is stored.)
    mount_launch_refused(&stack.dagster).await;
    let upload = stack
        .upload(&bayu, "orders.parquet", &parquet_file("orders.parquet"))
        .await;
    let id = id_of(&upload);
    let reply = stack
        .ingest(&bayu, &id, parquet_ingest_body("orders_raw"))
        .await;
    assert_eq!(
        reply.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        reply.text
    );
    assert_eq!(stack.bucket.keys().len(), 1);

    let gone = stack
        .send(
            &bayu,
            "DELETE",
            &format!("/api/uploads/{id}"),
            Payload::None,
        )
        .await;

    assert_eq!(gone.status, StatusCode::NO_CONTENT, "{}", gone.text);
    assert!(stack.bucket.keys().is_empty(), "{:?}", stack.bucket.keys());
}

// ════════════════════════════════════════════════════════════════════════
// Chunked, resumable uploads (DATA-17 T1.3)
// ════════════════════════════════════════════════════════════════════════

const MIB: usize = 1024 * 1024;
const STILL_RECEIVING: &str = "This upload is still being received, so it cannot be used yet.";
const NOT_RECEIVING: &str = "This upload is not receiving pieces.";
const EMPTY_FILE: &str = "The file is empty.";
const SESSION_TOO_LARGE: &str = "The file is larger than the 2 GB limit.";
const OTHER_BINARY: &str = "This is not a file this upload reads. Upload CSV, TSV, JSON, JSON Lines, Avro, Parquet or Excel (.xls, .xlsx) files, a .gz of a text or JSON file, or a .zip of such files.";

/// A piece that takes 1.5 s to store, so requests sent together overlap.
struct SlowPart(Bucket);
impl Respond for SlowPart {
    fn respond(&self, request: &MockRequest) -> ResponseTemplate {
        PutPart(self.0.clone())
            .respond(request)
            .set_delay(Duration::from_millis(1_500))
    }
}

impl Stack {
    async fn open_session(&self, who: &Caller, name: &str, size: usize) -> Reply {
        self.send(
            who,
            "POST",
            "/api/uploads/sessions",
            Payload::Json(json!({ "filename": name, "sizeBytes": size })),
        )
        .await
    }

    /// Open a session; the answer must be 201. Returns the upload's id.
    async fn session(&self, who: &Caller, name: &str, size: usize) -> String {
        let reply = self.open_session(who, name, size).await;
        assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.text);
        reply.json["upload"]["id"].as_str().unwrap().to_owned()
    }

    async fn put_part(&self, who: &Caller, id: &str, number: &str, bytes: &[u8]) -> Reply {
        self.send(
            who,
            "PUT",
            &format!("/api/uploads/{id}/parts/{number}"),
            Payload::Raw("application/octet-stream", bytes.to_vec()),
        )
        .await
    }

    async fn complete(&self, who: &Caller, id: &str) -> Reply {
        self.send(
            who,
            "POST",
            &format!("/api/uploads/{id}/complete"),
            Payload::None,
        )
        .await
    }

    /// Open a session for `bytes`, send every piece in order and complete it;
    /// the answer must be 200. Returns the completed upload's JSON.
    async fn upload_in_pieces(&self, who: &Caller, name: &str, bytes: &[u8]) -> Value {
        let id = self.session(who, name, bytes.len()).await;
        for (index, piece) in bytes.chunks(8 * MIB).enumerate() {
            let sent = self
                .put_part(who, &id, &(index + 1).to_string(), piece)
                .await;
            assert_eq!(sent.status, StatusCode::OK, "{}", sent.text);
        }
        let done = self.complete(who, &id).await;
        assert_eq!(done.status, StatusCode::OK, "{}", done.text);
        done.json
    }

    async fn received(&self, who: &Caller, id: &str) -> Value {
        let reply = self.get(who, &format!("/api/uploads/{id}/parts")).await;
        assert_eq!(reply.status, StatusCode::OK, "{}", reply.text);
        reply.json["received"].clone()
    }

    /// The store the sweep runs against: the same fake bucket.
    async fn upload_store(&self) -> lakehouse_api::upload_store::UploadStore {
        let config = lakehouse_api::config::Config::from_map(&HashMap::from([
            ("RUSTFS_S3_ENDPOINT".to_owned(), self.s3.uri()),
            (
                "RUSTFS_ACCESS_KEY_SECRET_REF".to_owned(),
                "env:CARGO_PKG_NAME".to_owned(),
            ),
            (
                "RUSTFS_SECRET_KEY_SECRET_REF".to_owned(),
                "env:CARGO_PKG_VERSION".to_owned(),
            ),
        ]))
        .unwrap();
        lakehouse_api::upload_store::UploadStore::connect(&config)
            .await
            .unwrap()
    }

    async fn row_count(&self, id: &str) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM file_upload WHERE id = $1")
            .bind(id)
            .fetch_one(&self.app.pool)
            .await
            .unwrap()
    }
}

/// A file of three pieces (8 MiB, 8 MiB, the rest) sent out of order arrives
/// whole, and the answer has the shape of `POST /api/uploads`: the upload,
/// `uploaded`, with none of the server-only fields.
#[tokio::test]
async fn a_file_sent_in_pieces_out_of_order_is_assembled_and_answered_like_a_whole_upload() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    let bytes = csv_of_size(17 * MIB);

    let opened = stack
        .open_session(&bayu, "big stock.csv", bytes.len())
        .await;
    assert_eq!(opened.status, StatusCode::CREATED, "{}", opened.text);
    assert_eq!(opened.json["partSize"], 8 * MIB);
    assert_eq!(opened.json["partCount"], 3);
    let upload = &opened.json["upload"];
    let id = id_of(upload);
    assert_eq!(upload["status"], "receiving");
    assert_eq!(upload["declaredSize"], bytes.len());
    assert_eq!(upload["originalFilename"], "big stock.csv");
    for hidden in [
        "storageKey",
        "multipartId",
        "partsDigest",
        "sha256",
        "tenantId",
    ] {
        assert!(upload.get(hidden).is_none(), "{hidden} leaked: {upload}");
    }
    assert_eq!(stack.bucket.open_multiparts().len(), 1);
    assert!(
        stack.bucket.keys().is_empty(),
        "nothing is stored before completion"
    );

    for number in [3_usize, 1, 2] {
        let piece = &bytes[(number - 1) * 8 * MIB..(number * 8 * MIB).min(bytes.len())];
        let sent = stack.put_part(&bayu, &id, &number.to_string(), piece).await;
        assert_eq!(sent.status, StatusCode::OK, "{}", sent.text);
        assert_eq!(sent.json["partNumber"], number);
        assert_eq!(sent.json["sizeBytes"], piece.len());
    }
    assert_eq!(stack.received(&bayu, &id).await, json!([1, 2, 3]));

    let done = stack.complete(&bayu, &id).await;
    assert_eq!(done.status, StatusCode::OK, "{}", done.text);
    assert_eq!(done.json["id"], id.as_str());
    assert_eq!(done.json["status"], "uploaded");
    assert_eq!(done.json["sizeBytes"], bytes.len());
    for hidden in [
        "storageKey",
        "multipartId",
        "partsDigest",
        "sha256",
        "tenantId",
    ] {
        assert!(
            done.json.get(hidden).is_none(),
            "{hidden} leaked: {}",
            done.json
        );
    }
    assert!(done.json.get("duplicateOf").is_none());

    let keys = stack.bucket.keys();
    assert_eq!(keys.len(), 1, "{keys:?}");
    assert!(
        keys[0].starts_with("uploads/")
            && std::path::Path::new(&keys[0])
                .extension()
                .is_some_and(|e| e == "csv"),
        "{keys:?}"
    );
    assert!(
        stack.bucket.get(&keys[0]).unwrap() == bytes,
        "the object is the file, byte for byte"
    );
    assert!(stack.bucket.open_multiparts().is_empty());
    let (digest, multipart, sha, pieces): (Option<Vec<u8>>, Option<String>, String, i64) =
        sqlx::query_as(
            "SELECT parts_digest, multipart_id, sha256, \
             (SELECT count(*) FROM file_upload_part WHERE upload_id = $1) \
             FROM file_upload WHERE id = $1",
        )
        .bind(&id)
        .fetch_one(&stack.app.pool)
        .await
        .unwrap();
    assert_eq!(digest.map(|d| d.len()), Some(32));
    assert_eq!(multipart, None);
    assert_eq!(
        sha, "",
        "no hash of the whole bytes exists and none is made up"
    );
    assert_eq!(pieces, 0, "the piece records are gone");

    // A completed upload is an ordinary one: it previews.
    let preview = stack
        .get(&bayu, &format!("/api/uploads/{id}/preview"))
        .await;
    assert_eq!(preview.status, StatusCode::OK, "{}", preview.text);
    let audit = stack.audit_actions().await;
    assert!(
        audit
            .iter()
            .any(|(action, _, resource, args)| action == "upload.create"
                && resource.as_deref() == Some(id.as_str())
                && args["chunked"] == true),
        "{audit:?}"
    );
}

/// A piece sent again replaces the earlier copy: the object holds the last.
#[tokio::test]
async fn a_piece_sent_again_replaces_the_earlier_copy() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    let right = b"id,name\n1,right\n".to_vec();
    let wrong = b"id,name\n1,wrong\n".to_vec();
    assert_eq!(right.len(), wrong.len());

    let id = stack.session(&bayu, "again.csv", right.len()).await;
    for bytes in [&wrong, &right] {
        let sent = stack.put_part(&bayu, &id, "1", bytes).await;
        assert_eq!(sent.status, StatusCode::OK, "{}", sent.text);
    }
    assert_eq!(
        stack.received(&bayu, &id).await,
        json!([1]),
        "one record, not two"
    );
    let rows: Vec<(String,)> =
        sqlx::query_as("SELECT sha256 FROM file_upload_part WHERE upload_id = $1")
            .bind(&id)
            .fetch_all(&stack.app.pool)
            .await
            .unwrap();
    assert_eq!(rows, [(sha256_hex(&right),)]);
    let done = stack.complete(&bayu, &id).await;
    assert_eq!(done.status, StatusCode::OK, "{}", done.text);
    let keys = stack.bucket.keys();
    assert_eq!(stack.bucket.get(&keys[0]).unwrap(), right);
}

/// Completing with a piece missing is a 409 that names the numbers; the
/// upload stays open, and sending the rest lets it complete.
#[tokio::test]
async fn completing_with_a_piece_missing_is_409_listing_it_and_the_upload_stays_open() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    let bytes = csv_of_size(17 * MIB);
    let id = stack.session(&bayu, "gap.csv", bytes.len()).await;
    for number in [1_usize, 3] {
        let piece = &bytes[(number - 1) * 8 * MIB..(number * 8 * MIB).min(bytes.len())];
        stack.put_part(&bayu, &id, &number.to_string(), piece).await;
    }

    let refused = stack.complete(&bayu, &id).await;
    assert_eq!(refused.status, StatusCode::CONFLICT, "{}", refused.text);
    assert_eq!(refused.json["error"], "Some pieces have not arrived: 2.");
    assert_eq!(stack.status_of(&id).await.0, "receiving");
    assert!(stack.bucket.keys().is_empty());
    assert_eq!(stack.bucket.open_multiparts().len(), 1);

    let sent = stack
        .put_part(&bayu, &id, "2", &bytes[8 * MIB..16 * MIB])
        .await;
    assert_eq!(sent.status, StatusCode::OK, "{}", sent.text);
    let done = stack.complete(&bayu, &id).await;
    assert_eq!(done.status, StatusCode::OK, "{}", done.text);
    // And it completes once: the upload is no longer receiving.
    let again = stack.complete(&bayu, &id).await;
    assert_eq!(again.status, StatusCode::CONFLICT);
    assert_eq!(again.json["error"], NOT_RECEIVING);
    let late = stack.put_part(&bayu, &id, "1", &bytes[..8 * MIB]).await;
    assert_eq!(late.status, StatusCode::CONFLICT);
    assert_eq!(late.json["error"], NOT_RECEIVING);
}

/// A piece of the wrong size, or with a number outside the upload, or a body
/// past the route's limit, is a 400 with a fixed sentence and records nothing.
#[tokio::test]
async fn a_piece_of_the_wrong_size_or_number_is_refused_and_records_nothing() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    let id = stack.session(&bayu, "sizes.csv", 17 * MIB).await;

    let size_rule = format!(
        "Every piece must be exactly {} bytes, except the last, which holds the rest of the file.",
        8 * MIB
    );
    for (number, len) in [("1", MIB), ("2", 8 * MIB + 1), ("3", 2 * MIB), ("3", 0)] {
        let reply = stack.put_part(&bayu, &id, number, &vec![b'a'; len]).await;
        assert_eq!(
            reply.status,
            StatusCode::BAD_REQUEST,
            "{number}/{len}: {}",
            reply.text
        );
        assert_eq!(reply.json["error"], size_rule.as_str(), "{number}/{len}");
    }
    for number in ["0", "4", "-1", "abc", "1.5", "99999999999999999999"] {
        let reply = stack
            .put_part(&bayu, &id, number, &vec![b'a'; 8 * MIB])
            .await;
        assert_eq!(
            reply.status,
            StatusCode::BAD_REQUEST,
            "{number}: {}",
            reply.text
        );
        assert_eq!(
            reply.json["error"], "That piece number is outside this upload.",
            "{number}"
        );
    }
    // Past the route's own limit (a piece plus 64 KiB): refused unread.
    let reply = stack
        .put_part(&bayu, &id, "1", &vec![b'a'; 8 * MIB + 128 * 1024])
        .await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST, "{}", reply.text);
    assert_eq!(reply.json["error"], "The piece could not be read.");

    assert_eq!(stack.received(&bayu, &id).await, json!([]));
    let stored_parts = stack
        .bucket
        .1
        .0
        .lock()
        .unwrap()
        .values()
        .map(|(_, parts)| parts.len())
        .sum::<usize>();
    assert_eq!(stored_parts, 0, "no refused piece reached the bucket");
}

/// A session is refused for no bytes, for more than the cap, and for a body
/// that is not `{filename, sizeBytes}`; nothing is opened in the bucket or
/// recorded. Exactly the cap is accepted.
#[tokio::test]
async fn a_session_over_the_cap_or_with_no_bytes_or_a_bad_body_is_refused_and_opens_nothing() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;

    let empty = stack.open_session(&bayu, "a.csv", 0).await;
    assert_eq!(empty.status, StatusCode::BAD_REQUEST);
    assert_eq!(empty.json["error"], EMPTY_FILE);
    let big = stack.open_session(&bayu, "a.csv", SESSION_CAP + 1).await;
    assert_eq!(big.status, StatusCode::BAD_REQUEST);
    assert_eq!(big.json["error"], SESSION_TOO_LARGE);
    let huge = stack.open_session(&bayu, "a.csv", usize::MAX).await;
    assert_eq!(huge.status, StatusCode::BAD_REQUEST);
    for body in [
        Payload::Json(json!({})),
        Payload::Json(json!({ "filename": "a.csv" })),
        Payload::Json(json!({ "filename": "a.csv", "sizeBytes": -5 })),
        Payload::Json(json!({ "filename": "a.csv", "sizeBytes": "10" })),
        Payload::Json(json!([1, 2])),
        Payload::Raw("application/json", b"not json".to_vec()),
        Payload::None,
    ] {
        let reply = stack
            .send(&bayu, "POST", "/api/uploads/sessions", body)
            .await;
        assert_eq!(reply.status, StatusCode::BAD_REQUEST, "{}", reply.text);
        assert_eq!(
            reply.json["error"],
            "The request body must be a JSON object with a filename and a sizeBytes (a whole number)."
        );
    }
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM file_upload")
        .fetch_one(&stack.app.pool)
        .await
        .unwrap();
    assert_eq!(rows, 0);
    assert!(stack.bucket.open_multiparts().is_empty());

    let at_cap = stack.open_session(&bayu, "a.csv", SESSION_CAP).await;
    assert_eq!(at_cap.status, StatusCode::CREATED, "{}", at_cap.text);
    // 2 GiB in pieces of 8 MiB.
    assert_eq!(at_cap.json["partCount"], 256);
}

/// Another tenant's session answers the 404 an unknown id gets, on every
/// route, and nothing of it is touched; an Analyst (no `connector:manage`)
/// cannot open one.
#[tokio::test]
async fn another_tenants_session_is_404_on_every_route_and_an_analyst_cannot_open_one() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    let andi = stack.andi().await;
    let id = stack.session(&bayu, "mine.csv", 10).await;

    for (method, uri) in [
        ("PUT", format!("/api/uploads/{id}/parts/1")),
        ("GET", format!("/api/uploads/{id}/parts")),
        ("POST", format!("/api/uploads/{id}/complete")),
        ("DELETE", format!("/api/uploads/{id}")),
    ] {
        let payload = Payload::Raw("application/octet-stream", b"a,b\n1,2\n3\n".to_vec());
        let reply = stack.send(&andi, method, &uri, payload).await;
        assert_eq!(
            reply.status,
            StatusCode::NOT_FOUND,
            "{method} {uri}: {}",
            reply.text
        );
        assert_eq!(reply.json["error"], "Upload not found.");
    }
    assert_eq!(stack.status_of(&id).await.0, "receiving");
    assert_eq!(stack.received(&bayu, &id).await, json!([]));
    assert_eq!(stack.bucket.open_multiparts().len(), 1);

    let sari = stack.caller("sari@meridian.example", Some(RETAIL)).await;
    let reply = stack.open_session(&sari, "x.csv", 10).await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN, "{}", reply.text);
    assert_eq!(
        stack.bucket.open_multiparts().len(),
        1,
        "the refusal opened nothing"
    );
}

/// A session cannot be previewed or loaded until it is complete; it is listed
/// and readable like any upload, with its declared size.
#[tokio::test]
async fn an_upload_still_being_received_cannot_be_previewed_or_loaded() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    let id = stack.session(&bayu, "later.csv", 12).await;

    let preview = stack
        .get(&bayu, &format!("/api/uploads/{id}/preview"))
        .await;
    assert_eq!(preview.status, StatusCode::CONFLICT, "{}", preview.text);
    assert_eq!(preview.json["error"], STILL_RECEIVING);
    let ingest = stack.ingest(&bayu, &id, ingest_body("later_raw")).await;
    assert_eq!(ingest.status, StatusCode::CONFLICT, "{}", ingest.text);
    assert_eq!(ingest.json["error"], STILL_RECEIVING);
    assert!(stack.dagster.received_requests().await.unwrap().is_empty());
    assert!(
        stack.claims().await.is_empty(),
        "a refused load claims no table"
    );

    let one = stack.get(&bayu, &format!("/api/uploads/{id}")).await;
    assert_eq!(one.status, StatusCode::OK);
    assert_eq!(one.json["status"], "receiving");
    assert_eq!(one.json["declaredSize"], 12);
    let listed = stack.get(&bayu, "/api/uploads").await;
    assert!(
        listed
            .json
            .as_array()
            .unwrap()
            .iter()
            .any(|u| u["id"] == id.as_str()),
        "{}",
        listed.text
    );
}

/// Deleting a session aborts its multipart upload, and the pieces go with the
/// row.
#[tokio::test]
async fn deleting_a_session_aborts_its_multipart_upload() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    let id = stack.session(&bayu, "drop.csv", 10).await;
    stack.put_part(&bayu, &id, "1", b"a,b\n1,2\n3\n").await;
    assert_eq!(stack.bucket.open_multiparts().len(), 1);

    let reply = stack
        .send(
            &bayu,
            "DELETE",
            &format!("/api/uploads/{id}"),
            Payload::None,
        )
        .await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT, "{}", reply.text);
    assert!(stack.bucket.open_multiparts().is_empty());
    assert!(stack.bucket.keys().is_empty());
    assert_eq!(stack.row_count(&id).await, 0);
    let late = stack.put_part(&bayu, &id, "1", b"a,b\n1,2\n3\n").await;
    assert_eq!(late.status, StatusCode::NOT_FOUND);
}

/// A file the rules of `POST /api/uploads` refuse is refused at completion
/// with the same sentence, and removed: object and row.
#[tokio::test]
async fn a_binary_that_is_not_a_supported_file_is_refused_at_completion_and_removed() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    let bytes: Vec<u8> = (0..3000_u32)
        .map(|n| u8::try_from(n % 251).unwrap())
        .collect();
    let id = stack.session(&bayu, "data.bin", bytes.len()).await;
    stack.put_part(&bayu, &id, "1", &bytes).await;

    let reply = stack.complete(&bayu, &id).await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST, "{}", reply.text);
    assert_eq!(reply.json["error"], OTHER_BINARY);
    assert!(
        stack.bucket.keys().is_empty(),
        "the refused file was removed"
    );
    assert_eq!(stack.row_count(&id).await, 0);
}

/// A workbook and a Parquet file complete with their facts, as they do through
/// `POST /api/uploads`.
#[tokio::test]
async fn a_workbook_and_a_parquet_file_complete_with_their_facts() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;

    let sheet = stack
        .upload_in_pieces(&bayu, "stock.xlsx", &workbook("stock.xlsx"))
        .await;
    assert_eq!(sheet["workbook"]["defaultSheet"], "Stock", "{sheet}");
    assert!(sheet.get("parquet").is_none());

    let table = stack
        .upload_in_pieces(&bayu, "orders.parquet", &parquet_file("orders.parquet"))
        .await;
    assert!(
        table["parquet"]["columns"]
            .as_array()
            .is_some_and(|c| !c.is_empty()),
        "{table}"
    );
    assert!(table.get("workbook").is_none());

    // Not named for what it is: refused with the whole-upload sentence.
    let id = stack
        .session(&bayu, "orders.csv", parquet_file("orders.parquet").len())
        .await;
    stack
        .put_part(&bayu, &id, "1", &parquet_file("orders.parquet"))
        .await;
    let reply = stack.complete(&bayu, &id).await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST, "{}", reply.text);
    assert!(
        reply.json["error"].as_str().unwrap().contains("Parquet"),
        "{}",
        reply.text
    );
    assert_eq!(stack.row_count(&id).await, 0);
}

/// The same pieces uploaded twice: the second completion names the first.
#[tokio::test]
async fn the_same_pieces_uploaded_twice_are_reported_as_a_duplicate() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    let andi = stack.andi().await;
    let bytes = csv_of_size(9 * MIB);

    let first = stack.upload_in_pieces(&bayu, "one.csv", &bytes).await;
    assert!(first.get("duplicateOf").is_none());
    let second = stack.upload_in_pieces(&bayu, "two.csv", &bytes).await;
    assert_eq!(second["duplicateOf"]["id"], first["id"], "{second}");
    // Another tenant's identical file is not reported to this one.
    let other = stack.upload_in_pieces(&andi, "three.csv", &bytes).await;
    assert!(other.get("duplicateOf").is_none(), "{other}");
}

/// `SEC-17` on the piece route: with four pieces of one user being received,
/// a fifth is a 429 with `Retry-After`, nothing of it is stored, and the
/// places are given back.
#[tokio::test]
async fn a_fifth_simultaneous_piece_of_one_user_is_429_with_retry_after() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    let bytes = b"a,b\n1,2\n".to_vec();
    let mut ids = Vec::new();
    for n in 0..5 {
        ids.push(
            stack
                .session(&bayu, &format!("f{n}.csv"), bytes.len())
                .await,
        );
    }
    // (A piece that takes 1.5 s to store, so the five overlap: `SlowPart`.)
    Mock::given(method("PUT"))
        .and(path_regex(OBJECT_PATH))
        .and(has_query("uploadId"))
        .respond_with(SlowPart(stack.bucket.clone()))
        .with_priority(1)
        .mount(&stack.s3)
        .await;

    let replies = tokio::join!(
        stack.put_part(&bayu, &ids[0], "1", &bytes),
        stack.put_part(&bayu, &ids[1], "1", &bytes),
        stack.put_part(&bayu, &ids[2], "1", &bytes),
        stack.put_part(&bayu, &ids[3], "1", &bytes),
        stack.put_part(&bayu, &ids[4], "1", &bytes),
    );
    let replies = [replies.0, replies.1, replies.2, replies.3, replies.4];
    let accepted = replies
        .iter()
        .filter(|r| r.status == StatusCode::OK)
        .count();
    let refused: Vec<&Reply> = replies
        .iter()
        .filter(|r| r.status == StatusCode::TOO_MANY_REQUESTS)
        .collect();
    assert_eq!(
        accepted,
        4,
        "{:?}",
        replies.iter().map(|r| &r.text).collect::<Vec<_>>()
    );
    assert_eq!(refused.len(), 1);
    assert_eq!(refused[0].json["error"], TOO_MANY_UPLOADS);
    assert_eq!(refused[0].retry_after.as_deref(), Some("5"));

    // The places were given back: the refused piece goes through now.
    let index = replies
        .iter()
        .position(|r| r.status == StatusCode::TOO_MANY_REQUESTS)
        .unwrap();
    let again = stack.put_part(&bayu, &ids[index], "1", &bytes).await;
    assert_eq!(again.status, StatusCode::OK, "{}", again.text);
}

/// A store that refuses answers a fixed sentence: nothing of its body reaches
/// the response, and nothing is recorded as received.
#[tokio::test]
async fn a_refusing_store_surfaces_a_fixed_sentence_on_every_chunked_route() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    let id = stack.session(&bayu, "down.csv", 10).await;
    break_bucket(&stack.s3).await;

    let part = stack.put_part(&bayu, &id, "1", b"a,b\n1,2\n3\n").await;
    assert_eq!(
        part.status,
        StatusCode::SERVICE_UNAVAILABLE,
        "{}",
        part.text
    );
    assert_eq!(
        part.json["error"],
        "Upload storage is unavailable (permission denied)."
    );
    let opened = stack.open_session(&bayu, "also.csv", 12).await;
    assert_eq!(
        opened.status,
        StatusCode::SERVICE_UNAVAILABLE,
        "{}",
        opened.text
    );
    for text in [&part.text, &opened.text] {
        assert!(!text.contains(MARKER), "{text}");
    }
    assert_eq!(stack.received(&bayu, &id).await, json!([]));
    assert_eq!(
        stack.row_count(&id).await,
        1,
        "the refused session opened no second row"
    );
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM file_upload")
        .fetch_one(&stack.app.pool)
        .await
        .unwrap();
    assert_eq!(rows, 1);
}

/// The sweep aborts and removes sessions older than the lifetime and leaves
/// the rest, and a session whose storage fails is left for the next pass.
#[tokio::test]
async fn the_sweep_aborts_and_removes_old_sessions_and_leaves_the_rest() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    let old = stack.session(&bayu, "old.csv", 10).await;
    let fresh = stack.session(&bayu, "fresh.csv", 10).await;
    let finished = stack
        .upload_in_pieces(&bayu, "done.csv", b"a,b\n1,2\n3\n")
        .await;
    stack.put_part(&bayu, &old, "1", b"a,b\n1,2\n3\n").await;
    sqlx::query(
        "UPDATE file_upload SET created_at = now() - interval '25 hours' WHERE id = ANY($1)",
    )
    .bind(vec![old.clone(), id_of(&finished)])
    .execute(&stack.app.pool)
    .await
    .unwrap();
    assert_eq!(stack.bucket.open_multiparts().len(), 2);
    let store = stack.upload_store().await;

    let outcome = lakehouse_api::upload_sweep::sweep_stale_sessions(&stack.app.pool, &store, 24)
        .await
        .unwrap();

    assert_eq!(outcome.removed, 1);
    assert_eq!(outcome.failed, 0);
    assert_eq!(stack.row_count(&old).await, 0);
    assert_eq!(stack.row_count(&fresh).await, 1, "a young session is kept");
    assert_eq!(
        stack.row_count(&id_of(&finished)).await,
        1,
        "a finished upload is never swept"
    );
    assert_eq!(
        stack.bucket.open_multiparts().len(),
        1,
        "only the old one was aborted"
    );
    assert_eq!(
        stack.bucket.keys().len(),
        1,
        "the finished file is still there"
    );

    // Storage down: the old session stays, counted as failed, for the next pass.
    sqlx::query("UPDATE file_upload SET created_at = now() - interval '30 hours' WHERE id = $1")
        .bind(&fresh)
        .execute(&stack.app.pool)
        .await
        .unwrap();
    break_bucket(&stack.s3).await;
    let outcome = lakehouse_api::upload_sweep::sweep_stale_sessions(&stack.app.pool, &store, 24)
        .await
        .unwrap();
    assert_eq!((outcome.removed, outcome.failed), (0, 1));
    assert_eq!(stack.row_count(&fresh).await, 1);
}

/// Phase 3 slice 1 review SHOULD-FIX 1: past 20 open sessions a user's next one
/// is a 429 with a fixed sentence and opens nothing in the bucket; another user
/// is not affected, and finishing or removing one frees a place.
#[tokio::test]
async fn a_twenty_first_open_session_of_one_user_is_429_and_opens_nothing() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    let andi = stack.andi().await;
    let mut ids = Vec::new();
    for n in 0..20 {
        ids.push(stack.session(&bayu, &format!("s{n}.csv"), 10).await);
    }
    assert_eq!(stack.bucket.open_multiparts().len(), 20);

    let refused = stack.open_session(&bayu, "one-more.csv", 10).await;
    assert_eq!(
        refused.status,
        StatusCode::TOO_MANY_REQUESTS,
        "{}",
        refused.text
    );
    assert_eq!(
        refused.json["error"],
        "Too many uploads are open. Finish or remove some of them, then try again."
    );
    assert_eq!(refused.retry_after.as_deref(), Some("30"));
    assert_eq!(
        stack.bucket.open_multiparts().len(),
        20,
        "the refusal opened nothing"
    );

    // Another user has their own twenty.
    stack.session(&andi, "andi.csv", 10).await;
    // Removing one frees a place; so does finishing one.
    let removed = stack
        .send(
            &bayu,
            "DELETE",
            &format!("/api/uploads/{}", ids[0]),
            Payload::None,
        )
        .await;
    assert_eq!(removed.status, StatusCode::NO_CONTENT);
    stack.session(&bayu, "again.csv", 10).await;
    assert_eq!(
        stack.open_session(&bayu, "full-again.csv", 10).await.status,
        StatusCode::TOO_MANY_REQUESTS
    );
    stack.put_part(&bayu, &ids[1], "1", b"a,b\n1,2\n3\n").await;
    assert_eq!(stack.complete(&bayu, &ids[1]).await.status, StatusCode::OK);
    stack.session(&bayu, "after-complete.csv", 10).await;
}

/// Phase 3 slice 1 review SHOULD-FIX 2: S3 completed the upload but the row
/// never left `receiving` (the database failed after it). The retry of
/// `complete` finds the multipart upload gone and the object in place at the
/// declared size, and finishes the upload; an object of another size is not
/// taken for the file.
#[tokio::test]
async fn complete_recovers_when_s3_assembled_the_file_but_the_row_was_not_updated() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    let bytes = b"id,name\n1,a\n2,b\n".to_vec();

    let id = stack.session(&bayu, "half.csv", bytes.len()).await;
    stack.put_part(&bayu, &id, "1", &bytes).await;
    // What S3 did on the first, half-successful attempt: the object is there and
    // the multipart upload is gone.
    let (key,): (String,) = sqlx::query_as("SELECT storage_key FROM file_upload WHERE id = $1")
        .bind(&id)
        .fetch_one(&stack.app.pool)
        .await
        .unwrap();
    stack.bucket.put(&key, &bytes);
    stack.bucket.1.0.lock().unwrap().clear();

    let done = stack.complete(&bayu, &id).await;
    assert_eq!(done.status, StatusCode::OK, "{}", done.text);
    assert_eq!(done.json["status"], "uploaded");
    assert_eq!(done.json["sizeBytes"], bytes.len());
    assert_eq!(stack.bucket.get(&key).unwrap(), bytes);

    // An object of the wrong size at the key is no proof: the failure stands.
    let other = stack.session(&bayu, "wrong.csv", bytes.len()).await;
    stack.put_part(&bayu, &other, "1", &bytes).await;
    let (other_key,): (String,) =
        sqlx::query_as("SELECT storage_key FROM file_upload WHERE id = $1")
            .bind(&other)
            .fetch_one(&stack.app.pool)
            .await
            .unwrap();
    stack.bucket.put(&other_key, b"short");
    stack.bucket.1.0.lock().unwrap().clear();
    let refused = stack.complete(&bayu, &other).await;
    assert_eq!(
        refused.status,
        StatusCode::SERVICE_UNAVAILABLE,
        "{}",
        refused.text
    );
    assert_eq!(stack.status_of(&other).await.0, "receiving");
}

// ════════════════════════════════════════════════════════════════════════
// Column types (DATA-18 T3.2)
// ════════════════════════════════════════════════════════════════════════

const TYPED_CSV: &str = "sku,qty,price,due,active,note\n\
0250161,3,1.50,2026-10-16,yes,a\n\
0250162,40,12,2026-10-17,no,b\n\
0250163,,0.25,2026-10-18,yes,\n";

fn spec_of(name: &str, kind: &str) -> Value {
    json!({ "source": name, "name": name, "type": kind })
}

fn typed_columns() -> Value {
    json!([
        spec_of("sku", "text"),
        spec_of("qty", "integer"),
        spec_of("price", "decimal"),
        { "source": "due", "name": "due", "type": "date", "dateFormat": "iso" },
        spec_of("active", "boolean"),
        spec_of("note", "text"),
    ])
}

/// The preview offers every column a name, a starting type and the question a
/// date may need: the guess of decisions 1 to 5, over the sample.
#[tokio::test]
async fn the_preview_guesses_each_columns_type_from_the_sample_and_asks_about_an_ambiguous_date() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    let id = id_of(&stack.upload(&bayu, "typed.csv", TYPED_CSV.as_bytes()).await);

    let preview = stack
        .get(&bayu, &format!("/api/uploads/{id}/preview"))
        .await;

    assert_eq!(preview.status, StatusCode::OK, "{}", preview.text);
    assert_eq!(preview.json["sampledRows"], 3);
    let kinds: Vec<(String, String)> = preview.json["columnSpecs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            (
                c["name"].as_str().unwrap().to_owned(),
                c["type"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    let expected: Vec<(String, String)> = [
        ("sku", "text"),
        ("qty", "integer"),
        ("price", "decimal"),
        ("due", "date"),
        ("active", "boolean"),
        ("note", "text"),
    ]
    .iter()
    .map(|(a, b)| ((*a).to_owned(), (*b).to_owned()))
    .collect();
    assert_eq!(
        kinds, expected,
        "a leading zero stays text; 1 and 0 and empties do not break a guess"
    );
    let due = &preview.json["columnSpecs"][3];
    assert_eq!(due["dateFormat"], "iso");
    assert_eq!(due["askDateOrder"], false);
    assert_eq!(preview.json["columnSpecs"][0]["source"], "sku");
    assert!(preview.json.get("fromTable").is_none());

    let ambiguous = id_of(
        &stack
            .upload(
                &bayu,
                "dates.csv",
                b"day,Order Total\n01/02/2026,1\n03/04/2026,2\n",
            )
            .await,
    );
    let asked = stack
        .get(&bayu, &format!("/api/uploads/{ambiguous}/preview"))
        .await;
    let day = &asked.json["columnSpecs"][0];
    assert_eq!(
        (day["type"].as_str(), day["askDateOrder"].as_bool()),
        (Some("date"), Some(true))
    );
    assert!(
        day.get("dateFormat").is_none(),
        "no reading until the person answers"
    );
    assert_eq!(
        asked.json["columnSpecs"][1]["name"], "order_total",
        "the load's own cleaning"
    );
}

/// A Parquet file's own types are the starting guess (decision 10).
#[tokio::test]
async fn a_parquet_previews_with_the_types_the_file_declares() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    let id = id_of(
        &stack
            .upload(&bayu, "orders.parquet", &parquet_file("orders.parquet"))
            .await,
    );
    let preview = stack
        .get(&bayu, &format!("/api/uploads/{id}/preview"))
        .await;
    assert_eq!(preview.status, StatusCode::OK, "{}", preview.text);
    let kinds: Vec<&str> = preview.json["columnSpecs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["type"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["text", "text", "integer", "decimal", "date"]);
    assert_eq!(preview.json["columnSpecs"][4]["dateFormat"], "iso");
}

/// `?table=` takes the columns from a table of the caller's tenant, and from no
/// other (decision 9).
#[tokio::test]
async fn a_preview_takes_a_tables_columns_only_from_the_callers_own_table() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    let andi = stack.andi().await;
    Mock::given(method("POST"))
        .and(body_string_contains("DESCRIBE TABLE"))
        .respond_with(ResponseTemplate::new(200).set_body_json(rows_body(&json!([
            { "name": "sku", "type": "Nullable(String)" },
            { "name": "qty", "type": "Nullable(Int64)" },
            { "name": "_ingested_at", "type": "DateTime64(6, 'UTC')" },
        ]))))
        .mount(&stack.ch)
        .await;
    stack.seed(GROUP, "up-seed", b"sku,qty\n1,2\n").await;
    stack.claim(GROUP, "stock_raw", "up-seed").await;
    let mine = id_of(
        &stack
            .upload(&bayu, "more.csv", b"id,count\nA,x\nB,y\n")
            .await,
    );
    let theirs = id_of(
        &stack
            .upload(&andi, "more.csv", b"id,count\nA,x\nB,y\n")
            .await,
    );

    let own = stack
        .get(
            &bayu,
            &format!("/api/uploads/{mine}/preview?table=stock_raw"),
        )
        .await;
    assert_eq!(own.json["fromTable"], true, "{}", own.text);
    let names: Vec<&str> = own.json["columnSpecs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        ["sku", "qty"],
        "the table's names, without the loader's column"
    );
    assert_eq!(own.json["columnSpecs"][1]["type"], "integer");
    assert_eq!(
        own.json["columnSpecs"][1]["guessedType"], "text",
        "the guess is still the file's"
    );

    // Another tenant's table is not readable through this route, and the answer does not say so.
    let other = stack
        .get(
            &andi,
            &format!("/api/uploads/{theirs}/preview?table=stock_raw"),
        )
        .await;
    assert_eq!(other.status, StatusCode::OK);
    assert!(other.json.get("fromTable").is_none());
    assert_eq!(other.json["columnSpecs"][0]["name"], "id");
}

/// The check counts every failing value of the sample and lists the first ones.
#[tokio::test]
async fn the_check_lists_the_values_that_do_not_fit_and_stores_nothing() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    let id = id_of(
        &stack
            .upload(
                &bayu,
                "qty.csv",
                b"qty,when\n1,2026-10-16\nabc,2026-13-01\n,2026-10-17\n7x,2026-10-18\n",
            )
            .await,
    );
    let url = format!("/api/uploads/{id}/check");
    let columns = json!([
        spec_of("qty", "integer"),
        { "source": "when", "name": "when_at", "type": "date", "dateFormat": "iso" },
    ]);
    let body = json!({ "encoding": "utf-8", "delimiter": ",", "headerRow": 0, "columns": columns });

    let reply = stack.send(&bayu, "POST", &url, Payload::Json(body)).await;

    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text);
    assert_eq!(reply.json["sampledRows"], 4);
    assert_eq!(reply.json["count"], 3);
    assert_eq!(
        reply.json["byColumn"],
        json!([{ "column": "qty", "count": 2 }, { "column": "when_at", "count": 1 }])
    );
    assert_eq!(
        reply.json["rows"][0],
        json!({ "row": 2, "column": "qty", "value": "abc" })
    );
    assert_eq!(stack.status_of(&id).await.0, "uploaded");

    for (columns, needle) in [
        (
            json!([spec_of("qty", "integer")]),
            "1 columns but the file has 2",
        ),
        (
            json!([spec_of("qty", "integer"), spec_of("qty", "text")]),
            "each name is used once",
        ),
        (
            json!([spec_of("Qty", "integer"), spec_of("b", "text")]),
            "lower-case",
        ),
        (
            json!([spec_of("qty", "integer"), spec_of("d", "date")]),
            "date reading",
        ),
    ] {
        let refused = stack
            .send(
                &bayu,
                "POST",
                &url,
                Payload::Json(json!({ "columns": columns })),
            )
            .await;
        assert_eq!(refused.status, StatusCode::BAD_REQUEST, "{}", refused.text);
        assert!(
            refused.json["error"].as_str().unwrap().contains(needle),
            "{}",
            refused.text
        );
    }
    let junk = stack
        .send(
            &bayu,
            "POST",
            &url,
            Payload::Raw("application/json", b"nope".to_vec()),
        )
        .await;
    assert_eq!(junk.status, StatusCode::BAD_REQUEST);

    let andi = stack.andi().await;
    let theirs = stack
        .send(&andi, "POST", &url, Payload::Json(json!({ "columns": [] })))
        .await;
    assert_eq!(theirs.status, StatusCode::NOT_FOUND);
    let receiving = stack.session(&bayu, "later.csv", 10).await;
    let early = stack
        .send(
            &bayu,
            "POST",
            &format!("/api/uploads/{receiving}/check"),
            Payload::Json(json!({ "columns": [] })),
        )
        .await;
    assert_eq!(early.status, StatusCode::CONFLICT);
}

/// An ingest with a column list is validated, sent to the job whole, and kept on
/// the upload; one without a list is every column text, as before.
#[tokio::test]
async fn an_ingest_with_a_column_list_sends_it_to_the_job_and_keeps_it_on_the_upload() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    mount_free_table(&stack.ch).await;
    mount_launch(&stack.dagster, "run-1").await;
    let id = id_of(&stack.upload(&bayu, "typed.csv", TYPED_CSV.as_bytes()).await);

    let mut body = ingest_body("typed_raw");
    body["columns"] = typed_columns();
    let missing = stack.ingest(&bayu, &id, body.clone()).await;
    assert_eq!(missing.status, StatusCode::BAD_REQUEST, "{}", missing.text);
    assert!(
        missing.json["error"]
            .as_str()
            .unwrap()
            .contains("onBadValue")
    );
    body["onBadValue"] = json!("later");
    assert_eq!(
        stack.ingest(&bayu, &id, body.clone()).await.status,
        StatusCode::BAD_REQUEST
    );
    let mut stray = ingest_body("typed_raw");
    stray["onBadValue"] = json!("stop");
    assert_eq!(
        stack.ingest(&bayu, &id, stray).await.status,
        StatusCode::BAD_REQUEST
    );
    assert!(
        stack.dagster.received_requests().await.unwrap().is_empty(),
        "nothing was launched"
    );
    assert!(stack.claims().await.is_empty(), "and nothing was claimed");

    body["onBadValue"] = json!("null");
    let reply = stack.ingest(&bayu, &id, body).await;

    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text);
    let launches = requests_containing(&stack.dagster, "launchRun").await;
    let config = &launches[0]["variables"]["cfg"]["ops"]["ingest_uploaded_file"]["config"];
    assert_eq!(config["on_bad_value"], "null");
    assert_eq!(
        config["columns"][1],
        json!({ "source": "qty", "name": "qty", "type": "integer", "date_format": "", "timezone": false })
    );
    assert_eq!(config["columns"][3]["date_format"], "iso");
    let upload = stack.get(&bayu, &format!("/api/uploads/{id}")).await;
    assert_eq!(upload.json["columns"], typed_columns());
    assert_eq!(upload.json["onBadValue"], "null");
}

/// Adding to a table that exists keeps its names and types (decision 9): a
/// different list is refused with the differences, and so is no list for a typed
/// table; replacing may change the types.
#[tokio::test]
async fn adding_to_a_typed_table_keeps_its_columns_and_replacing_may_change_them() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    mount_launch(&stack.dagster, "run-1").await;
    Mock::given(method("POST"))
        .and(body_string_contains("DESCRIBE TABLE"))
        .respond_with(ResponseTemplate::new(200).set_body_json(rows_body(&json!([
            { "name": "sku", "type": "Nullable(String)" },
            { "name": "qty", "type": "Nullable(Int64)" },
            { "name": "_ingested_at", "type": "DateTime64(6, 'UTC')" },
        ]))))
        .mount(&stack.ch)
        .await;
    Mock::given(method("POST"))
        .and(body_string_contains("dataset_catalog"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(rows_body(&json!([{ "slug": "stock-raw" }]))),
        )
        .mount(&stack.ch)
        .await;
    stack.seed(GROUP, "up-seed", b"sku,qty\n1,2\n").await;
    stack.claim(GROUP, "stock_raw", "up-seed").await;
    let id = id_of(
        &stack
            .upload(&bayu, "more.csv", b"sku,qty\nA,1\nB,2\n")
            .await,
    );

    let mut append = ingest_body("stock_raw");
    append["mode"] = json!("append");
    append["onBadValue"] = json!("stop");
    append["columns"] = json!([spec_of("sku", "text"), spec_of("qty", "text")]);
    let differs = stack.ingest(&bayu, &id, append.clone()).await;
    assert_eq!(differs.status, StatusCode::BAD_REQUEST, "{}", differs.text);
    assert!(
        differs.json["error"]
            .as_str()
            .unwrap()
            .contains("column qty is Integer in the table and Text in the file")
    );

    let mut bare = ingest_body("stock_raw");
    bare["mode"] = json!("append");
    let typed = stack.ingest(&bayu, &id, bare).await;
    assert_eq!(typed.status, StatusCode::BAD_REQUEST, "{}", typed.text);
    assert!(
        typed.json["error"]
            .as_str()
            .unwrap()
            .contains("typed columns")
    );
    assert!(stack.dagster.received_requests().await.unwrap().is_empty());

    append["columns"] = json!([spec_of("sku", "text"), spec_of("qty", "integer")]);
    assert_eq!(
        stack.ingest(&bayu, &id, append).await.status,
        StatusCode::OK
    );

    let again = id_of(&stack.upload(&bayu, "again.csv", b"sku,qty\nA,x\n").await);
    // (Another table, so the append above, still loading, is not in the way.)
    stack.claim(GROUP, "other_raw", "up-seed").await;
    let mut replace = ingest_body("other_raw");
    replace["onBadValue"] = json!("stop");
    replace["columns"] = json!([spec_of("sku", "text"), spec_of("qty", "text")]);
    let changed = stack.ingest(&bayu, &again, replace).await;
    assert_eq!(
        changed.status,
        StatusCode::OK,
        "replacing may change the types: {}",
        changed.text
    );
}

// ════════════════════════════════════════════════════════════════════════
// The report of values that did not fit (DATA-18 T3.4)
// ════════════════════════════════════════════════════════════════════════

const BAD_VALUES_REASON: &str = "Some values do not fit their column's type.";

fn report_bytes(count: u64) -> Vec<u8> {
    json!({
        "count": count,
        "byColumn": [{ "column": "qty", "count": count }],
        "rows": [{ "row": 2, "column": "qty", "value": "abc" }],
        "secret": "never copied",
    })
    .to_string()
    .into_bytes()
}

async fn seed_typed_loading(stack: &Stack, id: &str) {
    stack
        .seed_loading(GROUP, id, "stock_raw", Some("run-1"), "1 hour")
        .await;
    sqlx::query("UPDATE file_upload SET column_spec = $2, on_bad_value = 'null' WHERE id = $1")
        .bind(id)
        .bind(sqlx::types::Json(
            json!([{ "source": "qty", "name": "qty", "type": "integer" }]),
        ))
        .execute(&stack.app.pool)
        .await
        .unwrap();
}

/// When a typed load is settled, the job's report is kept on the upload, rebuilt
/// from the fields the API knows; a load that succeeded and one told to stop both
/// carry it.
#[tokio::test]
async fn a_settled_typed_load_carries_the_report_of_values_that_did_not_fit() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    mount_run_status(&stack.dagster, "SUCCESS").await;
    seed_typed_loading(&stack, "up-1").await;
    stack.bucket.put(
        &format!("uploads/{GROUP}/up-1.csv.bad-values.json"),
        &report_bytes(3),
    );
    mount_results(
        &stack.ch,
        &json!([result_row("up-1", "succeeded", Some(7), "", 30)]),
    )
    .await;

    let reply = stack.get(&bayu, "/api/uploads/up-1").await;

    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text);
    assert_eq!(reply.json["status"], "ingested");
    assert_eq!(
        reply.json["badValues"],
        json!({ "count": 3, "byColumn": [{ "column": "qty", "count": 3 }], "rows": [{ "row": 2, "column": "qty", "value": "abc" }] }),
        "only the fields the API knows"
    );
    assert!(!reply.text.contains("never copied"));
    assert!(
        !reply.text.contains("bad-values.json"),
        "no storage key reaches a response"
    );

    // A load told to stop: failed, with the fixed reason, and the report.
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    mount_run_status(&stack.dagster, "FAILURE").await;
    seed_typed_loading(&stack, "up-2").await;
    stack.bucket.put(
        &format!("uploads/{GROUP}/up-2.csv.bad-values.json"),
        &report_bytes(5),
    );
    mount_results(
        &stack.ch,
        &json!([result_row("up-2", "failed", None, BAD_VALUES_REASON, 30)]),
    )
    .await;
    let failed = stack.get(&bayu, "/api/uploads/up-2").await;
    assert_eq!(failed.json["status"], "failed");
    assert_eq!(failed.json["error"], BAD_VALUES_REASON);
    assert_eq!(failed.json["badValues"]["count"], 5);
}

/// No report, a report that is not the job's shape, and a report of an upload
/// that was loaded with no column list are all no report.
#[tokio::test]
async fn a_missing_malformed_or_unasked_for_report_is_no_report() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    mount_run_status(&stack.dagster, "SUCCESS").await;
    seed_typed_loading(&stack, "up-1").await; // typed, nothing written
    seed_typed_loading(&stack, "up-2").await; // typed, a report that is not the shape
    stack.bucket.put(
        &format!("uploads/{GROUP}/up-2.csv.bad-values.json"),
        b"{\"count\":\"many\"}",
    );
    stack
        .seed_loading(GROUP, "up-3", "stock_raw", Some("run-1"), "1 hour")
        .await; // untyped
    stack.bucket.put(
        &format!("uploads/{GROUP}/up-3.csv.bad-values.json"),
        &report_bytes(9),
    );
    mount_results(
        &stack.ch,
        &json!([
            result_row("up-1", "succeeded", Some(1), "", 30),
            result_row("up-2", "succeeded", Some(1), "", 30),
            result_row("up-3", "succeeded", Some(1), "", 30),
        ]),
    )
    .await;
    for id in ["up-1", "up-2", "up-3"] {
        let reply = stack.get(&bayu, &format!("/api/uploads/{id}")).await;
        assert_eq!(reply.json["status"], "ingested", "{id}: {}", reply.text);
        assert!(reply.json.get("badValues").is_none(), "{id}");
    }
}

// ── `DATA-17` slice 4: JSON, JSON Lines, Avro, gzip and zip ──────────────

/// The expected `columns` and `rows` of a fixture of `formats/` (`null` is an empty cell).
fn expected_of(name: &str) -> (Value, Value) {
    let value: Value =
        serde_json::from_slice(&format_file(&format!("{name}.expected.json"))).unwrap();
    let rows: Vec<Value> = value["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            Value::Array(
                row.as_array()
                    .unwrap()
                    .iter()
                    .map(|cell| Value::String(cell.as_str().unwrap_or_default().to_owned()))
                    .collect(),
            )
        })
        .collect();
    (value["columns"].clone(), Value::Array(rows))
}

/// (file, format, compression)
const NEW_KINDS: [(&str, &str, &str); 5] = [
    ("orders.json", "json", "none"),
    ("orders.jsonl", "jsonl", "none"),
    ("orders.avro", "avro", "none"),
    ("orders.csv.gz", "delimited", "gzip"),
    ("three.zip", "delimited", "zip"),
];

/// A zip of stored (uncompressed) entries, built by hand: the tests need no zip writer.
fn stored_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
    zip_of(entries, false)
}

fn zip_of(entries: &[(&str, &[u8])], deflate: bool) -> Vec<u8> {
    fn crc32(data: &[u8]) -> u32 {
        let mut crc = 0xffff_ffff_u32;
        for byte in data {
            crc ^= u32::from(*byte);
            for _ in 0..8 {
                crc = if crc & 1 == 1 {
                    (crc >> 1) ^ 0xedb8_8320
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, data) in entries {
        let offset = u32::try_from(out.len()).unwrap();
        let packed = if deflate {
            let mut encoder =
                flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::best());
            std::io::Write::write_all(&mut encoder, data).unwrap();
            encoder.finish().unwrap()
        } else {
            data.to_vec()
        };
        let method = u8::from(deflate) * 8;
        let (crc, size, packed_size, name_len) = (
            crc32(data),
            u32::try_from(data.len()).unwrap(),
            u32::try_from(packed.len()).unwrap(),
            u16::try_from(name.len()).unwrap(),
        );
        out.extend_from_slice(&[0x50, 0x4b, 3, 4, 20, 0, 0, 0, method, 0, 0, 0, 0, 0]);
        for word in [crc, packed_size, size] {
            out.extend_from_slice(&word.to_le_bytes());
        }
        out.extend_from_slice(&name_len.to_le_bytes());
        out.extend_from_slice(&[0, 0]);
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(&packed);
        central.extend_from_slice(&[0x50, 0x4b, 1, 2, 20, 0, 20, 0, 0, 0, method, 0, 0, 0, 0, 0]);
        for word in [crc, packed_size, size] {
            central.extend_from_slice(&word.to_le_bytes());
        }
        central.extend_from_slice(&name_len.to_le_bytes());
        central.extend_from_slice(&[0; 12]);
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name.as_bytes());
    }
    let start = u32::try_from(out.len()).unwrap();
    let count = u16::try_from(entries.len()).unwrap();
    out.extend_from_slice(&central);
    out.extend_from_slice(&[0x50, 0x4b, 5, 6, 0, 0, 0, 0]);
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&u32::try_from(central.len()).unwrap().to_le_bytes());
    out.extend_from_slice(&start.to_le_bytes());
    out.extend_from_slice(&[0, 0]);
    out
}

/// The 400 sentence `POST /api/uploads` answers a file with.
async fn refusal_of(stack: &Stack, who: &Caller, name: &str, bytes: Vec<u8>) -> String {
    let reply = stack
        .send(
            who,
            "POST",
            "/api/uploads",
            file_form(name, "application/octet-stream", &bytes),
        )
        .await;
    assert_eq!(
        reply.status,
        StatusCode::BAD_REQUEST,
        "{name}: {}",
        reply.text
    );
    reply.json["error"].as_str().unwrap().to_owned()
}

#[tokio::test]
async fn each_new_format_previews_with_the_columns_and_rows_of_the_shared_fixture() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    for (name, format, compression) in NEW_KINDS {
        let upload = stack.upload(&bayu, name, &format_file(name)).await;
        let id = id_of(&upload);
        let reply = stack
            .get(&bayu, &format!("/api/uploads/{id}/preview"))
            .await;
        assert_eq!(reply.status, StatusCode::OK, "{name}: {}", reply.text);
        let (columns, rows) = expected_of(name);
        assert_eq!(reply.json["columns"], columns, "{name}");
        // A zip previews its first file (`a.csv`, two rows); the load reads all of them.
        let shown = if compression == "zip" {
            Value::Array(rows.as_array().unwrap()[..2].to_vec())
        } else {
            rows
        };
        assert_eq!(reply.json["rows"], shown, "{name}");
        assert_eq!(
            reply.json["layout"],
            json!({ "format": format, "compression": compression }),
            "{name}"
        );
        assert_eq!(reply.json["truncated"], false, "{name}");
        assert_eq!(
            reply.json["columnSpecs"].as_array().unwrap().len(),
            columns.as_array().unwrap().len(),
            "{name}"
        );
    }
}

#[tokio::test]
async fn an_avro_preview_starts_each_column_at_the_type_its_schema_declares() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    let id = id_of(
        &stack
            .upload(&bayu, "orders.avro", &format_file("orders.avro"))
            .await,
    );
    let reply = stack
        .get(&bayu, &format!("/api/uploads/{id}/preview"))
        .await;
    let kinds: Vec<&str> = reply.json["columnSpecs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["type"].as_str().unwrap())
        .collect();
    assert_eq!(
        kinds,
        [
            "integer", "text", "float", "boolean", "text", "text", "text", "date"
        ],
        "{}",
        reply.text
    );
    assert_eq!(reply.json["columnSpecs"][7]["dateFormat"], "iso");
}

#[tokio::test]
async fn a_zip_preview_lists_its_files_and_a_load_with_a_file_of_other_columns_is_refused() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    let id = id_of(
        &stack
            .upload(&bayu, "three.zip", &format_file("three.zip"))
            .await,
    );
    let reply = stack
        .get(&bayu, &format!("/api/uploads/{id}/preview"))
        .await;
    let files = reply.json["archive"]["files"].as_array().unwrap();
    let names: Vec<&str> = files.iter().map(|f| f["name"].as_str().unwrap()).collect();
    assert_eq!(
        names,
        ["a.csv", "sub/b.csv", "c.csv"],
        "folders are left out"
    );
    assert!(
        files
            .iter()
            .all(|f| f["matchesFirst"] == true && f["columns"] == 3),
        "{}",
        reply.text
    );

    // The same kind of archive with a file that has another column.
    let other = stored_zip(&[("a.csv", b"id,name\n1,a\n"), ("b.csv", b"id,other\n2,b\n")]);
    let id = id_of(&stack.upload(&bayu, "other.zip", &other).await);
    let reply = stack
        .get(&bayu, &format!("/api/uploads/{id}/preview"))
        .await;
    let files = reply.json["archive"]["files"].as_array().unwrap();
    assert_eq!(files[0]["matchesFirst"], true);
    assert_eq!(files[1]["matchesFirst"], false);
    // Load is refused until the files agree, and nothing is launched.
    let body =
        json!({ "bronzeTable": "zipped", "encoding": "utf-8", "delimiter": ",", "headerRow": 0 });
    let refused = stack.ingest(&bayu, &id, body).await;
    assert_eq!(refused.status, StatusCode::BAD_REQUEST, "{}", refused.text);
    assert!(
        refused.json["error"]
            .as_str()
            .unwrap()
            .contains("do not all have the same columns")
    );
    assert!(
        requests_containing(&stack.dagster, "launchRun")
            .await
            .is_empty()
    );
}

#[tokio::test]
async fn the_job_is_told_the_format_and_the_wrapper_of_each_new_kind() {
    for (name, format, compression) in NEW_KINDS {
        let stack = Stack::start().await;
        let bayu = stack.bayu().await;
        mount_free_table(&stack.ch).await;
        mount_launch(&stack.dagster, "run-1").await;
        let id = id_of(&stack.upload(&bayu, name, &format_file(name)).await);
        // JSON, JSON Lines and Avro have no dialect: what a retry sends back is not read.
        let body = if format == "delimited" {
            json!({ "bronzeTable": "formatted", "encoding": "utf-8", "delimiter": ",", "headerRow": 0 })
        } else {
            json!({ "bronzeTable": "formatted", "encoding": "utf-16", "delimiter": ";", "headerRow": 3 })
        };
        let reply = stack.ingest(&bayu, &id, body).await;
        assert_eq!(reply.status, StatusCode::OK, "{name}: {}", reply.text);
        let launches = requests_containing(&stack.dagster, "launchRun").await;
        let config = &launches[0]["variables"]["cfg"]["ops"]["ingest_uploaded_file"]["config"];
        assert_eq!(config["format"], format, "{name}");
        assert_eq!(config["compression"], compression, "{name}");
        assert_eq!(config["max_bytes"], UNPACKED_CAP, "{name}");
        assert_eq!(config["encoding"], "utf-8", "{name}");
        assert_eq!(config["delimiter"], ",", "{name}");
        assert_eq!(config["header_row"], 0, "{name}");
        let extension = name.rsplit('.').next().unwrap();
        assert_eq!(
            config["storage_keys"],
            json!([format!("uploads/{GROUP}/{id}.{extension}")]),
            "{name}: the original object, read by the job"
        );
    }
}

#[tokio::test]
async fn a_gzip_bomb_is_refused_when_it_arrives_and_nothing_is_stored() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    // One byte more than the unpacked cap (2 GiB), made in 1 MiB blocks and never held
    // whole: it packs into a couple of MiB. Plan T6.2 item 2: the cap moved from 50 MiB.
    // (The zip check shares the cap and `count`; its refusal is pinned by the
    // `upload_formats` tests, which take the cap as a parameter, since a 2 GiB zip entry
    // could only be built in memory here.)
    // `PR #186 CI`: constant bytes at the best level pack about 1000 to 1, so the bomb is
    // about 2 MiB; the fast level left it past the setup bound below.
    let block = vec![b'a'; MIB];
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
    for _ in 0..2048 {
        std::io::Write::write_all(&mut encoder, &block).unwrap();
    }
    std::io::Write::write_all(&mut encoder, b"a").unwrap();
    let bomb = encoder.finish().unwrap();
    assert!(bomb.len() < 8 * MIB, "the setup: a small file");
    let unpacked = "The file is larger than the upload limit once unpacked.";

    assert_eq!(
        refusal_of(&stack, &bayu, "bomb.csv.gz", bomb.clone()).await,
        unpacked
    );
    assert!(stack.bucket.keys().is_empty(), "nothing was stored");

    // The same file by pieces: refused at completion and removed.
    let id = stack.session(&bayu, "bomb.csv.gz", bomb.len()).await;
    stack.put_part(&bayu, &id, "1", &bomb).await;
    let reply = stack.complete(&bayu, &id).await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST, "{}", reply.text);
    assert_eq!(reply.json["error"], unpacked);
    assert_eq!(stack.row_count(&id).await, 0);
}

#[tokio::test]
async fn an_archive_inside_an_archive_a_mixed_zip_and_a_misnamed_file_are_refused() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    let refuse = |name: &'static str, bytes: Vec<u8>| refusal_of(&stack, &bayu, name, bytes);
    let inner = stored_zip(&[("a.csv", b"a\n1\n")]);
    assert!(
        refuse("nested.zip", stored_zip(&[("inner.zip", &inner)]))
            .await
            .contains("another archive")
    );
    let mixed = stored_zip(&[("a.csv", b"a\n1\n"), ("b.json", b"[{}]")]);
    assert!(
        refuse("mixed.zip", mixed)
            .await
            .contains("one supported format")
    );
    let names: Vec<String> = (0..11).map(|i| format!("{i}.csv")).collect();
    let eleven: Vec<(&str, &[u8])> = names.iter().map(|n| (n.as_str(), &b"a\n1\n"[..])).collect();
    assert!(
        refuse("many.zip", stored_zip(&eleven))
            .await
            .contains("at most 10")
    );
    assert!(
        refuse("not.zip", b"not a zip".to_vec())
            .await
            .contains("is not one")
    );
    assert!(
        refuse("array.json", b"{\"a\": 1}".to_vec())
            .await
            .contains("array of objects")
    );
    assert!(
        refuse("data.avro", b"id,name\n".to_vec())
            .await
            .contains("not an Avro file")
    );
    assert!(
        refuse("data.csv", format_file("orders.avro"))
            .await
            .contains("does not end in .avro")
    );
    assert!(stack.bucket.keys().is_empty());
}

#[tokio::test]
async fn the_new_kinds_complete_by_pieces_with_the_same_checks() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    for (name, _format, _compression) in NEW_KINDS {
        let upload = stack
            .upload_in_pieces(&bayu, name, &format_file(name))
            .await;
        assert_eq!(upload["status"], "uploaded", "{name}: {upload}");
    }
}

// ════════════════════════════════════════════════════════════════════════
// Several files into one table (DATA-17 slice 5)
// ════════════════════════════════════════════════════════════════════════

const STOCK_A: &[u8] = b"id,name,qty\n1,bolt,10\n2,nut,20\n";
/// The same columns in another order.
const STOCK_B: &[u8] = b"qty,id,name\n30,3,screw\n40,4,washer\n";
const STOCK_C: &[u8] = b"id,name,qty\n5,pin,50\n";
/// One column more.
const STOCK_D: &[u8] = b"id,name,qty,notes\n6,rivet,60,fragile\n";

impl Stack {
    async fn compare(&self, who: &Caller, id: &str, body: Value) -> Reply {
        self.send(
            who,
            "POST",
            &format!("/api/uploads/{id}/compare"),
            Payload::Json(body),
        )
        .await
    }

    /// Upload `files` (name, bytes) as `who` and return their ids, in order.
    async fn upload_all(&self, who: &Caller, files: &[(&str, &[u8])]) -> Vec<String> {
        let mut ids = Vec::new();
        for (name, bytes) in files {
            ids.push(id_of(&self.upload(who, name, bytes).await));
        }
        ids
    }

    /// `(status, loaded_with, error, row_count)` of an upload.
    async fn group_state(&self, id: &str) -> (String, Option<String>, Option<String>, Option<i64>) {
        sqlx::query_as(
            "SELECT status, loaded_with, error, row_count FROM file_upload WHERE id = $1",
        )
        .bind(id)
        .fetch_one(&self.app.pool)
        .await
        .unwrap()
    }
}

/// The files the comparison lists, by upload id, with what differs in each.
fn mismatches_of(reply: &Reply) -> Vec<(String, Vec<String>, Vec<String>)> {
    reply.json["mismatches"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| {
            let list = |key: &str| {
                m[key]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_str().unwrap().to_owned())
                    .collect::<Vec<_>>()
            };
            (
                m["uploadId"].as_str().unwrap().to_owned(),
                list("missing"),
                list("extra"),
            )
        })
        .collect()
}

/// Decision 4: columns are matched by name and their order may differ; a file whose columns
/// differ is listed with what it lacks and what it adds. The answer carries the file's name.
#[tokio::test]
async fn the_comparison_lists_a_file_with_other_columns_and_passes_one_in_another_order() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    let ids = stack
        .upload_all(
            &bayu,
            &[
                ("a.csv", STOCK_A),
                ("b.csv", STOCK_B),
                ("c.csv", STOCK_C),
                ("d.csv", STOCK_D),
            ],
        )
        .await;

    let reply = stack
        .compare(
            &bayu,
            &ids[0],
            json!({ "alsoUploads": [ids[1], ids[2], ids[3]] }),
        )
        .await;

    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text);
    assert_eq!(
        mismatches_of(&reply),
        vec![(ids[3].clone(), vec![], vec!["notes".to_owned()])],
        "only the file with an extra column is listed"
    );
    assert_eq!(reply.json["mismatches"][0]["file"], "d.csv");
    assert_eq!(reply.json["mismatches"][0]["typeConflicts"], json!([]));

    let missing = stack
        .compare(&bayu, &ids[3], json!({ "alsoUploads": [ids[0]] }))
        .await;
    assert_eq!(
        mismatches_of(&missing),
        vec![(ids[0].clone(), vec!["notes".to_owned()], vec![])],
        "from the other side the same column is missing"
    );
    assert!(
        stack.dagster.received_requests().await.unwrap().is_empty(),
        "a comparison launches nothing"
    );
    assert_eq!(
        stack.status_of(&ids[0]).await.0,
        "uploaded",
        "and claims nothing"
    );
}

/// A value that does not fit the first file's type is a type conflict, unless the person made
/// that column text (the column list the console sends), which every value fits.
#[tokio::test]
async fn a_value_that_does_not_fit_the_first_files_type_is_a_type_conflict_until_the_column_is_text()
 {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    let ids = stack
        .upload_all(
            &bayu,
            &[
                ("a.csv", STOCK_A),
                ("n.csv", b"id,name,qty\n7,cap,n/a\n8,cog,5\n"),
            ],
        )
        .await;

    let guessed = stack
        .compare(&bayu, &ids[0], json!({ "alsoUploads": [ids[1]] }))
        .await;
    assert_eq!(guessed.status, StatusCode::OK, "{}", guessed.text);
    assert_eq!(
        guessed.json["mismatches"],
        json!([{
            "uploadId": ids[1], "file": "n.csv", "missing": [], "extra": [],
            "typeConflicts": [{ "column": "qty", "expected": "integer", "found": "text" }]
        }])
    );

    let columns = json!([
        { "source": "id", "name": "id", "type": "integer" },
        { "source": "name", "name": "name", "type": "text" },
        { "source": "qty", "name": "qty", "type": "text" },
    ]);
    let as_text = stack
        .compare(
            &bayu,
            &ids[0],
            json!({ "alsoUploads": [ids[1]], "columns": columns }),
        )
        .await;
    assert_eq!(as_text.status, StatusCode::OK, "{}", as_text.text);
    assert_eq!(as_text.json["mismatches"], json!([]));

    let wrong_width = stack
        .compare(
            &bayu,
            &ids[0],
            json!({ "alsoUploads": [ids[1]], "columns": [{ "source": "id", "name": "id", "type": "integer" }] }),
        )
        .await;
    assert_eq!(
        wrong_width.status,
        StatusCode::BAD_REQUEST,
        "{}",
        wrong_width.text
    );
}

/// A load of three files, one in another column order: one launch whose config lists the
/// three objects (the primary first), every file `ingesting` and pointing at the primary,
/// the table claimed once by the primary, the audit event naming the others. The single-file
/// config keys are all still there (AD5).
#[tokio::test]
async fn three_files_are_one_launch_with_every_object_listed_and_every_file_claimed() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    mount_free_table(&stack.ch).await;
    mount_launch(&stack.dagster, "run-g").await;
    let ids = stack
        .upload_all(
            &bayu,
            &[("a.csv", STOCK_A), ("b.csv", STOCK_B), ("c.csv", STOCK_C)],
        )
        .await;

    let mut body = ingest_body("stock_raw");
    body["alsoUploads"] = json!([ids[1], ids[2]]);
    let reply = stack.ingest(&bayu, &ids[0], body).await;

    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text);
    assert_eq!(reply.json["runId"], "run-g");
    assert_eq!(reply.json["upload"]["id"], ids[0].as_str());
    assert!(reply.json["upload"].get("loadedWith").is_none());
    let launches = requests_containing(&stack.dagster, "launchRun").await;
    assert_eq!(launches.len(), 1, "one ingest call, one run");
    let config = &launches[0]["variables"]["cfg"]["ops"]["ingest_uploaded_file"]["config"];
    assert_eq!(
        config["storage_keys"],
        json!([
            format!("uploads/{GROUP}/{}.csv", ids[0]),
            format!("uploads/{GROUP}/{}.csv", ids[1]),
            format!("uploads/{GROUP}/{}.csv", ids[2]),
        ])
    );
    assert_eq!(config["upload_id"], ids[0].as_str());
    assert_eq!(config["bronze_table_name"], "stock_raw");
    assert_eq!(config["format"], "delimited");

    for (position, id) in ids.iter().enumerate() {
        let state = stack.group_state(id).await;
        assert_eq!(state.0, "ingesting", "{id}");
        let expected = (position > 0).then(|| ids[0].clone());
        assert_eq!(state.1, expected, "{id} loaded with");
    }
    assert_eq!(
        stack.claims().await,
        [(
            "stock_raw".to_owned(),
            Some(GROUP.parse::<Uuid>().unwrap()),
            ids[0].clone()
        )],
        "the table is claimed once, by the primary"
    );
    let shown = stack.get(&bayu, &format!("/api/uploads/{}", ids[1])).await;
    assert_eq!(shown.json["loadedWith"], ids[0].as_str());
    assert_eq!(shown.json["bronzeTable"], "stock_raw");

    let audit = stack.audit_actions().await;
    let ingests: Vec<_> = audit.iter().filter(|a| a.0 == "upload.ingest").collect();
    assert_eq!(ingests.len(), 1);
    assert_eq!(ingests[0].3["alsoUploads"], json!([ids[1], ids[2]]));
}

/// A fourth file with an extra column is refused with the list, the file named, and
/// nothing is claimed, marked or launched (decision 4).
#[tokio::test]
async fn a_file_with_an_extra_column_refuses_the_whole_load_with_the_list() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    mount_free_table(&stack.ch).await;
    mount_launch(&stack.dagster, "run-x").await;
    let ids = stack
        .upload_all(
            &bayu,
            &[
                ("a.csv", STOCK_A),
                ("b.csv", STOCK_B),
                ("c.csv", STOCK_C),
                ("d.csv", STOCK_D),
            ],
        )
        .await;

    let mut body = ingest_body("stock_raw");
    body["alsoUploads"] = json!([ids[1], ids[2], ids[3]]);
    let reply = stack.ingest(&bayu, &ids[0], body).await;

    assert_eq!(reply.status, StatusCode::BAD_REQUEST, "{}", reply.text);
    let message = reply.json["error"].as_str().unwrap();
    assert!(message.contains("d.csv (extra notes)"), "{message}");
    assert!(
        !message.contains("b.csv") && !message.contains("c.csv"),
        "{message}"
    );
    assert!(
        requests_containing(&stack.dagster, "launchRun")
            .await
            .is_empty()
    );
    assert!(stack.claims().await.is_empty(), "no table was claimed");
    for id in &ids {
        assert_eq!(
            stack.group_state(id).await,
            ("uploaded".to_owned(), None, None, None)
        );
    }
}

/// The ingest of one file is what it was: no `alsoUploads`, one key, no pointer anywhere, the
/// audit event without the new key.
#[tokio::test]
async fn a_load_of_one_file_is_unchanged_by_the_group_columns() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    mount_free_table(&stack.ch).await;
    mount_launch(&stack.dagster, "run-1").await;
    let id = id_of(&stack.upload(&bayu, "a.csv", STOCK_A).await);

    let reply = stack.ingest(&bayu, &id, ingest_body("stock_raw")).await;

    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text);
    let launches = requests_containing(&stack.dagster, "launchRun").await;
    assert_eq!(
        launches[0]["variables"]["cfg"]["ops"]["ingest_uploaded_file"]["config"]["storage_keys"],
        json!([format!("uploads/{GROUP}/{id}.csv")])
    );
    assert!(stack.group_state(&id).await.1.is_none());
    let audit = stack.audit_actions().await;
    assert!(
        audit
            .iter()
            .find(|a| a.0 == "upload.ingest")
            .unwrap()
            .3
            .get("alsoUploads")
            .is_none()
    );
}

/// Settling: when the run ends, the list settles every file of the load from the one recorded
/// result: the primary has the total, the others have no count of their own (not measured,
/// never 0), and a failure carries the one reason to all.
#[tokio::test]
async fn the_files_of_a_load_end_together_with_the_reason_or_total_of_the_one_run() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    mount_free_table(&stack.ch).await;
    mount_launch(&stack.dagster, "run-g").await;
    let ids = stack
        .upload_all(&bayu, &[("a.csv", STOCK_A), ("b.csv", STOCK_B)])
        .await;
    let mut body = ingest_body("stock_raw");
    body["alsoUploads"] = json!([ids[1]]);
    assert_eq!(
        stack.ingest(&bayu, &ids[0], body).await.status,
        StatusCode::OK
    );

    mount_run_status(&stack.dagster, "SUCCESS").await;
    mount_results(
        &stack.ch,
        &json!([result_row(&ids[0], "succeeded", Some(4), "", 0)]),
    )
    .await;

    // Reading the OTHER file settles the whole load, through the primary.
    let other = stack.get(&bayu, &format!("/api/uploads/{}", ids[1])).await;
    assert_eq!(other.status, StatusCode::OK, "{}", other.text);
    assert_eq!(other.json["status"], "ingested");
    assert!(
        other.json.get("rows").is_none(),
        "its own count is not measured"
    );
    assert_eq!(other.json["loadedWith"], ids[0].as_str());
    let primary = stack.group_state(&ids[0]).await;
    assert_eq!((primary.0.as_str(), primary.3), ("ingested", Some(4)));
}

#[tokio::test]
async fn a_failed_run_fails_every_file_of_the_load_with_one_reason() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    mount_free_table(&stack.ch).await;
    mount_launch(&stack.dagster, "run-g").await;
    let ids = stack
        .upload_all(
            &bayu,
            &[("a.csv", STOCK_A), ("b.csv", STOCK_B), ("c.csv", STOCK_C)],
        )
        .await;
    let mut body = ingest_body("stock_raw");
    body["alsoUploads"] = json!([ids[1], ids[2]]);
    assert_eq!(
        stack.ingest(&bayu, &ids[0], body).await.status,
        StatusCode::OK
    );

    mount_run_status(&stack.dagster, "FAILURE").await;
    mount_results(
        &stack.ch,
        &json!([result_row(
            &ids[0],
            "failed",
            None,
            "The file has no rows below the header row.",
            0
        )]),
    )
    .await;
    let list = stack.get(&bayu, "/api/uploads").await;
    assert_eq!(list.status, StatusCode::OK, "{}", list.text);

    for id in &ids {
        let state = stack.group_state(id).await;
        assert_eq!(state.0, "failed", "{id}");
        assert_eq!(
            state.2.as_deref(),
            Some("The file has no rows below the header row.")
        );
        assert_eq!(state.3, None);
    }
}

/// A file left `ingesting` with a primary that is not loading (a claim that never finished)
/// is failed after the two minutes a claim with no run gets, and not before.
#[tokio::test]
async fn a_file_whose_primary_is_not_loading_is_failed_after_two_minutes_and_not_before() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    let ids = stack
        .upload_all(&bayu, &[("a.csv", STOCK_A), ("b.csv", STOCK_B)])
        .await;
    stack.claim(GROUP, "stock_raw", &ids[0]).await;
    let options = json!({ "encoding": "utf-8", "delimiter": ",", "headerRow": 0 });
    uploads::mark_ingesting(
        &stack.app.pool,
        &ids[1],
        &options,
        "stock_raw",
        LoadMode::Replace,
        None,
    )
    .await
    .unwrap()
    .unwrap();
    sqlx::query("UPDATE file_upload SET loaded_with = $1 WHERE id = $2")
        .bind(&ids[0])
        .bind(&ids[1])
        .execute(&stack.app.pool)
        .await
        .unwrap();

    let young = stack.get(&bayu, &format!("/api/uploads/{}", ids[1])).await;
    assert_eq!(
        young.json["status"], "ingesting",
        "a younger orphan is not an orphan yet"
    );

    sqlx::query("UPDATE file_upload SET updated_at = now() - interval '3 minutes' WHERE id = $1")
        .bind(&ids[1])
        .execute(&stack.app.pool)
        .await
        .unwrap();
    let old = stack.get(&bayu, &format!("/api/uploads/{}", ids[1])).await;
    assert_eq!(old.json["status"], "failed", "{}", old.text);
    assert_eq!(old.json["error"], "The load was not started.");
}

/// A load of several files is one place against the limits: with four loads allowed per
/// person, a ten-file load is refused by none of its files, and a fifth LOAD is.
#[tokio::test]
async fn ten_files_are_one_load_for_the_per_person_limit() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    mount_free_table(&stack.ch).await;
    mount_launch(&stack.dagster, "run-g").await;
    let mut files = Vec::new();
    for n in 0..10 {
        files.push((
            format!("f{n}.csv"),
            format!("id,name,qty\n{n},x,1\n").into_bytes(),
        ));
    }
    let mut ids = Vec::new();
    for (name, bytes) in &files {
        ids.push(id_of(&stack.upload(&bayu, name, bytes).await));
    }
    let mut body = ingest_body("stock_raw");
    body["alsoUploads"] = json!(ids[1..]);

    let reply = stack.ingest(&bayu, &ids[0], body).await;

    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text);
    let loading = stack.group_state(&ids[9]).await;
    assert_eq!(loading.0, "ingesting");
    // Another load of the same person still has its place (4 allowed, 1 used).
    let extra = id_of(&stack.upload(&bayu, "z.csv", STOCK_C).await);
    let again = stack.ingest(&bayu, &extra, ingest_body("other_raw")).await;
    assert_eq!(again.status, StatusCode::OK, "{}", again.text);
}

/// The list of other files is checked before anything is read: more than nine, a repeat, the
/// upload itself, something that is not a list, and an id that is not text are each a 400, and
/// asking nobody.
#[tokio::test]
async fn a_bad_list_of_other_files_is_400_and_asks_nobody() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    let ids = stack
        .upload_all(&bayu, &[("a.csv", STOCK_A), ("b.csv", STOCK_B)])
        .await;
    let ten: Vec<String> = (0..10).map(|n| format!("up-{n}")).collect();
    for also in [
        json!(ten),
        json!([ids[1], ids[1]]),
        json!([ids[0]]),
        json!("up-1"),
        json!([7]),
        json!({ "a": 1 }),
    ] {
        let mut body = ingest_body("stock_raw");
        body["alsoUploads"] = also.clone();
        let reply = stack.ingest(&bayu, &ids[0], body).await;
        assert_eq!(
            reply.status,
            StatusCode::BAD_REQUEST,
            "{also}: {}",
            reply.text
        );
        let compared = stack
            .compare(&bayu, &ids[0], json!({ "alsoUploads": also }))
            .await;
        assert_eq!(
            compared.status,
            StatusCode::BAD_REQUEST,
            "{also}: {}",
            compared.text
        );
    }
    assert!(stack.dagster.received_requests().await.unwrap().is_empty());
    assert!(stack.claims().await.is_empty());
}

/// A file that is unknown, of another tenant, or of a tenant the caller is not in is one
/// sentence, so the answer is no oracle; a file of another tenant the caller IS in is refused
/// too (one load never mixes tenants).
#[tokio::test]
async fn a_file_of_another_tenant_or_an_unknown_one_is_the_same_404_and_a_load_never_mixes_tenants()
{
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    let andi = stack.andi().await;
    mount_free_table(&stack.ch).await;
    mount_launch(&stack.dagster, "run-g").await;
    let mine = id_of(&stack.upload(&bayu, "a.csv", STOCK_A).await);
    let theirs = id_of(&stack.upload(&andi, "b.csv", STOCK_B).await);
    // Bayu belongs to two tenants; this file is in the other one.
    let logistics = stack.caller("bayu@meridian.example", Some(LOGISTICS)).await;
    let sibling = id_of(&stack.upload(&logistics, "c.csv", STOCK_C).await);

    let mut answers = Vec::new();
    for other in [theirs.as_str(), sibling.as_str(), "up-does-not-exist"] {
        let compared = stack
            .compare(&bayu, &mine, json!({ "alsoUploads": [other] }))
            .await;
        assert_eq!(
            compared.status,
            StatusCode::NOT_FOUND,
            "{other}: {}",
            compared.text
        );
        let mut body = ingest_body("stock_raw");
        body["alsoUploads"] = json!([other]);
        let ingested = stack.ingest(&bayu, &mine, body).await;
        assert_eq!(
            ingested.status,
            StatusCode::NOT_FOUND,
            "{other}: {}",
            ingested.text
        );
        assert_eq!(compared.json, ingested.json);
        answers.push(compared.json);
    }
    assert!(
        answers.windows(2).all(|pair| pair[0] == pair[1]),
        "{answers:?}"
    );
    assert!(
        requests_containing(&stack.dagster, "launchRun")
            .await
            .is_empty()
    );
    assert!(stack.claims().await.is_empty());
    assert_eq!(stack.group_state(&mine).await.0, "uploaded");
    assert_eq!(stack.group_state(&theirs).await.0, "uploaded");
}

/// A file still being received, or already loading, cannot be taken; and nothing of the load
/// is claimed when one cannot.
#[tokio::test]
async fn a_file_that_is_being_received_or_loading_cannot_be_loaded_with_another() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    mount_free_table(&stack.ch).await;
    mount_launch(&stack.dagster, "run-g").await;
    let mine = id_of(&stack.upload(&bayu, "a.csv", STOCK_A).await);
    let receiving = stack.session(&bayu, "big.csv", 1024).await;
    stack
        .seed_loading(GROUP, "up-busy", "busy_raw", Some("run-0"), "1 minute")
        .await;

    for (other, expected) in [
        (receiving.as_str(), "still being received"),
        ("up-busy", "already being loaded"),
    ] {
        let mut body = ingest_body("stock_raw");
        body["alsoUploads"] = json!([other]);
        let reply = stack.ingest(&bayu, &mine, body).await;
        assert_eq!(
            reply.status,
            StatusCode::CONFLICT,
            "{other}: {}",
            reply.text
        );
        assert!(
            reply.json["error"].as_str().unwrap().contains(expected),
            "{}",
            reply.text
        );
    }
    assert_eq!(stack.group_state(&mine).await.0, "uploaded");
    assert!(stack.claims().await.iter().all(|c| c.0 != "stock_raw"));
}

/// One load is one kind of file: a CSV and a workbook, or a CSV and JSON Lines, are refused;
/// and the files together are under the cap, which counts all of them.
#[tokio::test]
async fn files_of_two_kinds_or_together_over_the_cap_are_refused() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    let csv = id_of(&stack.upload(&bayu, "a.csv", STOCK_A).await);
    let lines = id_of(
        &stack
            .upload(&bayu, "b.jsonl", b"{\"id\":1,\"name\":\"a\",\"qty\":2}\n")
            .await,
    );
    let sheet = id_of(&stack.upload(&bayu, "c.xlsx", &workbook("stock.xlsx")).await);
    let second = id_of(&stack.upload(&bayu, "d.csv", STOCK_C).await);

    for other in [&lines, &sheet] {
        let compared = stack
            .compare(&bayu, &csv, json!({ "alsoUploads": [other] }))
            .await;
        assert_eq!(
            compared.status,
            StatusCode::BAD_REQUEST,
            "{}",
            compared.text
        );
        assert!(
            compared.json["error"]
                .as_str()
                .unwrap()
                .contains("same kind of file")
        );
    }

    // 1.5 GiB and 1.5 GiB are each under the cap and together over it (the
    // sizes are written to the rows; no such file is stored).
    sqlx::query("UPDATE file_upload SET size_bytes = 1610612736 WHERE id = ANY($1)")
        .bind(vec![csv.clone(), second.clone()])
        .execute(&stack.app.pool)
        .await
        .unwrap();
    let mut body = ingest_body("stock_raw");
    body["alsoUploads"] = json!([second]);
    let reply = stack.ingest(&bayu, &csv, body).await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST, "{}", reply.text);
    assert_eq!(
        reply.json["error"],
        "The files together are larger than the 2 GB limit."
    );
    assert!(stack.dagster.received_requests().await.unwrap().is_empty());
}

/// Parquet files load together too: each footer is checked and each is listed, unconverted.
#[tokio::test]
async fn parquet_files_load_together_without_conversion() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    mount_free_table(&stack.ch).await;
    mount_launch(&stack.dagster, "run-p").await;
    let first = id_of(
        &stack
            .upload(&bayu, "a.parquet", &parquet_file("types.parquet"))
            .await,
    );
    let second = id_of(
        &stack
            .upload(&bayu, "b.parquet", &parquet_file("types.parquet"))
            .await,
    );

    let compared = stack
        .compare(&bayu, &first, json!({ "alsoUploads": [second] }))
        .await;
    assert_eq!(compared.status, StatusCode::OK, "{}", compared.text);
    assert_eq!(compared.json["mismatches"], json!([]));

    let mut body = ingest_body("typed_raw");
    body["alsoUploads"] = json!([second]);
    let reply = stack.ingest(&bayu, &first, body).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text);
    let launches = requests_containing(&stack.dagster, "launchRun").await;
    let config = &launches[0]["variables"]["cfg"]["ops"]["ingest_uploaded_file"]["config"];
    assert_eq!(config["format"], "parquet");
    assert_eq!(
        config["storage_keys"],
        json!([
            format!("uploads/{GROUP}/{first}.parquet"),
            format!("uploads/{GROUP}/{second}.parquet"),
        ]),
        "unconverted: the job reads the Parquet objects themselves"
    );
}

/// Loading a file alone again, or its primary alone, lets the group go: the label of lineage
/// and the pointer follow the LAST load.
#[tokio::test]
async fn a_pointer_to_the_primary_goes_when_the_primary_loads_again_alone() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    mount_free_table(&stack.ch).await;
    mount_launch(&stack.dagster, "run-g").await;
    let ids = stack
        .upload_all(&bayu, &[("a.csv", STOCK_A), ("b.csv", STOCK_B)])
        .await;
    let mut body = ingest_body("stock_raw");
    body["alsoUploads"] = json!([ids[1]]);
    assert_eq!(
        stack.ingest(&bayu, &ids[0], body).await.status,
        StatusCode::OK
    );
    assert_eq!(
        stack.group_state(&ids[1]).await.1.as_deref(),
        Some(ids[0].as_str())
    );
    // The load ends (settled by hand: the run is not what this test is about).
    uploads::mark_finished(&stack.app.pool, &ids[0], Some("run-g"), None, Some(4))
        .await
        .unwrap()
        .unwrap();

    let again = stack.ingest(&bayu, &ids[0], ingest_body("stock_raw")).await;

    assert_eq!(again.status, StatusCode::OK, "{}", again.text);
    let state = stack.group_state(&ids[1]).await;
    assert_eq!((state.0.as_str(), state.1), ("ingested", None));
}
