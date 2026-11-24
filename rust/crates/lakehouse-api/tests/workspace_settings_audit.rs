//! `PUT /api/settings/general`: the audit row must name the fields that
//! really changed, also when two saves race (`ADM-2`, plan adm-120 step 3b).
//!
//! What this proves:
//!
//! * a save of the name the row already holds names no field;
//! * two saves that race each name the change, in whichever order they land;
//! * two saves of the SAME name that race: exactly one of them names the
//!   change, because the second one replaces the name the first one wrote;
//! * the changed list comes from the value the write replaced: a name that
//!   changed while the write waited for the row's lock is named as changed.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use crate::common;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use common::{TestApp, audit_rows, create_principal_with_permissions, session_cookie_for_user};
use serde_json::json;
use sqlx::{PgPool, Transaction};
use std::time::Duration;
use tower::ServiceExt;

const GENERAL: &str = "/api/settings/general";

/// A session cookie for a principal that may change the workspace settings.
async fn admin(app: &TestApp) -> String {
    let id = create_principal_with_permissions(&app.pool, "settings:write").await;
    session_cookie_for_user(&app.pool, id).await
}

/// The status a save of `name` answers with.
///
/// The router and the cookie are owned, and the name is a literal, so the
/// call can be spawned: a save that waits for the row's lock has to run
/// while the test does something else.
async fn put_name(router: axum::Router, cookie: String, name: &'static str) -> StatusCode {
    let request = Request::builder()
        .method(Method::PUT)
        .uri(GENERAL)
        .header("cookie", cookie)
        .header("content-type", "application/json")
        .body(Body::from(json!({ "workspaceName": name }).to_string()))
        .unwrap();
    router.oneshot(request).await.unwrap().status()
}

/// Holds the settings row's lock for as long as the transaction lives, so a
/// save that runs meanwhile waits for it.
async fn lock_the_row(tx: &mut Transaction<'_, sqlx::Postgres>) {
    sqlx::query("SELECT workspace_name FROM workspace_settings WHERE singleton = TRUE FOR UPDATE")
        .fetch_one(&mut **tx)
        .await
        .unwrap();
}

/// The status a spawned save answered with, or a loud failure when it never
/// answered.
///
/// A save that waits for the row's lock must answer once the lock is
/// released; a save that never answers is a failure to report, not a test
/// to wait for.
async fn await_save(handle: tokio::task::JoinHandle<StatusCode>, what: &str) -> StatusCode {
    tokio::time::timeout(Duration::from_secs(30), handle)
        .await
        .unwrap_or_else(|_| panic!("{what} never answered"))
        .unwrap()
}

/// Waits until `wanted` sessions of this test's own database wait on a lock,
/// or fails loudly.
///
/// Every test gets its own database, so a session waiting on a lock in it is
/// a save this test started: nothing else could wait there. The wait is a
/// poll of `pg_stat_activity` and not a sleep, so the test cannot pass
/// without the save really waiting.
async fn wait_for_lock_waits(pool: &PgPool, wanted: i64, what: &str) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let waiting: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pg_stat_activity \
             WHERE datname = current_database() \
               AND wait_event_type = 'Lock' \
               AND pid <> pg_backend_pid()",
        )
        .fetch_one(pool)
        .await
        .unwrap();
        if waiting >= wanted {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "{what}: only {waiting} of {wanted} sessions waited on a lock"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// A save of the name the row already holds changes nothing, so the audit
/// row names no field.
#[tokio::test]
async fn a_save_of_the_same_name_names_no_field() {
    let app = common::spin_up().await;
    let cookie = admin(&app).await;

    assert_eq!(
        put_name(app.router.clone(), cookie.clone(), "Acme").await,
        StatusCode::OK
    );
    assert_eq!(
        put_name(app.router.clone(), cookie.clone(), "Acme").await,
        StatusCode::OK
    );

    let updates = audit_rows(&app.pool, "settings.general.update").await;
    assert_eq!(updates.len(), 2);
    assert_eq!(
        updates[0]["args"],
        json!({ "fields": ["workspaceName"] }),
        "the first save set the name"
    );
    assert_eq!(
        updates[1]["args"],
        json!({ "fields": [] }),
        "the second save wrote the name the row already held"
    );
}

/// Two saves that race each name the change, in whichever order they land.
///
/// The row's lock is held while both saves are started, so they really
/// overlap: the second one to write replaces the name the first one wrote,
/// and that name is not the one it wanted.
#[tokio::test]
async fn two_concurrent_saves_of_different_names_each_name_the_change() {
    let app = common::spin_up().await;
    let cookie = admin(&app).await;
    assert_eq!(
        put_name(app.router.clone(), cookie.clone(), "First").await,
        StatusCode::OK
    );

    let mut tx = app.pool.begin().await.unwrap();
    lock_the_row(&mut tx).await;

    let first = tokio::spawn(put_name(app.router.clone(), cookie.clone(), "Second"));
    let second = tokio::spawn(put_name(app.router.clone(), cookie.clone(), "Third"));

    wait_for_lock_waits(&app.pool, 2, "the two concurrent saves").await;
    tx.commit().await.unwrap();

    assert_eq!(
        await_save(first, "the first concurrent save").await,
        StatusCode::OK
    );
    assert_eq!(
        await_save(second, "the second concurrent save").await,
        StatusCode::OK
    );

    let updates = audit_rows(&app.pool, "settings.general.update").await;
    assert_eq!(updates.len(), 3);
    for row in &updates {
        assert_eq!(
            row["args"],
            json!({ "fields": ["workspaceName"] }),
            "every save changed the name: {row}"
        );
    }
}

/// Two saves of the SAME name that race: exactly one of them changed the
/// name.
///
/// The second save replaces the name the first one wrote, which is the name
/// it wanted anyway: nothing changed, and its audit row must say so.
#[tokio::test]
async fn two_concurrent_saves_of_the_same_name_name_the_change_once() {
    let app = common::spin_up().await;
    let cookie = admin(&app).await;
    assert_eq!(
        put_name(app.router.clone(), cookie.clone(), "First").await,
        StatusCode::OK
    );

    let mut tx = app.pool.begin().await.unwrap();
    lock_the_row(&mut tx).await;

    let first = tokio::spawn(put_name(app.router.clone(), cookie.clone(), "Second"));
    let second = tokio::spawn(put_name(app.router.clone(), cookie.clone(), "Second"));

    wait_for_lock_waits(&app.pool, 2, "the two concurrent saves").await;
    tx.commit().await.unwrap();

    assert_eq!(
        await_save(first, "the first concurrent save").await,
        StatusCode::OK
    );
    assert_eq!(
        await_save(second, "the second concurrent save").await,
        StatusCode::OK
    );

    let updates = audit_rows(&app.pool, "settings.general.update").await;
    assert_eq!(updates.len(), 3);
    let named = updates
        .iter()
        .filter(|row| row["args"] == json!({ "fields": ["workspaceName"] }))
        .count();
    let unnamed = updates
        .iter()
        .filter(|row| row["args"] == json!({ "fields": [] }))
        .count();
    assert_eq!(named, 2, "the first save changed the name: {updates:?}");
    assert_eq!(
        unnamed, 1,
        "the second save wrote the name the first one wrote: {updates:?}"
    );
}

/// The changed list comes from the value the write replaced, not from a
/// value read before the write.
///
/// The row's lock is held while the save waits for it, and the stored name
/// moves to "Y" before the lock is released: the save then replaces "Y"
/// with "X", so the name DID change and the audit row must name it. A route
/// that read the row before its write read "X" and named no field.
#[tokio::test]
async fn the_changed_list_names_the_value_the_write_replaced() {
    let app = common::spin_up().await;
    let cookie = admin(&app).await;
    assert_eq!(
        put_name(app.router.clone(), cookie.clone(), "X").await,
        StatusCode::OK
    );

    let mut tx = app.pool.begin().await.unwrap();
    lock_the_row(&mut tx).await;

    let save = tokio::spawn(put_name(app.router.clone(), cookie.clone(), "X"));
    wait_for_lock_waits(&app.pool, 1, "the save of the same name").await;

    // The stored name moves to "Y" while the save waits for the lock.
    sqlx::query("UPDATE workspace_settings SET workspace_name = 'Y' WHERE singleton = TRUE")
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();

    assert_eq!(
        await_save(save, "the save of the same name").await,
        StatusCode::OK
    );

    let updates = audit_rows(&app.pool, "settings.general.update").await;
    assert_eq!(updates.len(), 2);
    for row in &updates {
        assert_eq!(
            row["args"],
            json!({ "fields": ["workspaceName"] }),
            "the write replaced \"Y\" with \"X\", so the name changed: {row}"
        );
    }

    let stored: Option<String> =
        sqlx::query_scalar("SELECT workspace_name FROM workspace_settings WHERE singleton = TRUE")
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert_eq!(
        stored.as_deref(),
        Some("X"),
        "the save's write landed after \"Y\""
    );
}
