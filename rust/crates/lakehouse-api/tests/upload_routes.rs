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

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/../../../ops/fixtures/uploads/{name}",
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

/// What the fake bucket holds, by object key.
#[derive(Clone, Default)]
struct Bucket(Arc<Mutex<HashMap<String, Vec<u8>>>>);

impl Bucket {
    fn keys(&self) -> Vec<String> {
        let mut keys: Vec<String> = self.0.lock().unwrap().keys().cloned().collect();
        keys.sort();
        keys
    }

    fn get(&self, key: &str) -> Option<Vec<u8>> {
        self.0.lock().unwrap().get(key).cloned()
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

async fn mount_bucket(server: &MockServer, bucket: &Bucket) {
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
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let text = String::from_utf8_lossy(&bytes).into_owned();
        Reply {
            status,
            json: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
            text,
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

    let workbook = refuse(
        "data.csv",
        b"PK\x03\x04\x14\x00\x06\x00rest of a zip".to_vec(),
    )
    .await;
    assert_eq!(
        workbook,
        "This looks like an Excel workbook or a zip archive. Only delimited text files (CSV, TSV) can be uploaded; save the sheet as CSV first."
    );
    assert!(
        refuse(
            "old.xls",
            vec![0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1]
        )
        .await
        .contains("Excel workbook")
    );
    assert!(
        refuse("t.parquet", b"PAR1\x15\x04\x15\x30rest".to_vec())
            .await
            .contains("Parquet")
    );
    assert!(
        refuse("report.pdf", b"%PDF-1.7 ...".to_vec())
            .await
            .contains("not a delimited text file")
    );
    assert!(
        refuse("x.csv", vec![0x1F, 0x8B, 8, 0, 0, 0])
            .await
            .contains("not a delimited text file")
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
        ["bytes=0-262143"],
        "one ranged read of the first 256 KiB, never the whole object"
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
            "storage_key": format!("uploads/{GROUP}/{id}.xls"),
            "bronze_table_name": "sap_stock",
            "load_mode": "append",
            "encoding": "utf-16",
            "delimiter": "\t",
            "header_row": 4,
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

/// The one sentence for a table name that is not this tenant's to load into
/// (review finding B4): another tenant's claim, and an existing table nobody
/// claimed, are told apart by nothing a caller can see.
const NOT_FREE: &str = "That table name is in use and no upload of this tenant created it, so a file cannot be loaded into it.";

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
