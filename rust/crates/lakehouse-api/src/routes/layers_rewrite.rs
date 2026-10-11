//! `BI-13`: the questions of a combined chart under the real role rewrite and
//! the real engine.
//!
//! Every layer statement is built by the dashboard's own functions
//! (`annotate_layers` over `stored_chart_sql`), passed through the real
//! [`enforce`] with real obligations (a row filter, a mask, a table the role
//! cannot read), and run on a real `ClickHouse`. Those tests are `#[ignore]`d
//! because they need that engine; run them by hand with
//!
//! ```text
//! LAYER_ENGINE_URL=http://localhost:8123 LAYER_ENGINE_USER=... \
//! LAYER_ENGINE_PASSWORD=... cargo test -p lakehouse-api --lib \
//!   layers_rewrite -- --ignored --test-threads=1 --nocapture
//! ```
//!
//! They only read: the demo tables of `serving`, and `SELECT` statements over
//! literal rows for the escaping checks (a SQL source needs no table). The raw
//! and the rewritten statements and the results are printed, so they can be
//! quoted (plan section 4). The last group of tests needs no engine: a stub
//! answers, to pin how a layer fails and what it leaves alone.

#![cfg(test)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashMap;

use lakehouse_bi::builder::{ReadContext, RelationColumns};
use lakehouse_bi::fields::FieldCatalog;
use lakehouse_bi::filters::ColumnKind;
use lakehouse_bi::grain::TimeContext;
use lakehouse_bi::sources::{SourceColumn, SqlSource};
use lakehouse_bi::specs::ChartSource;
use lakehouse_bi::store::{ChartInput, ChartSpec, StoredChartSpec};
use lakehouse_clickhouse::ChClient;
use serde_json::{Value, json};
use sqlparser::dialect::ClickHouseDialect;

use crate::routes::layers::{LayerContext, LayerRunner, annotate_layers};
use crate::routes::support::{SpecRunFailure, annotate_grain, stored_chart_sql};
use crate::sql_rewrite::{
    NoViews, ObligationsSource, PlaceholderValues, RewriteError, TableObligations, enforce,
};

const POINTS: &str = "mart_demo_map_points";
const EVENTS: &str = "mart_demo_events";
const CAPACITY: &str = "mart_demo_province_capacity";

/// What one governed table of the test role carries.
struct Governed {
    table: &'static str,
    columns: &'static [&'static str],
    mask: &'static [&'static str],
    row_filter: Option<&'static str>,
    /// The role cannot read the table: its real columns are unknown to the
    /// rewrite, which then refuses (`UnprovableSubstitution`).
    unreadable: bool,
}

struct Policies(Vec<Governed>);

impl ObligationsSource for Policies {
    fn obligations_for(&self, table: &str, _roles: &[String]) -> Option<TableObligations> {
        self.0
            .iter()
            .find(|g| table == format!("serving.{}", g.table))
            .map(|g| TableObligations {
                mask: g.mask.iter().map(|m| (*m).to_owned()).collect(),
                row_filter: g.row_filter.map(str::to_owned),
                real_columns: (!g.unreadable)
                    .then(|| g.columns.iter().map(|c| (*c).to_owned()).collect()),
            })
    }
    fn has_any_obligation(&self, _roles: &[String]) -> bool {
        !self.0.is_empty()
    }
}

/// Runs a statement as the test role: the real rewrite, then the real engine.
struct Engine {
    ch: ChClient,
    policies: Policies,
}

impl Engine {
    fn from_env(policies: Vec<Governed>) -> Self {
        let var = |k: &str| std::env::var(k).unwrap_or_else(|_| panic!("set {k}"));
        Self {
            ch: ChClient::new(
                var("LAYER_ENGINE_URL"),
                var("LAYER_ENGINE_USER"),
                var("LAYER_ENGINE_PASSWORD"),
            ),
            policies: Policies(policies),
        }
    }

    /// A statement straight on the engine, no rewrite: the "direct single
    /// query" a series is compared with.
    async fn direct(&self, sql: &str) -> Vec<Value> {
        let r = self.ch.query(sql, None).await.unwrap();
        r.data.into_iter().map(Value::Object).collect()
    }
}

impl LayerRunner for Engine {
    async fn run(&self, sql: &str) -> Result<Value, SpecRunFailure> {
        let rewritten = enforce(
            sql,
            &ClickHouseDialect {},
            &["analyst".to_owned()],
            &PlaceholderValues::none(),
            &self.policies,
            &NoViews,
        )
        .map_err(|err: RewriteError| {
            SpecRunFailure::Refused(crate::policy_engine::refusal_message(&err))
        })?;
        eprintln!(
            "RAW      {}\nREWRITTEN {}",
            one_line(sql),
            one_line(&rewritten)
        );
        let r = self
            .ch
            .query(&rewritten, None)
            .await
            .map_err(SpecRunFailure::Clickhouse)?;
        let columns: Vec<String> = r.meta.iter().map(|m| m.name.clone()).collect();
        Ok(json!({ "columns": columns, "rows": r.data }))
    }
}

fn one_line(sql: &str) -> String {
    sql.replace('\n', " ")
}

fn marts() -> HashMap<String, RelationColumns> {
    let table = |cols: &[(&str, ColumnKind)]| -> RelationColumns {
        cols.iter().map(|(n, k)| ((*n).to_owned(), *k)).collect()
    };
    HashMap::from([
        (
            POINTS.to_owned(),
            table(&[
                ("place", ColumnKind::Text),
                ("kab_kota", ColumnKind::Text),
                ("provinsi", ColumnKind::Text),
                ("lat", ColumnKind::Number),
                ("lon", ColumnKind::Number),
                ("visitors", ColumnKind::Number),
                ("category", ColumnKind::Text),
                ("visit_date", ColumnKind::Date),
            ]),
        ),
        (
            EVENTS.to_owned(),
            table(&[
                ("event_time", ColumnKind::DateTime),
                ("channel", ColumnKind::Text),
                ("orders", ColumnKind::Number),
            ]),
        ),
        (
            CAPACITY.to_owned(),
            table(&[
                ("provinsi", ColumnKind::Text),
                ("hotels", ColumnKind::Number),
                ("rooms", ColumnKind::Number),
            ]),
        ),
    ])
}

fn points_policy(mask: &'static [&'static str], row_filter: Option<&'static str>) -> Governed {
    Governed {
        table: POINTS,
        columns: &[
            "place",
            "kab_kota",
            "provinsi",
            "lat",
            "lon",
            "visitors",
            "category",
            "visit_date",
        ],
        mask,
        row_filter,
        unreadable: false,
    }
}

fn events_unreadable() -> Governed {
    Governed {
        table: EVENTS,
        columns: &["event_time", "channel", "orders"],
        mask: &[],
        row_filter: None,
        unreadable: true,
    }
}

/// A chart as the store holds it, from the JSON the builder sends.
fn chart(definition: &Value) -> StoredChartSpec {
    let mut def: ChartInput = serde_json::from_value(definition.clone()).unwrap();
    // The statement is always built from the definition, as for a chart that
    // has any shaping: the stored text below is a placeholder.
    def.shape.resolved_params = true;
    let spec: ChartSpec = serde_json::from_value(json!({
        "id": "c_1", "title": "t", "kind": def.kind, "mart": def.mart,
        "sqlSource": def.sql_source, "sql": "SELECT 1",
        "x": def.dimension, "y": def.measures.iter().collect::<Vec<_>>(),
    }))
    .unwrap();
    StoredChartSpec {
        spec,
        source: ChartSource::Ui,
        board: "default".to_owned(),
        def,
        has_year: false,
        created_by: None,
        created_at: None,
    }
}

fn source(id: &str, sql: &str, columns: &[(&str, &str)]) -> SqlSource {
    SqlSource {
        id: id.to_owned(),
        title: id.to_owned(),
        sql: sql.to_owned(),
        columns: columns
            .iter()
            .map(|(n, t)| SourceColumn {
                name: (*n).to_owned(),
                ty: (*t).to_owned(),
            })
            .collect(),
        folder_id: String::new(),
        created_by: String::new(),
        updated_at: None,
    }
}

/// What a board read does for one chart: the base tile as the dashboard builds
/// it, then its layers. Returns the tile.
async fn tile(
    engine: &Engine,
    base: &StoredChartSpec,
    sources: &HashMap<String, SqlSource>,
    layer_sources: Option<HashMap<String, SqlSource>>,
) -> Value {
    let clock = TimeContext::default();
    let catalog = FieldCatalog::default();
    let read = ReadContext {
        time: &clock,
        grain: None,
        fields: &catalog,
    };
    let cols = marts();
    let filtered = stored_chart_sql(base, &[], &[], &cols, sources, &read).unwrap();
    let Ok(mut tile) = engine.run(&filtered.sql).await else {
        panic!("the base statement failed");
    };
    annotate_grain(&mut tile, &filtered, base);
    let ctx = LayerContext {
        runner: engine,
        years: &[],
        filters: &[],
        cols: &cols,
        chart_sources: sources,
        layer_sources: &layer_sources,
        read: &read,
    };
    annotate_layers(&ctx, &mut tile, base, filtered.grain).await;
    eprintln!("TILE {tile}");
    tile
}

fn rows_of(v: &Value) -> Vec<Value> {
    v["rows"].as_array().cloned().unwrap_or_default()
}

fn as_f64(v: &Value) -> f64 {
    v.as_f64()
        .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
        .unwrap_or(f64::NAN)
}

/// `(bucket, value)` pairs of a result, by the named columns.
fn pairs(rows: &[Value], x: &str, y: &str) -> Vec<(String, f64)> {
    rows.iter()
        .map(|r| (r[x].as_str().unwrap_or_default().to_owned(), as_f64(&r[y])))
        .collect()
}

fn points_by_month(limit: u32) -> Value {
    json!({
        "title": "Visitors", "kind": "bar", "mart": POINTS,
        "dimension": "visit_date", "measures": ["visitors"], "aggregate": "sum",
        "grain": "month", "order": "none", "limit": limit,
        "layers": [{
            "mart": EVENTS, "dimension": "event_time", "measures": ["orders"],
            "aggregate": "sum", "draw": "line", "axis": "right", "label": "Orders"
        }]
    })
}

/// `BI-13-AC1`: visitors by month and orders by month, two tables, one chart.
#[tokio::test]
#[ignore = "needs the dev engine; see the module docs"]
async fn each_series_equals_a_direct_query_and_a_layer_lacking_a_month_is_a_gap() {
    let engine = Engine::from_env(Vec::new());
    let base = chart(&points_by_month(20));
    let tile = tile(&engine, &base, &HashMap::new(), Some(HashMap::new())).await;

    let direct_points = pairs(
        &engine
            .direct(
                "SELECT toString(date_trunc('month', visit_date)) AS m, round(sum(visitors)) AS v \
                 FROM serving.mart_demo_map_points GROUP BY m ORDER BY m",
            )
            .await,
        "m",
        "v",
    );
    let direct_events = pairs(
        &engine
            .direct(
                "SELECT toString(date_trunc('month', toTimeZone(event_time, 'Asia/Jakarta'))) AS m, \
                 round(sum(orders)) AS v FROM serving.mart_demo_events GROUP BY m ORDER BY m",
            )
            .await,
        "m",
        "v",
    );
    assert_eq!(
        pairs(&rows_of(&tile), "visit_date", "visitors"),
        direct_points
    );
    let layer = &tile["layers"][0];
    assert!(layer.get("error").is_none(), "{layer}");
    assert_eq!(
        pairs(&rows_of(layer), "event_time", "orders"),
        direct_events
    );
    // The base has 15 months and the layer 2: the other 13 are gaps (the layer
    // simply has no row), never a zero.
    assert_eq!(direct_points.len(), 15);
    assert_eq!(direct_events.len(), 2);
    assert_eq!(rows_of(layer).len(), 2);
    assert_eq!(layer["label"], "Orders");
    assert_eq!(layer["axis"], "right");
    assert_eq!(layer["source"], EVENTS);
}

/// The base cut to its latest three months: the layer is asked for every
/// month and trimmed to the base's range, so August (before the range) goes.
#[tokio::test]
#[ignore = "needs the dev engine; see the module docs"]
async fn a_grained_base_cut_to_its_latest_buckets_trims_the_layer_to_its_range() {
    let engine = Engine::from_env(Vec::new());
    let base = chart(&points_by_month(3));
    let tile = tile(&engine, &base, &HashMap::new(), Some(HashMap::new())).await;
    assert_eq!(tile["truncated"], json!(true));
    let months: Vec<String> = rows_of(&tile)
        .iter()
        .map(|r| r["visit_date"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(months, vec!["2026-09-01", "2026-10-01", "2026-11-01"]);
    let layer_months: Vec<String> = rows_of(&tile["layers"][0])
        .iter()
        .map(|r| r["event_time"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(layer_months, vec!["2026-09-01"]);
}

/// A layer the role cannot read is refused on that layer only; the base and
/// the other layer still answer.
#[tokio::test]
#[ignore = "needs the dev engine; see the module docs"]
async fn a_layer_the_role_cannot_read_is_refused_alone() {
    let engine = Engine::from_env(vec![events_unreadable()]);
    let mut def = points_by_month(20);
    def["layers"].as_array_mut().unwrap().push(json!({
        "mart": POINTS, "dimension": "visit_date", "measures": ["lat"],
        "aggregate": "count", "draw": "bar", "axis": "left", "label": "Points"
    }));
    let base = chart(&def);
    let tile = tile(&engine, &base, &HashMap::new(), Some(HashMap::new())).await;
    assert_eq!(rows_of(&tile).len(), 15, "the base still answers");
    let refused = &tile["layers"][0];
    assert!(
        refused["error"].as_str().is_some_and(|e| !e.is_empty()),
        "{refused}"
    );
    assert!(rows_of(refused).is_empty());
    let other = &tile["layers"][1];
    assert!(other.get("error").is_none(), "{other}");
    assert_eq!(rows_of(other).len(), 15);
}

/// The kept-values condition narrows the layer; it never shows what the
/// layer's own rules hide or mask. The base lists the ten provinces with the
/// most hotels; the layer's table is row-filtered to Bali (or its province column
/// masked): the layer shows Bali only (or nothing), not the other nine.
#[tokio::test]
#[ignore = "needs the dev engine; see the module docs"]
async fn the_kept_values_never_show_what_the_layers_own_rules_hide() {
    let def = json!({
        "title": "Hotels", "kind": "bar", "mart": CAPACITY,
        "dimension": "provinsi", "measures": ["hotels"], "aggregate": "sum",
        "order": "desc", "limit": 10,
        "layers": [{
            "mart": POINTS, "dimension": "provinsi", "measures": ["visitors"],
            "aggregate": "sum", "draw": "line", "axis": "right"
        }]
    });
    let base = chart(&def);

    // Unrestricted: the layer is exactly the base's provinces that have visits.
    let open = Engine::from_env(Vec::new());
    let tile_open = tile(&open, &base, &HashMap::new(), Some(HashMap::new())).await;
    let kept: Vec<String> = rows_of(&tile_open)
        .iter()
        .map(|r| r["provinsi"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(kept.len(), 10);
    let open_layer: Vec<String> = rows_of(&tile_open["layers"][0])
        .iter()
        .map(|r| r["provinsi"].as_str().unwrap().to_owned())
        .collect();
    assert!(
        open_layer.iter().all(|p| kept.contains(p)),
        "{open_layer:?}"
    );

    // A row filter on the layer's table that lets one of the base's provinces
    // through: only that one can come back, never the other nine.
    let filtered = Engine::from_env(vec![points_policy(&[], Some("provinsi = 'Bali'"))]);
    let tile_f = tile(&filtered, &base, &HashMap::new(), Some(HashMap::new())).await;
    let seen: Vec<String> = rows_of(&tile_f["layers"][0])
        .iter()
        .map(|r| r["provinsi"].as_str().unwrap().to_owned())
        .collect();
    assert!(kept.contains(&"Bali".to_owned()), "{kept:?}");
    assert_eq!(seen, vec!["Bali".to_owned()]);
    // And the reverse: a row filter that removes one of the base's provinces.
    let hiding = Engine::from_env(vec![points_policy(&[], Some("provinsi <> 'Bali'"))]);
    let tile_h = tile(&hiding, &base, &HashMap::new(), Some(HashMap::new())).await;
    let seen_h: Vec<String> = rows_of(&tile_h["layers"][0])
        .iter()
        .map(|r| r["provinsi"].as_str().unwrap().to_owned())
        .collect();
    assert!(!seen_h.contains(&"Bali".to_owned()), "{seen_h:?}");
    assert_eq!(seen_h.len(), open_layer.len() - 1);

    // A mask on the layer's province column: no real province name comes back,
    // so nothing is matched and nothing is revealed.
    let masked = Engine::from_env(vec![points_policy(&["provinsi"], None)]);
    let tile_m = tile(&masked, &base, &HashMap::new(), Some(HashMap::new())).await;
    let seen_m: Vec<String> = rows_of(&tile_m["layers"][0])
        .iter()
        .map(|r| r["provinsi"].as_str().unwrap().to_owned())
        .collect();
    assert!(
        seen_m.iter().all(|p| p == "***"),
        "a masked column shows only the mask: {seen_m:?}"
    );
    assert!(
        seen_m.is_empty(),
        "the base's real names must not match a masked column: {seen_m:?}"
    );
}

/// Base values with quotes, backslashes and SQL-looking text are matched as
/// text, not interpreted.
#[tokio::test]
#[ignore = "needs the dev engine; see the module docs"]
async fn base_values_with_quotes_and_backslashes_are_matched_not_interpreted() {
    let engine = Engine::from_env(Vec::new());
    // Five hostile values, written in the SQL sources as ClickHouse literals.
    let values = r"['O''Brien', 'back\\slash', 'x'') OR 1=1 --', '日本', 'plain']";
    let base_sql = format!("SELECT arrayJoin({values}) AS d, 1 AS v");
    let layer_sql = format!(
        "SELECT arrayJoin(arrayConcat({values}, ['extra', 'z'' OR ''1''=''1'])) AS d, 2 AS w"
    );
    let sources = HashMap::from([(
        "s_base".to_owned(),
        source("s_base", &base_sql, &[("d", "String"), ("v", "UInt8")]),
    )]);
    let layer_sources = HashMap::from([(
        "s_layer".to_owned(),
        source("s_layer", &layer_sql, &[("d", "String"), ("w", "UInt8")]),
    )]);
    let base = chart(&json!({
        "title": "t", "kind": "bar", "sqlSource": "s_base", "dimension": "d",
        "measures": ["v"], "aggregate": "sum", "order": "none", "limit": 5,
        "layers": [{ "sqlSource": "s_layer", "dimension": "d", "measures": ["w"],
            "aggregate": "sum", "draw": "line", "axis": "right" }]
    }));
    let tile = tile(&engine, &base, &sources, Some(layer_sources)).await;
    let base_values: Vec<String> = rows_of(&tile)
        .iter()
        .map(|r| r["d"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(base_values.len(), 5);
    let layer = &tile["layers"][0];
    assert!(layer.get("error").is_none(), "{layer}");
    let mut got: Vec<String> = rows_of(layer)
        .iter()
        .map(|r| r["d"].as_str().unwrap().to_owned())
        .collect();
    let mut expected = base_values;
    got.sort();
    expected.sort();
    assert_eq!(got, expected, "exactly the base's five values, intact");
    assert!(!got.iter().any(|v| v == "extra"));
}

/// A number dimension is matched through the same condition.
#[tokio::test]
#[ignore = "needs the dev engine; see the module docs"]
async fn a_number_dimension_is_matched_on_the_bases_values() {
    let engine = Engine::from_env(Vec::new());
    let sources = HashMap::from([(
        "s_base".to_owned(),
        source(
            "s_base",
            "SELECT arrayJoin(range(10)) AS n, 1 AS v",
            &[("n", "UInt64"), ("v", "UInt64")],
        ),
    )]);
    let layer_sources = HashMap::from([(
        "s_layer".to_owned(),
        source(
            "s_layer",
            "SELECT arrayJoin(range(10)) AS n, 10 AS w",
            &[("n", "UInt64"), ("w", "UInt64")],
        ),
    )]);
    let base = chart(&json!({
        "title": "t", "kind": "bar", "sqlSource": "s_base", "dimension": "n",
        "measures": ["v"], "aggregate": "sum", "order": "none", "limit": 4,
        "layers": [{ "sqlSource": "s_layer", "dimension": "n", "measures": ["w"],
            "aggregate": "sum", "draw": "bar", "axis": "left" }]
    }));
    let tile = tile(&engine, &base, &sources, Some(layer_sources)).await;
    let layer = &tile["layers"][0];
    assert!(layer.get("error").is_none(), "{layer}");
    let mut seen: Vec<f64> = rows_of(layer).iter().map(|r| as_f64(&r["n"])).collect();
    seen.sort_by(f64::total_cmp);
    assert_eq!(seen, vec![0.0, 1.0, 2.0, 3.0]);
}

/// The failures that are not the role's: a source that cannot be read, a
/// dimension of another kind, and an engine error. Each is its layer's alone,
/// with a fixed sentence.
#[tokio::test]
#[ignore = "needs the dev engine; see the module docs"]
async fn a_layer_that_cannot_be_asked_fails_alone_with_a_fixed_sentence() {
    let engine = Engine::from_env(Vec::new());
    let mut def = points_by_month(20);
    let layers = def["layers"].as_array_mut().unwrap();
    layers.push(
        json!({ "sqlSource": "s_gone", "dimension": "d", "measures": ["w"],
        "draw": "bar", "axis": "left" }),
    );
    // A text dimension against the chart's date.
    layers.push(
        json!({ "mart": POINTS, "dimension": "provinsi", "measures": ["visitors"],
        "draw": "bar", "axis": "left" }),
    );
    // A measure the table does not have: the engine fails.
    layers.push(
        json!({ "mart": EVENTS, "dimension": "event_time", "measures": ["nope"],
        "draw": "bar", "axis": "left" }),
    );
    let base = chart(&def);
    // The list of SQL sources could not be read.
    let tile = tile(&engine, &base, &HashMap::new(), None).await;
    assert_eq!(rows_of(&tile).len(), 15);
    assert!(tile["layers"][0].get("error").is_none());
    assert_eq!(
        tile["layers"][1]["error"],
        "this question's SQL source could not be read."
    );
    assert!(
        tile["layers"][2]["error"]
            .as_str()
            .unwrap()
            .contains("no longer lines up"),
        "{}",
        tile["layers"][2]
    );
    let failed = &tile["layers"][3];
    assert_eq!(
        failed["error"], "This chart could not be loaded.",
        "{failed}"
    );
    assert!(failed["errorId"].is_string());
    assert!(!failed.to_string().contains("nope"), "{failed}");
}

// ── no engine needed: how a layer fails, and what it leaves alone ──────────

/// Answers every statement from a table of canned results and records what it
/// was asked.
struct Stub {
    asked: std::sync::Mutex<Vec<String>>,
}

impl Stub {
    fn new() -> Self {
        Self {
            asked: std::sync::Mutex::new(Vec::new()),
        }
    }
    fn asked(&self) -> Vec<String> {
        self.asked.lock().unwrap().clone()
    }
}

impl LayerRunner for Stub {
    async fn run(&self, sql: &str) -> Result<Value, SpecRunFailure> {
        self.asked.lock().unwrap().push(sql.to_owned());
        if sql.contains("SECRETCOLUMN") {
            return Err(SpecRunFailure::Clickhouse(
                lakehouse_clickhouse::ChError::Server(
                    "Code: 47. Unknown identifier SECRETCOLUMN on db-internal.example.net"
                        .to_owned(),
                ),
            ));
        }
        if sql.contains("REFUSED") {
            return Err(SpecRunFailure::Refused("refused by the role rewrite"));
        }
        Ok(json!({
            "columns": ["provinsi", "v"],
            "rows": [{ "provinsi": "A", "v": 1 }, { "provinsi": "B", "v": 2 }],
        }))
    }
}

async fn stub_tile(stub: &Stub, base: &StoredChartSpec, mut tile: Value) -> Value {
    let clock = TimeContext::default();
    let catalog = FieldCatalog::default();
    let read = ReadContext {
        time: &clock,
        grain: None,
        fields: &catalog,
    };
    let cols = marts();
    let sources = HashMap::new();
    let layer_sources = Some(HashMap::new());
    let ctx = LayerContext {
        runner: stub,
        years: &[],
        filters: &[],
        cols: &cols,
        chart_sources: &sources,
        layer_sources: &layer_sources,
        read: &read,
    };
    annotate_layers(&ctx, &mut tile, base, None).await;
    tile
}

fn capacity_layer(measure: &str) -> Value {
    json!({ "mart": CAPACITY, "dimension": "provinsi", "measures": [measure],
        "aggregate": "sum", "draw": "line", "axis": "right" })
}

fn by_province(limit: u32, layers: &[Value]) -> StoredChartSpec {
    chart(&json!({
        "title": "t", "kind": "bar", "mart": POINTS, "dimension": "provinsi",
        "measures": ["visitors"], "aggregate": "sum", "order": "desc", "limit": limit,
        "layers": layers,
    }))
}

fn tile_of(rows: &[&str]) -> Value {
    json!({
        "columns": ["provinsi", "visitors"],
        "rows": rows.iter().map(|p| json!({ "provinsi": p, "visitors": 1 })).collect::<Vec<_>>(),
    })
}

/// Charts saved before this change: nothing is asked and the tile keeps its
/// exact shape, with no `layers` key.
#[tokio::test]
async fn a_chart_without_layers_asks_nothing_and_gets_no_layers_key() {
    let stub = Stub::new();
    let plain = by_province(5, &[]);
    let before = tile_of(&["A", "B"]);
    let after = stub_tile(&stub, &plain, before.clone()).await;
    assert_eq!(after, before);
    assert!(after.get("layers").is_none());
    assert!(stub.asked().is_empty());
}

/// A tile that carries an error is left alone: the layers are not asked.
#[tokio::test]
async fn a_tile_that_failed_is_left_alone_and_its_layers_are_not_asked() {
    let stub = Stub::new();
    let base = by_province(5, &[capacity_layer("hotels")]);
    let failed = json!({ "error": "This chart could not be loaded.", "errorId": "x1" });
    assert_eq!(stub_tile(&stub, &base, failed.clone()).await, failed);
    assert!(stub.asked().is_empty());
}

/// Each way a statement can fail is that layer's own sentence; the database's
/// text never reaches the entry, and its neighbours answer.
#[tokio::test]
async fn a_failing_or_refused_statement_is_that_layers_sentence_and_its_neighbours_answer() {
    let stub = Stub::new();
    let base = by_province(
        5,
        &[
            capacity_layer("hotels"),
            capacity_layer("SECRETCOLUMN"),
            capacity_layer("REFUSED"),
        ],
    );
    let tile = stub_tile(&stub, &base, tile_of(&["A", "B"])).await;
    let layers = tile["layers"].as_array().unwrap();
    assert_eq!(layers.len(), 3);
    assert!(layers[0].get("error").is_none(), "{}", layers[0]);
    assert_eq!(layers[0]["rows"].as_array().unwrap().len(), 2);
    assert_eq!(layers[1]["error"], "This chart could not be loaded.");
    assert!(layers[1]["errorId"].is_string());
    assert_eq!(layers[1]["rows"], json!([]));
    assert_eq!(layers[2]["error"], "refused by the role rewrite");
    assert!(layers[2].get("errorId").is_none());
    let all = tile.to_string();
    assert!(
        !all.contains("SECRETCOLUMN"),
        "the database's text leaked: {all}"
    );
    assert!(
        !all.contains("db-internal"),
        "the database's text leaked: {all}"
    );
    // The base rows are untouched.
    assert_eq!(tile["rows"].as_array().unwrap().len(), 2);
}

/// The layer is asked for the base's values only when the base filled its
/// limit; otherwise it is asked for its own.
#[tokio::test]
async fn a_layer_is_asked_for_the_bases_values_only_when_the_base_filled_its_limit() {
    let layers = [capacity_layer("hotels")];
    let full = Stub::new();
    stub_tile(&full, &by_province(2, &layers), tile_of(&["A", "B"])).await;
    assert!(
        full.asked()[0].contains("provinsi IN ('A','B')"),
        "{:?}",
        full.asked()
    );
    let short = Stub::new();
    stub_tile(&short, &by_province(5, &layers), tile_of(&["A", "B"])).await;
    assert!(!short.asked()[0].contains(" IN ("), "{:?}", short.asked());
}

/// A relation that is not there, and a source list that could not be read, are
/// the layer's sentence, not a run of the statement over nothing.
#[tokio::test]
async fn a_layer_over_a_table_that_is_gone_is_not_asked() {
    let stub = Stub::new();
    let mut gone = capacity_layer("hotels");
    gone["mart"] = json!("mart_deleted");
    let base = by_province(5, &[gone]);
    let tile = stub_tile(&stub, &base, tile_of(&["A"])).await;
    assert_eq!(
        tile["layers"][0]["error"],
        "this question's source is not available."
    );
    assert!(stub.asked().is_empty());
}
