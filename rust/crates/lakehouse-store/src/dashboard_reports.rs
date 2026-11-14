//! Reports: written pages of text and live charts (`BI-18` part D, migration
//! `0142`, plan `docs/superpowers/plans/2026-11-13-bi-18d-reports-templates.md`
//! T1; feature page `docs/core/features/reports-and-templates.md`).
//!
//! This module stores and answers; it decides nothing about who may read or
//! write a report, and it does not check the blocks. The routes
//! (`lakehouse-api`'s `routes::dashboard_reports`) own both: who may do what
//! depends on the report's folder and its access levels, which live in
//! `ClickHouse` and in `dashboard_access_rule`, and the block rules are
//! `lakehouse_bi::reports`. What is kept here is what the database can keep:
//! one row per report, a live row has `deleted_at IS NULL`, and the trash is
//! that column.
//!
//! # Errors
//!
//! Every function returns [`StoreError::Database`] when its query fails.

use serde_json::Value;
use sqlx::types::Json;
use sqlx::{FromRow, PgPool};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::StoreError;

/// The most reports one list reads. A deployment with more is told so by the
/// route (`truncated`), never shown a silently shortened list.
pub const LIST_LIMIT: i64 = 1000;

/// A live report, blocks and all.
#[derive(Debug, Clone, PartialEq)]
pub struct Report {
    /// The report's id.
    pub id: Uuid,
    /// The dashboard folder it is filed in; `""` is the root.
    pub folder_id: String,
    /// Its title.
    pub title: String,
    /// The ordered blocks, as `lakehouse_bi::reports::Block` JSON.
    pub blocks: Value,
    /// Display name of whoever made it.
    pub created_by: String,
    /// When it was made.
    pub created_at: OffsetDateTime,
    /// Display name of whoever last saved it.
    pub updated_by: String,
    /// When it was last saved.
    pub updated_at: OffsetDateTime,
}

#[derive(FromRow)]
struct ReportRow {
    id: Uuid,
    folder_id: String,
    title: String,
    blocks: Json<Value>,
    created_by: String,
    created_at: OffsetDateTime,
    updated_by: String,
    updated_at: OffsetDateTime,
}

impl From<ReportRow> for Report {
    fn from(row: ReportRow) -> Self {
        Self {
            id: row.id,
            folder_id: row.folder_id,
            title: row.title,
            blocks: row.blocks.0,
            created_by: row.created_by,
            created_at: row.created_at,
            updated_by: row.updated_by,
            updated_at: row.updated_at,
        }
    }
}

/// A live report as the list shows it: no blocks, their counts.
#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct ReportSummary {
    /// The report's id.
    pub id: Uuid,
    /// The dashboard folder it is filed in; `""` is the root.
    pub folder_id: String,
    /// Its title.
    pub title: String,
    /// Display name of whoever made it.
    pub created_by: String,
    /// Display name of whoever last saved it.
    pub updated_by: String,
    /// When it was last saved.
    pub updated_at: OffsetDateTime,
    /// How many blocks it holds.
    pub block_count: i64,
    /// How many of them are charts.
    pub chart_count: i64,
}

/// A report in the trash.
#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct TrashedReport {
    /// The report's id.
    pub id: Uuid,
    /// The folder it comes back into.
    pub folder_id: String,
    /// Its title.
    pub title: String,
    /// Display name of whoever deleted it.
    pub deleted_by: String,
    /// When it was deleted.
    pub deleted_at: OffsetDateTime,
}

/// What a save writes: the whole report, already checked by the route.
#[derive(Debug, Clone, Copy)]
pub struct ReportInput<'a> {
    /// The folder; `""` is the root.
    pub folder_id: &'a str,
    /// The title.
    pub title: &'a str,
    /// The ordered blocks.
    pub blocks: &'a Value,
    /// Display name of whoever saves it.
    pub by: &'a str,
}

const REPORT_COLUMNS: &str =
    "id, folder_id, title, blocks, created_by, created_at, updated_by, updated_at";

/// The live reports, most recently saved first, at most [`LIST_LIMIT`] of
/// them, and whether more exist that are not returned.
///
/// # Errors
///
/// [`StoreError::Database`] if the query fails.
pub async fn list_live(pool: &PgPool) -> Result<(Vec<ReportSummary>, bool), StoreError> {
    let mut rows: Vec<ReportSummary> = sqlx::query_as(
        "SELECT id, folder_id, title, created_by, updated_by, updated_at, \
                jsonb_array_length(blocks)::bigint AS block_count, \
                (SELECT count(*) FROM jsonb_array_elements(blocks) b \
                  WHERE b->>'type' = 'chart')::bigint AS chart_count \
           FROM dashboard_report WHERE deleted_at IS NULL \
          ORDER BY updated_at DESC, id LIMIT $1",
    )
    .bind(LIST_LIMIT + 1)
    .fetch_all(pool)
    .await?;
    let shown = usize::try_from(LIST_LIMIT).unwrap_or(usize::MAX);
    let truncated = rows.len() > shown;
    rows.truncate(shown);
    Ok((rows, truncated))
}

/// One live report; a deleted or unknown id is `None`.
///
/// # Errors
///
/// [`StoreError::Database`] if the query fails.
pub async fn get_live(pool: &PgPool, id: Uuid) -> Result<Option<Report>, StoreError> {
    // safe: only the constant `REPORT_COLUMNS` is formatted in; values are bound (SEC-41).
    let row: Option<ReportRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {REPORT_COLUMNS} FROM dashboard_report WHERE id = $1 AND deleted_at IS NULL"
    )))
    .bind(id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(Report::from))
}

/// Write a new report.
///
/// # Errors
///
/// [`StoreError::Database`] if the insert fails (a `CHECK` included: the
/// route checks the same limits first).
pub async fn insert(pool: &PgPool, input: ReportInput<'_>) -> Result<Report, StoreError> {
    // safe: only the constant `REPORT_COLUMNS` is formatted in; values are bound (SEC-41).
    let row: ReportRow = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "INSERT INTO dashboard_report (folder_id, title, blocks, created_by, updated_by) \
         VALUES ($1, $2, $3, $4, $4) RETURNING {REPORT_COLUMNS}"
    )))
    .bind(input.folder_id)
    .bind(input.title)
    .bind(Json(input.blocks))
    .bind(input.by)
    .fetch_one(pool)
    .await?;
    Ok(row.into())
}

/// Replace a live report's folder, title and blocks. `None` when it is not
/// live (unknown, or in the trash): nothing was written.
///
/// # Errors
///
/// [`StoreError::Database`] if the update fails.
pub async fn update(
    pool: &PgPool,
    id: Uuid,
    input: ReportInput<'_>,
) -> Result<Option<Report>, StoreError> {
    // safe: only the constant `REPORT_COLUMNS` is formatted in; values are bound (SEC-41).
    let row: Option<ReportRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "UPDATE dashboard_report \
            SET folder_id = $2, title = $3, blocks = $4, updated_by = $5, updated_at = now() \
          WHERE id = $1 AND deleted_at IS NULL RETURNING {REPORT_COLUMNS}"
    )))
    .bind(id)
    .bind(input.folder_id)
    .bind(input.title)
    .bind(Json(input.blocks))
    .bind(input.by)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(Report::from))
}

/// Move a live report to the trash. `false` when it was not live.
///
/// # Errors
///
/// [`StoreError::Database`] if the update fails.
pub async fn trash(pool: &PgPool, id: Uuid, by: &str) -> Result<bool, StoreError> {
    let done = sqlx::query(
        "UPDATE dashboard_report SET deleted_at = now(), deleted_by = $2 \
          WHERE id = $1 AND deleted_at IS NULL",
    )
    .bind(id)
    .bind(by)
    .execute(pool)
    .await?;
    Ok(done.rows_affected() == 1)
}

/// The reports in the trash, most recently deleted first, at most
/// [`LIST_LIMIT`].
///
/// # Errors
///
/// [`StoreError::Database`] if the query fails.
pub async fn list_trashed(pool: &PgPool) -> Result<Vec<TrashedReport>, StoreError> {
    Ok(sqlx::query_as(
        "SELECT id, folder_id, title, deleted_by, deleted_at FROM dashboard_report \
          WHERE deleted_at IS NOT NULL ORDER BY deleted_at DESC, id LIMIT $1",
    )
    .bind(LIST_LIMIT)
    .fetch_all(pool)
    .await?)
}

/// One report in the trash; `None` when it is not there.
///
/// # Errors
///
/// [`StoreError::Database`] if the query fails.
pub async fn get_trashed(pool: &PgPool, id: Uuid) -> Result<Option<TrashedReport>, StoreError> {
    Ok(sqlx::query_as(
        "SELECT id, folder_id, title, deleted_by, deleted_at FROM dashboard_report \
          WHERE id = $1 AND deleted_at IS NOT NULL",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?)
}

/// Bring a report back from the trash into `folder_id` (its own, or the root
/// when that folder is gone). `false` when it was not in the trash.
///
/// # Errors
///
/// [`StoreError::Database`] if the update fails.
pub async fn restore(pool: &PgPool, id: Uuid, folder_id: &str) -> Result<bool, StoreError> {
    let done = sqlx::query(
        "UPDATE dashboard_report SET deleted_at = NULL, deleted_by = '', folder_id = $2 \
          WHERE id = $1 AND deleted_at IS NOT NULL",
    )
    .bind(id)
    .bind(folder_id)
    .execute(pool)
    .await?;
    Ok(done.rows_affected() == 1)
}

/// Remove for good the reports that have been in the trash longer than
/// `days`; how many were removed.
///
/// # Errors
///
/// [`StoreError::Database`] if the delete fails.
pub async fn purge_expired(pool: &PgPool, days: u32) -> Result<u64, StoreError> {
    let done = sqlx::query(
        "DELETE FROM dashboard_report \
          WHERE deleted_at IS NOT NULL AND deleted_at < now() - make_interval(days => $1)",
    )
    .bind(i32::try_from(days).unwrap_or(i32::MAX))
    .execute(pool)
    .await?;
    Ok(done.rows_affected())
}

/// How many reports name `folder_id`, live or in the trash (a report in the
/// trash comes back into its folder, so it counts).
///
/// # Errors
///
/// [`StoreError::Database`] if the query fails.
pub async fn count_in_folder(pool: &PgPool, folder_id: &str) -> Result<i64, StoreError> {
    Ok(
        sqlx::query_scalar("SELECT count(*) FROM dashboard_report WHERE folder_id = $1")
            .bind(folder_id)
            .fetch_one(pool)
            .await?,
    )
}
