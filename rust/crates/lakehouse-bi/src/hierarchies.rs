//! Hierarchies (`BI-10`, with `AI-6`): a named, ordered list of 2 to 6
//! columns of one table or SQL source (province > city > place), defined once
//! and used to drill down on any chart whose dimension is one of its levels.
//!
//! # What is stored, and what is not
//!
//! `console.bi_hierarchy`, created by [`crate::store::ensure_bi_table`] with
//! the versioned-insert shape of `bi_field` (a save is an `INSERT`, the newest
//! `created_at` wins under `FINAL`, a delete is an `is_deleted = 1` tombstone).
//! Only the list of column names is kept. **Charts do not store a hierarchy**:
//! drilling is the viewer's own, temporary view (feature page, decision 5), so
//! deleting a hierarchy can never break a saved chart, and no chart needs
//! checking when one changes.
//!
//! # Permissions
//!
//! A hierarchy is metadata about column names the reader can already see
//! through `fields`; it carries no data. A drill request is an ordinary tile
//! statement through the same guard and role rewrite as every other.

use lakehouse_clickhouse::{ChClient, ChError};
use lakehouse_core::ident::{Ident, SqlLiteral};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::builder::RelationColumns;
use crate::fields::SourceKind;
use crate::store::{ensure_bi_table, random_hex};

/// Fewest and most levels.
pub const MIN_LEVELS: usize = 2;
/// See [`MIN_LEVELS`].
pub const MAX_LEVELS: usize = 6;
/// Longest name, in characters.
pub const MAX_NAME_LEN: usize = 64;
/// Most hierarchies one source may carry.
pub const MAX_PER_SOURCE: usize = 50;

/// A saved hierarchy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HierarchyDef {
    /// `h_<8 hex>`.
    pub id: String,
    /// Whether it belongs to a mart or a SQL source.
    pub source_kind: SourceKind,
    /// The mart name or the SQL source id.
    pub source_id: String,
    /// What people call it.
    pub name: String,
    /// The columns, outermost first.
    pub levels: Vec<String>,
    /// Display name of whoever last saved it.
    pub created_by: String,
    /// When this version was saved, `ClickHouse`-formatted.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub updated_at: Option<String>,
}

/// A fresh hierarchy id.
#[must_use]
pub fn new_id() -> String {
    format!("h_{}", random_hex(4))
}

/// Check a hierarchy's name and levels against the source's columns.
///
/// # Errors
///
/// The sentence to show the person who defined it.
pub fn validate<H: std::hash::BuildHasher>(
    name: &str,
    levels: &[String],
    columns: &RelationColumns<H>,
) -> Result<(), String> {
    if name.trim().is_empty()
        || name.chars().count() > MAX_NAME_LEN
        || name.chars().any(char::is_control)
    {
        return Err(format!(
            "a hierarchy name is 1 to {MAX_NAME_LEN} characters."
        ));
    }
    if !(MIN_LEVELS..=MAX_LEVELS).contains(&levels.len()) {
        return Err(format!(
            "a hierarchy has {MIN_LEVELS} to {MAX_LEVELS} levels."
        ));
    }
    let mut seen = std::collections::HashSet::new();
    for level in levels {
        if Ident::new(level.as_str()).is_err() || !columns.contains_key(level) {
            return Err(format!("'{level}' is not a column of this source."));
        }
        if !seen.insert(level.as_str()) {
            return Err(format!("'{level}' is listed twice."));
        }
    }
    Ok(())
}

const COLS: &str =
    "id, source_kind, source_id, name, levels_json, created_by, toString(created_at) AS updated_at";

fn row_str<'a>(row: &'a serde_json::Map<String, Value>, key: &str) -> &'a str {
    row.get(key).and_then(Value::as_str).unwrap_or("")
}

fn row_to_def(row: &serde_json::Map<String, Value>) -> Option<HierarchyDef> {
    Some(HierarchyDef {
        id: row_str(row, "id").to_owned(),
        source_kind: SourceKind::parse(row_str(row, "source_kind"))?,
        source_id: row_str(row, "source_id").to_owned(),
        name: row_str(row, "name").to_owned(),
        levels: serde_json::from_str(row_str(row, "levels_json")).ok()?,
        created_by: row_str(row, "created_by").to_owned(),
        updated_at: Some(row_str(row, "updated_at").to_owned()),
    })
}

/// The live hierarchies of one source.
///
/// # Errors
///
/// Returns [`ChError`] on a `ClickHouse` failure.
pub async fn list_for(
    ch: &ChClient,
    kind: SourceKind,
    source_id: &str,
) -> Result<Vec<HierarchyDef>, ChError> {
    ensure_bi_table(ch).await?;
    let rows = ch
        .rows(
            &format!(
                "SELECT {COLS} FROM console.bi_hierarchy FINAL WHERE is_deleted = 0 \
                 AND source_kind = {} AND source_id = {} ORDER BY name LIMIT {MAX_PER_SOURCE}",
                SqlLiteral::from(kind.as_str()),
                SqlLiteral::from(source_id)
            ),
            None,
        )
        .await?;
    Ok(rows.iter().filter_map(row_to_def).collect())
}

/// One live hierarchy by id.
///
/// # Errors
///
/// Returns [`ChError`] on a `ClickHouse` failure.
pub async fn get(ch: &ChClient, id: &str) -> Result<Option<HierarchyDef>, ChError> {
    ensure_bi_table(ch).await?;
    let rows = ch
        .rows(
            &format!(
                "SELECT {COLS} FROM console.bi_hierarchy FINAL WHERE is_deleted = 0 AND id = {} LIMIT 1",
                SqlLiteral::from(id)
            ),
            None,
        )
        .await?;
    Ok(rows.first().and_then(row_to_def))
}

/// Save (create or replace) a hierarchy. The caller has validated it.
///
/// # Errors
///
/// Returns [`ChError`] on a `ClickHouse` failure.
pub async fn save(ch: &ChClient, h: &HierarchyDef) -> Result<(), ChError> {
    ensure_bi_table(ch).await?;
    let levels = serde_json::to_string(&h.levels).unwrap_or_else(|_| "[]".to_owned());
    let sql = format!(
        "INSERT INTO console.bi_hierarchy (id, source_kind, source_id, name, levels_json, created_by) VALUES ({}, {}, {}, {}, {}, {})",
        SqlLiteral::from(h.id.as_str()),
        SqlLiteral::from(h.source_kind.as_str()),
        SqlLiteral::from(h.source_id.as_str()),
        SqlLiteral::from(h.name.as_str()),
        SqlLiteral::from(levels.as_str()),
        SqlLiteral::from(h.created_by.as_str()),
    );
    ch.exec(&sql, None).await
}

/// Tombstone a hierarchy.
///
/// # Errors
///
/// Returns [`ChError`] on a `ClickHouse` failure.
pub async fn delete(ch: &ChClient, id: &str) -> Result<(), ChError> {
    ensure_bi_table(ch).await?;
    let sql = format!(
        "INSERT INTO console.bi_hierarchy (id, is_deleted) VALUES ({}, 1)",
        SqlLiteral::from(id)
    );
    ch.exec(&sql, None).await
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use crate::filters::ColumnKind;

    fn cols() -> RelationColumns {
        ["a", "b", "c", "d", "e", "f", "g"]
            .into_iter()
            .map(|n| (n.to_owned(), ColumnKind::Text))
            .collect()
    }

    fn levels(names: &[&str]) -> Vec<String> {
        names.iter().map(|n| (*n).to_owned()).collect()
    }

    #[test]
    fn two_to_six_distinct_columns_of_the_source_are_a_hierarchy() {
        assert!(validate("Geo", &levels(&["a", "b"]), &cols()).is_ok());
        assert!(validate("Geo", &levels(&["a", "b", "c", "d", "e", "f"]), &cols()).is_ok());
    }

    #[test]
    fn anything_else_is_refused_with_a_sentence() {
        let bad = |name: &str, l: &[&str]| validate(name, &levels(l), &cols()).unwrap_err();
        assert!(bad("Geo", &["a"]).contains("2 to 6"));
        assert!(bad("Geo", &["a", "b", "c", "d", "e", "f", "g"]).contains("2 to 6"));
        assert!(bad("Geo", &["a", "a"]).contains("twice"));
        assert!(bad("Geo", &["a", "zz"]).contains("not a column"));
        assert!(bad("Geo", &["a", "b; DROP"]).contains("not a column"));
        assert!(bad("  ", &["a", "b"]).contains("name"));
    }

    #[test]
    fn a_stored_row_reads_back() {
        let row: serde_json::Map<String, Value> = serde_json::from_value(serde_json::json!({
            "id": "h_1", "source_kind": "mart", "source_id": "m", "name": "Geo",
            "levels_json": "[\"a\",\"b\"]", "created_by": "u", "updated_at": "t",
        }))
        .unwrap();
        let h = row_to_def(&row).unwrap();
        assert_eq!(h.levels, ["a", "b"]);
        assert_eq!(h.source_kind, SourceKind::Mart);
    }
}
