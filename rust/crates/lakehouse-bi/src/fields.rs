//! Saved calculated fields (`BI-8`): a named formula that belongs to one
//! table or one SQL source and is picked in a chart like a column.
//!
//! # Storage
//!
//! `console.bi_field`, created by [`crate::store::ensure_bi_table`] with the
//! same versioned-insert shape as `bi_source` (a save is an `INSERT`, the
//! newest `created_at` wins under `FINAL`, a delete is an `is_deleted = 1`
//! tombstone). No Postgres migration: the dashboards live in `ClickHouse`.
//! What is stored is the formula text and the level and type the server
//! inferred when it was saved; neither is trusted when a chart is built,
//! because a chart using a field is rebuilt at read time from the formula
//! (so a changed formula, or a changed time zone, takes effect), and the
//! formula is checked and compiled again against the source's current
//! columns.
//!
//! # How a chart uses a field
//!
//! [`prepare`] looks for field names in the slots of a chart definition
//! (`dimension`, `breakdown`, `measures`, the columns of a table, ...).
//! A row-level field becomes a column of the relation:
//! `SELECT *, <expression> AS <name> FROM <source>` ([`Relation::Calculated`]),
//! so everything that already works on a column (an aggregate, a grain, a
//! filter on another column) works on it. An aggregate field can only be a
//! measure: its expression replaces the chart's `sum(measure)` there.
//!
//! # Permissions
//!
//! The field's expression reads columns of the source and nothing else, and
//! the statement it ends up in goes through the same guard and role rewrite
//! as every tile. The rewrite replaces the *table* with its masked and
//! filtered projection, so the expression sees masked values, never raw ones
//! (verified against the real rewrite, `docs/superpowers/plans/2026-10-11-bi-8-calculated-fields.md`
//! section 7).

use std::collections::{BTreeMap, BTreeSet};

use lakehouse_clickhouse::{ChClient, ChError};
use lakehouse_core::ident::{Ident, SqlLiteral};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::builder::{AggregateField, Relation, RelationColumns};
use crate::formula::compile::{FType, Level, Scope, compile};
use crate::grain::TimeContext;
use crate::specs::ChartKind;
use crate::store::{ChartInput, StoredChartSpec, ensure_bi_table, random_hex};

/// Longest field name.
pub const MAX_NAME_LEN: usize = 64;
/// Most fields one source may carry; a bound on what a chart read loads.
pub const MAX_FIELDS_PER_SOURCE: usize = 200;
/// Longest formula stored; the language's own limit is characters, this is
/// bytes of the same text.
const MAX_FORMULA_BYTES: usize = 8000;

/// Words a field cannot be called: the formula language's own words and the
/// SQL keywords an alias must not be.
const RESERVED: [&str; 36] = [
    "and", "or", "not", "true", "false", "null", "select", "from", "where", "group", "order", "by",
    "as", "limit", "having", "union", "distinct", "join", "on", "in", "is", "case", "when", "then",
    "else", "end", "like", "between", "with", "all", "any", "asc", "desc", "interval", "final",
    "settings",
];

/// What kind of source a field belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    /// A Gold mart (`serving.<id>`).
    Mart,
    /// A dashboard SQL source (`s_<hex>`).
    SqlSource,
}

impl SourceKind {
    /// The stored word.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Mart => "mart",
            Self::SqlSource => "sql_source",
        }
    }

    /// Parse the stored word.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "mart" => Some(Self::Mart),
            "sql_source" => Some(Self::SqlSource),
            _ => None,
        }
    }
}

/// A saved calculated field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldDef {
    /// `f_<8 hex>`.
    pub id: String,
    /// Whether it belongs to a mart or a SQL source.
    pub source_kind: SourceKind,
    /// The mart name or the SQL source id.
    pub source_id: String,
    /// The name charts pick it by: an identifier, unique per source.
    pub name: String,
    /// The formula text.
    pub formula: String,
    /// `row` or `aggregate`, as inferred when it was saved.
    pub level: String,
    /// `number`, `text`, `date`, `datetime` or `boolean`, as inferred.
    #[serde(rename = "type")]
    pub ty: String,
    /// Principal id of whoever last saved it.
    pub created_by: String,
    /// When this version was saved, `ClickHouse`-formatted.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub updated_at: Option<String>,
}

impl FieldDef {
    /// Whether the stored level is aggregate.
    #[must_use]
    pub fn is_aggregate(&self) -> bool {
        Level::parse(&self.level) == Some(Level::Aggregate)
    }
}

/// A fresh field id.
#[must_use]
pub fn new_field_id() -> String {
    format!("f_{}", random_hex(4))
}

/// Check a field name: an identifier of at most [`MAX_NAME_LEN`] characters,
/// not a reserved word, and not a column of the source (`columns`).
///
/// # Errors
///
/// The sentence to show the person who named it.
pub fn validate_name<H: std::hash::BuildHasher>(
    name: &str,
    columns: &RelationColumns<H>,
) -> Result<(), String> {
    if name.len() > MAX_NAME_LEN || Ident::new(name).is_err() {
        return Err(format!(
            "a field name is 1 to {MAX_NAME_LEN} letters, digits or _, and does not start with a digit."
        ));
    }
    if RESERVED.contains(&name.to_ascii_lowercase().as_str()) {
        return Err(format!("'{name}' is a reserved word; choose another name."));
    }
    if columns.contains_key(name) {
        return Err(format!("'{name}' is already a column of this source."));
    }
    Ok(())
}

/// Check a formula's length before it is parsed.
///
/// # Errors
///
/// The sentence to show.
pub fn validate_formula_size(formula: &str) -> Result<(), String> {
    if formula.len() > MAX_FORMULA_BYTES {
        return Err("the formula is too long.".to_owned());
    }
    Ok(())
}

// ── storage ─────────────────────────────────────────────────────────────

const FIELD_COLS: &str = "id, source_kind, source_id, name, formula, level, data_type, created_by, toString(created_at) AS updated_at";

fn row_str<'a>(row: &'a serde_json::Map<String, Value>, key: &str) -> &'a str {
    row.get(key).and_then(Value::as_str).unwrap_or("")
}

fn row_to_field(row: &serde_json::Map<String, Value>) -> Option<FieldDef> {
    Some(FieldDef {
        id: row_str(row, "id").to_owned(),
        source_kind: SourceKind::parse(row_str(row, "source_kind"))?,
        source_id: row_str(row, "source_id").to_owned(),
        name: row_str(row, "name").to_owned(),
        formula: row_str(row, "formula").to_owned(),
        level: row_str(row, "level").to_owned(),
        ty: row_str(row, "data_type").to_owned(),
        created_by: row_str(row, "created_by").to_owned(),
        updated_at: Some(row_str(row, "updated_at").to_owned()),
    })
}

/// Every live field, by name. A row with an unknown source kind is left out
/// (it can only come from a newer version of the console).
///
/// # Errors
///
/// Returns [`ChError`] on a `ClickHouse` failure.
pub async fn list_fields(ch: &ChClient) -> Result<Vec<FieldDef>, ChError> {
    ensure_bi_table(ch).await?;
    let rows = ch
        .rows(
            &format!(
                "SELECT {FIELD_COLS} FROM console.bi_field FINAL WHERE is_deleted = 0 ORDER BY name"
            ),
            None,
        )
        .await?;
    Ok(rows.iter().filter_map(row_to_field).collect())
}

/// The live fields of one source.
///
/// # Errors
///
/// Returns [`ChError`] on a `ClickHouse` failure.
pub async fn list_fields_for(
    ch: &ChClient,
    kind: SourceKind,
    source_id: &str,
) -> Result<Vec<FieldDef>, ChError> {
    ensure_bi_table(ch).await?;
    let rows = ch
        .rows(
            &format!(
                "SELECT {FIELD_COLS} FROM console.bi_field FINAL WHERE is_deleted = 0 \
                 AND source_kind = {} AND source_id = {} ORDER BY name LIMIT {MAX_FIELDS_PER_SOURCE}",
                SqlLiteral::from(kind.as_str()),
                SqlLiteral::from(source_id)
            ),
            None,
        )
        .await?;
    Ok(rows.iter().filter_map(row_to_field).collect())
}

/// One live field by id.
///
/// # Errors
///
/// Returns [`ChError`] on a `ClickHouse` failure.
pub async fn get_field(ch: &ChClient, id: &str) -> Result<Option<FieldDef>, ChError> {
    ensure_bi_table(ch).await?;
    let rows = ch
        .rows(
            &format!(
                "SELECT {FIELD_COLS} FROM console.bi_field FINAL WHERE is_deleted = 0 AND id = {} LIMIT 1",
                SqlLiteral::from(id)
            ),
            None,
        )
        .await?;
    Ok(rows.first().and_then(row_to_field))
}

/// Save (create or replace) a field. The caller has already checked the name
/// and the formula and inferred the level and type.
///
/// # Errors
///
/// Returns [`ChError`] on a `ClickHouse` failure.
pub async fn save_field(ch: &ChClient, field: &FieldDef) -> Result<(), ChError> {
    ensure_bi_table(ch).await?;
    let sql = format!(
        "INSERT INTO console.bi_field (id, source_kind, source_id, name, formula, level, data_type, created_by) VALUES \
         ({}, {}, {}, {}, {}, {}, {}, {})",
        SqlLiteral::from(field.id.as_str()),
        SqlLiteral::from(field.source_kind.as_str()),
        SqlLiteral::from(field.source_id.as_str()),
        SqlLiteral::from(field.name.as_str()),
        SqlLiteral::from(field.formula.as_str()),
        SqlLiteral::from(field.level.as_str()),
        SqlLiteral::from(field.ty.as_str()),
        SqlLiteral::from(field.created_by.as_str()),
    );
    ch.exec(&sql, None).await
}

/// Tombstone a field.
///
/// # Errors
///
/// Returns [`ChError`] on a `ClickHouse` failure.
pub async fn delete_field(ch: &ChClient, id: &str) -> Result<(), ChError> {
    ensure_bi_table(ch).await?;
    let sql = format!(
        "INSERT INTO console.bi_field (id, is_deleted) VALUES ({}, 1)",
        SqlLiteral::from(id)
    );
    ch.exec(&sql, None).await
}

// ── the catalog a read uses ─────────────────────────────────────────────

fn key(kind: SourceKind, id: &str) -> String {
    format!("{}:{id}", kind.as_str())
}

/// Every field, by source: what a dashboard read loads once and every tile
/// looks its own source up in.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FieldCatalog {
    by_source: BTreeMap<String, Vec<FieldDef>>,
}

static NO_FIELDS: FieldCatalog = FieldCatalog {
    by_source: BTreeMap::new(),
};

impl FieldCatalog {
    /// A catalog with no fields: a read that does not use them.
    #[must_use]
    pub fn none() -> &'static Self {
        &NO_FIELDS
    }

    /// Group `fields` by source.
    #[must_use]
    pub fn from_fields(fields: Vec<FieldDef>) -> Self {
        let mut by_source: BTreeMap<String, Vec<FieldDef>> = BTreeMap::new();
        for f in fields {
            by_source
                .entry(key(f.source_kind, &f.source_id))
                .or_default()
                .push(f);
        }
        Self { by_source }
    }

    /// The fields of one source.
    #[must_use]
    pub fn of(&self, kind: SourceKind, id: &str) -> &[FieldDef] {
        self.by_source
            .get(&key(kind, id))
            .map_or(&[], Vec::as_slice)
    }

    /// The fields of the source `def` reads.
    #[must_use]
    pub fn for_chart(&self, def: &ChartInput) -> &[FieldDef] {
        match def.sql_source.as_deref().filter(|s| !s.is_empty()) {
            Some(id) => self.of(SourceKind::SqlSource, id),
            None => self.of(SourceKind::Mart, &def.mart),
        }
    }

    /// Whether `def` names a field of its source (and so must be built at
    /// read time, with the source's columns at hand).
    #[must_use]
    pub fn chart_uses(&self, def: &ChartInput) -> bool {
        let fields = self.for_chart(def);
        !fields.is_empty()
            && referenced(def)
                .iter()
                .any(|(n, _)| fields.iter().any(|f| f.name == *n))
    }
}

// ── use in a chart ──────────────────────────────────────────────────────

/// Where in a chart definition a name stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Slot {
    /// A measure: the one place an aggregate field can stand.
    Measure,
    /// Any other column slot.
    Other,
}

/// Every column name a definition mentions, with the slot it stands in.
fn referenced(def: &ChartInput) -> Vec<(&str, Slot)> {
    let mut out: Vec<(&str, Slot)> = Vec::new();
    if !def.dimension.is_empty() {
        out.push((&def.dimension, Slot::Other));
    }
    out.extend(def.breakdown.iter().map(|b| (b.as_str(), Slot::Other)));
    out.extend(def.measures.iter().map(|m| (m.as_str(), Slot::Measure)));
    out.extend(def.lat.iter().map(|c| (c.as_str(), Slot::Other)));
    out.extend(def.lon.iter().map(|c| (c.as_str(), Slot::Other)));
    let t = &def.tables;
    for list in [&t.columns, &t.rows].into_iter().flatten() {
        out.extend(list.iter().map(|c| (c.as_str(), Slot::Other)));
    }
    out.extend(
        t.values
            .iter()
            .flatten()
            .map(|v| (v.column.as_str(), Slot::Other)),
    );
    out.extend(t.sort_column.iter().map(|c| (c.as_str(), Slot::Other)));
    out.extend(
        t.compare
            .iter()
            .filter_map(|c| c.date_column.as_deref())
            .map(|c| (c, Slot::Other)),
    );
    out
}

/// Whether a chart of this shape builds its measures through
/// `QueryBuilder` / the KPI builder, the two places an aggregate field's
/// expression can replace `sum(measure)`.
fn takes_aggregate_measure(def: &ChartInput) -> bool {
    let kind = def.kind;
    match kind {
        ChartKind::Kpi => !def.tables.compares_to_previous(kind),
        ChartKind::Gauge => true,
        ChartKind::Boxplot
        | ChartKind::Pointmap
        | ChartKind::Geoheat
        | ChartKind::Pivot
        | ChartKind::Text => false,
        _ => !def.tables.is_rows_mode(kind),
    }
}

/// A chart's relation and columns once its calculated fields are applied.
#[derive(Debug, Clone)]
pub struct Prepared {
    /// The relation the chart is built over.
    pub from: Relation,
    /// The relation's columns plus the fields the chart uses, with kinds.
    pub cols: RelationColumns,
    /// The names of the fields the chart uses.
    pub used: Vec<String>,
}

/// Apply the calculated fields `def` names to `from`. `None` when it names
/// none. `fields` are the fields of `def`'s source; `cols` its columns.
///
/// A name that is also a column of the source is the column (the field is
/// ignored), so a source that later gains a column with a field's name keeps
/// working as the column.
///
/// # Errors
///
/// The sentence for the person: a formula that no longer checks out against
/// the source, or a field used where its level does not fit.
pub fn prepare<H: std::hash::BuildHasher>(
    fields: &[FieldDef],
    def: &ChartInput,
    from: Relation,
    cols: &RelationColumns<H>,
    time: &TimeContext,
) -> Result<Option<Prepared>, String> {
    prepare_refs(
        fields,
        &referenced(def),
        takes_aggregate_measure(def),
        from,
        cols,
        time,
    )
}

/// [`prepare`] for a records list (`/api/dashboard/records`), which has no
/// chart definition: only `names`, all as plain columns, so a field is
/// offered as a row-level column and an aggregate field is refused.
///
/// # Errors
///
/// As [`prepare`].
pub fn prepare_named<H: std::hash::BuildHasher>(
    fields: &[FieldDef],
    names: &[&str],
    from: Relation,
    cols: &RelationColumns<H>,
    time: &TimeContext,
) -> Result<Option<Prepared>, String> {
    let refs: Vec<(&str, Slot)> = names.iter().map(|n| (*n, Slot::Other)).collect();
    prepare_refs(fields, &refs, false, from, cols, time)
}

fn prepare_refs<H: std::hash::BuildHasher>(
    fields: &[FieldDef],
    refs: &[(&str, Slot)],
    aggregate_measure_ok: bool,
    from: Relation,
    cols: &RelationColumns<H>,
    time: &TimeContext,
) -> Result<Option<Prepared>, String> {
    let raw: RelationColumns = cols.iter().map(|(k, v)| (k.clone(), *v)).collect();
    let live: Vec<&FieldDef> = fields
        .iter()
        .filter(|f| !raw.contains_key(&f.name))
        .collect();
    let mut used: BTreeSet<&str> = BTreeSet::new();
    for (name, _) in refs {
        if let Some(field) = live.iter().find(|f| f.name == *name) {
            used.insert(field.name.as_str());
        }
    }
    if used.is_empty() {
        return Ok(None);
    }
    let mut scope = Scope::new(&raw, time);
    for f in &live {
        scope = scope.with_field(&f.name, &f.formula);
    }
    let mut columns: Vec<(String, String)> = Vec::new();
    let mut aggregates: Vec<AggregateField> = Vec::new();
    let mut aug = raw.clone();
    for name in &used {
        let Some(field) = live.iter().find(|f| f.name == *name) else {
            continue;
        };
        let compiled = compile(&field.formula, &scope, Some(&field.name)).map_err(|problem| {
            format!(
                "the calculated field '{}' does not check out: {problem}",
                field.name
            )
        })?;
        // The level is read from the formula as it is now, never from the
        // stored word, so an edit that changed it cannot leave a chart
        // building a row-level column out of an aggregate.
        if compiled.level == Level::Aggregate {
            let as_measure = aggregate_measure_ok
                && refs
                    .iter()
                    .filter(|(n, _)| n == name)
                    .all(|(_, s)| *s == Slot::Measure);
            if !as_measure {
                return Err(format!(
                    "'{name}' is an aggregate field; it can only be a value (measure) of a chart that groups, not a dimension, breakdown or column."
                ));
            }
            aggregates.push(AggregateField {
                name: field.name.clone(),
                sql: compiled.sql,
                reads: compiled.reads.into_iter().collect(),
            });
        } else {
            columns.push((field.name.clone(), compiled.sql));
        }
        aug.insert(field.name.clone(), compiled.ty.column_kind());
    }
    Ok(Some(Prepared {
        from: Relation::Calculated {
            base: Box::new(from),
            columns,
            aggregates,
        },
        cols: aug,
        used: used.into_iter().map(str::to_owned).collect(),
    }))
}

/// The inferred level and type of a formula over a source, for saving it.
/// Every field of the source except the one being saved (`own`) is nameable.
///
/// # Errors
///
/// The formula's error, with its character position.
pub fn check<H: std::hash::BuildHasher>(
    formula: &str,
    own: &str,
    others: &[FieldDef],
    cols: &RelationColumns<H>,
    time: &TimeContext,
) -> Result<(Level, FType), crate::formula::FormulaError> {
    let raw: RelationColumns = cols.iter().map(|(k, v)| (k.clone(), *v)).collect();
    let mut scope = Scope::new(&raw, time);
    for f in others.iter().filter(|f| f.name != own) {
        scope = scope.with_field(&f.name, &f.formula);
    }
    // The field being saved is in scope with its NEW formula, so a formula
    // that reaches itself, directly or through another field, is reported as
    // the circle it is.
    scope = scope.with_field(own, formula);
    let c = compile(formula, &scope, Some(own))?;
    Ok((c.level, c.ty))
}

/// Ids of the stored charts that use the field `name` of the source
/// (`kind`, `source_id`) — a field still in use cannot be deleted, and the
/// API names them.
#[must_use]
pub fn charts_using<'a>(
    charts: &'a [StoredChartSpec],
    kind: SourceKind,
    source_id: &str,
    name: &str,
) -> Vec<&'a str> {
    charts
        .iter()
        .filter(|c| {
            let same_source = match kind {
                SourceKind::SqlSource => c.def.sql_source.as_deref() == Some(source_id),
                SourceKind::Mart => {
                    c.def.sql_source.as_deref().is_none_or(str::is_empty) && c.def.mart == source_id
                }
            };
            same_source && referenced(&c.def).iter().any(|(n, _)| *n == name)
        })
        .map(|c| c.spec.id.as_str())
        .collect()
}

/// Most charts a refusal names before it says "and N more".
pub const MAX_NAMED: usize = 5;

/// BI-8 review fix (SHOULD-FIX) R2: charts named the way a person knows them,
/// by title and dashboard (the built-in board is "Main"), ids kept out of the
/// sentence, at most [`MAX_NAMED`] and then "and N more".
#[must_use]
pub fn name_charts(
    charts: &[StoredChartSpec],
    boards: &[crate::store::Board],
    ids: &[&str],
) -> Vec<String> {
    let board_name = |id: &str| {
        if id.is_empty() || id == crate::store::DEFAULT_BOARD_ID {
            "Main".to_owned()
        } else {
            boards
                .iter()
                .find(|b| b.id == id)
                .map_or_else(String::new, |b| b.name.clone())
        }
    };
    let mut out: Vec<String> = ids
        .iter()
        .filter_map(|id| charts.iter().find(|c| c.spec.id == *id))
        .map(|c| {
            let board = board_name(&c.board);
            if board.is_empty() {
                format!("the chart \"{}\"", c.spec.title)
            } else {
                format!("the chart \"{}\" on {board}", c.spec.title)
            }
        })
        .collect();
    out.sort();
    out.dedup();
    if out.len() > MAX_NAMED {
        let more = out.len() - MAX_NAMED;
        out.truncate(MAX_NAMED);
        out.push(format!("and {more} more"));
    }
    out
}

/// Names of the other fields of the same source whose formulas use `name`,
/// directly or through another field.
#[must_use]
pub fn fields_using<'a>(
    siblings: &'a [FieldDef],
    cols: &RelationColumns,
    time: &TimeContext,
    name: &str,
) -> Vec<&'a str> {
    let mut scope = Scope::new(cols, time);
    for f in siblings {
        scope = scope.with_field(&f.name, &f.formula);
    }
    siblings
        .iter()
        .filter(|f| f.name != name)
        .filter(|f| {
            compile(&f.formula, &scope, Some(&f.name)).is_ok_and(|c| c.fields.contains(name))
        })
        .map(|f| f.name.as_str())
        .collect()
}

/// The inferred type word and level word of a compile result, as stored.
#[must_use]
pub fn stored_words(level: Level, ty: FType) -> (String, String) {
    (level.as_str().to_owned(), ty.as_str().to_owned())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::collections::HashMap;

    use super::*;
    use crate::builder::{
        ReadContext, SkipReason, sql_for_sql_source_report, sql_with_filters_report,
    };
    use crate::filters::{ColumnKind, FilterDef};
    use crate::grain::TimeContext;

    fn field(name: &str, formula: &str, level: Level) -> FieldDef {
        FieldDef {
            id: format!("f_{name}"),
            source_kind: SourceKind::Mart,
            source_id: "mart_x".to_owned(),
            name: name.to_owned(),
            formula: formula.to_owned(),
            level: level.as_str().to_owned(),
            ty: "number".to_owned(),
            created_by: "u".to_owned(),
            updated_at: None,
        }
    }

    fn catalog() -> FieldCatalog {
        FieldCatalog::from_fields(vec![
            field("profit", "v - w", Level::Row),
            field("margin", "profit / v", Level::Row),
            field("ratio", "Sum(v) / Count()", Level::Aggregate),
            field("bad", "v + nowhere", Level::Row),
            field("v2", "v * 2", Level::Row),
        ])
    }

    fn cols() -> RelationColumns {
        [
            ("g", ColumnKind::Text),
            ("d", ColumnKind::Date),
            ("v", ColumnKind::Number),
            ("w", ColumnKind::Number),
        ]
        .into_iter()
        .map(|(n, k)| (n.to_owned(), k))
        .collect()
    }

    fn spec(kind: ChartKind, dimension: &str, measures: &[&str]) -> StoredChartSpec {
        StoredChartSpec::for_test(kind, "mart_x", dimension, measures, "SELECT 1".to_owned())
    }

    fn sql_of(spec: &StoredChartSpec, filters: &[FilterDef]) -> crate::builder::FilteredSql {
        let time = TimeContext::default();
        let cat = catalog();
        let read = ReadContext {
            time: &time,
            grain: None,
            fields: &cat,
        };
        let marts = HashMap::from([("mart_x".to_owned(), cols())]);
        sql_with_filters_report(spec, &[], filters, &marts, &read)
    }

    #[test]
    fn a_row_level_field_is_a_column_of_the_relation_the_chart_aggregates() {
        let got = sql_of(&spec(ChartKind::Bar, "g", &["profit"]), &[]);
        assert_eq!(
            got.sql,
            "SELECT g, round(sum(profit)) AS profit FROM \
             (SELECT *, (v - w) AS profit FROM serving.mart_x) AS calc \
             GROUP BY g ORDER BY g LIMIT 20"
        );
    }

    #[test]
    fn a_field_made_of_another_field_is_expanded_into_raw_columns() {
        let got = sql_of(&spec(ChartKind::Bar, "g", &["margin"]), &[]);
        assert!(
            got.sql
                .contains("(SELECT *, (((v - w)) / nullIf(v, 0)) AS margin FROM serving.mart_x)"),
            "{}",
            got.sql
        );
    }

    #[test]
    fn a_field_can_be_the_dimension_and_a_breakdown() {
        let mut s = spec(ChartKind::Bar, "v2", &["v"]);
        s.def.breakdown = Some("profit".to_owned());
        let got = sql_of(&s, &[]).sql;
        assert!(
            got.contains(
                "(SELECT *, (v - w) AS profit, (v * 2) AS v2 FROM serving.mart_x) AS calc"
            ),
            "{got}"
        );
        assert!(
            got.starts_with("SELECT v2, profit, round(sum(v)) AS v FROM"),
            "{got}"
        );
    }

    #[test]
    fn an_aggregate_field_replaces_the_charts_own_aggregate_as_a_measure() {
        let got = sql_of(&spec(ChartKind::Bar, "g", &["ratio"]), &[]);
        assert_eq!(
            got.sql,
            "SELECT g, (sum(v) / nullIf(count(), 0)) AS ratio FROM serving.mart_x \
             GROUP BY g ORDER BY g LIMIT 20"
        );
        // And as a KPI, where it is the number itself.
        let kpi = sql_of(&spec(ChartKind::Kpi, "", &["ratio"]), &[]);
        assert_eq!(
            kpi.sql,
            "SELECT (sum(v) / nullIf(count(), 0)) AS v FROM serving.mart_x"
        );
    }

    #[test]
    fn an_aggregate_field_orders_and_keeps_the_top_buckets_by_its_own_expression() {
        let mut s = spec(ChartKind::Bar, "g", &["ratio"]);
        s.def.breakdown = Some("d".to_owned());
        let got = sql_of(&s, &[]).sql;
        assert!(
            got.contains("ORDER BY (sum(v) / nullIf(count(), 0)) DESC LIMIT 20"),
            "{got}"
        );
    }

    #[test]
    fn an_aggregate_field_where_a_row_level_value_is_needed_leaves_the_stored_sql() {
        for s in [
            spec(ChartKind::Bar, "ratio", &["v"]),
            spec(ChartKind::Pivot, "", &[]),
            spec(ChartKind::Boxplot, "g", &["ratio"]),
        ] {
            let mut s = s;
            if s.spec.kind == ChartKind::Pivot {
                s.def.tables.rows = Some(vec!["g".to_owned()]);
                s.def.tables.values = Some(vec![crate::tables::PivotValue {
                    column: "ratio".to_owned(),
                    aggregate: "sum".to_owned(),
                }]);
            }
            let got = sql_of(&s, &[]);
            assert_eq!(got.sql, "SELECT 1", "{:?}", s.def);
        }
        // The message the save path gives:
        let c = cols();
        let t = TimeContext::default();
        let e = prepare(
            catalog().of(SourceKind::Mart, "mart_x"),
            &spec(ChartKind::Bar, "ratio", &["v"]).def,
            Relation::Mart(Ident::new("mart_x").unwrap()),
            &c,
            &t,
        )
        .unwrap_err();
        assert!(e.contains("aggregate field"), "{e}");
    }

    #[test]
    fn a_chart_that_names_a_field_is_rebuilt_even_with_no_filter_active() {
        let plain = sql_of(&spec(ChartKind::Bar, "g", &["v"]), &[]);
        assert_eq!(plain.sql, "SELECT 1", "no field, no filter: the stored SQL");
        let with = sql_of(&spec(ChartKind::Bar, "g", &["profit"]), &[]);
        assert_ne!(with.sql, "SELECT 1");
    }

    #[test]
    fn a_field_whose_formula_no_longer_checks_out_leaves_the_stored_sql() {
        let got = sql_of(&spec(ChartKind::Bar, "g", &["bad"]), &[]);
        assert_eq!(got.sql, "SELECT 1");
    }

    #[test]
    fn dashboard_filters_keep_applying_to_raw_columns_and_skip_a_field() {
        let filters: Vec<FilterDef> = serde_json::from_str(
            r#"[{"id":"1","column":"g","op":"in","values":["a"]},
                {"id":"2","column":"profit","op":"in","values":["1"]}]"#,
        )
        .unwrap();
        let got = sql_of(&spec(ChartKind::Bar, "g", &["profit"]), &filters);
        assert!(got.sql.contains("WHERE g IN ('a')"), "{}", got.sql);
        assert!(!got.sql.contains("profit IN"), "{}", got.sql);
        assert_eq!(got.skipped.len(), 1);
        assert_eq!(got.skipped[0].column, "profit");
        assert_eq!(got.skipped[0].reason, SkipReason::NoColumn);
    }

    #[test]
    fn a_column_with_a_fields_name_wins_over_the_field() {
        let mut c = cols();
        c.insert("profit".to_owned(), ColumnKind::Number);
        let t = TimeContext::default();
        let prepared = prepare(
            catalog().of(SourceKind::Mart, "mart_x"),
            &spec(ChartKind::Bar, "g", &["profit"]).def,
            Relation::Mart(Ident::new("mart_x").unwrap()),
            &c,
            &t,
        )
        .unwrap();
        assert!(
            prepared.is_none(),
            "the source's own column is used as it is"
        );
    }

    #[test]
    fn a_grained_chart_over_a_field_carries_the_field_and_the_columns_an_aggregate_reads() {
        let time = TimeContext::default();
        let cat = catalog();
        let read = ReadContext {
            time: &time,
            grain: None,
            fields: &cat,
        };
        let marts = HashMap::from([("mart_x".to_owned(), cols())]);
        let mut row = spec(ChartKind::Line, "d", &["profit"]);
        row.def.grain = Some("month".to_owned());
        let got = sql_with_filters_report(&row, &[], &[], &marts, &read).sql;
        assert!(
            got.contains(
                "(SELECT date_trunc('month', d) AS d, profit FROM \
                 (SELECT *, (v - w) AS profit FROM serving.mart_x) AS calc) AS bkt"
            ),
            "{got}"
        );
        let mut agg = spec(ChartKind::Line, "d", &["ratio"]);
        agg.def.grain = Some("month".to_owned());
        let got = sql_with_filters_report(&agg, &[], &[], &marts, &read).sql;
        assert!(
            got.contains("(SELECT date_trunc('month', d) AS d, v FROM serving.mart_x) AS bkt"),
            "{got}"
        );
        assert!(
            got.contains("(sum(v) / nullIf(count(), 0)) AS ratio"),
            "{got}"
        );
    }

    #[test]
    fn an_aggregate_that_reads_the_grouped_column_builds_no_grained_statement() {
        let time = TimeContext::default();
        let cat = FieldCatalog::from_fields(vec![field("last_day", "Max(d)", Level::Aggregate)]);
        let read = ReadContext {
            time: &time,
            grain: None,
            fields: &cat,
        };
        let marts = HashMap::from([("mart_x".to_owned(), cols())]);
        let mut s = spec(ChartKind::Line, "d", &["last_day"]);
        s.def.grain = Some("month".to_owned());
        let got = sql_with_filters_report(&s, &[], &[], &marts, &read);
        assert_eq!(got.sql, "SELECT 1", "falls back to the stored statement");
    }

    #[test]
    fn a_sql_source_chart_is_built_over_its_sources_fields_and_a_broken_one_is_an_error() {
        let time = TimeContext::default();
        let mut f = field("profit", "v - w", Level::Row);
        f.source_kind = SourceKind::SqlSource;
        f.source_id = "s_1234abcd".to_owned();
        let mut broken = field("bad", "v + nowhere", Level::Row);
        broken.source_kind = SourceKind::SqlSource;
        broken.source_id = "s_1234abcd".to_owned();
        let cat = FieldCatalog::from_fields(vec![f, broken]);
        let read = ReadContext {
            time: &time,
            grain: None,
            fields: &cat,
        };
        let mut s = spec(ChartKind::Bar, "g", &["profit"]);
        s.def.mart = String::new();
        s.def.sql_source = Some("s_1234abcd".to_owned());
        let got = sql_for_sql_source_report(
            &s,
            "SELECT g, v, w FROM serving.t",
            &cols(),
            &[],
            &[],
            &read,
        )
        .unwrap();
        assert!(
            got.sql.contains(
                "(SELECT *, (v - w) AS profit FROM (\nSELECT g, v, w FROM serving.t\n) AS src) AS calc"
            ),
            "{}",
            got.sql
        );
        assert!(got.sql.ends_with("SETTINGS max_result_rows = 2000, result_overflow_mode = 'break', max_execution_time = 30"));
        s.def.measures = vec!["bad".to_owned()];
        assert!(
            sql_for_sql_source_report(
                &s,
                "SELECT g, v, w FROM serving.t",
                &cols(),
                &[],
                &[],
                &read
            )
            .is_none()
        );
    }

    #[test]
    fn a_raw_table_can_list_a_field_as_a_column() {
        let mut s = spec(ChartKind::Table, "", &[]);
        s.def.tables.table_mode = Some("rows".to_owned());
        s.def.tables.columns = Some(vec!["g".to_owned(), "profit".to_owned()]);
        let got = sql_of(&s, &[]);
        assert!(
            got.sql.starts_with("SELECT g, profit FROM (SELECT *, (v - w) AS profit FROM serving.mart_x) AS calc ORDER BY"),
            "{}",
            got.sql
        );
        assert!(got.table.is_some());
    }

    #[test]
    fn names_are_checked_against_the_source_and_the_reserved_words() {
        let c = cols();
        assert!(validate_name("profit_2", &c).is_ok());
        for bad in [
            "",
            "1abc",
            "has space",
            "a-b",
            "v",
            "select",
            "AND",
            "x'y",
            &"a".repeat(65),
        ] {
            assert!(validate_name(bad, &c).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn which_charts_and_which_fields_use_a_field() {
        let a = spec(ChartKind::Bar, "g", &["profit"]);
        let b = spec(ChartKind::Bar, "g", &["v"]);
        let charts = vec![a, b];
        assert_eq!(
            charts_using(&charts, SourceKind::Mart, "mart_x", "profit"),
            vec!["test"]
        );
        assert!(charts_using(&charts, SourceKind::Mart, "mart_other", "profit").is_empty());
        assert!(charts_using(&charts, SourceKind::SqlSource, "s_1", "profit").is_empty());
        let cat = catalog();
        let t = TimeContext::default();
        let users = fields_using(cat.of(SourceKind::Mart, "mart_x"), &cols(), &t, "profit");
        assert_eq!(users, vec!["margin"]);
    }

    #[test]
    fn check_infers_level_and_type_and_refuses_a_cycle_with_the_field_being_saved() {
        let c = cols();
        let t = TimeContext::default();
        let cat = catalog();
        let others = cat.of(SourceKind::Mart, "mart_x");
        let (level, ty) = check("Sum(v) / 2", "newfield", others, &c, &t).unwrap();
        assert_eq!((level, ty), (Level::Aggregate, FType::Number));
        let (level, ty) = check("profit > 1", "newfield", others, &c, &t).unwrap();
        assert_eq!((level, ty), (Level::Row, FType::Boolean));
        // `profit` edited to use `margin`, which uses `profit`.
        let e = check("margin + 1", "profit", others, &c, &t).unwrap_err();
        assert!(e.message.contains("circle"), "{e:?}");
        let e = check("profit + 1", "profit", &[], &c, &t).unwrap_err();
        assert!(e.message.contains("circle"), "{e:?}");
    }

    #[test]
    fn charts_in_a_refusal_are_named_by_title_and_dashboard_without_ids_and_capped() {
        let mk = |n: usize, board: &str| {
            let mut c = StoredChartSpec::for_test(ChartKind::Bar, "m", "d", &["v"], String::new());
            c.spec.id = format!("u_{n:04}");
            c.spec.title = format!("Chart {n}");
            c.board = board.to_owned();
            c
        };
        let charts: Vec<_> = (0..8).map(|n| mk(n, "default")).collect();
        let ids: Vec<&str> = charts.iter().map(|c| c.spec.id.as_str()).collect();
        let named = name_charts(&charts, &[], &ids);
        assert_eq!(named.len(), MAX_NAMED + 1, "{named:?}");
        assert_eq!(named[0], "the chart \"Chart 0\" on Main");
        assert_eq!(named[MAX_NAMED], "and 3 more");
        assert!(named.iter().all(|n| !n.contains("u_00")), "{named:?}");
    }
}
