//! Volume and freshness anomalies learned from a table's own load history
//! (`DATA-14`, finding F5).
//!
//! # Why the product records its own history
//!
//! Iceberg snapshots are kept 7 days, the ingest log keys on the source
//! object with no run id, and the per-pipeline volume alert looks at the last
//! 30 Dagster runs of one pipeline. None of them gives a table's loads over 14
//! days whatever loaded it, so every load the on-load hook sees
//! (`routes::quality_load`) is written to `table_load` with the table's row
//! count after it, and kept 90 days.
//!
//! # Learned, or still learning
//!
//! A table is **learned** once its history spans [`LEARN_DAYS`] days (from its
//! first recorded load to now) and holds [`LEARN_LOADS`] loads. Until then no
//! anomaly is raised, and the table says so ([`Learning`]): an alert from too
//! little history would be a guess.
//!
//! # Volume, on each load
//!
//! The load's size is the increase over the previous load when the table only
//! ever grew across the history looked at, and its whole row count otherwise
//! (a table that is replaced, or that sometimes shrinks, has no meaningful
//! increase). It is an anomaly when under half the median of the previous
//! [`BASELINE_LOADS`] loads' sizes, measured the same way: a load of 10% of
//! the usual is flagged (`DATA-14-AC2`).
//!
//! # Freshness, on the tick
//!
//! An anomaly when the time since the table's last load exceeds twice the
//! median gap between its loads (over the last [`BASELINE_LOADS`] gaps), and
//! at least [`MIN_FRESHNESS_SECONDS`]. One silence opens one incident: once a
//! person has resolved the freshness incident of a table that has not loaded
//! since, the tick does not open another for the same silence; the next load
//! ends the silence.
//!
//! Everything here is bookkeeping around a load or a tick: a failure is logged
//! and never fails the event that started it.

use axum::Extension;
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use lakehouse_auth::Principal;
use lakehouse_core::ApiError;
use lakehouse_store::quality::{self, IncidentKind, LoadSummary, TableLoad};
use serde_json::{Value, json};
#[cfg(test)]
use time::Duration;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::error::ApiResult;
use crate::json::ApiJson;
use crate::routes::quality::pool;
use crate::routes::quality_schedules::table_key;
use crate::state::AppState;

/// Days of history before a table's anomalies are raised.
pub(crate) const LEARN_DAYS: i64 = 14;
/// Loads of history before a table's anomalies are raised.
pub(crate) const LEARN_LOADS: i64 = 5;
/// How many earlier loads (or gaps) the usual size (or gap) is the median of.
pub(crate) const BASELINE_LOADS: usize = 30;
/// A load smaller than this share of the usual is a volume anomaly.
const VOLUME_SHARE: f64 = 0.5;
/// A table is late when quiet for more than this many median gaps.
const FRESHNESS_GAPS: i64 = 2;
/// ... and for at least this long, so a table loaded every minute is not
/// flagged after three quiet ones.
pub(crate) const MIN_FRESHNESS_SECONDS: i64 = 3_600;

/// Whether a table has enough history for its anomalies to be raised.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Learning {
    /// Not yet: how much history there is so far.
    Learning { days: i64, loads: i64 },
    /// Enough.
    Learned,
}

/// Judges `summary` at `now`.
pub(crate) fn learning(summary: &LoadSummary, now: OffsetDateTime) -> Learning {
    let days = (now - summary.first_at).whole_days().max(0);
    if days >= LEARN_DAYS && summary.loads >= LEARN_LOADS {
        Learning::Learned
    } else {
        Learning::Learning {
            days,
            loads: summary.loads,
        }
    }
}

/// What a table with too little history is told, word for word.
pub(crate) fn learning_message(days: i64, loads: i64) -> String {
    format!(
        "learning: {days} day{} and {loads} load{} so far; anomalies start after \
         {LEARN_DAYS} days and {LEARN_LOADS} loads",
        if days == 1 { "" } else { "s" },
        if loads == 1 { "" } else { "s" },
    )
}

/// The median of `values` (the mean of the two middle ones for an even
/// count); `None` for none.
fn median(values: &[i64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let mid = sorted.len() / 2;
    #[allow(
        clippy::cast_precision_loss,
        reason = "row counts and gaps far below 2^53"
    )]
    let value = if sorted.len() % 2 == 1 {
        sorted[mid] as f64
    } else {
        f64::midpoint(sorted[mid - 1] as f64, sorted[mid] as f64)
    };
    Some(value)
}

/// How a load's size was measured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Measure {
    /// The rows it added to a table that only ever grew.
    Added,
    /// The rows the table holds after it.
    Total,
}

/// A load found much smaller than the usual.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct VolumeFinding {
    pub(crate) measure: Measure,
    /// This load's size.
    pub(crate) size: i64,
    /// The median size of the loads before it.
    pub(crate) usual: f64,
    /// How many earlier loads the median is of.
    pub(crate) baseline: usize,
}

impl VolumeFinding {
    /// What the incident says.
    #[allow(
        clippy::cast_precision_loss,
        reason = "row counts far below 2^53, shown as a rounded percentage"
    )]
    pub(crate) fn detail(&self) -> String {
        let percent = if self.usual > 0.0 {
            self.size as f64 / self.usual * 100.0
        } else {
            0.0
        };
        let (this, usual) = match self.measure {
            Measure::Added => ("added", "added"),
            Measure::Total => ("left", "held"),
        };
        format!(
            "The latest load {this} {} rows, {percent:.0}% of the usual: the median of the \
             previous {} loads {usual} {:.0}.",
            self.size, self.baseline, self.usual
        )
    }
}

/// Whether the newest of `rows_after` (the table's row count after each load,
/// oldest first) is a volume anomaly. `None` for a normal load, and for a
/// history too short to have a usual (fewer than two loads).
pub(crate) fn volume_anomaly(rows_after: &[i64]) -> Option<VolumeFinding> {
    let (&latest, earlier) = rows_after.split_last()?;
    if earlier.is_empty() {
        return None;
    }
    let only_grew = rows_after.windows(2).all(|pair| pair[1] >= pair[0]);
    let (measure, size, sizes): (Measure, i64, Vec<i64>) = if only_grew {
        let increments: Vec<i64> = rows_after.windows(2).map(|p| p[1] - p[0]).collect();
        let (&last, before) = increments.split_last()?;
        (Measure::Added, last, before.to_vec())
    } else {
        (Measure::Total, latest, earlier.to_vec())
    };
    let baseline: Vec<i64> = sizes
        .iter()
        .copied()
        .skip(sizes.len().saturating_sub(BASELINE_LOADS))
        .collect();
    let usual = median(&baseline)?;
    #[allow(clippy::cast_precision_loss, reason = "row counts far below 2^53")]
    let small = usual > 0.0 && (size as f64) < usual * VOLUME_SHARE;
    small.then_some(VolumeFinding {
        measure,
        size,
        usual,
        baseline: baseline.len(),
    })
}

/// A table quiet for much longer than its usual gap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FreshnessFinding {
    /// Time since the last load.
    pub(crate) quiet_seconds: i64,
    /// The median gap between loads.
    pub(crate) usual_seconds: i64,
}

impl FreshnessFinding {
    /// What the incident says.
    pub(crate) fn detail(&self) -> String {
        format!(
            "No load for {}; the median gap between its loads is {}.",
            human(self.quiet_seconds),
            human(self.usual_seconds)
        )
    }
}

fn human(seconds: i64) -> String {
    let s = seconds.max(0);
    if s < 3_600 {
        format!("{} min", s / 60)
    } else if s < 86_400 {
        format!("{}h {}m", s / 3_600, s % 3_600 / 60)
    } else {
        format!("{}d {}h", s / 86_400, s % 86_400 / 3_600)
    }
}

/// Whether a table loaded at `loads` (oldest first) is late at `now`.
pub(crate) fn freshness_anomaly(
    loads: &[OffsetDateTime],
    now: OffsetDateTime,
) -> Option<FreshnessFinding> {
    let gaps: Vec<i64> = loads
        .windows(2)
        .map(|pair| (pair[1] - pair[0]).whole_seconds())
        .collect();
    let gaps = &gaps[gaps.len().saturating_sub(BASELINE_LOADS)..];
    #[allow(
        clippy::cast_possible_truncation,
        reason = "a median of whole seconds, rounded back to whole seconds"
    )]
    let usual = median(gaps)?.round() as i64;
    let quiet = (now - *loads.last()?).whole_seconds();
    let limit = (usual * FRESHNESS_GAPS).max(MIN_FRESHNESS_SECONDS);
    (quiet > limit).then_some(FreshnessFinding {
        quiet_seconds: quiet,
        usual_seconds: usual,
    })
}

/// Records the load of `asset` that just finished (`rows_after` rows) and,
/// when the table is learned, opens a volume incident if it was far smaller
/// than the usual. Returns the id of an incident this call opened.
pub(crate) async fn note_load(state: &AppState, asset: &str, rows_after: i64) -> Option<String> {
    let pg = state.pg.as_deref()?;
    if let Err(err) = quality::record_load(pg, asset, rows_after).await {
        tracing::warn!(?err, %asset, "quality: the load could not be recorded");
        return None;
    }
    let summary = match quality::load_summary(pg, asset).await {
        Ok(Some(summary)) => summary,
        Ok(None) => return None,
        Err(err) => {
            tracing::warn!(?err, %asset, "quality: the load history could not be read");
            return None;
        }
    };
    if learning(&summary, OffsetDateTime::now_utc()) != Learning::Learned {
        return None;
    }
    // The newest load plus the baseline before it, and one more for the
    // increment the oldest baseline load made.
    #[allow(clippy::cast_possible_wrap, reason = "BASELINE_LOADS is 30")]
    let wanted = BASELINE_LOADS as i64 + 2;
    let loads = match quality::recent_loads(pg, asset, wanted).await {
        Ok(loads) => loads,
        Err(err) => {
            tracing::warn!(?err, %asset, "quality: the load history could not be read");
            return None;
        }
    };
    let series: Vec<i64> = loads.iter().map(|l: &TableLoad| l.rows_after).collect();
    let finding = volume_anomaly(&series)?;
    crate::routes::quality_incidents::open_anomaly(
        state,
        asset,
        IncidentKind::VolumeAnomaly,
        &finding.detail(),
    )
    .await
}

/// The tick's share: forget loads past their retention, and open a freshness
/// incident for every learned table that has gone quiet for too long.
pub(crate) async fn tick(state: &AppState) {
    let Some(pg) = state.pg.as_deref() else {
        return;
    };
    if let Err(err) = quality::purge_loads(pg).await {
        tracing::warn!(?err, "quality: old loads were not purged");
    }
    let summaries = match quality::load_summaries(pg).await {
        Ok(summaries) => summaries,
        Err(err) => {
            tracing::warn!(
                ?err,
                "quality: load history unavailable for the freshness pass"
            );
            return;
        }
    };
    let now = OffsetDateTime::now_utc();
    for table in summaries {
        let summary = LoadSummary {
            loads: table.loads,
            first_at: table.first_at,
            last_at: table.last_at,
        };
        if learning(&summary, now) != Learning::Learned {
            continue;
        }
        #[allow(clippy::cast_possible_wrap, reason = "BASELINE_LOADS is 30")]
        let wanted = BASELINE_LOADS as i64 + 1;
        let times: Vec<OffsetDateTime> = match quality::recent_loads(pg, &table.asset, wanted).await
        {
            Ok(loads) => loads.into_iter().map(|l| l.loaded_at).collect(),
            Err(err) => {
                tracing::warn!(?err, asset = %table.asset, "quality: loads unreadable");
                continue;
            }
        };
        let Some(finding) = freshness_anomaly(&times, now) else {
            continue;
        };
        // One silence, one incident: not a second after it was resolved.
        match quality::freshness_resolved_since(pg, &table.asset, summary.last_at).await {
            Ok(false) => {}
            Ok(true) => continue,
            Err(err) => {
                tracing::warn!(?err, asset = %table.asset, "quality: incidents unreadable");
                continue;
            }
        }
        crate::routes::quality_incidents::open_anomaly(
            state,
            &table.asset,
            IncidentKind::FreshnessAnomaly,
            &finding.detail(),
        )
        .await;
    }
}

/// `GET /api/governance/quality/learning/{asset}` — whether the table's
/// anomalies are being raised yet, and if not, how much history it has.
/// Anything the console shows about "learning" comes from here, so the
/// numbers and the sentence are the ones the detection uses.
///
/// # Errors
///
/// `400` when the table is not `bronze.`/`silver.`/`serving.` plus a plain
/// name; `503`/`500` on a store failure.
pub async fn learning_status(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    headers: HeaderMap,
    Path(asset): Path<String>,
) -> ApiResult<ApiJson<Value>> {
    // PR #106 review R1 SHOULD-FIX DATA-14 1 / D4
    if let Some(reason) =
        crate::routes::catalog::catalog_tenant_refusal(&state, &principal, &headers).await?
    {
        return Err(ApiError::PermissionDenied(reason.to_owned()).into());
    }
    let key = table_key(&asset).ok_or_else(|| {
        ApiError::BadRequest(
            "Name the table as bronze.<table>, silver.<table> or serving.<table>.".to_owned(),
        )
    })?;
    let summary = quality::load_summary(pool(&state)?, &key).await?;
    let now = OffsetDateTime::now_utc();
    let (state_now, first, last, loads) = match &summary {
        Some(s) => (
            learning(s, now),
            s.first_at.format(&Rfc3339).ok(),
            s.last_at.format(&Rfc3339).ok(),
            s.loads,
        ),
        None => (Learning::Learning { days: 0, loads: 0 }, None, None, 0),
    };
    let (learned, days, message) = match state_now {
        Learning::Learned => (
            true,
            summary.map_or(0, |s| (now - s.first_at).whole_days().max(0)),
            None,
        ),
        Learning::Learning { days, loads } => (false, days, Some(learning_message(days, loads))),
    };
    Ok(ApiJson(json!({
        "asset": key,
        "learned": learned,
        "days": days,
        "loads": loads,
        "requiredDays": LEARN_DAYS,
        "requiredLoads": LEARN_LOADS,
        "message": message,
        "firstLoadAt": first,
        "lastLoadAt": last,
    })))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn at(day: i64, hour: i64) -> OffsetDateTime {
        OffsetDateTime::UNIX_EPOCH + Duration::days(day) + Duration::hours(hour)
    }

    fn summary(first: OffsetDateTime, loads: i64) -> LoadSummary {
        LoadSummary {
            loads,
            first_at: first,
            last_at: first,
        }
    }

    #[test]
    fn a_table_is_learned_after_fourteen_days_and_five_loads() {
        let first = at(0, 0);
        assert_eq!(
            learning(&summary(first, 40), at(13, 23)),
            Learning::Learning {
                days: 13,
                loads: 40
            }
        );
        assert_eq!(
            learning(&summary(first, 4), at(30, 0)),
            Learning::Learning { days: 30, loads: 4 }
        );
        assert_eq!(learning(&summary(first, 5), at(14, 0)), Learning::Learned);
        // A clock behind the first load reads as no days, not a negative.
        assert_eq!(
            learning(&summary(at(5, 0), 1), at(4, 0)),
            Learning::Learning { days: 0, loads: 1 }
        );
    }

    #[test]
    fn an_unlearned_table_says_so_in_words() {
        assert_eq!(
            learning_message(3, 7),
            "learning: 3 days and 7 loads so far; anomalies start after 14 days and 5 loads"
        );
        assert_eq!(
            learning_message(1, 1),
            "learning: 1 day and 1 load so far; anomalies start after 14 days and 5 loads"
        );
    }

    #[test]
    fn the_median_of_an_even_count_is_the_mean_of_the_middle_two() {
        assert_eq!(median(&[]), None);
        assert_eq!(median(&[5]), Some(5.0));
        assert_eq!(median(&[3, 1, 2]), Some(2.0));
        assert_eq!(median(&[4, 1, 3, 2]), Some(2.5));
    }

    /// `DATA-14-AC2`: a load of 10% of the usual rows is flagged. The table
    /// grows 1000 rows a load, so the load is measured by what it added.
    #[test]
    fn a_growing_table_is_measured_by_what_each_load_added() {
        let mut rows: Vec<i64> = (1..=10).map(|n| n * 1_000).collect();
        // A normal load adds the usual 1000.
        rows.push(11_000);
        assert_eq!(volume_anomaly(&rows), None);
        // A load that adds 100 is 10% of the usual.
        *rows.last_mut().unwrap() = 10_100;
        let finding = volume_anomaly(&rows).expect("a 10% load is an anomaly");
        assert_eq!(finding.measure, Measure::Added);
        assert_eq!(finding.size, 100);
        assert!((finding.usual - 1_000.0).abs() < f64::EPSILON);
        assert_eq!(finding.baseline, 9);
        assert_eq!(
            finding.detail(),
            "The latest load added 100 rows, 10% of the usual: the median of the previous 9 \
             loads added 1000."
        );
        // Just under half is flagged, exactly half is not.
        *rows.last_mut().unwrap() = 10_499;
        assert!(volume_anomaly(&rows).is_some());
        *rows.last_mut().unwrap() = 10_500;
        assert_eq!(volume_anomaly(&rows), None);
    }

    /// A table that is replaced each time (or that ever shrank) has no
    /// increase to measure: its whole size is.
    #[test]
    fn a_table_that_is_replaced_is_measured_by_its_size() {
        let mut rows = vec![5_000, 5_100, 4_900, 5_000, 5_050, 4_950];
        assert_eq!(volume_anomaly(&rows), None);
        rows.push(500);
        let finding = volume_anomaly(&rows).expect("10% of the usual size");
        assert_eq!(finding.measure, Measure::Total);
        assert_eq!(finding.size, 500);
        assert!((finding.usual - 5_000.0).abs() < f64::EPSILON);
        assert_eq!(
            finding.detail(),
            "The latest load left 500 rows, 10% of the usual: the median of the previous 6 \
             loads held 5000."
        );
    }

    /// A growing table whose newest load shrank it is not "only grew": the
    /// whole size is compared.
    #[test]
    fn a_growing_table_that_suddenly_shrinks_is_flagged_by_its_size() {
        let rows = [1_000, 2_000, 3_000, 4_000, 5_000, 6_000, 400];
        let finding = volume_anomaly(&rows).expect("a collapse is an anomaly");
        assert_eq!(finding.measure, Measure::Total);
        assert_eq!(finding.size, 400);
    }

    #[test]
    fn too_little_history_or_a_zero_usual_flags_nothing() {
        assert_eq!(volume_anomaly(&[]), None);
        assert_eq!(volume_anomaly(&[100]), None);
        // Two loads: one increment, no earlier one to compare with.
        assert_eq!(volume_anomaly(&[100, 110]), None);
        // The usual increase is zero: nothing is "under half" of nothing.
        assert_eq!(volume_anomaly(&[100, 100, 100, 100, 101]), None);
    }

    /// Only the last 30 earlier loads are the usual.
    #[test]
    fn the_usual_is_the_median_of_the_last_thirty_loads() {
        // 40 old loads of 10, then 30 of 1000, then one of 400.
        let mut sizes = vec![10_i64; 40];
        sizes.extend(vec![1_000; 30]);
        sizes.push(400);
        let mut rows = Vec::new();
        let mut total = 0;
        for size in sizes {
            total += size;
            rows.push(total);
        }
        let finding = volume_anomaly(&rows).expect("400 is under half of 1000");
        assert_eq!(finding.baseline, BASELINE_LOADS);
        assert!((finding.usual - 1_000.0).abs() < f64::EPSILON);
    }

    fn loads_every(hours: i64, count: i64) -> Vec<OffsetDateTime> {
        (0..count).map(|n| at(0, n * hours)).collect()
    }

    /// A table loaded every 24 hours that is quiet for 3 days is late; at 47
    /// hours it is not.
    #[test]
    fn a_missed_load_is_a_freshness_anomaly_after_twice_the_usual_gap() {
        let loads = loads_every(24, 10);
        let last = *loads.last().unwrap();
        assert_eq!(freshness_anomaly(&loads, last + Duration::hours(47)), None);
        assert_eq!(freshness_anomaly(&loads, last + Duration::hours(48)), None);
        let finding = freshness_anomaly(&loads, last + Duration::hours(49))
            .expect("quiet for more than twice the gap");
        assert_eq!(finding.usual_seconds, 86_400);
        assert_eq!(finding.quiet_seconds, 49 * 3_600);
        assert_eq!(
            finding.detail(),
            "No load for 2d 1h; the median gap between its loads is 1d 0h."
        );
    }

    /// A table loaded every few minutes is not late after a quiet quarter hour.
    #[test]
    fn a_frequent_table_must_be_quiet_for_an_hour_to_be_late() {
        let loads: Vec<OffsetDateTime> = (0..10)
            .map(|n| at(0, 0) + Duration::minutes(n * 5))
            .collect();
        let last = *loads.last().unwrap();
        assert_eq!(
            freshness_anomaly(&loads, last + Duration::minutes(59)),
            None
        );
        assert!(freshness_anomaly(&loads, last + Duration::minutes(61)).is_some());
    }

    #[test]
    fn a_table_with_one_load_has_no_usual_gap() {
        let one = [at(0, 0)];
        assert_eq!(freshness_anomaly(&one, at(100, 0)), None);
        assert_eq!(freshness_anomaly(&[], at(100, 0)), None);
    }

    // ── Against a database: the history, the incidents, the tick ──────────

    use std::collections::HashMap;

    use lakehouse_store::quality::{StatusFilter, list};
    use sqlx::PgPool;

    use crate::config::Config;

    fn state_for(pool: &PgPool) -> AppState {
        let options = pool.connect_options();
        let url = format!(
            "postgres://{}:postgres@{}:{}/{}",
            options.get_username(),
            options.get_host(),
            options.get_port(),
            options.get_database().expect("a named test database"),
        );
        let mut env = HashMap::new();
        env.insert("DATABASE_URL".to_owned(), url);
        env.insert("CH_URL".to_owned(), "http://127.0.0.1:1".to_owned());
        env.insert("DAGSTER_URL".to_owned(), "http://127.0.0.1:1".to_owned());
        AppState::new(Config::from_map(&env).expect("a valid test Config"))
    }

    /// A load recorded `days_ago` days ago, holding `rows` rows afterwards.
    async fn load_at(pool: &PgPool, asset: &str, days_ago: i32, rows: i64) {
        sqlx::query(
            "INSERT INTO table_load (asset, loaded_at, rows_after) \
             VALUES ($1, now() - make_interval(days => $2), $3)",
        )
        .bind(asset)
        .bind(days_ago)
        .bind(rows)
        .execute(pool)
        .await
        .unwrap();
    }

    /// Ten loads over 18 days, each adding 1000 rows.
    async fn learned_growing_table(pool: &PgPool, asset: &str) {
        for i in 1..=10_i32 {
            load_at(pool, asset, 20 - 2 * i, i64::from(i) * 1_000).await;
        }
    }

    async fn open_incidents(pool: &PgPool) -> Vec<(String, String)> {
        list(pool, StatusFilter::Open, None)
            .await
            .unwrap()
            .into_iter()
            .map(|i| (i.asset, i.kind))
            .collect()
    }

    /// A table that has not been watched for 14 days and 5 loads raises
    /// nothing, however small the load.
    #[sqlx::test(migrations = "../../migrations")]
    async fn an_unlearned_table_raises_no_volume_incident(pool: PgPool) {
        let state = state_for(&pool);
        for i in 1..=4_i32 {
            load_at(&pool, "silver.orders", 4 - i, i64::from(i) * 1_000).await;
        }
        assert_eq!(note_load(&state, "silver.orders", 4_001).await, None);
        assert!(open_incidents(&pool).await.is_empty());
        // The load is in the history all the same.
        assert_eq!(
            quality::recent_loads(&pool, "silver.orders", 50)
                .await
                .unwrap()
                .len(),
            5
        );
    }

    /// `DATA-14-AC2`: a learned table's load of 10% of the usual opens a
    /// volume incident; a normal load does not; a second small load moves
    /// the open one instead of opening another.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_load_of_ten_percent_of_the_usual_opens_a_volume_incident(pool: PgPool) {
        let state = state_for(&pool);
        learned_growing_table(&pool, "silver.orders").await;

        assert_eq!(
            note_load(&state, "silver.orders", 11_000).await,
            None,
            "a normal load"
        );
        assert!(open_incidents(&pool).await.is_empty());

        // The next load adds 100 rows: 10% of the usual 1000.
        let opened = note_load(&state, "silver.orders", 11_100).await;
        assert!(opened.is_some());
        assert_eq!(
            open_incidents(&pool).await,
            vec![("silver.orders".to_owned(), "volume_anomaly".to_owned())]
        );
        let incident = list(&pool, StatusFilter::Open, None)
            .await
            .unwrap()
            .remove(0);
        assert!(
            incident.detail.contains("added 100 rows, 10% of the usual"),
            "{}",
            incident.detail
        );

        assert_eq!(
            note_load(&state, "silver.orders", 11_150).await,
            None,
            "already open"
        );
        assert_eq!(open_incidents(&pool).await.len(), 1);
    }

    /// A table that is replaced on each load is measured by its size.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_replaced_table_is_flagged_when_it_collapses(pool: PgPool) {
        let state = state_for(&pool);
        for (i, rows) in [5_000_i64, 5_100, 4_900, 5_000, 5_050, 4_950]
            .into_iter()
            .enumerate()
        {
            load_at(
                &pool,
                "serving.mart",
                20 - 3 * i32::try_from(i).unwrap(),
                rows,
            )
            .await;
        }
        assert!(note_load(&state, "serving.mart", 500).await.is_some());
        let incident = list(&pool, StatusFilter::Open, None)
            .await
            .unwrap()
            .remove(0);
        assert!(
            incident.detail.contains("left 500 rows"),
            "{}",
            incident.detail
        );
    }

    /// A table loaded daily that has been quiet for four days opens one
    /// freshness incident; the tick does not open another, nor one after a
    /// person resolved it; a table with too little history is left alone.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_missed_load_opens_one_freshness_incident_per_silence(pool: PgPool) {
        let state = state_for(&pool);
        // Daily loads from 14 days ago to 5 days ago, then nothing.
        for days_ago in 5..=14_i32 {
            load_at(&pool, "silver.orders", days_ago, 1_000).await;
        }
        // On time: the same history, and a load today.
        for days_ago in (5..=14_i32).chain([0]) {
            load_at(&pool, "silver.fresh", days_ago, 1_000).await;
        }
        // Not learned: three loads, the last one old.
        for days_ago in [9, 8, 7] {
            load_at(&pool, "silver.young", days_ago, 1_000).await;
        }

        tick(&state).await;
        assert_eq!(
            open_incidents(&pool).await,
            vec![("silver.orders".to_owned(), "freshness_anomaly".to_owned())]
        );
        let incident = list(&pool, StatusFilter::Open, None)
            .await
            .unwrap()
            .remove(0);
        assert!(
            incident.detail.starts_with("No load for 5d"),
            "{}",
            incident.detail
        );

        tick(&state).await;
        assert_eq!(
            open_incidents(&pool).await.len(),
            1,
            "the silence is one incident"
        );

        let resolved = quality::resolve(
            &pool,
            &incident.id,
            uuid::Uuid::from_u128(0x3333_3333_3333_4333_8333_0000_0000_0001),
            "known outage",
        )
        .await
        .unwrap();
        assert!(matches!(resolved, quality::Resolution::Resolved(_)));
        tick(&state).await;
        assert!(
            open_incidents(&pool).await.is_empty(),
            "not again for the same silence"
        );

        // A load ends the silence.
        load_at(&pool, "silver.orders", 0, 1_000).await;
        tick(&state).await;
        assert!(open_incidents(&pool).await.is_empty());
    }

    /// Loads older than ninety days are forgotten on the tick.
    #[sqlx::test(migrations = "../../migrations")]
    async fn the_tick_forgets_loads_past_ninety_days(pool: PgPool) {
        let state = state_for(&pool);
        load_at(&pool, "silver.orders", 91, 1).await;
        load_at(&pool, "silver.orders", 89, 2).await;
        tick(&state).await;
        let kept = quality::recent_loads(&pool, "silver.orders", 10)
            .await
            .unwrap();
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].rows_after, 2);
    }
}
