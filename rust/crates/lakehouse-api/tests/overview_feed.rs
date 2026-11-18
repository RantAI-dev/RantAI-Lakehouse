//! `GOV-1` Decision 9 (feature page `audit-trail-coverage`): only `audit:read`
//! reads the audit trail, and `POST /api/overview` is a read of it. The route
//! is open to every signed-in person (`Policy::RequiresAuth`), so the
//! handler narrows the feed itself: a holder of `audit:read` sees the newest
//! rows of everyone, anybody else sees the newest rows of their own.
//!
//! The tests drive the real router over a real per-test database. Rows are
//! seeded with `INSERT` at timestamps in 2099, so they are the newest rows in
//! the table whatever the clock or the sign-ins of the callers wrote, and the
//! expected ids are literals written here, not values the code computes.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use serde_json::Value;
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

use crate::common::{
    TestApp, create_principal_with_permissions, leave_no_seeded_tenants, session_cookie_for_user,
    spin_up,
};

/// One audit row at `at` (a 2099 timestamp), written by `principal_id`.
async fn seed(pool: &PgPool, id: &str, at: &str, principal_id: &str) {
    sqlx::query(
        "INSERT INTO audit_event (id, at, principal_id, principal_kind, actor_label, action, \
         resource_kind, resource_id, args, outcome) \
         VALUES ($1, $2::timestamptz, $3, 'user', 'someone', 'dashboard.update', 'dashboard', \
         'b-1', '{}'::jsonb, 'executed')",
    )
    .bind(id)
    .bind(at)
    .bind(principal_id)
    .execute(pool)
    .await
    .expect("seed an audit_event row");
}

/// `POST /api/overview` as the user behind `user`: the status and the ids of
/// the activity entries, newest first.
async fn feed_ids(pool: &PgPool, router: &axum::Router, user: Uuid) -> (StatusCode, Vec<String>) {
    let cookie = session_cookie_for_user(pool, user).await;
    let resp = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/overview")
                .header("cookie", cookie)
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("router never fails a request outright");
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&bytes)
        .unwrap_or_else(|_| panic!("a JSON body: {}", String::from_utf8_lossy(&bytes)));
    let ids = body["activity"]
        .as_array()
        .unwrap_or_else(|| panic!("an activity array: {body}"))
        .iter()
        .map(|a| a["id"].as_str().expect("an activity id").to_owned())
        .collect();
    (status, ids)
}

/// Two people with rows of their own and the rows they wrote: `mine-1`,
/// `mine-2` by the first, `theirs-1` by the second.
async fn two_writers(pool: &PgPool) -> (Uuid, Uuid) {
    let me = create_principal_with_permissions(pool, "governance:read").await;
    let other = create_principal_with_permissions(pool, "governance:read").await;
    seed(pool, "mine-1", "2099-01-01T10:00:00Z", &me.to_string()).await;
    seed(pool, "theirs-1", "2099-01-02T10:00:00Z", &other.to_string()).await;
    seed(pool, "mine-2", "2099-01-03T10:00:00Z", &me.to_string()).await;
    (me, other)
}

/// A person without `audit:read` reads their own actions and nobody else's.
#[tokio::test]
async fn a_person_without_audit_read_sees_only_their_own_actions_in_the_feed() {
    let TestApp { router, pool } = spin_up().await;
    // `SEC-20` (plan 100): the trail is read per tenant, and the seeded
    // deployment holds four tenants, so this test makes it a one-tenant one.
    leave_no_seeded_tenants(&pool).await;
    let (me, _) = two_writers(&pool).await;

    let (status, ids) = feed_ids(&pool, &router, me).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(ids, ["mine-2", "mine-1"]);
}

/// A holder of `audit:read` keeps today's feed: the newest rows of everyone.
#[tokio::test]
async fn a_holder_of_audit_read_sees_every_persons_actions_in_the_feed() {
    let TestApp { router, pool } = spin_up().await;
    // `SEC-20` (plan 100): the trail is read per tenant, and the seeded
    // deployment holds four tenants, so this test makes it a one-tenant one.
    leave_no_seeded_tenants(&pool).await;
    two_writers(&pool).await;
    let auditor = create_principal_with_permissions(&pool, "audit:read").await;

    let (status, ids) = feed_ids(&pool, &router, auditor).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(&ids[..3], ["mine-2", "theirs-1", "mine-1"]);
}

/// A person with no rows of their own gets an empty feed, not the rows of
/// somebody else.
#[tokio::test]
async fn a_person_with_no_actions_of_their_own_gets_an_empty_feed() {
    let TestApp { router, pool } = spin_up().await;
    // `SEC-20` (plan 100): the trail is read per tenant, and the seeded
    // deployment holds four tenants, so this test makes it a one-tenant one.
    leave_no_seeded_tenants(&pool).await;
    two_writers(&pool).await;
    let newcomer = create_principal_with_permissions(&pool, "governance:read").await;

    let (status, ids) = feed_ids(&pool, &router, newcomer).await;

    assert_eq!(status, StatusCode::OK);
    assert!(ids.is_empty(), "{ids:?}");
}

/// The feed of a person without `audit:read` is capped at 20 rows, as the
/// feed of everyone is: their own newest 20, not the whole of their history.
#[tokio::test]
async fn the_feed_of_a_person_without_audit_read_keeps_the_limit_of_twenty() {
    let TestApp { router, pool } = spin_up().await;
    // `SEC-20` (plan 100): the trail is read per tenant, and the seeded
    // deployment holds four tenants, so this test makes it a one-tenant one.
    leave_no_seeded_tenants(&pool).await;
    let me = create_principal_with_permissions(&pool, "governance:read").await;
    for day in 1..=25 {
        seed(
            &pool,
            &format!("row-{day:02}"),
            &format!("2099-02-{day:02}T10:00:00Z"),
            &me.to_string(),
        )
        .await;
    }

    let (_, ids) = feed_ids(&pool, &router, me).await;

    assert_eq!(ids.len(), 20, "{ids:?}");
    assert_eq!(ids.first().map(String::as_str), Some("row-25"));
    assert_eq!(ids.last().map(String::as_str), Some("row-06"));
}
