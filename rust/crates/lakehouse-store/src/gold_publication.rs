//! Per-mart publish-to-`Iceberg` switches (`gold_publication`, migration
//! `0054`), the storage half of DATA-1: Gold (`serving.*`) marts are
//! copied to the `gold` Iceberg namespace only after a principal holding
//! `gold:export` enables them, and the scheduler (`GET
//! /api/gold/publications`'s caller) exports exactly the enabled rows.
//!
//! A missing row means "off", so `get` returning `None` is the disabled
//! state, not an error — the detail route renders a switchable-off
//! default from it and `list_enabled` never sees it. Switching off flips
//! `enabled` only; nothing here ever touches the Iceberg copy itself.
//!
//! DATA-65: the key is `(zone, mart)` (migration `0157`). Silver and Gold
//! publications of the same bare mart name are independent rows, in the
//! `silver` and `gold` Iceberg namespaces respectively. The `get`/`upsert`
//! helpers still mean Gold, so every pre-DATA-65 caller is unchanged;
//! zone-aware callers use `get_in_zone`/`upsert_in_zone`/
//! `list_enabled_in_zone`, and `list_enabled` returns both zones.
//!
//! The CHECK-free `BOOLEAN`/`TEXT` shape is deliberate: the mart name is
//! validated at the route (`routes::gold` reuses `export`'s
//! allowlist-shaped validation before calling [`upsert`]), and a CHECK
//! here could not express anything the route does not already know —
//! the same reasoning `0050_pipeline_sla.sql` records.
//!
//! DATA-66: each row also says how closely the Iceberg copy follows
//! `ClickHouse` ([`SyncMode`], migration `0162`): `on_update`,
//! `scheduled` at one hourly/daily slot in an IANA zone, or `manual`.
//! [`upsert_in_zone`] deliberately never touches the sync columns — a
//! switch flipped on the card must not silently reset a schedule — so
//! the setting has its own writer, [`set_sync_in_zone`]. The migration's
//! table CHECK and [`SyncSetting`]'s validator admit the same shapes,
//! and the validator's refusals are [`StoreError::Validation`] (the
//! API's 400), never a raw database sentence.

use serde::Serialize;
use sqlx::FromRow;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::report_schedule::{Schedule, zone_is_known};
use crate::{PgPool, StoreError};

/// DATA-66 D2: how a publication follows `ClickHouse`. The strings are
/// the JSON values the console sends and the API returns, exactly as
/// `0162`'s `CHECK` stores them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncMode {
    /// After each authored pipeline write, and after the watcher sees a
    /// change made outside one (D7, D8).
    OnUpdate,
    /// At one hourly/daily slot in the publication's IANA zone (D5).
    Scheduled,
    /// Only when a person presses "Publish now".
    Manual,
}

impl SyncMode {
    /// The stored and JSON spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OnUpdate => "on_update",
            Self::Scheduled => "scheduled",
            Self::Manual => "manual",
        }
    }

    /// Parse a stored/JSON spelling; `None` for anything else.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "on_update" => Some(Self::OnUpdate),
            "scheduled" => Some(Self::Scheduled),
            "manual" => Some(Self::Manual),
            _ => None,
        }
    }
}

/// The JSON view of a publication's [`Schedule`] (DATA-66 D5):
/// `{"kind":"hourly","minute":15}` or
/// `{"kind":"daily","hour":4,"minute":0}`. Separate from
/// [`crate::report_schedule::Schedule`] because only these two kinds can
/// be stored for a publication, and because that type is not `Serialize`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum SyncSchedule {
    /// Every hour at `minute` past.
    Hourly {
        /// 0 to 59.
        minute: u8,
    },
    /// Every day at `hour:minute` on the zone's wall clock.
    Daily {
        /// 0 to 23.
        hour: u8,
        /// 0 to 59.
        minute: u8,
    },
}

impl SyncSchedule {
    /// Read the setting as the schedule the runner evaluates.
    #[must_use]
    pub fn to_schedule(self) -> Schedule {
        match self {
            Self::Hourly { minute } => Schedule::Hourly { minute },
            Self::Daily { hour, minute } => Schedule::Daily { hour, minute },
        }
    }
}

impl TryFrom<Schedule> for SyncSchedule {
    type Error = ();

    /// `Err(())` for Weekly/Monthly: they are not a publication schedule
    /// (`set_sync_in_zone` refuses them before anything is stored).
    fn try_from(schedule: Schedule) -> Result<Self, Self::Error> {
        match schedule {
            Schedule::Hourly { minute } => Ok(Self::Hourly { minute }),
            Schedule::Daily { hour, minute } => Ok(Self::Daily { hour, minute }),
            Schedule::Weekly { .. } | Schedule::Monthly { .. } => Err(()),
        }
    }
}

/// One requested sync setting (DATA-66 D5), the input of
/// [`set_sync_in_zone`]. A `scheduled` mode always carries both a
/// [`Schedule`] and a known IANA zone; the other modes carry neither.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncSetting {
    /// How the publication follows `ClickHouse`.
    pub mode: SyncMode,
    /// The slot, for [`SyncMode::Scheduled`].
    pub schedule: Option<Schedule>,
    /// The IANA zone the slot is read in, for [`SyncMode::Scheduled`].
    pub time_zone: Option<String>,
}

/// Refuse a setting the migration's `CHECK` would also refuse, with a
/// fixed sentence the API answers as `400` (AGENTS.md principle 4: the
/// text is this crate's own, never a database message).
fn validate_sync_setting(zone: &str, setting: &SyncSetting) -> Result<(), StoreError> {
    if zone != "gold" && zone != "silver" {
        return Err(StoreError::Validation(
            "invalid zone: expected one of silver or gold".to_owned(),
        ));
    }
    match (setting.mode, &setting.schedule, &setting.time_zone) {
        (SyncMode::Scheduled, Some(schedule), Some(time_zone)) => {
            if !matches!(schedule, Schedule::Hourly { .. } | Schedule::Daily { .. }) {
                return Err(StoreError::Validation(
                    "schedule kind must be hourly or daily".to_owned(),
                ));
            }
            schedule
                .validate()
                .map_err(|_| StoreError::Validation("that schedule is not valid".to_owned()))?;
            if !zone_is_known(time_zone) {
                return Err(StoreError::Validation("unknown time zone".to_owned()));
            }
            Ok(())
        }
        (SyncMode::Scheduled, _, _) => Err(StoreError::Validation(
            "a scheduled sync needs a schedule and a time zone".to_owned(),
        )),
        (_, Some(_), _) | (_, _, Some(_)) => Err(StoreError::Validation(
            "only a scheduled sync has a schedule and a time zone".to_owned(),
        )),
        (_, None, None) => Ok(()),
    }
}

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

/// One row of `gold_publication`: a mart whose Iceberg copy is being
/// kept fresh. Serialized straight to the console and to the scheduler.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GoldPublication {
    /// DATA-65: the publication zone (`"silver"` or `"gold"`). A Silver
    /// publication reads `silver.<mart>` and writes `silver.<mart>` in
    /// Iceberg; a Gold one reads `serving.<mart>` and writes `gold.<mart>`.
    /// Pre-DATA-65 rows carry the default `"gold"` (migration 0157).
    pub zone: String,
    /// The mart's bare name (asset id minus the `serving.`/`silver.`
    /// prefix).
    pub mart: String,
    /// Whether the scheduler should keep the Iceberg copy fresh. `false`
    /// rows are kept (not deleted) so the detail page can show who
    /// switched a mart off last; only `true` rows reach
    /// [`list_enabled`]'s callers.
    pub enabled: bool,
    /// Who last wrote this row.
    pub updated_by: Uuid,
    /// ISO 8601 (millisecond precision, UTC).
    pub updated_at: String,
    /// DATA-66: how closely this publication follows `ClickHouse`.
    pub sync_mode: SyncMode,
    /// The slot, when [`SyncMode::Scheduled`]; `null` otherwise.
    pub schedule: Option<SyncSchedule>,
    /// The IANA zone the slot is read in, when [`SyncMode::Scheduled`];
    /// `null` otherwise.
    pub time_zone: Option<String>,
}

/// Read one `(zone, mart)` publication row, or `None` when it was never
/// enabled (the "off by default" state).
///
/// # Errors
///
/// Returns [`StoreError::Database`] on any query failure.
pub async fn get_in_zone(
    pool: &PgPool,
    zone: &str,
    mart: &str,
) -> Result<Option<GoldPublication>, StoreError> {
    let row: Option<PublicationRow> = sqlx::query_as(
        "SELECT zone, mart, enabled, updated_by, updated_at, \
                sync_mode, sync_schedule_kind, sync_hour, sync_minute, sync_time_zone \
           FROM gold_publication WHERE zone = $1 AND mart = $2",
    )
    .bind(zone)
    .bind(mart)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(GoldPublication::from))
}

/// Read one Gold mart's publication row — thin wrapper over
/// [`get_in_zone`] with `"gold"`, so every existing caller keeps its
/// behaviour (DATA-65).
///
/// # Errors
///
/// Returns [`StoreError::Database`] on any query failure.
pub async fn get(pool: &PgPool, mart: &str) -> Result<Option<GoldPublication>, StoreError> {
    get_in_zone(pool, "gold", mart).await
}

/// Read every enabled publication in `zone`, ordered by mart. The
/// scheduler unions these with the `GOLD_EXPORT_MARTS` env override.
///
/// # Errors
///
/// Returns [`StoreError::Database`] on any query failure.
pub async fn list_enabled_in_zone(
    pool: &PgPool,
    zone: &str,
) -> Result<Vec<GoldPublication>, StoreError> {
    let rows: Vec<PublicationRow> = sqlx::query_as(
        "SELECT zone, mart, enabled, updated_by, updated_at, \
                sync_mode, sync_schedule_kind, sync_hour, sync_minute, sync_time_zone \
           FROM gold_publication WHERE enabled AND zone = $1 \
          ORDER BY mart",
    )
    .bind(zone)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(GoldPublication::from).collect())
}

/// Read every enabled publication across BOTH zones, ordered by
/// `zone, mart`. The scheduler's `GET /api/gold/publications` source of
/// truth: Silver and Gold are both returned, each item carrying its own
/// `zone` (DATA-65).
///
/// # Errors
///
/// Returns [`StoreError::Database`] on any query failure.
pub async fn list_enabled(pool: &PgPool) -> Result<Vec<GoldPublication>, StoreError> {
    let rows: Vec<PublicationRow> = sqlx::query_as(
        "SELECT zone, mart, enabled, updated_by, updated_at, \
                sync_mode, sync_schedule_kind, sync_hour, sync_minute, sync_time_zone \
           FROM gold_publication WHERE enabled \
          ORDER BY zone, mart",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(GoldPublication::from).collect())
}

/// Upsert one `(zone, mart)` switch. Idempotent per pair: `updated_by`/
/// `updated_at` always reflect the last write, so the console can show
/// who switched a mart on or off last. A Silver toggle never touches the
/// Gold row for the same bare mart name (DATA-65).
///
/// # Errors
///
/// Returns [`StoreError::Database`] on any query failure.
pub async fn upsert_in_zone(
    pool: &PgPool,
    zone: &str,
    mart: &str,
    enabled: bool,
    updated_by: Uuid,
) -> Result<GoldPublication, StoreError> {
    let row: PublicationRow = sqlx::query_as(
        "INSERT INTO gold_publication (zone, mart, enabled, updated_by) \
         VALUES ($1, $2, $3, $4) \
         ON CONFLICT (zone, mart) DO UPDATE SET \
            enabled    = EXCLUDED.enabled, \
            updated_by = EXCLUDED.updated_by, \
            updated_at = now() \
         RETURNING zone, mart, enabled, updated_by, updated_at, \
                   sync_mode, sync_schedule_kind, sync_hour, sync_minute, sync_time_zone",
    )
    .bind(zone)
    .bind(mart)
    .bind(enabled)
    .bind(updated_by)
    .fetch_one(pool)
    .await?;
    Ok(GoldPublication::from(row))
}

/// Upsert one Gold mart's switch — thin wrapper over [`upsert_in_zone`]
/// with `"gold"`, so every existing caller keeps its behaviour (DATA-65).
///
/// # Errors
///
/// Returns [`StoreError::Database`] on any query failure.
pub async fn upsert(
    pool: &PgPool,
    mart: &str,
    enabled: bool,
    updated_by: Uuid,
) -> Result<GoldPublication, StoreError> {
    upsert_in_zone(pool, "gold", mart, enabled, updated_by).await
}

/// DATA-66: write one publication's sync setting. `Ok(None)` when the
/// `(zone, mart)` row does not exist — the switch was never flipped, so
/// there is no setting to change; the route answers its own 404 rather
/// than this function inventing a row.
///
/// `updated_at` moves, because a scheduled publication only fires a slot
/// at or after it (D8: a schedule set at 10:30 first fires at the next
/// slot, not the 10:00 one).
///
/// # Errors
///
/// [`StoreError::Validation`] for a zone this schema does not hold, a
/// Weekly/Monthly schedule, a schedule out of range, an unknown IANA
/// zone, or a mode that disagrees with its schedule (the same shapes
/// `0162`'s table CHECK refuses); [`StoreError::Database`] on a query
/// failure.
pub async fn set_sync_in_zone(
    pool: &PgPool,
    zone: &str,
    mart: &str,
    setting: &SyncSetting,
    updated_by: Uuid,
) -> Result<Option<GoldPublication>, StoreError> {
    validate_sync_setting(zone, setting)?;
    let (kind, hour, minute): (Option<&str>, Option<i16>, Option<i16>) = match setting.schedule {
        Some(Schedule::Hourly { minute }) => (Some("hourly"), None, Some(i16::from(minute))),
        Some(Schedule::Daily { hour, minute }) => (
            Some("daily"),
            Some(i16::from(hour)),
            Some(i16::from(minute)),
        ),
        // Weekly/Monthly were refused above, and a non-scheduled mode
        // carries no schedule; nothing to store either way.
        Some(Schedule::Weekly { .. } | Schedule::Monthly { .. }) | None => (None, None, None),
    };
    let row: Option<PublicationRow> = sqlx::query_as(
        "UPDATE gold_publication SET \
            sync_mode = $1, sync_schedule_kind = $2, sync_hour = $3, \
            sync_minute = $4, sync_time_zone = $5, \
            updated_by = $6, updated_at = now() \
          WHERE zone = $7 AND mart = $8 \
         RETURNING zone, mart, enabled, updated_by, updated_at, \
                   sync_mode, sync_schedule_kind, sync_hour, sync_minute, sync_time_zone",
    )
    .bind(setting.mode.as_str())
    .bind(kind)
    .bind(hour)
    .bind(minute)
    .bind(setting.time_zone.as_deref())
    .bind(updated_by)
    .bind(zone)
    .bind(mart)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(GoldPublication::from))
}

#[derive(Debug, FromRow)]
struct PublicationRow {
    zone: String,
    mart: String,
    enabled: bool,
    updated_by: Uuid,
    updated_at: OffsetDateTime,
    sync_mode: String,
    sync_schedule_kind: Option<String>,
    sync_hour: Option<i16>,
    sync_minute: Option<i16>,
    sync_time_zone: Option<String>,
}

impl From<PublicationRow> for GoldPublication {
    fn from(row: PublicationRow) -> Self {
        // The column's `CHECK` admits only the three spellings, so the
        // fallback is unreachable; a row that somehow held another value
        // reads as the default instead of breaking every consumer of the
        // list. `0162`'s table CHECK makes the same true of the schedule:
        // kind, minute and zone travel together, and an hourly row has no
        // hour.
        let sync_mode = SyncMode::parse(&row.sync_mode).unwrap_or(SyncMode::OnUpdate);
        let schedule = row.sync_schedule_kind.as_deref().and_then(|kind| {
            let minute = row.sync_minute?;
            Schedule::from_parts(kind, minute, row.sync_hour, None, None)
                .ok()
                .and_then(|schedule| SyncSchedule::try_from(schedule).ok())
        });
        Self {
            zone: row.zone,
            mart: row.mart,
            enabled: row.enabled,
            updated_by: row.updated_by,
            updated_at: iso_millis(row.updated_at),
            sync_mode,
            schedule,
            time_zone: row.sync_time_zone,
        }
    }
}
