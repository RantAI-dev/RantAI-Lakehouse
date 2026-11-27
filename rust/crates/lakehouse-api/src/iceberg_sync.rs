//! `DATA-66` D7/D8: the per-table Iceberg sync — what is due right now,
//! and the run it becomes.
//!
//! `gold_export_job` exported **every** enabled publication after any
//! authored pipeline and nightly at 04:00, so one table waited behind
//! all the others and a change made outside a pipeline, at any other
//! time, was picked up only by the night. `DATA-66` moves the decision
//! to this module and the launch to one small job per `(zone, mart)`.
//!
//! # Two triggers, one place
//!
//! - **D8, the watcher**: `GET /api/gold/sync-due` reads the enabled
//!   publications, one cheap `system.parts` signal per table (M4: the
//!   `DATA-10` fingerprint is a full scan and must not run every 15 s)
//!   and one `console.gold_export_run` read, then answers the runs that
//!   are due with a deterministic `runKey`. Dagster's
//!   `iceberg_sync_sensor` turns each key into one `RunRequest`; a key it
//!   has seen starts nothing, so one change signal starts at most one run.
//! - **D7, the pipeline**: after its write, the authored op POSTs
//!   `after-write` and the API launches the same job for that one table.
//!
//! Both start [`ICEBERG_SYNC_JOB`], whose op POSTs the existing export
//! route with `ifChanged=true` and then trims — so `DATA-2`, `DATA-6`,
//! `DATA-63`, `DATA-64` and `SEC-11` hold by construction, and the
//! content fingerprint still makes the exact "did it change" decision.
//! A background merge can move `modification_time` and start a run that
//! then skips; the run key keeps that to one run per signal.
//!
//! The pure decision ([`due_syncs`]) has no I/O, so every rule in D8 is
//! unit-tested without a server. The two reads are here too, so the
//! route and its test share one SQL owner.

use std::collections::HashMap;

use lakehouse_clickhouse::{ChClient, ChError};
use lakehouse_core::ident::SqlLiteral;
use lakehouse_iceberg::gold::PublishZone;
use lakehouse_store::gold_publication::{GoldPublication, SyncMode, SyncSchedule};
use lakehouse_store::report_schedule::Schedule;
use serde::Serialize;
use serde_json::{Value, json};
use time::OffsetDateTime;

use crate::gold_export_history;

/// The Dagster job that exports and trims ONE publication (D6, D12).
/// Mirrored as the constant in `dagster/dispar_orchestrate/iceberg_sync.py`,
/// like `gold_latest_copy::GOLD_LATEST_COPY_JOB`.
pub const ICEBERG_SYNC_JOB: &str = "iceberg_sync_job";

/// The op inside [`ICEBERG_SYNC_JOB`] the run config addresses (D12).
pub const ICEBERG_SYNC_TARGET_OP: &str = "iceberg_sync_target";

/// D6: `dagster/dagster.yaml` limits this tag to one run per unique
/// value, so two tables sync in parallel and two syncs of one table
/// queue (never dropped). Deliberately not `lakehouse/trigger`, which the
/// run report reads with other values (D12).
pub const ICEBERG_SYNC_TARGET_TAG: &str = "lakehouse/iceberg-sync-target";

/// D12: how this run was asked for: `pipeline`, `change`, `retry` or
/// `schedule`. A separate key from [`ICEBERG_SYNC_TARGET_TAG`] because a
/// tag value is one string and the run report's trigger tag has its own
/// vocabulary.
pub const ICEBERG_SYNC_TRIGGER_TAG: &str = "lakehouse/iceberg-sync-trigger";

/// D7: the authored op asked right after its write.
pub const TRIGGER_PIPELINE: &str = "pipeline";
/// D8: the table's `max(modification_time)` moved past its newest run.
pub const TRIGGER_CHANGE: &str = "change";
/// D8: the newest run failed and the backoff has passed.
pub const TRIGGER_RETRY: &str = "retry";
/// D8: a scheduled publication reached a slot.
pub const TRIGGER_SCHEDULE: &str = "schedule";

/// D8 (feature page decision 9): how long a failed publication waits
/// before the watcher starts its retry run.
pub const RETRY_BACKOFF_SECONDS: i64 = 300;

/// One enabled publication, as the due decision reads it (D8). Built
/// from the stored row, with `updated_at` parsed once: a schedule only
/// fires a slot at or after the moment it was set, so a schedule saved
/// at 10:30 does not fire for the 10:00 slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DuePublication {
    /// The Iceberg namespace name (`"silver"`/`"gold"`), which is also
    /// the history table's `zone` value.
    pub zone: String,
    /// The bare mart name.
    pub mart: String,
    /// The row's switch. The route reads enabled rows only, and the
    /// decision skips a disabled one anyway (D8's "per enabled
    /// publication").
    pub enabled: bool,
    /// How the publication follows `ClickHouse`.
    pub mode: SyncMode,
    /// The slot, for [`SyncMode::Scheduled`].
    pub schedule: Option<Schedule>,
    /// The IANA zone the slot is read in.
    pub time_zone: Option<String>,
    /// When the setting was last written; `None` when the stored string
    /// cannot be parsed (it always can, but a bad row must not be read as
    /// the epoch).
    pub updated_at: Option<OffsetDateTime>,
}

impl DuePublication {
    /// Read one stored row into the decision's shape.
    #[must_use]
    pub fn from_publication(row: &GoldPublication) -> Self {
        Self {
            zone: row.zone.clone(),
            mart: row.mart.clone(),
            enabled: row.enabled,
            mode: row.sync_mode,
            schedule: row
                .schedule
                .map(SyncSchedule::to_schedule)
                .filter(|schedule| {
                    matches!(schedule, Schedule::Hourly { .. } | Schedule::Daily { .. })
                }),
            time_zone: row.time_zone.clone(),
            updated_at: OffsetDateTime::parse(
                &row.updated_at,
                &time::format_description::well_known::Rfc3339,
            )
            .ok(),
        }
    }
}

/// The newest `console.gold_export_run` row of one publication (D8): the
/// change rule compares the part time against `started_at`, and the
/// retry rule needs the outcome and `finished_at`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewestRun {
    /// `"success"` or `"failed"`, as stored.
    pub status: String,
    /// When the run started.
    pub started_at: OffsetDateTime,
    /// When the run finished.
    pub finished_at: OffsetDateTime,
}

/// One run that should start now (D8): the job config's target and the
/// deterministic key that makes Dagster start it at most once.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DueSync {
    /// The Iceberg namespace name (`"silver"`/`"gold"`).
    pub zone: String,
    /// The bare mart name.
    pub mart: String,
    /// One of the `TRIGGER_*` values.
    pub trigger: String,
    /// `change:<zone>:<mart>:<unix seconds>`, `retry:<zone>:<mart>:<unix
    /// ms>` or `schedule:<zone>:<mart>:<unix seconds>`.
    pub run_key: String,
}

/// The runs due at `now`, by D8's rules, in the publications' order:
///
/// - `manual`: never due.
/// - `on_update`: not due without an active part. Due with trigger
///   `change` when there is no newest run, or the part time is at or
///   after its start (both at the signal's whole-second precision, R2);
///   otherwise due with `retry` when that newest run
///   failed and at least [`RETRY_BACKOFF_SECONDS`] have passed since it
///   finished.
/// - `scheduled`: due at the latest slot at or before `now` when the
///   slot is at or after the setting's `updated_at`.
// The two maps are the caller's own reads (std `HashMap`, default
// SipHash); generalizing over `BuildHasher` would add a type parameter
// for a capability nothing here uses (the same reasoning
// `pipeline_lag::upstream_stops_for_list` records).
#[allow(
    clippy::implicit_hasher,
    reason = "the maps are always this module's own std HashMap reads"
)]
#[must_use]
pub fn due_syncs(
    now: OffsetDateTime,
    publications: &[DuePublication],
    changes: &HashMap<(String, String), OffsetDateTime>,
    newest_runs: &HashMap<(String, String), NewestRun>,
) -> Vec<DueSync> {
    let mut due = Vec::new();
    for publication in publications {
        if !publication.enabled {
            continue;
        }
        let key = (publication.zone.clone(), publication.mart.clone());
        match publication.mode {
            SyncMode::Manual => {}
            SyncMode::OnUpdate => {
                // M4/D8: no active part means nothing to publish, so the
                // watcher stays quiet rather than starting a run that
                // would find an empty table.
                let Some(part_changed_at) = changes.get(&key) else {
                    continue;
                };
                let newest = newest_runs.get(&key);
                // DATA-66 review R2: `active_part_changes` reads
                // `modification_time` in WHOLE seconds, so the signal
                // cannot tell 10:00:00.500 from 10:00:00.900. Comparing
                // that second with a millisecond `started_at` missed a
                // write made in the same second a run started (a run at
                // .500 and a write at .900 both read 10:00:00, and
                // 10:00:00 < 10:00:00.500). Truncate the run's start to
                // the signal's own precision instead: the cost is at most
                // one extra run per signal, which the export's `ifChanged`
                // fingerprint skips without writing.
                let changed_is_new = match newest {
                    None => true,
                    Some(run) => {
                        let started_at = run
                            .started_at
                            .replace_millisecond(0)
                            .unwrap_or(run.started_at);
                        *part_changed_at >= started_at
                    }
                };
                if changed_is_new {
                    due.push(DueSync {
                        zone: publication.zone.clone(),
                        mart: publication.mart.clone(),
                        trigger: TRIGGER_CHANGE.to_owned(),
                        run_key: format!(
                            "change:{}:{}:{}",
                            publication.zone,
                            publication.mart,
                            part_changed_at.unix_timestamp()
                        ),
                    });
                    continue;
                }
                let Some(newest) = newest else {
                    continue;
                };
                if newest.status == "failed"
                    && now - newest.finished_at >= time::Duration::seconds(RETRY_BACKOFF_SECONDS)
                {
                    due.push(DueSync {
                        zone: publication.zone.clone(),
                        mart: publication.mart.clone(),
                        trigger: TRIGGER_RETRY.to_owned(),
                        run_key: format!(
                            "retry:{}:{}:{}",
                            publication.zone,
                            publication.mart,
                            epoch_millis(newest.finished_at)
                        ),
                    });
                }
            }
            SyncMode::Scheduled => {
                let (Some(schedule), Some(time_zone), Some(updated_at)) = (
                    publication.schedule,
                    publication.time_zone.as_deref(),
                    publication.updated_at,
                ) else {
                    // A scheduled row always has all three (the store and
                    // the migration CHECK refuse anything else); without
                    // them there is no slot that can be called due.
                    continue;
                };
                let Ok(slot) = schedule.last_run_at_or_before(time_zone, now) else {
                    continue;
                };
                if slot >= updated_at {
                    due.push(DueSync {
                        zone: publication.zone.clone(),
                        mart: publication.mart.clone(),
                        trigger: TRIGGER_SCHEDULE.to_owned(),
                        run_key: format!(
                            "schedule:{}:{}:{}",
                            publication.zone,
                            publication.mart,
                            slot.unix_timestamp()
                        ),
                    });
                }
            }
        }
    }
    due
}

/// `at` as Unix-milliseconds, the unit the retry key keeps so two
/// failures a second apart are two keys.
fn epoch_millis(at: OffsetDateTime) -> i64 {
    at.unix_timestamp() * 1_000 + i64::from(at.millisecond())
}

/// D8/M4: the cheap change signal — `max(modification_time)` over the
/// active parts of every table in both source schemas, one query,
/// keyed by `(zone namespace, table)`. A table with no active parts
/// (a view, an engine without parts) has no entry: "nothing to
/// publish", never the epoch.
///
/// # Errors
///
/// Returns [`ChError`] when the query fails; the route classifies it,
/// never surfacing the engine's text (`SEC-11`).
pub async fn active_part_changes(
    ch: &ChClient,
    gold_source_schema: &str,
) -> Result<HashMap<(String, String), OffsetDateTime>, ChError> {
    let silver_schema = PublishZone::Silver.source_ch_schema(gold_source_schema);
    let gold_schema = PublishZone::Gold.source_ch_schema(gold_source_schema);
    // Schema names are deployment configuration, never caller text, and
    // SqlLiteral neutralises a quote either way; the table names never
    // reach the statement (they are grouped, not filtered).
    let sql = format!(
        "SELECT database, table, \
                toString(toUnixTimestamp(max(modification_time))) AS changed_s \
           FROM system.parts \
          WHERE active AND database IN ({silver}, {gold}) \
          GROUP BY database, table FORMAT JSON",
        silver = SqlLiteral::from(silver_schema),
        gold = SqlLiteral::from(gold_schema),
    );
    let rows = ch.rows(&sql, None).await?;
    let mut changes = HashMap::new();
    for row in &rows {
        let (Some(database), Some(table)) = (
            row.get("database").and_then(Value::as_str),
            row.get("table").and_then(Value::as_str),
        ) else {
            continue;
        };
        let Some(part_changed_at) = row
            .get("changed_s")
            .and_then(Value::as_str)
            .and_then(|seconds| seconds.parse::<i64>().ok())
            .and_then(|seconds| OffsetDateTime::from_unix_timestamp(seconds).ok())
        else {
            continue;
        };
        // One row can serve both zones when an unusual deployment points
        // the Gold source schema at `silver`; each zone's publication
        // still gets its own key.
        for zone in [PublishZone::Silver, PublishZone::Gold] {
            if zone.source_ch_schema(gold_source_schema) == database {
                changes.insert(
                    (zone.namespace_name().to_owned(), table.to_owned()),
                    part_changed_at,
                );
            }
        }
    }
    Ok(changes)
}

/// D8: the newest `console.gold_export_run` row per `(zone, mart)`, one
/// query. The table is ensured first, exactly as every other reader of
/// it does.
///
/// # Errors
///
/// Returns [`ChError`] when the table cannot be ensured or the query
/// fails; the route classifies it (`SEC-11`).
pub async fn newest_runs(ch: &ChClient) -> Result<HashMap<(String, String), NewestRun>, ChError> {
    gold_export_history::ensure_gold_export_run_table(ch).await?;
    let sql = "SELECT zone, mart, \
                      argMax(status, started_at) AS status, \
                      toString(toUnixTimestamp64Milli(argMax(started_at, started_at))) AS started_ms, \
                      toString(toUnixTimestamp64Milli(argMax(finished_at, started_at))) AS finished_ms \
                 FROM console.gold_export_run \
                GROUP BY zone, mart FORMAT JSON";
    let rows = ch.rows(sql, None).await?;
    let mut newest = HashMap::new();
    for row in &rows {
        let (Some(zone), Some(mart), Some(status)) = (
            row.get("zone").and_then(Value::as_str),
            row.get("mart").and_then(Value::as_str),
            row.get("status").and_then(Value::as_str),
        ) else {
            continue;
        };
        let started_at = row
            .get("started_ms")
            .and_then(Value::as_str)
            .and_then(|ms| ms.parse::<i64>().ok())
            .and_then(millis_to_offset);
        let finished_at = row
            .get("finished_ms")
            .and_then(Value::as_str)
            .and_then(|ms| ms.parse::<i64>().ok())
            .and_then(millis_to_offset);
        let (Some(started_at), Some(finished_at)) = (started_at, finished_at) else {
            continue;
        };
        newest.insert(
            (zone.to_owned(), mart.to_owned()),
            NewestRun {
                status: status.to_owned(),
                started_at,
                finished_at,
            },
        );
    }
    Ok(newest)
}

fn millis_to_offset(ms: i64) -> Option<OffsetDateTime> {
    OffsetDateTime::from_unix_timestamp_nanos(i128::from(ms) * 1_000_000).ok()
}

/// D6/D12: the run config [`ICEBERG_SYNC_JOB`] expects — the one op,
/// with the publication and the trigger that asked for it. The route and
/// its test share this, so a config the op refuses cannot be shipped
/// here.
#[must_use]
pub fn run_config(zone: PublishZone, mart: &str, trigger: &str) -> Value {
    json!({
        "ops": {
            ICEBERG_SYNC_TARGET_OP: {
                "config": {
                    "zone": zone.namespace_name(),
                    "mart": mart,
                    "trigger": trigger,
                }
            }
        }
    })
}

/// D6: the value of [`ICEBERG_SYNC_TARGET_TAG`] — the Iceberg table name,
/// so `dagster/dagster.yaml`'s per-unique-value limit queues two syncs of
/// one table behind each other and lets two tables run together.
#[must_use]
pub fn target_tag_value(zone: PublishZone, mart: &str) -> String {
    format!("{}.{}", zone.namespace_name(), mart)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn utc(text: &str) -> OffsetDateTime {
        OffsetDateTime::parse(text, &time::format_description::well_known::Rfc3339).unwrap()
    }

    fn publication(mode: SyncMode) -> DuePublication {
        DuePublication {
            zone: "gold".to_owned(),
            mart: "orders".to_owned(),
            enabled: true,
            mode,
            schedule: None,
            time_zone: None,
            updated_at: Some(utc("2026-11-27T00:00:00Z")),
        }
    }

    fn key() -> (String, String) {
        ("gold".to_owned(), "orders".to_owned())
    }

    fn changes(changed: &str) -> HashMap<(String, String), OffsetDateTime> {
        HashMap::from([(key(), utc(changed))])
    }

    fn newest(status: &str, started: &str, finished: &str) -> HashMap<(String, String), NewestRun> {
        HashMap::from([(
            key(),
            NewestRun {
                status: status.to_owned(),
                started_at: utc(started),
                finished_at: utc(finished),
            },
        )])
    }

    #[test]
    fn a_manual_publication_is_never_due() {
        let due = due_syncs(
            utc("2026-11-27T10:00:00Z"),
            &[publication(SyncMode::Manual)],
            &changes("2026-11-27T09:59:00Z"),
            &newest("failed", "2026-11-27T08:00:00Z", "2026-11-27T08:05:00Z"),
        );
        assert_eq!(due, Vec::<DueSync>::new());
    }

    #[test]
    fn a_disabled_publication_is_never_due() {
        let mut row = publication(SyncMode::OnUpdate);
        row.enabled = false;
        let due = due_syncs(
            utc("2026-11-27T10:00:00Z"),
            &[row],
            &changes("2026-11-27T09:59:00Z"),
            &HashMap::new(),
        );
        assert_eq!(due, Vec::<DueSync>::new());
    }

    #[test]
    fn an_on_update_publication_with_no_run_yet_is_due_with_the_change_key() {
        let due = due_syncs(
            utc("2026-11-27T10:00:00Z"),
            &[publication(SyncMode::OnUpdate)],
            &changes("2026-11-27T09:59:00Z"),
            &HashMap::new(),
        );
        assert_eq!(
            due,
            vec![DueSync {
                zone: "gold".to_owned(),
                mart: "orders".to_owned(),
                trigger: TRIGGER_CHANGE.to_owned(),
                run_key: "change:gold:orders:1795773540".to_owned(),
            }]
        );
    }

    #[test]
    fn a_change_after_the_newest_start_is_due_and_an_older_one_is_not() {
        let row = publication(SyncMode::OnUpdate);
        let runs = newest("success", "2026-11-27T09:00:00Z", "2026-11-27T09:01:00Z");
        let newer = due_syncs(
            utc("2026-11-27T10:00:00Z"),
            std::slice::from_ref(&row),
            &changes("2026-11-27T09:30:00Z"),
            &runs,
        );
        assert_eq!(newer.len(), 1);
        assert_eq!(newer[0].trigger, TRIGGER_CHANGE);
        assert_eq!(newer[0].run_key, "change:gold:orders:1795771800");

        let older = due_syncs(
            utc("2026-11-27T10:00:00Z"),
            &[row],
            &changes("2026-11-27T08:30:00Z"),
            &runs,
        );
        assert_eq!(older, Vec::<DueSync>::new(), "an unchanged table waits");
    }

    #[test]
    fn a_write_in_the_same_second_as_a_run_start_is_due() {
        // DATA-66 review R2: a run starting at 10:00:00.500 and a write at
        // 10:00:00.900. `active_part_changes` gives the write the whole
        // second 10:00:00, so a millisecond comparison would call it old
        // and the change would wait for the next write.
        let row = publication(SyncMode::OnUpdate);
        let runs = newest(
            "success",
            "2026-11-27T10:00:00.500Z",
            "2026-11-27T10:00:05Z",
        );
        let due = due_syncs(
            utc("2026-11-27T10:00:10Z"),
            &[row],
            &changes("2026-11-27T10:00:00Z"),
            &runs,
        );
        assert_eq!(due.len(), 1, "the write in the run's own second is due");
        assert_eq!(due[0].trigger, TRIGGER_CHANGE);
        assert_eq!(due[0].run_key, "change:gold:orders:1795773600");
    }

    #[test]
    fn a_failed_newest_run_retries_after_the_backoff_and_not_before() {
        let row = publication(SyncMode::OnUpdate);
        let runs = newest("failed", "2026-11-27T09:00:00Z", "2026-11-27T09:05:00Z");
        let unchanged = changes("2026-11-27T08:00:00Z");

        let too_soon = due_syncs(
            utc("2026-11-27T09:09:59Z"),
            std::slice::from_ref(&row),
            &unchanged,
            &runs,
        );
        assert_eq!(
            too_soon,
            Vec::<DueSync>::new(),
            "299 s is inside the backoff"
        );

        let due = due_syncs(utc("2026-11-27T09:10:00Z"), &[row], &unchanged, &runs);
        assert_eq!(
            due,
            vec![DueSync {
                zone: "gold".to_owned(),
                mart: "orders".to_owned(),
                trigger: TRIGGER_RETRY.to_owned(),
                run_key: "retry:gold:orders:1795770300000".to_owned(),
            }]
        );
    }

    #[test]
    fn a_failed_run_with_a_newer_change_uses_the_change_trigger_not_the_retry() {
        let due = due_syncs(
            utc("2026-11-27T10:00:00Z"),
            &[publication(SyncMode::OnUpdate)],
            &changes("2026-11-27T09:59:00Z"),
            &newest("failed", "2026-11-27T09:00:00Z", "2026-11-27T09:05:00Z"),
        );
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].trigger, TRIGGER_CHANGE);
    }

    #[test]
    fn a_table_with_no_active_parts_is_not_due() {
        let due = due_syncs(
            utc("2026-11-27T10:00:00Z"),
            &[publication(SyncMode::OnUpdate)],
            &HashMap::new(),
            &HashMap::new(),
        );
        assert_eq!(due, Vec::<DueSync>::new());
    }

    #[test]
    fn a_scheduled_publication_is_due_from_the_slot_its_setting_was_written() {
        let mut row = publication(SyncMode::Scheduled);
        row.schedule = Some(Schedule::Daily { hour: 4, minute: 0 });
        row.time_zone = Some("Asia/Jakarta".to_owned());
        // Set at 00:30Z on the 26th: the 26th's 04:00 Jakarta slot is
        // 21:00Z on the 25th, before the setting, so it does not fire.
        row.updated_at = Some(utc("2026-11-26T00:30:00Z"));
        let not_yet = due_syncs(
            utc("2026-11-26T10:00:00Z"),
            std::slice::from_ref(&row),
            &HashMap::new(),
            &HashMap::new(),
        );
        assert!(
            not_yet.is_empty(),
            "the 26th's 04:00 slot is before the setting"
        );

        // Move the setting before that slot: it becomes due with the
        // slot's own key.
        row.updated_at = Some(utc("2026-11-25T00:00:00Z"));
        let due = due_syncs(
            utc("2026-11-26T10:00:00Z"),
            &[row],
            &HashMap::new(),
            &HashMap::new(),
        );
        assert_eq!(
            due,
            vec![DueSync {
                zone: "gold".to_owned(),
                mart: "orders".to_owned(),
                trigger: TRIGGER_SCHEDULE.to_owned(),
                run_key: "schedule:gold:orders:1795640400".to_owned(),
            }]
        );
    }

    #[test]
    fn a_scheduled_publication_at_or_after_its_setting_is_due() {
        let mut row = publication(SyncMode::Scheduled);
        row.schedule = Some(Schedule::Hourly { minute: 15 });
        row.time_zone = Some("UTC".to_owned());
        // The 10:15 slot was due exactly when the setting was written.
        row.updated_at = Some(utc("2026-11-27T10:15:00Z"));
        let due = due_syncs(
            utc("2026-11-27T10:20:00Z"),
            &[row],
            &HashMap::new(),
            &HashMap::new(),
        );
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].trigger, TRIGGER_SCHEDULE);
    }

    #[test]
    fn the_change_signal_is_grouped_both_ways_per_zone() {
        assert_eq!(PublishZone::Silver.source_ch_schema("serving"), "silver");
        assert_eq!(PublishZone::Gold.source_ch_schema("serving"), "serving");
        assert_eq!(target_tag_value(PublishZone::Gold, "orders"), "gold.orders");
        assert_eq!(
            target_tag_value(PublishZone::Silver, "orders"),
            "silver.orders"
        );
    }

    #[test]
    fn the_run_config_addresses_the_one_op_with_both_the_zone_and_the_trigger() {
        let config = run_config(PublishZone::Silver, "orders", TRIGGER_CHANGE);
        assert_eq!(
            config,
            json!({
                "ops": {
                    "iceberg_sync_target": {
                        "config": {
                            "zone": "silver",
                            "mart": "orders",
                            "trigger": "change"
                        }
                    }
                }
            })
        );
    }
}
