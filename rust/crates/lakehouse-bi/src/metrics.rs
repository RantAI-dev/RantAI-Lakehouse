//! Reusable metrics and segments (`BI-2`, with `AI-7`).
//!
//! # What they are
//!
//! A **metric** is a named aggregate formula (the language of `BI-8`) over one
//! table or one SQL source, with filters of its own, an optional date column
//! and default grain, and a label and description. A chart or a KPI names it
//! (`{ "metric": <name> }` as a measure) and stores the name, never a copy, so
//! a changed metric reaches every chart that uses it at the next read.
//!
//! A **segment** is a named list of filters over one source. It is used as
//! `{ "segment": <name> }` wherever a chart filter stands and inside a
//! metric's filters, and is expanded to its filters before any predicate is
//! built ([`expand_segments`]).
//!
//! # Storage
//!
//! `console.bi_metric` and `console.bi_segment`, created by
//! [`crate::store::ensure_bi_table`] with the same versioned-insert shape as
//! `bi_field` (a save is an `INSERT`, the newest `created_at` wins under
//! `FINAL`, a delete is an `is_deleted = 1` tombstone). No Postgres table:
//! dashboards live in `ClickHouse`. The permission that writes them is
//! `metric:write`, enforced by the routes; reading them needs only
//! `dashboard:read`.
//!
//! # How a read uses them
//!
//! Every read already loads the calculated fields of the board's sources into
//! a [`crate::fields::FieldCatalog`]. [`catalog_entries`] turns the metrics and
//! segments into entries of that catalog (level `metric` or `segment`, the body
//! as JSON in `formula`, as a value group keeps its definition), so every path
//! that builds a tile (dashboard, embed, public link, preview, records,
//! layers, alerts) sees them without a new argument. A segment a metric names
//! is expanded when the entry is made; a segment that is gone leaves the entry
//! with an `error`, so the tile that uses the metric says so and nothing
//! falls back to an unfiltered number.
//!
//! # Why a metric's filters apply to its own number
//!
//! Each aggregate in the compiled formula takes the filter as its condition
//! (the engine's `-If` form of that aggregate, [`Scope::with_condition`]), so a
//! filtered metric and an unfiltered measure share one statement and one
//! grouping. The condition is built by the ordinary predicate builder from
//! validated filters: values are escaped literals, columns are identifiers of
//! the source. A filter that cannot apply (its column is gone, its type no
//! longer fits) fails the metric and is never dropped, because dropping it
//! would show a larger number than the author chose.
//!
//! # Permissions
//!
//! A metric reads the columns of its source through the very statement a
//! chart on that source is, so the guard and the role rewrite of the *viewer*
//! govern it (masks, row filters, refused tables). Nothing here runs a
//! statement under the author's access: the author's roles are not stored and
//! not read.

use lakehouse_clickhouse::{ChClient, ChError};
use lakehouse_core::ident::{Ident, SqlLiteral};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::builder::{AggregateField, RelationColumns, filter_predicates};
use crate::fields::{FieldDef, SourceKind};
use crate::filters::{ColumnKind, FilterDef, MAX_FILTERS};
use crate::formula::compile::{Level, Scope, compile};
use crate::grain::{Grain, TimeContext};
use crate::store::{ensure_bi_table, random_hex};

/// The stored `level` of a metric's entry in the field catalog.
pub const LEVEL_METRIC: &str = "metric";
/// The stored `level` of a segment's entry in the field catalog.
pub const LEVEL_SEGMENT: &str = "segment";

/// Longest description, in characters (`BI-2` decision table).
pub const MAX_DESCRIPTION_CHARS: usize = 500;
/// Longest label, in characters.
pub const MAX_LABEL_CHARS: usize = 100;
/// Longest name, in characters.
pub const MAX_NAME_CHARS: usize = 64;
/// Longest format hint, in characters.
const MAX_FORMAT_CHARS: usize = 20;
/// Most metrics, and most segments, one deployment keeps; a bound on what a
/// read loads.
pub const MAX_PER_KIND: usize = 500;
/// Longest formula, in bytes.
const MAX_FORMULA_BYTES: usize = 8000;

/// A saved metric.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetricDef {
    /// `m_<8 hex>`.
    pub id: String,
    /// The name charts pick it by: an identifier, unique in the deployment.
    pub name: String,
    /// What people read.
    pub label: String,
    /// At most [`MAX_DESCRIPTION_CHARS`] characters.
    #[serde(default)]
    pub description: String,
    /// A mart or a SQL source.
    pub source_kind: SourceKind,
    /// The mart name or the SQL source id.
    pub source_id: String,
    /// The aggregate formula.
    pub formula: String,
    /// The metric's own filters and segment references.
    #[serde(default)]
    pub filters: Vec<FilterDef>,
    /// The date or timestamp column of the source its comparisons run on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time_column: Option<String>,
    /// The default grain of its comparisons.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grain: Option<String>,
    /// A display hint (`number`, `percent`, `currency`, `compact`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    /// Principal id of whoever last saved it.
    #[serde(default)]
    pub created_by: String,
    /// When this version was saved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
}

/// A saved segment: a named list of filters over one source.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SegmentDef {
    /// `g_<8 hex>`.
    pub id: String,
    /// The name filters refer to it by: an identifier, unique per source.
    pub name: String,
    /// What people read.
    pub label: String,
    /// A mart or a SQL source.
    pub source_kind: SourceKind,
    /// The mart name or the SQL source id.
    pub source_id: String,
    /// Plain filters; a segment does not refer to another segment.
    #[serde(default)]
    pub filters: Vec<FilterDef>,
    /// Principal id of whoever last saved it.
    #[serde(default)]
    pub created_by: String,
    /// When this version was saved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
}

/// A fresh metric id.
#[must_use]
pub fn new_metric_id() -> String {
    format!("m_{}", random_hex(4))
}

/// A fresh segment id.
#[must_use]
pub fn new_segment_id() -> String {
    format!("g_{}", random_hex(4))
}

// ── checks ──────────────────────────────────────────────────────────────

/// Check a metric or segment name: an identifier of at most
/// [`MAX_NAME_CHARS`] characters that is not a column of the source.
///
/// # Errors
///
/// The sentence to show the person who named it.
pub fn validate_name<H: std::hash::BuildHasher>(
    name: &str,
    columns: &RelationColumns<H>,
) -> Result<(), String> {
    if name.chars().count() > MAX_NAME_CHARS || Ident::new(name).is_err() {
        return Err(format!(
            "a name is 1 to {MAX_NAME_CHARS} letters, digits or _, and does not start with a digit."
        ));
    }
    if columns.contains_key(name) {
        return Err(format!("'{name}' is already a column of this source."));
    }
    Ok(())
}

/// Check the label and description lengths.
///
/// # Errors
///
/// The sentence to show.
pub fn validate_texts(label: &str, description: &str) -> Result<(), String> {
    if label.trim().is_empty() || label.chars().count() > MAX_LABEL_CHARS {
        return Err(format!("a label is 1 to {MAX_LABEL_CHARS} characters."));
    }
    if description.chars().count() > MAX_DESCRIPTION_CHARS {
        return Err(format!(
            "a description is at most {MAX_DESCRIPTION_CHARS} characters."
        ));
    }
    Ok(())
}

/// Replace every `{ "segment": … }` of `filters` with the segment's filters.
/// `lookup` gives a segment's filters by name.
///
/// # Errors
///
/// The sentence to show: a segment that does not exist, or one that refers to
/// another segment.
pub fn expand_segments(
    filters: &[FilterDef],
    lookup: &dyn Fn(&str) -> Option<Vec<FilterDef>>,
) -> Result<Vec<FilterDef>, String> {
    let mut out = Vec::with_capacity(filters.len());
    for f in filters {
        let Some(name) = &f.segment else {
            out.push(f.clone());
            continue;
        };
        let Some(inner) = lookup(name) else {
            return Err(format!("the segment '{name}' does not exist."));
        };
        if inner.iter().any(|i| i.segment.is_some()) {
            return Err(format!(
                "the segment '{name}' refers to another segment, which is not allowed."
            ));
        }
        out.extend(inner);
    }
    Ok(out)
}

/// The names of the segments `filters` refer to.
pub fn segment_names(filters: &[FilterDef]) -> impl Iterator<Item = &str> {
    filters.iter().filter_map(|f| f.segment.as_deref())
}

/// A filter list as one SQL condition over `cols`, with the columns it reads.
/// `None` for no filters.
///
/// # Errors
///
/// The sentence to show when a filter cannot apply to this source: it is
/// reported and never dropped (a smaller condition would show a larger number
/// than the author chose).
pub fn condition_of<H: std::hash::BuildHasher>(
    filters: &[FilterDef],
    cols: &RelationColumns<H>,
    time: &TimeContext,
) -> Result<Option<(String, Vec<String>)>, String> {
    if filters.len() > MAX_FILTERS {
        return Err(format!("at most {MAX_FILTERS} filters."));
    }
    for (i, f) in filters.iter().enumerate() {
        f.validate()
            .map_err(|m| format!("filter {}: {m}.", i + 1))?;
    }
    let active: Vec<FilterDef> = filters.iter().filter(|f| f.is_active()).cloned().collect();
    if active.is_empty() {
        return Ok(None);
    }
    let out = filter_predicates(cols, &[], &active, time);
    if let Some(skipped) = out.skipped.first() {
        return Err(match skipped.reason {
            crate::builder::SkipReason::NoColumn => format!(
                "the filter on '{}' names a column this source does not have.",
                skipped.column
            ),
            crate::builder::SkipReason::WrongType => format!(
                "the filter on '{}' does not fit the column's type.",
                skipped.column
            ),
        });
    }
    let condition = out
        .predicates
        .iter()
        .map(|p| format!("({p})"))
        .collect::<Vec<_>>()
        .join(" AND ");
    Ok(Some((condition, out.columns)))
}

/// What a metric needs checked beyond its text: the source's columns and
/// fields and the segments of the source.
pub struct MetricContext<'a, H: std::hash::BuildHasher = std::collections::hash_map::RandomState> {
    /// The source's columns.
    pub cols: &'a RelationColumns<H>,
    /// The source's fields (calculated fields a formula may name).
    pub fields: &'a [FieldDef],
    /// The source's segments.
    pub segments: &'a [SegmentDef],
    /// The time context, with the parameters at their defaults.
    pub time: &'a TimeContext,
}

/// A metric that checks out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckedMetric {
    /// The compiled aggregate expression under the metric's condition.
    pub sql: String,
    /// The source columns it reads (formula and filters).
    pub reads: Vec<String>,
}

/// The sentence for a metric that does not check out, and where in the
/// formula it is when the formula is at fault.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MetricProblem {
    /// What is wrong.
    pub message: String,
    /// Character position in the formula, when the formula is at fault.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<usize>,
}

impl std::fmt::Display for MetricProblem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.position {
            Some(at) => write!(f, "{} (at character {})", self.message, at + 1),
            None => f.write_str(&self.message),
        }
    }
}

impl MetricProblem {
    fn plain(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            position: None,
        }
    }
}

/// Check a metric definition against its source, as saving does.
///
/// # Errors
///
/// A [`MetricProblem`]: the first thing wrong, in plain words.
pub fn check_metric<H: std::hash::BuildHasher>(
    def: &MetricDef,
    ctx: &MetricContext<'_, H>,
) -> Result<CheckedMetric, MetricProblem> {
    validate_texts(&def.label, &def.description).map_err(MetricProblem::plain)?;
    if def.formula.len() > MAX_FORMULA_BYTES {
        return Err(MetricProblem::plain("the formula is too long."));
    }
    if let Some(format) = &def.format
        && (format.chars().count() > MAX_FORMAT_CHARS
            || !format
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_'))
    {
        return Err(MetricProblem::plain("the format hint is not valid."));
    }
    match (&def.time_column, &def.grain) {
        (None, Some(_)) => {
            return Err(MetricProblem::plain(
                "a default grain needs a date column to cut.",
            ));
        }
        (Some(column), grain) => {
            let kind = ctx.cols.get(column).copied();
            if !matches!(kind, Some(ColumnKind::Date | ColumnKind::DateTime)) {
                return Err(MetricProblem::plain(format!(
                    "'{column}' is not a date or timestamp column of this source."
                )));
            }
            if let Some(raw) = grain {
                let ok = Grain::parse(raw).is_some_and(|g| {
                    g.is_truncation() && g.supports(crate::specs::ChartKind::Line, kind)
                });
                if !ok {
                    return Err(MetricProblem::plain(
                        "the default grain must be a way of cutting dates into periods that fits the date column (such as month).",
                    ));
                }
            }
        }
        (None, None) => {}
    }
    let lookup = |name: &str| {
        ctx.segments
            .iter()
            .find(|s| s.name == name)
            .map(|s| s.filters.clone())
    };
    let expanded = expand_segments(&def.filters, &lookup).map_err(MetricProblem::plain)?;
    let entry = entry_of_metric(def, Ok(expanded));
    let compiled = compile_entry(&entry, ctx.cols, ctx.fields, ctx.time)?;
    Ok(CheckedMetric {
        sql: compiled.sql,
        reads: compiled.reads,
    })
}

/// Check a segment definition against its source.
///
/// # Errors
///
/// The sentence to show.
pub fn check_segment<H: std::hash::BuildHasher>(
    def: &SegmentDef,
    cols: &RelationColumns<H>,
    time: &TimeContext,
) -> Result<(), String> {
    validate_texts(&def.label, "")?;
    if def.filters.is_empty() {
        return Err("a segment needs at least one filter.".to_owned());
    }
    if def.filters.iter().any(|f| f.segment.is_some()) {
        return Err("a segment cannot refer to another segment.".to_owned());
    }
    condition_of(&def.filters, cols, time)?;
    Ok(())
}

// ── the catalog entries ─────────────────────────────────────────────────

/// The body kept in a metric's catalog entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct EntryBody {
    formula: String,
    filters: Vec<FilterDef>,
    /// Why the metric cannot be used (a segment it names is gone).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    /// BI-2 review fix (SHOULD-FIX) R3: the metric's label, so a tile can show
    /// it where it would show the identifier. Empty when the metric has none.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    label: String,
    /// `BI-17` (D4): the metric's format hint, so a tile can use it as the
    /// default format of the value. Empty when the metric has none.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    format: String,
    /// The date column and default grain of the metric's comparisons.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    time_column: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    grain: Option<String>,
}

fn json_of<T: Serialize>(value: &T) -> String {
    serde_json::to_string(value).unwrap_or_default()
}

/// The catalog entry of a metric. `expanded` is its filters with segments
/// replaced, or the sentence for why that failed.
#[must_use]
pub fn entry_of_metric(def: &MetricDef, expanded: Result<Vec<FilterDef>, String>) -> FieldDef {
    let (filters, error) = match expanded {
        Ok(f) => (f, None),
        Err(e) => (Vec::new(), Some(e)),
    };
    FieldDef {
        id: def.id.clone(),
        source_kind: def.source_kind,
        source_id: def.source_id.clone(),
        name: def.name.clone(),
        formula: json_of(&EntryBody {
            formula: def.formula.clone(),
            filters,
            error,
            label: def.label.trim().to_owned(),
            format: def.format.clone().unwrap_or_default(),
            time_column: def.time_column.clone(),
            grain: def.grain.clone(),
        }),
        level: LEVEL_METRIC.to_owned(),
        ty: "number".to_owned(),
        created_by: def.created_by.clone(),
        updated_at: def.updated_at.clone(),
    }
}

/// The format hint a metric's catalog entry carries; `None` when it has none
/// or is not a metric entry.
#[must_use]
pub fn entry_format(entry: &FieldDef) -> Option<String> {
    if !entry.is_metric() {
        return None;
    }
    serde_json::from_str::<EntryBody>(&entry.formula)
        .ok()
        .map(|b| b.format)
        .filter(|f| !f.is_empty())
}

/// The label a metric's catalog entry carries; `None` when it has none or is
/// not a metric entry (the caller then shows the name).
#[must_use]
pub fn entry_label(entry: &FieldDef) -> Option<String> {
    if !entry.is_metric() {
        return None;
    }
    serde_json::from_str::<EntryBody>(&entry.formula)
        .ok()
        .map(|b| b.label)
        .filter(|l| !l.is_empty())
}

/// The catalog entry of a segment.
#[must_use]
pub fn entry_of_segment(def: &SegmentDef) -> FieldDef {
    FieldDef {
        id: def.id.clone(),
        source_kind: def.source_kind,
        source_id: def.source_id.clone(),
        name: def.name.clone(),
        formula: json_of(&def.filters),
        level: LEVEL_SEGMENT.to_owned(),
        ty: String::new(),
        created_by: def.created_by.clone(),
        updated_at: def.updated_at.clone(),
    }
}

/// The catalog entries of `metrics` and `segments`.
#[must_use]
pub fn catalog_entries(metrics: &[MetricDef], segments: &[SegmentDef]) -> Vec<FieldDef> {
    let mut out: Vec<FieldDef> = segments.iter().map(entry_of_segment).collect();
    for m in metrics {
        let lookup = |name: &str| {
            segments
                .iter()
                .find(|s| {
                    s.name == name && s.source_kind == m.source_kind && s.source_id == m.source_id
                })
                .map(|s| s.filters.clone())
        };
        out.push(entry_of_metric(m, expand_segments(&m.filters, &lookup)));
    }
    out
}

/// The filters of a segment's catalog entry.
#[must_use]
pub fn segment_filters(entry: &FieldDef) -> Option<Vec<FilterDef>> {
    (entry.level == LEVEL_SEGMENT)
        .then(|| serde_json::from_str(&entry.formula).ok())
        .flatten()
}

/// The aggregate expression of a metric's catalog entry, built over `cols`.
/// `fields` are the source's calculated fields the formula may name.
///
/// # Errors
///
/// A [`MetricProblem`]: the metric's filters cannot apply to this source, its
/// formula no longer checks out, or it is not an aggregate that every
/// aggregate of which can take a condition.
pub fn compile_entry<H: std::hash::BuildHasher>(
    entry: &FieldDef,
    cols: &RelationColumns<H>,
    fields: &[FieldDef],
    time: &TimeContext,
) -> Result<AggregateField, MetricProblem> {
    let body: EntryBody = serde_json::from_str(&entry.formula)
        .map_err(|_| MetricProblem::plain("the metric's definition could not be read."))?;
    if let Some(error) = body.error {
        return Err(MetricProblem::plain(format!(
            "the metric '{}' cannot be used: {error}",
            entry.name
        )));
    }
    let raw: RelationColumns = cols.iter().map(|(k, v)| (k.clone(), *v)).collect();
    let condition = condition_of(&body.filters, &raw, time).map_err(|m| {
        MetricProblem::plain(format!("the filters of the metric '{}': {m}", entry.name))
    })?;
    let mut scope = Scope::new(&raw, time);
    let mut reads: Vec<String> = Vec::new();
    if let Some((sql, columns)) = condition {
        scope = scope.with_condition(sql);
        reads = columns;
    }
    for f in fields
        .iter()
        .filter(|f| !f.is_group() && f.level != LEVEL_METRIC && f.level != LEVEL_SEGMENT)
        .filter(|f| !raw.contains_key(&f.name))
    {
        scope = scope.with_field(&f.name, &f.formula);
    }
    let compiled =
        compile(&body.formula, &scope, Some(&entry.name)).map_err(|e| MetricProblem {
            message: e.message,
            position: Some(e.position),
        })?;
    if compiled.level != Level::Aggregate {
        return Err(MetricProblem {
            message: "a metric is an aggregate: wrap the formula in an aggregate such as Sum() or Count()."
                .to_owned(),
            position: Some(0),
        });
    }
    if !compiled.staged.aggs.is_empty() || !compiled.staged.fixed.is_empty() {
        return Err(MetricProblem {
            message: "a metric cannot use a table calculation, a period comparison or Fixed."
                .to_owned(),
            position: Some(0),
        });
    }
    reads.extend(compiled.reads);
    reads.sort();
    reads.dedup();
    Ok(AggregateField {
        name: entry.name.clone(),
        sql: compiled.sql,
        reads,
        prefer_columns: true,
    })
}

/// A KPI's comparison with the date column and period left out filled from the
/// metric it measures: "the KPI's `compare` defaults to the metric's column and
/// grain" (`BI-2`). Anything the KPI says itself wins. The stored chart keeps
/// the comparison as it was written, so a changed default reaches it.
#[must_use]
pub fn fill_compare(
    tables: &crate::tables::TableFields,
    measures: &crate::shape::MeasureList,
    entries: &[FieldDef],
) -> crate::tables::TableFields {
    let mut out = tables.clone();
    let Some(compare) = out.compare.as_mut() else {
        return out;
    };
    if compare.kind != "previous" && compare.kind != "last_year" {
        return out;
    }
    let Some(name) = measures.metric_names().next() else {
        return out;
    };
    let body = entries
        .iter()
        .find(|f| f.is_metric() && f.name == name)
        .and_then(|f| serde_json::from_str::<EntryBody>(&f.formula).ok());
    let Some(body) = body else {
        return out;
    };
    if compare.date_column.is_none() {
        compare.date_column.clone_from(&body.time_column);
    }
    if compare.period.is_none() && body.time_column.is_some() {
        compare.period = Some(body.grain.unwrap_or_else(|| "month".to_owned()));
    }
    out
}

// ── using a metric on its own ───────────────────────────────────────────

/// What `query_metric` and the editor's live value ask: one metric's number,
/// optionally by one column or by a time grain, with filters.
#[derive(Debug, Clone)]
pub struct ValueQuery<'a> {
    /// The metric's name.
    pub metric: &'a str,
    /// The metric's source.
    pub source_kind: SourceKind,
    /// The mart name or SQL source id.
    pub source_id: &'a str,
    /// One column to group by; with `grain` the date column cut by it.
    pub group_by: Option<&'a str>,
    /// The grain the `group_by` date column is cut by.
    pub grain: Option<&'a str>,
    /// Filters applied to every aggregate beside the metric's own.
    pub filters: Vec<FilterDef>,
}

/// Most groups a grouped value returns.
pub const MAX_VALUE_GROUPS: u32 = 100;

/// The synthetic chart a [`ValueQuery`] is read as: a single-number card, or
/// the ordinary one-dimension bar chart. It has no stored statement and is
/// always rebuilt from its definition, through the very builder every tile
/// uses, so the guard and the caller's role rewrite apply to it exactly as to
/// a chart that names the metric.
#[must_use]
pub fn value_chart(q: &ValueQuery<'_>) -> crate::store::StoredChartSpec {
    use crate::specs::{ChartKind, ChartSource, ChartY};
    use crate::store::{ChartInput, ChartSpec, StoredChartSpec};
    let (mart, sql_source) = match q.source_kind {
        SourceKind::Mart => (q.source_id.to_owned(), None),
        SourceKind::SqlSource => (String::new(), Some(q.source_id.to_owned())),
    };
    let grouped = q.group_by.is_some();
    let kind = if grouped {
        ChartKind::Bar
    } else {
        ChartKind::Kpi
    };
    let measures: crate::shape::MeasureList =
        crate::shape::MeasureList::from_pairs(vec![(q.metric.to_owned(), None)])
            .with_metrics(vec![true]);
    let def = ChartInput {
        title: q.metric.to_owned(),
        subtitle: None,
        mart: mart.clone(),
        sql_source: sql_source.clone(),
        kind,
        dimension: q.group_by.unwrap_or_default().to_owned(),
        measures,
        breakdown: None,
        map: None,
        lat: None,
        lon: None,
        aggregate: None,
        limit: grouped.then(|| MAX_VALUE_GROUPS.into()),
        order: grouped.then(|| if q.grain.is_some() { "none" } else { "desc" }.to_owned()),
        span: None,
        board: None,
        text: None,
        caption: None,
        target: None,
        click: None,
        grain: q.grain.map(str::to_owned),
        tables: crate::tables::TableFields::default(),
        shape: crate::shape::ShapeFields {
            filters: Some(q.filters.clone()).filter(|f| !f.is_empty()),
            layer_query: true,
            ..crate::shape::ShapeFields::default()
        },
        style: crate::style::StyleFields::default(),
        types: Box::default(),
    };
    StoredChartSpec {
        spec: ChartSpec {
            id: format!("metric:{}", q.metric),
            title: q.metric.to_owned(),
            subtitle: None,
            kind,
            mart,
            sql_source,
            sql: String::new(),
            x: q.group_by.unwrap_or_default().to_owned(),
            y: ChartY::Single(if grouped {
                q.metric.to_owned()
            } else {
                "v".to_owned()
            }),
            series: None,
            map: None,
            lat: None,
            lon: None,
            format: None,
            span: None,
            text: None,
            caption: None,
            target: None,
            click: None,
        },
        source: ChartSource::Ui,
        board: String::new(),
        def,
        has_year: false,
        created_by: None,
        created_at: None,
    }
}

/// Stored charts that name the metric `name` of a source, as a measure of the
/// chart or of one of its questions, by chart id.
///
/// BI-2 review fix (SHOULD-FIX) R2: a measure without the metric marker (a
/// chart saved before the marker was kept) counts when the reader would read
/// it as the metric, by [`crate::fields::plain_name_is_metric`]. `fields` and
/// `cols` are the fields and columns of the source `(kind, source_id)` itself;
/// they are only consulted for a measure of that source.
#[must_use]
pub fn charts_using_metric<'a>(
    charts: &'a [crate::store::StoredChartSpec],
    (kind, source_id): (SourceKind, &str),
    (fields, cols): (&[FieldDef], &RelationColumns),
    name: &str,
) -> Vec<&'a str> {
    let on = |mart: &str, sql: Option<&str>| match kind {
        SourceKind::SqlSource => sql == Some(source_id),
        SourceKind::Mart => sql.is_none_or(str::is_empty) && mart == source_id,
    };
    let names = |m: &crate::shape::MeasureList| {
        m.iter().enumerate().any(|(i, n)| {
            n == name && (m.is_metric(i) || crate::fields::plain_name_is_metric(fields, cols, name))
        })
    };
    charts
        .iter()
        .filter(|c| {
            (on(&c.def.mart, c.def.sql_source.as_deref()) && names(&c.def.measures))
                || c.def
                    .shape
                    .layers_list()
                    .iter()
                    .any(|l| on(&l.mart, l.sql_source.as_deref()) && names(&l.measures))
        })
        .map(|c| c.spec.id.as_str())
        .collect()
}

/// Stored charts that refer to the segment `name` of a source in their own
/// filters, or in the filters of one of their questions.
#[must_use]
pub fn charts_using_segment<'a>(
    charts: &'a [crate::store::StoredChartSpec],
    kind: SourceKind,
    source_id: &str,
    name: &str,
) -> Vec<&'a str> {
    let names = |f: &[FilterDef]| segment_names(f).any(|n| n == name);
    charts
        .iter()
        .filter(|c| {
            let on = |mart: &str, sql: Option<&str>| match kind {
                SourceKind::SqlSource => sql == Some(source_id),
                SourceKind::Mart => sql.is_none_or(str::is_empty) && mart == source_id,
            };
            (on(&c.def.mart, c.def.sql_source.as_deref()) && names(c.def.shape.own_filters()))
                || c.def.shape.layers_list().iter().any(|l| {
                    on(&l.mart, l.sql_source.as_deref())
                        && names(l.filters.as_deref().unwrap_or_default())
                })
        })
        .map(|c| c.spec.id.as_str())
        .collect()
}

// ── storage ─────────────────────────────────────────────────────────────

const METRIC_COLS: &str = "id, name, label, description, source_kind, source_id, formula, filters_json, time_column, grain, format, created_by, toString(created_at) AS updated_at";
const SEGMENT_COLS: &str = "id, name, label, source_kind, source_id, filters_json, created_by, toString(created_at) AS updated_at";

fn str_of<'a>(row: &'a serde_json::Map<String, Value>, key: &str) -> &'a str {
    row.get(key).and_then(Value::as_str).unwrap_or("")
}

fn some_of(row: &serde_json::Map<String, Value>, key: &str) -> Option<String> {
    Some(str_of(row, key).to_owned()).filter(|s| !s.is_empty())
}

fn row_to_metric(row: &serde_json::Map<String, Value>) -> Option<MetricDef> {
    Some(MetricDef {
        id: str_of(row, "id").to_owned(),
        name: str_of(row, "name").to_owned(),
        label: str_of(row, "label").to_owned(),
        description: str_of(row, "description").to_owned(),
        source_kind: SourceKind::parse(str_of(row, "source_kind"))?,
        source_id: str_of(row, "source_id").to_owned(),
        formula: str_of(row, "formula").to_owned(),
        filters: serde_json::from_str(str_of(row, "filters_json")).unwrap_or_default(),
        time_column: some_of(row, "time_column"),
        grain: some_of(row, "grain"),
        format: some_of(row, "format"),
        created_by: str_of(row, "created_by").to_owned(),
        updated_at: some_of(row, "updated_at"),
    })
}

fn row_to_segment(row: &serde_json::Map<String, Value>) -> Option<SegmentDef> {
    Some(SegmentDef {
        id: str_of(row, "id").to_owned(),
        name: str_of(row, "name").to_owned(),
        label: str_of(row, "label").to_owned(),
        source_kind: SourceKind::parse(str_of(row, "source_kind"))?,
        source_id: str_of(row, "source_id").to_owned(),
        filters: serde_json::from_str(str_of(row, "filters_json")).unwrap_or_default(),
        created_by: str_of(row, "created_by").to_owned(),
        updated_at: some_of(row, "updated_at"),
    })
}

/// Every live metric.
///
/// # Errors
///
/// [`ChError`] on a `ClickHouse` failure.
pub async fn list_metrics(ch: &ChClient) -> Result<Vec<MetricDef>, ChError> {
    ensure_bi_table(ch).await?;
    let rows = ch
        .rows(
            &format!(
                "SELECT {METRIC_COLS} FROM console.bi_metric FINAL WHERE is_deleted = 0 ORDER BY name LIMIT {MAX_PER_KIND}"
            ),
            None,
        )
        .await?;
    Ok(rows.iter().filter_map(row_to_metric).collect())
}

/// One live metric by id.
///
/// # Errors
///
/// [`ChError`] on a `ClickHouse` failure.
pub async fn get_metric(ch: &ChClient, id: &str) -> Result<Option<MetricDef>, ChError> {
    ensure_bi_table(ch).await?;
    let rows = ch
        .rows(
            &format!(
                "SELECT {METRIC_COLS} FROM console.bi_metric FINAL WHERE is_deleted = 0 AND id = {} LIMIT 1",
                SqlLiteral::from(id)
            ),
            None,
        )
        .await?;
    Ok(rows.first().and_then(row_to_metric))
}

/// Save (create or replace) a metric. The caller has checked it.
///
/// # Errors
///
/// [`ChError`] on a `ClickHouse` failure.
pub async fn save_metric(ch: &ChClient, m: &MetricDef) -> Result<(), ChError> {
    ensure_bi_table(ch).await?;
    let filters = serde_json::to_string(&m.filters).unwrap_or_else(|_| "[]".to_owned());
    let sql = format!(
        "INSERT INTO console.bi_metric (id, name, label, description, source_kind, source_id, formula, filters_json, time_column, grain, format, created_by) VALUES \
         ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})",
        SqlLiteral::from(m.id.as_str()),
        SqlLiteral::from(m.name.as_str()),
        SqlLiteral::from(m.label.as_str()),
        SqlLiteral::from(m.description.as_str()),
        SqlLiteral::from(m.source_kind.as_str()),
        SqlLiteral::from(m.source_id.as_str()),
        SqlLiteral::from(m.formula.as_str()),
        SqlLiteral::from(filters.as_str()),
        SqlLiteral::from(m.time_column.as_deref().unwrap_or("")),
        SqlLiteral::from(m.grain.as_deref().unwrap_or("")),
        SqlLiteral::from(m.format.as_deref().unwrap_or("")),
        SqlLiteral::from(m.created_by.as_str()),
    );
    ch.exec(&sql, None).await
}

/// Tombstone a metric.
///
/// # Errors
///
/// [`ChError`] on a `ClickHouse` failure.
pub async fn delete_metric(ch: &ChClient, id: &str) -> Result<(), ChError> {
    ensure_bi_table(ch).await?;
    ch.exec(
        &format!(
            "INSERT INTO console.bi_metric (id, is_deleted) VALUES ({}, 1)",
            SqlLiteral::from(id)
        ),
        None,
    )
    .await
}

/// Every live segment.
///
/// # Errors
///
/// [`ChError`] on a `ClickHouse` failure.
pub async fn list_segments(ch: &ChClient) -> Result<Vec<SegmentDef>, ChError> {
    ensure_bi_table(ch).await?;
    let rows = ch
        .rows(
            &format!(
                "SELECT {SEGMENT_COLS} FROM console.bi_segment FINAL WHERE is_deleted = 0 ORDER BY name LIMIT {MAX_PER_KIND}"
            ),
            None,
        )
        .await?;
    Ok(rows.iter().filter_map(row_to_segment).collect())
}

/// One live segment by id.
///
/// # Errors
///
/// [`ChError`] on a `ClickHouse` failure.
pub async fn get_segment(ch: &ChClient, id: &str) -> Result<Option<SegmentDef>, ChError> {
    ensure_bi_table(ch).await?;
    let rows = ch
        .rows(
            &format!(
                "SELECT {SEGMENT_COLS} FROM console.bi_segment FINAL WHERE is_deleted = 0 AND id = {} LIMIT 1",
                SqlLiteral::from(id)
            ),
            None,
        )
        .await?;
    Ok(rows.first().and_then(row_to_segment))
}

/// Save (create or replace) a segment. The caller has checked it.
///
/// # Errors
///
/// [`ChError`] on a `ClickHouse` failure.
pub async fn save_segment(ch: &ChClient, s: &SegmentDef) -> Result<(), ChError> {
    ensure_bi_table(ch).await?;
    let filters = serde_json::to_string(&s.filters).unwrap_or_else(|_| "[]".to_owned());
    let sql = format!(
        "INSERT INTO console.bi_segment (id, name, label, source_kind, source_id, filters_json, created_by) VALUES \
         ({}, {}, {}, {}, {}, {}, {})",
        SqlLiteral::from(s.id.as_str()),
        SqlLiteral::from(s.name.as_str()),
        SqlLiteral::from(s.label.as_str()),
        SqlLiteral::from(s.source_kind.as_str()),
        SqlLiteral::from(s.source_id.as_str()),
        SqlLiteral::from(filters.as_str()),
        SqlLiteral::from(s.created_by.as_str()),
    );
    ch.exec(&sql, None).await
}

/// Tombstone a segment.
///
/// # Errors
///
/// [`ChError`] on a `ClickHouse` failure.
pub async fn delete_segment(ch: &ChClient, id: &str) -> Result<(), ChError> {
    ensure_bi_table(ch).await?;
    ch.exec(
        &format!(
            "INSERT INTO console.bi_segment (id, is_deleted) VALUES ({}, 1)",
            SqlLiteral::from(id)
        ),
        None,
    )
    .await
}

/// The live fields of the sources, with the metrics and segments as catalog
/// entries (see the module doc). A failure to read the metric or segment
/// tables leaves the fields as they were: a chart that uses a metric then
/// fails its own tile with "the metric … does not exist", and every other tile
/// reads on.
///
/// # Errors
///
/// [`ChError`] when the fields themselves cannot be read.
pub async fn with_metrics(ch: &ChClient, fields: Vec<FieldDef>) -> Result<Vec<FieldDef>, ChError> {
    let mut out = fields;
    let (metrics, segments) = (list_metrics(ch).await, list_segments(ch).await);
    match (metrics, segments) {
        (Ok(m), Ok(s)) => out.extend(catalog_entries(&m, &s)),
        (m, s) => {
            // Logged by the caller's tracing subscriber under this target; the
            // text never reaches a response.
            tracing::warn!(
                metrics_ok = m.is_ok(),
                segments_ok = s.is_ok(),
                "the metric or segment tables could not be read; tiles that use them will say so"
            );
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::collections::HashMap;

    use serde_json::json;

    use super::*;
    use crate::builder::{ReadContext, sql_with_filters_report};
    use crate::fields::FieldCatalog;
    use crate::specs::ChartKind;
    use crate::store::StoredChartSpec;

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

    fn filter(json: serde_json::Value) -> FilterDef {
        serde_json::from_value(json).unwrap()
    }

    fn metric(name: &str, formula: &str, filters: Vec<FilterDef>) -> MetricDef {
        MetricDef {
            id: format!("m_{name}"),
            name: name.to_owned(),
            label: name.to_owned(),
            description: String::new(),
            source_kind: SourceKind::Mart,
            source_id: "mart_x".to_owned(),
            formula: formula.to_owned(),
            filters,
            time_column: Some("d".to_owned()),
            grain: Some("month".to_owned()),
            format: None,
            created_by: "u".to_owned(),
            updated_at: None,
        }
    }

    fn segment(name: &str, filters: Vec<FilterDef>) -> SegmentDef {
        SegmentDef {
            id: format!("g_{name}"),
            name: name.to_owned(),
            label: name.to_owned(),
            source_kind: SourceKind::Mart,
            source_id: "mart_x".to_owned(),
            filters,
            created_by: "u".to_owned(),
            updated_at: None,
        }
    }

    fn sql_for(chart: &StoredChartSpec, catalog: &FieldCatalog) -> String {
        let time = TimeContext::default();
        let read = ReadContext {
            time: &time,
            grain: None,
            fields: catalog,
        };
        let marts = HashMap::from([("mart_x".to_owned(), cols())]);
        sql_with_filters_report(chart, &[], &[], &marts, &read).sql
    }

    fn chart_of_metrics(measures: serde_json::Value) -> StoredChartSpec {
        let mut c =
            StoredChartSpec::for_test(ChartKind::Bar, "mart_x", "g", &[], "SELECT 1".to_owned());
        c.def.measures = serde_json::from_value(measures).unwrap();
        c
    }

    #[test]
    fn a_bare_measure_that_is_a_metric_of_the_source_is_a_use_and_a_real_column_is_not() {
        let m = metric("total", "Sum(v)", vec![]);
        let fields = catalog_entries(&[m, metric("v2", "Sum(w)", vec![])], &[]);
        let c = cols();
        // A chart saved before the marker was kept: the metric is a bare name.
        let mut bare =
            StoredChartSpec::for_test(ChartKind::Bar, "mart_x", "g", &["total"], "S".to_owned());
        bare.spec.id = "bare".to_owned();
        let mut marked = chart_of_metrics(json!([{ "metric": "total" }]));
        marked.spec.id = "marked".to_owned();
        // A bare name that is a column of the source stays the column.
        let mut column =
            StoredChartSpec::for_test(ChartKind::Bar, "mart_x", "g", &["v"], "S".to_owned());
        column.spec.id = "column".to_owned();
        let mut other_source =
            StoredChartSpec::for_test(ChartKind::Bar, "mart_y", "g", &["total"], "S".to_owned());
        other_source.spec.id = "other".to_owned();
        let charts = vec![bare, marked, column, other_source];
        let used = |name: &str| {
            charts_using_metric(&charts, (SourceKind::Mart, "mart_x"), (&fields, &c), name)
        };
        assert_eq!(used("total"), vec!["bare", "marked"]);
        assert!(used("v2").is_empty());
        // A metric that has a column's name never claims the column's users.
        let clash = catalog_entries(&[metric("v", "Sum(w)", vec![])], &[]);
        assert!(
            charts_using_metric(&charts, (SourceKind::Mart, "mart_x"), (&clash, &c), "v")
                .is_empty()
        );
    }

    #[test]
    fn the_tile_carries_the_label_of_each_metric_and_leaves_out_one_with_no_label() {
        let mut labelled = metric("total", "Sum(v)", vec![]);
        labelled.label = "Total visitors".to_owned();
        let mut blank = metric("w_sum", "Sum(w)", vec![]);
        blank.label = String::new();
        let catalog = FieldCatalog::from_fields(catalog_entries(&[labelled, blank], &[]));
        let c = cols();
        let marked = chart_of_metrics(json!([{ "metric": "total" }, { "metric": "w_sum" }, "v"]));
        let labels = catalog.metric_labels(&marked.def, None);
        assert_eq!(labels.len(), 1, "{labels:?}");
        assert_eq!(labels["total"], "Total visitors");
        // A chart saved before the marker was kept: the bare name needs the
        // source's columns to be read as the metric, and a column stays one.
        let mut bare = StoredChartSpec::for_test(
            ChartKind::Bar,
            "mart_x",
            "g",
            &["total", "v"],
            "S".to_owned(),
        );
        bare.spec.id = "bare".to_owned();
        assert!(catalog.metric_labels(&bare.def, None).is_empty());
        let healed = catalog.metric_labels(&bare.def, Some(&c));
        assert_eq!(healed.len(), 1, "{healed:?}");
        assert_eq!(healed["total"], "Total visitors");
    }

    /// `BI-17` (D4): the tile carries each metric's format hint, and a metric
    /// with none is left out.
    #[test]
    fn the_tile_carries_the_format_hint_of_each_metric_that_has_one() {
        let mut shaped = metric("total", "Sum(v)", vec![]);
        shaped.format = Some("percent".to_owned());
        let plain = metric("w_sum", "Sum(w)", vec![]);
        let catalog = FieldCatalog::from_fields(catalog_entries(&[shaped, plain], &[]));
        let marked = chart_of_metrics(json!([{ "metric": "total" }, { "metric": "w_sum" }, "v"]));
        let formats = catalog.metric_formats(&marked.def, None);
        assert_eq!(formats.len(), 1, "{formats:?}");
        assert_eq!(formats["total"], "percent");
    }

    #[test]
    fn a_metric_with_a_filter_sits_beside_an_unfiltered_measure_in_one_statement() {
        let catalog = FieldCatalog::from_fields(catalog_entries(
            &[metric(
                "domestic",
                "Sum([v])",
                vec![filter(json!({ "column": "g", "values": ["it's"] }))],
            )],
            &[],
        ));
        let chart = chart_of_metrics(json!([{ "metric": "domestic" }, "v"]));
        let sql = sql_for(&chart, &catalog);
        assert!(
            sql.contains("sumIf(v, (g IN ('it''s'))) AS domestic")
                || sql.contains("sumIf(v, (g IN ('it''s')))"),
            "{sql}"
        );
        assert!(sql.contains("round(sum(v)) AS v"), "{sql}");
        assert!(sql.contains("GROUP BY g"), "{sql}");
    }

    #[test]
    fn a_segment_is_expanded_in_a_metric_and_in_a_chart_filter() {
        let seg = segment(
            "bali",
            vec![filter(json!({ "column": "g", "values": ["Bali"] }))],
        );
        let m = metric("m", "Count()", vec![FilterDef::segment_ref("bali")]);
        let catalog = FieldCatalog::from_fields(catalog_entries(&[m], &[seg]));
        let chart = chart_of_metrics(json!([{ "metric": "m" }]));
        let sql = sql_for(&chart, &catalog);
        assert!(sql.contains("countIf((g IN ('Bali')))"), "{sql}");
        let mut plain =
            StoredChartSpec::for_test(ChartKind::Bar, "mart_x", "g", &["v"], "SELECT 1".to_owned());
        plain.def.shape.filters = Some(vec![FilterDef::segment_ref("bali")]);
        let sql = sql_for(&plain, &catalog);
        assert!(sql.contains("g IN ('Bali')"), "{sql}");
        // A segment that is gone fails the chart; it never shows more rows.
        let gone = FieldCatalog::from_fields(Vec::new());
        assert_eq!(sql_for(&plain, &gone), "SELECT 1");
    }

    #[test]
    fn a_metric_whose_segment_is_gone_or_that_does_not_exist_never_falls_back() {
        let m = metric("m", "Count()", vec![FilterDef::segment_ref("gone")]);
        let catalog = FieldCatalog::from_fields(catalog_entries(&[m], &[]));
        let chart = chart_of_metrics(json!([{ "metric": "m" }]));
        assert_eq!(sql_for(&chart, &catalog), "SELECT 1");
        let missing = chart_of_metrics(json!([{ "metric": "ghost" }]));
        assert_eq!(sql_for(&missing, &catalog), "SELECT 1");
        let err = crate::fields::prepare(
            catalog.of(SourceKind::Mart, "mart_x"),
            &missing.def,
            crate::builder::Relation::Mart(Ident::new("mart_x").unwrap()),
            &cols(),
            &TimeContext::default(),
        )
        .unwrap_err();
        assert!(err.contains("does not exist"), "{err}");
    }

    #[test]
    fn a_metric_must_be_an_aggregate_with_no_table_calculation_and_a_filter_must_apply() {
        let time = TimeContext::default();
        let c = cols();
        let ctx = MetricContext {
            cols: &c,
            fields: &[],
            segments: &[],
            time: &time,
        };
        let ok = check_metric(&metric("a", "Sum(v) / Count()", vec![]), &ctx).unwrap();
        assert!(ok.reads.contains(&"v".to_owned()));
        let row = check_metric(&metric("a", "v + 1", vec![]), &ctx).unwrap_err();
        assert!(row.message.contains("aggregate"), "{row:?}");
        let table = check_metric(&metric("a", "RunningTotal(Sum(v))", vec![]), &ctx).unwrap_err();
        assert!(
            table.message.contains("aggregate") || table.message.contains("table"),
            "{table:?}"
        );
        let no_column = metric(
            "a",
            "Sum(v)",
            vec![filter(json!({ "column": "nope", "values": ["x"] }))],
        );
        assert!(
            check_metric(&no_column, &ctx)
                .unwrap_err()
                .message
                .contains("column")
        );
        let missing_segment = metric("a", "Sum(v)", vec![FilterDef::segment_ref("none")]);
        assert!(
            check_metric(&missing_segment, &ctx)
                .unwrap_err()
                .message
                .contains("does not exist")
        );
        let mut bad_time = metric("a", "Sum(v)", vec![]);
        bad_time.time_column = Some("g".to_owned());
        assert!(
            check_metric(&bad_time, &ctx)
                .unwrap_err()
                .message
                .contains("date")
        );
        let mut long = metric("a", "Sum(v)", vec![]);
        long.description = "x".repeat(MAX_DESCRIPTION_CHARS + 1);
        assert!(check_metric(&long, &ctx).is_err());
    }

    #[test]
    fn names_are_identifiers_that_are_not_columns() {
        let c = cols();
        assert!(validate_name("total_visitors", &c).is_ok());
        assert!(validate_name("v", &c).is_err());
        assert!(validate_name("has space", &c).is_err());
        assert!(validate_name("1abc", &c).is_err());
    }

    #[test]
    fn a_comparing_kpi_takes_the_metrics_date_column_and_grain_unless_it_says_otherwise() {
        let catalog = catalog_entries(&[metric("m", "Sum(v)", vec![])], &[]);
        let measures: crate::shape::MeasureList =
            serde_json::from_value(json!([{ "metric": "m" }])).unwrap();
        let tables: crate::tables::TableFields =
            serde_json::from_value(json!({ "compare": { "kind": "last_year" } })).unwrap();
        let filled = fill_compare(&tables, &measures, &catalog);
        let c = filled.compare.unwrap();
        assert_eq!(
            (c.date_column.as_deref(), c.period.as_deref()),
            (Some("d"), Some("month"))
        );
        let own: crate::tables::TableFields = serde_json::from_value(
            json!({ "compare": { "kind": "previous", "dateColumn": "d", "period": "week" } }),
        )
        .unwrap();
        let kept = fill_compare(&own, &measures, &catalog).compare.unwrap();
        assert_eq!(kept.period.as_deref(), Some("week"));
    }

    #[test]
    fn hostile_text_in_a_metric_filter_appears_only_inside_an_escaped_literal() {
        let hostile = [
            "O'Brien",
            "back\\slash",
            "x') OR 1=1 --",
            "日本",
            "a\"b;c`d",
        ];
        for value in hostile {
            let m = metric(
                "m",
                "Sum(v)",
                vec![filter(json!({ "column": "g", "values": [value] }))],
            );
            let catalog = FieldCatalog::from_fields(catalog_entries(&[m], &[]));
            let sql = sql_for(&chart_of_metrics(json!([{ "metric": "m" }])), &catalog);
            let literal = SqlLiteral::from(value).to_string();
            assert!(sql.contains(&format!("g IN ({literal})")), "{value}: {sql}");
            let outside = sql.replace(&literal, "");
            assert!(
                !outside.contains("OR 1=1") && !outside.contains("日本"),
                "{outside}"
            );
        }
    }

    /// The property behind the hand-picked cases above: 20,000 generated strings made of the characters
    /// that break SQL (quotes, backslash, backtick, semicolon, comment markers, newline, non-ASCII) used
    /// as a metric filter value and as a segment's value. In every compiled statement the value occurs
    /// only as the escaped literal the existing helper prints, and nothing outside a literal is
    /// anything but the compiler's own punctuation, identifiers and keywords.
    #[test]
    fn generated_hostile_filter_and_segment_values_only_ever_appear_as_escaped_literals() {
        const ALPHABET: [&str; 18] = [
            "'", "\\", "`", ";", "--", "/*", "*/", "\n", "\t", "\"", "%", "日", "😀", " OR 1=1 ",
            ")", "(", "a", "Z",
        ];
        let mut seed: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for _ in 0..20_000 {
            let len = 1 + (next() % 8) as usize;
            let value: String = (0..len)
                .map(|_| ALPHABET[usize::try_from(next() % 18).unwrap_or(0)])
                .collect();
            let literal = SqlLiteral::from(value.as_str()).to_string();
            // As the metric's own filter, and as the filter of a segment the metric names.
            let own = metric(
                "m",
                "Sum(v)",
                vec![filter(json!({ "column": "g", "values": [value] }))],
            );
            let seg = segment(
                "s",
                vec![filter(json!({ "column": "g", "values": [value] }))],
            );
            let by_segment = metric("m", "Sum(v)", vec![FilterDef::segment_ref("s")]);
            for catalog in [
                FieldCatalog::from_fields(catalog_entries(&[own], &[])),
                FieldCatalog::from_fields(catalog_entries(&[by_segment], &[seg])),
            ] {
                let sql = sql_for(&chart_of_metrics(json!([{ "metric": "m" }])), &catalog);
                assert_eq!(sql.matches(&literal).count(), 1, "{value:?}: {sql}");
                let outside = sql.replacen(&literal, "", 1);
                assert!(
                    outside.chars().all(|c| c.is_ascii_alphanumeric()
                        || " _(),.*=<>+-".contains(c)
                        || c == '\n'),
                    "{value:?}: something outside the literal: {outside}"
                );
                assert!(
                    !outside.contains("--") && !outside.contains("/*"),
                    "{value:?}: {outside}"
                );
            }
        }
    }
}
