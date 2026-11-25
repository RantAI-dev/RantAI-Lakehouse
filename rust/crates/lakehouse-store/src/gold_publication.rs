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

use serde::Serialize;
use sqlx::FromRow;
use time::OffsetDateTime;
use uuid::Uuid;

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
        "SELECT zone, mart, enabled, updated_by, updated_at \
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
        "SELECT zone, mart, enabled, updated_by, updated_at \
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
        "SELECT zone, mart, enabled, updated_by, updated_at \
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
         RETURNING zone, mart, enabled, updated_by, updated_at",
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

#[derive(Debug, FromRow)]
struct PublicationRow {
    zone: String,
    mart: String,
    enabled: bool,
    updated_by: Uuid,
    updated_at: OffsetDateTime,
}

impl From<PublicationRow> for GoldPublication {
    fn from(row: PublicationRow) -> Self {
        Self {
            zone: row.zone,
            mart: row.mart,
            enabled: row.enabled,
            updated_by: row.updated_by,
            updated_at: iso_millis(row.updated_at),
        }
    }
}
