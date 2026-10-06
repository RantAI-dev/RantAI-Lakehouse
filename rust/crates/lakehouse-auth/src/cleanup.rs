//! Session, service-credential, and throttle-row cleanup (SEC-5,
//! login-protection-and-session-cleanup plan). Runs as a background task
//! inside `lakehouse-api`, not as a Dagster job — the default stack runs
//! without Dagster.
//!
//! # Retention
//!
//! Retention is configurable via `AUTH_RETENTION_DAYS` (default 30 days).
//! A row is deleted when it has been expired/revoked/stale for longer than
//! the retention.
//!
//! # Cleanup granularity — 3 DELETE statements
//!
//! Each table gets its own `DELETE` (not one big joined statement) so a
//! slow scan on one table doesn't hold back the others, and so the row
//! counts in [`PurgeCounts`] are honest per table.

use sqlx::PgPool;
use time::Duration;
use time::OffsetDateTime;

use crate::AuthError;

/// How many rows were deleted from each table during this purge run.
#[derive(Debug, Clone, Copy, Default)]
pub struct PurgeCounts {
    /// Rows deleted from `session`.
    pub sessions: u64,
    /// Rows deleted from `service_credential`.
    pub service_credentials: u64,
    /// Rows deleted from `login_throttle`.
    pub throttle_entries: u64,
}

/// Delete expired/revoked sessions, revoked service credentials, and
/// stale throttle rows.
///
/// `retention` gates sessions and credentials: a session whose
/// `expires_at`/`revoked_at`, or a credential whose `revoked_at`, is
/// older than `now() - retention` is deleted.
///
/// `throttle_window` is the `login_failure_window_secs` config value and
/// gates throttle rows, NOT `retention`: a row is deleted once it is no
/// longer locked and its window has passed (such a row can never lock
/// again — the next failure starts a fresh window — so keeping it only
/// preserves a stale counter).
///
/// # Errors
///
/// Returns [`AuthError::Database`] on a storage failure.
pub async fn purge(
    pool: &PgPool,
    retention: Duration,
    throttle_window: Duration,
) -> Result<PurgeCounts, AuthError> {
    let cutoff = OffsetDateTime::now_utc() - retention;
    let throttle_cutoff = OffsetDateTime::now_utc() - throttle_window;

    let session_rows = sqlx::query(
        "DELETE FROM session \
         WHERE (expires_at < $1 OR revoked_at < $1) \
         RETURNING 1",
    )
    .bind(cutoff)
    .execute(pool)
    .await?
    .rows_affected();

    let credential_rows = sqlx::query(
        "DELETE FROM service_credential \
         WHERE revoked_at < $1 \
         RETURNING 1",
    )
    .bind(cutoff)
    .execute(pool)
    .await?
    .rows_affected();

    let throttle_rows = sqlx::query(
        "DELETE FROM login_throttle \
         WHERE (locked_until IS NULL OR locked_until < $1) \
           AND window_started_at < $2 \
         RETURNING 1",
    )
    .bind(OffsetDateTime::now_utc())
    .bind(throttle_cutoff)
    .execute(pool)
    .await?
    .rows_affected();

    Ok(PurgeCounts {
        sessions: session_rows,
        service_credentials: credential_rows,
        throttle_entries: throttle_rows,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    // `#[sqlx::test]` needs `DATABASE_URL` pointing at a Postgres and the
    // migrations applied; `lakehouse-test-support` sets both up (review
    // blocker 3, 2026-10-03 — without this link the tests panic
    // `DATABASE_URL must be set` before a single assertion runs).
    use super::*;
    use lakehouse_test_support as _;
    use sqlx::PgPool;
    use time::OffsetDateTime;
    use uuid::Uuid;

    // The broken first draft of these tests (review blocker 3,
    // 2026-10-03) wrote to a non-existent `session.session_hash` column
    // and invented FK ids; every test here seeds the real rows the
    // schema demands (an `app_user`, a `service_identity`) and uses the
    // real column names, and every test opts into the migrations the
    // tables come from.

    /// Insert one `app_user` with a unique email and return its id.
    async fn seed_app_user(pool: &PgPool, email: &str) -> Uuid {
        let (id,): (Uuid,) =
            sqlx::query_as("INSERT INTO app_user (name, email) VALUES ($1, $2) RETURNING id")
                .bind("cleanup test user")
                .bind(email)
                .fetch_one(pool)
                .await
                .unwrap();
        id
    }

    /// Insert one `service_identity` with a unique name and return its id.
    async fn seed_service_identity(pool: &PgPool, name: &str) -> Uuid {
        let (id,): (Uuid,) = sqlx::query_as(
            "INSERT INTO service_identity (name, environment, expires_at) \
             VALUES ($1, 'test', now() + interval '30 days') RETURNING id",
        )
        .bind(name)
        .fetch_one(pool)
        .await
        .unwrap();
        id
    }

    /// Insert one session for `user_id` that expires `expires_in` from
    /// now (negative for already expired), returning its `token_hash`.
    async fn seed_session(pool: &PgPool, user_id: Uuid, expires_in: Duration) -> String {
        let token_hash = format!("session-{}", Uuid::new_v4());
        sqlx::query(
            "INSERT INTO session (app_user_id, token_hash, expires_at) \
             VALUES ($1, $2, $3)",
        )
        .bind(user_id)
        .bind(&token_hash)
        .bind(OffsetDateTime::now_utc() + expires_in)
        .execute(pool)
        .await
        .unwrap();
        token_hash
    }

    /// Insert one revoked service credential whose `revoked_at` is
    /// `revoked_in` from now (negative for already revoked), returning
    /// its `token_hash`.
    async fn seed_revoked_credential(
        pool: &PgPool,
        service_identity_id: Uuid,
        revoked_in: Duration,
    ) -> String {
        let token_hash = format!("credential-{}", Uuid::new_v4());
        sqlx::query(
            "INSERT INTO service_credential (service_identity_id, token_hash, revoked_at) \
             VALUES ($1, $2, $3)",
        )
        .bind(service_identity_id)
        .bind(&token_hash)
        .bind(OffsetDateTime::now_utc() + revoked_in)
        .execute(pool)
        .await
        .unwrap();
        token_hash
    }

    /// Insert one active (non-revoked) service credential, returning its
    /// `token_hash`.
    async fn seed_active_credential(pool: &PgPool, service_identity_id: Uuid) -> String {
        let token_hash = format!("credential-{}", Uuid::new_v4());
        sqlx::query(
            "INSERT INTO service_credential (service_identity_id, token_hash) \
             VALUES ($1, $2)",
        )
        .bind(service_identity_id)
        .bind(&token_hash)
        .execute(pool)
        .await
        .unwrap();
        token_hash
    }

    /// Insert one `login_throttle` row with the given window start
    /// offset and lock expiry offset from now.
    async fn seed_throttle_row(
        pool: &PgPool,
        email: &str,
        window_started_in: Duration,
        locked_in: Option<Duration>,
    ) {
        let key = crate::throttle::key_for(email);
        sqlx::query(
            "INSERT INTO login_throttle (key_hash, failures, window_started_at, locked_until) \
             VALUES ($1, 3, $2, $3)",
        )
        .bind(&key)
        .bind(OffsetDateTime::now_utc() + window_started_in)
        .bind(locked_in.map(|d| OffsetDateTime::now_utc() + d))
        .execute(pool)
        .await
        .unwrap();
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn purge_on_empty_tables_returns_zeros(pool: PgPool) {
        let counts = purge(&pool, Duration::days(2), Duration::seconds(900))
            .await
            .unwrap();
        assert_eq!(counts.sessions, 0);
        assert_eq!(counts.service_credentials, 0);
        assert_eq!(counts.throttle_entries, 0);
    }

    /// The plan's acceptance matrix: one row of each kind on each side of
    /// both cut-offs, then assert EXACTLY which survive — by identity
    /// (token hash / key hash), not by count, so a purge that deletes the
    /// wrong row fails this test even when the totals happen to match.
    #[sqlx::test(migrations = "../../migrations")]
    async fn purge_deletes_exactly_the_expired_revoked_and_stale_rows(pool: PgPool) {
        // Retention 2 days: "expired/revoked 3 days ago" is past it,
        // "yesterday" is inside it.
        let retention = Duration::days(2);
        let throttle_window = Duration::seconds(900);

        let user = seed_app_user(&pool, "cleanup-sessions@test.com").await;
        let live_session = seed_session(&pool, user, Duration::hours(1)).await;
        let recently_expired_session = seed_session(&pool, user, -Duration::hours(20)).await;
        let _long_expired_session = seed_session(&pool, user, -Duration::days(3)).await;

        let si = seed_service_identity(&pool, "cleanup-test-identity").await;
        let _revoked_credential = seed_revoked_credential(&pool, si, -Duration::days(3)).await;
        let active_credential = seed_active_credential(&pool, si).await;

        seed_throttle_row(&pool, "in-window@test.com", -Duration::seconds(60), None).await;
        seed_throttle_row(
            &pool,
            "past-window@test.com",
            -Duration::seconds(1800),
            None,
        )
        .await;
        seed_throttle_row(
            &pool,
            "still-locked@test.com",
            -Duration::seconds(1800),
            Some(Duration::hours(1)),
        )
        .await;

        let counts = purge(&pool, retention, throttle_window).await.unwrap();

        assert_eq!(counts.sessions, 1, "only the long-expired session goes");
        assert_eq!(
            counts.service_credentials, 1,
            "only the revoked credential goes"
        );
        assert_eq!(
            counts.throttle_entries, 1,
            "only the past-window throttle row goes"
        );

        let session_hashes: Vec<String> =
            sqlx::query_as::<_, (String,)>("SELECT token_hash FROM session ORDER BY token_hash")
                .fetch_all(&pool)
                .await
                .unwrap()
                .into_iter()
                .map(|(h,)| h)
                .collect();
        let mut expected_sessions = vec![live_session, recently_expired_session];
        expected_sessions.sort();
        assert_eq!(session_hashes, expected_sessions);

        let credential_hashes: Vec<String> = sqlx::query_as::<_, (String,)>(
            "SELECT token_hash FROM service_credential ORDER BY token_hash",
        )
        .fetch_all(&pool)
        .await
        .unwrap()
        .into_iter()
        .map(|(h,)| h)
        .collect();
        assert_eq!(credential_hashes, vec![active_credential]);

        let throttle_keys: Vec<String> =
            sqlx::query_as::<_, (String,)>("SELECT key_hash FROM login_throttle ORDER BY key_hash")
                .fetch_all(&pool)
                .await
                .unwrap()
                .into_iter()
                .map(|(h,)| h)
                .collect();
        let mut expected_keys = vec![
            crate::throttle::key_for("in-window@test.com"),
            crate::throttle::key_for("still-locked@test.com"),
        ];
        expected_keys.sort();
        assert_eq!(throttle_keys, expected_keys);
    }
}
