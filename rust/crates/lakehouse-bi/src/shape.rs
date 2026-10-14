//! What a chart can say about its own data (`BI-6`, with `AI-6`): a
//! measure with its own aggregation, the chart's own filters, a sort, the
//! top or bottom N with an "Other" group, and the several groupings of a
//! table.
//!
//! # Why the shapes are what they are
//!
//! - **Compatibility.** Every field is optional and absent on a chart saved
//!   before this module existed; absent means the statement that chart always
//!   had, byte for byte. A chart that uses any of them is rebuilt at read time
//!   from its definition ([`crate::store::ChartInput::uses_shaping`]).
//! - **Measures.** `measures` is still a list of column names on the wire, and
//!   an entry may be `{"column": …, "aggregate": …}` instead. [`MeasureList`]
//!   keeps the column names as a slice (what every reader of the list already
//!   expects) and the optional aggregate beside each, so a reader that does not
//!   know about aggregates still sees the right columns.
//! - **Own filters.** The same [`FilterDef`] as a dashboard filter, with the
//!   same validation and limits, checked against the chart's relation when the
//!   chart is saved. At read time they are applied before and in addition to
//!   the dashboard's filters ([`crate::builder`]), and one that can no longer
//!   apply (its column went away) fails the tile rather than being dropped:
//!   dropping it would show more rows than the author chose.
//! - **Other.** The computed group is aggregated from the underlying rows that
//!   fall outside the kept groups, never from the hidden results (an average
//!   of averages is wrong). It carries a `__other` flag in the result, so a
//!   real value that is literally called "Other" is kept apart from it.
//!
//! # Safety of the SQL
//!
//! Nothing here formats caller text into SQL. Columns are [`Ident`]s checked
//! against the relation, aggregates are the closed [`Aggregate`] set, filter
//! values go through the existing predicate builder, and the result names of
//! an own aggregation (`avg_stay`) are an [`Aggregate`] name and an [`Ident`].

use std::collections::{HashMap, HashSet};
use std::ops::Deref;

use lakehouse_core::ident::Ident;
use serde::de::Error as _;
use serde::ser::SerializeSeq;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::bin::{BinSpec, GridSpec};
use crate::builder::{Measure, Relation, filter_predicates};
use crate::filters::{ColumnKind, FilterDef, MAX_FILTERS, validate_filters};
use crate::grain::{Grain, TimeContext};
use crate::joins::{JoinSpec, MAX_JOINS};
use crate::specs::{Aggregate, ChartKind};

/// Most columns a table may be grouped by.
pub const MAX_GROUPINGS: usize = 5;

/// The aggregates a measure may carry.
fn aggregate_allowed(raw: &str) -> bool {
    matches!(raw, "sum" | "avg" | "max" | "min" | "count")
}

/// One entry of `measures` as it is written: a column name, or a column with
/// its own aggregate (the shape a pivot's `values` use).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
enum MeasureEntry {
    Name(String),
    /// `BI-2`: `{ "metric": <name> }`.
    Metric(MetricRef),
    Own(OwnAggregate),
}

/// A measure that is a saved metric, by name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct MetricRef {
    metric: String,
}

/// A measure with its own aggregation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct OwnAggregate {
    /// A column, or a parameter standing for one (`BI-11`).
    #[serde(with = "crate::params::slot")]
    column: String,
    /// Absent only for a column switch without an aggregate of its own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    aggregate: Option<String>,
}

/// The measures of a chart: column names, and for some of them an aggregate of
/// their own. Dereferences to the column names, so code that only needs the
/// columns reads it as a `&[String]`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MeasureList {
    columns: Vec<String>,
    aggregates: Vec<Option<String>>,
    /// `BI-2`: whether the entry is a metric (its "column" is the metric's
    /// name). Empty or shorter than `columns` reads as `false`.
    metrics: Vec<bool>,
}

impl Deref for MeasureList {
    type Target = [String];

    fn deref(&self) -> &[String] {
        &self.columns
    }
}

impl From<Vec<String>> for MeasureList {
    fn from(columns: Vec<String>) -> Self {
        let aggregates = vec![None; columns.len()];
        Self {
            columns,
            aggregates,
            metrics: Vec::new(),
        }
    }
}

/// A list of plain column names equals the measures only when none of them has
/// an aggregate of its own.
impl PartialEq<Vec<String>> for MeasureList {
    fn eq(&self, other: &Vec<String>) -> bool {
        self.columns == *other && !self.has_own_aggregates() && !self.has_metrics()
    }
}

impl MeasureList {
    /// A list from `(column, own aggregate)` pairs.
    #[must_use]
    pub fn from_pairs(pairs: Vec<(String, Option<String>)>) -> Self {
        let (columns, aggregates) = pairs.into_iter().unzip();
        Self {
            columns,
            aggregates,
            metrics: Vec::new(),
        }
    }

    /// A list where entry `i` is a metric when `is_metric[i]` (`BI-2`).
    #[must_use]
    pub fn with_metrics(mut self, is_metric: Vec<bool>) -> Self {
        // Kept empty when no entry is a metric, so two lists with the same
        // entries compare equal however they were built.
        self.metrics = if is_metric.contains(&true) {
            is_metric
        } else {
            Vec::new()
        };
        self
    }

    /// The same list marking as metrics the entries `from` marks (entry `i` of
    /// both is the same measure), except `replaced`, an entry whose column was
    /// swapped for another. BI-2 review fix (BLOCKER) R1: a list rebuilt from
    /// `(column, aggregate)` pairs forgets which entries are metrics, and a
    /// saved chart then reads its metric's name as a column.
    #[must_use]
    pub fn keeping_metrics_of(self, from: &Self, replaced: Option<usize>) -> Self {
        let flags = (0..self.columns.len())
            .map(|i| Some(i) != replaced && from.is_metric(i))
            .collect();
        self.with_metrics(flags)
    }

    /// Whether entry `index` is a saved metric, by name.
    #[must_use]
    pub fn is_metric(&self, index: usize) -> bool {
        self.metrics.get(index) == Some(&true)
    }

    /// The names of the metrics the list names, in order.
    pub fn metric_names(&self) -> impl Iterator<Item = &str> {
        self.columns
            .iter()
            .enumerate()
            .filter(|(i, _)| self.is_metric(*i))
            .map(|(_, c)| c.as_str())
    }

    /// Whether any entry is a saved metric.
    #[must_use]
    pub fn has_metrics(&self) -> bool {
        self.metrics.contains(&true)
    }

    /// The columns and their own aggregates, in order.
    pub fn entries(&self) -> impl Iterator<Item = (&str, Option<&str>)> {
        self.columns
            .iter()
            .map(String::as_str)
            .zip(self.aggregates.iter().map(Option::as_deref))
    }

    /// The name each measure's result column has, without a relation to look
    /// at: `avg_stay` for a measure with its own aggregate, the column's name
    /// otherwise. (A calculated aggregate field ignores its own aggregate and
    /// keeps its name; that needs the relation, see [`resolve_measures`].)
    #[must_use]
    pub fn result_names(&self) -> Vec<String> {
        self.entries()
            .map(|(column, own)| match own {
                Some(raw) => own_alias(Aggregate::from_str_lossy(raw), column),
                None => column.to_owned(),
            })
            .collect()
    }

    /// Whether any measure has an aggregate of its own.
    #[must_use]
    pub fn has_own_aggregates(&self) -> bool {
        self.aggregates.iter().any(Option::is_some)
    }

    /// The same list keeping only the first `n` entries.
    #[must_use]
    pub fn first_n(&self, n: usize) -> Self {
        Self {
            columns: self.columns.iter().take(n).cloned().collect(),
            aggregates: self.aggregates.iter().take(n).cloned().collect(),
            metrics: self.metrics.iter().take(n).copied().collect(),
        }
    }
}

impl Serialize for MeasureList {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut seq = serializer.serialize_seq(Some(self.columns.len()))?;
        for (i, (column, aggregate)) in self.entries().enumerate() {
            if self.is_metric(i) {
                seq.serialize_element(&MetricRef {
                    metric: column.to_owned(),
                })?;
                continue;
            }
            match aggregate {
                // BI-11: a column switch is written as an object even without
                // an aggregate of its own.
                None if crate::params::slot_param(column).is_none() => {
                    seq.serialize_element(column)?;
                }
                _ => seq.serialize_element(&OwnAggregate {
                    column: column.to_owned(),
                    aggregate: aggregate.map(str::to_owned),
                })?,
            }
        }
        seq.end()
    }
}

impl<'de> Deserialize<'de> for MeasureList {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let entries = Vec::<MeasureEntry>::deserialize(deserializer).map_err(|_| {
            D::Error::custom("measures must be column names, {column, aggregate} or {metric}")
        })?;
        let is_metric = entries
            .iter()
            .map(|e| matches!(e, MeasureEntry::Metric(_)))
            .collect();
        Ok(Self::from_pairs(
            entries
                .into_iter()
                .map(|e| match e {
                    MeasureEntry::Name(column) => (column, None),
                    MeasureEntry::Metric(m) => (m.metric, None),
                    MeasureEntry::Own(own) => (own.column, own.aggregate),
                })
                .collect(),
        )
        .with_metrics(is_metric))
    }
}

/// The result name of a measure with its own aggregate: `avg_stay`. Two
/// aggregations of one column therefore never share a name, and the chart's
/// own aggregate keeps the column's name as before.
#[must_use]
pub fn own_alias(aggregate: Aggregate, column: &str) -> String {
    format!("{}_{column}", aggregate.as_str())
}

/// Resolve `list` against the chart's relation: each measure's result name and
/// aggregate. A calculated field of aggregate or table level is the whole
/// measure and ignores any aggregate, so it keeps its own name (`BI-8`).
/// `None` when a name is not a valid identifier (data corruption only).
#[must_use]
pub fn resolve_measures(list: &MeasureList, from: &Relation) -> Option<Vec<Measure>> {
    list.entries()
        .map(|(column, own)| {
            let column_ident = Ident::new(column).ok()?;
            let ignores_aggregate =
                from.aggregate_field(column).is_some() || from.table_calc(column).is_some();
            match own {
                Some(raw) if !ignores_aggregate => {
                    let aggregate = Aggregate::from_str_lossy(raw);
                    Some(Measure {
                        alias: Ident::new(own_alias(aggregate, column)).ok()?,
                        column: column_ident,
                        aggregate: Some(aggregate),
                    })
                }
                _ => Some(Measure::plain(column_ident)),
            }
        })
        .collect()
}

/// A saved order: a column of the result, ascending or descending.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SortSpec {
    /// The dimension, a grouping, or the result name of a measure.
    pub by: String,
    /// `asc` or `desc`.
    pub dir: String,
}

impl SortSpec {
    /// Whether the order is descending.
    #[must_use]
    pub fn descending(&self) -> bool {
        self.dir == "desc"
    }
}

/// One grouping of a table: a column, and for a date or timestamp column the
/// grain it is cut by (`BI-9`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Grouping {
    /// The column.
    pub column: String,
    /// A grain name, for a date or timestamp column.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub grain: Option<String>,
    /// How a number column is binned (`BI-10`); never with a grain.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub bin: Option<BinSpec>,
}

/// One condition of a filter step, on a column of the result before it.
///
/// `value`, `min` and `max` are read as a JSON number or a string (BI-6 review
/// fix (SHOULD-FIX) R2: the assistant's model and API callers send numbers, and
/// a typed `Option<String>` refused them as "body JSON is invalid"). Any other
/// JSON type is not a decode error either: the field is recorded in `invalid`
/// and the step check names it in a plain sentence, because the route's decode
/// error carries no text (an upstream-style message must not reach a response).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "RawStepCondition")]
pub struct StepCondition {
    /// A grouping or a measure of the result the step reads.
    pub column: String,
    /// `eq`, `ne`, `gt`, `gte`, `lt`, `lte` or `between`.
    pub op: String,
    /// The number, date or text compared with (all but `between`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// `between`: the lower bound, included.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min: Option<String>,
    /// `between`: the upper bound, included.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max: Option<String>,
    /// Names of the fields (`value`, `min`, `max`) that were neither a number
    /// nor a string. Never written back; the step check refuses the condition.
    #[serde(skip)]
    pub invalid: Vec<&'static str>,
}

/// The wire shape of [`StepCondition`]: the three values as loose JSON.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawStepCondition {
    column: String,
    op: String,
    #[serde(default)]
    value: Option<serde_json::Value>,
    #[serde(default)]
    min: Option<serde_json::Value>,
    #[serde(default)]
    max: Option<serde_json::Value>,
}

impl From<RawStepCondition> for StepCondition {
    fn from(raw: RawStepCondition) -> Self {
        let mut invalid = Vec::new();
        let mut text = |name: &'static str, v: Option<serde_json::Value>| match v {
            None | Some(serde_json::Value::Null) => None,
            Some(serde_json::Value::String(s)) => Some(s),
            Some(serde_json::Value::Number(n)) => Some(n.to_string()),
            Some(_) => {
                invalid.push(name);
                None
            }
        };
        let value = text("value", raw.value);
        let min = text("min", raw.min);
        let max = text("max", raw.max);
        Self {
            column: raw.column,
            op: raw.op,
            value,
            min,
            max,
            invalid,
        }
    }
}

/// One measure of a summarise step: an aggregate of a measure of the result before.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StepMeasure {
    /// A measure of the result the step reads.
    pub column: String,
    /// `sum`, `avg`, `max`, `min` or `count`.
    pub aggregate: String,
}

/// A summarise step: group the result before by a subset of its groupings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Summarise {
    /// The groupings kept, a subset of the result before's.
    pub by: Vec<String>,
    /// The measures computed, named `<aggregate>_<column>`.
    pub measures: Vec<StepMeasure>,
}

/// A step applied to the result of the one before (`BI-6` part 2): exactly one
/// of `filter` (conditions, AND-ed) and `summarise`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    /// Keep the rows that meet every condition.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub filter: Option<Vec<StepCondition>>,
    /// Summarise the rows again.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub summarise: Option<Summarise>,
}

/// Most steps a chart takes after its summary.
pub const MAX_STEPS: usize = 2;

/// The fields `BI-6` adds to a chart definition (all but the measures' own
/// aggregates, which live in [`MeasureList`]). Flattened into
/// [`crate::store::ChartInput`] so the wire shape stays flat and a chart saved
/// before they existed reads back with every one absent.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ShapeFields {
    /// The chart's own filters, always applied.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub filters: Option<Vec<FilterDef>>,
    /// The order of the groups, in place of `order`.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub sort: Option<SortSpec>,
    /// Gather the groups past `limit` into one group called `Other`.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub other: Option<bool>,
    /// `table`: the columns it is grouped by (1 to 5).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub groupings: Option<Vec<Grouping>>,
    /// Up to [`MAX_STEPS`] steps applied to the grouped result (`BI-6` part 2).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub steps: Option<Vec<Step>>,
    /// Up to [`crate::joins::MAX_JOINS`] tables of the serving layer joined to
    /// the chart's mart (`BI-7`); the chart reads them as one flat table.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub joins: Option<Vec<JoinSpec>>,
    /// How the number `dimension` (or a pivot's first number field) is binned
    /// (`BI-10`).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub bin: Option<BinSpec>,
    /// How the number `breakdown` is binned (`BI-10`).
    #[serde(
        rename = "breakdownBin",
        skip_serializing_if = "Option::is_none",
        default
    )]
    pub breakdown_bin: Option<BinSpec>,
    /// `pointmap`, `geoheat`: group the points by grid cell (`BI-10`).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub grid: Option<GridSpec>,
    /// Up to [`crate::params::MAX_REFERENCE_LINES`] lines drawn on the value
    /// axis, each a number or a number parameter (`BI-11`).
    #[serde(
        rename = "referenceLines",
        skip_serializing_if = "Option::is_none",
        default
    )]
    pub reference_lines: Option<Vec<crate::params::ReferenceLine>>,
    /// Set on the copy of a chart whose parameters were replaced by this
    /// read's values (`BI-11`), so the builder rebuilds its statement instead
    /// of serving the stored one. Never stored and never sent.
    #[serde(skip)]
    pub resolved_params: bool,
    /// Up to [`crate::layers::MAX_LAYERS`] more questions drawn on the same
    /// chart, each from a source of its own (`BI-13`).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub layers: Option<Vec<crate::layers::LayerDef>>,
    /// How the chart's own series is drawn, `bar` or `line`, when it has
    /// layers (`BI-13`).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub draw: Option<String>,
    /// Which value axis the chart's own series uses, `left` or `right`, when it
    /// has layers (`BI-13`).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub axis: Option<String>,
    /// Set on the synthetic chart a layer is read as (`BI-13`): the builder
    /// then always rebuilds its statement from the definition (a layer has no
    /// stored statement to fall back to) and a table calculation is refused.
    /// Never stored and never sent.
    #[serde(skip)]
    pub layer_query: bool,
}

impl ShapeFields {
    /// The layers (none when absent or empty).
    #[must_use]
    pub fn layers_list(&self) -> &[crate::layers::LayerDef] {
        self.layers.as_deref().unwrap_or_default()
    }

    /// The joins (none when absent or empty).
    #[must_use]
    pub fn joins_list(&self) -> &[JoinSpec] {
        self.joins.as_deref().unwrap_or_default()
    }

    /// Whether any grouping of the chart is binned (`BI-10`).
    #[must_use]
    pub fn has_bins(&self) -> bool {
        self.bin.is_some()
            || self.breakdown_bin.is_some()
            || self.groupings.iter().flatten().any(|g| g.bin.is_some())
    }

    /// The steps (none when absent).
    #[must_use]
    pub fn steps_list(&self) -> &[Step] {
        self.steps.as_deref().unwrap_or_default()
    }

    /// Whether the chart asks for an "Other" group.
    #[must_use]
    pub fn wants_other(&self) -> bool {
        self.other == Some(true)
    }

    /// The chart's own filters (none when absent).
    #[must_use]
    pub fn own_filters(&self) -> &[FilterDef] {
        self.filters.as_deref().unwrap_or_default()
    }

    /// Whether any of the fields is present. Does not count the measures'
    /// own aggregates; see [`crate::store::ChartInput::uses_shaping`].
    #[must_use]
    pub fn is_used(&self) -> bool {
        !self.own_filters().is_empty()
            || self.sort.is_some()
            || self.wants_other()
            || self.groupings.is_some()
            || self.steps.is_some()
            || !self.joins_list().is_empty()
            || self.bin.is_some()
            || self.breakdown_bin.is_some()
            || self.grid.is_some()
            || self.resolved_params
            || self.layer_query
    }
}

/// Kinds that can gather the rest of their groups into "Other": the ones that
/// list categories along one dimension and are read as a ranking or a share.
fn takes_other(kind: ChartKind) -> bool {
    matches!(
        kind,
        ChartKind::Bar
            | ChartKind::Hbar
            | ChartKind::Stacked
            | ChartKind::Pie
            | ChartKind::Rose
            | ChartKind::Funnel
            | ChartKind::Treemap
            | ChartKind::Table
    )
}

/// Kinds whose statement is a plain `GROUP BY` over a dimension: the only ones
/// that take a sort, a top-N with "Other", or measures with their own
/// aggregate.
fn is_grouped_kind(kind: ChartKind) -> bool {
    !matches!(
        kind,
        ChartKind::Text
            | ChartKind::Heading
            | ChartKind::Divider
            | ChartKind::Section
            | ChartKind::Kpi
            | ChartKind::Gauge
            | ChartKind::Boxplot
            | ChartKind::Pointmap
            | ChartKind::Geoheat
            | ChartKind::Pivot
    )
}

fn kind_word(kind: ChartKind) -> String {
    serde_json::to_value(kind)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}

/// The checks that need neither a relation nor its columns: which fields the
/// kind takes, and the shape of each.
///
/// # Errors
///
/// The message to show the person who saved the chart.
pub fn check_kind_fields(
    measures: &MeasureList,
    shape: &ShapeFields,
    kind: ChartKind,
    rows_mode: bool,
) -> Result<(), String> {
    let grouped = is_grouped_kind(kind) && !rows_mode;
    if measures.has_own_aggregates() && !grouped {
        return Err(format!(
            "a {} takes the chart's aggregate; a measure with its own aggregate is for grouped charts and tables.",
            kind_word(kind)
        ));
    }
    if let Some(filters) = &shape.filters {
        if kind.is_static() {
            return Err("a text tile has no data to filter.".to_owned());
        }
        validate_filters(filters).map_err(|m| format!("chart filter: {m}"))?;
        if let Some(i) = filters.iter().position(|f| !f.is_active()) {
            return Err(format!(
                "chart filter {} has no value: choose a value or remove it.",
                i + 1
            ));
        }
        if filters.len() > MAX_FILTERS {
            return Err(format!("at most {MAX_FILTERS} chart filters."));
        }
    }
    if let Some(sort) = &shape.sort {
        if !grouped {
            return Err(
                "a sort is for grouped charts and tables (a table in rows mode has its own sort column)."
                    .to_owned(),
            );
        }
        if sort.dir != "asc" && sort.dir != "desc" {
            return Err(format!(
                "invalid sort direction '{}': use asc or desc.",
                sort.dir
            ));
        }
    }
    if shape.other.is_some_and(|o| o) {
        if rows_mode || !takes_other(kind) {
            return Err(format!(
                "a {} has no 'Other' group; it is for bar, hbar, stacked, pie, rose, funnel, treemap and grouped table.",
                kind_word(kind)
            ));
        }
        if shape.groupings.is_some() {
            return Err(
                "'Other' needs one dimension; a table grouped by several columns cannot gather the rest."
                    .to_owned(),
            );
        }
    }
    if shape.groupings.is_some() && (kind != ChartKind::Table || rows_mode) {
        return Err("groupings are only for a grouped table.".to_owned());
    }
    check_joins_kind(shape, kind)?;
    check_bins_kind(shape, kind, rows_mode)?;
    check_steps_kind(shape, kind, rows_mode)
}

/// Kinds whose dimension (and breakdown) can be binned: the charts that list
/// groups along one dimension, a grouped table, and a pivot. A scatter, a
/// bubble, a map, a heatmap, a calendar and a flow chart read their columns
/// as coordinates or links, not as groups.
fn takes_bin(kind: ChartKind) -> bool {
    matches!(
        kind,
        ChartKind::Bar
            | ChartKind::Hbar
            | ChartKind::Line
            | ChartKind::Area
            | ChartKind::Stacked
            | ChartKind::Combo
            | ChartKind::Pie
            | ChartKind::Rose
            | ChartKind::Funnel
            | ChartKind::Treemap
            | ChartKind::Radar
            | ChartKind::Waterfall
            | ChartKind::Table
            | ChartKind::Pivot
    )
}

/// `BI-10`: which kinds take a bin or a grid, and the combinations that make
/// no sense. Whether the columns are numbers is checked against the relation
/// ([`check_bins_against_relation`]).
fn check_bins_kind(shape: &ShapeFields, kind: ChartKind, rows_mode: bool) -> Result<(), String> {
    let any_bin = shape.bin.is_some()
        || shape.breakdown_bin.is_some()
        || shape.groupings.iter().flatten().any(|g| g.bin.is_some());
    if any_bin && (rows_mode || !takes_bin(kind)) {
        return Err(format!(
            "a {} cannot group a number in bins; bins are for bar, hbar, line, area, stacked, combo, pie, rose, funnel, treemap, radar, waterfall, grouped table and pivot charts.",
            kind_word(kind)
        ));
    }
    for spec in [&shape.bin, &shape.breakdown_bin].into_iter().flatten() {
        spec.mode()?;
    }
    for g in shape.groupings.iter().flatten() {
        if let Some(spec) = &g.bin {
            spec.mode()?;
            if g.grain.is_some() {
                return Err(format!(
                    "'{}' is grouped by a grain or by bins, not both.",
                    g.column
                ));
            }
        }
    }
    if shape.groupings.is_some() && (shape.bin.is_some() || shape.breakdown_bin.is_some()) {
        return Err("a table grouped by several columns bins each grouping on its own.".to_owned());
    }
    if kind == ChartKind::Pivot && shape.breakdown_bin.is_some() {
        return Err("a pivot bins one of its row or column fields.".to_owned());
    }
    if let Some(grid) = &shape.grid {
        if !matches!(kind, ChartKind::Pointmap | ChartKind::Geoheat) {
            return Err("a grid is for point maps and heat maps.".to_owned());
        }
        grid.checked()?;
    }
    Ok(())
}

/// `BI-7`: a text tile has no data to join, and the count of joins is bounded
/// before any schema lookup. The tables and columns are checked against the
/// schema by [`crate::joins::plan`].
fn check_joins_kind(shape: &ShapeFields, kind: ChartKind) -> Result<(), String> {
    let Some(joins) = &shape.joins else {
        return Ok(());
    };
    if kind.is_static() {
        return Err("a text tile has no data to join.".to_owned());
    }
    if joins.len() > MAX_JOINS {
        return Err(format!("a chart joins 1 to {MAX_JOINS} tables."));
    }
    Ok(())
}

/// Where steps are offered: the charts and tables grouped by one dimension (or
/// the groupings of a table), whose last result is a list of groups. A pivot, a
/// raw table, a single number and a map have none; "Other" and a breakdown
/// would change what a step's row stands for.
fn check_steps_kind(shape: &ShapeFields, kind: ChartKind, rows_mode: bool) -> Result<(), String> {
    let Some(steps) = &shape.steps else {
        return Ok(());
    };
    if rows_mode
        || !is_grouped_kind(kind)
        || matches!(
            kind,
            ChartKind::Geomap
                | ChartKind::Calendar
                | ChartKind::Heatmap
                | ChartKind::Sankey
                | ChartKind::Sunburst
        )
    {
        return Err(format!(
            "a {} takes no steps; they are for bar, hbar, line, area, stacked, combo, pie, rose, funnel, treemap, radar, waterfall and grouped table charts.",
            kind_word(kind)
        ));
    }
    if steps.is_empty() || steps.len() > MAX_STEPS {
        return Err(format!("a chart takes 1 to {MAX_STEPS} steps."));
    }
    if shape.wants_other() {
        return Err("steps cannot be combined with 'Other': a step's rows are no longer the source's groups.".to_owned());
    }
    Ok(())
}

/// What [`check_against_relation`] needs besides the definition.
pub struct RelationCheck<'a> {
    /// The chart's kind.
    pub kind: ChartKind,
    /// The relation (with its calculated fields) the chart reads.
    pub from: &'a Relation,
    /// The relation's columns.
    pub cols: &'a HashSet<String>,
    /// The kind of each column.
    pub kinds: &'a HashMap<String, ColumnKind>,
    /// The report time settings (a relative filter reads them).
    pub time: &'a TimeContext,
    /// The chart's dimension (empty when it has none).
    pub dimension: &'a str,
    /// The chart's breakdown (empty when none).
    pub breakdown: &'a str,
    /// Whether the chart carries a time grain on its dimension.
    pub grained: bool,
}

/// The checks that need the relation: the chart's own filters apply to its
/// columns, the sort names something the chart returns, the measures' result
/// names do not collide, the groupings are real columns with a grain that
/// fits, and "Other" is not asked for beside a time grain or a table
/// calculation.
///
/// # Errors
///
/// The message to show the person who saved the chart.
pub fn check_against_relation(
    measures: &MeasureList,
    shape: &ShapeFields,
    check: &RelationCheck<'_>,
) -> Result<(), String> {
    let RelationCheck {
        from,
        cols,
        kinds,
        time,
        ..
    } = check;
    for (_, own) in measures.entries() {
        if let Some(raw) = own
            && !aggregate_allowed(&raw.to_lowercase())
        {
            return Err(format!("invalid aggregate: {raw}"));
        }
    }
    let resolved = resolve_measures(measures, from)
        .ok_or_else(|| "invalid or missing measure column.".to_owned())?;
    check_aliases(&resolved, check)?;
    check_own_filters(shape, cols, kinds, time)?;
    // With steps the sort names the last step's columns; the steps' own check does it.
    if shape.steps.is_none() {
        check_sort(shape, &resolved, check)?;
    }
    check_groupings(shape, cols, kinds)?;
    check_bins_against_relation(shape, check)?;
    let has_table_calc = resolved
        .iter()
        .any(|m| from.table_calc(m.column.as_str()).is_some());
    if has_table_calc
        && (shape.sort.is_some()
            || shape.wants_other()
            || shape.groupings.is_some()
            || shape.steps.is_some())
    {
        return Err(
            "a chart with a table calculation keeps its order and its groups: no sort, 'Other' or groupings."
                .to_owned(),
        );
    }
    if shape.wants_other() && check.grained {
        return Err("'Other' is not offered with a time grain.".to_owned());
    }
    if shape.steps.is_some() && (check.grained || !check.breakdown.is_empty()) {
        return Err(
            "steps are not offered with a time grain or a breakdown; use a table grouped by several columns (a date can be cut by a grain there)."
                .to_owned(),
        );
    }
    Ok(())
}

/// `BI-10`: a binned dimension, breakdown or grouping is a number column; a
/// chart with a time grain on the same column cannot also be binned; and
/// automatic or counted bins need a table, because a SQL source's rows are cut
/// at a fixed cap and the range would be read from the cut result.
fn check_bins_against_relation(
    shape: &ShapeFields,
    check: &RelationCheck<'_>,
) -> Result<(), String> {
    let number = |column: &str| check.kinds.get(column) == Some(&ColumnKind::Number);
    let mut binned: Vec<(&str, &BinSpec)> = Vec::new();
    if let Some(spec) = &shape.bin
        && check.kind != ChartKind::Pivot
    {
        binned.push((check.dimension, spec));
    }
    if let Some(spec) = &shape.breakdown_bin {
        binned.push((check.breakdown, spec));
    }
    for g in shape.groupings.iter().flatten() {
        if let Some(spec) = &g.bin {
            binned.push((g.column.as_str(), spec));
        }
    }
    let specs = binned
        .iter()
        .map(|(_, s)| *s)
        .chain(shape.bin.iter().filter(|_| check.kind == ChartKind::Pivot));
    for spec in specs {
        let auto = !matches!(spec.mode(), Ok(crate::bin::BinMode::Width(_)));
        if auto && check.from.is_sql_source() {
            return Err(
                "automatic and counted bins need a table; for a SQL source choose a width."
                    .to_owned(),
            );
        }
    }
    for (column, _) in binned {
        if column.is_empty() {
            return Err("a bin needs the column it groups; choose it first.".to_owned());
        }
        if !number(column) {
            return Err(format!(
                "'{column}' is not a number column; bins are for numbers (a date has a grain)."
            ));
        }
    }
    if check.grained && (shape.bin.is_some() || shape.breakdown_bin.is_some()) {
        return Err("a chart groups by a grain or by bins, not both.".to_owned());
    }
    Ok(())
}

/// No two measures share a result name, and none takes the name of the
/// dimension or the breakdown.
fn check_aliases(resolved: &[Measure], check: &RelationCheck<'_>) -> Result<(), String> {
    let mut seen: HashSet<&str> = HashSet::new();
    for m in resolved {
        let alias = m.alias.as_str();
        if !seen.insert(alias) {
            return Err(format!(
                "measure '{alias}' is listed twice; remove one, or give the two a different aggregate."
            ));
        }
        if alias == check.dimension || alias == check.breakdown {
            return Err(format!(
                "the measure '{alias}' has the same name as a grouping; use another aggregate or column."
            ));
        }
    }
    Ok(())
}

/// The chart's own filters name real columns of the relation and fit their
/// types, exactly as a dashboard filter must to apply.
fn check_own_filters(
    shape: &ShapeFields,
    cols: &HashSet<String>,
    kinds: &HashMap<String, ColumnKind>,
    time: &TimeContext,
) -> Result<(), String> {
    let filters = shape.own_filters();
    if filters.is_empty() {
        return Ok(());
    }
    let relation_cols: HashMap<String, ColumnKind> = cols
        .iter()
        .map(|c| (c.clone(), kinds.get(c).copied().unwrap_or(ColumnKind::Text)))
        .collect();
    let outcome = filter_predicates(&relation_cols, &[], filters, time);
    if let Some(skipped) = outcome.skipped.first() {
        // The column name was validated as an identifier; the sentence names
        // the column and the reason, never a filter value.
        return Err(match skipped.reason {
            crate::builder::SkipReason::NoColumn => format!(
                "the chart filter on '{}' names a column this data does not have.",
                skipped.column
            ),
            crate::builder::SkipReason::WrongType => format!(
                "the chart filter on '{}' does not fit the column's type.",
                skipped.column
            ),
        });
    }
    Ok(())
}

/// The sort names the dimension, a grouping, or a measure of the chart.
fn check_sort(
    shape: &ShapeFields,
    resolved: &[Measure],
    check: &RelationCheck<'_>,
) -> Result<(), String> {
    let Some(sort) = &shape.sort else {
        return Ok(());
    };
    let by = sort.by.as_str();
    let is_measure = resolved.iter().any(|m| m.alias.as_str() == by);
    let is_grouping = shape.groupings.iter().flatten().any(|g| g.column == by);
    let is_dimension = !check.dimension.is_empty() && by == check.dimension;
    if !(is_measure || is_grouping || is_dimension) {
        return Err(format!(
            "cannot sort by '{by}': sort by the dimension, a grouping, or one of the chart's measures."
        ));
    }
    Ok(())
}

/// The groupings of a table: 1 to [`MAX_GROUPINGS`] distinct columns of the
/// relation, each with a grain that fits its column.
fn check_groupings(
    shape: &ShapeFields,
    cols: &HashSet<String>,
    kinds: &HashMap<String, ColumnKind>,
) -> Result<(), String> {
    let Some(groupings) = &shape.groupings else {
        return Ok(());
    };
    if groupings.is_empty() || groupings.len() > MAX_GROUPINGS {
        return Err(format!(
            "a table is grouped by 1 to {MAX_GROUPINGS} columns."
        ));
    }
    let mut seen: HashSet<&str> = HashSet::new();
    for g in groupings {
        if Ident::new(g.column.as_str()).is_err() || !cols.contains(&g.column) {
            return Err(format!(
                "invalid or missing grouping column '{}'.",
                g.column
            ));
        }
        if !seen.insert(g.column.as_str()) {
            return Err(format!("column '{}' is grouped twice.", g.column));
        }
        if let Some(raw) = &g.grain {
            let grain = Grain::parse(raw).ok_or_else(|| {
                format!(
                    "invalid grain '{raw}': use one of {}.",
                    crate::grain::ALL_GRAINS.map(Grain::as_str).join(", ")
                )
            })?;
            grain.validate_column(kinds.get(&g.column).copied())?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn measures(json: &str) -> MeasureList {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn a_plain_list_of_names_reads_and_writes_back_unchanged() {
        let list = measures(r#"["visitors","orders"]"#);
        assert_eq!(&*list, ["visitors", "orders"]);
        assert!(!list.has_own_aggregates());
        assert_eq!(
            serde_json::to_string(&list).unwrap(),
            r#"["visitors","orders"]"#
        );
    }

    #[test]
    fn a_name_and_an_object_mix_and_keep_their_order() {
        let list = measures(r#"["visitors",{"column":"stay","aggregate":"avg"}]"#);
        assert_eq!(&*list, ["visitors", "stay"]);
        let entries: Vec<_> = list.entries().collect();
        assert_eq!(entries, [("visitors", None), ("stay", Some("avg"))]);
        assert_eq!(
            serde_json::to_string(&list).unwrap(),
            r#"["visitors",{"column":"stay","aggregate":"avg"}]"#
        );
    }

    #[test]
    fn an_object_with_an_unknown_key_is_refused() {
        let err =
            serde_json::from_str::<MeasureList>(r#"[{"column":"stay","aggregate":"avg","x":1}]"#)
                .unwrap_err();
        assert!(err.to_string().contains("measures must be"), "{err}");
        assert!(serde_json::from_str::<MeasureList>("[1]").is_err());
    }

    #[test]
    fn two_aggregations_of_one_column_get_different_result_names() {
        let list = measures(
            r#"[{"column":"stay","aggregate":"avg"},{"column":"stay","aggregate":"max"}]"#,
        );
        let from = Relation::Mart(Ident::new("mart_x").unwrap());
        let resolved = resolve_measures(&list, &from).unwrap();
        let aliases: Vec<&str> = resolved.iter().map(|m| m.alias.as_str()).collect();
        assert_eq!(aliases, ["avg_stay", "max_stay"]);
    }

    #[test]
    fn a_plain_name_keeps_the_column_name_and_the_chart_aggregate() {
        let list = measures(r#"["visitors"]"#);
        let from = Relation::Mart(Ident::new("mart_x").unwrap());
        let resolved = resolve_measures(&list, &from).unwrap();
        assert_eq!(resolved[0].alias.as_str(), "visitors");
        assert_eq!(resolved[0].aggregate, None);
    }

    fn kind_check(
        measures: &MeasureList,
        shape: &ShapeFields,
        kind: ChartKind,
    ) -> Result<(), String> {
        check_kind_fields(measures, shape, kind, false)
    }

    #[test]
    fn a_kpi_refuses_a_measure_with_its_own_aggregate() {
        let list = measures(r#"[{"column":"stay","aggregate":"avg"}]"#);
        let err = kind_check(&list, &ShapeFields::default(), ChartKind::Kpi).unwrap_err();
        assert!(err.contains("takes the chart's aggregate"), "{err}");
    }

    #[test]
    fn a_text_tile_refuses_chart_filters() {
        let shape = ShapeFields {
            filters: Some(vec![FilterDef::in_values("c", vec!["a".to_owned()])]),
            ..ShapeFields::default()
        };
        let err = kind_check(&MeasureList::default(), &shape, ChartKind::Text).unwrap_err();
        assert!(err.contains("no data to filter"), "{err}");
    }

    #[test]
    fn a_chart_filter_without_a_value_is_refused_not_stored_inert() {
        let shape = ShapeFields {
            filters: Some(vec![FilterDef::in_values("c", Vec::new())]),
            ..ShapeFields::default()
        };
        let err = kind_check(&MeasureList::default(), &shape, ChartKind::Bar).unwrap_err();
        assert!(err.contains("has no value"), "{err}");
    }

    #[test]
    fn a_sort_direction_other_than_asc_or_desc_is_refused() {
        let shape = ShapeFields {
            sort: Some(SortSpec {
                by: "x".to_owned(),
                dir: "up".to_owned(),
            }),
            ..ShapeFields::default()
        };
        let err = kind_check(&MeasureList::default(), &shape, ChartKind::Bar).unwrap_err();
        assert!(err.contains("use asc or desc"), "{err}");
    }

    #[test]
    fn a_sort_is_refused_on_a_kpi_a_pivot_and_a_raw_table() {
        let shape = ShapeFields {
            sort: Some(SortSpec {
                by: "x".to_owned(),
                dir: "asc".to_owned(),
            }),
            ..ShapeFields::default()
        };
        for kind in [ChartKind::Kpi, ChartKind::Pivot, ChartKind::Boxplot] {
            assert!(kind_check(&MeasureList::default(), &shape, kind).is_err());
        }
        assert!(
            check_kind_fields(&MeasureList::default(), &shape, ChartKind::Table, true).is_err()
        );
    }

    #[test]
    fn other_is_refused_on_a_line_a_pivot_and_a_raw_table_with_a_plain_sentence() {
        let shape = ShapeFields {
            other: Some(true),
            ..ShapeFields::default()
        };
        for kind in [ChartKind::Line, ChartKind::Pivot, ChartKind::Heatmap] {
            let err = kind_check(&MeasureList::default(), &shape, kind).unwrap_err();
            assert!(err.contains("has no 'Other' group"), "{err}");
        }
        assert!(
            check_kind_fields(&MeasureList::default(), &shape, ChartKind::Table, true).is_err()
        );
        assert!(kind_check(&MeasureList::default(), &shape, ChartKind::Bar).is_ok());
    }

    #[test]
    fn other_beside_groupings_is_refused() {
        let shape = ShapeFields {
            other: Some(true),
            groupings: Some(vec![Grouping {
                column: "a".to_owned(),
                grain: None,
                bin: None,
            }]),
            ..ShapeFields::default()
        };
        let err = kind_check(&MeasureList::default(), &shape, ChartKind::Table).unwrap_err();
        assert!(err.contains("one dimension"), "{err}");
    }

    #[test]
    fn groupings_are_only_for_a_grouped_table() {
        let shape = ShapeFields {
            groupings: Some(vec![Grouping {
                column: "a".to_owned(),
                grain: None,
                bin: None,
            }]),
            ..ShapeFields::default()
        };
        assert!(kind_check(&MeasureList::default(), &shape, ChartKind::Bar).is_err());
        assert!(
            check_kind_fields(&MeasureList::default(), &shape, ChartKind::Table, true).is_err()
        );
        assert!(kind_check(&MeasureList::default(), &shape, ChartKind::Table).is_ok());
    }

    fn cols(names: &[(&str, ColumnKind)]) -> (HashSet<String>, HashMap<String, ColumnKind>) {
        (
            names.iter().map(|(n, _)| (*n).to_owned()).collect(),
            names.iter().map(|(n, k)| ((*n).to_owned(), *k)).collect(),
        )
    }

    fn relation_check<'a>(
        c: &'a (HashSet<String>, HashMap<String, ColumnKind>),
        from: &'a Relation,
        time: &'a TimeContext,
        dimension: &'a str,
    ) -> RelationCheck<'a> {
        RelationCheck {
            kind: ChartKind::Bar,
            from,
            cols: &c.0,
            kinds: &c.1,
            time,
            dimension,
            breakdown: "",
            grained: false,
        }
    }

    fn time() -> TimeContext {
        TimeContext::new("UTC", crate::grain::WeekStart::Monday).unwrap()
    }

    #[test]
    fn a_measure_listed_twice_with_the_same_aggregate_is_refused() {
        let c = cols(&[("p", ColumnKind::Text), ("stay", ColumnKind::Number)]);
        let from = Relation::Mart(Ident::new("m").unwrap());
        let t = time();
        let list = measures(
            r#"[{"column":"stay","aggregate":"avg"},{"column":"stay","aggregate":"avg"}]"#,
        );
        let err = check_against_relation(
            &list,
            &ShapeFields::default(),
            &relation_check(&c, &from, &t, "p"),
        )
        .unwrap_err();
        assert!(err.contains("listed twice"), "{err}");
    }

    #[test]
    fn an_own_aggregate_that_names_the_dimension_is_refused() {
        let c = cols(&[("p", ColumnKind::Text), ("sum_x", ColumnKind::Number)]);
        let from = Relation::Mart(Ident::new("m").unwrap());
        let t = time();
        let list = measures(r#"[{"column":"x","aggregate":"sum"}]"#);
        let err = check_against_relation(
            &list,
            &ShapeFields::default(),
            &relation_check(&c, &from, &t, "sum_x"),
        )
        .unwrap_err();
        assert!(err.contains("same name as a grouping"), "{err}");
    }

    #[test]
    fn an_unknown_aggregate_is_refused_with_its_name() {
        let c = cols(&[("p", ColumnKind::Text), ("x", ColumnKind::Number)]);
        let from = Relation::Mart(Ident::new("m").unwrap());
        let t = time();
        let list = measures(r#"[{"column":"x","aggregate":"median"}]"#);
        let err = check_against_relation(
            &list,
            &ShapeFields::default(),
            &relation_check(&c, &from, &t, "p"),
        )
        .unwrap_err();
        assert_eq!(err, "invalid aggregate: median");
    }

    #[test]
    fn a_chart_filter_on_a_missing_column_is_refused_without_its_value() {
        let c = cols(&[("p", ColumnKind::Text), ("x", ColumnKind::Number)]);
        let from = Relation::Mart(Ident::new("m").unwrap());
        let t = time();
        let shape = ShapeFields {
            filters: Some(vec![FilterDef::in_values(
                "nope",
                vec!["secret".to_owned()],
            )]),
            ..ShapeFields::default()
        };
        let err = check_against_relation(
            &measures(r#"["x"]"#),
            &shape,
            &relation_check(&c, &from, &t, "p"),
        )
        .unwrap_err();
        assert!(err.contains("'nope'"), "{err}");
        assert!(!err.contains("secret"), "{err}");
    }

    #[test]
    fn a_sort_by_something_the_chart_does_not_return_is_refused() {
        let c = cols(&[("p", ColumnKind::Text), ("x", ColumnKind::Number)]);
        let from = Relation::Mart(Ident::new("m").unwrap());
        let t = time();
        let shape = |by: &str| ShapeFields {
            sort: Some(SortSpec {
                by: by.to_owned(),
                dir: "desc".to_owned(),
            }),
            ..ShapeFields::default()
        };
        let list = measures(r#"["x"]"#);
        let check = relation_check(&c, &from, &t, "p");
        assert!(check_against_relation(&list, &shape("p"), &check).is_ok());
        assert!(check_against_relation(&list, &shape("x"), &check).is_ok());
        let err = check_against_relation(&list, &shape("other"), &check).unwrap_err();
        assert!(err.contains("cannot sort by 'other'"), "{err}");
    }

    #[test]
    fn groupings_are_one_to_five_distinct_real_columns() {
        let c = cols(&[
            ("a", ColumnKind::Text),
            ("b", ColumnKind::Text),
            ("d", ColumnKind::Date),
            ("x", ColumnKind::Number),
        ]);
        let from = Relation::Mart(Ident::new("m").unwrap());
        let t = time();
        let g = |cols: &[(&str, Option<&str>)]| ShapeFields {
            groupings: Some(
                cols.iter()
                    .map(|(c, grain)| Grouping {
                        column: (*c).to_owned(),
                        grain: grain.map(str::to_owned),
                        bin: None,
                    })
                    .collect(),
            ),
            ..ShapeFields::default()
        };
        let list = measures(r#"["x"]"#);
        let mut check = relation_check(&c, &from, &t, "");
        check.kind = ChartKind::Table;
        assert!(check_against_relation(&list, &g(&[("a", None), ("b", None)]), &check).is_ok());
        assert!(check_against_relation(&list, &g(&[("d", Some("month"))]), &check).is_ok());
        for bad in [
            g(&[]),
            g(&[("a", None), ("a", None)]),
            g(&[("zz", None)]),
            g(&[("a", Some("month"))]),
            g(&[("d", Some("hour"))]),
            g(&[("d", Some("fortnight"))]),
            g(&[
                ("a", None),
                ("b", None),
                ("d", None),
                ("x", None),
                ("a", None),
                ("b", None),
            ]),
        ] {
            assert!(check_against_relation(&list, &bad, &check).is_err());
        }
    }

    #[test]
    fn other_beside_a_time_grain_is_refused() {
        let c = cols(&[("d", ColumnKind::Date), ("x", ColumnKind::Number)]);
        let from = Relation::Mart(Ident::new("m").unwrap());
        let t = time();
        let mut check = relation_check(&c, &from, &t, "d");
        check.grained = true;
        let shape = ShapeFields {
            other: Some(true),
            ..ShapeFields::default()
        };
        let err = check_against_relation(&measures(r#"["x"]"#), &shape, &check).unwrap_err();
        assert!(err.contains("time grain"), "{err}");
    }

    // ── BI-10 ───────────────────────────────────────────────────────────

    fn shape_of(json: &str) -> ShapeFields {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn bins_and_grids_are_for_the_kinds_that_group_or_map() {
        let bin = shape_of(r#"{"bin":{"mode":"auto"}}"#);
        let none = MeasureList::default();
        assert!(kind_check(&none, &bin, ChartKind::Bar).is_ok());
        assert!(kind_check(&none, &bin, ChartKind::Pivot).is_ok());
        for kind in [
            ChartKind::Kpi,
            ChartKind::Pointmap,
            ChartKind::Scatter,
            ChartKind::Text,
        ] {
            assert!(kind_check(&none, &bin, kind).is_err(), "{kind:?}");
        }
        assert!(check_kind_fields(&none, &bin, ChartKind::Table, true).is_err());
        let grid = shape_of(r#"{"grid":{"size":0.1}}"#);
        assert!(kind_check(&none, &grid, ChartKind::Pointmap).is_ok());
        assert!(kind_check(&none, &grid, ChartKind::Geoheat).is_ok());
        assert!(kind_check(&none, &grid, ChartKind::Bar).is_err());
        let off_list = shape_of(r#"{"grid":{"size":0.3}}"#);
        assert!(kind_check(&none, &off_list, ChartKind::Pointmap).is_err());
    }

    #[test]
    fn a_bad_bin_or_a_bin_with_a_grain_is_refused_with_a_sentence() {
        let none = MeasureList::default();
        let bad = shape_of(r#"{"bin":{"mode":"count","count":7}}"#);
        assert!(
            kind_check(&none, &bad, ChartKind::Bar)
                .unwrap_err()
                .contains("count")
        );
        let both =
            shape_of(r#"{"groupings":[{"column":"a","grain":"month","bin":{"mode":"auto"}}]}"#);
        assert!(
            kind_check(&none, &both, ChartKind::Table)
                .unwrap_err()
                .contains("not both")
        );
        let mixed = shape_of(r#"{"bin":{"mode":"auto"},"groupings":[{"column":"a"}]}"#);
        assert!(kind_check(&none, &mixed, ChartKind::Table).is_err());
    }

    #[test]
    fn a_bin_needs_a_number_column_and_a_table_for_automatic_edges() {
        let c = cols(&[
            ("p", ColumnKind::Text),
            ("stay", ColumnKind::Number),
            ("w", ColumnKind::Number),
        ]);
        let t = time();
        let mart = Relation::Mart(Ident::new("m").unwrap());
        let list = measures(r#"["w"]"#);
        let auto = shape_of(r#"{"bin":{"mode":"auto"}}"#);
        assert!(
            check_against_relation(&list, &auto, &relation_check(&c, &mart, &t, "stay")).is_ok()
        );
        let err =
            check_against_relation(&list, &auto, &relation_check(&c, &mart, &t, "p")).unwrap_err();
        assert!(err.contains("not a number"), "{err}");
        let sql = Relation::Sql("SELECT 1".to_owned());
        let err = check_against_relation(&list, &auto, &relation_check(&c, &sql, &t, "stay"))
            .unwrap_err();
        assert!(err.contains("SQL source"), "{err}");
        let width = shape_of(r#"{"bin":{"mode":"width","width":5}}"#);
        assert!(
            check_against_relation(&list, &width, &relation_check(&c, &sql, &t, "stay")).is_ok()
        );
    }
}
