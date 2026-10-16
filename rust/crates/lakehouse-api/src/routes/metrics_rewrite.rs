//! `BI-2` (with `AI-7`): metrics and segments under the real role rewrite and
//! the real engine.
//!
//! Every statement here is built by the same functions a dashboard tile and
//! `query_metric` use (`stored_chart_sql` over a chart that names a metric, or
//! `lakehouse_bi::metrics::value_chart` for a metric read on its own), passed
//! through the real `enforce` with real obligations (a mask, a row filter, a
//! table the role cannot read) and run on a real `ClickHouse`. Those tests are
//! `#[ignore]`d because they need that engine; run them by hand with
//!
//! ```text
//! LAYER_ENGINE_URL=http://localhost:8123 LAYER_ENGINE_USER=... \
//! LAYER_ENGINE_PASSWORD=... cargo test -p lakehouse-api --lib \
//!   metrics_rewrite -- --ignored --test-threads=1 --nocapture
//! ```
//!
//! They only read: the demo tables of `serving`, and `SELECT`s over literal
//! rows for the escaping check. The raw and rewritten statements and the
//! results are printed so they can be quoted (plan section 4). The tests that
//! need no engine pin how a missing metric fails its own tile.

#![cfg(test)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashMap;

use lakehouse_bi::builder::ReadContext;
use lakehouse_bi::fields::{FieldCatalog, SourceKind};
use lakehouse_bi::filters::FilterDef;
use lakehouse_bi::grain::TimeContext;
use lakehouse_bi::metrics::{self, MetricDef, SegmentDef, ValueQuery};
use lakehouse_bi::sources::{SourceColumn, SqlSource};
use serde_json::{Value, json};

use crate::routes::layers::LayerRunner;
use crate::routes::layers::layers_rewrite::{
    Engine, POINTS, chart, events_unreadable, marts, points_policy,
};
use crate::routes::support::stored_chart_sql;

fn metric(name: &str, formula: &str, filters: Vec<FilterDef>) -> MetricDef {
    MetricDef {
        id: format!("m_{name}"),
        name: name.to_owned(),
        label: name.to_owned(),
        description: String::new(),
        source_kind: SourceKind::Mart,
        source_id: POINTS.to_owned(),
        formula: formula.to_owned(),
        filters,
        time_column: Some("visit_date".to_owned()),
        grain: Some("month".to_owned()),
        format: None,
        created_by: "an administrator".to_owned(),
        updated_at: None,
    }
}

fn segment(name: &str, filters: Vec<FilterDef>) -> SegmentDef {
    SegmentDef {
        id: format!("g_{name}"),
        name: name.to_owned(),
        label: name.to_owned(),
        source_kind: SourceKind::Mart,
        source_id: POINTS.to_owned(),
        filters,
        created_by: String::new(),
        updated_at: None,
    }
}

fn filter(value: &Value) -> FilterDef {
    serde_json::from_value(value.clone()).unwrap()
}

fn catalog(metrics: &[MetricDef], segments: &[SegmentDef]) -> FieldCatalog {
    FieldCatalog::from_fields(metrics::catalog_entries(metrics, segments))
}

fn num(v: &Value) -> f64 {
    v.as_f64()
        .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
        .unwrap_or(f64::NAN)
}

/// A tile as the dashboard builds it: the chart's statement through the real
/// rewrite and the engine.
async fn tile(
    engine: &Engine,
    c: &lakehouse_bi::store::StoredChartSpec,
    cat: &FieldCatalog,
) -> Value {
    let clock = TimeContext::default();
    let read = ReadContext {
        time: &clock,
        grain: None,
        fields: cat,
    };
    let filtered = stored_chart_sql(c, &[], &[], &marts(), &HashMap::new(), &read)
        .unwrap_or_else(|_missing| panic!("the tile has no statement"));
    eprintln!("BUILT {}", filtered.sql.replace('\n', " "));
    match engine.run(&filtered.sql).await {
        Ok(v) => v,
        Err(_failure) => json!({ "error": "the statement was refused or failed" }),
    }
}

/// `query_metric` / the editor's live value: the metric on its own.
async fn value(engine: &Engine, cat: &FieldCatalog, q: &ValueQuery<'_>) -> Value {
    let c = metrics::value_chart(q);
    tile(engine, &c, cat).await
}

fn q(name: &str) -> ValueQuery<'_> {
    ValueQuery {
        metric: name,
        source_kind: SourceKind::Mart,
        source_id: POINTS,
        group_by: None,
        grain: None,
        filters: Vec::new(),
    }
}

fn first_number(t: &Value) -> f64 {
    let row = t["rows"][0]
        .as_object()
        .unwrap_or_else(|| panic!("no row: {t}"));
    num(row.values().next().unwrap())
}

async fn direct_number(engine: &Engine, sql: &str) -> f64 {
    let rows = engine.direct(sql).await;
    num(rows[0].as_object().unwrap().values().next().unwrap())
}

/// `BI-2-AC1`: one metric, the same number in a chart, a KPI and `query_metric`.
#[tokio::test]
#[ignore = "needs the dev engine; see the module docs"]
async fn ac1_one_metric_gives_the_same_number_in_a_chart_a_kpi_and_a_query() {
    let engine = Engine::from_env(Vec::new());
    let cat = catalog(&[metric("total_visitors", "Sum([visitors])", vec![])], &[]);
    let direct = direct_number(
        &engine,
        "SELECT sum(visitors) FROM serving.mart_demo_map_points",
    )
    .await;
    assert!((direct - 278_955.0).abs() < 0.5, "{direct}");

    let kpi = chart(
        &json!({ "title": "k", "kind": "kpi", "mart": POINTS, "measures": [{ "metric": "total_visitors" }] }),
    );
    let kpi_tile = tile(&engine, &kpi, &cat).await;
    let bar = chart(&json!({
        "title": "b", "kind": "bar", "mart": POINTS, "dimension": "category",
        "measures": [{ "metric": "total_visitors" }], "order": "desc", "limit": 20
    }));
    let bar_tile = tile(&engine, &bar, &cat).await;
    let chart_sum: f64 = bar_tile["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| num(&r["total_visitors"]))
        .sum();
    let query_tile = value(&engine, &cat, &q("total_visitors")).await;
    eprintln!("KPI {kpi_tile}\nBAR {bar_tile}\nQUERY {query_tile}");
    assert!((first_number(&kpi_tile) - direct).abs() < 0.5);
    assert!(
        (chart_sum - direct).abs() < 0.5,
        "the chart's groups add up to the metric: {chart_sum}"
    );
    assert!((first_number(&query_tile) - direct).abs() < 0.5);
    // The same metric by month, through query_metric's grouped form: 15 months adding to the total.
    let by_month = value(
        &engine,
        &cat,
        &ValueQuery {
            group_by: Some("visit_date"),
            grain: Some("month"),
            ..q("total_visitors")
        },
    )
    .await;
    let months = by_month["rows"].as_array().unwrap();
    eprintln!("BY MONTH {by_month}");
    assert_eq!(months.len(), 15);
    assert!(
        (months
            .iter()
            .map(|r| num(&r["total_visitors"]))
            .sum::<f64>()
            - direct)
            .abs()
            < 0.5
    );
}

/// `BI-2-AC2`: a filtered metric beside an unfiltered measure, two numbers that each match a direct query.
#[tokio::test]
#[ignore = "needs the dev engine; see the module docs"]
async fn ac2_a_filtered_metric_beside_an_unfiltered_measure_match_direct_queries() {
    let engine = Engine::from_env(Vec::new());
    let office = filter(&json!({ "column": "category", "values": ["Office"] }));
    let cat = catalog(
        &[metric("office_visitors", "Sum([visitors])", vec![office])],
        &[],
    );
    let c = chart(&json!({
        "title": "b", "kind": "bar", "mart": POINTS, "dimension": "category",
        "measures": [{ "metric": "office_visitors" }, "visitors"], "aggregate": "sum", "order": "desc", "limit": 20
    }));
    let t = tile(&engine, &c, &cat).await;
    eprintln!("TILE {t}");
    let direct = engine
        .direct("SELECT category, sum(visitors) AS v FROM serving.mart_demo_map_points GROUP BY category ORDER BY category")
        .await;
    let rows = t["rows"].as_array().unwrap();
    for d in &direct {
        let cat_name = d["category"].as_str().unwrap();
        let row = rows.iter().find(|r| r["category"] == cat_name).unwrap();
        assert!(
            (num(&row["visitors"]) - num(&d["v"])).abs() < 0.5,
            "{cat_name} unfiltered"
        );
        let want = if cat_name == "Office" {
            num(&d["v"])
        } else {
            0.0
        };
        assert!(
            (num(&row["office_visitors"]) - want).abs() < 0.5,
            "{cat_name} filtered"
        );
    }
    // And as a number on its own: the filtered total equals a direct query with a WHERE.
    let own = value(&engine, &cat, &q("office_visitors")).await;
    let want = direct_number(
        &engine,
        "SELECT sum(visitors) FROM serving.mart_demo_map_points WHERE category = 'Office'",
    )
    .await;
    assert!((first_number(&own) - want).abs() < 0.5, "{own} vs {want}");
    eprintln!("OFFICE {want}");
}

/// Every aggregate of the catalog under a condition: one statement each, run through the real
/// pipeline, equal to the plain function over a `WHERE`.
#[tokio::test]
#[ignore = "needs the dev engine; see the module docs"]
async fn every_aggregate_runs_under_a_condition_and_equals_the_plain_function_over_a_where() {
    let engine = Engine::from_env(Vec::new());
    let cond = filter(&json!({ "column": "category", "values": ["Office", "Retail"] }));
    let where_ = "WHERE category IN ('Office','Retail')";
    let cases: [(&str, &str); 14] = [
        ("Count()", "count()"),
        ("Count([place])", "count(place)"),
        ("CountDistinct([place])", "uniqExact(place)"),
        ("CountIf([visitors] > 50)", "countIf(visitors > 50)"),
        ("Sum([visitors])", "sum(visitors)"),
        ("Avg([visitors])", "avg(visitors)"),
        ("Median([visitors])", "medianExact(visitors)"),
        ("Min([visitors])", "min(visitors)"),
        ("Max([visitors])", "max(visitors)"),
        ("StdDev([visitors])", "stddevSamp(visitors)"),
        (
            "Percentile([visitors], 0.9)",
            "quantileExact(0.9)(visitors)",
        ),
        (
            "SumIf([visitors], [provinsi] = 'Bali')",
            "sumIf(visitors, provinsi = 'Bali')",
        ),
        (
            "AvgIf([visitors], [provinsi] = 'Bali')",
            "avgIf(visitors, provinsi = 'Bali')",
        ),
        ("Max([visit_date])", "max(visit_date)"),
    ];
    for (formula, plain) in cases {
        let cat = catalog(&[metric("m", formula, vec![cond.clone()])], &[]);
        let got = value(&engine, &cat, &q("m")).await;
        assert!(got.get("error").is_none(), "{formula}: {got}");
        // A date prints as text; compare it as the engine prints it.
        let sql = format!("SELECT {plain} AS v FROM serving.mart_demo_map_points {where_}");
        let want = engine.direct(&sql).await;
        let (a, b) = (
            got["rows"][0]
                .as_object()
                .unwrap()
                .values()
                .next()
                .unwrap()
                .clone(),
            want[0]["v"].clone(),
        );
        eprintln!("AGG {formula} -> {a} (direct {b})");
        if formula.starts_with("Max([visit_date") {
            assert_eq!(a, b, "{formula}");
        } else if formula.starts_with("Count")
            || formula.starts_with("Min")
            || formula.starts_with("Max")
        {
            assert!((num(&a) - num(&b)).abs() < 0.5, "{formula}: {a} vs {b}");
        } else {
            // The metric is not rounded; a plain avg is not either, so they agree to rounding of the display.
            assert!(
                (num(&a) - num(&b)).abs() < 1e-6 * num(&b).abs().max(1.0) + 0.5,
                "{formula}: {a} vs {b}"
            );
        }
    }
}

/// `last_year` against a direct query on `visit_date` (data runs Sep 2025 to Nov 2026).
#[tokio::test]
#[ignore = "needs the dev engine; see the module docs"]
async fn last_year_matches_a_direct_query_by_month() {
    let engine = Engine::from_env(Vec::new());
    let cat = catalog(&[metric("total_visitors", "Sum([visitors])", vec![])], &[]);
    // The comparison leaves its date column and period to the metric (visit_date, month).
    let kpi = chart(&json!({
        "title": "k", "kind": "kpi", "mart": POINTS, "measures": [{ "metric": "total_visitors" }],
        "compare": { "kind": "last_year" }
    }));
    let t = tile(&engine, &kpi, &cat).await;
    eprintln!("TREND {t}");
    let rows = t["rows"].as_array().unwrap();
    let last = rows.last().unwrap();
    let ago = rows
        .iter()
        .find(|r| r["visit_date"].as_str().unwrap().starts_with("2025-11"))
        .unwrap();
    let direct = engine
        .direct("SELECT toString(toStartOfMonth(visit_date)) AS m, sum(visitors) AS v FROM serving.mart_demo_map_points WHERE toStartOfMonth(visit_date) IN ('2026-11-01','2025-11-01') GROUP BY m ORDER BY m")
        .await;
    assert!(last["visit_date"].as_str().unwrap().starts_with("2026-11"));
    assert!(
        (num(&ago["v"]) - num(&direct[0]["v"])).abs() < 0.5,
        "2025-11"
    );
    assert!(
        (num(&last["v"]) - num(&direct[1]["v"])).abs() < 0.5,
        "2026-11"
    );
    eprintln!(
        "LAST YEAR {} -> {}",
        num(&direct[0]["v"]),
        num(&direct[1]["v"])
    );
}

/// The role rewrite: a mask and a row filter apply under a metric as under a plain chart; a table
/// the role cannot read is refused; and a metric defined by an administrator lends the viewer nothing.
#[tokio::test]
#[ignore = "needs the dev engine; see the module docs"]
async fn the_role_rewrite_governs_a_metric_as_it_governs_a_plain_chart() {
    let office = filter(&json!({ "column": "category", "values": ["Office"] }));
    let cat = catalog(
        &[
            metric("total_visitors", "Sum([visitors])", vec![]),
            metric("office_visitors", "Sum([visitors])", vec![office]),
        ],
        &[],
    );
    let admin = Engine::from_env(Vec::new());
    let everything = first_number(&value(&admin, &cat, &q("total_visitors")).await);

    // A row filter on the table: the metric sees only Bali, exactly as a plain sum does.
    let viewer = Engine::from_env(vec![points_policy(&[], Some("provinsi = 'Bali'"))]);
    let as_viewer = first_number(&value(&viewer, &cat, &q("total_visitors")).await);
    let plain = chart(
        &json!({ "title": "p", "kind": "kpi", "mart": POINTS, "measures": ["visitors"], "aggregate": "sum" }),
    );
    let plain_viewer = first_number(&tile(&viewer, &plain, &cat).await);
    let bali = direct_number(
        &admin,
        "SELECT sum(visitors) FROM serving.mart_demo_map_points WHERE provinsi = 'Bali'",
    )
    .await;
    eprintln!(
        "ADMIN {everything}  VIEWER(metric) {as_viewer}  VIEWER(plain chart) {plain_viewer}  BALI(direct) {bali}"
    );
    assert!((as_viewer - bali).abs() < 0.5 && (plain_viewer - bali).abs() < 0.5);
    assert!(
        everything > as_viewer,
        "the administrator's reading is not the viewer's"
    );

    // A mask on the filter column: the metric's filter is evaluated on the masked value, so it matches
    // nothing; the real values are never compared.
    let masked = Engine::from_env(vec![points_policy(&["category"], None)]);
    let t = value(&masked, &cat, &q("office_visitors")).await;
    eprintln!("MASKED FILTER COLUMN {t}");
    assert!(
        t.get("error").is_some() || first_number(&t).abs() < 0.5,
        "{t}"
    );
    // A mask on the measure: the engine refuses to sum masked text, as for a plain chart.
    let masked_measure = Engine::from_env(vec![points_policy(&["visitors"], None)]);
    let m = value(&masked_measure, &cat, &q("total_visitors")).await;
    let p = tile(&masked_measure, &plain, &cat).await;
    eprintln!("MASKED MEASURE metric {m}\nMASKED MEASURE plain {p}");
    assert!(m.get("error").is_some() && p.get("error").is_some());

    // A table the role cannot read: refused by the rewrite, for the metric as for a plain chart.
    let mut policy = points_policy(&[], None);
    policy.unreadable = true;
    let blocked = Engine::from_env(vec![policy]);
    let refused = value(&blocked, &cat, &q("total_visitors")).await;
    eprintln!("UNREADABLE {refused}");
    assert!(refused.get("error").is_some());
    let _unused = events_unreadable();
}

/// Values with quotes, backslashes, comment markers and non-ASCII in a metric's filter and in a
/// segment reach the engine only inside escaped literals, and select exactly their rows.
#[tokio::test]
#[ignore = "needs the dev engine; see the module docs"]
async fn hostile_filter_and_segment_values_select_exactly_their_rows() {
    let engine = Engine::from_env(Vec::new());
    let sql = "SELECT 'O''Brien' AS d, 1 AS n UNION ALL SELECT 'back\\\\slash', 10 UNION ALL \
               SELECT 'x'') OR 1=1 --', 100 UNION ALL SELECT '日本', 1000 UNION ALL SELECT 'plain', 10000";
    let source = SqlSource {
        id: "s_lit".to_owned(),
        title: "lit".to_owned(),
        sql: sql.to_owned(),
        columns: vec![
            SourceColumn {
                name: "d".to_owned(),
                ty: "String".to_owned(),
            },
            SourceColumn {
                name: "n".to_owned(),
                ty: "UInt8".to_owned(),
            },
        ],
        folder_id: String::new(),
        created_by: String::new(),
        updated_at: None,
    };
    let sources = HashMap::from([(source.id.clone(), source.clone())]);
    let hostile = ["O'Brien", "back\\slash", "x') OR 1=1 --", "日本"];
    let on = |kind: SourceKind| (kind, "s_lit".to_owned());
    let (k, id) = on(SourceKind::SqlSource);
    let mut m = metric("m", "Sum([n])", vec![FilterDef::segment_ref("odd")]);
    (m.source_kind, m.source_id, m.time_column, m.grain) = (k, id.clone(), None, None);
    let mut seg = segment(
        "odd",
        vec![filter(&json!({ "column": "d", "values": hostile }))],
    );
    (seg.source_kind, seg.source_id) = (k, id);
    let cat = catalog(&[m], &[seg]);
    let clock = TimeContext::default();
    let read = ReadContext {
        time: &clock,
        grain: None,
        fields: &cat,
    };
    let c = metrics::value_chart(&ValueQuery {
        metric: "m",
        source_kind: SourceKind::SqlSource,
        source_id: "s_lit",
        group_by: None,
        grain: None,
        filters: Vec::new(),
    });
    let filtered = stored_chart_sql(&c, &[], &[], &HashMap::new(), &sources, &read).unwrap();
    eprintln!("BUILT {}", filtered.sql.replace('\n', " "));
    let t = engine
        .run(&filtered.sql)
        .await
        .unwrap_or_else(|_| panic!("failed"));
    eprintln!("RESULT {t}");
    // 1 + 10 + 100 + 1000: the four hostile values; `plain` (10000) is outside the segment.
    assert!((first_number(&t) - 1111.0).abs() < 0.5, "{t}");
}

/// A metric or segment that is missing fails its own tile with a plain sentence; the stored
/// statement is never run, and the neighbours build as before. No engine needed.
#[test]
fn a_missing_metric_or_segment_is_that_tiles_error_and_its_neighbours_build() {
    let clock = TimeContext::default();
    let cat = catalog(&[metric("total_visitors", "Sum([visitors])", vec![])], &[]);
    let read = ReadContext {
        time: &clock,
        grain: None,
        fields: &cat,
    };
    let ok = chart(
        &json!({ "title": "ok", "kind": "kpi", "mart": POINTS, "measures": [{ "metric": "total_visitors" }] }),
    );
    let ghost = chart(
        &json!({ "title": "g", "kind": "kpi", "mart": POINTS, "measures": [{ "metric": "gone" }] }),
    );
    let seg = chart(&json!({
        "title": "s", "kind": "bar", "mart": POINTS, "dimension": "category", "measures": ["visitors"],
        "filters": [{ "segment": "gone" }]
    }));
    let plain = chart(
        &json!({ "title": "p", "kind": "bar", "mart": POINTS, "dimension": "category", "measures": ["visitors"] }),
    );
    let build = |c| stored_chart_sql(c, &[], &[], &marts(), &HashMap::new(), &read);
    assert!(build(&ok).is_ok());
    assert!(build(&plain).is_ok());
    for broken in [&ghost, &seg] {
        let err = build(broken).unwrap_err();
        assert!(err.contains("metric or segment"));
    }
    // With no catalog at all (the metric store could not be read), a chart that names a metric fails its
    // tile and one that does not is untouched.
    let none = FieldCatalog::default();
    let read_none = ReadContext {
        time: &clock,
        grain: None,
        fields: &none,
    };
    assert!(stored_chart_sql(&ok, &[], &[], &marts(), &HashMap::new(), &read_none).is_err());
    assert!(stored_chart_sql(&plain, &[], &[], &marts(), &HashMap::new(), &read_none).is_ok());
}

/// Charts saved before this change produce the same statement whatever the catalog holds.
#[test]
fn a_chart_that_names_no_metric_builds_the_same_statement_with_or_without_metrics() {
    let clock = TimeContext::default();
    let with = catalog(
        &[metric("total_visitors", "Sum([visitors])", vec![])],
        &[segment(
            "bali",
            vec![filter(&json!({ "column": "provinsi", "values": ["Bali"] }))],
        )],
    );
    let without = FieldCatalog::default();
    let c = chart(&json!({
        "title": "p", "kind": "bar", "mart": POINTS, "dimension": "category", "measures": ["visitors"],
        "aggregate": "sum", "order": "desc", "limit": 20
    }));
    let sql = |cat: &FieldCatalog| {
        let read = ReadContext {
            time: &clock,
            grain: None,
            fields: cat,
        };
        stored_chart_sql(&c, &[], &[], &marts(), &HashMap::new(), &read)
            .unwrap()
            .sql
    };
    assert_eq!(sql(&with), sql(&without));
    eprintln!("SAME {}", sql(&with));
}
