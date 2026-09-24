//! `console.gold_export_run` — per-run history for `GET /api/gold/exports?mart=`
//! (WS6 item 3). This is a NEW `ClickHouse` table
//! in the `console` database — the same database
//! `lakehouse-bi::store::ensure_bi_table` already owns
//! (`console.bi_chart`/`console.bi_board`) — but this table's DDL is owned
//! HERE, not there: Gold export's other Rust glue (`gold_export.rs`,
//! `gold_lock.rs`) already lives in `lakehouse-api`, not `lakehouse-bi`,
//! and `lakehouse-api` has no dependency on `lakehouse-bi` internals for
//! this. `console.gold_export_run` cannot be declared through Dagster's
//! `EXPECTED_SCHEMAS` single-owner mechanism either: that mechanism
//! (`dagster/dispar_orchestrate/bronze_catalog.py`) only ever creates
//! tables inside the `lake` database (`TableSchema.create_ddl` renders
//! `CREATE TABLE IF NOT EXISTS lake.{table_name} ...`, backtick-quoting
//! `{table_name}` itself) — it
//! has no path to a `console.*` table at all. So this module gets the same
//! kind of single owner `lakehouse-bi::store` uses for its own
//! `console.*` tables: an idempotent Rust-side `ensure_*` function.
//!
//! One row per call to `POST /api/gold/export/{mart}`, success or failure,
//! written by `routes::gold::export` itself right after
//! `gold_export::export_mart` returns — this captures BOTH trigger sources
//! (the console's "Export now" button and the Dagster schedule) through
//! the one HTTP handler both call, rather than recording only
//! Dagster-triggered runs.

use lakehouse_clickhouse::{ChClient, ChError};
use lakehouse_core::ident::SqlLiteral;
use serde::Serialize;
use serde_json::Value;

use crate::routes::support::{nullable_i64_col, nullable_u64_col};

/// Cached "has the table been created this process" flag, same pattern as
/// `lakehouse_bi::store::BI_TABLE_ENSURED` (a failed attempt is not
/// cached, so a transient `ClickHouse` outage doesn't permanently wedge
/// every later call).
static TABLE_ENSURED: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();

/// Idempotently create the `console` database (if `lakehouse-bi` has not
/// already, on this process) and `console.gold_export_run`.
///
/// # Errors
///
/// Returns [`ChError`] if any DDL statement fails.
pub async fn ensure_gold_export_run_table(ch: &ChClient) -> Result<(), ChError> {
    TABLE_ENSURED
        .get_or_try_init(|| ensure_gold_export_run_table_uncached(ch))
        .await
        .map(drop)
}

async fn ensure_gold_export_run_table_uncached(ch: &ChClient) -> Result<(), ChError> {
    ch.exec("CREATE DATABASE IF NOT EXISTS console", None)
        .await?;
    ch.exec(
        "CREATE TABLE IF NOT EXISTS console.gold_export_run (\n\
           id String,\n\
           mart String,\n\
           status String,\n\
           rows_exported Nullable(UInt64),\n\
           format_version Nullable(UInt8),\n\
           snapshot_id Nullable(Int64),\n\
           error Nullable(String),\n\
           triggered_by String,\n\
           started_at DateTime64(3),\n\
           finished_at DateTime64(3)\n\
         ) ENGINE = MergeTree ORDER BY (mart, started_at)",
        None,
    )
    .await?;
    Ok(())
}

/// One export attempt to record — always both an id (the caller mints a
/// UUID) and the outcome, never a partial row: `routes::gold::export`
/// calls this exactly once per call, after `export_mart` has already
/// returned `Ok` or `Err`.
pub struct NewGoldExportRun<'a> {
    /// The Gold mart's identifier (already validated by the caller's
    /// `Ident`), unquoted.
    pub mart: &'a str,
    /// `"success"` or `"failed"` — a closed, two-value set; callers build
    /// this from `result.is_ok()`, never free text.
    pub status: &'a str,
    /// Row count the export wrote, when the export succeeded — `None`,
    /// never `0`, for a run the backend never measured (a failed export
    /// never populates this).
    pub rows_exported: Option<u64>,
    /// The Iceberg table's format version at export time, when the export
    /// succeeded.
    pub format_version: Option<u8>,
    /// The Iceberg table's snapshot id at export time, when the export
    /// succeeded and the source table has a current snapshot.
    pub snapshot_id: Option<i64>,
    /// Classified error text (never raw upstream error bodies) — the
    /// `Display` of `GoldExportError`, which is already a closed,
    /// hand-written message per variant (see `gold_export.rs`).
    pub error: Option<&'a str>,
    /// `"user:<display name>"` or `"service:<display name>"` — see
    /// `routes::gold::export`'s call site for how this is built from a
    /// `Principal`.
    pub triggered_by: &'a str,
    /// Unix-millisecond timestamp when the export attempt began.
    pub started_at_ms: i64,
    /// Unix-millisecond timestamp when the export attempt finished
    /// (success or failure).
    pub finished_at_ms: i64,
}

fn opt_num(v: Option<impl std::fmt::Display>) -> String {
    v.map_or_else(|| "NULL".to_owned(), |n| n.to_string())
}

fn opt_str(v: Option<&str>) -> String {
    v.map_or_else(|| "NULL".to_owned(), |s| SqlLiteral::from(s).to_string())
}

/// `DateTime64(3)` literal from a Unix-millisecond timestamp.
fn datetime64_literal(ms: i64) -> String {
    format!("fromUnixTimestamp64Milli({ms})")
}

fn insert_sql(row: &NewGoldExportRun<'_>) -> String {
    format!(
        "INSERT INTO console.gold_export_run \
         (id, mart, status, rows_exported, format_version, snapshot_id, error, \
          triggered_by, started_at, finished_at) VALUES \
         ({id}, {mart}, {status}, {rows_exported}, {format_version}, {snapshot_id}, {error}, \
          {triggered_by}, {started_at}, {finished_at})",
        id = SqlLiteral::from(uuid::Uuid::new_v4().to_string()),
        mart = SqlLiteral::from(row.mart),
        status = SqlLiteral::from(row.status),
        rows_exported = opt_num(row.rows_exported),
        format_version = opt_num(row.format_version),
        snapshot_id = opt_num(row.snapshot_id),
        error = opt_str(row.error),
        triggered_by = SqlLiteral::from(row.triggered_by),
        started_at = datetime64_literal(row.started_at_ms),
        finished_at = datetime64_literal(row.finished_at_ms),
    )
}

/// Record one export attempt. Best-effort: a failure here is logged and
/// swallowed by the caller (`routes::gold::export`), never turned into a
/// 500 for what was otherwise a successful (or already-failed, for a
/// different reason) export — this table is a history view, not a
/// correctness dependency of the export itself.
///
/// # Errors
///
/// Returns [`ChError`] if the table cannot be ensured or the insert
/// fails.
pub async fn record_export_run(ch: &ChClient, row: &NewGoldExportRun<'_>) -> Result<(), ChError> {
    ensure_gold_export_run_table(ch).await?;
    ch.exec(&insert_sql(row), None).await
}

fn list_sql(mart: &str, limit: u32) -> String {
    format!(
        "SELECT id, status, rows_exported, format_version, snapshot_id, error, triggered_by, \
         toString(started_at) AS started_at, toString(finished_at) AS finished_at \
         FROM console.gold_export_run WHERE mart = {mart} \
         ORDER BY started_at DESC LIMIT {limit} FORMAT JSON",
        mart = SqlLiteral::from(mart),
    )
}

/// One row of `GET /api/gold/exports?mart=`'s `runs` array, already
/// shaped for `serde_json::json!` (camelCase keys, JSON-native types).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GoldExportRunRow {
    /// The UUID minted for this row by [`insert_sql`], rendered back as a
    /// plain string.
    pub id: String,
    /// `"success"` or `"failed"` — mirrors [`NewGoldExportRun::status`].
    pub status: String,
    /// Mirrors [`NewGoldExportRun::rows_exported`].
    pub rows_exported: Option<u64>,
    /// Mirrors [`NewGoldExportRun::format_version`].
    pub format_version: Option<u8>,
    /// Mirrors [`NewGoldExportRun::snapshot_id`].
    pub snapshot_id: Option<i64>,
    /// Mirrors [`NewGoldExportRun::error`].
    pub error: Option<String>,
    /// Mirrors [`NewGoldExportRun::triggered_by`].
    pub triggered_by: String,
    /// `ClickHouse`'s `toString(started_at)` rendering — a plain UTC
    /// datetime string, not yet RFC 3339 (the console formats this for
    /// display; this route reports it verbatim).
    pub started_at: String,
    /// Mirrors [`Self::started_at`].
    pub finished_at: String,
}

fn row_to_export_run(row: &serde_json::Map<String, Value>) -> GoldExportRunRow {
    let str_field = |key: &str| {
        row.get(key)
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned()
    };
    GoldExportRunRow {
        id: str_field("id"),
        status: str_field("status"),
        rows_exported: nullable_u64_col(row, "rows_exported"),
        // `ClickHouse` renders these integers as JSON numbers or as quoted
        // strings depending on `output_format_json_quote_64bit_integers`
        // (and never quotes a `UInt8`); reading only strings turned every
        // real value into `None`.
        format_version: nullable_u64_col(row, "format_version").and_then(|v| u8::try_from(v).ok()),
        snapshot_id: nullable_i64_col(row, "snapshot_id"),
        error: row
            .get("error")
            .filter(|v| !v.is_null())
            .and_then(Value::as_str)
            .map(str::to_owned),
        triggered_by: str_field("triggered_by"),
        started_at: str_field("started_at"),
        finished_at: str_field("finished_at"),
    }
}

/// The most recent `limit` export attempts for `mart`, newest first.
///
/// # Errors
///
/// Returns [`ChError`] if the table cannot be ensured or the query fails.
pub async fn list_export_runs(
    ch: &ChClient,
    mart: &str,
    limit: u32,
) -> Result<Vec<GoldExportRunRow>, ChError> {
    ensure_gold_export_run_table(ch).await?;
    let result = ch.query(&list_sql(mart, limit), None).await?;
    Ok(result.data.iter().map(row_to_export_run).collect())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn insert_sql_escapes_every_string_field() {
        let row = NewGoldExportRun {
            mart: "sales'; DROP TABLE x; --",
            status: "success",
            rows_exported: Some(7),
            format_version: Some(2),
            snapshot_id: Some(123),
            error: None,
            triggered_by: "user:o'brien",
            started_at_ms: 1_700_000_000_000,
            finished_at_ms: 1_700_000_001_000,
        };
        let sql = insert_sql(&row);
        // Escaping (SqlLiteral's doubled-quote convention) makes the
        // injected quote inert — it neutralizes the payload as harmless
        // text INSIDE the string literal, it does not (and cannot, without
        // parsing SQL semantics out of a plain string) make the substring
        // "DROP TABLE" vanish from the rendered text. What actually proves
        // the injection is defused is that the lone `'` right after
        // "sales" comes back doubled (`''`), which is what stops the
        // literal from being closed early — asserting `!sql.contains("DROP
        // TABLE")` would be testing the wrong thing (and always fails,
        // since escaping is not redaction).
        assert!(sql.contains("sales''; DROP TABLE x; --"), "{sql}");
        assert!(sql.contains("user:o''brien"), "{sql}");
    }

    #[test]
    fn insert_sql_renders_null_for_absent_optional_fields() {
        let row = NewGoldExportRun {
            mart: "sales",
            status: "failed",
            rows_exported: None,
            format_version: None,
            snapshot_id: None,
            error: Some("row cap exceeded"),
            triggered_by: "service:gold-export-scheduler",
            started_at_ms: 1_700_000_000_000,
            finished_at_ms: 1_700_000_000_500,
        };
        let sql = insert_sql(&row);
        assert!(sql.contains("NULL"), "{sql}");
        assert!(sql.contains("row cap exceeded"), "{sql}");
    }

    #[test]
    fn list_sql_orders_by_started_at_descending_and_binds_the_mart() {
        let sql = list_sql("sales", 50);
        assert!(sql.contains("ORDER BY started_at DESC"), "{sql}");
        assert!(sql.contains("LIMIT 50"), "{sql}");
        assert!(sql.contains("'sales'"), "{sql}");
    }

    /// `ClickHouse` 26.8's `FORMAT JSON` renders these integer columns as
    /// bare JSON numbers (`output_format_json_quote_64bit_integers` is off
    /// by default there); a server with it on sends quoted strings. Both
    /// must round-trip, and a `NULL` must stay `None`, never `0`.
    #[test]
    fn row_to_export_run_reads_integers_as_numbers_or_quoted_strings() {
        let as_numbers = serde_json::json!({
            "id": "r1", "status": "success", "rows_exported": 7,
            "format_version": 2, "snapshot_id": 123, "error": null,
            "triggered_by": "t", "started_at": "s", "finished_at": "f"
        });
        let row = row_to_export_run(as_numbers.as_object().expect("object"));
        assert_eq!(row.rows_exported, Some(7));
        assert_eq!(row.format_version, Some(2));
        assert_eq!(row.snapshot_id, Some(123));

        let as_strings = serde_json::json!({
            "id": "r2", "status": "success", "rows_exported": "7",
            "format_version": "2", "snapshot_id": "-5", "error": null,
            "triggered_by": "t", "started_at": "s", "finished_at": "f"
        });
        let row = row_to_export_run(as_strings.as_object().expect("object"));
        assert_eq!(row.rows_exported, Some(7));
        assert_eq!(row.format_version, Some(2));
        assert_eq!(row.snapshot_id, Some(-5));

        let as_nulls = serde_json::json!({
            "id": "r3", "status": "failed", "rows_exported": null,
            "format_version": null, "snapshot_id": null, "error": "e",
            "triggered_by": "t", "started_at": "s", "finished_at": "f"
        });
        let row = row_to_export_run(as_nulls.as_object().expect("object"));
        assert_eq!(row.rows_exported, None);
        assert_eq!(row.format_version, None);
        assert_eq!(row.snapshot_id, None);
    }
}
