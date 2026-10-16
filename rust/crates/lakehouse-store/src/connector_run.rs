//! The product's own record of a connector run, of each attempt at each
//! table, and of the run's log (`SRC-11` decisions D5, D6, D7, D8; tables
//! `connector_run`, `connector_run_attempt`, `connector_run_log`, migration
//! `0072_connector_run_record.sql` -- read its header for the shape of each).
//!
//! # Who writes
//!
//! The API, from reports the orchestrator posts on its `ingest:read` service
//! path (`routes::connector_runs`). The orchestrator sends structured facts
//! and the fixed class words of [`ErrorClass`]; the log lines are rendered
//! HERE from them. So the log can only hold the product's own words, a
//! table name the connector's spec spells, and numbers: never a driver's, a
//! source's or the orchestrator's error text (`AGENTS.md` principle 4). The
//! migration's CHECK constraints repeat the closed lists, so a caller that
//! skipped the enums still cannot store free text in a class column.
//!
//! # Idempotence
//!
//! The orchestrator may post a report twice (a timeout it did not see the
//! answer to). A run start is a no-op the second time, an attempt is stored
//! once per `(run, table, attempt)` (the first report wins and writes the
//! log lines; the repeat writes nothing), and the end of a run is recorded
//! once. Each function says which.
//!
//! # Retention (D6)
//!
//! [`purge_older_than`] removes log lines and attempts only. The run rows
//! stay, with their totals, so the list of the last runs stays exact.

use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use time::OffsetDateTime;

use crate::schema_change::IsoTime;
use crate::{PgPool, StoreError};

/// How long a run's log and attempts are kept (`SRC-11` decision D5).
pub const LOG_RETENTION_DAYS: i64 = 30;

/// The most runs the history lists (`SRC-11` decision D7).
pub const RUN_HISTORY_LIMIT: i64 = 50;

/// What started a run (`SRC-11` decision D7). `other` is a run started from
/// the orchestrator's own screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunTrigger {
    /// The connector's schedule.
    Schedule,
    /// A person pressed "Run now".
    User,
    /// A person asked for a refresh of one table.
    Refresh,
    /// A person asked to clear one table.
    Clear,
    /// Anything else, such as a run launched in the orchestrator.
    Other,
}

impl RunTrigger {
    /// The value stored in `connector_run.trigger_kind` and sent on the wire.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Schedule => "schedule",
            Self::User => "user",
            Self::Refresh => "refresh",
            Self::Clear => "clear",
            Self::Other => "other",
        }
    }

    const fn phrase(self) -> &'static str {
        match self {
            Self::Schedule => "the schedule",
            Self::User => "a user",
            Self::Refresh => "a table refresh",
            Self::Clear => "a table clear",
            Self::Other => "the orchestrator",
        }
    }
}

/// How a run ended. `Running` is the state before the end is reported; it is
/// never a valid end.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    /// No end reported yet.
    Running,
    /// Every table loaded.
    Succeeded,
    /// A table failed after its attempts.
    Failed,
    /// The connector was paused (`SRC-11` decision D13): nothing was loaded.
    Paused,
}

impl RunStatus {
    /// The value stored in `connector_run.status`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Paused => "paused",
        }
    }
}

/// Why a run ended, as a fixed word (`SRC-11` decision D5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EndReason {
    /// Every table was processed.
    Completed,
    /// A table failed and its attempts were used up (or it cannot be retried).
    TableFailed,
    /// The connector is paused.
    ConnectorPaused,
    /// None of the above.
    Other,
}

impl EndReason {
    /// The value stored in `connector_run.end_reason`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::TableFailed => "table_failed",
            Self::ConnectorPaused => "connector_paused",
            Self::Other => "other",
        }
    }

    const fn phrase(self) -> &'static str {
        match self {
            Self::Completed => "all tables were processed",
            Self::TableFailed => "a table failed",
            Self::ConnectorPaused => "the connector is paused, nothing was loaded",
            Self::Other => "no further reason is recorded",
        }
    }
}

/// The kind of a failed attempt: the fixed words of `SRC-11` decision D2,
/// the only description of a failure the log ever holds. The first four are
/// retried, the rest are not (`other` is an unclassified failure and is not
/// retried: an unknown failure is not guessed to be transient).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorClass {
    /// The source or the network cannot be reached.
    Unreachable,
    /// The source did not answer in time.
    Timeout,
    /// The source answered "busy" or with a server error.
    SourceBusy,
    /// The product's own API could not be reached.
    ApiUnreachable,
    /// The source refused the sign-in.
    SignInRefused,
    /// The address is refused by the product's address check.
    AddressRefused,
    /// A source type or load mode the product does not support.
    Unsupported,
    /// A setting the product rejects.
    SettingRejected,
    /// The table waits for a schema decision.
    SchemaDecisionPending,
    /// An unclassified failure.
    Other,
}

impl ErrorClass {
    /// The value stored in `connector_run_attempt.error_class`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unreachable => "unreachable",
            Self::Timeout => "timeout",
            Self::SourceBusy => "source_busy",
            Self::ApiUnreachable => "api_unreachable",
            Self::SignInRefused => "sign_in_refused",
            Self::AddressRefused => "address_refused",
            Self::Unsupported => "unsupported",
            Self::SettingRejected => "setting_rejected",
            Self::SchemaDecisionPending => "schema_decision_pending",
            Self::Other => "other",
        }
    }

    /// Whether the class is one the run retries (`SRC-11` decision D2).
    #[must_use]
    pub const fn is_transient(self) -> bool {
        matches!(
            self,
            Self::Unreachable | Self::Timeout | Self::SourceBusy | Self::ApiUnreachable
        )
    }

    const fn phrase(self) -> &'static str {
        match self {
            Self::Unreachable => "the source could not be reached",
            Self::Timeout => "the source did not answer in time",
            Self::SourceBusy => "the source was busy or returned a server error",
            Self::ApiUnreachable => "the product's API could not be reached",
            Self::SignInRefused => "the source refused the sign-in",
            Self::AddressRefused => "the address is refused",
            Self::Unsupported => "this source type or load mode is not supported",
            Self::SettingRejected => "a setting was rejected",
            Self::SchemaDecisionPending => "the table waits for a schema decision",
            Self::Other => "the failure is not classified",
        }
    }
}

/// A line the orchestrator may add to a run's log besides attempts
/// (`POST .../log`): a closed list, each rendered here from a table name and
/// a count.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogEvent {
    /// A table was held back for a schema decision (needs a table).
    SchemaDecisionPending,
    /// Columns were left out of a table's load (needs a table and a count).
    ColumnsLeftOut,
    /// A report of this run could not be sent, so the record has a gap
    /// (`SRC-11` task 6: the record shows the gap honestly).
    ReportGap,
}

impl LogEvent {
    const fn as_str(self) -> &'static str {
        match self {
            Self::SchemaDecisionPending => "schema_decision_pending",
            Self::ColumnsLeftOut => "columns_left_out",
            Self::ReportGap => "report_gap",
        }
    }
}

/// A table name as it goes into a log line: control characters dropped, so a
/// name can never break a line of the text download in two.
fn clean(text: &str) -> String {
    text.chars().filter(|c| !c.is_control()).collect()
}

/// A wait in the words of the log: whole minutes from one minute up,
/// seconds below.
fn wait_phrase(seconds: i32) -> String {
    if seconds >= 60 && seconds % 60 == 0 {
        format!("{} min", seconds / 60)
    } else {
        format!("{seconds} s")
    }
}

async fn insert_line(
    conn: &mut sqlx::PgConnection,
    run_id: &str,
    at: OffsetDateTime,
    event: &str,
    table: Option<&str>,
    attempt: Option<i32>,
    line: &str,
) -> Result<(), StoreError> {
    sqlx::query(
        "INSERT INTO connector_run_log (run_id, at, event, table_name, attempt, line) \
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(run_id)
    .bind(at)
    .bind(event)
    .bind(table)
    .bind(attempt)
    .bind(line)
    .execute(conn)
    .await?;
    Ok(())
}

/// Lock the run row and check it belongs to `connector_id`. A run of another
/// connector is `NotFound`, the answer an unknown run gets.
async fn lock_run(
    conn: &mut sqlx::PgConnection,
    connector_id: &str,
    run_id: &str,
) -> Result<(), StoreError> {
    let owner: Option<(String,)> =
        sqlx::query_as("SELECT connector_id FROM connector_run WHERE run_id = $1 FOR UPDATE")
            .bind(run_id)
            .fetch_optional(conn)
            .await?;
    match owner {
        Some((owner,)) if owner == connector_id => Ok(()),
        _ => Err(StoreError::NotFound),
    }
}

/// Record that a run started (`SRC-11` decision D7) and write its first log
/// line. A second report of the same run changes nothing and returns
/// `false`.
///
/// `table` is the one source object a `refresh` or `clear` run is about
/// (`SRC-11` review 1 BLOCKER 2: the run list names it, `SRC-10` decision 5);
/// `None` for every other run. A table on any other trigger is
/// [`StoreError::Validation`].
///
/// # Errors
///
/// [`StoreError::Validation`] for a table on a run that is not a refresh or a
/// clear, [`StoreError::Conflict`] when `run_id` already belongs to another
/// connector, [`StoreError::ForeignKeyViolation`] when `connector_id` names
/// no connector, [`StoreError::Database`] otherwise.
pub async fn start_run(
    pool: &PgPool,
    connector_id: &str,
    run_id: &str,
    trigger: RunTrigger,
    table: Option<&str>,
    at: OffsetDateTime,
) -> Result<bool, StoreError> {
    let table = table.map(clean);
    if table.is_some() && !matches!(trigger, RunTrigger::Refresh | RunTrigger::Clear) {
        return Err(StoreError::Validation(
            "only a refresh or a clear names a table".to_owned(),
        ));
    }
    let mut tx = pool.begin().await?;
    let inserted: Option<(String,)> = sqlx::query_as(
        "INSERT INTO connector_run (run_id, connector_id, trigger_kind, table_name, started_at) \
         VALUES ($1, $2, $3, $4, $5) ON CONFLICT (run_id) DO NOTHING RETURNING run_id",
    )
    .bind(run_id)
    .bind(connector_id)
    .bind(trigger.as_str())
    .bind(table.as_deref())
    .bind(at)
    .fetch_optional(&mut *tx)
    .await?;
    if inserted.is_none() {
        let owner: Option<(String,)> =
            sqlx::query_as("SELECT connector_id FROM connector_run WHERE run_id = $1")
                .bind(run_id)
                .fetch_optional(&mut *tx)
                .await?;
        return match owner {
            Some((owner,)) if owner == connector_id => Ok(false),
            _ => Err(StoreError::Conflict),
        };
    }
    insert_line(
        &mut tx,
        run_id,
        at,
        "run_started",
        None,
        None,
        &format!("run started by {}", trigger.phrase()),
    )
    .await?;
    tx.commit().await?;
    Ok(true)
}

/// One attempt at one table, as the orchestrator reports it when the attempt
/// ends.
#[derive(Debug, Clone)]
pub struct AttemptReport<'a> {
    /// The source object's name.
    pub table: &'a str,
    /// 1 for the first attempt.
    pub attempt: i32,
    /// When the attempt started.
    pub started_at: OffsetDateTime,
    /// When it ended.
    pub ended_at: OffsetDateTime,
    /// Rows it loaded (0 for a failure).
    pub rows: i64,
    /// `None` for a success, the failure's class otherwise.
    pub error_class: Option<ErrorClass>,
    /// Seconds the orchestrator waits before the next attempt; only for a
    /// failure that will be retried.
    pub wait_seconds: Option<i32>,
}

/// Record one attempt, write its log lines (the attempt, and the wait before
/// the next one) and bring the run's totals up to date (`attempts`, and
/// `rows_loaded` as the rows of the attempts that succeeded). Returns
/// `false` when this `(run, table, attempt)` was already recorded: the first
/// report wins and a repeat writes nothing.
///
/// # Errors
///
/// [`StoreError::NotFound`] when the run is unknown or belongs to another
/// connector, [`StoreError::Validation`] when the report contradicts itself
/// (a wait on a success, a negative number, an attempt below 1),
/// [`StoreError::Database`] otherwise. A failed attempt's rows are stored as
/// sent and never counted in the run's total.
pub async fn record_attempt(
    pool: &PgPool,
    connector_id: &str,
    run_id: &str,
    report: &AttemptReport<'_>,
) -> Result<bool, StoreError> {
    if report.attempt < 1
        || report.rows < 0
        || report.wait_seconds.is_some_and(|w| w < 0)
        || (report.error_class.is_none() && report.wait_seconds.is_some())
    {
        return Err(StoreError::Validation(
            "the attempt report contradicts itself".to_owned(),
        ));
    }
    let mut tx = pool.begin().await?;
    lock_run(&mut tx, connector_id, run_id).await?;
    let table = clean(report.table);
    let result = if report.error_class.is_none() {
        "succeeded"
    } else {
        "failed"
    };
    let inserted = sqlx::query(
        "INSERT INTO connector_run_attempt \
           (run_id, table_name, attempt, started_at, ended_at, rows_loaded, result, \
            error_class, wait_seconds) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) \
         ON CONFLICT (run_id, table_name, attempt) DO NOTHING",
    )
    .bind(run_id)
    .bind(&table)
    .bind(report.attempt)
    .bind(report.started_at)
    .bind(report.ended_at)
    .bind(report.rows)
    .bind(result)
    .bind(report.error_class.map(ErrorClass::as_str))
    .bind(report.wait_seconds)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if inserted == 0 {
        tx.commit().await?;
        return Ok(false);
    }
    let seconds = (report.ended_at - report.started_at).whole_seconds().max(0);
    let line = match report.error_class {
        None => format!(
            "table {table}, attempt {}: loaded {} rows in {seconds} s",
            report.attempt, report.rows
        ),
        Some(class) => format!(
            "table {table}, attempt {}: failed, {}",
            report.attempt,
            class.phrase()
        ),
    };
    insert_line(
        &mut tx,
        run_id,
        report.ended_at,
        "attempt",
        Some(&table),
        Some(report.attempt),
        &line,
    )
    .await?;
    if let Some(wait) = report.wait_seconds {
        insert_line(
            &mut tx,
            run_id,
            report.ended_at,
            "retry_wait",
            Some(&table),
            Some(report.attempt),
            &format!("table {table}: next attempt in {}", wait_phrase(wait)),
        )
        .await?;
    }
    sqlx::query(
        "UPDATE connector_run SET \
           attempts = (SELECT count(*)::int FROM connector_run_attempt WHERE run_id = $1), \
           rows_loaded = (SELECT COALESCE(SUM(rows_loaded), 0)::bigint \
                          FROM connector_run_attempt \
                          WHERE run_id = $1 AND result = 'succeeded') \
         WHERE run_id = $1",
    )
    .bind(run_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(true)
}

/// Add one of the closed [`LogEvent`] lines to a run's log.
///
/// # Errors
///
/// [`StoreError::NotFound`] when the run is unknown or belongs to another
/// connector, [`StoreError::Validation`] when the event needs a table or a
/// count and has none, [`StoreError::Database`] otherwise.
pub async fn add_log_line(
    pool: &PgPool,
    connector_id: &str,
    run_id: &str,
    event: LogEvent,
    table: Option<&str>,
    count: Option<i32>,
    at: OffsetDateTime,
) -> Result<(), StoreError> {
    let table = table.map(clean);
    let line = match (event, table.as_deref(), count) {
        (LogEvent::SchemaDecisionPending, Some(t), _) => {
            format!("table {t}: held back, it waits for a schema decision")
        }
        (LogEvent::ColumnsLeftOut, Some(t), Some(n)) if n >= 0 => {
            format!("table {t}: {n} columns left out of this load")
        }
        (LogEvent::ReportGap, _, _) => {
            "a report of this run could not be sent; this log may be incomplete".to_owned()
        }
        _ => {
            return Err(StoreError::Validation(
                "the log line needs a table and, for left-out columns, a count".to_owned(),
            ));
        }
    };
    let mut tx = pool.begin().await?;
    lock_run(&mut tx, connector_id, run_id).await?;
    insert_line(
        &mut tx,
        run_id,
        at,
        event.as_str(),
        table.as_deref(),
        None,
        &line,
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

/// Record how a run ended (`SRC-11` decisions D5, D7, D13) and write the
/// last log line. Recorded once: a run that already has an end keeps it and
/// the call returns `false`.
///
/// # Errors
///
/// [`StoreError::NotFound`] when the run is unknown or belongs to another
/// connector, [`StoreError::Validation`] for [`RunStatus::Running`] (that is
/// not an end), [`StoreError::Database`] otherwise.
pub async fn end_run(
    pool: &PgPool,
    connector_id: &str,
    run_id: &str,
    status: RunStatus,
    reason: EndReason,
    at: OffsetDateTime,
) -> Result<bool, StoreError> {
    if status == RunStatus::Running {
        return Err(StoreError::Validation(
            "a run cannot end as running".to_owned(),
        ));
    }
    let mut tx = pool.begin().await?;
    lock_run(&mut tx, connector_id, run_id).await?;
    let ended = sqlx::query(
        "UPDATE connector_run SET status = $2, ended_at = $3, end_reason = $4 \
         WHERE run_id = $1 AND status = 'running'",
    )
    .bind(run_id)
    .bind(status.as_str())
    .bind(at)
    .bind(reason.as_str())
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if ended == 0 {
        tx.commit().await?;
        return Ok(false);
    }
    insert_line(
        &mut tx,
        run_id,
        at,
        "run_ended",
        None,
        None,
        &format!("run ended: {}, {}", status.as_str(), reason.phrase()),
    )
    .await?;
    tx.commit().await?;
    Ok(true)
}

/// Settle a run that is still `running` from the orchestrator's own final
/// status, as the run sensor confirmed it: a run that was killed, or an
/// orchestrator that could not send its end report, would otherwise stay
/// `running` for good. A run with no record (an orchestrator that does not
/// report yet) and a run that already ended are left alone: nothing is
/// invented for them.
///
/// # Errors
///
/// [`StoreError::Database`] on a failed statement.
pub async fn settle_run(
    pool: &PgPool,
    connector_id: &str,
    run_id: &str,
    succeeded: bool,
    at: OffsetDateTime,
) -> Result<(), StoreError> {
    let (status, reason) = if succeeded {
        (RunStatus::Succeeded, EndReason::Completed)
    } else {
        (RunStatus::Failed, EndReason::TableFailed)
    };
    match end_run(pool, connector_id, run_id, status, reason, at).await {
        Ok(_) | Err(StoreError::NotFound) => Ok(()),
        Err(err) => Err(err),
    }
}

/// The recorded status of a run of this connector, `None` when there is no
/// record. The run sensor reads it to leave a `paused` run out of the
/// connector's failure streak (`SRC-11` decision D13).
///
/// # Errors
///
/// [`StoreError::Database`] on a failed statement.
pub async fn run_status(
    pool: &PgPool,
    connector_id: &str,
    run_id: &str,
) -> Result<Option<String>, StoreError> {
    Ok(sqlx::query_scalar::<_, String>(
        "SELECT status FROM connector_run WHERE run_id = $1 AND connector_id = $2",
    )
    .bind(run_id)
    .bind(connector_id)
    .fetch_optional(pool)
    .await?)
}

#[derive(FromRow)]
struct RunRow {
    run_id: String,
    trigger_kind: String,
    table_name: Option<String>,
    status: String,
    started_at: OffsetDateTime,
    ended_at: Option<OffsetDateTime>,
    rows_loaded: i64,
    attempts: i32,
    end_reason: Option<String>,
}

const RUN_COLUMNS: &str = "run_id, trigger_kind, table_name, status, started_at, ended_at, rows_loaded, attempts, end_reason";

/// One run in the history (`SRC-11` decision D7). Mirrors the console's
/// `ConnectorRun`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunSummary {
    /// The orchestrator's run id.
    pub run_id: String,
    /// `schedule`, `user`, `refresh`, `clear` or `other`.
    pub trigger: String,
    /// The one table a `refresh` or `clear` run is about; `null` for any other
    /// run and for a run recorded before it was kept (`SRC-11` review 1
    /// BLOCKER 2).
    pub table: Option<String>,
    /// `running`, `succeeded`, `failed` or `paused`.
    pub status: String,
    /// When the run started.
    pub started_at: IsoTime,
    /// When it ended; `null` while it runs.
    pub ended_at: Option<IsoTime>,
    /// Whole seconds from start to end; `null` while it runs.
    pub duration_seconds: Option<i64>,
    /// Rows loaded by the attempts that succeeded.
    pub rows_loaded: i64,
    /// Attempts made, over all tables.
    pub attempts: i32,
    /// Why the run ended, one of the fixed words; `null` while it runs.
    pub end_reason: Option<String>,
}

impl From<RunRow> for RunSummary {
    fn from(row: RunRow) -> Self {
        Self {
            run_id: row.run_id,
            trigger: row.trigger_kind,
            table: row.table_name,
            status: row.status,
            duration_seconds: row
                .ended_at
                .map(|end| (end - row.started_at).whole_seconds()),
            started_at: row.started_at.into(),
            ended_at: row.ended_at.map(IsoTime::from),
            rows_loaded: row.rows_loaded,
            attempts: row.attempts,
            end_reason: row.end_reason,
        }
    }
}

/// The connector's newest runs, newest first, at most `limit`. Exact for any
/// number of connectors: the rows are the product's own, keyed by connector.
///
/// # Errors
///
/// [`StoreError::Database`] if the query fails.
pub async fn list_runs(
    pool: &PgPool,
    connector_id: &str,
    limit: i64,
) -> Result<Vec<RunSummary>, StoreError> {
    let sql = format!(
        "SELECT {RUN_COLUMNS} FROM connector_run WHERE connector_id = $1 \
         ORDER BY started_at DESC, run_id DESC LIMIT $2"
    );
    let rows: Vec<RunRow> = sqlx::query_as(&sql)
        .bind(connector_id)
        .bind(limit)
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().map(RunSummary::from).collect())
}

/// One attempt of a run's log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AttemptRecord {
    /// The source object's name.
    pub table: String,
    /// 1 for the first attempt.
    pub attempt: i32,
    /// When it started.
    pub started_at: IsoTime,
    /// When it ended.
    pub ended_at: IsoTime,
    /// Rows it loaded.
    pub rows_loaded: i64,
    /// `succeeded` or `failed`.
    pub result: String,
    /// One of [`ErrorClass`]'s words, only for a failure.
    pub error_class: Option<String>,
    /// Seconds waited before the next attempt, when there was one.
    pub wait_seconds: Option<i32>,
}

#[derive(FromRow)]
struct AttemptRow {
    table_name: String,
    attempt: i32,
    started_at: OffsetDateTime,
    ended_at: OffsetDateTime,
    rows_loaded: i64,
    result: String,
    error_class: Option<String>,
    wait_seconds: Option<i32>,
}

/// One line of a run's log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogLine {
    /// When it happened.
    pub at: IsoTime,
    /// `run_started`, `attempt`, `retry_wait`, `schema_decision_pending`,
    /// `columns_left_out`, `report_gap` or `run_ended`.
    pub event: String,
    /// The table, when the line is about one.
    pub table: Option<String>,
    /// The attempt number, when the line is about one.
    pub attempt: Option<i32>,
    /// The line, in the product's own words.
    pub text: String,
}

#[derive(FromRow)]
struct LineRow {
    at: OffsetDateTime,
    event: String,
    table_name: Option<String>,
    attempt: Option<i32>,
    line: String,
}

/// A run with its attempts and log lines (`GET .../runs/{runId}/log`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunLog {
    /// The run.
    pub run: RunSummary,
    /// Its attempts, by table then attempt.
    pub attempts: Vec<AttemptRecord>,
    /// Its lines, in the order they were written.
    pub lines: Vec<LogLine>,
}

impl RunLog {
    /// The log as the text file the console offers to download: a heading
    /// line, then one line per log line, the same lines the screen shows
    /// (`SRC-11` decision D5).
    #[must_use]
    pub fn to_text(&self) -> String {
        let mut out = format!(
            "run {} ({}), {}\n",
            self.run.run_id, self.run.trigger, self.run.status
        );
        for line in &self.lines {
            out.push_str(line.at.as_str());
            out.push_str("  ");
            out.push_str(&line.text);
            out.push('\n');
        }
        out
    }
}

/// A run of `connector_id` with its attempts and lines; `None` when the run
/// is unknown or belongs to another connector. Lines older than the
/// retention window are gone (see [`purge_older_than`]).
///
/// # Errors
///
/// [`StoreError::Database`] if a query fails.
pub async fn get_run_log(
    pool: &PgPool,
    connector_id: &str,
    run_id: &str,
) -> Result<Option<RunLog>, StoreError> {
    let sql =
        format!("SELECT {RUN_COLUMNS} FROM connector_run WHERE run_id = $1 AND connector_id = $2");
    let run: Option<RunRow> = sqlx::query_as(&sql)
        .bind(run_id)
        .bind(connector_id)
        .fetch_optional(pool)
        .await?;
    let Some(run) = run else {
        return Ok(None);
    };
    let attempts: Vec<AttemptRow> = sqlx::query_as(
        "SELECT table_name, attempt, started_at, ended_at, rows_loaded, result, error_class, \
                wait_seconds \
         FROM connector_run_attempt WHERE run_id = $1 ORDER BY table_name, attempt",
    )
    .bind(run_id)
    .fetch_all(pool)
    .await?;
    let lines: Vec<LineRow> = sqlx::query_as(
        "SELECT at, event, table_name, attempt, line FROM connector_run_log \
         WHERE run_id = $1 ORDER BY id",
    )
    .bind(run_id)
    .fetch_all(pool)
    .await?;
    Ok(Some(RunLog {
        run: run.into(),
        attempts: attempts
            .into_iter()
            .map(|a| AttemptRecord {
                table: a.table_name,
                attempt: a.attempt,
                started_at: a.started_at.into(),
                ended_at: a.ended_at.into(),
                rows_loaded: a.rows_loaded,
                result: a.result,
                error_class: a.error_class,
                wait_seconds: a.wait_seconds,
            })
            .collect(),
        lines: lines
            .into_iter()
            .map(|l| LogLine {
                at: l.at.into(),
                event: l.event,
                table: l.table_name,
                attempt: l.attempt,
                text: l.line,
            })
            .collect(),
    }))
}

/// What [`purge_older_than`] removed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Purged {
    /// Log lines removed.
    pub lines: u64,
    /// Attempts removed.
    pub attempts: u64,
}

/// Remove the log lines and the attempts older than `cutoff` (`SRC-11`
/// decision D6). The run rows, with their totals, and every per-table result
/// elsewhere are kept. A line is judged by its own time, an attempt by its
/// start.
///
/// # Errors
///
/// [`StoreError::Database`] if a statement fails (the first may already have
/// run; the next daily pass removes the rest).
pub async fn purge_older_than(pool: &PgPool, cutoff: OffsetDateTime) -> Result<Purged, StoreError> {
    let lines = sqlx::query("DELETE FROM connector_run_log WHERE at < $1")
        .bind(cutoff)
        .execute(pool)
        .await?
        .rows_affected();
    let attempts = sqlx::query("DELETE FROM connector_run_attempt WHERE started_at < $1")
        .bind(cutoff)
        .execute(pool)
        .await?
        .rows_affected();
    Ok(Purged { lines, attempts })
}
