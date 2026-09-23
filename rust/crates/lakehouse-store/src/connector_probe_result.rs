//! Per-connector connectivity-probe HISTORY (`connector_probe_result`,
//! `0044_connector_probe_result.sql`) -- distinct from
//! `connector.health`/`last_test_at`, which only ever hold the current
//! state of the most recent probe. See that migration's header comment
//! for the full rationale.
//!
//! Nothing in this module ever inserts a row itself outside
//! [`crate::connectors::record_test_result`]: that function is the only
//! writer, and it only ever writes a SUPPORTED probe's outcome, in the
//! same transaction as the `connector.health`/`last_test_at` update. This
//! module's own [`insert_and_trim`] is the mechanics that call uses; a
//! second caller inserting through some other path would break the
//! "current state and newest history row always agree" guarantee that
//! transaction exists to provide.
//!
//! Bounded to [`MAX_ROWS_PER_CONNECTOR`] rows per connector -- see the
//! migration header comment for why this is a count cap, not a calendar
//! window.

use serde::Serialize;
use sqlx::FromRow;
use time::OffsetDateTime;

use crate::{PgPool, StoreError};

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

/// The most recent probe results, newest first, for one connector.
/// Mirrors `ConnectorProbeResult` in `contracts/connectors.ts`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorProbeResult {
    /// When this probe ran, ISO 8601 -- the same `last_test_at` value the
    /// current-state `connector` row was stamped with by the same
    /// transaction that inserted this row.
    pub tested_at: String,
    /// Whether the probe succeeded.
    pub ok: bool,
    /// Real measured latency in milliseconds, or `None` if the probe
    /// attempt itself never completed (e.g. a misconfigured dial).
    pub latency_ms: Option<i64>,
    /// The probe's human-readable result message -- see
    /// `record_test_result`'s doc comment for why this is always safe to
    /// store and return verbatim.
    pub message: String,
}

/// The row [`list_probe_results`] selects, before it is reshaped into
/// [`ConnectorProbeResult`] (`tested_at` formatted to ISO 8601).
#[derive(FromRow)]
struct ProbeResultRow {
    tested_at: OffsetDateTime,
    ok: bool,
    latency_ms: Option<i64>,
    message: String,
}

impl From<ProbeResultRow> for ConnectorProbeResult {
    fn from(row: ProbeResultRow) -> Self {
        Self {
            tested_at: iso_millis(row.tested_at),
            ok: row.ok,
            latency_ms: row.latency_ms,
            message: row.message,
        }
    }
}

/// List `connector_id`'s most recent probe results, newest first.
///
/// Ordered by `id DESC`, not `tested_at DESC`: rows inserted by the same
/// transaction share the transaction's `now()` value (and two probes
/// could in principle even be bound the same millisecond by a caller), so
/// `tested_at` alone cannot break a tie -- `id`, an `IDENTITY` column, is
/// strictly increasing by insertion order and never ties.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the query fails.
pub async fn list_probe_results(
    pool: &PgPool,
    connector_id: &str,
    limit: i64,
) -> Result<Vec<ConnectorProbeResult>, StoreError> {
    let rows: Vec<ProbeResultRow> = sqlx::query_as(
        "SELECT tested_at, ok, latency_ms, message FROM connector_probe_result \
         WHERE connector_id = $1 ORDER BY id DESC LIMIT $2",
    )
    .bind(connector_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(ConnectorProbeResult::from).collect())
}

/// The most history rows [`insert_and_trim`] ever leaves for one
/// connector -- see the migration header comment for why this is a count
/// cap, not a calendar window.
const MAX_ROWS_PER_CONNECTOR: i64 = 200;

/// Insert one probe-result row for `connector_id`, then trim that
/// connector's rows down to the newest [`MAX_ROWS_PER_CONNECTOR`].
///
/// Takes a `&mut PgConnection` (a transaction's connection, per
/// `sqlx::Transaction`'s `Deref`) rather than a `&PgPool`, so the caller
/// (`record_test_result`) can run this in the SAME transaction as its own
/// `connector` UPDATE -- see this module's doc comment for why that
/// matters.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if either query fails.
pub(crate) async fn insert_and_trim(
    conn: &mut sqlx::PgConnection,
    connector_id: &str,
    tested_at: OffsetDateTime,
    ok: bool,
    latency_ms: Option<i64>,
    message: &str,
) -> Result<(), StoreError> {
    sqlx::query(
        "INSERT INTO connector_probe_result (connector_id, tested_at, ok, latency_ms, message) \
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(connector_id)
    .bind(tested_at)
    .bind(ok)
    .bind(latency_ms)
    .bind(message)
    .execute(&mut *conn)
    .await?;

    sqlx::query(
        "DELETE FROM connector_probe_result WHERE connector_id = $1 AND id NOT IN \
         (SELECT id FROM connector_probe_result WHERE connector_id = $1 ORDER BY id DESC LIMIT $2)",
    )
    .bind(connector_id)
    .bind(MAX_ROWS_PER_CONNECTOR)
    .execute(&mut *conn)
    .await?;

    Ok(())
}
