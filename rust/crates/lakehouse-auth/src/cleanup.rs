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
#[allow(missing_docs, reason = "field names are self-documenting in context")]
#[derive(Debug, Clone, Copy, Default)]
pub struct PurgeCounts {
    pub sessions: u64,
    pub service_credentials: u64,
    pub throttle_entries: u64,
}

/// Delete expired/revoked sessions, revoked service credentials, and
/// stale throttle rows that are older than `retention`.
///
/// `throttle_window` is the `login_failure_window_secs` config value — a
/// throttle row whose window has passed and which is no longer locked is
/// deleted once it is at least `retention` old.
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

    use super::*;
    use sqlx::PgPool;
    use time::OffsetDateTime;
    use time::ext::NumericalDuration as _;
    use uuid::Uuid;

    /// Seed one session that expires `offset` seconds from now.
    async fn seed_session(pool: &PgPool, user_id: Uuid, offset: i64) {
        let expires_at = OffsetDateTime::now_utc() + offset.seconds();
        sqlx::query(
            "INSERT INTO session \
             (id, session_hash, app_user_id, created_at, expires_at) \
             VALUES (gen_random_uuid(), encode(gen_random_bytes(32), 'hex'), $1, $2, $3)",
        )
        .bind(user_id)
        .bind(OffsetDateTime::now_utc())
        .bind(expires_at)
        .execute(pool)
        .await
        .unwrap();
    }

    /// Seed one revoked service credential whose `revoked_at` is `offset`
    /// seconds from now.
    async fn seed_revoked_credential(pool: &PgPool, service_identity_id: Uuid, offset: i64) {
        let revoked_at = OffsetDateTime::now_utc() + offset.seconds();
        sqlx::query(
            "INSERT INTO service_credential \
             (id, service_identity_id, token_hash, revoked_at) \
             VALUES (gen_random_uuid(), $1, encode(gen_random_bytes(32), 'hex'), $2)",
        )
        .bind(service_identity_id)
        .bind(revoked_at)
        .execute(pool)
        .await
        .unwrap();
    }

    /// Seed one active (non-revoked) service credential.
    async fn seed_active_credential(pool: &PgPool, service_identity_id: Uuid) {
        sqlx::query(
            "INSERT INTO service_credential \
             (id, service_identity_id, token_hash) \
             VALUES (gen_random_uuid(), $1, encode(gen_random_bytes(32), 'hex'))",
        )
        .bind(service_identity_id)
        .execute(pool)
        .await
        .unwrap();
    }

    /// Seed one `login_throttle` row that is no longer locked, within its
    /// window.
    async fn seed_throttle_stale(pool: &PgPool) {
        let key = crate::throttle::key_for("stale@test.com");
        sqlx::query(
            "INSERT INTO login_throttle (key_hash, failures, window_started_at, locked_until) \
             VALUES ($1, 3, $2, NULL)",
        )
        .bind(&key)
        .bind(OffsetDateTime::now_utc() - 1800.seconds())
        .execute(pool)
        .await
        .unwrap();
    }

    /// Seed one `login_throttle` row that is still locked.
    async fn seed_throttle_locked(pool: &PgPool) {
        let key = crate::throttle::key_for("locked@test.com");
        sqlx::query(
            "INSERT INTO login_throttle (key_hash, failures, window_started_at, locked_until) \
             VALUES ($1, 5, $2, $3)",
        )
        .bind(&key)
        .bind(OffsetDateTime::now_utc())
        .bind(OffsetDateTime::now_utc() + 3600.seconds())
        .execute(pool)
        .await
        .unwrap();
    }

    fn short_retention() -> Duration {
        Duration::seconds(1)
    }

    fn short_throttle_window() -> Duration {
        Duration::seconds(900)
    }

    #[sqlx::test]
    async fn purge_on_empty_tables_returns_zeros(pool: PgPool) {
        let counts = purge(&pool, short_retention(), short_throttle_window())
            .await
            .unwrap();
        assert_eq!(counts.sessions, 0);
        assert_eq!(counts.service_credentials, 0);
        assert_eq!(counts.throttle_entries, 0);
    }

    #[sqlx::test]
    async fn purge_deletes_expired_sessions_past_retention(pool: PgPool) {
        let user_id = Uuid::new_v4();
        seed_session(&pool, user_id, -3600).await; // expired 1 h ago
        seed_session(&pool, user_id, 3600).await; // expires in 1 h
        let counts = purge(&pool, Duration::seconds(900), short_throttle_window())
            .await
            .unwrap();
        assert_eq!(counts.sessions, 1);
    }

    #[sqlx::test]
    async fn purge_deletes_revoked_credentials_past_retention(pool: PgPool) {
        let si_id = Uuid::new_v4();
        seed_revoked_credential(&pool, si_id, -3600).await;
        seed_active_credential(&pool, si_id).await;
        let counts = purge(&pool, Duration::seconds(900), short_throttle_window())
            .await
            .unwrap();
        assert_eq!(counts.service_credentials, 1);
    }

    #[sqlx::test]
    async fn purge_deletes_stale_throttle_rows_past_window(pool: PgPool) {
        seed_throttle_stale(&pool).await;
        let counts = purge(&pool, short_retention(), short_throttle_window())
            .await
            .unwrap();
        assert_eq!(counts.throttle_entries, 1);
    }

    #[sqlx::test]
    async fn purge_does_not_delete_locked_throttle_rows(pool: PgPool) {
        seed_throttle_locked(&pool).await;
        let counts = purge(&pool, short_retention(), short_throttle_window())
            .await
            .unwrap();
        assert_eq!(counts.throttle_entries, 0);
    }

    #[sqlx::test]
    async fn purge_does_not_delete_live_sessions(pool: PgPool) {
        let user_id = Uuid::new_v4();
        seed_session(&pool, user_id, 86400).await;
        let counts = purge(&pool, short_retention(), short_throttle_window())
            .await
            .unwrap();
        assert_eq!(counts.sessions, 0);
    }

    #[sqlx::test]
    async fn purge_does_not_delete_active_credentials(pool: PgPool) {
        let si_id = Uuid::new_v4();
        seed_active_credential(&pool, si_id).await;
        let counts = purge(&pool, short_retention(), short_throttle_window())
            .await
            .unwrap();
        assert_eq!(counts.service_credentials, 0);
    }
}
