//! The deployment's reporting settings (migration `0066`): the report time
//! zone and the first day of the week.
//!
//! At most one row exists (`singleton`). No row is not an error: it means the
//! deployment never saved the settings, and the caller supplies the defaults
//! (`lakehouse_bi::grain::TimeContext::default`). This module stores two
//! strings and knows nothing about what a valid zone is; the route checks the
//! zone against the engine's list and the week start against the two values
//! the SQL builder implements, and the table's `CHECK`s are the backstop for
//! shape.

use crate::{PgPool, StoreError};

/// The saved settings, as stored.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct ReportingSettings {
    /// An IANA zone name.
    pub time_zone: String,
    /// `"monday"` or `"sunday"`.
    pub week_start: String,
}

/// The saved settings, or `None` when the deployment never saved them.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the query fails.
pub async fn get(pool: &PgPool) -> Result<Option<ReportingSettings>, StoreError> {
    Ok(
        sqlx::query_as("SELECT time_zone, week_start FROM reporting_settings")
            .fetch_optional(pool)
            .await?,
    )
}

/// Save the settings, replacing any earlier ones, and return what is stored.
/// `updated_by` is the saving principal's id.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the write fails, including when the
/// table's `CHECK`s refuse the values.
pub async fn upsert(
    pool: &PgPool,
    time_zone: &str,
    week_start: &str,
    updated_by: &str,
) -> Result<ReportingSettings, StoreError> {
    Ok(sqlx::query_as(
        "INSERT INTO reporting_settings (singleton, time_zone, week_start, updated_at, updated_by) \
         VALUES (TRUE, $1, $2, now(), $3) \
         ON CONFLICT (singleton) DO UPDATE SET time_zone = EXCLUDED.time_zone, \
         week_start = EXCLUDED.week_start, updated_at = now(), updated_by = EXCLUDED.updated_by \
         RETURNING time_zone, week_start",
    )
    .bind(time_zone)
    .bind(week_start)
    .bind(updated_by)
    .fetch_one(pool)
    .await?)
}
