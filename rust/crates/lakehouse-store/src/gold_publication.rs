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
    /// The `serving.*` mart's bare name (asset id minus the `serving.`
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

/// Read one mart's publication row, or `None` when it was never enabled
/// (the "off by default" state).
///
/// # Errors
///
/// Returns [`StoreError::Database`] on any query failure.
pub async fn get(pool: &PgPool, mart: &str) -> Result<Option<GoldPublication>, StoreError> {
    let row: Option<PublicationRow> = sqlx::query_as(
        "SELECT mart, enabled, updated_by, updated_at \
           FROM gold_publication WHERE mart = $1",
    )
    .bind(mart)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(GoldPublication::from))
}

/// Read every enabled mart, ordered by name. The scheduler unions this
/// with the `GOLD_EXPORT_MARTS` env override.
///
/// # Errors
///
/// Returns [`StoreError::Database`] on any query failure.
pub async fn list_enabled(pool: &PgPool) -> Result<Vec<GoldPublication>, StoreError> {
    let rows: Vec<PublicationRow> = sqlx::query_as(
        "SELECT mart, enabled, updated_by, updated_at \
           FROM gold_publication WHERE enabled \
          ORDER BY mart",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(GoldPublication::from).collect())
}

/// Upsert one mart's switch. Idempotent per mart: `updated_by`/
/// `updated_at` always reflect the last write, so the console can show
/// who switched a mart on or off last.
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
    let row: PublicationRow = sqlx::query_as(
        "INSERT INTO gold_publication (mart, enabled, updated_by) \
         VALUES ($1, $2, $3) \
         ON CONFLICT (mart) DO UPDATE SET \
            enabled    = EXCLUDED.enabled, \
            updated_by = EXCLUDED.updated_by, \
            updated_at = now() \
         RETURNING mart, enabled, updated_by, updated_at",
    )
    .bind(mart)
    .bind(enabled)
    .bind(updated_by)
    .fetch_one(pool)
    .await?;
    Ok(GoldPublication::from(row))
}

#[derive(Debug, FromRow)]
struct PublicationRow {
    mart: String,
    enabled: bool,
    updated_by: Uuid,
    updated_at: OffsetDateTime,
}

impl From<PublicationRow> for GoldPublication {
    fn from(row: PublicationRow) -> Self {
        Self {
            mart: row.mart,
            enabled: row.enabled,
            updated_by: row.updated_by,
            updated_at: iso_millis(row.updated_at),
        }
    }
}
