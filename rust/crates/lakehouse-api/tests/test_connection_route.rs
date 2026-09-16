//! WS3 item 15: `POST /api/connectors/{id}/test` covers mysql/mssql
//! adapters at the route level, using the same live-pool harness
//! `tests/route_auth.rs`/`tests/debezium_properties.rs` already use.
//!
//! `POST /api/connectors/{id}/test`'s dispatch onto `probe_mysql`/
//! `probe_mssql` (`rust/crates/lakehouse-api/src/connector_probe.rs`)
//! already landed in an earlier commit (WS3 item 13) — these tests add
//! regression coverage of that dispatch at the ROUTE level (the actual
//! HTTP handler, a real seeded row, a real authenticated session), which
//! the in-crate `routes::connectors::tests` module cannot do: that
//! module's `state_without_pool()` helper only ever builds an
//! `AppState` with `pg: None` (`routes/connectors.rs:437+`), because
//! `AppState::new` only ever receives a `DATABASE_URL` string, never an
//! already-open pool, so a `#[sqlx::test]`-provided pool cannot be handed
//! to it directly. `tests/common/mod.rs`'s `spin_up`/`TestApp`/
//! `session_cookie_for_seeded_user` harness is the one that can.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use serde_json::Value;
use tower::ServiceExt;

use common::{session_cookie_for_seeded_user, spin_up};

#[tokio::test]
async fn test_connection_route_reports_unsupported_dial_not_unsupported_type_for_mysql() {
    let app = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&app.pool, "bayu@meridian.example").await;

    // A "sql" adapter, "mysql" driver connector pointed at a
    // non-listening local port -- the dial fails, which is a DIFFERENT
    // outcome than "this adapter type is not implemented at all"
    // (connector_probe.rs's probe_mysql already implements this dial,
    // WS3 item 13).
    sqlx::query(
        "INSERT INTO connector (id, name, type, direction, host, secret_ref, environment, tenant, \
         adapter, ingest_mode, dial) VALUES \
         ('conn-mysql-test', 'mysql test', 'MySQL', 'source', 'unused', 'env:CONNECTOR_MYSQL_PASSWORD', \
         'production', 'meridian', 'sql', 'batch', \
         '{\"driver\":\"mysql\",\"host\":\"127.0.0.1\",\"port\":1,\"database\":\"d\",\"user\":\"u\"}'::jsonb)",
    )
    .execute(&app.pool)
    .await
    .expect("seed a mysql-adapter connector");

    let response = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/connectors/conn-mysql-test/test")
                .header("cookie", &cookie)
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("router never fails a request outright");

    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read body");
    let body: Value = serde_json::from_slice(&bytes).expect("valid JSON");
    assert_eq!(
        body["supported"], true,
        "mysql IS implemented -- the dial failed, not the adapter type"
    );
    assert_eq!(
        body["ok"], false,
        "port 1 refuses instantly, so the probe must report failure"
    );
}

/// The `mssql` counterpart to the mysql test above — same shape, `tiberius`
/// driver dispatch (`connector_probe.rs`'s `probe_mssql`) instead of the
/// `MySQL` one.
#[tokio::test]
async fn test_connection_route_reports_unsupported_dial_not_unsupported_type_for_mssql() {
    let app = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&app.pool, "bayu@meridian.example").await;

    sqlx::query(
        "INSERT INTO connector (id, name, type, direction, host, secret_ref, environment, tenant, \
         adapter, ingest_mode, dial) VALUES \
         ('conn-mssql-test', 'mssql test', 'SQL Server', 'source', 'unused', 'env:CONNECTOR_MSSQL_PASSWORD', \
         'production', 'meridian', 'sql', 'batch', \
         '{\"driver\":\"mssql\",\"host\":\"127.0.0.1\",\"port\":1,\"database\":\"d\",\"user\":\"u\"}'::jsonb)",
    )
    .execute(&app.pool)
    .await
    .expect("seed an mssql-adapter connector");

    let response = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/connectors/conn-mssql-test/test")
                .header("cookie", &cookie)
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("router never fails a request outright");

    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read body");
    let body: Value = serde_json::from_slice(&bytes).expect("valid JSON");
    assert_eq!(
        body["supported"], true,
        "mssql IS implemented -- the dial failed, not the adapter type"
    );
    assert_eq!(
        body["ok"], false,
        "port 1 refuses instantly, so the probe must report failure"
    );
}

/// `GET /api/connectors/{id}/debezium-properties` for a `mysql`-driver
/// `sql`/`cdc` connector: the rendered template must name the real
/// Debezium `MySQL` connector class, and still carry only `${ENV_VAR_NAME}`
/// references for every credential-shaped field — never a resolved
/// secret. This is the route-level counterpart to
/// `lakehouse-store::cdc`'s pure
/// `template_renders_the_mysql_connector_class_for_a_mysql_source` unit
/// test: it proves the HTTP handler actually reaches that renderer with
/// the right `connector_class` for a non-Postgres driver, not just that
/// the renderer itself is correct in isolation.
#[tokio::test]
async fn debezium_properties_route_names_the_mysql_connector_class_and_leaks_no_secret() {
    let app = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&app.pool, "bayu@meridian.example").await;

    sqlx::query(
        "INSERT INTO connector (id, name, type, direction, host, secret_ref, environment, tenant, \
         adapter, ingest_mode, dial) VALUES \
         ('conn-mysql-cdc-test', 'mysql cdc test', 'MySQL CDC', 'source', 'unused', \
         'env:CONNECTOR_MYSQL_CDC_PASSWORD', 'production', 'meridian', 'cdc', 'cdc', \
         '{\"driver\":\"mysql\",\"host\":\"mysql-src\",\"port\":3306,\"database\":\"oms\",\"user\":\"repl\",\
         \"slotName\":\"orders_slot\",\"publicationName\":\"orders_pub\"}'::jsonb)",
    )
    .execute(&app.pool)
    .await
    .expect("seed a mysql-cdc-adapter connector");

    let response = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/connectors/conn-mysql-cdc-test/debezium-properties?table=oms.orders")
                .header("cookie", &cookie)
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("router never fails a request outright");

    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read body");
    let body: Value = serde_json::from_slice(&bytes).expect("valid JSON");
    let properties = body["properties"].as_str().expect("properties field");

    assert!(
        properties
            .contains("debezium.source.connector.class=io.debezium.connector.mysql.MySqlConnector"),
        "must name the real Debezium MySQL connector class: {properties}"
    );
    assert!(
        properties.contains("debezium.source.database.password=${CONNECTOR_MYSQL_CDC_PASSWORD}"),
        "must reference this connector's own secretRef by name, never resolve it: {properties}"
    );
    // The property that must hold end to end: nothing resolved-looking
    // ever appears. There is no real credential configured for this
    // connector in this test's environment, so this also proves the
    // handler never attempts (and never could accidentally succeed at)
    // resolving one.
    assert!(!properties.contains("hunter2"));
}
