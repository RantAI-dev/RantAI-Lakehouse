//! Integration tests for `lakehouse_auth::throttle` against a real Postgres,
//! using the shared `lakehouse-test-support` harness.
//!
//! `sqlx::test` with `migrations` re-migrates a fresh database per test
//! and injects the pool.

#![allow(clippy::unwrap_used, clippy::expect_used)]

// Force-links `lakehouse-test-support` so its `#[ctor]` Postgres
// testcontainer bootstrap actually runs for this test binary (an
// unreferenced dev-dependency's rlib member can otherwise be dropped
// by the linker before its ctor section is ever considered). Without
// this line `sqlx::test` finds no `DATABASE_URL` and every test in the
// file fails before its first statement — review blocker 2, 2026-10-03.
use lakehouse_test_support as _;

use lakehouse_auth::throttle::{self, ThrottlePolicy};
use sqlx::PgPool;
use std::sync::Arc;
use time::Duration;

fn policy() -> ThrottlePolicy {
    ThrottlePolicy {
        max_failures: 5,
        window: Duration::seconds(900),
        lockout: Duration::seconds(300),
    }
}

#[sqlx::test(migrations = "../../migrations")]
async fn locked_until_returns_none_when_no_row_exists(pool: PgPool) {
    let result = throttle::locked_until(&pool, &throttle::key_for("nonexistent@test.com"))
        .await
        .unwrap();
    assert!(result.is_none());
}

#[sqlx::test(migrations = "../../migrations")]
async fn record_failure_no_lock_below_limit(pool: PgPool) {
    let key = throttle::key_for("user@test.com");
    for _ in 0..4 {
        let lock = throttle::record_failure(&pool, &key, &policy())
            .await
            .unwrap();
        assert!(lock.is_none());
    }
    let locked = throttle::locked_until(&pool, &key).await.unwrap();
    assert!(locked.is_none());
}

#[sqlx::test(migrations = "../../migrations")]
async fn record_failure_locks_on_nth_failure(pool: PgPool) {
    let key = throttle::key_for("user@test.com");
    for _ in 0..4 {
        assert!(
            throttle::record_failure(&pool, &key, &policy())
                .await
                .unwrap()
                .is_none()
        );
    }
    let lock = throttle::record_failure(&pool, &key, &policy())
        .await
        .unwrap();
    assert!(lock.is_some());
    let locked = throttle::locked_until(&pool, &key).await.unwrap();
    assert!(locked.is_some());
}

#[sqlx::test(migrations = "../../migrations")]
async fn record_failure_starts_new_window_after_window_passes(pool: PgPool) {
    let key = throttle::key_for("user@test.com");
    let short = ThrottlePolicy {
        max_failures: 5,
        window: Duration::seconds(3),
        lockout: Duration::seconds(300),
    };
    assert!(
        throttle::record_failure(&pool, &key, &short)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        throttle::record_failure(&pool, &key, &short)
            .await
            .unwrap()
            .is_none()
    );
    tokio::time::sleep(std::time::Duration::from_secs(4)).await;
    let lock = throttle::record_failure(&pool, &key, &short).await.unwrap();
    assert!(lock.is_none());
}

#[sqlx::test(migrations = "../../migrations")]
async fn clear_removes_the_row(pool: PgPool) {
    let key = throttle::key_for("user@test.com");
    for _ in 0..3 {
        throttle::record_failure(&pool, &key, &policy())
            .await
            .unwrap();
    }
    throttle::clear(&pool, &key).await.unwrap();
    let locked = throttle::locked_until(&pool, &key).await.unwrap();
    assert!(locked.is_none());
}

#[sqlx::test(migrations = "../../migrations")]
async fn expired_lock_reads_as_not_locked(pool: PgPool) {
    let key = throttle::key_for("user@test.com");
    let short = ThrottlePolicy {
        max_failures: 2,
        window: Duration::seconds(900),
        lockout: Duration::seconds(2),
    };
    assert!(
        throttle::record_failure(&pool, &key, &short)
            .await
            .unwrap()
            .is_none()
    );
    let lock = throttle::record_failure(&pool, &key, &short).await.unwrap();
    assert!(lock.is_some());
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    let locked = throttle::locked_until(&pool, &key).await.unwrap();
    assert!(locked.is_none());
}

#[sqlx::test(migrations = "../../migrations")]
async fn ten_concurrent_record_failure_calls_all_count(pool: PgPool) {
    let key = throttle::key_for("concurrent@test.com");
    let pool = Arc::new(pool);
    let mut handles = Vec::new();
    let p = policy();
    for _ in 0..10 {
        let pool = pool.clone();
        let key = key.clone();
        let p = p.clone();
        let handle = tokio::spawn(async move { throttle::record_failure(&pool, &key, &p).await });
        handles.push(handle);
    }
    for handle in handles {
        handle.await.unwrap().unwrap();
    }
    // Every call must have counted: the stored count is exactly 10, and
    // exactly the calls from the 5th increment onward report a lock. A
    // weaker `>= 1` check here would pass even if half the concurrent
    // attempts went uncounted (review SHOULD-FIX 2, 2026-10-03).
    let (failures, locked): (i32, Option<time::OffsetDateTime>) =
        sqlx::query_as("SELECT failures, locked_until FROM login_throttle WHERE key_hash = $1")
            .bind(&key)
            .fetch_one(pool.as_ref())
            .await
            .unwrap();
    assert_eq!(failures, 10, "every concurrent failure must be counted");
    assert!(locked.is_some());
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_failure_after_the_lock_expires_starts_the_count_fresh(pool: PgPool) {
    // After a lock expires, ONE further failure must only be failure #1:
    // the plan (as corrected in review SHOULD-FIX 1, 2026-10-03) requires
    // the count to restart, not carry the pre-lock failures into an
    // immediate re-lock.
    let key = throttle::key_for("expired-lock@test.com");
    let short = ThrottlePolicy {
        max_failures: 2,
        window: Duration::seconds(900),
        lockout: Duration::seconds(2),
    };
    throttle::record_failure(&pool, &key, &short).await.unwrap();
    assert!(
        throttle::record_failure(&pool, &key, &short)
            .await
            .unwrap()
            .is_some()
    );
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;

    let lock = throttle::record_failure(&pool, &key, &short).await.unwrap();
    assert!(
        lock.is_none(),
        "one failure after an expired lock must not re-lock"
    );
    let (failures,): (i32,) =
        sqlx::query_as("SELECT failures FROM login_throttle WHERE key_hash = $1")
            .bind(&key)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(failures, 1, "the count restarts at 1 after a lock expires");

    // And the key needs a full new set of failures to lock again.
    assert!(
        throttle::record_failure(&pool, &key, &short)
            .await
            .unwrap()
            .is_some(),
        "the second failure after expiry must lock again"
    );
}
