//! What the orchestrator last saw of each table of a change-capture connector,
//! and the connector's lag limit (`SRC-4` slice 4, decisions S4-b, S4-d and
//! S4-e; migration `0118_connector_capture_lag.sql` -- read its header).
//!
//! This module stores facts and the alert's open flag. It does not decide what
//! a lag is: `lakehouse-api::capture_lag` does, from these rows and a `now` it
//! is given.
//!
//! # Two kinds of report, one row
//!
//! The sensor reports a table's backlog and the moment it saw it
//! ([`record_backlog`]); the applier reports the source commit time of the last
//! change it wrote ([`record_last_change`]). Either creates the row. Only a
//! backlog report sets `observed_at`, because only that proves the orchestrator
//! is alive and looking.

use sqlx::FromRow;
use time::OffsetDateTime;

use crate::{PgPool, StoreError};

/// The lowest `lag_alert_minutes` the column accepts.
pub const MIN_LAG_ALERT_MINUTES: i32 = 1;
/// The highest `lag_alert_minutes` the column accepts (one day).
pub const MAX_LAG_ALERT_MINUTES: i32 = 1440;

/// One table's stored observations.
#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct CaptureTable {
    /// The source object's name as the connector's spec spells it.
    pub table_name: String,
    /// Source commit time of the last change the applier wrote.
    pub last_source_ts: Option<OffsetDateTime>,
    /// The last moment the table was known to be up to date.
    pub caught_up_at: Option<OffsetDateTime>,
    /// Messages on the table's topic the applier had not read.
    pub backlog: Option<i64>,
    /// When `backlog` was seen.
    pub observed_at: Option<OffsetDateTime>,
}

/// Store a table's backlog seen at `at`. `caught_up` also moves
/// `caught_up_at` to `at` (the caller has checked that `Kafka Connect` said
/// `running`); it never moves it backwards.
///
/// # Errors
///
/// [`StoreError::Database`] on a failure, [`StoreError::NotFound`] for a
/// connector that does not exist.
pub async fn record_backlog(
    pool: &PgPool,
    connector_id: &str,
    table: &str,
    backlog: i64,
    caught_up: bool,
    at: OffsetDateTime,
) -> Result<(), StoreError> {
    let caught_up_at = caught_up.then_some(at);
    let done = sqlx::query(
        "INSERT INTO connector_capture_table \
           (connector_id, table_name, backlog, observed_at, caught_up_at) \
         SELECT id, $2, $3, $4, $5 FROM connector WHERE id = $1 \
         ON CONFLICT (connector_id, table_name) DO UPDATE SET \
           backlog = EXCLUDED.backlog, \
           observed_at = EXCLUDED.observed_at, \
           caught_up_at = GREATEST(connector_capture_table.caught_up_at, EXCLUDED.caught_up_at)",
    )
    .bind(connector_id)
    .bind(table)
    .bind(backlog)
    .bind(at)
    .bind(caught_up_at)
    .execute(pool)
    .await?;
    if done.rows_affected() == 0 {
        return Err(StoreError::NotFound);
    }
    Ok(())
}

/// Store the source commit time of the last change the applier wrote to
/// `table`. It never moves backwards: a late report of an earlier batch changes
/// nothing.
///
/// # Errors
///
/// [`StoreError::Database`] on a failure, [`StoreError::NotFound`] for a
/// connector that does not exist.
pub async fn record_last_change(
    pool: &PgPool,
    connector_id: &str,
    table: &str,
    last_source_ts: OffsetDateTime,
) -> Result<(), StoreError> {
    let done = sqlx::query(
        "INSERT INTO connector_capture_table (connector_id, table_name, last_source_ts) \
         SELECT id, $2, $3 FROM connector WHERE id = $1 \
         ON CONFLICT (connector_id, table_name) DO UPDATE SET \
           last_source_ts = GREATEST(connector_capture_table.last_source_ts, EXCLUDED.last_source_ts)",
    )
    .bind(connector_id)
    .bind(table)
    .bind(last_source_ts)
    .execute(pool)
    .await?;
    if done.rows_affected() == 0 {
        return Err(StoreError::NotFound);
    }
    Ok(())
}

/// Every table of a connector that has been reported, by name.
///
/// # Errors
///
/// [`StoreError::Database`] on a failure.
pub async fn list_tables(
    pool: &PgPool,
    connector_id: &str,
) -> Result<Vec<CaptureTable>, StoreError> {
    let rows = sqlx::query_as(
        "SELECT table_name, last_source_ts, caught_up_at, backlog, observed_at \
         FROM connector_capture_table WHERE connector_id = $1 ORDER BY table_name",
    )
    .bind(connector_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// A connector's lag limit and whether its alert is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, FromRow)]
pub struct LagLimit {
    /// The limit, in minutes.
    pub minutes: i32,
    /// Whether a `connector_lag` alert has fired and not been answered by a
    /// "back under".
    pub open: bool,
}

/// Read a connector's lag limit and open flag.
///
/// # Errors
///
/// [`StoreError::NotFound`] for an unknown connector, [`StoreError::Database`]
/// otherwise.
pub async fn get_lag_limit(pool: &PgPool, connector_id: &str) -> Result<LagLimit, StoreError> {
    let row: Option<LagLimit> = sqlx::query_as(
        "SELECT lag_alert_minutes AS minutes, lag_alert_open AS open FROM connector WHERE id = $1",
    )
    .bind(connector_id)
    .fetch_optional(pool)
    .await?;
    row.ok_or(StoreError::NotFound)
}

/// Set a connector's lag limit. The caller has validated `minutes` against
/// [`MIN_LAG_ALERT_MINUTES`] and [`MAX_LAG_ALERT_MINUTES`]; the column's CHECK
/// repeats it.
///
/// # Errors
///
/// [`StoreError::NotFound`] for an unknown connector, [`StoreError::Database`]
/// otherwise.
pub async fn set_lag_limit(
    pool: &PgPool,
    connector_id: &str,
    minutes: i32,
) -> Result<(), StoreError> {
    let done = sqlx::query("UPDATE connector SET lag_alert_minutes = $2 WHERE id = $1")
        .bind(connector_id)
        .bind(minutes)
        .execute(pool)
        .await?;
    if done.rows_affected() == 0 {
        return Err(StoreError::NotFound);
    }
    Ok(())
}

/// Flip the open flag from `from` to `!from`. Returns `true` only for the call
/// that changed it: two evaluations racing over the same crossing see one `true`
/// and one `false`, so the alert is sent once (`SRC-4` S4-d).
///
/// # Errors
///
/// [`StoreError::Database`] on a failure.
pub async fn flip_lag_alert_open(
    pool: &PgPool,
    connector_id: &str,
    from: bool,
) -> Result<bool, StoreError> {
    let done = sqlx::query(
        "UPDATE connector SET lag_alert_open = $3 WHERE id = $1 AND lag_alert_open = $2",
    )
    .bind(connector_id)
    .bind(from)
    .bind(!from)
    .execute(pool)
    .await?;
    Ok(done.rows_affected() == 1)
}
