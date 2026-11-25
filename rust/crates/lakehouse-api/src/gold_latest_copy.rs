//! `DATA-6`: the API's half of the latest-copy table — launch, record,
//! read.
//!
//! Every successful publish that wrote rows is followed by ONE Dagster
//! run of `gold_latest_copy_job` for that mart (D6), launched from
//! `routes::gold::export`, the one place every publish goes through —
//! a manual "Publish now", a scheduled run, a run after an authored
//! pipeline. The job's single op reads this publish's rows from
//! `gold.<table>` (by the publish's one `_exported_at` stamp, D9) and
//! replaces `gold.<table>_latest` with them, as one catalog commit
//! (`PyIceberg`'s `Transaction.overwrite` stages the delete and the
//! append and sends them together — M1: the Rust Iceberg library offers
//! no replace at all). Reading from the published
//! table, not from `ClickHouse`, keeps the copy exactly the rows the
//! publish wrote, masking included. The second table is written by the
//! orchestrator through `PyIceberg`, never by the API: the Rust Iceberg
//! library cannot write "delete everything, add this copy" as one
//! commit, while `PyIceberg`'s `Transaction.overwrite` can — measured by
//! the acceptance gate on the real stack (`DATA-6` T6, AC2: one metadata-log
//! entry for the replace, no mixed samples).
//!
//! The run config carries the publish's own stamp and row count, so the
//! op can refuse to move the table backwards (D9: a run that executes
//! out of order is `superseded`) and can prove it read the whole copy
//! (`copy_incomplete` on a row-count mismatch, 0 rows included).
//!
//! Each run — the op's outcomes and the API's own "could not launch" —
//! writes exactly one row to `console.gold_latest_copy_run` (D12), the
//! same pattern as `console.gold_trim_run`. No Postgres migration: the
//! DDL lives here and in `gold_latest_copy.py`, identical text.
//!
//! A failed latest-copy update NEVER changes the main publish's answer
//! (D13): a refused or unreachable launch is logged, recorded as a
//! `failed`/`not_started` row, and the publish still answers success
//! with `latestCopy.status = "not_started"` — the publish happened, and
//! saying otherwise would be false. The upstream detail (Dagster's own
//! refusal text) goes to the log only, never into the row or the
//! response: the row's `reason` is the fixed `"not_started"`.

use lakehouse_clickhouse::{ChClient, ChError, ChTableEnsured};
use lakehouse_core::ident::SqlLiteral;
use lakehouse_dagster::DgClient;
use lakehouse_iceberg::gold;
use serde::Serialize;
use serde_json::{Map, Value, json};

use crate::routes::support::nullable_u64_col;

/// The Dagster job that replaces one mart's latest-copy table. One op,
/// no schedule, no sensor: only `routes::gold::export` launches it, per
/// publish (D6). Also mirrored as the constant in
/// `dagster/dispar_orchestrate/gold_latest_copy.py`.
pub const GOLD_LATEST_COPY_JOB: &str = "gold_latest_copy_job";

/// The op's name inside [`GOLD_LATEST_COPY_JOB`]; the run config below
/// addresses exactly this op.
const OP_NAME: &str = "refresh_gold_latest_copy";

/// What one publish asks the orchestrator to copy (D9): the mart as
/// given to the route, the publish's own sanitized table name, the
/// publish's ONE `_exported_at` stamp in microseconds, and the row
/// count the op must read back exactly.
pub struct LatestCopyRequest<'a> {
    /// The mart name as the route received it.
    pub mart: &'a str,
    /// The published table's sanitized name (`GoldExportResult.table`).
    pub table: &'a str,
    /// The publish's one `_exported_at` value, epoch microseconds (M2).
    pub copy_exported_at_micros: i64,
    /// How many rows the publish wrote (`GoldExportResult.rows_exported`).
    pub rows: u64,
}

/// The run config the API sends for [`GOLD_LATEST_COPY_JOB`] — the exact
/// five keys the op's `config_schema` declares (section 5 of the plan).
/// `latest_table` is derived here, once, so the op and the API cannot
/// disagree about the suffix.
#[must_use]
pub fn run_config(req: &LatestCopyRequest<'_>) -> Value {
    json!({"ops": {OP_NAME: {"config": {
        "mart": req.mart,
        "table": req.table,
        "latest_table": gold::latest_table_name(req.table),
        "copy_exported_at_micros": req.copy_exported_at_micros,
        "rows": req.rows,
    }}}})
}

/// Cached per-URL "has `console.gold_latest_copy_run` been created" answer
/// (`lakehouse_clickhouse::ChTableEnsured`) — a failed attempt is not
/// cached, so a transient `ClickHouse` outage doesn't permanently wedge
/// every later call.
static LATEST_COPY_TABLE_ENSURED: ChTableEnsured = ChTableEnsured::new();

/// Idempotently create `console.gold_latest_copy_run` (D12). The DDL is
/// identical to the Python writer's
/// (`gold_latest_copy.py::ensure_gold_latest_copy_run_table`) — two
/// writers, one table shape.
///
/// # Errors
///
/// Returns [`ChError`] if the DDL statement fails.
pub async fn ensure_gold_latest_copy_run_table(ch: &ChClient) -> Result<(), ChError> {
    LATEST_COPY_TABLE_ENSURED
        .ensure(ch, || ensure_gold_latest_copy_run_table_uncached(ch))
        .await
}

async fn ensure_gold_latest_copy_run_table_uncached(ch: &ChClient) -> Result<(), ChError> {
    ch.exec("CREATE DATABASE IF NOT EXISTS console", None)
        .await?;
    ch.exec(
        "CREATE TABLE IF NOT EXISTS console.gold_latest_copy_run (\n\
            id String,\n\
            mart String,\n\
            latest_table String,\n\
            status String,\n\
            reason Nullable(String),\n\
            copy_exported_at Nullable(DateTime64(6)),\n\
            rows Nullable(UInt64),\n\
            snapshot_id Nullable(Int64),\n\
            cleanup Nullable(String),\n\
            started_at DateTime64(3),\n\
            finished_at DateTime64(3)\n\
          ) ENGINE = MergeTree ORDER BY (mart, started_at)",
        None,
    )
    .await?;
    Ok(())
}

/// Launch [`GOLD_LATEST_COPY_JOB`] for one publish (D6, D13). Best
/// effort, by design: the publish has already succeeded by the time
/// this runs, so the answer is only ever `"queued"` (the orchestrator
/// took the run) or `"not_started"` (it did not — refused or
/// unreachable, recorded as a `failed`/`not_started` row, and the
/// publish's own answer stays a success).
///
/// The upstream detail never travels: `DgClient`'s typed
/// [`lakehouse_dagster::LaunchOutcome`] is reduced to its fixed
/// `failure` label, and a transport error to a fixed sentence, both
/// logged with the mart and the job only. A failed record insert is
/// logged and swallowed — the row is a history view, not a
/// correctness dependency of the publish.
pub async fn launch(
    dagster: &DgClient,
    ch: &ChClient,
    req: &LatestCopyRequest<'_>,
) -> &'static str {
    match dagster
        .launch_run_with_config(GOLD_LATEST_COPY_JOB, &run_config(req))
        .await
    {
        Ok(outcome) if outcome.failure.is_none() => "queued",
        Ok(outcome) => {
            tracing::warn!(
                mart = req.mart,
                job = GOLD_LATEST_COPY_JOB,
                failure = outcome
                    .failure
                    .map_or("refused", lakehouse_dagster::LaunchFailure::as_str),
                "DATA-6: the orchestrator refused the latest-copy job launch"
            );
            record_not_started(ch, req).await;
            "not_started"
        }
        Err(_) => {
            tracing::warn!(
                mart = req.mart,
                job = GOLD_LATEST_COPY_JOB,
                "DATA-6: the orchestrator could not be reached for the latest-copy job launch"
            );
            record_not_started(ch, req).await;
            "not_started"
        }
    }
}

/// Insert the one `failed`/`not_started` row the record table owes for a
/// launch that did not happen (D12, D13). `copy_exported_at` and `rows`
/// are filled — they are facts of the publish that asked, not of a run —
/// and the times are this record's own, since no run started.
async fn record_not_started(ch: &ChClient, req: &LatestCopyRequest<'_>) {
    if let Err(err) = ensure_gold_latest_copy_run_table(ch).await {
        tracing::error!(%err, mart = req.mart, "DATA-6: could not ensure the latest-copy run table");
        return;
    }
    let now_ms = time::OffsetDateTime::now_utc().unix_timestamp() * 1_000;
    let sql = format!(
        "INSERT INTO console.gold_latest_copy_run \
         (id, mart, latest_table, status, reason, copy_exported_at, rows, \
          snapshot_id, cleanup, started_at, finished_at) VALUES \
         ({id}, {mart}, {latest_table}, 'failed', 'not_started', \
          fromUnixTimestamp64Micro({copy_micros}), {rows}, NULL, NULL, \
          fromUnixTimestamp64Milli({now_ms}), fromUnixTimestamp64Milli({now_ms}))",
        id = SqlLiteral::from(uuid::Uuid::new_v4().to_string()),
        mart = SqlLiteral::from(req.mart),
        latest_table = SqlLiteral::from(gold::latest_table_name(req.table)),
        copy_micros = req.copy_exported_at_micros,
        rows = req.rows,
    );
    if let Err(err) = ch.exec(&sql, None).await {
        // Same posture as `record_export_history`: a failed history write
        // never fails the publish it describes.
        tracing::error!(%err, mart = req.mart, "DATA-6: could not record the not-started latest-copy row");
    }
}

/// What the latest-copy table currently holds, read from the newest
/// `success` row (D12): the copy's stamp, its row count, and when the
/// replace finished. `None` fields mean "not recorded", never a guessed
/// number.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LatestCopyHolds {
    /// The copy's `_exported_at`, RFC 3339 with microseconds — two
    /// publishes can fall in one second, so the sub-second part is kept.
    pub copy_exported_at: Option<String>,
    /// The copy's row count.
    pub rows: Option<u64>,
    /// When the replace finished, RFC 3339.
    pub replaced_at: Option<String>,
}

/// The newest run of any status (D12): `status` from the closed set
/// `success`/`failed`/`skipped`/`superseded`, `reason`/`cleanup` from
/// their own closed sets — never exception text (principle 4).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LatestCopyRun {
    /// The run's outcome.
    pub status: String,
    /// The fixed failure/skip reason, or `None` on success.
    pub reason: Option<String>,
    /// The cleanup outcome of a success run (`DATA-6` D11), or `None`.
    pub cleanup: Option<String>,
    /// When the run finished, RFC 3339.
    pub finished_at: Option<String>,
}

/// Renders a copy stamp's epoch microseconds as an RFC 3339 UTC string,
/// keeping the microseconds — [`gold_export_history::millis_to_rfc3339`]
/// deliberately drops the sub-second part and two publishes can fall in
/// one second, so the copy stamp cannot use it.
#[must_use]
pub fn micros_to_rfc3339(us: i64) -> Option<String> {
    time::OffsetDateTime::from_unix_timestamp_nanos(i128::from(us) * 1_000)
        .ok()
        .and_then(|dt| {
            dt.format(&time::format_description::well_known::Rfc3339)
                .ok()
        })
}

fn latest_holds_sql(mart: &str) -> String {
    format!(
        "SELECT toString(toUnixTimestamp64Micro(copy_exported_at)) AS copy_micros, \
                rows, \
                toString(toUnixTimestamp64Milli(finished_at)) AS finished_ms \
         FROM console.gold_latest_copy_run \
         WHERE mart = {mart} AND status = 'success' \
         ORDER BY started_at DESC LIMIT 1 FORMAT JSON",
        mart = SqlLiteral::from(mart),
    )
}

fn latest_run_sql(mart: &str) -> String {
    format!(
        "SELECT status, reason, cleanup, \
                toString(toUnixTimestamp64Milli(finished_at)) AS finished_ms \
         FROM console.gold_latest_copy_run \
         WHERE mart = {mart} \
         ORDER BY started_at DESC LIMIT 1 FORMAT JSON",
        mart = SqlLiteral::from(mart),
    )
}

/// Parse one `DateTime64(3)`-as-milliseconds cell back to RFC 3339, via
/// the microsecond renderer (a millisecond value is just a microsecond
/// value with a zero tail, and `Rfc3339` prints only the significant
/// digits).
fn millis_cell_to_rfc3339(row: &Map<String, Value>, key: &str) -> Option<String> {
    row.get(key)
        .and_then(Value::as_str)
        .and_then(|ms| ms.parse::<i64>().ok())
        .and_then(|ms| micros_to_rfc3339(ms.saturating_mul(1_000)))
}

fn row_to_holds(row: &Map<String, Value>) -> LatestCopyHolds {
    LatestCopyHolds {
        copy_exported_at: row
            .get("copy_micros")
            .and_then(Value::as_str)
            .and_then(|us| us.parse::<i64>().ok())
            .and_then(micros_to_rfc3339),
        rows: nullable_u64_col(row, "rows"),
        replaced_at: millis_cell_to_rfc3339(row, "finished_ms"),
    }
}

fn row_to_run(row: &Map<String, Value>) -> LatestCopyRun {
    LatestCopyRun {
        status: row
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned(),
        reason: row
            .get("reason")
            .filter(|v| !v.is_null())
            .and_then(Value::as_str)
            .map(str::to_owned),
        cleanup: row
            .get("cleanup")
            .filter(|v| !v.is_null())
            .and_then(Value::as_str)
            .map(str::to_owned),
        finished_at: millis_cell_to_rfc3339(row, "finished_ms"),
    }
}

/// The latest-copy state one mart's publication detail shows (D4): what
/// the table holds, from the newest `success` row, and the newest run of
/// any status. Each is `None` when no row exists.
///
/// # Errors
///
/// Returns [`ChError`] if the record table cannot be ensured or either
/// query fails. The caller classifies it before surfacing
/// (`routes::gold::publication_body`), per principle 4.
pub async fn latest_copy_state(
    ch: &ChClient,
    mart: &str,
) -> Result<(Option<LatestCopyHolds>, Option<LatestCopyRun>), ChError> {
    ensure_gold_latest_copy_run_table(ch).await?;
    let holds = ch.rows(&latest_holds_sql(mart), None).await?;
    let holds = holds.first().map(row_to_holds);
    let runs = ch.rows(&latest_run_sql(mart), None).await?;
    let last = runs.first().map(row_to_run);
    Ok((holds, last))
}

// The `snapshot_id` column has no reader on this side yet (the op
// records it, the console does not show it); its reader shape is pinned
// by a test below so the DDL and its reader set cannot drift silently.
#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use crate::routes::support::nullable_i64_col;

    fn request(table: &str) -> LatestCopyRequest<'_> {
        LatestCopyRequest {
            mart: "gold_export_smoke",
            table,
            copy_exported_at_micros: 1_795_168_800_123_456,
            rows: 123,
        }
    }

    #[test]
    fn run_config_holds_exactly_the_five_op_config_keys() {
        let config = run_config(&request("sales"));
        let op = &config["ops"][OP_NAME]["config"];
        assert_eq!(config["ops"].as_object().unwrap().len(), 1);
        let mut keys: Vec<&str> = op.as_object().unwrap().keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "copy_exported_at_micros",
                "latest_table",
                "mart",
                "rows",
                "table"
            ]
        );
        assert_eq!(op["latest_table"], "sales_latest");
        assert_eq!(op["mart"], "gold_export_smoke");
        assert_eq!(
            op["copy_exported_at_micros"],
            json!(1_795_168_800_123_456_i64)
        );
        assert_eq!(op["rows"], json!(123));
    }

    #[test]
    fn micros_to_rfc3339_keeps_the_microseconds() {
        // 2026-11-20T10:00:00.123456Z — the plan's own example stamp.
        let text = micros_to_rfc3339(1_795_168_800_123_456).unwrap();
        assert!(text.ends_with(".123456Z"), "{text}");
        assert_eq!(text, "2026-11-20T10:00:00.123456Z");
        assert!(micros_to_rfc3339(i64::MAX).is_none());
    }

    #[test]
    fn both_selects_read_the_record_table_newest_first_and_bind_the_mart() {
        for sql in [latest_holds_sql("orders"), latest_run_sql("orders")] {
            assert!(sql.contains("console.gold_latest_copy_run"), "{sql}");
            assert!(sql.contains("ORDER BY started_at DESC LIMIT 1"), "{sql}");
            assert!(sql.contains("'orders'"), "{sql}");
        }
        // A mart holding a quote is escaped by `SqlLiteral`, so it can
        // never break out of its string literal — belt and braces: the
        // route's `Ident::new` refuses a quote long before this.
        let hostile = latest_holds_sql("mart'o");
        assert!(hostile.contains("'mart''o'"), "{hostile}");
        assert!(!hostile.contains("'mart'o'"), "{hostile}");
    }

    #[test]
    fn row_readers_take_numbers_or_quoted_strings_and_keep_nulls() {
        let mut row = Map::new();
        row.insert("copy_micros".to_owned(), json!("1795168800123456"));
        row.insert("rows".to_owned(), json!(42));
        row.insert("finished_ms".to_owned(), json!("1795168800123"));
        let holds = row_to_holds(&row);
        assert_eq!(holds.rows, Some(42));
        assert_eq!(
            holds.copy_exported_at.as_deref(),
            Some("2026-11-20T10:00:00.123456Z")
        );
        assert!(holds.replaced_at.is_some());

        let mut run_row = Map::new();
        run_row.insert("status".to_owned(), json!("success"));
        run_row.insert("reason".to_owned(), Value::Null);
        run_row.insert("cleanup".to_owned(), Value::Null);
        let run = row_to_run(&run_row);
        assert_eq!(run.status, "success");
        assert_eq!(run.reason, None);
        assert_eq!(run.cleanup, None);
        assert_eq!(run.finished_at, None);
    }

    // `nullable_i64_col` reads the `snapshot_id` column's shape (a quoted
    // or bare `Int64`), proved here so the DDL's reader set stays honest.
    #[test]
    fn nullable_i64_reads_the_snapshot_id_shape() {
        let mut row = Map::new();
        row.insert("snapshot_id".to_owned(), json!("4920915523900734040"));
        assert_eq!(
            nullable_i64_col(&row, "snapshot_id"),
            Some(4_920_915_523_900_734_040)
        );
    }
}
