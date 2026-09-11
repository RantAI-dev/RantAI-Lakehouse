//! Per-table `Iceberg` maintenance policy (migration `0030`), replacing the
//! single global sweep `dagster/dispar_orchestrate/maintenance.py` currently
//! runs against every `bronze.*` table. A table with no row in
//! `table_maintenance_policy` keeps today's default behaviour
//! (`remove_orphan_files` only).
//!
//! `table_maintenance_policy`'s own CHECK constraints are the last line of
//! defense against an invalid `namespace`/`table_name`/`schedule`/bound; the
//! future `POST /api/lakehouse/tables/{ns}/{table}/maintenance` route is
//! expected to validate first and return `400` before a request ever
//! reaches this module, so a CHECK violation here surfaces as the generic
//! [`StoreError::Database`] (see `error.rs`'s classification table) rather
//! than a dedicated variant.

use sqlx::FromRow;

use crate::{PgPool, StoreError};

/// Everything [`upsert_policy`] needs to create or replace one table's
/// maintenance policy. Mirrors [`MaintenancePolicyRow`], minus `updated_at`
/// (server-assigned).
#[derive(Debug, Clone)]
pub struct MaintenancePolicyInput {
    /// `Iceberg` namespace, e.g. `"bronze"`. Must match `^[a-z0-9_]+$`.
    pub namespace: String,
    /// `Iceberg` table name within `namespace`. Must match `^[a-z0-9_]+$`.
    pub table_name: String,
    /// Number of snapshots `expire_snapshots` should retain. `None` means
    /// "not configured" (the maintenance job keeps its own default). When
    /// set, must be at least 1: the current snapshot is always kept.
    pub snapshots_to_keep: Option<i32>,
    /// Minimum age, in hours, a file must reach before
    /// `remove_orphan_files` may delete it. When set, must be at least 1:
    /// an age of 0 could delete files an in-flight write still needs.
    pub orphan_age_hours: Option<i32>,
    /// Whether the maintenance job should also run small-file compaction
    /// for this table.
    pub compact_small_files: bool,
    /// How often the maintenance job should run for this table: `"daily"`,
    /// `"weekly"`, or `None` for "not scheduled".
    pub schedule: Option<String>,
}

/// One row of `table_maintenance_policy`. Uniquely keyed by
/// `(namespace, table_name)`.
#[derive(Debug, Clone, FromRow)]
pub struct MaintenancePolicyRow {
    /// `Iceberg` namespace, e.g. `"bronze"`.
    pub namespace: String,
    /// `Iceberg` table name within `namespace`.
    pub table_name: String,
    /// Number of snapshots `expire_snapshots` should retain, or `None` if
    /// not configured for this table.
    pub snapshots_to_keep: Option<i32>,
    /// Minimum orphan-file age, in hours, before `remove_orphan_files` may
    /// delete it, or `None` if not configured for this table.
    pub orphan_age_hours: Option<i32>,
    /// Whether small-file compaction runs for this table.
    pub compact_small_files: bool,
    /// Configured run cadence (`"daily"`/`"weekly"`), or `None`.
    pub schedule: Option<String>,
}

/// Create this table's maintenance policy, or replace it if one already
/// exists for `(input.namespace, input.table_name)`.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the insert/update fails — including
/// when `table_maintenance_policy`'s CHECK constraints refuse the row (an
/// out-of-range bound, an unrecognized `schedule`, or a `namespace`/
/// `table_name` that doesn't match `^[a-z0-9_]+$`).
pub async fn upsert_policy(
    pool: &PgPool,
    input: &MaintenancePolicyInput,
) -> Result<(), StoreError> {
    sqlx::query(
        "INSERT INTO table_maintenance_policy \
         (namespace, table_name, snapshots_to_keep, orphan_age_hours, compact_small_files, schedule, updated_at) \
         VALUES ($1, $2, $3, $4, $5, $6, now()) \
         ON CONFLICT (namespace, table_name) DO UPDATE SET \
           snapshots_to_keep = EXCLUDED.snapshots_to_keep, \
           orphan_age_hours = EXCLUDED.orphan_age_hours, \
           compact_small_files = EXCLUDED.compact_small_files, \
           schedule = EXCLUDED.schedule, \
           updated_at = now()",
    )
    .bind(&input.namespace)
    .bind(&input.table_name)
    .bind(input.snapshots_to_keep)
    .bind(input.orphan_age_hours)
    .bind(input.compact_small_files)
    .bind(&input.schedule)
    .execute(pool)
    .await?;
    Ok(())
}

/// Fetch one table's maintenance policy, if configured.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the query fails.
pub async fn get_policy(
    pool: &PgPool,
    namespace: &str,
    table_name: &str,
) -> Result<Option<MaintenancePolicyRow>, StoreError> {
    let row = sqlx::query_as(
        "SELECT namespace, table_name, snapshots_to_keep, orphan_age_hours, compact_small_files, schedule \
         FROM table_maintenance_policy WHERE namespace = $1 AND table_name = $2",
    )
    .bind(namespace)
    .bind(table_name)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// List every configured maintenance policy, ordered by `(namespace,
/// table_name)`.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the query fails.
pub async fn list_all_policies(pool: &PgPool) -> Result<Vec<MaintenancePolicyRow>, StoreError> {
    let rows = sqlx::query_as(
        "SELECT namespace, table_name, snapshots_to_keep, orphan_age_hours, compact_small_files, schedule \
         FROM table_maintenance_policy ORDER BY namespace, table_name",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}
