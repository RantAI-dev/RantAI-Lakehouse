//! Dashboard templates: saved copies of a dashboard's definition to start new
//! dashboards from (`BI-18` part D, migration `0143`, plan
//! `docs/superpowers/plans/2026-11-13-bi-18d-reports-templates.md` T3; feature
//! page `docs/core/features/reports-and-templates.md`, decisions T1 to T4).
//!
//! This module stores and answers. The routes (`lakehouse-api`'s
//! `routes::dashboard_templates`) decide who may save, list, use and delete a
//! template, build the definition from a dashboard and make a dashboard from
//! one. The definition is opaque here: a JSON object holding the dashboard
//! file (`lakehouse_bi::dashboard_file::DashboardFile`) and each chart's
//! stored document, never a row of data.
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

/// The most templates one list reads. A deployment with more is told so by
/// the route (`truncated`).
pub const LIST_LIMIT: i64 = 500;

/// A template as the list shows it: no definition.
#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct TemplateSummary {
    /// The template's id.
    pub id: Uuid,
    /// Its name.
    pub name: String,
    /// The dashboard it was saved from (it may be gone since).
    pub source_board: String,
    /// That dashboard's name when the template was saved.
    pub source_board_name: String,
    /// How many charts it holds.
    pub chart_count: i32,
    /// Display name of whoever saved it.
    pub created_by: String,
    /// When it was saved.
    pub created_at: OffsetDateTime,
}

/// A template with its definition.
#[derive(Debug, Clone, PartialEq)]
pub struct Template {
    /// The list's fields.
    pub summary: TemplateSummary,
    /// The definition (see the module doc).
    pub definition: Value,
}

#[derive(FromRow)]
struct TemplateRow {
    id: Uuid,
    name: String,
    source_board: String,
    source_board_name: String,
    chart_count: i32,
    created_by: String,
    created_at: OffsetDateTime,
    definition: Json<Value>,
}

/// What a save writes, already checked by the route.
#[derive(Debug, Clone, Copy)]
pub struct TemplateInput<'a> {
    /// The name.
    pub name: &'a str,
    /// The dashboard it is saved from.
    pub source_board: &'a str,
    /// That dashboard's name.
    pub source_board_name: &'a str,
    /// How many charts the definition holds.
    pub chart_count: i32,
    /// The definition.
    pub definition: &'a Value,
    /// Display name of whoever saves it.
    pub by: &'a str,
}

const SUMMARY_COLUMNS: &str =
    "id, name, source_board, source_board_name, chart_count, created_by, created_at";

/// The templates, newest first, at most [`LIST_LIMIT`], and whether more exist.
///
/// # Errors
///
/// [`StoreError::Database`] if the query fails.
pub async fn list(pool: &PgPool) -> Result<(Vec<TemplateSummary>, bool), StoreError> {
    // safe: only the constant `SUMMARY_COLUMNS` is formatted in; values are bound (SEC-41).
    let mut rows: Vec<TemplateSummary> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {SUMMARY_COLUMNS} FROM dashboard_template ORDER BY created_at DESC, id LIMIT $1"
    )))
    .bind(LIST_LIMIT + 1)
    .fetch_all(pool)
    .await?;
    let shown = usize::try_from(LIST_LIMIT).unwrap_or(usize::MAX);
    let truncated = rows.len() > shown;
    rows.truncate(shown);
    Ok((rows, truncated))
}

/// One template with its definition; `None` for an unknown id.
///
/// # Errors
///
/// [`StoreError::Database`] if the query fails.
pub async fn get(pool: &PgPool, id: Uuid) -> Result<Option<Template>, StoreError> {
    // safe: only the constant `SUMMARY_COLUMNS` is formatted in; values are bound (SEC-41).
    let row: Option<TemplateRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {SUMMARY_COLUMNS}, definition FROM dashboard_template WHERE id = $1"
    )))
    .bind(id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| Template {
        summary: TemplateSummary {
            id: r.id,
            name: r.name,
            source_board: r.source_board,
            source_board_name: r.source_board_name,
            chart_count: r.chart_count,
            created_by: r.created_by,
            created_at: r.created_at,
        },
        definition: r.definition.0,
    }))
}

/// Save a template.
///
/// # Errors
///
/// [`StoreError::Database`] if the insert fails.
pub async fn insert(
    pool: &PgPool,
    input: TemplateInput<'_>,
) -> Result<TemplateSummary, StoreError> {
    // safe: only the constant `SUMMARY_COLUMNS` is formatted in; values are bound (SEC-41).
    Ok(sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "INSERT INTO dashboard_template \
            (name, source_board, source_board_name, chart_count, definition, created_by) \
         VALUES ($1, $2, $3, $4, $5, $6) RETURNING {SUMMARY_COLUMNS}"
    )))
    .bind(input.name)
    .bind(input.source_board)
    .bind(input.source_board_name)
    .bind(input.chart_count)
    .bind(Json(input.definition))
    .bind(input.by)
    .fetch_one(pool)
    .await?)
}

/// Delete a template for good; `false` when there was none.
///
/// # Errors
///
/// [`StoreError::Database`] if the delete fails.
pub async fn delete(pool: &PgPool, id: Uuid) -> Result<bool, StoreError> {
    let done = sqlx::query("DELETE FROM dashboard_template WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(done.rows_affected() == 1)
}
