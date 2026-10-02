//! HTTP-level acceptance tests for gold-publish-per-mart plan T3: the
//! per-mart publication routes (`GET /api/gold/publications`, and the
//! detail/`PUT` pair on `/api/gold/export/{mart}/publication`).
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

/// An app wired to the shared `ClickHouse`, plus a direct `ChClient` for
/// seeding `serving.*` marts.
async fn spin_up_with_clickhouse() -> (TestApp, ChClient) {
    let (ch_url, _container) = shared_clickhouse().await;
    let mut overrides = HashMap::new();
    overrides.insert("CH_URL".to_owned(), ch_url.clone());
    let app = spin_up_with_env(&overrides).await;
    let ch = ChClient::new(ch_url.clone(), "default".to_owned(), String::new());
    (app, ch)
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
    async fn with(&self, permissions: &str) -> String {
        let user = create_principal_with_permissions(&self.app.pool, permissions).await;
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
    assert!(body["lastChangedAt"].is_string(), "{body}");
    assert_eq!(body["canEdit"], Value::Bool(false), "{body}");
}
