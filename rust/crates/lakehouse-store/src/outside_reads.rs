//! The daily read counter for outside tools (DATA-5, ADR 0016).
//!
//! ADR 0016 decision 5: a successful table load through the API's door
//! (`/api/iceberg/v1/...`) adds one read to a daily counter for that tool
//! and that table. A load whose count cannot be written is REFUSED (503)
//! rather than served uncounted — an uncounted read would make the count
//! a lie (principle 5), which is why this is a write on the request path
//! and not a fire-and-forget side effect.
//!
//! # Why the name is stored, not joined
//!
//! `identity_name` is the tool's name AT THE TIME OF THE READ, and
//! `identity_id` carries no foreign key (see the migration's own header):
//! ADR 0016 requires a deleted tool's past reads to stay readable, so
//! the history must not vanish with the identity row. The name of the
//! most recent read in the window is the one reported — a tool that was
//! renamed mid-week is shown under the name it had when it last read.
//!
//! # Why a daily row and not a running total
//!
//! The page shows 7 days and the history is kept for 90
//! ([`RETENTION_DAYS`]), so the counter is a daily row: pruning the
//! history is a plain `DELETE` of old rows and needs no rewrite of a
//! total that has to survive it. [`outside_readers_of`] SUMS the daily
//! rows over the window it is asked about.

use serde::Serialize;
use sqlx::FromRow;
use time::{Date, OffsetDateTime};
use uuid::Uuid;

use crate::{PgPool, StoreError};

/// How many days of read history are kept. ADR 0016 decision 5: "Rows
/// older than 90 days are deleted."
pub const RETENTION_DAYS: i32 = 90;

/// One tool that read a published table, over the window a reader asked
/// about.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutsideReader {
    /// The tool's service identity id. Kept even when the identity row
    /// is gone: the history outlives the tool.
    pub identity_id: Uuid,
    /// The tool's name at the time of its most recent read in the window.
    pub name: String,
    /// How many reads the tool made over the window, summed over the
    /// days.
    pub reads: i64,
    /// The most recent read in the window, ISO 8601 (millisecond
    /// precision, UTC).
    pub last_read_at: String,
}

#[derive(Debug, FromRow)]
struct ReaderRow {
    identity_id: Uuid,
    identity_name: String,
    reads: i64,
    last_read_at: OffsetDateTime,
}

/// Render a timestamp the way the console's contracts expect it: ISO 8601
/// with millisecond precision, in UTC.
fn iso_millis(at: OffsetDateTime) -> String {
    let at = at.to_offset(time::UtcOffset::UTC);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        at.year(),
        u8::from(at.month()),
        at.day(),
        at.hour(),
        at.minute(),
        at.second(),
        at.millisecond()
    )
}

/// Count one read of `namespace.table` by `identity_id`, under the name
/// the tool has right now.
///
/// Idempotent per (day, tool, table): a second read on the same day adds
/// to the day's row rather than appending a second one, and moves that
/// row's `last_read_at` and name to the newest read. The name is
/// overwritten on purpose — see this module's header: the name of the
/// most recent read is the one reported.
///
/// # Errors
///
/// Returns [`StoreError::Database`] on any query failure. The caller
/// (the API's door) must treat that as "the read cannot be counted" and
/// refuse the load, never serve it uncounted.
pub async fn record_outside_read(
    pool: &PgPool,
    identity_id: Uuid,
    identity_name: &str,
    namespace: &str,
    table_name: &str,
) -> Result<(), StoreError> {
    sqlx::query(
        "INSERT INTO outside_table_read_day \
           (day, identity_id, identity_name, namespace, table_name, reads, last_read_at) \
         VALUES (current_date, $1, $2, $3, $4, 1, now()) \
         ON CONFLICT (day, identity_id, namespace, table_name) DO UPDATE SET \
            reads         = outside_table_read_day.reads + 1, \
            identity_name = EXCLUDED.identity_name, \
            last_read_at  = now()",
    )
    .bind(identity_id)
    .bind(identity_name)
    .bind(namespace)
    .bind(table_name)
    .execute(pool)
    .await?;
    Ok(())
}

/// Every tool that read `namespace.table` on or after `since`, with the
/// reads summed over that window and the list ordered by reads
/// descending (most-read first).
///
/// A tool that read nothing in the window is not in the list; a tool
/// whose identity row has since been deleted still is, under the name it
/// had when it last read (see this module's header).
///
/// # Errors
///
/// Returns [`StoreError::Database`] on any query failure.
pub async fn outside_readers_of(
    pool: &PgPool,
    namespace: &str,
    table_name: &str,
    since: Date,
) -> Result<Vec<OutsideReader>, StoreError> {
    // The inner query sums the daily rows per tool and numbers them by
    // recency, so the outer one can pick the row carrying the tool's
    // newest name and last read while keeping the summed count.
    //
    // `SUM(...) OVER (...)` over a `BIGINT` column returns `NUMERIC` in
    // Postgres, so the sum is cast back to `BIGINT` here — a read count
    // that overflowed an i64 would be a bug far beyond this table's
    // lifetime, and the cast keeps the column type the row struct
    // declares.
    let rows: Vec<ReaderRow> = sqlx::query_as(
        "WITH per_day AS ( \
             SELECT identity_id, \
                    identity_name, \
                    reads, \
                    last_read_at, \
                    (SUM(reads) OVER (PARTITION BY identity_id))::bigint AS window_reads, \
                    ROW_NUMBER() OVER (PARTITION BY identity_id \
                                       ORDER BY last_read_at DESC, day DESC) AS recency \
               FROM outside_table_read_day \
              WHERE namespace = $1 AND table_name = $2 AND day >= $3 \
         ) \
         SELECT identity_id, identity_name, window_reads AS reads, last_read_at \
           FROM per_day \
          WHERE recency = 1 \
          ORDER BY reads DESC, identity_id",
    )
    .bind(namespace)
    .bind(table_name)
    .bind(since)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| OutsideReader {
            identity_id: row.identity_id,
            name: row.identity_name,
            reads: row.reads,
            last_read_at: iso_millis(row.last_read_at),
        })
        .collect())
}

/// Delete the read history older than `keep_days` days.
///
/// Returns how many rows were removed. Called at most once a day per
/// process (the API keeps the last prune date in memory), so this is a
/// cheap maintenance statement, never a per-request one.
///
/// # Errors
///
/// Returns [`StoreError::Database`] on any query failure.
pub async fn prune_outside_reads(pool: &PgPool, keep_days: i32) -> Result<u64, StoreError> {
    let deleted = sqlx::query("DELETE FROM outside_table_read_day WHERE day < current_date - $1")
        .bind(keep_days)
        .execute(pool)
        .await?;
    Ok(deleted.rows_affected())
}
