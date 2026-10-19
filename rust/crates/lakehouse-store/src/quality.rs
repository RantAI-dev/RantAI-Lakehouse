//! Data-quality incidents, schedules and load history (`DATA-14`,
//! migration `0081`).
//!
//! # Incidents
//!
//! A failed run of an authored rule, or a learned anomaly, opens an
//! incident ([`record_failure`]); it stays open until a person resolves it
//! with a written reason ([`resolve`]). The partial unique index
//! `quality_incident_one_open` allows one open incident per (table, rule)
//! or (table, anomaly kind), and [`record_failure`] upserts on it: a failure
//! seen again moves `last_seen_at` and the detail, it does not open a second
//! incident, and it reports `opened: false` so nobody is alerted twice.
//! A passing run closes nothing here: that is a person's call.
//!
//! Timestamps are rendered by Postgres as `YYYY-MM-DDTHH:MM:SS.mmmZ` text,
//! so no caller needs a date library to show them.

use serde::Serialize;
use sqlx::FromRow;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{PgPool, StoreError};

/// The most incidents one list returns.
pub const MAX_INCIDENTS: i64 = 500;

/// The longest a resolution may be, in characters (the table's `CHECK`).
pub const MAX_RESOLUTION_CHARS: usize = 500;

/// The longest incident detail kept, in characters (the table's `CHECK`).
pub const MAX_DETAIL_CHARS: usize = 1000;

/// What kind of finding an incident records.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IncidentKind {
    /// An authored rule's run failed.
    Check,
    /// A load much smaller than the table's history.
    VolumeAnomaly,
    /// No load for much longer than the table's usual gap.
    FreshnessAnomaly,
}

impl IncidentKind {
    /// The value stored in `quality_incident.kind`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Check => "check",
            Self::VolumeAnomaly => "volume_anomaly",
            Self::FreshnessAnomaly => "freshness_anomaly",
        }
    }
}

/// One incident, as the API lists it. Mirrors `QualityIncident` in
/// `contracts/governance.ts`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct QualityIncident {
    /// `quality_incident.id`.
    pub id: String,
    /// The table, lower-cased (`silver.orders`).
    pub asset: String,
    /// The rule that failed; `None` for an anomaly.
    pub rule_id: Option<String>,
    /// The rule's name, while the rule still exists.
    pub rule_name: Option<String>,
    /// `check`, `volume_anomaly` or `freshness_anomaly`.
    pub kind: String,
    /// When the incident opened.
    pub opened_at: String,
    /// When a run or a pass last found it still broken.
    pub last_seen_at: String,
    /// What was found, in words.
    pub detail: String,
    /// `open` or `resolved`.
    pub status: String,
    /// When it was resolved.
    pub resolved_at: Option<String>,
    /// Who resolved it, by name where the person is still known.
    pub resolved_by: Option<String>,
    /// The reason given.
    pub resolution: Option<String>,
}

const TS: &str = "'YYYY-MM-DD\"T\"HH24:MI:SS.MS\"Z\"'";

/// The columns of [`QualityIncident`], after `FROM quality_incident i
/// LEFT JOIN quality_rule r ON r.id::text = i.rule_id LEFT JOIN app_user u
/// ON u.id = i.resolved_by_user_id`. Built once so a `SELECT` and a
/// `RETURNING` cannot drift.
fn columns() -> String {
    format!(
        "i.id::text AS id, i.asset, i.rule_id, r.name AS rule_name, i.kind, \
         to_char(i.opened_at AT TIME ZONE 'UTC', {TS}) AS opened_at, \
         to_char(i.last_seen_at AT TIME ZONE 'UTC', {TS}) AS last_seen_at, \
         i.detail, i.status, \
         to_char(i.resolved_at AT TIME ZONE 'UTC', {TS}) AS resolved_at, \
         u.name AS resolved_by, i.resolution"
    )
}

const JOINS: &str = "FROM quality_incident i \
     LEFT JOIN quality_rule r ON r.id::text = i.rule_id \
     LEFT JOIN app_user u ON u.id = i.resolved_by_user_id";

/// Keeps `text` within `max` characters, ending in an ellipsis when it was
/// cut, so a long upstream-derived detail cannot trip the table's `CHECK`.
fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let kept: String = text.chars().take(max.saturating_sub(1)).collect();
    format!("{kept}…")
}

/// What [`record_failure`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recorded {
    /// The incident's id.
    pub id: String,
    /// True when this call opened it, false when it was already open and
    /// only moved `last_seen_at`.
    pub opened: bool,
}

/// Opens an incident for `asset` and `rule_id` (or, with no rule, for the
/// anomaly `kind`), or — when one is already open for the same key — moves
/// its `last_seen_at` and replaces its detail with the latest finding.
///
/// # Errors
///
/// [`StoreError::Database`] if the upsert fails, or [`StoreError::Validation`]
/// when `kind` and `rule_id` disagree (a check names a rule, an anomaly
/// does not).
pub async fn record_failure(
    pool: &PgPool,
    asset: &str,
    rule_id: Option<&str>,
    kind: IncidentKind,
    detail: &str,
) -> Result<Recorded, StoreError> {
    if (kind == IncidentKind::Check) != rule_id.is_some() {
        return Err(StoreError::Validation(
            "a check incident names its rule, an anomaly names none".to_owned(),
        ));
    }
    // `xmax = 0` is true only for a row this statement inserted; the
    // conflicting row comes back through the `DO UPDATE` with its
    // transaction id set.
    // PR #106 review R1 SHOULD-FIX DATA-14 3 / D2
    let (id, opened): (Uuid, bool) = sqlx::query_as(
        "INSERT INTO quality_incident (asset, rule_id, kind, detail) \
         VALUES ($1, $2, $3, $4) \
         ON CONFLICT (asset, (COALESCE(rule_id, kind))) WHERE status IN ('open', 'stale') \
         DO UPDATE SET last_seen_at = now(), detail = EXCLUDED.detail, status = 'open' \
         RETURNING id, (xmax = 0) AS opened",
    )
    .bind(asset.trim().to_lowercase())
    .bind(rule_id)
    .bind(kind.as_str())
    .bind(clip(detail, MAX_DETAIL_CHARS))
    .fetch_one(pool)
    .await?;
    Ok(Recorded {
        id: id.to_string(),
        opened,
    })
}

/// Which incidents [`list`] returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusFilter {
    /// Only open incidents.
    Open,
    /// Only resolved ones.
    Resolved,
    /// Both.
    All,
}

impl StatusFilter {
    /// Reads the `status` query value; `None` for anything else.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "open" => Some(Self::Open),
            "resolved" => Some(Self::Resolved),
            "all" => Some(Self::All),
            _ => None,
        }
    }
}

/// Incidents, newest first, at most [`MAX_INCIDENTS`]. `asset` narrows to one
/// table (compared lower-cased).
///
/// # Errors
///
/// [`StoreError::Database`] if the query fails.
pub async fn list(
    pool: &PgPool,
    status: StatusFilter,
    asset: Option<&str>,
) -> Result<Vec<QualityIncident>, StoreError> {
    let wanted = match status {
        StatusFilter::Open => Some("open"),
        StatusFilter::Resolved => Some("resolved"),
        StatusFilter::All => None,
    };
    // PR #106 review R1 SHOULD-FIX DATA-14 3 / D2: 'open' filter includes 'stale'
    let sql = format!(
        "SELECT {} {JOINS} \
         WHERE ($1::text IS NULL \
                OR ($1 = 'open' AND i.status IN ('open', 'stale')) \
                OR i.status = $1) \
           AND ($2::text IS NULL OR i.asset = $2) \
         ORDER BY i.opened_at DESC, i.id LIMIT {MAX_INCIDENTS}",
        columns()
    );
    let rows = sqlx::query_as::<_, QualityIncident>(&sql)
        .bind(wanted)
        .bind(asset.map(|a| a.trim().to_lowercase()))
        .fetch_all(pool)
        .await?;
    Ok(rows)
}

/// What [`resolve`] found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    /// It was open and is now resolved.
    Resolved(Box<QualityIncident>),
    /// It was resolved before.
    AlreadyResolved,
    /// No incident has that id (including an id that is not a UUID).
    NotFound,
}

/// Resolves the open incident `id` as `by`, with the reason `resolution`.
///
/// # Errors
///
/// [`StoreError::Validation`] when the reason is empty or longer than
/// [`MAX_RESOLUTION_CHARS`]; [`StoreError::Database`] on a query failure.
pub async fn resolve(
    pool: &PgPool,
    id: &str,
    by: Uuid,
    resolution: &str,
) -> Result<Resolution, StoreError> {
    let resolution = resolution.trim();
    if resolution.is_empty() {
        return Err(StoreError::Validation(
            "Say why the incident is resolved.".to_owned(),
        ));
    }
    if resolution.chars().count() > MAX_RESOLUTION_CHARS {
        return Err(StoreError::Validation(format!(
            "The reason may be at most {MAX_RESOLUTION_CHARS} characters."
        )));
    }
    let Ok(id) = Uuid::parse_str(id) else {
        return Ok(Resolution::NotFound);
    };
    // PR #106 review R1 SHOULD-FIX DATA-14 3 / D2
    let updated = sqlx::query(
        "UPDATE quality_incident \
         SET status = 'resolved', resolved_at = now(), resolved_by_user_id = $2, resolution = $3 \
         WHERE id = $1 AND status IN ('open', 'stale')",
    )
    .bind(id)
    .bind(by)
    .bind(resolution)
    .execute(pool)
    .await?;
    let sql = format!("SELECT {} {JOINS} WHERE i.id = $1", columns());
    let row = sqlx::query_as::<_, QualityIncident>(&sql)
        .bind(id)
        .fetch_optional(pool)
        .await?;
    Ok(match (row, updated.rows_affected()) {
        (Some(row), 1) => Resolution::Resolved(Box::new(row)),
        (Some(_), _) => Resolution::AlreadyResolved,
        (None, _) => Resolution::NotFound,
    })
}

/// Resolves every open incident of the rule `rule_id` with `resolution`
/// (a deleted rule), returning how many it closed.
///
/// # Errors
///
/// [`StoreError::Database`] if the update fails.
pub async fn resolve_for_rule(
    pool: &PgPool,
    rule_id: &str,
    by: Uuid,
    resolution: &str,
) -> Result<u64, StoreError> {
    // PR #106 review R1 SHOULD-FIX DATA-14 3 / D2
    let done = sqlx::query(
        "UPDATE quality_incident \
         SET status = 'resolved', resolved_at = now(), resolved_by_user_id = $2, resolution = $3 \
         WHERE rule_id = $1 AND status IN ('open', 'stale')",
    )
    .bind(rule_id)
    .bind(by)
    .bind(clip(resolution, MAX_RESOLUTION_CHARS))
    .execute(pool)
    .await?;
    Ok(done.rows_affected())
}

// ── Schedules ──────────────────────────────────────────────────────────

/// The shortest and longest interval a table's rules may run at, in
/// minutes (one hour to one day; the table's `CHECK`).
pub const MIN_INTERVAL_MINUTES: i32 = 60;
/// See [`MIN_INTERVAL_MINUTES`].
pub const MAX_INTERVAL_MINUTES: i32 = 1440;

/// The interval of a table with no schedule row: today's behaviour.
pub const DEFAULT_INTERVAL_MINUTES: i32 = 60;

/// How a table's rules are run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScheduleMode {
    /// After every successful load of the table, and every
    /// `interval_minutes` as a backstop for loads that never report.
    OnLoad,
    /// Every `interval_minutes`.
    Interval,
}

impl ScheduleMode {
    /// The value stored in `quality_schedule.mode`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OnLoad => "on_load",
            Self::Interval => "interval",
        }
    }

    /// Reads a stored or requested mode; `None` for anything else.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "on_load" => Some(Self::OnLoad),
            "interval" => Some(Self::Interval),
            _ => None,
        }
    }
}

/// One table's schedule. Mirrors `QualitySchedule` in
/// `contracts/governance.ts`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct QualitySchedule {
    /// The table, lower-cased.
    pub asset: String,
    /// `on_load` or `interval`.
    pub mode: String,
    /// Minutes between runs (the backstop for `on_load`).
    pub interval_minutes: i32,
    /// Who set it last, by name where the person is still known.
    pub updated_by: Option<String>,
    /// When it was set.
    pub updated_at: String,
}

const SCHEDULE_SELECT: &str = "SELECT s.asset, s.mode, s.interval_minutes, u.name AS updated_by, \
     to_char(s.updated_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.MS\"Z\"') AS updated_at \
     FROM quality_schedule s LEFT JOIN app_user u ON u.id = s.updated_by";

/// Every table that has a schedule. A table not listed runs every
/// [`DEFAULT_INTERVAL_MINUTES`].
///
/// # Errors
///
/// [`StoreError::Database`] if the query fails.
pub async fn list_schedules(pool: &PgPool) -> Result<Vec<QualitySchedule>, StoreError> {
    let sql = format!("{SCHEDULE_SELECT} ORDER BY s.asset");
    Ok(sqlx::query_as::<_, QualitySchedule>(&sql)
        .fetch_all(pool)
        .await?)
}

/// Sets the schedule of `asset`.
///
/// # Errors
///
/// [`StoreError::Validation`] when `interval_minutes` is outside
/// [`MIN_INTERVAL_MINUTES`]..=[`MAX_INTERVAL_MINUTES`] or the table name is
/// empty or longer than 256 characters; [`StoreError::Database`] otherwise.
pub async fn put_schedule(
    pool: &PgPool,
    asset: &str,
    mode: ScheduleMode,
    interval_minutes: i32,
    by: Uuid,
) -> Result<QualitySchedule, StoreError> {
    if !(MIN_INTERVAL_MINUTES..=MAX_INTERVAL_MINUTES).contains(&interval_minutes) {
        return Err(StoreError::Validation(format!(
            "The interval must be from {MIN_INTERVAL_MINUTES} minutes (1 hour) to \
             {MAX_INTERVAL_MINUTES} minutes (1 day)."
        )));
    }
    let asset = asset.trim().to_lowercase();
    if asset.is_empty() || asset.chars().count() > 256 {
        return Err(StoreError::Validation(
            "Name the table as zone.table.".to_owned(),
        ));
    }
    sqlx::query(
        "INSERT INTO quality_schedule (asset, mode, interval_minutes, updated_by, updated_at) \
         VALUES ($1, $2, $3, $4, now()) \
         ON CONFLICT (asset) DO UPDATE SET mode = EXCLUDED.mode, \
             interval_minutes = EXCLUDED.interval_minutes, \
             updated_by = EXCLUDED.updated_by, updated_at = now()",
    )
    .bind(&asset)
    .bind(mode.as_str())
    .bind(interval_minutes)
    .bind(by)
    .execute(pool)
    .await?;
    let sql = format!("{SCHEDULE_SELECT} WHERE s.asset = $1");
    Ok(sqlx::query_as::<_, QualitySchedule>(&sql)
        .bind(&asset)
        .fetch_one(pool)
        .await?)
}

// ── Load history ───────────────────────────────────────────────────────

/// How long a load is kept, in days (the spec's history for learning).
pub const LOAD_RETENTION_DAYS: i32 = 90;

/// One recorded load of a table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, FromRow)]
pub struct TableLoad {
    /// When the load finished.
    pub loaded_at: OffsetDateTime,
    /// The table's row count after it.
    pub rows_after: i64,
}

/// What a table's history holds, for the learned-or-learning decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, FromRow)]
pub struct LoadSummary {
    /// How many loads are recorded.
    pub loads: i64,
    /// The oldest one.
    pub first_at: OffsetDateTime,
    /// The newest one.
    pub last_at: OffsetDateTime,
}

/// A table's summary together with its name, for the tick's pass over every
/// table.
#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct AssetLoadSummary {
    /// The table, lower-cased.
    pub asset: String,
    /// How many loads are recorded.
    pub loads: i64,
    /// The oldest one.
    pub first_at: OffsetDateTime,
    /// The newest one.
    pub last_at: OffsetDateTime,
}

/// Records that `asset` just finished a load and now holds `rows_after`
/// rows. Written for every loaded table whatever its schedule.
///
/// # Errors
///
/// [`StoreError::Database`] if the insert fails (including a negative count
/// or an empty or over-long table name, which the table's `CHECK`s refuse).
pub async fn record_load(pool: &PgPool, asset: &str, rows_after: i64) -> Result<(), StoreError> {
    sqlx::query(
        "INSERT INTO table_load (asset, loaded_at, rows_after) VALUES ($1, now(), $2) \
         ON CONFLICT (asset, loaded_at) DO NOTHING",
    )
    .bind(asset.trim().to_lowercase())
    .bind(rows_after)
    .execute(pool)
    .await?;
    Ok(())
}

/// The newest `limit` loads of `asset`, oldest first.
///
/// # Errors
///
/// [`StoreError::Database`] if the query fails.
pub async fn recent_loads(
    pool: &PgPool,
    asset: &str,
    limit: i64,
) -> Result<Vec<TableLoad>, StoreError> {
    Ok(sqlx::query_as::<_, TableLoad>(
        "SELECT loaded_at, rows_after FROM ( \
             SELECT loaded_at, rows_after FROM table_load WHERE asset = $1 \
             ORDER BY loaded_at DESC LIMIT $2 \
         ) newest ORDER BY loaded_at",
    )
    .bind(asset.trim().to_lowercase())
    .bind(limit.clamp(1, 1000))
    .fetch_all(pool)
    .await?)
}

/// What `asset`'s history holds, or `None` for a table never recorded.
///
/// # Errors
///
/// [`StoreError::Database`] if the query fails.
pub async fn load_summary(pool: &PgPool, asset: &str) -> Result<Option<LoadSummary>, StoreError> {
    Ok(sqlx::query_as::<_, LoadSummary>(
        "SELECT count(*) AS loads, min(loaded_at) AS first_at, max(loaded_at) AS last_at \
         FROM table_load WHERE asset = $1 HAVING count(*) > 0",
    )
    .bind(asset.trim().to_lowercase())
    .fetch_optional(pool)
    .await?)
}

/// Every recorded table's summary, for the tick's freshness pass.
///
/// # Errors
///
/// [`StoreError::Database`] if the query fails.
pub async fn load_summaries(pool: &PgPool) -> Result<Vec<AssetLoadSummary>, StoreError> {
    Ok(sqlx::query_as::<_, AssetLoadSummary>(
        "SELECT asset, count(*) AS loads, min(loaded_at) AS first_at, max(loaded_at) AS last_at \
         FROM table_load GROUP BY asset ORDER BY asset",
    )
    .fetch_all(pool)
    .await?)
}

/// Deletes loads older than [`LOAD_RETENTION_DAYS`], returning how many.
///
/// # Errors
///
/// [`StoreError::Database`] if the delete fails.
pub async fn purge_loads(pool: &PgPool) -> Result<u64, StoreError> {
    let done =
        sqlx::query("DELETE FROM table_load WHERE loaded_at < now() - make_interval(days => $1)")
            .bind(LOAD_RETENTION_DAYS)
            .execute(pool)
            .await?;
    Ok(done.rows_affected())
}

/// Whether a person already resolved a freshness incident of `asset` that
/// opened after `since` (its last load). A table that stays silent after
/// someone resolved the incident has not become a new problem, so the tick
/// does not open a second incident for the same silence.
///
/// # Errors
///
/// [`StoreError::Database`] if the query fails.
pub async fn freshness_resolved_since(
    pool: &PgPool,
    asset: &str,
    since: OffsetDateTime,
) -> Result<bool, StoreError> {
    Ok(sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM quality_incident WHERE asset = $1 \
         AND kind = 'freshness_anomaly' AND status = 'resolved' AND opened_at >= $2)",
    )
    .bind(asset.trim().to_lowercase())
    .bind(since)
    .fetch_one(pool)
    .await?)
}

/// Marks any open incident for `rule_id` as stale because a subsequent run passed.
///
/// # Errors
///
/// [`StoreError::Database`] if the update fails.
// PR #106 review R1 SHOULD-FIX DATA-14 3 / D2: a passing run marks any open incident for this rule stale
pub async fn mark_stale_for_rule(pool: &PgPool, rule_id: &str) -> Result<u64, StoreError> {
    let done = sqlx::query(
        "UPDATE quality_incident \
         SET status = 'stale' \
         WHERE rule_id = $1 AND status = 'open'",
    )
    .bind(rule_id)
    .execute(pool)
    .await?;
    Ok(done.rows_affected())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_long_detail_is_cut_with_an_ellipsis_within_the_limit() {
        let long = "x".repeat(MAX_DETAIL_CHARS + 50);
        let clipped = clip(&long, MAX_DETAIL_CHARS);
        assert_eq!(clipped.chars().count(), MAX_DETAIL_CHARS);
        assert!(clipped.ends_with('…'));
        assert_eq!(clip("short", MAX_DETAIL_CHARS), "short");
    }

    #[test]
    fn the_schedule_mode_reads_only_its_two_words() {
        assert_eq!(ScheduleMode::parse("on_load"), Some(ScheduleMode::OnLoad));
        assert_eq!(
            ScheduleMode::parse("interval"),
            Some(ScheduleMode::Interval)
        );
        assert_eq!(ScheduleMode::parse("hourly"), None);
        assert_eq!(ScheduleMode::OnLoad.as_str(), "on_load");
    }

    #[test]
    fn the_status_filter_reads_only_its_three_words() {
        assert_eq!(StatusFilter::parse("open"), Some(StatusFilter::Open));
        assert_eq!(
            StatusFilter::parse("resolved"),
            Some(StatusFilter::Resolved)
        );
        assert_eq!(StatusFilter::parse("all"), Some(StatusFilter::All));
        assert_eq!(StatusFilter::parse("OPEN"), None);
        assert_eq!(StatusFilter::parse(""), None);
    }
}
