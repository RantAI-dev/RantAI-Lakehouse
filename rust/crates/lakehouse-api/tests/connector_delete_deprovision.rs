//! `DELETE /api/connectors/{id}` — CDC deprovision-before-delete
//! (`routes::connectors::delete`'s doc comment: the fix for a deleted
//! `PostgreSQL` connector orphaning its replication slot/publication).
//!
//! Uses the real seeded `conn-pg-lakehouse` / `conn-s3-warehouse` rows
//! (`0022_prune_connector_seed.sql`) against the real, isolated Postgres
//! `common::spin_up` provisions — `conn-pg-lakehouse`'s `host` names
//! `postgres`, a hostname that does not exist in this test's network, so
//! deprovisioning it always fails at the connect step. That is exactly
//! what these tests need: a REAL, deterministic deprovision failure
//! (never a mock), to prove the 409-without-`force` /
//! 204-with-`?force=true` contract end to end.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use lakehouse_store::connectors::{
    CreateConnectorInput, IngestSpecInput, create_connector, set_ingest_spec,
};
use sqlx::PgPool;
use tower::ServiceExt;

use common::{session_cookie_for_seeded_user, spin_up};

/// A minimal, valid [`CreateConnectorInput`] — `kind`/`host`/`secret_ref`
/// are overridden per test, everything else is filler.
fn minimal_input(name: &str, kind: &str, host: &str, secret_ref: &str) -> CreateConnectorInput {
    CreateConnectorInput {
        name: name.to_owned(),
        kind: kind.to_owned(),
        direction: "source".to_owned(),
        host: host.to_owned(),
        secret_ref: secret_ref.to_owned(),
        secret_ref_secondary: None,
        environment: "staging".to_owned(),
        tenant: "Meridian Group".to_owned(),
        residency: "in-region".to_owned(),
        capabilities: vec![],
        owner: None,
    }
}

async fn delete(app: &axum::Router, path: &str, cookie: &str) -> axum::http::Response<Body> {
    app.clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(path)
                .header("cookie", cookie)
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("router never fails to produce a response")
}

async fn connector_row_exists(pool: &PgPool, id: &str) -> bool {
    let row: Option<(i32,)> = sqlx::query_as("SELECT 1 FROM connector WHERE id = $1")
        .bind(id)
        .fetch_optional(pool)
        .await
        .expect("query connector table");
    row.is_some()
}

/// Deleting a `PostgreSQL` connector whose source host cannot be dialed
/// must NOT delete the registry row, and must answer 409 naming the
/// slot/publication — the exact fix for the defect this task closes
/// (silently deleting the row while the slot survived).
#[tokio::test]
async fn postgres_connector_delete_without_force_keeps_the_row_and_returns_409() {
    let app = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&app.pool, "bayu@meridian.example").await;

    assert!(
        connector_row_exists(&app.pool, "conn-pg-lakehouse").await,
        "0022_prune_connector_seed.sql must seed conn-pg-lakehouse"
    );

    let response = delete(&app.router, "/api/connectors/conn-pg-lakehouse", &cookie).await;
    assert_eq!(response.status(), StatusCode::CONFLICT);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read body");
    let text = String::from_utf8_lossy(&body);
    assert!(text.contains("conn_pg_lakehouse_slot"), "{text}");
    assert!(text.contains("conn_pg_lakehouse_pub"), "{text}");
    assert!(
        text.contains("force"),
        "409 body must mention the ?force escape hatch: {text}"
    );

    // The whole point: the row must still be there.
    assert!(
        connector_row_exists(&app.pool, "conn-pg-lakehouse").await,
        "the registry row must survive a failed deprovision attempt"
    );
}

/// `?force=true` deletes the row anyway, despite the same deprovision
/// failure — the documented escape hatch for a connector pointing at a
/// decommissioned/unreachable host.
#[tokio::test]
async fn postgres_connector_delete_with_force_deletes_despite_deprovision_failure() {
    let app = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&app.pool, "bayu@meridian.example").await;

    let response = delete(
        &app.router,
        "/api/connectors/conn-pg-lakehouse?force=true",
        &cookie,
    )
    .await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert!(
        !connector_row_exists(&app.pool, "conn-pg-lakehouse").await,
        "?force=true must delete the row even though deprovisioning failed"
    );
}

/// A non-`PostgreSQL` connector (the seeded S3 warehouse) deletes exactly
/// as before: no deprovision attempt, straight 204.
#[tokio::test]
async fn non_postgres_connector_deletes_without_any_deprovision_attempt() {
    let app = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&app.pool, "bayu@meridian.example").await;

    assert!(connector_row_exists(&app.pool, "conn-s3-warehouse").await);
    let response = delete(&app.router, "/api/connectors/conn-s3-warehouse", &cookie).await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert!(!connector_row_exists(&app.pool, "conn-s3-warehouse").await);
}

/// Deleting an unknown connector id is still a 404, unaffected by any of
/// the deprovision logic above (the dial-info lookup returns `None` before
/// any `PostgreSQL`-kind check happens).
#[tokio::test]
async fn deleting_an_unknown_connector_is_still_404() {
    let app = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&app.pool, "bayu@meridian.example").await;

    let response = delete(&app.router, "/api/connectors/conn-does-not-exist", &cookie).await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

/// WS3 plan review X4 — the exact defect this task fixes: a `sql`-adapter
/// connector whose `kind` string says "`PostgreSQL`" must never have its
/// deprovision path invoked, so an unreachable/malformed `host` (which
/// would deterministically 409 if deprovisioning were even attempted) has
/// no effect at all, and `DELETE` succeeds immediately.
#[tokio::test]
async fn deprovision_never_attempted_for_a_batch_sql_connector() {
    let app = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&app.pool, "bayu@meridian.example").await;

    let created = create_connector(
        &app.pool,
        &minimal_input(
            "batch sql postgres never deprovisioned",
            "PostgreSQL",
            // Deliberately NOT shaped like "<user>@<host>:<port>/<database>"
            // -- `connector_probe::parse_postgres_host` would reject this
            // immediately, so if deprovisioning were attempted at all
            // (today's bug: gated on `kind` alone), this DELETE would 409.
            "this-is-not-a-dsn-shaped-host",
            "env:CONNECTOR_PG_PASSWORD",
        ),
    )
    .await
    .expect("create connector");
    let spec = IngestSpecInput {
        adapter: "sql".to_owned(),
        ingest_mode: "batch".to_owned(),
        dial: serde_json::json!({
            "driver": "postgres",
            "host": "warehouse.example.internal",
            "port": 5432,
            "database": "oms",
            "user": "reader",
        }),
        source_objects: serde_json::json!([]),
        schedule_cron: None,
    };
    set_ingest_spec(&app.pool, &created.id, &spec)
        .await
        .expect("set sql ingest spec");

    let path = format!("/api/connectors/{}", created.id);
    let response = delete(&app.router, &path, &cookie).await;
    assert_eq!(
        response.status(),
        StatusCode::NO_CONTENT,
        "a sql-adapter connector must delete cleanly even though kind says PostgreSQL"
    );
    assert!(!connector_row_exists(&app.pool, &created.id).await);
}
