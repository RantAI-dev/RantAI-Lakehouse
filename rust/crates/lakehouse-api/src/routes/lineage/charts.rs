//! Charts, SQL sources and metrics in the lineage graph, and the route that
//! answers "where does this chart come from" (`BI-14`, plan
//! `2026-10-24-bi-14-chart-lineage.md`, decisions D1 to D4).
//!
//! # Only what is stored
//!
//! A chart's start nodes are read from its stored definition, nothing is
//! inferred: its mart, the tables it joins, the marts or SQL sources of its
//! extra questions (layers), the SQL source it reads (whose tables are the
//! `column_lineage` rows recorded when the source was saved), and the metrics
//! its measures name (each metric is read from its own mart or SQL source).
//! From those the existing table graph (`lineage::assemble`, not a second
//! builder) already knows the Silver, Bronze, connector, upload and pipeline
//! nodes behind them.
//!
//! | Edge | Record |
//! | --- | --- |
//! | table -> chart | the stored chart's definition (`console.bi_chart`) |
//! | table -> SQL source | the source's column mappings (`column_lineage`, `sql_source:<id>`) |
//! | SQL source -> chart | the stored chart's definition |
//! | mart or SQL source -> metric | the metric's definition (`console.bi_metric`) |
//! | metric -> chart | the stored chart's definition |
//!
//! # Who sees what
//!
//! The charts are listed through the caller's [`Viewer`], so a chart in
//! somebody else's personal space is not in the graph and the chart route
//! answers 404 for it as for an unknown id. The column mappings of SQL
//! sources are `dashboard:read` rows and follow the same `Scope` as the rest
//! of the column lineage. A store that cannot be read is said in `coverage`
//! on the chart route; the lineage page leaves the dashboard objects out and
//! logs it (its answer is the table graph, which stays honest without them).

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};

use axum::http::HeaderMap;
use lakehouse_auth::Principal;
use lakehouse_bi::fields::{FieldCatalog, SourceKind};
use lakehouse_bi::metrics::MetricDef;
use lakehouse_bi::sources::SqlSource;
use lakehouse_bi::store::{self, ChartInput, StoredChartSpec};
use lakehouse_core::ApiError;
use lakehouse_store::column_lineage::{self, Direction};
use serde_json::{Value, json};

use super::columns::{scope_for, sql_source_key};
use super::{Assembled, Graph, table_id, upstream};
use crate::state::AppState;
use crate::upstream_error;

/// The node id of a stored chart.
fn chart_node(id: &str) -> String {
    format!("chart:{id}")
}

/// The node id of a SQL source.
fn source_node(id: &str) -> String {
    format!("sql_source:{id}")
}

/// The node id of a metric.
fn metric_node(id: &str) -> String {
    format!("metric:{id}")
}

/// What a stored chart reads, straight from its definition (D2).
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct Reads {
    /// Marts, without the `serving.` prefix.
    pub(super) marts: BTreeSet<String>,
    /// Dashboard SQL source ids.
    pub(super) sql_sources: BTreeSet<String>,
    /// Metric ids.
    pub(super) metrics: BTreeSet<String>,
}

fn bare_mart(mart: &str) -> &str {
    mart.strip_prefix("serving.").unwrap_or(mart)
}

/// The start nodes of a chart: its mart or SQL source, the tables it joins,
/// its layers' sources, and the metrics its measures name.
pub(super) fn reads_of(def: &ChartInput, metrics: &[MetricDef]) -> Reads {
    let mut reads = Reads::default();
    match def.sql_source.as_deref().filter(|s| !s.is_empty()) {
        Some(id) => {
            reads.sql_sources.insert(id.to_owned());
        }
        None if !def.mart.is_empty() => {
            reads.marts.insert(bare_mart(&def.mart).to_owned());
        }
        None => {}
    }
    for join in def.shape.joins_list() {
        if !join.mart.is_empty() {
            reads.marts.insert(bare_mart(&join.mart).to_owned());
        }
    }
    let by_name: HashMap<&str, &MetricDef> = metrics.iter().map(|m| (m.name.as_str(), m)).collect();
    let mut named: Vec<&str> = def.measures.metric_names().collect();
    for layer in def.shape.layers_list() {
        if let Some(id) = layer.sql_source.as_deref().filter(|s| !s.is_empty()) {
            reads.sql_sources.insert(id.to_owned());
        } else if !layer.mart.is_empty() {
            reads.marts.insert(bare_mart(&layer.mart).to_owned());
        }
        named.extend(layer.measures.metric_names());
    }
    for name in named {
        if let Some(metric) = by_name.get(name) {
            reads.metrics.insert(metric.id.clone());
        }
    }
    reads
}

/// The stored objects a graph is drawn from, read once.
struct Dash {
    charts: Vec<StoredChartSpec>,
    metrics: Vec<MetricDef>,
    sources: Vec<SqlSource>,
}

async fn load(
    ch: &lakehouse_clickhouse::ChClient,
    viewer: &lakehouse_bi::access::Viewer,
) -> Result<Dash, lakehouse_clickhouse::ChError> {
    Ok(Dash {
        charts: store::list_stored_charts(ch, viewer).await?,
        metrics: lakehouse_bi::metrics::list_metrics(ch).await?,
        sources: lakehouse_bi::sources::list_sources(ch).await?,
    })
}

/// The table keys (`serving.x`, `silver.x`, `bronze.x`) each SQL source in
/// `ids` reads, from its recorded column mappings.
async fn source_tables(
    state: &AppState,
    scope: lakehouse_store::column_lineage::Scope,
    ids: &BTreeSet<String>,
) -> HashMap<String, Vec<String>> {
    let mut out = HashMap::new();
    let Some(pool) = state.pg.as_deref() else {
        return out;
    };
    for id in ids {
        let key = sql_source_key(id);
        match column_lineage::touching_table(pool, &key, scope).await {
            Ok(rows) => {
                let tables: BTreeSet<String> = rows
                    .into_iter()
                    .filter(|r| r.to_table == key)
                    .filter_map(|r| r.from_table)
                    .collect();
                out.insert(id.clone(), tables.into_iter().collect());
            }
            Err(err) => {
                tracing::warn!(%err, "lineage: a SQL source's column mappings could not be read");
            }
        }
    }
    out
}

/// A table key as its graph node, made if the graph does not have it.
fn table_node(g: &mut Graph, key: &str) -> Option<String> {
    let (db, name) = key.split_once('.')?;
    let (id, kind) = match db {
        "serving" => (table_id("serving", name), "gold"),
        "silver" => (table_id("silver", name), "silver"),
        "bronze" => (format!("bronze:{name}"), "bronze"),
        _ => return None,
    };
    g.node(&id, key, kind);
    Some(id)
}

/// What the graph needs besides the chart to draw its sources and metrics.
struct Deps<'a> {
    metrics: HashMap<&'a str, &'a MetricDef>,
    sources: HashMap<&'a str, &'a SqlSource>,
    tables: HashMap<String, Vec<String>>,
}

impl<'a> Deps<'a> {
    fn new(dash: &'a Dash, tables: HashMap<String, Vec<String>>) -> Self {
        Self {
            metrics: dash.metrics.iter().map(|m| (m.id.as_str(), m)).collect(),
            sources: dash.sources.iter().map(|s| (s.id.as_str(), s)).collect(),
            tables,
        }
    }
}

const SOURCE_EVIDENCE: &str = "column mappings of the SQL source (column_lineage)";

/// The SQL source node, and an edge into it from each table it reads.
fn add_source(g: &mut Graph, id: &str, deps: &Deps<'_>) -> String {
    let node = source_node(id);
    let label = deps.sources.get(id).map_or(id, |s| s.title.as_str());
    g.node(&node, label, "sql_source");
    for key in deps.tables.get(id).into_iter().flatten() {
        if let Some(table) = table_node(g, key) {
            g.edge(&table, &node, "sql_source", SOURCE_EVIDENCE);
        }
    }
    node
}

/// The metric node, read from its own mart or SQL source.
fn add_metric(g: &mut Graph, id: &str, deps: &Deps<'_>) -> Option<String> {
    let metric = deps.metrics.get(id)?;
    let node = metric_node(id);
    g.node(&node, &metric.label, "metric");
    let evidence = format!("metric definition {} (console.bi_metric)", metric.name);
    match metric.source_kind {
        SourceKind::Mart => {
            if let Some(table) = table_node(g, &format!("serving.{}", bare_mart(&metric.source_id)))
            {
                g.edge(&table, &node, "metric", &evidence);
            }
        }
        SourceKind::SqlSource => {
            let source = add_source(g, &metric.source_id, deps);
            g.edge(&source, &node, "metric", &evidence);
        }
    }
    Some(node)
}

/// One stored chart with the edges from everything it reads.
fn add_chart(g: &mut Graph, chart: &StoredChartSpec, deps: &Deps<'_>) {
    let node = chart_node(&chart.spec.id);
    g.node(&node, &chart.spec.title, "chart");
    let evidence = format!("stored chart \"{}\" ({})", chart.spec.title, chart.spec.id);
    let metrics: Vec<MetricDef> = deps.metrics.values().map(|m| (*m).clone()).collect();
    let reads = reads_of(&chart.def, &metrics);
    for mart in &reads.marts {
        if let Some(table) = table_node(g, &format!("serving.{mart}")) {
            g.edge(&table, &node, "chart", &evidence);
        }
    }
    for id in &reads.sql_sources {
        let source = add_source(g, id, deps);
        g.edge(&source, &node, "chart", &evidence);
    }
    for id in &reads.metrics {
        if let Some(metric) = add_metric(g, id, deps) {
            g.edge(&metric, &node, "chart", &evidence);
        }
    }
}

/// Every SQL source `dash`'s charts and metrics read, so their tables are
/// read once.
fn sources_in_use(dash: &Dash) -> BTreeSet<String> {
    let mut ids = BTreeSet::new();
    for chart in &dash.charts {
        ids.extend(reads_of(&chart.def, &dash.metrics).sql_sources);
    }
    ids.extend(
        dash.metrics
            .iter()
            .filter(|m| m.source_kind == SourceKind::SqlSource)
            .map(|m| m.source_id.clone()),
    );
    ids
}

/// Adds the charts `principal` may see, with their SQL sources and metrics,
/// downstream of the tables they read (`BI-14` D2 and D5): `?focus=` on a
/// chart, or on a table, then shows them. A store that cannot be read leaves
/// them out (logged); the table graph is unchanged.
pub(super) async fn add_dashboard_nodes(
    state: &AppState,
    principal: &Principal,
    tenant: Option<uuid::Uuid>,
    shared: bool,
    g: &mut Graph,
) {
    // BI-14 review BLOCKER B1: chart, SQL source and metric names are
    // dashboard objects; without `dashboard:read` the graph stays tables only.
    if !principal.has("dashboard:read") {
        return;
    }
    // BI-19 B1 (with BI-14 D2): the caller's resolved reach, not the baseline;
    // a store that cannot answer shows no dashboard objects (fail closed).
    let viewer = match crate::routes::dashboard_access::usage_viewer_of(state, principal).await {
        Ok(viewer) => viewer,
        Err(err) => {
            tracing::warn!(?err, "lineage: the caller's dashboard reach could not be resolved");
            return;
        }
    };
    let dash = match load(&state.clickhouse, &viewer).await {
        Ok(dash) => dash,
        Err(err) => {
            tracing::warn!(?err, "lineage: the dashboard objects could not be read");
            return;
        }
    };
    let tables = source_tables(
        state,
        scope_for(principal, tenant, shared),
        &sources_in_use(&dash),
    )
    .await;
    let deps = Deps::new(&dash, tables);
    for chart in &dash.charts {
        add_chart(g, chart, &deps);
    }
}

// ── the chart route ─────────────────────────────────────────────────────

const NOT_FOUND: &str = "No such chart.";

/// Breadth-first distance of each node in `keep` from `from`, walking edges
/// backwards (towards what the chart is built from).
fn distances(g: &Graph, from: &str, keep: &BTreeSet<String>) -> BTreeMap<String, usize> {
    let mut seen = BTreeMap::from([(from.to_owned(), 0)]);
    let mut queue = VecDeque::from([from.to_owned()]);
    while let Some(node) = queue.pop_front() {
        let d = seen[&node];
        for (a, b, _, _) in &g.edges {
            if *b == node && keep.contains(a) && !seen.contains_key(a) {
                seen.insert(a.clone(), d + 1);
                queue.push_back(a.clone());
            }
        }
    }
    seen
}

/// `YYYY-MM-DD HH:MM` (UTC, as the query log gives it) as an ISO instant.
fn iso(at: &str) -> String {
    match at.split_once(' ') {
        Some((date, time)) => format!("{date}T{time}:00Z"),
        None => at.to_owned(),
    }
}

/// The last successful run of a connector, as an ISO instant.
async fn connector_refresh(state: &AppState, connector_id: &str) -> Option<String> {
    let pool = state.pg.as_deref()?;
    let runs = lakehouse_store::connector_run::list_runs(pool, connector_id, 20)
        .await
        .ok()?;
    let run = runs.into_iter().find(|r| r.status == "succeeded")?;
    let at = run.ended_at.unwrap_or(run.started_at);
    serde_json::to_value(at).ok()?.as_str().map(str::to_owned)
}

/// The words after "via" for a column mapping's producer.
fn via(producer_kind: &str) -> &'static str {
    match producer_kind {
        "build" => "table build",
        "pipeline" => "pipeline",
        "sql_source" => "SQL source",
        _ => "recorded mapping",
    }
}

/// Column names a formula mentions: `[Bracketed Names]` and bare identifiers
/// that are not a function call. Pure text reading, no compilation: a name
/// that is not a column is simply not in the mappings it is looked up in.
fn formula_columns(formula: &str) -> Vec<String> {
    const WORDS: [&str; 3] = ["and", "or", "not"];
    let mut out: Vec<String> = Vec::new();
    let mut rest = formula;
    let mut bare = String::new();
    while let Some(open) = rest.find('[') {
        bare.push_str(&rest[..open]);
        bare.push(' ');
        let Some(close) = rest[open..].find(']') else {
            rest = "";
            break;
        };
        let name = rest[open + 1..open + close].trim();
        if !name.is_empty() && !out.iter().any(|n| n == name) {
            out.push(name.to_owned());
        }
        rest = &rest[open + close + 1..];
    }
    bare.push_str(rest);
    let chars: Vec<char> = bare.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_ascii_alphabetic() || chars[i] == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            let call = chars[i..].iter().find(|c| !c.is_whitespace()) == Some(&'(');
            if !call && !WORDS.contains(&word.to_ascii_lowercase().as_str()) && !out.contains(&word)
            {
                out.push(word);
            }
        } else if chars[i].is_ascii_digit() {
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '.') {
                i += 1;
            }
        } else if chars[i] == '\'' || chars[i] == '"' {
            let quote = chars[i];
            i += 1;
            while i < chars.len() && chars[i] != quote {
                i += 1;
            }
            i += 1;
        } else {
            i += 1;
        }
    }
    out
}

/// The fields a chart shows, by the name the table or formula knows them by.
fn field_names(def: &ChartInput) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    let mut push = |n: &str| {
        if !n.is_empty() && !names.iter().any(|x| x == n) {
            names.push(n.to_owned());
        }
    };
    push(&def.dimension);
    if let Some(b) = def.breakdown.as_deref() {
        push(b);
    }
    for (column, _) in def.measures.entries() {
        push(column);
    }
    names
}

/// The table keys a chart's columns are looked up in: its marts and SQL sources.
fn lookup_keys(reads: &Reads) -> Vec<String> {
    reads
        .marts
        .iter()
        .map(|m| format!("serving.{m}"))
        .chain(reads.sql_sources.iter().map(|s| sql_source_key(s)))
        .collect()
}

async fn columns_behind(
    pool: &lakehouse_store::PgPool,
    scope: lakehouse_store::column_lineage::Scope,
    keys: &[String],
    column: &str,
) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    for key in keys {
        match column_lineage::walk(pool, key, column, Direction::Up, scope).await {
            Ok(rows) => {
                for row in rows {
                    if let (Some(table), Some(from)) = (row.from_table, row.from_column) {
                        let entry = json!({ "table": table, "column": from, "via": via(&row.producer_kind) });
                        if !out.contains(&entry) {
                            out.push(entry);
                        }
                    }
                }
            }
            Err(err) => {
                tracing::warn!(%err, "lineage: a chart column's mappings could not be read");
            }
        }
    }
    out
}

/// The `fields` of the answer: for each field of the chart, the columns it is
/// built from where mappings exist (D4).
async fn fields_json(
    state: &AppState,
    scope: lakehouse_store::column_lineage::Scope,
    chart: &StoredChartSpec,
    reads: &Reads,
    catalog: &FieldCatalog,
) -> Vec<Value> {
    let Some(pool) = state.pg.as_deref() else {
        return Vec::new();
    };
    let keys = lookup_keys(reads);
    let first = keys.first().cloned().unwrap_or_default();
    let mut out = Vec::new();
    for field in field_names(&chart.def) {
        // A calculated field or a metric is read from its formula; a plain
        // column from the mappings of the tables the chart reads.
        let formula = catalog
            .for_chart(&chart.def)
            .iter()
            .find(|f| f.name == field)
            .map(|f| f.formula.clone());
        let mut columns: Vec<Value> = Vec::new();
        if let Some(formula) = formula {
            for referenced in formula_columns(&formula) {
                columns.push(json!({ "table": first, "column": referenced, "via": "formula" }));
                for behind in columns_behind(pool, scope, &keys, &referenced).await {
                    if !columns.contains(&behind) {
                        columns.push(behind);
                    }
                }
            }
        } else {
            columns = columns_behind(pool, scope, &keys, &field).await;
        }
        out.push(json!({ "field": field, "columns": columns }));
    }
    out
}

/// The path as the answer's `steps`: nearest the chart first, each with the
/// refresh time the platform recorded or `notRecorded`. The flag says whether
/// any step had none.
async fn steps_json(
    state: &AppState,
    g: &Graph,
    keep: &BTreeSet<String>,
    dist: &BTreeMap<String, usize>,
) -> (Vec<Value>, bool) {
    let mut order: Vec<&String> = keep.iter().collect();
    order.sort_by_key(|id| (dist.get(*id).copied().unwrap_or(usize::MAX), (*id).clone()));
    let mut not_recorded = false;
    let mut steps: Vec<Value> = Vec::new();
    for id in order {
        let Some((label, kind)) = g.nodes.get(id) else {
            continue;
        };
        let mut step = json!({ "id": id, "kind": kind, "label": label });
        // `None`: the step has no refresh time of its own (a chart, a SQL
        // source, a metric). `Some(None)`: it should have one and none is on record.
        let refresh: Option<Option<(String, &'static str)>> = match *kind {
            "gold" | "silver" => Some(g.refreshed.get(id).map(|(at, source)| (iso(at), *source))),
            "connector" => {
                let connector = id.strip_prefix("connector:").unwrap_or(id);
                Some(
                    connector_refresh(state, connector)
                        .await
                        .map(|at| (at, "last successful connector run")),
                )
            }
            "bronze" | "upload" => Some(None),
            _ => None,
        };
        match refresh {
            Some(Some((at, source))) => {
                step["refreshedAt"] = json!(at);
                step["refreshSource"] = json!(source);
            }
            Some(None) => {
                step["notRecorded"] = json!(true);
                not_recorded = true;
            }
            None => {}
        }
        steps.push(step);
    }
    (steps, not_recorded)
}

/// `GET /api/dashboard/charts/{id}/lineage` (`BI-14` D1): the path from one
/// stored chart upstream to the connectors and uploads behind it, with what
/// is recorded of when each step was refreshed, and the columns behind each
/// field.
///
/// # Errors
///
/// [`ApiError::NotFound`] for a chart the caller cannot see (an unknown id and
/// somebody else's personal chart are the same answer); the errors of
/// [`super::assemble`]; a `ClickHouse` failure through `upstream_error`.
pub(crate) async fn read(
    state: &AppState,
    principal: &Principal,
    headers: &HeaderMap,
    chart_id: &str,
) -> Result<Value, ApiError> {
    // BI-14 review BLOCKER B1: without `dashboard:read` no chart is named, so
    // the answer is the one an unknown id gets.
    if !principal.has("dashboard:read") {
        return Err(ApiError::NotFound(NOT_FOUND.to_owned()));
    }
    let ch = &state.clickhouse;
    // BI-19 B1: the resolved viewer, not the baseline (a chart on a dashboard the
    // rules leave out reads as an unknown id).
    let viewer = crate::routes::dashboard_access::usage_viewer_of(state, principal).await?;
    let dash = load(ch, &viewer)
        .await
        .map_err(|err| upstream_error::ch_error(&upstream_error::DATABASE, &err))?;
    let chart = dash
        .charts
        .iter()
        .find(|c| c.spec.id == chart_id)
        .ok_or_else(|| ApiError::NotFound(NOT_FOUND.to_owned()))?;
    let Assembled {
        mut g,
        tenant,
        shared,
    } = super::assemble(state, principal, headers).await?;
    let scope = scope_for(principal, tenant, shared);
    let reads = reads_of(&chart.def, &dash.metrics);
    let mut wanted = reads.sql_sources.clone();
    for id in &reads.metrics {
        if let Some(m) = dash.metrics.iter().find(|m| &m.id == id)
            && m.source_kind == SourceKind::SqlSource
        {
            wanted.insert(m.source_id.clone());
        }
    }
    let deps = Deps::new(&dash, source_tables(state, scope, &wanted).await);
    add_chart(&mut g, chart, &deps);

    let start = chart_node(&chart.spec.id);
    let keep = upstream(&g, std::slice::from_ref(&start));
    let dist = distances(&g, &start, &keep);
    let (steps, not_recorded) = steps_json(state, &g, &keep, &dist).await;
    let edges: Vec<Value> = g
        .edges
        .iter()
        .enumerate()
        .filter(|(_, (from, to, _, _))| keep.contains(from) && keep.contains(to))
        .map(|(i, (from, to, kind, evidence))| {
            json!({ "id": format!("e{i}"), "from": from, "to": to, "kind": kind, "evidence": evidence })
        })
        .collect();
    let catalog = match lakehouse_bi::fields::list_catalog_fields(ch).await {
        Ok(found) => FieldCatalog::from_fields(found),
        Err(err) => {
            tracing::warn!(?err, "lineage: the calculated fields could not be read");
            g.coverage.push(
                "calculated fields and metrics are not traced: their formulas could not be read"
                    .to_owned(),
            );
            FieldCatalog::from_fields(Vec::new())
        }
    };
    let fields = fields_json(state, scope, chart, &reads, &catalog).await;
    if not_recorded {
        g.coverage.push(
            "a step marked not recorded has no refresh time on record (Bronze tables, uploads and tables not built by a logged statement)"
                .to_owned(),
        );
    }
    Ok(json!({
        "chart": { "id": chart.spec.id, "title": chart.spec.title },
        "steps": steps,
        "edges": edges,
        "fields": fields,
        "coverage": g.coverage,
        "supported": true,
    }))
}

/// The handler of `GET /api/dashboard/charts/{id}/lineage` (policy
/// `lineage:read`, owner decision Q6).
///
/// # Errors
///
/// As [`read`].
pub(crate) async fn chart_lineage(
    axum::extract::State(state): axum::extract::State<AppState>,
    axum::Extension(principal): axum::Extension<Principal>,
    headers: HeaderMap,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> crate::error::ApiResult<crate::json::ApiJson<Value>> {
    Ok(crate::json::ApiJson(
        read(&state, &principal, &headers, &id).await?,
    ))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use serde_json::json;

    use super::*;

    fn def(extra: &Value) -> ChartInput {
        let mut base = json!({ "title": "t", "mart": "mart_visits", "kind": "bar", "dimension": "province", "measures": ["visitors"] });
        base.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        serde_json::from_value(base).unwrap()
    }

    fn metric(id: &str, name: &str, kind: &str, source: &str) -> MetricDef {
        serde_json::from_value(json!({
            "id": id, "name": name, "label": name, "sourceKind": kind, "sourceId": source,
            "formula": "Sum([visitors])", "description": "", "createdBy": "x", "updatedAt": "2026-01-01 00:00:00",
        }))
        .unwrap()
    }

    #[test]
    fn a_charts_start_nodes_are_its_mart_joins_layers_and_metrics() {
        let reads = reads_of(&def(&json!({})), &[]);
        assert_eq!(reads.marts.iter().collect::<Vec<_>>(), ["mart_visits"]);
        let joined = def(
            &json!({ "joins": [{ "mart": "serving.mart_rooms", "type": "left", "on": [{ "left": "province", "op": "=", "right": "province" }] }] }),
        );
        assert_eq!(reads_of(&joined, &[]).marts.len(), 2);
        let from_source = def(&json!({ "mart": "", "sqlSource": "s_1" }));
        let reads = reads_of(&from_source, &[]);
        assert!(reads.marts.is_empty() && reads.sql_sources.contains("s_1"));
        let with_metric = def(&json!({ "measures": [{ "metric": "total_visitors" }] }));
        let metrics = [metric("m_1", "total_visitors", "mart", "mart_visits")];
        assert!(reads_of(&with_metric, &metrics).metrics.contains("m_1"));
        assert!(reads_of(&with_metric, &[]).metrics.is_empty());
    }

    #[test]
    fn the_graph_gets_chart_source_and_metric_nodes_with_evidence() {
        let mut g = Graph::default();
        let sources: Vec<SqlSource> = Vec::new();
        let metrics = vec![metric("m_1", "total", "mart", "mart_visits")];
        let dash = Dash {
            charts: Vec::new(),
            metrics,
            sources,
        };
        let deps = Deps::new(
            &dash,
            HashMap::from([("s_1".to_owned(), vec!["serving.mart_rooms".to_owned()])]),
        );
        let chart_def =
            def(&json!({ "mart": "", "sqlSource": "s_1", "measures": [{ "metric": "total" }] }));
        let spec: store::ChartSpec = serde_json::from_value(json!({
            "id": "c_1", "title": "Visitors", "kind": "bar", "mart": "", "sql": "SELECT 1",
            "x": "province", "y": "visitors",
        }))
        .unwrap();
        let stored = StoredChartSpec {
            spec,
            source: lakehouse_bi::specs::ChartSource::Ui,
            board: "default".to_owned(),
            def: chart_def,
            has_year: false,
            created_by: None,
            created_at: None,
        };
        add_chart(&mut g, &stored, &deps);
        assert_eq!(g.nodes["chart:c_1"].1, "chart");
        assert_eq!(g.nodes["sql_source:s_1"].1, "sql_source");
        assert_eq!(g.nodes["metric:m_1"].1, "metric");
        let edge = |from: &str, to: &str| {
            g.edges
                .iter()
                .any(|(a, b, _, e)| a == from && b == to && !e.is_empty())
        };
        assert!(edge("table:serving.mart_rooms", "sql_source:s_1"));
        assert!(edge("sql_source:s_1", "chart:c_1"));
        assert!(edge("table:serving.mart_visits", "metric:m_1"));
        assert!(edge("metric:m_1", "chart:c_1"));
        // Upstream of the chart reaches the marts; nothing downstream is walked.
        let keep = upstream(&g, &["chart:c_1".to_owned()]);
        assert!(
            keep.contains("table:serving.mart_rooms") && keep.contains("table:serving.mart_visits")
        );
    }

    #[test]
    fn a_formula_names_its_columns_and_not_its_functions() {
        assert_eq!(
            formula_columns("Sum([revenue]) / CountDistinct([customer id])"),
            ["revenue", "customer id"]
        );
        assert_eq!(formula_columns("revenue - cost"), ["revenue", "cost"]);
        assert_eq!(
            formula_columns("If(price > 10 and 'x y' = a, 1, 2)"),
            ["price", "a"]
        );
        assert_eq!(iso("2026-09-22 10:03"), "2026-09-22T10:03:00Z");
    }
}
