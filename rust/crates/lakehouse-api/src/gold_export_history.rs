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
//!
//! # Copy retention and trim history (DATA-2)
//!
//! `console.gold_trim_run` records each trim operation executed by Dagster's
//! `trim_gold_mart` op, capturing status, copies kept, oldest kept timestamp,
//! copies removed, snapshots expired, files removed, and fixed failure reasons.
//! [`last_trim`] reads the latest trim record per mart for inclusion in
//! `GET /api/gold/export/{mart}/publication`.

use lakehouse_clickhouse::{ChClient, ChError};
use lakehouse_core::ident::SqlLiteral;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::routes::support::{nullable_i64_col, nullable_u64_col};

/// Cached "has the table been created this process" flag, same pattern as
/// `lakehouse_bi::store::BI_TABLE_ENSURED` (a failed attempt is not
/// cached, so a transient `ClickHouse` outage doesn't permanently wedge
/// every later call).
static TABLE_ENSURED: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();

/// Idempotently create the `console` database (if `lakehouse-bi` has not
/// Cached flag for `console.gold_trim_run`.
static TRIM_TABLE_ENSURED: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();

/// Idempotently create `console.gold_trim_run` (DATA-2 D5 + A1).
///
/// # Errors
///
/// Returns [`ChError`] if any DDL statement fails.
pub async fn ensure_gold_trim_run_table(ch: &ChClient) -> Result<(), ChError> {
    TRIM_TABLE_ENSURED
        .get_or_try_init(|| ensure_gold_trim_run_table_uncached(ch))
        .await
        .map(drop)
}

async fn ensure_gold_trim_run_table_uncached(ch: &ChClient) -> Result<(), ChError> {
    ch.exec("CREATE DATABASE IF NOT EXISTS console", None)
        .await?;
    ch.exec(
        "CREATE TABLE IF NOT EXISTS console.gold_trim_run (\n\
           id String,\n\
           mart String,\n\
           status String,\n\
           copies_kept Nullable(UInt32),\n\
           oldest_kept_exported_at Nullable(DateTime64(3)),\n\
           copies_removed Nullable(UInt32),\n\
           snapshots_expired Nullable(UInt32),\n\
           files_removed Nullable(UInt32),\n\
           reason Nullable(String),\n\
           started_at DateTime64(3),\n\
           finished_at DateTime64(3)\n\
         ) ENGINE = MergeTree ORDER BY (mart, started_at)",
        None,
    )
    .await?;
    ch.exec(
        "ALTER TABLE console.gold_trim_run ADD COLUMN IF NOT EXISTS files_removed Nullable(UInt32)",
        None,
    )
    .await?;
    Ok(())
}

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
           content_fingerprint String DEFAULT '',\n\
           started_at DateTime64(3),\n\
           finished_at DateTime64(3)\n\
         ) ENGINE = MergeTree ORDER BY (mart, started_at)",
        None,
    )
    .await?;
    ch.exec(
        "ALTER TABLE console.gold_export_run ADD COLUMN IF NOT EXISTS content_fingerprint String DEFAULT ''",
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
    /// DATA-10: The content fingerprint of the mart before the export ran,
    /// formatted as `"<count>:<sum>:<schemaHash>"`, or an empty string if
    /// unavailable or not measured.
    pub content_fingerprint: &'a str,
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
          triggered_by, content_fingerprint, started_at, finished_at) VALUES \
         ({id}, {mart}, {status}, {rows_exported}, {format_version}, {snapshot_id}, {error}, \
          {triggered_by}, {content_fingerprint}, {started_at}, {finished_at})",
        id = SqlLiteral::from(uuid::Uuid::new_v4().to_string()),
        mart = SqlLiteral::from(row.mart),
        status = SqlLiteral::from(row.status),
        rows_exported = opt_num(row.rows_exported),
        format_version = opt_num(row.format_version),
        snapshot_id = opt_num(row.snapshot_id),
        error = opt_str(row.error),
        triggered_by = SqlLiteral::from(row.triggered_by),
        content_fingerprint = SqlLiteral::from(row.content_fingerprint),
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
          content_fingerprint, \
          toString(started_at) AS started_at, toString(finished_at) AS finished_at \
          FROM console.gold_export_run WHERE mart = {mart} \
          ORDER BY started_at DESC LIMIT {limit} FORMAT JSON",
        mart = SqlLiteral::from(mart),
    )
}

fn last_success_sql(mart: &str) -> String {
    format!(
        "SELECT toString(toUnixTimestamp64Milli(started_at)) AS started_ms, \
                content_fingerprint \
          FROM console.gold_export_run \
          WHERE mart = {mart} AND status = 'success' \
          ORDER BY started_at DESC LIMIT 1 FORMAT JSON",
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
    /// DATA-10: Stored content fingerprint of the mart before the export.
    pub content_fingerprint: String,
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
        content_fingerprint: str_field("content_fingerprint"),
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

/// The latest successful export run's start timestamp and content fingerprint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LastSuccessExport {
    /// Milliseconds since Unix epoch when the export started, or `None` if unparseable.
    pub started_at_ms: Option<i64>,
    /// Stored content fingerprint of the mart before the export.
    pub content_fingerprint: String,
}

/// DATA-10: Read `mart`'s newest `status = 'success'` export run, or `None` if it
/// has never exported successfully.
///
/// # Errors
///
/// Returns [`ChError`] if the table cannot be ensured or the query fails.
pub async fn last_success_export(
    ch: &ChClient,
    mart: &str,
) -> Result<Option<LastSuccessExport>, ChError> {
    ensure_gold_export_run_table(ch).await?;
    let rows = ch.rows(&last_success_sql(mart), None).await?;
    let Some(row) = rows.first() else {
        return Ok(None);
    };
    let started_at_ms = row
        .get("started_ms")
        .and_then(Value::as_str)
        .and_then(|ms| ms.parse::<i64>().ok());
    let content_fingerprint = row
        .get("content_fingerprint")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    Ok(Some(LastSuccessExport {
        started_at_ms,
        content_fingerprint,
    }))
}

/// Unix-millisecond `started_at` of `mart`'s newest `status = 'success'`
/// run, or `None` when it has never exported successfully. DATA-1's
/// publication detail route and `?ifChanged=true` both need "when was
/// this mart's Iceberg copy last refreshed" — the history table is that
/// fact's owner (every export attempt, both triggers, writes one row).
/// A stored timestamp that cannot be parsed also reads as `None`:
/// "unknown", never a guessed time.
///
/// # Errors
///
/// Returns [`ChError`] if the table cannot be ensured or the query fails.
#[allow(dead_code)]
pub async fn last_success_started_at(ch: &ChClient, mart: &str) -> Result<Option<i64>, ChError> {
    Ok(last_success_export(ch, mart)
        .await?
        .and_then(|e| e.started_at_ms))
}

/// DATA-10: Read the stored content fingerprint of `mart`'s newest
/// `status = 'success'` export run, or `None` if it has never exported
/// successfully or has an empty stored fingerprint.
///
/// # Errors
///
/// Returns [`ChError`] if the table cannot be ensured or the query fails.
#[allow(
    dead_code,
    reason = "convenience helper for callers wanting only the fingerprint"
)]
pub async fn last_success_fingerprint(
    ch: &ChClient,
    mart: &str,
) -> Result<Option<String>, ChError> {
    Ok(last_success_export(ch, mart)
        .await?
        .map(|e| e.content_fingerprint)
        .filter(|s| !s.is_empty()))
}

/// DATA-10 review SHOULD-FIX 1: Whether `{schema}.{mart}` exists in `system.tables`.
///
/// # Errors
///
/// Returns [`ChError`] if the query fails.
pub async fn mart_exists(ch: &ChClient, schema: &str, mart: &str) -> Result<bool, ChError> {
    let sql = format!(
        "SELECT toString(count()) AS n FROM system.tables \
         WHERE database = {schema} AND name = {mart}",
        schema = SqlLiteral::from(schema),
        mart = SqlLiteral::from(mart),
    );
    let rows = ch.rows(&sql, None).await?;
    Ok(rows
        .first()
        .and_then(|row| row.get("n"))
        .and_then(Value::as_str)
        .is_some_and(|n| n != "0"))
}

/// DATA-10 review SHOULD-FIX 1: Read the content fingerprint of `mart` in `schema`.
///
/// Computes `count()`, `sum(cityHash64(*))`, and a hash of the column
/// list from `system.columns`. Returns `Some("<count>:<sum>:<schemaHash>")`,
/// or `None` if the mart does not exist in `system.tables`.
///
/// # Errors
///
/// Returns [`ChError`] if a `ClickHouse` query fails.
pub async fn mart_content_fingerprint(
    ch: &ChClient,
    schema: &str,
    mart: &str,
) -> Result<Option<String>, ChError> {
    if !mart_exists(ch, schema, mart).await? {
        return Ok(None);
    }
    let sql = content_fingerprint_sql(schema, mart);
    let rows = ch.rows(&sql, None).await?;
    let Some(row) = rows.first() else {
        return Ok(None);
    };
    Ok(parse_content_fingerprint_row(row))
}

// DATA-10 review SHOULD-FIX 1 / Nit: use `\` line continuations like the other queries in the file.
fn content_fingerprint_sql(schema: &str, mart: &str) -> String {
    format!(
        "SELECT \
             toString(count()) AS row_count, \
             toString(ifNull(sum(cityHash64(*)), toUInt64(0))) AS content_sum, \
             ( \
                 SELECT toString(cityHash64(groupArray(concat(name, \x27:\x27, type)))) \
                 FROM ( \
                     SELECT name, type \
                     FROM system.columns \
                     WHERE database = {schema_lit} AND table = {mart_lit} \
                     ORDER BY position \
                 ) \
             ) AS schema_hash \
         FROM `{clean_schema}`.`{clean_mart}` FORMAT JSON",
        schema_lit = SqlLiteral::from(schema),
        mart_lit = SqlLiteral::from(mart),
        clean_schema = schema.replace('`', "``"),
        clean_mart = mart.replace('`', "``"),
    )
}

fn parse_content_fingerprint_row(row: &serde_json::Map<String, Value>) -> Option<String> {
    let str_val = |key: &str| -> Option<String> {
        row.get(key).and_then(|v| match v {
            Value::String(s) => Some(s.clone()),
            Value::Number(n) => Some(n.to_string()),
            _ => None,
        })
    };
    let row_count = str_val("row_count")?;
    let content_sum = str_val("content_sum")?;
    let schema_hash = str_val("schema_hash")?;
    Some(format!("{row_count}:{content_sum}:{schema_hash}"))
}

/// The latest trim run summary from `console.gold_trim_run` (DATA-2 D6).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GoldTrimSummary {
    /// Trim run status (`"success"`, `"failed"`, `"nothing_to_trim"`).
    pub status: String,
    /// Number of copies kept after trimming, or `None` if failed or not measured.
    pub copies_kept: Option<u32>,
    /// Oldest retained copy's `_exported_at` formatted as RFC 3339, or `None`.
    pub oldest_kept_exported_at: Option<String>,
    /// RFC 3339 timestamp when the trim run finished.
    pub finished_at: String,
    /// Failure or status reason from a closed set of fixed strings, or `None`.
    pub reason: Option<String>,
}

/// Convert unix epoch milliseconds to an RFC 3339 UTC string.
#[must_use]
pub fn millis_to_rfc3339(ms: i64) -> Option<String> {
    time::OffsetDateTime::from_unix_timestamp(ms / 1000)
        .ok()
        .and_then(|dt| {
            dt.format(&time::format_description::well_known::Rfc3339)
                .ok()
        })
}

fn last_trim_sql(mart: &str) -> String {
    format!(
        "SELECT status, copies_kept, \
                toString(toUnixTimestamp64Milli(oldest_kept_exported_at)) AS oldest_kept_ms, \
                toString(toUnixTimestamp64Milli(finished_at)) AS finished_ms, \
                reason \
         FROM console.gold_trim_run \
         WHERE mart = {mart} \
         ORDER BY started_at DESC LIMIT 1 FORMAT JSON",
        mart = SqlLiteral::from(mart),
    )
}

fn row_to_trim_summary(row: &serde_json::Map<String, Value>) -> GoldTrimSummary {
    let status = row
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let copies_kept = nullable_u64_col(row, "copies_kept").and_then(|v| u32::try_from(v).ok());
    let oldest_kept_exported_at = row
        .get("oldest_kept_ms")
        .and_then(Value::as_str)
        .and_then(|ms| ms.parse::<i64>().ok())
        .and_then(millis_to_rfc3339);
    let finished_at = row
        .get("finished_ms")
        .and_then(Value::as_str)
        .and_then(|ms| ms.parse::<i64>().ok())
        .and_then(millis_to_rfc3339)
        .unwrap_or_default();
    let reason = row
        .get("reason")
        .filter(|v| !v.is_null())
        .and_then(Value::as_str)
        .map(str::to_owned);
    GoldTrimSummary {
        status,
        copies_kept,
        oldest_kept_exported_at,
        finished_at,
        reason,
    }
}

/// Read `mart`'s newest trim run summary from `console.gold_trim_run`, or
/// `None` if it has never been trimmed (DATA-2 D6).
///
/// # Errors
///
/// Returns [`ChError`] if the table cannot be ensured or the query fails.
pub async fn last_trim(ch: &ChClient, mart: &str) -> Result<Option<GoldTrimSummary>, ChError> {
    ensure_gold_trim_run_table(ch).await?;
    let rows = ch.rows(&last_trim_sql(mart), None).await?;
    let Some(row) = rows.first() else {
        return Ok(None);
    };
    Ok(Some(row_to_trim_summary(row)))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn last_trim_sql_orders_by_started_at_desc_and_binds_mart() {
        let sql = last_trim_sql("orders");
        assert!(sql.contains("ORDER BY started_at DESC LIMIT 1"), "{sql}");
        assert!(sql.contains("console.gold_trim_run"), "{sql}");
        assert!(sql.contains("'orders'"), "{sql}");
    }

    #[test]
    fn row_to_trim_summary_parses_success_row() {
        let mut row = serde_json::Map::new();
        row.insert("status".to_owned(), Value::String("success".to_owned()));
        row.insert("copies_kept".to_owned(), Value::Number(7.into()));
        row.insert(
            "oldest_kept_ms".to_owned(),
            Value::String("1700000000000".to_owned()),
        );
        row.insert(
            "finished_ms".to_owned(),
            Value::String("1700000010000".to_owned()),
        );
        row.insert("reason".to_owned(), Value::Null);

        let summary = row_to_trim_summary(&row);
        assert_eq!(summary.status, "success");
        assert_eq!(summary.copies_kept, Some(7));
        assert!(summary.oldest_kept_exported_at.is_some());
        assert!(!summary.finished_at.is_empty());
        assert_eq!(summary.reason, None);
    }

    #[test]
    fn row_to_trim_summary_parses_failed_row() {
        let mut row = serde_json::Map::new();
        row.insert("status".to_owned(), Value::String("failed".to_owned()));
        row.insert("copies_kept".to_owned(), Value::Null);
        row.insert("oldest_kept_ms".to_owned(), Value::Null);
        row.insert(
            "finished_ms".to_owned(),
            Value::String("1700000010000".to_owned()),
        );
        row.insert(
            "reason".to_owned(),
            Value::String("delete_failed".to_owned()),
        );

        let summary = row_to_trim_summary(&row);
        assert_eq!(summary.status, "failed");
        assert_eq!(summary.copies_kept, None);
        assert_eq!(summary.oldest_kept_exported_at, None);
        assert_eq!(summary.reason, Some("delete_failed".to_owned()));
    }

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
            content_fingerprint: "10:12345:67890",
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
        assert!(sql.contains("'10:12345:67890'"), "{sql}");
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
            content_fingerprint: "",
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

    #[test]
    fn last_success_sql_filters_to_success_and_takes_only_the_newest_row() {
        let sql = last_success_sql("sales");
        assert!(sql.contains("status = 'success'"), "{sql}");
        assert!(sql.contains("ORDER BY started_at DESC LIMIT 1"), "{sql}");
        assert!(sql.contains("toUnixTimestamp64Milli"), "{sql}");
        assert!(sql.contains("content_fingerprint"), "{sql}");
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
            "triggered_by": "t", "content_fingerprint": "7:999:111",
            "started_at": "s", "finished_at": "f"
        });
        let row = row_to_export_run(as_numbers.as_object().expect("object"));
        assert_eq!(row.rows_exported, Some(7));
        assert_eq!(row.format_version, Some(2));
        assert_eq!(row.snapshot_id, Some(123));
        assert_eq!(row.content_fingerprint, "7:999:111");

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
    // ── content_fingerprint (DATA-10 T1 / SHOULD-FIX 1) ─────────────────

    #[test]
    fn content_fingerprint_sql_escapes_identifiers_and_queries_columns() {
        let sql = content_fingerprint_sql("serving", "sales'; DROP TABLE x; --");
        assert!(sql.contains("sum(cityHash64(*))"), "{sql}");
        assert!(sql.contains("system.columns"), "{sql}");
        assert!(sql.contains("groupArray(concat(name, ':', type))"), "{sql}");
        assert!(sql.contains("ORDER BY position"), "{sql}");
        assert!(sql.contains("'sales''; DROP TABLE x; --'"), "{sql}");
        assert!(sql.contains("'serving'"), "{sql}");
        assert!(
            sql.contains("`serving`.`sales'; DROP TABLE x; --`"),
            "{sql}"
        );

        let sql_backtick = content_fingerprint_sql("serv`ing", "sal`es");
        assert!(
            sql_backtick.contains("`serv``ing`.`sal``es`"),
            "{sql_backtick}"
        );
    }

    #[test]
    fn parse_content_fingerprint_row_keeps_large_uint64_exact() {
        // UInt64 sum above 2^53 (9_007_199_254_740_992) must not suffer
        // float rounding.
        let large_sum = "17600050468841063192";
        let row: serde_json::Map<String, Value> = serde_json::from_value(serde_json::json!({
            "row_count": "42",
            "content_sum": large_sum,
            "schema_hash": "987654321012345678"
        }))
        .unwrap();
        let fp = parse_content_fingerprint_row(&row).expect("fingerprint parsed");
        assert_eq!(fp, format!("42:{large_sum}:987654321012345678"));
    }

    #[test]
    fn parse_content_fingerprint_row_returns_none_on_missing_field() {
        let row: serde_json::Map<String, Value> = serde_json::from_value(serde_json::json!({
            "row_count": "42",
            "content_sum": null,
            "schema_hash": "123"
        }))
        .unwrap();
        assert!(parse_content_fingerprint_row(&row).is_none());
    }
}
