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
//!
//! WS5 item D1 (a judge finding against the previous commit): the test
//! that used to be named
//! `postgres_connector_delete_with_force_deletes_despite_deprovision_failure`
//! is affected the same way the 409 test was — `conn-pg-lakehouse` no
//! longer fails to deprovision, it has nothing to deprovision (0034 gave it
//! `adapter = 'sql'`) — but its own doc comment claimed the genuine
//! "`force` bypasses a real deprovision failure" case was "now covered by
//! `postgres_connector_delete_without_force_keeps_the_row_and_returns_409`'s
//! own connector". That was false: that 409 test never passes
//! `?force=true`, so nothing asserted the escape hatch's actual reason for
//! existing. Renamed to
//! `postgres_connector_delete_with_force_on_seeded_row_with_nothing_to_
//! deprovision` (it deletes despite no failure, not despite one), and
//! `postgres_connector_delete_with_force_bypasses_a_real_deprovision_failure`
//! below adds the missing coverage: a self-created null-adapter
//! `PostgreSQL` connector (the same shape the 409 test builds), deleted
//! with `?force=true`, row gone.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use lakehouse_store::connectors::{
    CreateConnectorInput, CredentialKind, CredentialSource, CredentialSpec, IngestSpecInput,
    create_connector, set_ingest_spec,
};
use sqlx::PgPool;
use tower::ServiceExt;

use common::{session_cookie_for_seeded_user, spin_up};

/// A minimal, valid [`CreateConnectorInput`] — `kind`/`host` are
/// overridden per test, everything else is filler. The credential is
/// always a single `env:`-sourced password slot: every test in this file
/// exercises deprovision dispatch (adapter/host shape), never credential
/// resolution, so the derived name's exact value does not matter here —
/// see `lakehouse-store/tests/connectors.rs` for tests that DO assert on
/// the derived name.
fn minimal_input(name: &str, kind: &str, host: &str) -> CreateConnectorInput {
    CreateConnectorInput {
        name: name.to_owned(),
        kind: kind.to_owned(),
        direction: "source".to_owned(),
        host: host.to_owned(),
        credential: CredentialSpec {
            source: CredentialSource::Env,
            primary: CredentialKind::Password,
            secondary: None,
        },
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

/// Creates a null-adapter `PostgreSQL` connector pointed at the
/// compose-network `postgres` host — `set_ingest_spec` is deliberately
/// never called, so `adapter` stays `NULL` and
/// `deprovision_postgres_connector` takes the legacy
/// `None if kind.to_lowercase().contains("postgres")` arm, which
/// deterministically fails to deprovision (the host is unreachable from
/// this test's network). Shared by the 409-without-`force` test and the
/// `force`-bypasses-a-real-failure test below (AGENTS.md rule 4 — a
/// duplicated guard/fixture is a finding) so both exercise the identical
/// genuine-failure shape.
async fn create_connector_with_undeprovisionable_slot(pool: &PgPool, name: &str) -> String {
    let (created, _credential_names) = create_connector(
        pool,
        &minimal_input(name, "PostgreSQL", "lakehouse@postgres:5432/lakehouse"),
    )
    .await
    .expect("create connector");
    created.id
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

    let id =
        create_connector_with_undeprovisionable_slot(&app.pool, "still has a slot postgres").await;
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
/// WS3 item 6 / WS5 item D1: since 0034 gave the seeded row
/// `adapter = 'sql'`, `deprovision_postgres_connector` treats it as
/// nothing to deprovision (WS3 plan review X4) — deleting it is
/// unremarkable, the unforced route would 204 too. Named for what this
/// test actually proves (the seeded row deletes with `force`, deprovision
/// never even attempted) rather than the old name's now-false claim of a
/// deprovision failure. The genuine "`force` bypasses a REAL failure"
/// case is `postgres_connector_delete_with_force_bypasses_a_real_
/// deprovision_failure` below.
#[tokio::test]
async fn postgres_connector_delete_with_force_on_seeded_row_with_nothing_to_deprovision() {
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
        "?force=true must delete the row"
    );
}

/// The genuine case the `?force` escape hatch exists for: a connector
/// whose deprovision attempt actually fails (the same self-created
/// null-adapter `PostgreSQL` connector the 409-without-`force` test above
/// builds, pointed at the same unreachable-in-this-network host) still
/// deletes when `?force=true` is passed.
///
/// WS5 item D1 (a judge finding against the previous commit): the
/// `postgres_connector_delete_with_force_deletes_despite_deprovision_
/// failure` test this replaces was renamed above because it no longer
/// exercised a real failure at all (0034 changed the seeded row's
/// adapter); this test is the missing coverage, not a rename of the old
/// one — nothing on this branch previously asserted `?force=true`
/// overrides a genuine deprovision failure.
#[tokio::test]
async fn postgres_connector_delete_with_force_bypasses_a_real_deprovision_failure() {
    let app = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&app.pool, "bayu@meridian.example").await;

    let id = create_connector_with_undeprovisionable_slot(
        &app.pool,
        "force bypasses a real deprovision failure",
    )
    .await;
    assert!(
        connector_row_exists(&app.pool, &id).await,
        "create_connector must have inserted the row"
    );

    let path = format!("/api/connectors/{id}?force=true");
    let response = delete(&app.router, &path, &cookie).await;
    assert_eq!(
        response.status(),
        StatusCode::NO_CONTENT,
        "?force=true must override a genuine deprovision failure"
    );
    assert!(
        !connector_row_exists(&app.pool, &id).await,
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

    let (created, _credential_names) = create_connector(
        &app.pool,
        &minimal_input(
            "batch sql postgres never deprovisioned",
            "PostgreSQL",
            // Deliberately NOT shaped like "<user>@<host>:<port>/<database>"
            // -- `connector_probe::parse_postgres_host` would reject this
            // immediately, so if deprovisioning were attempted at all
            // (today's bug: gated on `kind` alone), this DELETE would 409.
            "this-is-not-a-dsn-shaped-host",
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
