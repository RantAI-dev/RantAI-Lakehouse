//! Dashboard SQL sources — saved, user-authored `SELECT`s (typically joining
//! several `serving` marts) that a chart reads instead of a single mart.
//!
//! Storage only. Whether a statement may become a source at all is decided
//! by the API before anything reaches this module (`lakehouse-api`'s
//! `sql_guard::check_sql_source`: read-only, `serving.*` tables only, no
//! `;`/`SETTINGS`/`FORMAT`), and every execution goes through the API's
//! policy rewrite; see `docs/plans/DASHBOARD-SQL-SOURCES-FOLDERS-PLAN.md`.
//!
//! Stored in `console.bi_source`, next to `bi_chart`/`bi_board` and with the
//! same versioned-insert semantics (see [`crate::store::ensure_bi_table`]):
//! a save is an `INSERT`, the newest `created_at` wins under `FINAL`, and a
//! delete is an `is_deleted = 1` tombstone. `columns_json` is the column list
//! probed with `DESCRIBE` when the source was saved; chart validation and
//! dashboard filters read it instead of re-running the source.

use std::collections::HashSet;

use lakehouse_clickhouse::{ChClient, ChError};
use lakehouse_core::ident::SqlLiteral;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::store::{StoredChartSpec, ensure_bi_table, random_hex};

/// One column a SQL source returns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceColumn {
    /// Column name as `ClickHouse` reports it.
    pub name: String,
    /// `ClickHouse` type, e.g. `UInt64`, `Nullable(Date)`.
    #[serde(rename = "type")]
    pub ty: String,
}

/// A saved dashboard SQL source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SqlSource {
    /// `s_<8 hex>`.
    pub id: String,
    /// Display title.
    pub title: String,
    /// The validated `SELECT`.
    pub sql: String,
    /// Columns the statement returns, probed when it was saved.
    pub columns: Vec<SourceColumn>,
    /// Folder id; empty = root.
    pub folder_id: String,
    /// Principal id of whoever last saved it.
    pub created_by: String,
    /// When this version was saved, `ClickHouse`-formatted. Every save is a
    /// new row, so this is effectively "updated at".
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub updated_at: Option<String>,
}

impl SqlSource {
    /// The column names, for chart validation and filter matching.
    #[must_use]
    pub fn column_names(&self) -> HashSet<String> {
        self.columns.iter().map(|c| c.name.clone()).collect()
    }

    /// The columns with their filter kinds, from the types probed when the
    /// source was saved (never guessed from the names).
    #[must_use]
    pub fn column_kinds(&self) -> crate::builder::RelationColumns {
        self.columns
            .iter()
            .map(|c| {
                (
                    c.name.clone(),
                    crate::filters::ColumnKind::from_clickhouse_type(&c.ty),
                )
            })
            .collect()
    }
}

/// A fresh source id.
#[must_use]
pub fn new_source_id() -> String {
    format!("s_{}", random_hex(4))
}

const SOURCE_COLS: &str =
    "id, title, sql, columns_json, folder_id, created_by, toString(created_at) AS updated_at";

fn row_str<'a>(row: &'a serde_json::Map<String, Value>, key: &str) -> &'a str {
    row.get(key).and_then(Value::as_str).unwrap_or("")
}

fn row_to_source(row: &serde_json::Map<String, Value>) -> SqlSource {
    SqlSource {
        id: row_str(row, "id").to_owned(),
        title: row_str(row, "title").to_owned(),
        sql: row_str(row, "sql").to_owned(),
        // A corrupt column list degrades to "no columns", which makes every
        // chart built on the source fail validation loudly rather than
        // silently filtering on columns nobody checked.
        columns: serde_json::from_str(row_str(row, "columns_json")).unwrap_or_default(),
        folder_id: row_str(row, "folder_id").to_owned(),
        created_by: row_str(row, "created_by").to_owned(),
        updated_at: Some(row_str(row, "updated_at").to_owned()),
    }
}

/// Every live source, by title.
///
/// # Errors
///
/// Returns [`ChError`] on a `ClickHouse` failure.
pub async fn list_sources(ch: &ChClient) -> Result<Vec<SqlSource>, ChError> {
    ensure_bi_table(ch).await?;
    let rows = ch
        .rows(
            &format!(
                "SELECT {SOURCE_COLS} FROM console.bi_source FINAL WHERE is_deleted = 0 ORDER BY title"
            ),
            None,
        )
        .await?;
    Ok(rows.iter().map(row_to_source).collect())
}

/// One live source by id.
///
/// # Errors
///
/// Returns [`ChError`] on a `ClickHouse` failure.
pub async fn get_source(ch: &ChClient, id: &str) -> Result<Option<SqlSource>, ChError> {
    ensure_bi_table(ch).await?;
    let rows = ch
        .rows(
            &format!(
                "SELECT {SOURCE_COLS} FROM console.bi_source FINAL WHERE is_deleted = 0 AND id = {} LIMIT 1",
                SqlLiteral::from(id)
            ),
            None,
        )
        .await?;
    Ok(rows.first().map(row_to_source))
}

/// Save (create or replace) a source. The caller has already validated
/// `source.sql` and probed `source.columns`.
///
/// # Errors
///
/// Returns [`ChError`] on a `ClickHouse` failure.
pub async fn save_source(ch: &ChClient, source: &SqlSource) -> Result<(), ChError> {
    ensure_bi_table(ch).await?;
    let columns_json = serde_json::to_string(&source.columns).unwrap_or_else(|_| "[]".to_owned());
    let sql = format!(
        "INSERT INTO console.bi_source (id, title, sql, columns_json, folder_id, created_by) VALUES \
         ({}, {}, {}, {}, {}, {})",
        SqlLiteral::from(source.id.as_str()),
        SqlLiteral::from(source.title.as_str()),
        SqlLiteral::from(source.sql.as_str()),
        SqlLiteral::from(columns_json),
        SqlLiteral::from(source.folder_id.as_str()),
        SqlLiteral::from(source.created_by.as_str()),
    );
    ch.exec(&sql, None).await
}

/// Tombstone a source.
///
/// # Errors
///
/// Returns [`ChError`] on a `ClickHouse` failure.
pub async fn delete_source(ch: &ChClient, id: &str) -> Result<(), ChError> {
    ensure_bi_table(ch).await?;
    let sql = format!(
        "INSERT INTO console.bi_source (id, title, sql, is_deleted) VALUES ({}, '', '', 1)",
        SqlLiteral::from(id)
    );
    ch.exec(&sql, None).await
}

/// Ids of the stored charts that read source `id` — a source still in use
/// cannot be deleted (the API answers 409 and names them).
#[must_use]
pub fn charts_using_source<'a>(charts: &'a [StoredChartSpec], id: &str) -> Vec<&'a str> {
    charts
        .iter()
        .filter(|c| c.def.sql_source.as_deref() == Some(id))
        .map(|c| c.spec.id.as_str())
        .collect()
}
