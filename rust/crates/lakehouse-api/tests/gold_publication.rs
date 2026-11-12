//! HTTP-level acceptance tests for gold-publish-per-mart plan T3 + T4:
//! the per-mart publication routes (`GET /api/gold/publications`, and
//! the detail/`PUT` pair on `/api/gold/export/{mart}/publication`), and
//! `POST /api/gold/export/{mart}?ifChanged=true`'s skip.
//!
//! These routes have a genuinely `ClickHouse`-backed half — `PUT` must
//! return 404 for a mart that does not exist in `system.tables`, and the
//! detail body reports measured `lastChangedAt`/`lastExportedAt` — so a
//! dead-upstream `AppState` cannot prove them. Each test therefore starts
//! a real disposable `ClickHouse` (the compose file's own `26.8` image,
//! passwordless exactly like the compose service's empty
//! `CH_PASSWORD` default) and points `CH_URL` at it, on top of the usual
//! per-test Postgres from `common::spin_up_with_env`. Lakekeeper stays
//! dead (`127.0.0.1:1` via the common harness): a `PUT` that never
//! touches Iceberg by design (DATA-1: switching off only flips a flag)
//! must return 200 in exactly that setup, which is itself part of the
//! proof.
//!
//! The authorization halves are additionally covered by the
//! `POLICY_TABLE`-driven loops in `tests/route_auth.rs` (zero-permission
//! principal denied, `gold:export` principal not-401/403 on every new
//! entry), which pick these routes up automatically from the table.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::collections::HashMap;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use lakehouse_clickhouse::ChClient;
use serde_json::Value;
use testcontainers::ImageExt;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::clickhouse::ClickHouse;
use tower::ServiceExt;

use common::{
    TestApp, create_principal_with_permissions, session_cookie_for_user, spin_up_with_env,
};

/// ONE `ClickHouse` per test binary, shared by every test — mirroring
/// `gold_export_history::TABLE_ENSURED`'s process-wide "the table has
/// been ensured" cache, which assumes a single `ClickHouse` per process.
/// Per-test containers would race that cache (whoever runs the ensure
/// first pins it to THEIR container; another test's first
/// `console.gold_export_run` read then hits a server without the table
/// and 500s). Tests stay isolated the other way: each has its own
/// Postgres database, and every test uses its own mart names.
static CLICKHOUSE: tokio::sync::OnceCell<(String, testcontainers::ContainerAsync<ClickHouse>)> =
    tokio::sync::OnceCell::const_new();

async fn shared_clickhouse() -> &'static (String, testcontainers::ContainerAsync<ClickHouse>) {
    CLICKHOUSE
        .get_or_init(|| async {
            // Exactly the compose service's env trio (CH_USER defaults to
            // "default", CH_PASSWORD to empty,
            // CLICKHOUSE_DEFAULT_ACCESS_MANAGEMENT pinned "1"). ALL THREE
            // are required: the 26.8 image's entrypoint disables network
            // access for `default` when CLICKHOUSE_USER is unset — even
            // with an empty password — and
            // CLICKHOUSE_DEFAULT_ACCESS_MANAGEMENT is what keeps
            // passwordless `default` reachable (the compose file documents
            // the same trio as its local-dev default).
            let container = ClickHouse::default()
                .with_env_var("CLICKHOUSE_USER", "default")
                .with_env_var("CLICKHOUSE_PASSWORD", "")
                .with_env_var("CLICKHOUSE_DEFAULT_ACCESS_MANAGEMENT", "1")
                .with_tag("26.8")
                .start()
                .await
                .expect("start the ClickHouse testcontainer");
            let host = container.get_host().await.expect("container host");
            let port = container
                .get_host_port_ipv4(8123)
                .await
                .expect("container HTTP port");
            (format!("http://{host}:{port}"), container)
        })
        .await
}

/// The tenant that owns this deployment's shared catalog -- the tenant
/// the deployment names in `CATALOG_TENANT_ID`.
const CATALOG_TENANT: &str = "11111111-1111-4111-8111-000000000001";

/// An app wired to the shared `ClickHouse`, plus a direct `ChClient` for
/// seeding `serving.*` marts.
async fn spin_up_with_clickhouse() -> (TestApp, ChClient) {
    let (ch_url, _container) = shared_clickhouse().await;
    let mut overrides = HashMap::new();
    overrides.insert("CH_URL".to_owned(), ch_url.clone());
    // `SEC-50`: the export reads the SHARED catalog, so it is gated
    // against the tenant that owns it -- the catalog's own tenant for a
    // job with no user, and the caller's own tenant for a signed-in
    // person (R4). A deployment that holds several tenants (the four the
    // seeded identity migration creates) and names no catalog tenant
    // refuses the export outright.
    overrides.insert("CATALOG_TENANT_ID".to_owned(), CATALOG_TENANT.to_owned());
    let app = spin_up_with_env(&overrides).await;
    let ch = ChClient::new(ch_url.clone(), "default".to_owned(), String::new());
    (app, ch)
}

/// The app of [`spin_up_with_clickhouse`], with a Gold export token file that
/// exists, so a `POST /api/gold/export/{mart}` gets past `read_catalog_token`
/// and fails at the harness's dead catalog. The file's content is any text: the
/// harness never presents it to anything. The returned file is kept alive by the
/// caller for the test's duration.
async fn spin_up_with_clickhouse_and_export_token() -> (TestApp, ChClient, tempfile::NamedTempFile)
{
    let (ch_url, _container) = shared_clickhouse().await;
    let token = tempfile::NamedTempFile::new().expect("a temporary token file");
    std::fs::write(token.path(), "test-token").expect("write the token file");
    let mut overrides = HashMap::new();
    overrides.insert("CH_URL".to_owned(), ch_url.clone());
    overrides.insert("CATALOG_TENANT_ID".to_owned(), CATALOG_TENANT.to_owned());
    overrides.insert(
        "LAKEKEEPER_GOLD_EXPORT_TOKEN_FILE".to_owned(),
        token.path().display().to_string(),
    );
    let app = spin_up_with_env(&overrides).await;
    let ch = ChClient::new(ch_url.clone(), "default".to_owned(), String::new());
    (app, ch, token)
}

/// Seeds `serving.{mart}` as a real `MergeTree` with a few rows — the same
/// schema-on-write shape `authored_factory.py` produces (`CREATE TABLE
/// ... ENGINE = MergeTree` + a plain `INSERT`), so `system.tables` and
/// `system.parts` both have a row for it.
async fn seed_mart(ch: &ChClient, mart: &str) {
    ch.exec("CREATE DATABASE IF NOT EXISTS serving", None)
        .await
        .expect("create the serving database");
    ch.exec(
        &format!(
            "CREATE TABLE IF NOT EXISTS serving.`{mart}` \
             (n UInt64) ENGINE = MergeTree ORDER BY n"
        ),
        None,
    )
    .await
    .unwrap_or_else(|e| panic!("create serving.{mart}: {e}"));
    ch.exec(
        &format!("INSERT INTO serving.`{mart}` SELECT number FROM numbers(5)"),
        None,
    )
    .await
    .unwrap_or_else(|e| panic!("insert into serving.{mart}: {e}"));
}

/// A minter for session cookies: each call creates a FRESH principal
/// (and cookie) so tests never share authenticated state.
struct Sessions {
    app: TestApp,
}

impl Sessions {
    /// A cookie for a principal holding exactly `permissions`.
    ///
    /// `SEC-50` R4: a signed-in person is gated as THEMSELVES, so the
    /// person who triggers a Gold export has to be a member of the tenant
    /// that owns the shared catalog — the tenant this deployment names in
    /// `CATALOG_TENANT_ID`. The membership is added here, where the
    /// principal is minted, so every test in this file has the same
    /// shape and no test has to remember it.
    async fn with(&self, permissions: &str) -> String {
        let user = create_principal_with_permissions(&self.app.pool, permissions).await;
        sqlx::query("INSERT INTO app_user_tenant (user_id, tenant_id) VALUES ($1, $2)")
            .bind(user)
            .bind(uuid::Uuid::parse_str(CATALOG_TENANT).expect("a seeded tenant id"))
            .execute(&self.app.pool)
            .await
            .expect("add the test principal to the catalog's own tenant");
        session_cookie_for_user(&self.app.pool, user).await
    }
}

async fn send(
    router: &axum::Router,
    method: &str,
    uri: &str,
    cookie: Option<&str>,
    json_body: Option<Value>,
) -> (StatusCode, Value) {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json");
    if let Some(cookie) = cookie {
        builder = builder.header("cookie", cookie);
    }
    let request = builder
        .body(Body::from(
            json_body.map_or_else(String::new, |v| v.to_string()),
        ))
        .expect("build request");
    let response = router
        .clone()
        .oneshot(request)
        .await
        .expect("in-memory request");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read body");
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, body)
}

/// `PUT /api/gold/export/{mart}/publication` is floored at
/// `RequiresPermission("gold:export")` in `POLICY_TABLE`: an
/// authenticated principal WITHOUT the permission must get the gate's
/// 403, not a 404/500 from the handler body.
#[tokio::test]
async fn put_publication_is_denied_to_a_principal_without_gold_export() {
    let (app, ch) = spin_up_with_clickhouse().await;
    seed_mart(&ch, "orders_rollup").await;
    let sessions = Sessions { app };
    let cookie = sessions.with("").await;

    let (status, _) = send(
        &sessions.app.router,
        "PUT",
        "/api/gold/export/orders_rollup/publication",
        Some(&cookie),
        Some(serde_json::json!({ "enabled": true })),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "403 from the auth gate");
}

/// The scheduler's list route has TWO layers: the `RequiresAuth` floor
/// (covered by `route_auth.rs`) PLUS `check_export_token` in the handler
/// (the plan's own requirement). A session that clears the floor but
/// holds neither `gold:export` nor the shared token must not read the
/// publication list.
#[tokio::test]
async fn publications_list_still_requires_the_export_permission_beyond_the_auth_floor() {
    let (app, _ch) = spin_up_with_clickhouse().await;
    let sessions = Sessions { app };
    let cookie = sessions.with("").await;

    let (status, _) = send(
        &sessions.app.router,
        "GET",
        "/api/gold/publications",
        Some(&cookie),
        None,
    )
    .await;
    assert!(
        status == StatusCode::UNAUTHORIZED || status == StatusCode::SERVICE_UNAVAILABLE,
        "check_export_token must refuse a session without gold:export, got {status}"
    );
}

/// `PUT` returns 404 when `serving.{mart}` does not exist — measured
/// against `system.tables`, so a typo cannot enable publishing for a
/// mart nobody can build. The permissioned session clears the gate,
/// proving the 404 comes from the handler, not the gate.
#[tokio::test]
async fn put_publication_returns_404_for_a_mart_that_does_not_exist() {
    let (app, _ch) = spin_up_with_clickhouse().await;
    let sessions = Sessions { app };
    let cookie = sessions.with("gold:export").await;

    let (status, body) = send(
        &sessions.app.router,
        "PUT",
        "/api/gold/export/no_such_mart/publication",
        Some(&cookie),
        Some(serde_json::json!({ "enabled": true })),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
}

/// The full toggle round trip over real Postgres + real `ClickHouse`:
/// enabling lists the mart for the scheduler; disabling removes it from
/// the list again while the detail route keeps reporting the stored
/// state (who switched it off last) — and a 200 on both PUTs with a
/// deliberately DEAD Lakekeeper proves neither direction of the switch
/// ever touches Iceberg (DATA-1: off is a flag flip, never a drop).
#[tokio::test]
async fn enabling_lists_a_mart_and_disabling_removes_it_from_the_list() {
    let (app, ch) = spin_up_with_clickhouse().await;
    seed_mart(&ch, "orders_rollup").await;
    let sessions = Sessions { app };
    let cookie = sessions.with("gold:export").await;

    // On.
    let (status, body) = send(
        &sessions.app.router,
        "PUT",
        "/api/gold/export/orders_rollup/publication",
        Some(&cookie),
        Some(serde_json::json!({ "enabled": true })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["enabled"], Value::Bool(true), "{body}");
    assert_eq!(body["canEdit"], Value::Bool(true), "{body}");

    // The scheduler's list sees exactly it, with a non-null updatedAt.
    let (status, body) = send(
        &sessions.app.router,
        "GET",
        "/api/gold/publications",
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let publications = body["publications"].as_array().expect("publications array");
    assert_eq!(publications.len(), 1, "{body}");
    assert_eq!(publications[0]["mart"], "orders_rollup", "{body}");
    assert!(
        publications[0]["updatedAt"].is_string(),
        "updatedAt must be present: {body}"
    );

    // Off.
    let (status, body) = send(
        &sessions.app.router,
        "PUT",
        "/api/gold/export/orders_rollup/publication",
        Some(&cookie),
        Some(serde_json::json!({ "enabled": false })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["enabled"], Value::Bool(false), "{body}");

    // The list is empty again...
    let (status, body) = send(
        &sessions.app.router,
        "GET",
        "/api/gold/publications",
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["publications"]
            .as_array()
            .expect("publications array")
            .len(),
        0,
        "{body}"
    );

    // ...but the detail route keeps the stored state (the row is kept so
    // the console can show who switched the mart off last), and reports
    // the MEASURED freshness facts: lastChangedAt from real parts,
    // lastExportedAt null because nothing was ever exported.
    let (status, body) = send(
        &sessions.app.router,
        "GET",
        "/api/gold/export/orders_rollup/publication",
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["enabled"], Value::Bool(false), "{body}");
    assert!(body["updatedAt"].is_string(), "updatedAt is kept: {body}");
    assert!(
        body["lastChangedAt"].is_string(),
        "a seeded MergeTree with rows has a measured lastChangedAt: {body}"
    );
    assert!(
        body["lastExportedAt"].is_null(),
        "never exported -> null, never a guessed time: {body}"
    );
    assert!(
        body["changedSinceExport"].is_null(),
        "never exported -> changedSinceExport null: {body}"
    );
}

/// A mart that exists but was never switched on reports the OFF default:
/// `enabled: false, updatedAt: null`, honest nulls for both freshness
/// facts it cannot have yet, and `canEdit: false` for a session without
/// `gold:export` (the floor is only `RequiresAuth` here — read-only).
#[tokio::test]
async fn detail_defaults_for_a_never_enabled_mart_are_the_off_state() {
    let (app, ch) = spin_up_with_clickhouse().await;
    seed_mart(&ch, "plain_mart").await;
    let sessions = Sessions { app };
    let cookie = sessions.with("").await;

    let (status, body) = send(
        &sessions.app.router,
        "GET",
        "/api/gold/export/plain_mart/publication",
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["mart"], "plain_mart", "{body}");
    assert_eq!(body["enabled"], Value::Bool(false), "{body}");
    assert!(body["updatedAt"].is_null(), "{body}");
    assert!(body["lastExportedAt"].is_null(), "{body}");
    assert!(body["changedSinceExport"].is_null(), "{body}");
    assert!(body["lastChangedAt"].is_string(), "{body}");
    assert_eq!(body["canEdit"], Value::Bool(false), "{body}");
}

// ── T4: POST /api/gold/export/{mart}?ifChanged=true ─────────────────────

#[tokio::test]
async fn publication_detail_reports_changed_since_export_accurately() {
    let (app, ch) = spin_up_with_clickhouse().await;
    seed_mart(&ch, "fp_status_mart").await;
    let sessions = Sessions { app };
    let cookie = sessions.with("gold:export").await;

    // 1. Never exported -> changedSinceExport is null.
    let (status, body) = send(
        &sessions.app.router,
        "GET",
        "/api/gold/export/fp_status_mart/publication",
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body["changedSinceExport"].is_null(), "{body}");

    // 2. Exported with current fingerprint -> changedSinceExport is false (Up to date).
    let fp =
// DATA-10 review SHOULD-FIX 1: mart_content_fingerprint in gold_export_history
        lakehouse_api::gold_export_history::mart_content_fingerprint(&ch, "serving", "fp_status_mart")
            .await
            .expect("fingerprint read")
            .expect("fingerprint present");
    seed_success_run_with_fingerprint(&ch, "fp_status_mart", unix_ms(-10), &fp).await;

    let (status, body) = send(
        &sessions.app.router,
        "GET",
        "/api/gold/export/fp_status_mart/publication",
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["changedSinceExport"], Value::Bool(false), "{body}");

    // 3. Mutate table -> changedSinceExport is true (Out of date).
    ch.exec("INSERT INTO serving.fp_status_mart (n) VALUES (999)", None)
        .await
        .expect("insert new row");

    let (status, body) = send(
        &sessions.app.router,
        "GET",
        "/api/gold/export/fp_status_mart/publication",
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["changedSinceExport"], Value::Bool(true), "{body}");
}

use lakehouse_api::gold_export_history;

/// The unix-millisecond timestamp `offset_secs` from now, for history
/// rows seeded relative to the mart's own `modification_time`.
fn unix_ms(offset_secs: i64) -> i64 {
    let now_secs = i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock is after the epoch")
            .as_secs(),
    )
    .expect("a plausible epoch second");
    now_secs * 1000 + offset_secs * 1000
}

async fn seed_success_run_with_fingerprint(
    ch: &ChClient,
    mart: &str,
    started_ms: i64,
    content_fingerprint: &str,
) {
    gold_export_history::record_export_run(
        ch,
        &gold_export_history::NewGoldExportRun {
            mart,
            status: "success",
            rows_exported: Some(5),
            format_version: Some(2),
            snapshot_id: None,
            error: None,
            triggered_by: "user:test",
            content_fingerprint,
            started_at_ms: started_ms,
            finished_at_ms: started_ms,
        },
    )
    .await
    .expect("seed a successful export history row");
}

#[allow(dead_code)]
async fn seed_success_run(ch: &ChClient, mart: &str, started_ms: i64) {
    seed_success_run_with_fingerprint(ch, mart, started_ms, "").await;
}

async fn history_row_count(ch: &ChClient, mart: &str) -> u64 {
    let rows = ch
        .rows(
            &format!(
                "SELECT toString(count()) AS n FROM console.gold_export_run \
                 WHERE mart = '{mart}'"
            ),
            None,
        )
        .await
        .expect("count history rows");
    rows[0]["n"]
        .as_str()
        .expect("toString(count()) is a string")
        .parse::<u64>()
        .expect("a row count")
}

/// The skip case: the mart's content fingerprint MATCHES its last
/// successful export, so `?ifChanged=true` must answer 200 with
/// `skipped: true` and write NOTHING — the history row count is still
/// exactly the one seeded row, and no export was attempted (the dead
/// Lakekeeper is never reached: the handler returned before
/// `read_catalog_token`).
#[tokio::test]
async fn if_changed_skips_an_unchanged_mart_and_writes_no_history_row() {
    let (app, ch) = spin_up_with_clickhouse().await;
    seed_mart(&ch, "quiet_mart").await;
    // DATA-10 review SHOULD-FIX 1: mart_content_fingerprint in gold_export_history
    let fp =
        lakehouse_api::gold_export_history::mart_content_fingerprint(&ch, "serving", "quiet_mart")
            .await
            .expect("read fingerprint")
            .expect("quiet_mart has a fingerprint");
    seed_success_run_with_fingerprint(&ch, "quiet_mart", unix_ms(-3_600), &fp).await;
    let sessions = Sessions { app };
    let cookie = sessions.with("gold:export").await;

    let (status, body) = send(
        &sessions.app.router,
        "POST",
        "/api/gold/export/quiet_mart?ifChanged=true",
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["skipped"], Value::Bool(true), "{body}");
    let reason = body["reason"].as_str().expect("a reason string");
    assert!(
        reason.starts_with("unchanged since "),
        "the reason names the export it is fresh as of: {reason}"
    );

    assert_eq!(
        history_row_count(&ch, "quiet_mart").await,
        1,
        "a skip writes no console.gold_export_run row"
    );
}

/// A mart whose content fingerprint differs from its last export must NOT skip:
/// the handler proceeds into the export path, which with no Lakekeeper provisioned
/// stops deterministically at `read_catalog_token` (503 Unavailable,
/// fixed text) — proof it got past the skip gate and attempted the export.
#[tokio::test]
async fn if_changed_exports_a_mart_with_differing_fingerprint() {
    let (app, ch) = spin_up_with_clickhouse().await;
    seed_mart(&ch, "busy_mart").await;
    // The last export has a differing fingerprint.
    seed_success_run_with_fingerprint(&ch, "busy_mart", unix_ms(-3_600), "0:0:differing").await;
    let sessions = Sessions { app };
    let cookie = sessions.with("gold:export").await;

    let (status, body) = send(
        &sessions.app.router,
        "POST",
        "/api/gold/export/busy_mart?ifChanged=true",
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert!(body.get("skipped").is_none(), "{body}");
    assert_eq!(
        history_row_count(&ch, "busy_mart").await,
        2,
        "the seeded row plus one failed run: the 503 at read_catalog_token is \
         now a recorded failed publish (DATA-56), corrected from 1"
    );
}

/// A pre-DATA-10 export row has `content_fingerprint`: "" -> empty stored fingerprint
/// must NOT skip: fails toward publishing (503 at `read_catalog_token`).
#[tokio::test]
async fn if_changed_exports_when_stored_fingerprint_is_empty() {
    let (app, ch) = spin_up_with_clickhouse().await;
    seed_mart(&ch, "legacy_mart").await;
    seed_success_run_with_fingerprint(&ch, "legacy_mart", unix_ms(-3_600), "").await;
    let sessions = Sessions { app };
    let cookie = sessions.with("gold:export").await;

    let (status, body) = send(
        &sessions.app.router,
        "POST",
        "/api/gold/export/legacy_mart?ifChanged=true",
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert!(body.get("skipped").is_none(), "{body}");
}

/// No successful export ever recorded -> lastExportedAt is unknown ->
/// never skip: export (which again stops deterministically at the dead
/// Lakekeeper's missing token file, 503).
#[tokio::test]
async fn if_changed_exports_when_the_last_export_is_unknown() {
    let (app, ch) = spin_up_with_clickhouse().await;
    seed_mart(&ch, "fresh_mart").await;
    let sessions = Sessions { app };
    let cookie = sessions.with("gold:export").await;

    let (status, body) = send(
        &sessions.app.router,
        "POST",
        "/api/gold/export/fresh_mart?ifChanged=true",
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert!(body.get("skipped").is_none(), "{body}");
}

async fn seed_trim_run(
    ch: &ChClient,
    mart: &str,
    status: &str,
    copies_kept: Option<u32>,
    oldest_kept_ms: Option<i64>,
    finished_ms: i64,
    reason: Option<&str>,
) {
    gold_export_history::ensure_gold_trim_run_table(ch)
        .await
        .expect("ensure trim table");
    let oldest_sql = oldest_kept_ms.map_or_else(
        || "NULL".to_owned(),
        |ms| format!("fromUnixTimestamp64Milli({ms})"),
    );
    let reason_sql = reason.map_or_else(|| "NULL".to_owned(), |r| format!("'{r}'"));
    let copies_sql = copies_kept.map_or_else(|| "NULL".to_owned(), |cp| cp.to_string());
    ch.exec(
        &format!(
            "INSERT INTO console.gold_trim_run              (id, mart, status, copies_kept, oldest_kept_exported_at,               copies_removed, snapshots_expired, files_removed, reason,               started_at, finished_at) VALUES              ('{id}', '{mart}', '{status}', {copies_sql}, {oldest_sql},               0, 0, 0, {reason_sql}, fromUnixTimestamp64Milli({finished_ms}),               fromUnixTimestamp64Milli({finished_ms}))",
            id = uuid::Uuid::new_v4(),
        ),
        None,
    )
    .await
    .expect("seed trim run row");
}

#[tokio::test]
async fn publication_detail_reports_trim_null_when_never_trimmed() {
    let (app, ch) = spin_up_with_clickhouse().await;
    seed_mart(&ch, "untrimmed_mart").await;
    let sessions = Sessions { app };
    let cookie = sessions.with("gold:export").await;

    let (status, body) = send(
        &sessions.app.router,
        "GET",
        "/api/gold/export/untrimmed_mart/publication",
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        body["trim"].is_null(),
        "expected trim to be null, got: {body}"
    );
}

#[tokio::test]
async fn publication_detail_reports_trim_success() {
    let (app, ch) = spin_up_with_clickhouse().await;
    seed_mart(&ch, "trimmed_ok_mart").await;
    seed_trim_run(
        &ch,
        "trimmed_ok_mart",
        "success",
        Some(7),
        Some(1_700_000_000_000),
        1_700_000_010_000,
        None,
    )
    .await;
    let sessions = Sessions { app };
    let cookie = sessions.with("gold:export").await;

    let (status, body) = send(
        &sessions.app.router,
        "GET",
        "/api/gold/export/trimmed_ok_mart/publication",
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["trim"]["status"], "success");
    assert_eq!(body["trim"]["copiesKept"], 7);
    assert!(body["trim"]["oldestKeptExportedAt"].is_string());
    assert!(body["trim"]["finishedAt"].is_string());
    assert!(body["trim"]["reason"].is_null());
}

#[tokio::test]
async fn publication_detail_reports_trim_failed() {
    let (app, ch) = spin_up_with_clickhouse().await;
    seed_mart(&ch, "trimmed_fail_mart").await;
    seed_trim_run(
        &ch,
        "trimmed_fail_mart",
        "failed",
        None,
        None,
        1_700_000_010_000,
        Some("delete_failed"),
    )
    .await;
    let sessions = Sessions { app };
    let cookie = sessions.with("gold:export").await;

    let (status, body) = send(
        &sessions.app.router,
        "GET",
        "/api/gold/export/trimmed_fail_mart/publication",
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["trim"]["status"], "failed");
    assert!(body["trim"]["copiesKept"].is_null());
    assert!(body["trim"]["oldestKeptExportedAt"].is_null());
    assert!(body["trim"]["finishedAt"].is_string());
    assert_eq!(body["trim"]["reason"], "delete_failed");
}

/// QA `LH-DATA-1-01` (`DATA-49`): a failed publish stores the sentence the
/// response carried. The catalog is unreachable in this harness, so the run
/// fails there; the row must read the classified sentence with its reference,
/// never the catalog client's own text (which names the HTTP answer and its
/// headers), and the row and the response must agree word for word.
#[tokio::test]
async fn a_failed_publish_stores_the_sentence_the_response_carried_and_no_catalog_text() {
    let (app, ch, _token) = spin_up_with_clickhouse_and_export_token().await;
    seed_mart(&ch, "catalog_down_mart").await;
    let sessions = Sessions { app };
    let cookie = sessions.with("gold:export").await;

    let (status, body) = send(
        &sessions.app.router,
        "POST",
        "/api/gold/export/catalog_down_mart",
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{body}");
    let answered = body["error"]
        .as_str()
        .expect("a failed publish answers a sentence")
        .to_owned();
    assert!(
        answered.starts_with("The catalog request failed. Reference: "),
        "{answered}"
    );
    assert!(
        !answered.contains("http://"),
        "the catalog's address must not travel: {answered}"
    );

    let (status, body) = send(
        &sessions.app.router,
        "GET",
        "/api/gold/exports?mart=catalog_down_mart",
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let newest = &body["runs"][0];
    assert_eq!(newest["status"], "failed", "{body}");
    assert_eq!(
        newest["error"], answered,
        "the stored sentence is the response's own: {body}"
    );
}

/// The row-cap sentence this repository wrote, which the rewrite must leave
/// alone.
const ROW_CAP_SENTENCE: &str = "mart \"raw_history_mart\" has 5000001 rows, exceeding \
                                GOLD_EXPORT_MAX_ROWS (5000000); refusing to export a partial mart silently";

/// Seeds one history row per raw shape this code used to store, plus a failed
/// row with this repository's own sentence and one success — each a different
/// second, so the newest-first order is fixed.
async fn seed_history_rows_with_raw_text(ch: &ChClient, mart: &str) {
    for (offset, status, error) in [
        (
            -50,
            "failed",
            Some(
                "reading Gold mart from ClickHouse failed: Code: 60. DB::Exception: \
                 Unknown table (version 26.8.1.1)",
            ),
        ),
        (
            -40,
            "failed",
            Some(
                "Iceberg export failed: iceberg catalog operation failed: \
                 status: 401 Unauthorized, headers: {www-authenticate: Bearer}",
            ),
        ),
        (
            -30,
            "failed",
            Some("failed to build export batch: amount: \"12,5\" is not a valid decimal"),
        ),
        (-20, "failed", Some(ROW_CAP_SENTENCE)),
        (-10, "success", None),
    ] {
        gold_export_history::record_export_run(
            ch,
            &gold_export_history::NewGoldExportRun {
                mart,
                status,
                rows_exported: if status == "success" { Some(5) } else { None },
                format_version: None,
                snapshot_id: None,
                error,
                triggered_by: "user:test",
                content_fingerprint: "",
                started_at_ms: unix_ms(offset),
                finished_at_ms: unix_ms(offset),
            },
        )
        .await
        .expect("seed a history row");
    }
}

/// QA `LH-DATA-1-01` (`DATA-49`): the rows written before a failed publish was
/// classified hold the engine's or the catalog's own text. The list route never
/// shows it, and `rewrite_raw_errors` replaces it at rest — once, then never
/// again — while a sentence this repository wrote stays as it is.
#[tokio::test]
async fn stored_upstream_text_is_rewritten_at_rest_and_never_listed() {
    let (app, ch) = spin_up_with_clickhouse().await;
    seed_mart(&ch, "raw_history_mart").await;
    let sessions = Sessions { app };
    let cookie = sessions.with("gold:export").await;
    seed_history_rows_with_raw_text(&ch, "raw_history_mart").await;

    // The read path maps the raw shapes before a row leaves the API.
    let (status, body) = send(
        &sessions.app.router,
        "GET",
        "/api/gold/exports?mart=raw_history_mart",
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let listed: Vec<&str> = body["runs"]
        .as_array()
        .expect("runs")
        .iter()
        .map(|run| run["error"].as_str().unwrap_or(""))
        .collect();
    assert_eq!(
        listed,
        [
            "",
            ROW_CAP_SENTENCE,
            "A value in this table could not be written in open format.",
            "The catalog request failed.",
            "The database request failed.",
        ],
        "newest first: the success, the row cap, and one fixed sentence per raw shape: {body}"
    );

    // The rewrite replaces exactly the three raw shapes, at rest.
    let rewritten = gold_export_history::rewrite_raw_errors(&ch)
        .await
        .expect("the rewrite runs");
    assert_eq!(rewritten, 3, "three rows held a raw shape");

    let stored = ch
        .rows(
            "SELECT status, error FROM console.gold_export_run \
             WHERE mart = 'raw_history_mart' ORDER BY started_at ASC FORMAT JSON",
            None,
        )
        .await
        .expect("read the stored rows");
    let stored_errors: Vec<Option<&str>> = stored
        .iter()
        .map(|row| row.get("error").and_then(Value::as_str))
        .collect();
    assert_eq!(
        stored_errors,
        [
            Some("The database request failed."),
            Some("The catalog request failed."),
            Some("A value in this table could not be written in open format."),
            Some(ROW_CAP_SENTENCE),
            None,
        ],
        "the row cap is this repository's own sentence and stays"
    );

    assert_eq!(
        gold_export_history::rewrite_raw_errors(&ch)
            .await
            .expect("the second run reads the count"),
        0,
        "nothing matches after the rewrite"
    );
}

/// One history row of any status, for the `DATA-49` tests that need a failed
/// run beside a successful one.
async fn seed_run(
    ch: &ChClient,
    mart: &str,
    status: &str,
    started_ms: i64,
    content_fingerprint: &str,
    error: Option<&str>,
) {
    gold_export_history::record_export_run(
        ch,
        &gold_export_history::NewGoldExportRun {
            mart,
            status,
            rows_exported: if status == "success" { Some(5) } else { None },
            format_version: if status == "success" { Some(2) } else { None },
            snapshot_id: None,
            error,
            triggered_by: "user:test",
            content_fingerprint,
            started_at_ms: started_ms,
            finished_at_ms: started_ms,
        },
    )
    .await
    .expect("seed an export history row");
}

/// QA `LH-DATA-1-01` (`DATA-49`): after a failed publish the publication says so
/// — with the time and the stored sentence — while `lastExportedAt` still names
/// the last SUCCESSFUL publish. A later success clears the failure.
#[tokio::test]
async fn publication_detail_reports_the_last_failure_until_a_later_run_succeeds() {
    let (app, ch) = spin_up_with_clickhouse().await;
    seed_mart(&ch, "failure_status_mart").await;
    let fingerprint =
        gold_export_history::mart_content_fingerprint(&ch, "serving", "failure_status_mart")
            .await
            .expect("read the fingerprint")
            .expect("the mart has a fingerprint");
    seed_run(
        &ch,
        "failure_status_mart",
        "success",
        unix_ms(-3_600),
        &fingerprint,
        None,
    )
    .await;
    seed_run(
        &ch,
        "failure_status_mart",
        "failed",
        unix_ms(-60),
        &fingerprint,
        Some("The catalog request failed. Reference: abc123"),
    )
    .await;
    let sessions = Sessions { app };
    let cookie = sessions.with("gold:export").await;

    let (status, body) = send(
        &sessions.app.router,
        "GET",
        "/api/gold/export/failure_status_mart/publication",
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["lastFailure"]["reason"], "The catalog request failed. Reference: abc123",
        "{body}"
    );
    assert!(
        body["lastFailure"]["failedAt"].is_string(),
        "the failure names when it happened: {body}"
    );
    assert!(
        body["lastExportedAt"].is_string(),
        "the last successful publish is still reported: {body}"
    );

    // A later success clears the failure and moves lastExportedAt.
    seed_run(
        &ch,
        "failure_status_mart",
        "success",
        unix_ms(0),
        &fingerprint,
        None,
    )
    .await;
    let (status, body) = send(
        &sessions.app.router,
        "GET",
        "/api/gold/export/failure_status_mart/publication",
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        body["lastFailure"].is_null(),
        "a success after the failure clears it: {body}"
    );
}

/// QA `LH-DATA-1-01` (`DATA-49`): `?ifChanged=true` must not skip a mart whose
/// newest run failed, even when its fingerprint matches the last success — the
/// run has to be attempted again, which in this harness means reaching the
/// missing token file (503), not answering `skipped`.
#[tokio::test]
async fn if_changed_does_not_skip_after_a_failed_run() {
    let (app, ch) = spin_up_with_clickhouse().await;
    seed_mart(&ch, "retry_after_failure_mart").await;
    let fingerprint =
        gold_export_history::mart_content_fingerprint(&ch, "serving", "retry_after_failure_mart")
            .await
            .expect("read the fingerprint")
            .expect("the mart has a fingerprint");
    seed_run(
        &ch,
        "retry_after_failure_mart",
        "success",
        unix_ms(-3_600),
        &fingerprint,
        None,
    )
    .await;
    seed_run(
        &ch,
        "retry_after_failure_mart",
        "failed",
        unix_ms(-60),
        &fingerprint,
        Some("The database request failed. Reference: abc123"),
    )
    .await;
    let sessions = Sessions { app };
    let cookie = sessions.with("gold:export").await;

    let (status, body) = send(
        &sessions.app.router,
        "POST",
        "/api/gold/export/retry_after_failure_mart?ifChanged=true",
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert!(
        body.get("skipped").is_none(),
        "a failed newest run is published again, never skipped: {body}"
    );
    assert_eq!(
        history_row_count(&ch, "retry_after_failure_mart").await,
        3,
        "the two seeded rows plus one failed run: the 503 at read_catalog_token is \
         now a recorded failed publish (DATA-56), corrected from 2"
    );
}

/// #484 review R4 (`DATA-56`): a publish that fails before the export starts
/// still leaves a run, so the card says "Last publish
/// failed". Here the catalog's token file is missing: the caller is allowed,
/// the mart exists, and the answer is the fixed sentence of
/// `token_unavailable_message` — never a path or an io error.
#[tokio::test]
async fn a_publish_that_cannot_read_the_catalog_token_is_recorded_and_the_card_says_it_failed() {
    let (app, ch) = spin_up_with_clickhouse().await;
    seed_mart(&ch, "tokenless_mart").await;
    let sessions = Sessions { app };
    let cookie = sessions.with("gold:export").await;
    let sentence = lakehouse_api::lakekeeper_token::token_unavailable_message(
        "gold-export",
        "LAKEKEEPER_GOLD_EXPORT_TOKEN_FILE",
    );

    let (status, body) = send(
        &sessions.app.router,
        "POST",
        "/api/gold/export/tokenless_mart",
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert_eq!(body["error"].as_str(), Some(sentence.as_str()), "{body}");

    let (status, body) = send(
        &sessions.app.router,
        "GET",
        "/api/gold/export/tokenless_mart/publication",
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let reason = body["lastFailure"]["reason"]
        .as_str()
        .unwrap_or_else(|| panic!("the card names the failure: {body}"));
    assert!(reason.starts_with(&sentence), "{body}");
    assert_eq!(reason, sentence, "the card and the answer read the same");
    assert!(
        body["lastFailure"]["failedAt"].is_string(),
        "the failure names when it happened: {body}"
    );
    assert!(
        body["lastExportedAt"].is_null(),
        "nothing was ever published: {body}"
    );

    let (status, body) = send(
        &sessions.app.router,
        "GET",
        "/api/gold/exports?mart=tokenless_mart",
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["runs"][0]["status"], "failed", "{body}");
    assert_eq!(
        body["runs"][0]["error"].as_str(),
        Some(sentence.as_str()),
        "the history row carries the answer's sentence: {body}"
    );
    assert_eq!(
        history_row_count(&ch, "tokenless_mart").await,
        1,
        "one publish, one row, even though the export never started"
    );
}

/// `DATA-56` D22: a scheduler run (`?ifChanged=true`) that fails before the
/// export is recorded every time it is tried, so a mart whose every scheduled
/// publish fails never looks quiet.
#[tokio::test]
async fn a_scheduled_publish_that_fails_before_the_export_is_recorded_each_time() {
    let (app, ch) = spin_up_with_clickhouse().await;
    seed_mart(&ch, "scheduled_tokenless_mart").await;
    let sessions = Sessions { app };
    let cookie = sessions.with("gold:export").await;

    for expected in 1..=2 {
        let (status, body) = send(
            &sessions.app.router,
            "POST",
            "/api/gold/export/scheduled_tokenless_mart?ifChanged=true",
            Some(&cookie),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
        assert!(
            body.get("skipped").is_none(),
            "a failed publish is not a skip: {body}"
        );
        assert_eq!(
            history_row_count(&ch, "scheduled_tokenless_mart").await,
            expected,
            "run {expected} is recorded"
        );
    }
}

/// `DATA-56` D20: the sentence a fingerprint read that failed stores is the
/// classified one — a fixed message and a reference — and never the engine's
/// own text or the name of the table that is gone.
#[tokio::test]
async fn a_publish_whose_fingerprint_read_fails_is_recorded_with_the_classified_sentence() {
    let (app, ch) = spin_up_with_clickhouse().await;
    seed_mart(&ch, "ghost_base_mart").await;
    ch.exec(
        "CREATE VIEW serving.ghost_view_mart AS SELECT * FROM serving.ghost_base_mart",
        None,
    )
    .await
    .expect("create a view over the seeded mart");
    ch.exec("DROP TABLE serving.ghost_base_mart", None)
        .await
        .expect("drop the table the view reads");
    let sessions = Sessions { app };
    let cookie = sessions.with("gold:export").await;

    let (status, body) = send(
        &sessions.app.router,
        "POST",
        "/api/gold/export/ghost_view_mart",
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    let sentence = body["error"]
        .as_str()
        .unwrap_or_else(|| panic!("the answer carries a sentence: {body}"))
        .to_owned();
    assert!(
        sentence.starts_with("The database request failed. Reference: "),
        "{body}"
    );
    assert!(
        !sentence.contains("ghost_base_mart"),
        "the engine's own text never travels: {body}"
    );

    let (status, body) = send(
        &sessions.app.router,
        "GET",
        "/api/gold/exports?mart=ghost_view_mart",
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["runs"][0]["status"], "failed", "{body}");
    assert_eq!(
        body["runs"][0]["error"].as_str(),
        Some(sentence.as_str()),
        "the history row carries the answer's sentence: {body}"
    );
    assert_eq!(
        history_row_count(&ch, "ghost_view_mart").await,
        1,
        "the failed fingerprint read is recorded"
    );
}

/// `DATA-56` D19: a request that is turned away is not a publish and records
/// nothing — no permission, a mart of another tenant, or a name that is not a
/// mart at all (which goes on and fails where it failed before, `D23`).
#[tokio::test]
async fn a_refused_publish_and_a_name_that_is_not_a_mart_record_no_run() {
    // The tenant that does NOT own the shared catalog: a member of it is
    // answered 404 by the gate, and 404 is not a failed publish.
    const OTHER_TENANT: &str = "11111111-1111-4111-8111-000000000002";

    let (app, ch) = spin_up_with_clickhouse().await;
    seed_mart(&ch, "refused_mart").await;
    // A refused request never creates the table the count reads.
    gold_export_history::ensure_gold_export_run_table(&ch)
        .await
        .expect("ensure the history table");
    let sessions = Sessions { app };

    // (a) A session without `gold:export`.
    let cookie = sessions.with("query:read").await;
    let (status, body) = send(
        &sessions.app.router,
        "POST",
        "/api/gold/export/refused_mart",
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert!(
        body["error"]
            .as_str()
            .is_some_and(|text| text.starts_with("gold export is not configured")),
        "{body}"
    );
    assert_eq!(
        history_row_count(&ch, "refused_mart").await,
        0,
        "a turned-away call is not a publish"
    );

    // (b) A person holding `gold:export`, in another tenant than the
    // catalog's own.
    let outsider = create_principal_with_permissions(&sessions.app.pool, "gold:export").await;
    sqlx::query("INSERT INTO app_user_tenant (user_id, tenant_id) VALUES ($1, $2)")
        .bind(outsider)
        .bind(uuid::Uuid::parse_str(OTHER_TENANT).expect("a seeded tenant id"))
        .execute(&sessions.app.pool)
        .await
        .expect("add the principal to another tenant");
    let cookie = session_cookie_for_user(&sessions.app.pool, outsider).await;
    let (status, body) = send(
        &sessions.app.router,
        "POST",
        "/api/gold/export/refused_mart",
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(
        history_row_count(&ch, "refused_mart").await,
        0,
        "a mart of another tenant is not published"
    );

    // (c) A name that was never seeded: it goes on as before and stops at
    // the missing token file, and nothing is recorded for a name that is not
    // a table.
    let cookie = sessions.with("gold:export").await;
    let (status, body) = send(
        &sessions.app.router,
        "POST",
        "/api/gold/export/never_seeded_mart",
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert_eq!(
        history_row_count(&ch, "never_seeded_mart").await,
        0,
        "a name that is not a table is not a publish (DATA-56 D23)"
    );
}
