//! What the lakehouse was used for, summed from `query_history`, and the
//! rates that turn the sums into an estimate in money (`OPS-8` part B,
//! migration `0145_usage_rate_and_permission.sql`).
//!
//! # What is summed
//!
//! One row of `query_history` is one run. [`report`] adds the runs of a UTC
//! calendar period, `to` included, per day, per person, per engine or per
//! tenant, and returns one more row with the totals of the whole period. A
//! value is a recorded one (`scanned_bytes`, `duration_ms`, `status`); nothing
//! is guessed. `duration_ms` is the wall-clock time of the run, so the sum is
//! engine time as the caller waited for it, not CPU time.
//!
//! # The counts
//!
//! `query_history.status` holds five values (`0127_query_history_stopped.sql`).
//! The view shows four counts: completed, failed, blocked and cancelled. A run
//! the person stopped (`stopped`) is a cancelled run, so it is counted in
//! `cancelled`. The four counts add up to `queries`.
//!
//! # Who may be summed
//!
//! The caller of [`report`] chooses the [`TenantScope`]. [`TenantScope::Tenant`]
//! sums the rows whose `tenant_id` is that tenant and nothing else, so a run of
//! another tenant, and a run with no tenant (every row written before
//! `0144_query_history_tenant.sql`, which nobody can attribute), never enters
//! a sum. [`TenantScope::Everyone`] sums all rows, the no-tenant ones among
//! them; the route allows it only for a caller holding `*:*`. This module
//! decides nothing about who the caller is.
//!
//! # A person
//!
//! A run is a person's through `owner_id`. A run with no owner (written before
//! `0046_query_ownership.sql`) is grouped under its `user_name`, which is then
//! the row's key. The row's `label` is the `user_name` of the newest run of the
//! group, so a person who changed their name shows under the current one.
//!
//! # Money
//!
//! The estimate of a run is `scanned_bytes / 10^12 x scan_tb` plus
//! `duration_ms / 3,600,000 x engine_hour`, each with the rate in force on the
//! run's UTC day: per kind, the [`UsageRate`] with the latest `effective_from`
//! on or before that day, and among rows with the same date the one created
//! last. A part with no rate in force is not priced, not priced as zero: the
//! estimate of a row is `None` when no part of any of its runs had a rate, so
//! a period before the first effective date has no estimate. The sums are
//! `NUMERIC` in the database and travel as text with six decimals, so no
//! amount of money is ever a float. The totals row is its own sum, rounded
//! once, so it can differ in the last decimal from the sum of the rounded rows.
//!
//! The rates are append-only (the table refuses an UPDATE or a DELETE), and
//! all of them share one currency ([`add_rate`]).

use sqlx::PgPool;
use time::{Date, OffsetDateTime};
use uuid::Uuid;

use crate::StoreError;

/// What a row of the report is per.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    /// A UTC day; the key is `YYYY-MM-DD`.
    Day,
    /// A person; the key is the owner's id, or the `user_name` of a run with
    /// no owner.
    Person,
    /// An engine; the key is the engine's name as recorded.
    Engine,
    /// A tenant; the key is the tenant's id, or `none` for the runs with no
    /// tenant, whose label is `no tenant`.
    Tenant,
}

impl Group {
    /// The word the SQL switches on. A constant, bound as a value.
    const fn as_str(self) -> &'static str {
        match self {
            Self::Day => "day",
            Self::Person => "person",
            Self::Engine => "engine",
            Self::Tenant => "tenant",
        }
    }
}

/// Whose runs a report sums.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TenantScope {
    /// The runs recorded under this tenant, and no other.
    Tenant(Uuid),
    /// Every run, the ones with no tenant included.
    Everyone,
}

/// What to sum: the UTC days `from` to `to`, both included.
#[derive(Debug, Clone, Copy)]
pub struct UsageQuery {
    /// The first day.
    pub from: Date,
    /// The last day, included.
    pub to: Date,
    /// What a row is per.
    pub group: Group,
    /// Whose runs.
    pub scope: TenantScope,
}

/// One row of the report, or its totals.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct UsageRow {
    /// The group key: see [`Group`]. Empty on the totals row.
    pub key: String,
    /// What to show for the key. The person's newest `user_name`, the tenant's
    /// name or `no tenant`; for a day or an engine, the key.
    pub label: String,
    /// Runs.
    pub queries: i64,
    /// Runs with status `completed`.
    pub completed: i64,
    /// Runs with status `failed`.
    pub failed: i64,
    /// Runs with status `blocked`.
    pub blocked: i64,
    /// Runs with status `cancelled` or `stopped`.
    pub cancelled: i64,
    /// Bytes the runs scanned.
    pub scanned_bytes: i64,
    /// Wall-clock milliseconds the runs took.
    pub engine_ms: i64,
    /// The estimate in money, a decimal with six places, or `None` when no
    /// rate applied. The currency is the one of the stored rates.
    pub estimate: Option<String>,
}

/// The rows of a period, and the totals of all of them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageReport {
    /// One row per key: days oldest first, the other groups by `queries`
    /// descending, then by key.
    pub rows: Vec<UsageRow>,
    /// The sum of the whole period, also when `rows` is empty.
    pub totals: UsageRow,
}

/// The sum, as ONE constant statement: every value is bound (`$1` first day,
/// `$2` last day, `$3` tenant, `$4` whether every tenant counts, `$5` the
/// group). It is `pub` so the index test can `EXPLAIN` exactly the text the
/// store runs.
///
/// `runs` reads the period through `query_history_tenant_at_idx`
/// (`tenant_id`, `at`); `rated` is one row per day of the period with the
/// rate of each kind in force on it, so a run is priced with its own day's
/// rate. `MATERIALIZED` keeps the two rate lookups at one per day: inlined,
/// the planner runs them once per run. `GROUPING SETS ((key), ())` returns the rows and the totals in one
/// pass over the runs.
pub const REPORT_SQL: &str = "\
WITH rated AS MATERIALIZED ( \
    SELECT d.day, \
           (SELECT r.amount FROM usage_rate r \
             WHERE r.kind = 'scan_tb' AND r.effective_from <= d.day \
             ORDER BY r.effective_from DESC, r.created_at DESC, r.id DESC LIMIT 1) AS scan_tb, \
           (SELECT r.amount FROM usage_rate r \
             WHERE r.kind = 'engine_hour' AND r.effective_from <= d.day \
             ORDER BY r.effective_from DESC, r.created_at DESC, r.id DESC LIMIT 1) AS engine_hour \
      FROM (SELECT $1::date + n AS day \
              FROM generate_series(0, $2::date - $1::date) AS g(n)) d \
), runs AS ( \
    SELECT (q.at AT TIME ZONE 'UTC')::date AS day, q.at, q.status, q.engine, \
           q.owner_id, q.user_name, q.tenant_id, q.scanned_bytes, q.duration_ms \
      FROM query_history q \
     WHERE q.at >= ($1::date)::timestamp AT TIME ZONE 'UTC' \
       AND q.at <  (($2::date) + 1)::timestamp AT TIME ZONE 'UTC' \
       AND ($4::boolean OR q.tenant_id = $3::uuid) \
), keyed AS ( \
    SELECT CASE $5::text \
               WHEN 'day' THEN to_char(runs.day, 'YYYY-MM-DD') \
               WHEN 'person' THEN COALESCE(runs.owner_id::text, runs.user_name) \
               WHEN 'engine' THEN runs.engine \
               ELSE COALESCE(runs.tenant_id::text, 'none') \
           END AS key, \
           CASE $5::text \
               WHEN 'tenant' THEN COALESCE(t.name, 'no tenant') \
               WHEN 'person' THEN runs.user_name \
               ELSE '' \
           END AS name, \
           runs.at, runs.status, runs.scanned_bytes, runs.duration_ms, \
           runs.scanned_bytes::numeric / 1000000000000 * rated.scan_tb AS scan_part, \
           runs.duration_ms::numeric / 3600000 * rated.engine_hour AS engine_part \
      FROM runs \
      JOIN rated ON rated.day = runs.day \
      LEFT JOIN tenant t ON t.id = runs.tenant_id \
) \
SELECT GROUPING(k.key) = 1 AS is_total, \
       COALESCE(k.key, '') AS key, \
       CASE WHEN GROUPING(k.key) = 1 THEN '' \
            WHEN $5::text IN ('person', 'tenant') THEN (array_agg(k.name ORDER BY k.at DESC))[1] \
            ELSE k.key END AS label, \
       count(*)::bigint AS queries, \
       (count(*) FILTER (WHERE k.status = 'completed'))::bigint AS completed, \
       (count(*) FILTER (WHERE k.status = 'failed'))::bigint AS failed, \
       (count(*) FILTER (WHERE k.status = 'blocked'))::bigint AS blocked, \
       (count(*) FILTER (WHERE k.status IN ('cancelled', 'stopped')))::bigint AS cancelled, \
       COALESCE(sum(k.scanned_bytes), 0)::bigint AS scanned_bytes, \
       COALESCE(sum(k.duration_ms), 0)::bigint AS engine_ms, \
       CASE WHEN count(k.scan_part) + count(k.engine_part) = 0 THEN NULL \
            ELSE round(COALESCE(sum(k.scan_part), 0) + COALESCE(sum(k.engine_part), 0), 6)::text \
       END AS estimate \
  FROM keyed k \
 GROUP BY GROUPING SETS ((k.key), ())";

/// A row of [`REPORT_SQL`]: the report's row and whether it is the totals.
#[derive(sqlx::FromRow)]
struct ReportRow {
    is_total: bool,
    #[sqlx(flatten)]
    row: UsageRow,
}

/// Sum the runs of `query`.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the query fails.
pub async fn report(pool: &PgPool, query: &UsageQuery) -> Result<UsageReport, StoreError> {
    let (tenant, everyone) = match query.scope {
        TenantScope::Tenant(id) => (Some(id), false),
        TenantScope::Everyone => (None, true),
    };
    let fetched: Vec<ReportRow> = sqlx::query_as(REPORT_SQL)
        .bind(query.from)
        .bind(query.to)
        .bind(tenant)
        .bind(everyone)
        .bind(query.group.as_str())
        .fetch_all(pool)
        .await?;
    let mut rows = Vec::with_capacity(fetched.len());
    let mut totals = None;
    for fetched_row in fetched {
        if fetched_row.is_total {
            totals = Some(fetched_row.row);
        } else {
            rows.push(fetched_row.row);
        }
    }
    match query.group {
        Group::Day => rows.sort_by(|a, b| a.key.cmp(&b.key)),
        Group::Person | Group::Engine | Group::Tenant => {
            rows.sort_by(|a, b| b.queries.cmp(&a.queries).then_with(|| a.key.cmp(&b.key)));
        }
    }
    // `GROUPING SETS ((key), ())` always yields the empty set's row, also over
    // no runs, so a missing totals row would be the database's bug; answer
    // with zeros rather than a panic.
    let totals = totals.unwrap_or(UsageRow {
        key: String::new(),
        label: String::new(),
        queries: 0,
        completed: 0,
        failed: 0,
        blocked: 0,
        cancelled: 0,
        scanned_bytes: 0,
        engine_ms: 0,
        estimate: None,
    });
    Ok(UsageReport { rows, totals })
}

/// A stored rate.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct UsageRate {
    /// The row's id.
    pub id: Uuid,
    /// `scan_tb`, `engine_hour` or `storage_gb_month`.
    pub kind: String,
    /// Three upper-case letters.
    pub currency: String,
    /// The price, a decimal with six places, as text.
    pub amount: String,
    /// The first day the rate applies on.
    pub effective_from: Date,
    /// The principal id of whoever entered it.
    pub created_by: String,
    /// When it was entered.
    pub created_at: OffsetDateTime,
}

/// A rate to add. The table's `CHECK`s are the backstop for `kind`,
/// `currency` and `amount`; the route validates them first and answers with
/// its own sentence.
#[derive(Debug, Clone, Copy)]
pub struct NewUsageRate<'a> {
    /// `scan_tb`, `engine_hour` or `storage_gb_month`.
    pub kind: &'a str,
    /// Three upper-case letters.
    pub currency: &'a str,
    /// A non-negative decimal as text, at most six decimal places: it is
    /// cast to `NUMERIC` by the database, so it never passes through a float.
    pub amount: &'a str,
    /// The first day the rate applies on.
    pub effective_from: Date,
    /// The principal id of whoever enters it.
    pub created_by: &'a str,
}

const RATE_COLUMNS: &str =
    "id, kind, currency, amount::text AS amount, effective_from, created_by, created_at";

/// Every stored rate, newest `effective_from` first, then the newest entered.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the query fails.
pub async fn list_rates(pool: &PgPool) -> Result<Vec<UsageRate>, StoreError> {
    let sql = format!(
        "SELECT {RATE_COLUMNS} FROM usage_rate ORDER BY effective_from DESC, created_at DESC, id"
    );
    // safe: only the constant column list is formatted in.
    Ok(sqlx::query_as(sqlx::AssertSqlSafe(sql))
        .fetch_all(pool)
        .await?)
}

/// Add a rate. Returns `None`, and writes nothing, when another stored rate
/// has a different currency: one estimate never adds two currencies. The
/// check and the insert are one statement, so two concurrent writers cannot
/// both pass it.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the write fails, including when the
/// table's `CHECK`s refuse the values.
pub async fn add_rate(
    pool: &PgPool,
    rate: &NewUsageRate<'_>,
) -> Result<Option<UsageRate>, StoreError> {
    let sql = format!(
        "INSERT INTO usage_rate (kind, currency, amount, effective_from, created_by) \
         SELECT $1, $2, $3::numeric, $4, $5 \
          WHERE NOT EXISTS (SELECT 1 FROM usage_rate WHERE currency <> $2) \
         RETURNING {RATE_COLUMNS}"
    );
    // safe: only the constant column list is formatted in; every value is bound.
    Ok(sqlx::query_as(sqlx::AssertSqlSafe(sql))
        .bind(rate.kind)
        .bind(rate.currency)
        .bind(rate.amount)
        .bind(rate.effective_from)
        .bind(rate.created_by)
        .fetch_optional(pool)
        .await?)
}
