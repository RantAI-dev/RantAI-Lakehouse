//! More than one question on one chart (`BI-13`, with the `BI-13` part of
//! `AI-6`): a bar, line, area or combo chart may carry up to
//! [`MAX_LAYERS`] more questions, each over a source of its own, matched on the
//! values of the chart's dimension and drawn on the same axes.
//!
//! # Why a layer is read as a chart of its own
//!
//! A layer is not a join. Two tables that should not be joined row by row are
//! asked separately, and the console puts the two answers on one axis. So each
//! layer is turned into the ordinary one-dimension bar chart it would be if
//! saved alone ([`layer_input`], [`layer_chart`]) and goes through exactly the
//! path a tile goes through: the same builder, the same dashboard filters
//! ("a column the layer's source has"), the same guard and the same role
//! rewrite, which is applied to the layer's own statement and its own tables.
//! Nothing in this module builds SQL text.
//!
//! # What the base chart's result decides
//!
//! The base question's sort and limit decide which values the chart shows
//! ([`crate::store::ChartInput::limit_value`]). When the base was cut by that
//! limit on a text or number dimension, each layer is asked for those values
//! only, as an ordinary `in` filter built from the base result
//! ([`kept_values`]), bound as literals by the same predicate builder every
//! filter uses. The filter only narrows what the layer's own statement may
//! return; it never widens it, so a row the layer's source hides from the
//! viewer (a row filter) or masks (a mask) stays hidden or masked whatever the
//! base showed. When the base is a time grain cut to its latest buckets, the
//! layer is asked for all its buckets and trimmed to the base's range
//! ([`bucket_range`], [`trim_to_range`]), since the `in` of a bucket is not
//! a condition on the raw column. A value the layer lacks is a gap, never a
//! zero: nothing here invents a row.
//!
//! # What is refused
//!
//! A chart that has layers takes no breakdown, no "Other", no steps, no
//! table calculation and no bins, and neither does a layer: the matching is on
//! one dimension's values, and each of those would change what a value is.
//! The refusals are plain sentences ([`check_definition`]).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::filters::{ColumnKind, FilterDef, validate_filters};
use crate::grain::Grain;
use crate::joins::{JoinSpec, MAX_JOINS};
use crate::shape::{MeasureList, ShapeFields};
use crate::specs::{ChartKind, ChartY};
use crate::store::{ChartInput, ChartSpec, StoredChartSpec};

/// Most questions a chart takes besides its own.
pub const MAX_LAYERS: usize = 4;

/// Most values of the base result a layer is asked for: the largest limit of a
/// chart that is not cut by a time grain.
pub const MAX_KEPT_VALUES: usize = 100;

/// Longest label of a layer, in characters.
const MAX_LABEL_CHARS: usize = 60;

/// Most rows a layer returns where the base is not cut: a chart's largest
/// limit when it is not grained, and a grain's when it is.
pub const LAYER_ROW_LIMIT: u32 = 100;

/// One more question on a chart.
///
/// A source (`mart` or `sqlSource`, exactly one), a dimension, the measures
/// and the layer's own filters and joins; how to draw it (`draw`) and on which
/// value axis (`axis`). Anything else is refused with a plain sentence rather
/// than ignored ([`LayerDef::extra`]).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LayerDef {
    /// Mart name, without the `serving.` prefix.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub mart: String,
    /// Dashboard SQL source id, the alternative to `mart`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sql_source: Option<String>,
    /// The column matched with the chart's dimension.
    #[serde(default)]
    pub dimension: String,
    /// Checked at save like a chart's grain; at read the layer takes the
    /// chart's effective grain whatever this says.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grain: Option<String>,
    /// The values, each with an aggregate of its own if wanted.
    #[serde(default)]
    pub measures: MeasureList,
    /// The aggregate of the measures that have none of their own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aggregate: Option<String>,
    /// The layer's own filters, always applied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filters: Option<Vec<FilterDef>>,
    /// Tables joined to the layer's mart.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub joins: Option<Vec<JoinSpec>>,
    /// The name the legend shows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// `bar` or `line`.
    #[serde(default)]
    pub draw: String,
    /// `left` or `right`.
    #[serde(default)]
    pub axis: String,
    /// Any other field a body carried. Not stored: [`check_definition`]
    /// refuses a layer that has one, so a misspelt field is not silently
    /// dropped and a breakdown or steps (which a layer cannot have) get their
    /// own sentence.
    #[serde(flatten, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, Value>,
}

impl LayerDef {
    /// The source the layer reads, for the payload: the mart, or the SQL
    /// source's id.
    #[must_use]
    pub fn source_name(&self) -> String {
        match &self.sql_source {
            Some(id) if !id.trim().is_empty() => id.trim().to_owned(),
            _ => self.mart.clone(),
        }
    }

    /// The layer with its text fields trimmed, the mart without `serving.` and
    /// the measures' own aggregates in lower case: what is stored.
    #[must_use]
    pub fn normalized(&self) -> Self {
        Self {
            mart: self
                .mart
                .trim()
                .strip_prefix("serving.")
                .unwrap_or_else(|| self.mart.trim())
                .to_owned(),
            sql_source: self
                .sql_source
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned),
            dimension: self.dimension.trim().to_owned(),
            measures: MeasureList::from_pairs(
                self.measures
                    .entries()
                    .map(|(c, a)| (c.to_owned(), a.map(str::to_lowercase)))
                    .collect(),
            )
            .keeping_metrics_of(&self.measures, None),
            aggregate: self.aggregate.clone(),
            label: self
                .label
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned),
            filters: self.filters.clone().filter(|f| !f.is_empty()),
            joins: self.joins.clone().filter(|j| !j.is_empty()),
            grain: self.grain.clone(),
            draw: self.draw.clone(),
            axis: self.axis.clone(),
            extra: BTreeMap::new(),
        }
    }
}

/// Kinds that take layers: the ones that list their groups along one
/// dimension and draw a value axis.
#[must_use]
pub fn takes_layers(kind: ChartKind) -> bool {
    matches!(
        kind,
        ChartKind::Bar | ChartKind::Line | ChartKind::Area | ChartKind::Combo
    )
}

fn check_draw_axis(draw: &str, axis: &str, whose: &str) -> Result<(), String> {
    if !matches!(draw, "bar" | "line") {
        return Err(format!("{whose}: draw it as 'bar' or 'line'."));
    }
    if !matches!(axis, "left" | "right") {
        return Err(format!("{whose}: put it on the 'left' or 'right' axis."));
    }
    Ok(())
}

fn check_layer(n: usize, layer: &LayerDef) -> Result<(), String> {
    let who = format!("question {n}");
    if !layer.extra.is_empty() {
        return Err(format!(
            "{who}: a question takes a source, a dimension, measures, an aggregate, filters, joins, a label, how to draw it and an axis; it has no breakdown, 'Other' group, steps, sort or limit."
        ));
    }
    let has_source = layer
        .sql_source
        .as_deref()
        .is_some_and(|s| !s.trim().is_empty());
    match (layer.mart.trim().is_empty(), has_source) {
        (true, false) => return Err(format!("{who}: choose a table or a SQL source.")),
        (false, true) => {
            return Err(format!(
                "{who}: choose either a table or a SQL source, not both."
            ));
        }
        _ => {}
    }
    if layer.dimension.trim().is_empty() {
        return Err(format!("{who}: choose the dimension to match."));
    }
    if layer.measures.is_empty() {
        return Err(format!("{who}: choose at least one measure."));
    }
    let named = std::iter::once(&layer.dimension).chain(layer.measures.iter());
    if named
        .into_iter()
        .any(|c| crate::params::slot_param(c).is_some())
    {
        return Err(format!("{who}: a question takes no parameter."));
    }
    check_draw_axis(&layer.draw, &layer.axis, &who)?;
    if let Some(g) = &layer.grain
        && Grain::parse(g).is_none()
    {
        return Err(format!("{who}: '{g}' is not a time grain."));
    }
    if layer
        .label
        .as_deref()
        .is_some_and(|l| l.chars().count() > MAX_LABEL_CHARS)
    {
        return Err(format!(
            "{who}: the label is longer than {MAX_LABEL_CHARS} characters."
        ));
    }
    if let Some(filters) = &layer.filters {
        validate_filters(filters).map_err(|m| format!("{who}: filter: {m}"))?;
        if let Some(i) = filters.iter().position(|f| !f.is_active()) {
            return Err(format!(
                "{who}: filter {} has no value: choose a value or remove it.",
                i + 1
            ));
        }
    }
    if layer.joins.as_ref().is_some_and(|j| j.len() > MAX_JOINS) {
        return Err(format!("{who}: at most {MAX_JOINS} joined tables."));
    }
    if layer.sql_source.is_some() && layer.joins.as_ref().is_some_and(|j| !j.is_empty()) {
        return Err(format!("{who}: a SQL source takes no joins."));
    }
    Ok(())
}

/// What a chart that has layers (or says how its own series is drawn) may be,
/// before any source is read.
///
/// # Errors
///
/// A plain sentence for the person who saved the chart.
pub fn check_definition(input: &ChartInput) -> Result<(), String> {
    if let Some(draw) = &input.shape.draw
        && !matches!(draw.as_str(), "bar" | "line")
    {
        return Err("draw the chart's own series as 'bar' or 'line'.".to_owned());
    }
    if let Some(axis) = &input.shape.axis
        && !matches!(axis.as_str(), "left" | "right")
    {
        return Err("put the chart's own series on the 'left' or 'right' axis.".to_owned());
    }
    let layers = input.shape.layers_list();
    if layers.is_empty() {
        return Ok(());
    }
    if !takes_layers(input.kind) {
        return Err("questions can be added to a bar, line, area or combo chart only.".to_owned());
    }
    if layers.len() > MAX_LAYERS {
        return Err(format!(
            "a chart takes at most {MAX_LAYERS} more questions."
        ));
    }
    if input.breakdown.as_deref().is_some_and(|b| !b.is_empty())
        || input.shape.wants_other()
        || input.shape.steps.is_some()
    {
        return Err(
            "a chart that combines questions has no breakdown, 'Other' group or steps.".to_owned(),
        );
    }
    if input.shape.has_bins() {
        return Err(
            "a chart that combines questions cannot cut its dimension into bins.".to_owned(),
        );
    }
    for (i, layer) in layers.iter().enumerate() {
        check_layer(i + 1, layer)?;
    }
    Ok(())
}

/// Why a layer's dimension cannot be matched with the chart's. Each reason is
/// a fixed sentence written here, so a caller can show it without building text
/// from a value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KindMismatch {
    /// A date and a timestamp, with no time grain to cut both to one bucket.
    DateAgainstTimestamp,
    /// Dimensions of other kinds (a date with a text, a text with a number).
    Different,
}

impl KindMismatch {
    /// The sentence at save, which follows "question n:".
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::DateAgainstTimestamp => {
                "its date column and the chart's are not the same type (a date and a timestamp); choose a time grain so they line up."
            }
            Self::Different => {
                "its dimension must be of the same kind as the chart's: both dates, both text or both numbers."
            }
        }
    }

    /// The sentence at read, for a question that lined up when it was saved.
    #[must_use]
    pub fn no_longer(self) -> &'static str {
        match self {
            Self::DateAgainstTimestamp => {
                "this question no longer lines up with the chart: its date column and the chart's are not the same type (a date and a timestamp); choose a time grain so they line up."
            }
            Self::Different => {
                "this question no longer lines up with the chart: its dimension must be of the same kind as the chart's: both dates, both text or both numbers."
            }
        }
    }
}

/// Whether a layer's dimension can be matched with the chart's: both dates,
/// both text or both numbers. A date and a timestamp line up only through a
/// time grain, which cuts both to the same bucket text; ungrained they print
/// differently and would never meet.
///
/// # Errors
///
/// The [`KindMismatch`] when they cannot. An unknown kind (a joined column or a
/// calculated field that the caller could not type) is not refused here.
pub fn same_dimension_kind(
    base: Option<ColumnKind>,
    layer: Option<ColumnKind>,
    grained: bool,
) -> Result<(), KindMismatch> {
    let (Some(base), Some(layer)) = (base, layer) else {
        return Ok(());
    };
    let is_date = |k: ColumnKind| matches!(k, ColumnKind::Date | ColumnKind::DateTime);
    if base == layer || (is_date(base) && is_date(layer) && grained) {
        return Ok(());
    }
    if is_date(base) && is_date(layer) {
        return Err(KindMismatch::DateAgainstTimestamp);
    }
    Err(KindMismatch::Different)
}

/// The input a layer is built from, saved or read: the ordinary one-dimension
/// bar chart over the layer's source. `grain` is the chart's effective grain;
/// `kept` the filter that keeps the layer to the base's values, if any.
#[must_use]
pub fn layer_input(
    layer: &LayerDef,
    title: &str,
    grain: Option<&str>,
    order: &str,
    kept: Option<FilterDef>,
) -> ChartInput {
    let mut filters = layer.filters.clone().unwrap_or_default();
    filters.extend(kept);
    ChartInput {
        title: title.to_owned(),
        subtitle: None,
        mart: layer.mart.clone(),
        sql_source: layer.sql_source.clone(),
        kind: ChartKind::Bar,
        dimension: layer.dimension.clone(),
        measures: layer.measures.clone(),
        breakdown: None,
        map: None,
        lat: None,
        lon: None,
        aggregate: layer.aggregate.clone(),
        limit: Some(row_limit(grain).into()),
        order: Some(order.to_owned()),
        span: None,
        board: None,
        text: None,
        caption: None,
        target: None,
        click: None,
        grain: grain.map(str::to_owned),
        tables: crate::tables::TableFields::default(),
        shape: ShapeFields {
            filters: Some(filters).filter(|f| !f.is_empty()),
            joins: layer.joins.clone().filter(|j| !j.is_empty()),
            layer_query: true,
            ..ShapeFields::default()
        },
        style: crate::style::StyleFields::default(),
        types: Box::default(),
    }
}

/// The limit a layer is built with: all the buckets a grain may have, or the
/// largest limit of an ungrained chart plus one row, so the caller can tell a
/// layer that was cut from one that has exactly that many values
/// (`BI-13 review fix (SHOULD-FIX) R1`).
fn row_limit(grain: Option<&str>) -> u32 {
    if grain.is_some() {
        crate::grain::MAX_GRAIN_LIMIT
    } else {
        LAYER_ROW_LIMIT + 1
    }
}

/// The order a layer is asked in: by dimension when the base is, else by value.
#[must_use]
pub fn layer_order(base: &ChartInput) -> &'static str {
    if base.effective_order() == "none" {
        "none"
    } else {
        "desc"
    }
}

/// One layer as the synthetic chart the builder reads (`BI-13`). It has no
/// stored statement: `layer_query` makes the builder rebuild it from the
/// definition, and a caller treats an empty `sql` as "this definition cannot
/// be built" and reports that on the layer.
#[must_use]
pub fn layer_chart(
    base: &StoredChartSpec,
    index: usize,
    layer: &LayerDef,
    grain: Option<Grain>,
    kept: Option<FilterDef>,
) -> StoredChartSpec {
    let title = layer
        .label
        .clone()
        .unwrap_or_else(|| format!("question {}", index + 1));
    let def = layer_input(
        layer,
        &title,
        grain.map(Grain::as_str),
        layer_order(&base.def),
        kept,
    );
    let columns: Vec<String> = layer.measures.iter().cloned().collect();
    StoredChartSpec {
        spec: ChartSpec {
            id: format!("{}#{index}", base.spec.id),
            title,
            subtitle: None,
            kind: ChartKind::Bar,
            mart: layer.mart.clone(),
            sql_source: layer.sql_source.clone(),
            sql: String::new(),
            x: layer.dimension.clone(),
            y: ChartY::Multi(columns),
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
        source: base.source,
        board: base.board.clone(),
        def,
        has_year: false,
        created_by: None,
        created_at: None,
    }
}

/// The text a dimension value is matched by: a string as it is, a number as
/// JSON prints it. `None` for an empty (`NULL`) value, which no layer can be
/// asked for.
fn value_text(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// The distinct dimension values of the base result, in order, at most
/// [`MAX_KEPT_VALUES`].
#[must_use]
pub fn kept_values(rows: &[Value], dimension: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for row in rows {
        let Some(text) = row.get(dimension).and_then(value_text) else {
            continue;
        };
        if !out.contains(&text) {
            out.push(text);
        }
        if out.len() >= MAX_KEPT_VALUES {
            break;
        }
    }
    out
}

/// The `in` filter that keeps a layer to `values`, or `None` for none.
#[must_use]
pub fn kept_filter(dimension: &str, values: Vec<String>) -> Option<FilterDef> {
    (!values.is_empty()).then(|| FilterDef::in_values(dimension, values))
}

/// The first and last bucket of a grained base result, as the text the tile
/// carries (`YYYY-MM-DD` or `YYYY-MM-DD HH:MM:SS`, which sort as text).
#[must_use]
pub fn bucket_range(rows: &[Value], x: &str) -> Option<(String, String)> {
    let mut values = rows.iter().filter_map(|r| r.get(x).and_then(value_text));
    let first = values.next()?;
    Some(values.fold((first.clone(), first), |(lo, hi), v| {
        (lo.min(v.clone()), hi.max(v))
    }))
}

/// Keep the rows whose bucket lies in `range`; `true` when any row was
/// dropped. A row without a bucket is dropped: it cannot be placed on the
/// chart's axis.
pub fn trim_to_range(rows: &mut Vec<Value>, x: &str, range: &(String, String)) -> bool {
    let before = rows.len();
    rows.retain(|r| {
        r.get(x)
            .and_then(value_text)
            .is_some_and(|v| v >= range.0 && v <= range.1)
    });
    rows.len() != before
}

/// Whether the base result was cut by its limit, so layers are asked for its
/// values only: it filled the limit exactly.
#[must_use]
pub fn base_filled_limit(rows: usize, limit: Option<u32>) -> bool {
    let limit = limit.unwrap_or(20).clamp(1, 100) as usize;
    rows >= limit
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use serde_json::json;

    use super::*;

    fn input(extra: &Value) -> ChartInput {
        let mut base = json!({
            "title": "t", "kind": "bar", "mart": "mart_a", "dimension": "d",
            "measures": ["v"]
        });
        base.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        serde_json::from_value(base).unwrap()
    }

    fn layer() -> Value {
        json!({
            "mart": "mart_b", "dimension": "d", "measures": ["w"],
            "draw": "line", "axis": "right"
        })
    }

    #[test]
    fn a_chart_without_layers_has_no_layers_key_when_stored() {
        let stored = serde_json::to_value(input(&json!({}))).unwrap();
        assert!(stored.get("layers").is_none());
        assert!(stored.get("draw").is_none());
        assert!(stored.get("axis").is_none());
    }

    #[test]
    fn a_layer_round_trips_in_camel_case() {
        let chart = input(&json!({ "layers": [layer()], "draw": "bar", "axis": "left" }));
        assert_eq!(chart.shape.layers_list().len(), 1);
        let back = serde_json::to_value(&chart).unwrap();
        assert_eq!(back["layers"][0]["mart"], "mart_b");
        assert_eq!(back["layers"][0]["draw"], "line");
        assert!(back["layers"][0].get("extra").is_none());
        check_definition(&chart).unwrap();
    }

    #[test]
    fn five_questions_are_refused_with_a_plain_sentence() {
        let chart = input(&json!({ "layers": [layer(), layer(), layer(), layer(), layer()] }));
        assert_eq!(
            check_definition(&chart).unwrap_err(),
            "a chart takes at most 4 more questions."
        );
        let four = input(&json!({ "layers": [layer(), layer(), layer(), layer()] }));
        check_definition(&four).unwrap();
    }

    #[test]
    fn layers_on_a_pie_or_a_table_are_refused() {
        for kind in ["pie", "table", "kpi", "geomap", "stacked", "hbar"] {
            let chart = input(&json!({ "kind": kind, "layers": [layer()] }));
            assert!(
                check_definition(&chart)
                    .unwrap_err()
                    .contains("bar, line, area or combo"),
                "{kind}"
            );
        }
        for kind in ["bar", "line", "area", "combo"] {
            check_definition(&input(&json!({ "kind": kind, "layers": [layer()] }))).unwrap();
        }
    }

    #[test]
    fn a_breakdown_other_steps_or_bins_are_refused_beside_layers() {
        for extra in [
            json!({ "breakdown": "e" }),
            json!({ "other": true }),
            json!({ "steps": [{ "filter": [{ "column": "v", "op": "gt", "value": "1" }] }] }),
        ] {
            let mut body = extra.as_object().unwrap().clone();
            body.insert("layers".to_owned(), json!([layer()]));
            let chart = input(&Value::Object(body));
            assert!(
                check_definition(&chart)
                    .unwrap_err()
                    .contains("no breakdown, 'Other' group or steps"),
                "{extra}"
            );
        }
    }

    #[test]
    fn a_layer_with_a_breakdown_or_an_unknown_field_is_refused_not_dropped() {
        let mut l = layer();
        l["breakdown"] = json!("e");
        let chart = input(&json!({ "layers": [l] }));
        let why = check_definition(&chart).unwrap_err();
        assert!(why.starts_with("question 1:"), "{why}");
        assert!(why.contains("no breakdown"), "{why}");
        assert!(
            !why.contains("breakdown\""),
            "the field's value is not echoed"
        );
    }

    #[test]
    fn a_layer_needs_one_source_a_dimension_a_measure_a_draw_and_an_axis() {
        let cases = [
            (
                json!({ "mart": "", "sqlSource": null }),
                "choose a table or a SQL source",
            ),
            (json!({ "sqlSource": "s_1" }), "not both"),
            (json!({ "dimension": " " }), "choose the dimension"),
            (json!({ "measures": [] }), "at least one measure"),
            (json!({ "draw": "pie" }), "'bar' or 'line'"),
            (json!({ "axis": "up" }), "'left' or 'right'"),
            (json!({ "measures": ["{pick}"] }), "no parameter"),
            (json!({ "label": "x".repeat(61) }), "longer than 60"),
        ];
        for (patch, expect) in cases {
            let mut l = layer();
            l.as_object_mut()
                .unwrap()
                .extend(patch.as_object().unwrap().clone());
            let why = check_definition(&input(&json!({ "layers": [l] }))).unwrap_err();
            assert!(why.contains(expect), "{patch}: {why}");
        }
    }

    #[test]
    fn the_charts_own_draw_and_axis_are_checked_even_without_layers() {
        assert!(check_definition(&input(&json!({ "draw": "dots" }))).is_err());
        assert!(check_definition(&input(&json!({ "axis": "middle" }))).is_err());
        check_definition(&input(&json!({ "draw": "bar", "axis": "right" }))).unwrap();
    }

    #[test]
    fn kept_values_are_distinct_ordered_and_skip_empty() {
        let rows = vec![
            json!({ "d": "b", "v": 1 }),
            json!({ "d": "a", "v": 2 }),
            json!({ "d": "b", "v": 3 }),
            json!({ "d": null, "v": 4 }),
            json!({ "d": 7, "v": 5 }),
        ];
        assert_eq!(kept_values(&rows, "d"), vec!["b", "a", "7"]);
        assert!(kept_filter("d", Vec::new()).is_none());
    }

    #[test]
    fn a_grained_range_trims_a_layer_to_the_bases_buckets() {
        let base = vec![
            json!({ "m": "2026-02-01" }),
            json!({ "m": "2026-04-01" }),
            json!({ "m": "2026-03-01" }),
        ];
        let range = bucket_range(&base, "m").unwrap();
        assert_eq!(range, ("2026-02-01".to_owned(), "2026-04-01".to_owned()));
        let mut layer_rows = vec![
            json!({ "m": "2026-01-01", "n": 1 }),
            json!({ "m": "2026-02-01", "n": 2 }),
            json!({ "m": "2026-05-01", "n": 3 }),
            json!({ "m": null, "n": 4 }),
        ];
        assert!(trim_to_range(&mut layer_rows, "m", &range));
        assert_eq!(layer_rows, vec![json!({ "m": "2026-02-01", "n": 2 })]);
        assert!(bucket_range(&[], "m").is_none());
    }

    #[test]
    fn a_base_that_fills_its_limit_is_treated_as_cut() {
        assert!(base_filled_limit(20, None));
        assert!(!base_filled_limit(19, None));
        assert!(base_filled_limit(5, Some(5)));
        assert!(!base_filled_limit(4, Some(5)));
        assert!(base_filled_limit(100, Some(500)));
    }

    #[test]
    fn a_layer_is_read_as_a_one_dimension_bar_chart_that_always_rebuilds() {
        let def = layer_input(
            &serde_json::from_value(layer()).unwrap(),
            "L",
            Some("month"),
            "none",
            kept_filter("d", vec!["a".to_owned()]),
        );
        assert_eq!(def.kind, ChartKind::Bar);
        assert!(def.breakdown.is_none());
        assert!(def.shape.layer_query);
        assert!(def.uses_shaping());
        assert_eq!(def.grain.as_deref(), Some("month"));
        assert_eq!(def.limit_value(), Some(1000));
        assert_eq!(def.shape.own_filters().len(), 1);
        let plain = layer_input(
            &serde_json::from_value(layer()).unwrap(),
            "L",
            None,
            "desc",
            None,
        );
        // BI-13 review fix (SHOULD-FIX) R1: one row more than the layer may return, to tell a cut from an exact fit.
        assert_eq!(plain.limit_value(), Some(101));
        assert!(plain.shape.filters.is_none());
    }

    // ── the statements a layer is read with ─────────────────────────────

    use std::collections::HashMap;

    use crate::builder::{ReadContext, sql_with_filters_report};
    use crate::grain::{TimeContext, WeekStart};
    use crate::specs::ChartSource;

    fn base_stored() -> StoredChartSpec {
        let def = input(&json!({ "layers": [layer()] }));
        StoredChartSpec {
            spec: ChartSpec {
                id: "c_1".to_owned(),
                title: "t".to_owned(),
                subtitle: None,
                kind: ChartKind::Bar,
                mart: "mart_a".to_owned(),
                sql_source: None,
                sql: "SELECT 1".to_owned(),
                x: "d".to_owned(),
                y: ChartY::Single("v".to_owned()),
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
            board: "default".to_owned(),
            def,
            has_year: false,
            created_by: None,
            created_at: None,
        }
    }

    fn marts(kind: ColumnKind) -> HashMap<String, HashMap<String, ColumnKind>> {
        let mut cols = HashMap::new();
        cols.insert("d".to_owned(), kind);
        cols.insert("w".to_owned(), ColumnKind::Number);
        HashMap::from([("mart_b".to_owned(), cols)])
    }

    fn statement(
        grain: Option<Grain>,
        kind: ColumnKind,
        kept: Option<FilterDef>,
        filters: &[FilterDef],
    ) -> String {
        let base = base_stored();
        let l: LayerDef = serde_json::from_value(layer()).unwrap();
        let chart = layer_chart(&base, 0, &l, grain, kept);
        let time = TimeContext::new("Asia/Jakarta", WeekStart::Monday).unwrap();
        let read = ReadContext::new(&time);
        let out = sql_with_filters_report(&chart, &[], filters, &marts(kind), &read);
        assert!(!out.sql.is_empty(), "a layer is always rebuilt");
        out.sql
    }

    #[test]
    fn a_text_layer_asks_for_the_kept_values_only_as_bound_literals() {
        let hostile = "O'Brien\\ x') OR 1=1 --";
        let kept = kept_filter("d", vec!["a".to_owned(), hostile.to_owned()]);
        let sql = statement(None, ColumnKind::Text, kept, &[]);
        assert!(sql.contains("FROM serving.mart_b"), "{sql}");
        // Quotes are doubled and the backslash escaped inside one literal:
        // what follows the quote is still text, not SQL.
        assert!(
            sql.contains(r"d IN ('a','O''Brien\\ x'') OR 1=1 --')"),
            "{sql}"
        );
        assert!(sql.contains("LIMIT 101"), "{sql}");
    }

    #[test]
    fn a_grained_layer_is_cut_by_the_charts_grain_and_asks_for_every_bucket() {
        let sql = statement(Some(Grain::Month), ColumnKind::Date, None, &[]);
        assert!(sql.contains("date_trunc('month', d)"), "{sql}");
        assert!(sql.contains("LIMIT 1001"), "{sql}");
    }

    #[test]
    fn a_dashboard_filter_on_a_column_the_layer_lacks_does_not_apply_to_it() {
        let f = FilterDef::in_values("province", vec!["Bali".to_owned()]);
        let sql = statement(None, ColumnKind::Text, None, std::slice::from_ref(&f));
        assert!(!sql.contains("Bali"), "{sql}");
        let on_d = FilterDef::in_values("d", vec!["x".to_owned()]);
        let sql = statement(None, ColumnKind::Text, None, std::slice::from_ref(&on_d));
        assert!(sql.contains("'x'"), "{sql}");
    }

    #[test]
    fn the_layers_own_filters_are_always_applied_beside_the_kept_values() {
        let mut l = layer();
        l["filters"] = json!([{ "column": "d", "values": ["keep"] }]);
        let base = base_stored();
        let l: LayerDef = serde_json::from_value(l).unwrap();
        let chart = layer_chart(&base, 0, &l, None, kept_filter("d", vec!["a".to_owned()]));
        assert_eq!(chart.def.shape.own_filters().len(), 2);
    }

    #[test]
    fn a_dimension_of_another_kind_cannot_be_matched() {
        use super::same_dimension_kind as check;
        let (t, n, d, dt) = (
            Some(ColumnKind::Text),
            Some(ColumnKind::Number),
            Some(ColumnKind::Date),
            Some(ColumnKind::DateTime),
        );
        assert!(check(t, t, false).is_ok());
        assert!(check(d, d, false).is_ok());
        assert!(check(t, n, false).is_err());
        assert!(check(d, t, true).is_err());
        assert!(check(d, dt, true).is_ok());
        assert!(
            check(d, dt, false)
                .unwrap_err()
                .as_str()
                .contains("time grain")
        );
        assert!(check(None, t, false).is_ok());
    }

    // ── the save path against the real engine ───────────────────────────

    /// Run by hand with `LAYER_ENGINE_URL`, `LAYER_ENGINE_USER` and
    /// `LAYER_ENGINE_PASSWORD` set, `-- --ignored --nocapture`: it reads the
    /// schema of the demo tables of `serving` and writes nothing.
    async fn save(definition: Value) -> Result<StoredChartSpec, String> {
        let var = |k: &str| std::env::var(k).unwrap_or_else(|_| panic!("set {k}"));
        let ch = lakehouse_clickhouse::ChClient::new(
            var("LAYER_ENGINE_URL"),
            var("LAYER_ENGINE_USER"),
            var("LAYER_ENGINE_PASSWORD"),
        );
        let input: ChartInput = serde_json::from_value(definition).unwrap();
        let time = TimeContext::new("Asia/Jakarta", WeekStart::Monday).unwrap();
        crate::store::spec_from_input(&ch, &input, ChartSource::Ui, "ui", None, &time)
            .await
            .map_err(|e| e.to_string())
    }

    fn by_month(extra: &Value) -> Value {
        let mut def = json!({
            "title": "Visitors", "kind": "bar", "mart": "mart_demo_map_points",
            "dimension": "visit_date", "measures": ["visitors"], "aggregate": "sum",
            "grain": "month", "order": "none", "limit": 20,
            "layers": [{
                "mart": "mart_demo_events", "dimension": "event_time", "measures": ["orders"],
                "draw": "line", "axis": "right", "label": " Orders "
            }]
        });
        def.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        def
    }

    #[tokio::test]
    #[ignore = "needs the dev engine; see above"]
    async fn a_combined_chart_saves_with_the_statement_it_would_have_alone() {
        let combined = save(by_month(&json!({}))).await.unwrap();
        let mut alone = by_month(&json!({}));
        alone.as_object_mut().unwrap().remove("layers");
        let alone = save(alone).await.unwrap();
        eprintln!("SQL {}", combined.spec.sql);
        assert_eq!(
            combined.spec.sql, alone.spec.sql,
            "the chart's own statement is untouched"
        );
        let stored = combined.def.shape.layers_list();
        assert_eq!(stored.len(), 1);
        assert_eq!(
            stored[0].label.as_deref(),
            Some("Orders"),
            "the label is trimmed"
        );
        assert!(alone.def.shape.layers.is_none());
        let json = serde_json::to_value(&alone.def).unwrap();
        assert!(json.get("layers").is_none() && json.get("draw").is_none());
    }

    #[tokio::test]
    #[ignore = "needs the dev engine; see above"]
    async fn a_combined_chart_that_cannot_line_up_or_build_is_refused_in_plain_words() {
        let mut messages = Vec::new();
        for (name, patch) in [
            (
                "text against a date",
                json!({ "layers": [{ "mart": "mart_demo_map_points", "dimension": "provinsi", "measures": ["visitors"], "draw": "bar", "axis": "left" }] }),
            ),
            (
                "a timestamp against a date, no grain",
                json!({ "grain": null, "layers": [{ "mart": "mart_demo_events", "dimension": "event_time", "measures": ["orders"], "draw": "bar", "axis": "left" }] }),
            ),
            (
                "a missing column",
                json!({ "layers": [{ "mart": "mart_demo_events", "dimension": "event_time", "measures": ["nope"], "draw": "bar", "axis": "left" }] }),
            ),
            (
                "a missing table",
                json!({ "layers": [{ "mart": "mart_gone", "dimension": "d", "measures": ["m"], "draw": "bar", "axis": "left" }] }),
            ),
            (
                "a SQL source that is not there",
                json!({ "layers": [{ "sqlSource": "s_gone", "dimension": "d", "measures": ["m"], "draw": "bar", "axis": "left" }] }),
            ),
            (
                "an hour grain over a layer that holds dates",
                json!({ "mart": "mart_demo_events", "dimension": "event_time", "measures": ["orders"], "grain": "hour", "layers": [{ "mart": "mart_demo_map_points", "dimension": "visit_date", "measures": ["visitors"], "draw": "bar", "axis": "left" }] }),
            ),
            ("a breakdown", json!({ "breakdown": "category" })),
            ("a pie", json!({ "kind": "pie" })),
        ] {
            let err = save(by_month(&patch)).await.unwrap_err();
            eprintln!("{name}: {err}");
            messages.push(err);
        }
        assert!(messages[0].starts_with("question 1:"), "{}", messages[0]);
        assert!(messages[1].contains("time grain"), "{}", messages[1]);
        assert!(messages[2].starts_with("question 1:"), "{}", messages[2]);
        assert!(messages[3].starts_with("question 1:"), "{}", messages[3]);
        assert!(messages[4].contains("not found"), "{}", messages[4]);
        assert!(messages[5].starts_with("question 1:"), "{}", messages[5]);
        assert!(messages[6].contains("no breakdown"), "{}", messages[6]);
        assert!(
            messages[7].contains("bar, line, area or combo"),
            "{}",
            messages[7]
        );
    }
}
