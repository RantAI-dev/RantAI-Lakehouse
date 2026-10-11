//! `BI-13`: running the questions of a combined chart.
//!
//! A chart with `layers` is one base question and up to four more, each over a
//! source of its own. The base is read exactly as before; this module asks
//! each layer as the one-dimension bar chart it would be alone
//! ([`lakehouse_bi::layers::layer_chart`]) through the very same functions a
//! tile uses: `stored_chart_sql` (the dashboard's filters on the columns the
//! layer's source has, the layer's own filters, the chart's grain) and
//! `try_run_spec_sql` (the guard and the role rewrite of the layer's own
//! tables). The answers ride in the tile as `layers`, beside the base rows; the
//! console puts them on one axis.
//!
//! # A layer never fails the tile
//!
//! Every way a layer can fail (its source gone or unreadable, a dimension that
//! no longer lines up, a refusal by the role rewrite, a database error) ends
//! in that layer's own `error`, a fixed sentence with a reference when the
//! database is the cause (`upstream_error`, SEC-11). The base rows, the other
//! layers and the board still arrive. The lesson is the phase 1 embed one
//! (PR #104): an optional read that fails must not take the whole payload down,
//! so even the list of SQL sources a layer needs is read here, once, and its
//! failure is the failure of the layers that need it, not of the board.
//!
//! # What the base decides
//!
//! The base question's sort and limit decide which dimension values the chart
//! shows. A layer is asked for those values only when the base filled its
//! limit on a text or number dimension (an `in` filter built from the base
//! result); when the base is a time grain cut to its latest buckets, the layer
//! is asked for every bucket and trimmed to the base's range. Otherwise it is
//! asked for its own values and the chart shows the union, with a gap where one
//! question lacks a value. The condition only narrows the layer's own
//! statement, which is governed on its own (`lakehouse_bi::layers`).

use std::collections::HashMap;

use lakehouse_bi::builder::{ReadContext, RelationColumns};
use lakehouse_bi::filters::{ColumnKind, FilterDef};
use lakehouse_bi::grain::Grain;
use lakehouse_bi::layers::{self, LayerDef};
use lakehouse_bi::sources::SqlSource;
use lakehouse_bi::specs::ChartKind;
use lakehouse_bi::store::StoredChartSpec;
use lakehouse_clickhouse::ChClient;
use serde_json::{Value, json};

use crate::routes::support::{
    SpecRunFailure, annotate_grain, mart_columns, sources_for, stored_chart_sql, try_run_spec_sql,
};
use crate::upstream_error;

/// The SQL sources the layers of a board read, by id. `None` when the read
/// failed: the layers that need a source then say so, and the board renders.
pub(crate) type LayerSources = Option<HashMap<String, SqlSource>>;

/// The SQL sources the layers of `charts` read: one `console.bi_source` read,
/// and none when no layer uses a source. A failed read is logged under a
/// reference and leaves `None`.
pub(crate) async fn layer_sources<'a>(
    ch: &ChClient,
    charts: impl IntoIterator<Item = &'a StoredChartSpec>,
) -> LayerSources {
    let wanted = charts.into_iter().any(|c| {
        c.def
            .shape
            .layers_list()
            .iter()
            .any(|l| l.sql_source.is_some())
    });
    if !wanted {
        return Some(HashMap::new());
    }
    match lakehouse_bi::sources::list_sources(ch).await {
        Ok(found) => Some(found.into_iter().map(|s| (s.id.clone(), s)).collect()),
        Err(err) => {
            let _logged = upstream_error::report_ch(&upstream_error::DATABASE, &err);
            None
        }
    }
}

/// How one statement of a layer is run. The tile path sends it through the
/// role rewrite ([`RoleRunner`]); the section 4 tests send it through the real
/// `enforce` and the real engine.
pub(crate) trait LayerRunner {
    /// Run `sql` as the viewer.
    async fn run(&self, sql: &str) -> Result<Value, SpecRunFailure>;
}

/// The viewer's roles, placeholders and the policy obligations: the same
/// arguments every tile statement is run with.
pub(crate) struct RoleRunner<'a> {
    pub ch: &'a ChClient,
    pub roles: &'a [String],
    pub placeholders: &'a crate::sql_rewrite::PlaceholderValues,
    pub obligations: &'a crate::policy_engine::PolicyEngineObligations<'a>,
}

impl LayerRunner for RoleRunner<'_> {
    async fn run(&self, sql: &str) -> Result<Value, SpecRunFailure> {
        try_run_spec_sql(
            self.ch,
            sql,
            self.roles,
            self.placeholders,
            self.obligations,
        )
        .await
    }
}

/// What a board read gives the layers of its tiles.
pub(crate) struct LayerContext<'a, R: LayerRunner> {
    pub runner: &'a R,
    pub years: &'a [i64],
    pub filters: &'a [FilterDef],
    pub cols: &'a HashMap<String, RelationColumns>,
    /// The sources of the charts themselves (to type a base dimension).
    pub chart_sources: &'a HashMap<String, SqlSource>,
    pub layer_sources: &'a LayerSources,
    pub read: &'a ReadContext<'a>,
}

/// How the base result narrows a layer.
enum Matching {
    /// The layer's own values.
    Everything,
    /// The base's values only (the base filled its limit).
    Values(Vec<String>),
    /// The base's buckets from first to last (a grain cut to its latest).
    Range((String, String)),
}

fn matching(base: &StoredChartSpec, tile: &Value, grain: Option<Grain>) -> Matching {
    let rows: &[Value] = tile
        .get("rows")
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice);
    let dimension = base.def.dimension.as_str();
    match grain {
        Some(g) if g.is_truncation() => {
            let cut = tile.get("truncated") == Some(&json!(true));
            match cut.then(|| layers::bucket_range(rows, dimension)).flatten() {
                Some(range) => Matching::Range(range),
                None => Matching::Everything,
            }
        }
        None if layers::base_filled_limit(rows.len(), base.def.limit_value()) => {
            let values = layers::kept_values(rows, dimension);
            if values.is_empty() {
                Matching::Everything
            } else {
                Matching::Values(values)
            }
        }
        // A part of the date (an hour of the day) has its whole range; a base
        // that did not fill its limit shows every value it has.
        Some(_) | None => Matching::Everything,
    }
}

/// The kind of `dimension` in the relation a chart or a layer reads, when the
/// relation's columns say (a joined column or a calculated field is not among
/// them, which skips the comparison).
fn relation_kind(
    (mart, sql_source): (&str, Option<&str>),
    dimension: &str,
    cols: &HashMap<String, RelationColumns>,
    sources: &HashMap<String, SqlSource>,
) -> Option<ColumnKind> {
    match sql_source {
        Some(id) => sources.get(id)?.column_kinds().get(dimension).copied(),
        None => cols.get(mart)?.get(dimension).copied(),
    }
}

/// Add `layers` to the tile of `base`: one entry per layer, in order. A tile
/// that carries an error, and a chart without layers, are left alone.
///
/// `grain` is the grain the base's rows were cut with (its own, or the
/// dashboard's switch), which every layer takes.
pub(crate) async fn annotate_layers<R: LayerRunner>(
    ctx: &LayerContext<'_, R>,
    tile: &mut Value,
    base: &StoredChartSpec,
    grain: Option<Grain>,
) {
    let wanted = base.def.shape.layers_list();
    if wanted.is_empty() || !tile.get("rows").is_some_and(Value::is_array) {
        return;
    }
    let how = matching(base, tile, grain);
    let base_kind = relation_kind(
        (&base.def.mart, base.def.sql_source.as_deref()),
        &base.def.dimension,
        ctx.cols,
        ctx.chart_sources,
    );
    let mut out = Vec::with_capacity(wanted.len());
    for (i, layer) in wanted.iter().enumerate() {
        out.push(one(ctx, base, (i, layer), (grain, base_kind), &how).await);
    }
    if let Value::Object(tile) = tile {
        tile.insert("layers".to_owned(), Value::Array(out));
    }
}

/// One layer's entry.
async fn one<R: LayerRunner>(
    ctx: &LayerContext<'_, R>,
    base: &StoredChartSpec,
    (index, layer): (usize, &LayerDef),
    (grain, base_kind): (Option<Grain>, Option<ColumnKind>),
    how: &Matching,
) -> Value {
    let mut entry = json!({
        "label": layer.label.clone().unwrap_or_else(|| layer.source_name()),
        "draw": layer.draw,
        "axis": layer.axis,
        "source": layer.source_name(),
        // The column the layer's rows are matched on, which need not be named
        // like the chart's.
        "dimension": layer.dimension,
        "columns": [],
        "rows": [],
    });
    let outcome = ask(ctx, base, (index, layer), (grain, base_kind), how).await;
    if let Value::Object(map) = &mut entry {
        match outcome {
            Ok(answer) => map.extend(answer),
            Err(failure) => map.extend(failure),
        }
    }
    entry
}

/// A layer's failure as the fields of its entry: `error`, and `errorId` when
/// the database was the cause.
type Failure = serde_json::Map<String, Value>;

fn plain(message: impl Into<String>) -> Failure {
    let mut map = Failure::new();
    map.insert("error".to_owned(), Value::String(message.into()));
    map
}

fn failure_of(failure: SpecRunFailure) -> Failure {
    let value = match failure {
        SpecRunFailure::Refused(message) => json!({ "error": message }),
        // SEC-11: the database's text goes to the log under the reference.
        SpecRunFailure::Clickhouse(err) => {
            upstream_error::report_ch(&upstream_error::TILE, &err).to_json()
        }
    };
    match value {
        Value::Object(map) => map,
        _ => plain("this question could not be read."),
    }
}

/// The relation a layer reads must be there, and its dimension must still line
/// up with the chart's and fit its grain. `Err` is the layer's sentence.
fn relation_checks<R: LayerRunner>(
    ctx: &LayerContext<'_, R>,
    layer: &LayerDef,
    (grain, base_kind): (Option<Grain>, Option<ColumnKind>),
) -> Result<(), Failure> {
    let layer_sources: HashMap<String, SqlSource>;
    let sources = match (&layer.sql_source, ctx.layer_sources) {
        (Some(_), None) => {
            return Err(plain("this question's SQL source could not be read."));
        }
        (Some(id), Some(found)) => {
            if !found.contains_key(id) {
                return Err(plain("this question's SQL source no longer exists."));
            }
            found
        }
        (None, _) => {
            if !ctx.cols.contains_key(&layer.mart) {
                return Err(plain("this question's source is not available."));
            }
            layer_sources = HashMap::new();
            &layer_sources
        }
    };
    let kind = relation_kind(
        (&layer.mart, layer.sql_source.as_deref()),
        &layer.dimension,
        ctx.cols,
        sources,
    );
    layers::same_dimension_kind(base_kind, kind, grain.is_some())
        .map_err(|mismatch| plain(mismatch.no_longer()))?;
    if let (Some(g), Some(k)) = (grain, kind)
        && !g.supports(ChartKind::Bar, Some(k))
    {
        return Err(plain(format!(
            "this question's dimension cannot be cut by {}.",
            g.as_str()
        )));
    }
    Ok(())
}

/// Build and run one layer's statement; `Ok` is the entry's rows and flags.
async fn ask<R: LayerRunner>(
    ctx: &LayerContext<'_, R>,
    base: &StoredChartSpec,
    (index, layer): (usize, &LayerDef),
    checks: (Option<Grain>, Option<ColumnKind>),
    how: &Matching,
) -> Result<Failure, Failure> {
    let grain = checks.0;
    relation_checks(ctx, layer, checks)?;
    let kept = match how {
        Matching::Values(values) => layers::kept_filter(&layer.dimension, values.clone()),
        Matching::Everything | Matching::Range(_) => None,
    };
    let chart = layers::layer_chart(base, index, layer, grain, kept);
    // The dashboard's grain switch is already in `grain`; a second one here
    // would move a layer the base could not follow.
    let read = ReadContext {
        grain: None,
        ..*ctx.read
    };
    let none = HashMap::new();
    let sources = ctx.layer_sources.as_ref().unwrap_or(&none);
    let filtered = stored_chart_sql(&chart, ctx.years, ctx.filters, ctx.cols, sources, &read)
        .map_err(plain)?;
    // A layer has no stored statement: an empty one means the definition could
    // not be built, which must not read as "no rows".
    if filtered.sql.is_empty() {
        return Err(plain("this question's definition is invalid."));
    }
    let mut answer = ctx.runner.run(&filtered.sql).await.map_err(failure_of)?;
    annotate_grain(&mut answer, &filtered, &chart);
    let Value::Object(mut answer) = answer else {
        return Err(plain("this question could not be read."));
    };
    if let (Matching::Range(range), Some(Value::Array(rows))) = (how, answer.get_mut("rows")) {
        layers::trim_to_range(rows, &layer.dimension, range);
    }
    answer.remove("grain");
    answer.remove("grainColumn");
    if !filtered.skipped.is_empty() {
        answer.insert("filtersSkipped".to_owned(), json!(filtered.skipped));
    }
    Ok(answer)
}

/// The builder's preview: the questions of a chart that is not saved yet, asked
/// the way a dashboard asks them (no dashboard filters, the chart's own
/// grain). Every read this needs is optional for the preview too: a failure
/// leaves the catalog, the columns or the sources empty, and the questions that
/// needed them say so.
pub(crate) async fn annotate_preview_layers(
    ch: &ChClient,
    tile: &mut Value,
    spec: &StoredChartSpec,
    clock: &lakehouse_bi::grain::TimeContext,
    runner: &RoleRunner<'_>,
) {
    if spec.def.shape.layers_list().is_empty() {
        return;
    }
    let cols = match mart_columns(ch).await {
        Ok(found) => found,
        Err(err) => {
            let _logged = upstream_error::report_ch(&upstream_error::DATABASE, &err);
            HashMap::new()
        }
    };
    let catalog = match lakehouse_bi::fields::list_fields(ch).await {
        Ok(found) => lakehouse_bi::fields::FieldCatalog::from_fields(found),
        Err(err) => {
            let _logged = upstream_error::report_ch(&upstream_error::DATABASE, &err);
            lakehouse_bi::fields::FieldCatalog::default()
        }
    };
    let chart_sources = sources_for(ch, [spec]).await.unwrap_or_default();
    let layer_sources = layer_sources(ch, [spec]).await;
    let read = ReadContext {
        time: clock,
        grain: None,
        fields: &catalog,
    };
    let ctx = LayerContext {
        runner,
        years: &[],
        filters: &[],
        cols: &cols,
        chart_sources: &chart_sources,
        layer_sources: &layer_sources,
        read: &read,
    };
    let grain = spec.def.grain.as_deref().and_then(Grain::parse);
    annotate_layers(&ctx, tile, spec, grain).await;
}

// BI-13: the role rewrite and the real engine under the questions of a combined
// chart. A test-only module of its own file; declared here, last, because
// `routes/mod.rs` may have no non-test item after a `#[cfg(test)]` module
// (`tests/sec11_guard.rs`), as `dashboard.rs` does for its own.
#[cfg(test)]
#[path = "layers_rewrite.rs"]
mod layers_rewrite;
