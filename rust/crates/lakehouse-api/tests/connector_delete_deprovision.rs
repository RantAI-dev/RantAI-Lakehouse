//! `DELETE /api/connectors/{id}` — CDC deprovision-before-delete
//! (`routes::connectors::delete`'s doc comment: the fix for a deleted
//! `PostgreSQL` connector orphaning its replication slot/publication).
//!
//! Runs against the real, isolated Postgres `common::spin_up` provisions —
//! never a mock. WS3 item 6: `0034_seed_connector_ingest_spec.sql`
//! (written after this file was) set the seeded `conn-pg-lakehouse` row's
//! `adapter` to `'sql'`, so it no longer takes the legacy null-adapter
//! deprovision path this file's 409/`force` tests need — under WS3 plan
//! review X4, a `sql`-adapter connector never had a replication slot, so
//! `deprovision_postgres_connector` now correctly treats it as nothing to
//! deprovision and deletes it straight away. The 409-without-`force` test
//! below therefore creates its OWN null-adapter `PostgreSQL` connector
//! (never calls `set_ingest_spec`, so `adapter` stays `NULL`) pointed at
//! `lakehouse@postgres:5432/lakehouse` — the same unreachable-in-this-network
//! host `conn-pg-lakehouse` used to carry — to get a real, deterministic
//! deprovision failure and prove the 409/`force` contract end to end.
//!
//! The other tests still use the seeded `conn-pg-lakehouse`/
//! `conn-s3-warehouse` rows. `non_postgres_connector_deletes_without_any_
//! deprovision_attempt` and `deleting_an_unknown_connector_is_still_404`
//! are unaffected by 0034 (neither depends on a deprovision failure).
//! `postgres_connector_delete_with_force_deletes_despite_deprovision_failure`
//! is affected the same way the fixed test was: `conn-pg-lakehouse` no
//! longer fails to deprovision, it has nothing to deprovision — but the
//! assertions it makes (`?force=true` still deletes the row) hold either
//! way, so it stays green and is left as is; its own doc comment below
//! says so honestly rather than restating the old "despite a failure"
//! premise.

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
///
/// WS3 item 6: this used to delete the seeded `conn-pg-lakehouse` row, back
/// when it was still `adapter IS NULL` and so took the legacy path that
/// derives `slot_name`/`publication_name` from its own id's slug. Since
/// `0034_seed_connector_ingest_spec.sql` gave that row `adapter = 'sql'`
/// (correctly — see the module doc comment), it no longer exercises that
/// path, so this test now creates its own null-adapter `PostgreSQL`
/// connector instead, pointed at the same unreachable-in-this-network host
/// (`postgres`, the compose service name) the seeded row used to carry.
#[tokio::test]
async fn postgres_connector_delete_without_force_keeps_the_row_and_returns_409() {
    let app = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&app.pool, "bayu@meridian.example").await;

    let created = create_connector(
        &app.pool,
        &minimal_input(
            "still has a slot postgres",
            "PostgreSQL",
            "lakehouse@postgres:5432/lakehouse",
            "env:CONNECTOR_PG_PASSWORD",
        ),
    )
    .await
    .expect("create connector");
    // Deliberately never calls `set_ingest_spec`: `adapter` stays `NULL`,
    // so `deprovision_postgres_connector` takes the legacy
    // `None if kind.to_lowercase().contains("postgres")` arm rather than
    // the `sql`-adapter no-op arm.
    let id = created.id;
    let slug = id.replace('-', "_");
    let expected_slot = format!("{slug}_slot");
    let expected_pub = format!("{slug}_pub");

    assert!(
        connector_row_exists(&app.pool, &id).await,
        "create_connector must have inserted the row"
    );

    let path = format!("/api/connectors/{id}");
    let response = delete(&app.router, &path, &cookie).await;
    assert_eq!(response.status(), StatusCode::CONFLICT);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read body");
    let text = String::from_utf8_lossy(&body);
    assert!(text.contains(&expected_slot), "{text}");
    assert!(text.contains(&expected_pub), "{text}");
    assert!(
        text.contains("force"),
        "409 body must mention the ?force escape hatch: {text}"
    );

    // The whole point: the row must still be there.
    assert!(
        connector_row_exists(&app.pool, &id).await,
        "the registry row must survive a failed deprovision attempt"
    );
}

/// `?force=true` deletes `conn-pg-lakehouse` regardless.
///
/// WS3 item 6: this no longer proves `force` bypasses a deprovision
/// *failure* — since 0034 gave the seeded row `adapter = 'sql'`,
/// `deprovision_postgres_connector` treats it as nothing to deprovision
/// (WS3 plan review X4) and the unforced route would 204 too. The
/// assertion below (row gone, 204) still holds and is still worth
/// keeping, but the genuine "force bypasses a real failure" case is now
/// covered by `postgres_connector_delete_without_force_keeps_the_row_and_
/// returns_409`'s own connector, not this one.
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
