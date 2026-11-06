//! The deployment's sign-in limits, saved from the console (`ADM-8`,
//! migration `0129`): the failed-login limit, the window it is counted in,
//! the lockout, how long sign-in records are kept and the longest life of a
//! signed embed link.
//!
//! At most one row exists (`singleton`). No row is not an error: it means
//! nothing was ever saved in the console, and the API supplies the
//! environment's value or the code default (`lakehouse-api`
//! `runtime_settings.rs`). Every field is nullable: `None` means "not set in
//! the console", so the caller falls through to the environment or the
//! default — the merge lives in exactly one place, the resolver, never here
//! (rule 4: one merge rule).
//!
//! # The ranges are the table's own
//!
//! Each column's `CHECK` closes it to the range plan D12 names, so a value
//! outside it can never reach a row even if a caller skips the API's
//! validation. The API's own sentences still name the field to the person who
//! sent the value.

use crate::{PgPool, StoreError};

/// The saved sign-in limits, as stored. Every setting column is `Option`:
/// `None` means "not set in the console".
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct SignInSettings {
    /// Failed password tries tolerated per key before the lockout, 3 to 20.
    pub max_failures: Option<i32>,
    /// The window (seconds) the failed tries are counted in, 60 to 86400.
    pub failure_window_secs: Option<i32>,
    /// How long (seconds) a locked key stays locked, 60 to 86400.
    pub lockout_secs: Option<i32>,
    /// How long (days) sign-in records are kept, 7 to 3650.
    pub retention_days: Option<i32>,
    /// The longest life (seconds) of a signed embed link, 300 to 86400.
    pub embed_max_lifetime_secs: Option<i32>,
}

/// The saved limits, or `None` when nothing was ever saved in the console.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the query fails.
pub async fn get(pool: &PgPool) -> Result<Option<SignInSettings>, StoreError> {
    Ok(sqlx::query_as(
        "SELECT max_failures, failure_window_secs, lockout_secs, retention_days, \
              embed_max_lifetime_secs FROM sign_in_settings",
    )
    .fetch_optional(pool)
    .await?)
}

/// Save the limits, replacing any earlier row, and return what is stored.
/// A `None` field clears that setting in the console (falls through to the
/// environment or the default). `updated_by` is the saving principal's id.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the write fails, including when the
/// table's `CHECK`s refuse the values.
pub async fn upsert(
    pool: &PgPool,
    settings: &SignInSettings,
    updated_by: &str,
) -> Result<SignInSettings, StoreError> {
    Ok(sqlx::query_as(
        "INSERT INTO sign_in_settings \
             (singleton, max_failures, failure_window_secs, lockout_secs, retention_days, \
              embed_max_lifetime_secs, updated_at, updated_by) \
         VALUES (TRUE, $1, $2, $3, $4, $5, now(), $6) \
         ON CONFLICT (singleton) DO UPDATE SET max_failures = EXCLUDED.max_failures, \
              failure_window_secs = EXCLUDED.failure_window_secs, \
              lockout_secs = EXCLUDED.lockout_secs, \
              retention_days = EXCLUDED.retention_days, \
              embed_max_lifetime_secs = EXCLUDED.embed_max_lifetime_secs, \
              updated_at = now(), updated_by = EXCLUDED.updated_by \
         RETURNING max_failures, failure_window_secs, lockout_secs, retention_days, \
              embed_max_lifetime_secs",
    )
    .bind(settings.max_failures)
    .bind(settings.failure_window_secs)
    .bind(settings.lockout_secs)
    .bind(settings.retention_days)
    .bind(settings.embed_max_lifetime_secs)
    .bind(updated_by)
    .fetch_one(pool)
    .await?)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[sqlx::test(migrations = "../../migrations")]
    async fn an_upsert_round_trips_and_null_columns_stay_null(pool: PgPool) -> sqlx::Result<()> {
        assert!(
            get(&pool).await.unwrap().is_none(),
            "no row before the first save"
        );

        let saved = upsert(
            &pool,
            &SignInSettings {
                max_failures: Some(3),
                failure_window_secs: Some(600),
                lockout_secs: Some(120),
                retention_days: Some(14),
                embed_max_lifetime_secs: Some(3600),
            },
            "user-1",
        )
        .await
        .unwrap();

        assert_eq!(saved.max_failures, Some(3));
        assert_eq!(saved.failure_window_secs, Some(600));
        assert_eq!(saved.lockout_secs, Some(120));
        assert_eq!(saved.retention_days, Some(14));
        assert_eq!(saved.embed_max_lifetime_secs, Some(3600));

        let fetched = get(&pool).await.unwrap().unwrap();
        assert_eq!(fetched, saved);
        Ok(())
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn clearing_a_field_stores_null(pool: PgPool) -> sqlx::Result<()> {
        upsert(
            &pool,
            &SignInSettings {
                max_failures: Some(3),
                failure_window_secs: Some(600),
                lockout_secs: Some(120),
                retention_days: Some(14),
                embed_max_lifetime_secs: Some(3600),
            },
            "user-1",
        )
        .await
        .unwrap();

        let cleared = upsert(
            &pool,
            &SignInSettings {
                max_failures: None,
                failure_window_secs: None,
                lockout_secs: None,
                retention_days: None,
                embed_max_lifetime_secs: None,
            },
            "user-1",
        )
        .await
        .unwrap();
        assert_eq!(cleared.max_failures, None);
        assert_eq!(cleared.embed_max_lifetime_secs, None);

        let fetched = get(&pool).await.unwrap().unwrap();
        assert_eq!(fetched.max_failures, None);
        assert_eq!(fetched.retention_days, None);
        Ok(())
    }

    /// The table's own `CHECK`s are the last line: a value outside plan D12's
    /// range never reaches a row, even if a caller skips the API's check.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_value_outside_the_range_is_refused_by_the_table(pool: PgPool) -> sqlx::Result<()> {
        let refused = upsert(
            &pool,
            &SignInSettings {
                max_failures: Some(2),
                failure_window_secs: None,
                lockout_secs: None,
                retention_days: None,
                embed_max_lifetime_secs: None,
            },
            "user-1",
        )
        .await;
        assert!(refused.is_err(), "the table refuses a limit of 2 tries");
        Ok(())
    }
}
