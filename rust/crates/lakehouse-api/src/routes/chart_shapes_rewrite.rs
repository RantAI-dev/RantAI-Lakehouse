//! `BI-6`: the role rewrite on the statement shapes the chart builder adds
//! (the "Other" group, a sort beside a breakdown, tables grouped by several
//! columns, a chart's own filters, measures with their own aggregate).
//!
//! The rewrite replaces every read of a governed table with its masked,
//! row-filtered projection. A shape it could not see into would read the table
//! raw, which is a data leak, so each case asserts that *every* read of the
//! table in the statement became the projection, and that `enforce` refuses
//! (rather than half-protects) a shape it cannot handle. The tests use the
//! real [`enforce`] with a fixed obligation set, as
//! `routes::calc_fields` does, and write each rewritten statement to the file
//! named by `CHART_SHAPES_REWRITE_OUT` so the developer can run them by hand
//! against the real engine.

#![cfg(test)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashMap;
use std::io::Write as _;

use lakehouse_bi::builder::{ReadContext, RelationColumns, sql_with_filters_report};
use lakehouse_bi::filters::{ColumnKind, FilterDef};
use lakehouse_bi::grain::TimeContext;
use lakehouse_bi::specs::ChartSource;
use lakehouse_bi::store::{ChartInput, ChartSpec, StoredChartSpec};
use sqlparser::dialect::ClickHouseDialect;

use crate::sql_rewrite::{
    Engine, NoViews, ObligationsSource, PlaceholderValues, TableObligations, enforce,
};

const TABLE: &str = "serving.mart_demo_map_points";
const MART: &str = "mart_demo_map_points";

/// One governed table with a mask and a row filter for every role.
struct Governed {
    mask: &'static [&'static str],
}

/// Masks the measure too: every sum is over text and the engine refuses it,
/// which is what a plain chart over a masked number does today.
const MASK_MEASURE: Governed = Governed {
    mask: &["place", "visitors"],
};
/// Masks only text columns, so the statements run and return rows.
const MASK_TEXT: Governed = Governed {
    mask: &["place", "category"],
};

impl ObligationsSource for Governed {
    fn obligations_for(&self, table: &str, _roles: &[String]) -> Option<TableObligations> {
        (table == TABLE).then(|| TableObligations {
            mask: self.mask.iter().map(|m| (*m).to_owned()).collect(),
            row_filter: Some("provinsi = 'Dki Jakarta'".to_owned()),
            real_columns: Some(
                [
                    "place",
                    "kab_kota",
                    "provinsi",
                    "lat",
                    "lon",
                    "visitors",
                    "category",
                    "visit_date",
                ]
                .map(str::to_owned)
                .to_vec(),
            ),
        })
    }
    fn has_any_obligation(&self, _roles: &[String]) -> bool {
        true
    }
}

fn columns() -> HashMap<String, RelationColumns> {
    let cols: RelationColumns = [
        ("place", ColumnKind::Text),
        ("kab_kota", ColumnKind::Text),
        ("provinsi", ColumnKind::Text),
        ("lat", ColumnKind::Number),
        ("lon", ColumnKind::Number),
        ("visitors", ColumnKind::Number),
        ("category", ColumnKind::Text),
        ("visit_date", ColumnKind::Date),
    ]
    .into_iter()
    .map(|(n, k)| (n.to_owned(), k))
    .collect();
    HashMap::from([(MART.to_owned(), cols)])
}

/// A stored chart over the demo mart with `extra` laid over a bar chart.
fn chart(extra: &serde_json::Value) -> StoredChartSpec {
    let mut def = serde_json::json!({
        "title": "t", "kind": "bar", "mart": MART, "dimension": "provinsi",
        "measures": ["visitors"], "aggregate": "sum", "limit": 5, "order": "desc",
    });
    def.as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    let def: ChartInput = serde_json::from_value(def).unwrap();
    let spec: ChartSpec = serde_json::from_value(serde_json::json!({
        "id": "u_1", "title": "t", "kind": def.kind, "mart": MART, "sql": "SELECT 1",
        "x": def.dimension, "y": "visitors",
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

fn record(label: &str, rewritten: &str) {
    let Ok(path) = std::env::var("CHART_SHAPES_REWRITE_OUT") else {
        return;
    };
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .unwrap();
    writeln!(file, "-- {label}\n{}", rewritten.replace('\n', " ")).unwrap();
}

/// The statement the dashboard builds for `chart`, rewritten for a role with
/// the mask and the row filter above, checked: every read of the table is the
/// masked, row-filtered projection.
fn rewritten(
    label: &str,
    chart: &StoredChartSpec,
    dashboard: &[FilterDef],
    policy: &Governed,
) -> String {
    let time = TimeContext::default();
    let sql =
        sql_with_filters_report(chart, &[], dashboard, &columns(), &ReadContext::new(&time)).sql;
    let reads = sql.matches(TABLE).count();
    assert!(reads >= 1, "{label}: the statement reads nothing: {sql}");
    let out = enforce(
        &sql,
        &ClickHouseDialect {},
        Engine::ClickHouse,
        &["analyst".to_owned()],
        &PlaceholderValues::none(),
        policy,
        &NoViews,
    )
    .unwrap_or_else(|failure| panic!("{label}: refused: {failure}\n{sql}"));
    record(&format!("{label} [mask {}]", policy.mask.join("+")), &out);
    // Each read of the governed table is replaced by its projection: the
    // masked columns and the row filter appear once per read, and the table
    // name appears once per read (inside that projection), never bare.
    assert_eq!(out.matches(TABLE).count(), reads, "{label}: {out}");
    for column in policy.mask {
        assert_eq!(
            out.matches(&format!(
                "replaceRegexpOne(toString(`{column}`), '(?s)^.*$', '***') AS `{column}`"
            ))
            .count(),
            reads,
            "{label}: a read of the table does not mask {column}: {out}"
        );
    }
    assert_eq!(
        out.matches("WHERE provinsi = 'Dki Jakarta'").count(),
        reads,
        "{label}: a read of the table is not row-filtered: {out}"
    );
    out
}

#[test]
fn the_other_group_reads_the_governed_table_only_through_its_projection() {
    for policy in [&MASK_MEASURE, &MASK_TEXT] {
        other_group_cases(policy);
    }
}

fn other_group_cases(policy: &Governed) {
    // The "Other" statement reads the relation three times (kept groups, the
    // flag, the rows), and each is the masked, row-filtered projection.
    let out = rewritten(
        "other",
        &chart(&serde_json::json!({"sort": {"by": "visitors", "dir": "desc"}, "other": true})),
        &[],
        policy,
    );
    assert!(out.contains("'Other'"), "{out}");
    rewritten(
        "other with an average",
        &chart(&serde_json::json!({
            "measures": [{"column": "visitors", "aggregate": "avg"}],
            "sort": {"by": "avg_visitors", "dir": "asc"}, "other": true,
        })),
        &[],
        policy,
    );
    rewritten(
        "other per series",
        &chart(&serde_json::json!({
            "breakdown": "category", "sort": {"by": "visitors", "dir": "desc"}, "other": true,
        })),
        &[],
        policy,
    );
}

#[test]
fn a_sort_beside_a_breakdown_reads_the_governed_table_only_through_its_projection() {
    for policy in [&MASK_MEASURE, &MASK_TEXT] {
        sort_with_breakdown_cases(policy);
    }
}

fn sort_with_breakdown_cases(policy: &Governed) {
    rewritten(
        "sort with a breakdown",
        &chart(&serde_json::json!({
            "breakdown": "category", "sort": {"by": "visitors", "dir": "desc"},
        })),
        &[],
        policy,
    );
    rewritten(
        "sort by the dimension with a breakdown",
        &chart(&serde_json::json!({
            "breakdown": "category", "sort": {"by": "provinsi", "dir": "desc"},
        })),
        &[],
        policy,
    );
}

#[test]
fn a_table_grouped_by_several_columns_reads_the_governed_table_only_through_its_projection() {
    for policy in [&MASK_MEASURE, &MASK_TEXT] {
        groupings_case(policy);
    }
}

fn groupings_case(policy: &Governed) {
    rewritten(
        "groupings",
        &chart(&serde_json::json!({
            "kind": "table", "dimension": "",
            "groupings": [{"column": "category"}, {"column": "visit_date", "grain": "month"}],
            "measures": ["visitors", {"column": "visitors", "aggregate": "avg"}],
        })),
        &[],
        policy,
    );
}

#[test]
fn the_charts_own_filters_and_the_dashboards_apply_on_top_of_the_row_filter() {
    let policy = &MASK_MEASURE;
    let own = serde_json::json!({
        "filters": [{"column": "category", "values": ["Office"]}],
        "sort": {"by": "visitors", "dir": "desc"}, "other": true,
    });
    let out = rewritten(
        "own filter and dashboard filter on one column",
        &chart(&own),
        &[serde_json::from_value(serde_json::json!({
            "column": "category", "values": ["Retail"],
        }))
        .unwrap()],
        policy,
    );
    // The two filters narrow the rows the policy already left; they never
    // replace it.
    assert!(
        out.contains("category IN ('Office') AND category IN ('Retail')"),
        "{out}"
    );
}

#[test]
fn a_filter_on_a_masked_column_compares_the_masked_value_not_the_clear_one() {
    // `visitors` is masked for this role: a chart filter on it sits in an
    // inner `SELECT *` over the projection, so it can only see the mask.
    let out = rewritten(
        "own filter on a masked column",
        &chart(&serde_json::json!({
            "filters": [{"column": "visitors", "op": "between", "min": "100"}],
        })),
        &[],
        &MASK_MEASURE,
    );
    let mask_at = out.find("replaceRegexpOne").unwrap();
    let filter_at = out.find("visitors >= 100").unwrap();
    assert!(mask_at < filter_at, "{out}");
}

// ── the statements themselves, for a real-engine run ────────────────────

fn record_raw(label: &str, sql: &str) {
    let Ok(path) = std::env::var("CHART_SHAPES_OUT") else {
        return;
    };
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .unwrap();
    writeln!(file, "-- {label}\n{}", sql.replace('\n', " ")).unwrap();
}

fn demo_field(name: &str, formula: &str) -> lakehouse_bi::fields::FieldDef {
    use lakehouse_bi::fields::{self, SourceKind};
    let (level, ty) = fields::check(
        formula,
        name,
        &[],
        &columns()[MART],
        &TimeContext::default(),
    )
    .unwrap();
    lakehouse_bi::fields::FieldDef {
        id: format!("f_{name}"),
        source_kind: SourceKind::Mart,
        source_id: MART.to_owned(),
        name: name.to_owned(),
        formula: formula.to_owned(),
        level: level.as_str().to_owned(),
        ty: ty.as_str().to_owned(),
        created_by: String::new(),
        updated_at: None,
    }
}

fn dashboard_filter(json: &serde_json::Value) -> FilterDef {
    serde_json::from_value(json.clone()).unwrap()
}

/// The statements for the intersection checks and the shapes of `BI-6`, run
/// by hand on the real engine (see the plan's handoff for the counts). They
/// assert only that a statement was built; the numbers are the engine's.
#[test]
fn the_statements_for_a_real_engine_run_are_built() {
    let time = TimeContext::default();
    let own = serde_json::json!([{"column": "category", "values": ["Office", "Retail"]}]);
    let dash = [dashboard_filter(
        &serde_json::json!({"column": "category", "values": ["Retail", "Warehouse"]}),
    )];
    let read = ReadContext::new(&time);
    let build =
        |label: &str, chart: &StoredChartSpec, filters: &[FilterDef], read: &ReadContext<'_>| {
            let sql = sql_with_filters_report(chart, &[], filters, &columns(), read).sql;
            assert!(sql.starts_with("SELECT"), "{label}: {sql}");
            record_raw(label, &sql);
        };
    build(
        "intersection, plain: own Office+Retail, dashboard Retail+Warehouse",
        &chart(&serde_json::json!({"filters": own, "limit": 100})),
        &dash,
        &read,
    );
    build(
        "intersection, plain: own only",
        &chart(&serde_json::json!({"filters": own, "limit": 100})),
        &[],
        &read,
    );
    build(
        "intersection, with a breakdown",
        &chart(&serde_json::json!({"filters": own, "breakdown": "kab_kota", "limit": 100})),
        &dash,
        &read,
    );
    build(
        "intersection, with a grain",
        &chart(&serde_json::json!({
            "filters": own, "dimension": "visit_date", "grain": "month", "order": "none", "limit": 100,
        })),
        &dash,
        &read,
    );
    // A per-row calculated field the chart filters on.
    let catalog = lakehouse_bi::fields::FieldCatalog::from_fields(vec![demo_field(
        "is_big",
        "If(visitors > 1000, 'big', 'small')",
    )]);
    let with_fields = ReadContext {
        time: &time,
        grain: None,
        fields: &catalog,
    };
    build(
        "intersection, with a calculated field in the chart filter",
        &chart(&serde_json::json!({
            "filters": [{"column": "is_big", "values": ["big"]}], "limit": 100,
        })),
        &dash,
        &with_fields,
    );
    build(
        "calculated field filter plus Other",
        &chart(&serde_json::json!({
            "filters": [{"column": "is_big", "values": ["big"]}],
            "sort": {"by": "visitors", "dir": "desc"}, "limit": 3, "other": true,
        })),
        &[],
        &with_fields,
    );
}

/// The statements for the own-aggregate, "Other" and groupings checks, and one
/// over a SQL source, run by hand on the real engine.
#[test]
fn the_statements_for_the_new_shapes_for_a_real_engine_run_are_built() {
    let time = TimeContext::default();
    let own = serde_json::json!([{"column": "category", "values": ["Office", "Retail"]}]);
    let dash = [dashboard_filter(
        &serde_json::json!({"column": "category", "values": ["Retail", "Warehouse"]}),
    )];
    let read = ReadContext::new(&time);
    let build =
        |label: &str, chart: &StoredChartSpec, filters: &[FilterDef], read: &ReadContext<'_>| {
            let sql = sql_with_filters_report(chart, &[], filters, &columns(), read).sql;
            assert!(sql.starts_with("SELECT"), "{label}: {sql}");
            record_raw(label, &sql);
        };
    // Own aggregates: two aggregations of one column do not collide.
    build(
        "two aggregations of one column",
        &chart(&serde_json::json!({
            "measures": ["visitors", {"column": "visitors", "aggregate": "avg"}, {"column": "visitors", "aggregate": "max"}],
            "order": "none", "limit": 100,
        })),
        &[],
        &read,
    );
    build(
        "top 5 by sum with Other",
        &chart(
            &serde_json::json!({"sort": {"by": "visitors", "dir": "desc"}, "limit": 5, "other": true}),
        ),
        &[],
        &read,
    );
    build(
        "top 5 by avg with Other",
        &chart(&serde_json::json!({
            "measures": [{"column": "visitors", "aggregate": "avg"}],
            "sort": {"by": "avg_visitors", "dir": "desc"}, "limit": 5, "other": true,
        })),
        &[],
        &read,
    );
    build(
        "groupings with a month grain and a filter",
        &chart(&serde_json::json!({
            "kind": "table", "dimension": "",
            "groupings": [{"column": "visit_date", "grain": "month"}, {"column": "category"}],
            "measures": ["visitors", {"column": "visitors", "aggregate": "avg"}],
            "filters": own, "sort": {"by": "visitors", "dir": "desc"}, "limit": 10,
        })),
        &[],
        &read,
    );
    // Over a SQL source: the same chart rebuilt from the source's text.
    let source_cols: RelationColumns = columns()[MART].clone();
    let source_chart = chart(&serde_json::json!({"filters": own, "limit": 100}));
    let sql = lakehouse_bi::builder::sql_for_sql_source_report(
        &source_chart,
        "SELECT provinsi, category, visitors FROM serving.mart_demo_map_points",
        &source_cols,
        &[],
        &dash,
        &read,
    )
    .unwrap()
    .sql;
    record_raw("intersection, over a SQL source", &sql);
}

/// `BI-6-AC2`: visitors by province and month, then summarised by province, then
/// the provinces whose total is over 1,000.
fn ac2_chart() -> StoredChartSpec {
    chart(&serde_json::json!({
        "kind": "table", "dimension": "",
        "groupings": [{"column": "provinsi"}, {"column": "visit_date", "grain": "month"}],
        "measures": ["visitors"],
        "steps": [
            {"summarise": {"by": ["provinsi"], "measures": [{"column": "visitors", "aggregate": "sum"}]}},
            {"filter": [{"column": "sum_visitors", "op": "gt", "value": "1000"}]},
        ],
        "sort": {"by": "sum_visitors", "dir": "desc"}, "limit": 100,
    }))
}

/// Summarise twice: by province and category taking the largest cell, then by
/// province adding those up.
fn summarise_twice_chart() -> StoredChartSpec {
    chart(&serde_json::json!({
        "kind": "table", "dimension": "",
        "groupings": [{"column": "provinsi"}, {"column": "category"}, {"column": "visit_date", "grain": "month"}],
        "measures": ["visitors"],
        "steps": [
            {"summarise": {"by": ["provinsi", "category"], "measures": [{"column": "visitors", "aggregate": "max"}]}},
            {"summarise": {"by": ["provinsi"], "measures": [{"column": "max_visitors", "aggregate": "sum"}]}},
        ],
        "sort": {"by": "sum_max_visitors", "dir": "desc"}, "limit": 100,
    }))
}

/// A filter step on the cells, then a filter step on a grouping, over a plain bar.
fn filter_filter_chart() -> StoredChartSpec {
    chart(&serde_json::json!({
        "measures": ["visitors"],
        "steps": [
            {"filter": [{"column": "visitors", "op": "between", "min": "5000", "max": "50000"}]},
            {"filter": [{"column": "provinsi", "op": "ne", "value": "Bali"}]},
        ],
        "sort": {"by": "visitors", "dir": "desc"}, "limit": 100,
    }))
}

#[test]
fn nested_steps_read_the_governed_table_only_through_its_projection() {
    for policy in [&MASK_MEASURE, &MASK_TEXT] {
        rewritten("steps: AC2", &ac2_chart(), &[], policy);
        rewritten(
            "steps: summarise twice",
            &summarise_twice_chart(),
            &[],
            policy,
        );
        rewritten("steps: two filters", &filter_filter_chart(), &[], policy);
        rewritten(
            "steps: own filter and dashboard filter under steps",
            &chart(&serde_json::json!({
                "measures": ["visitors"],
                "filters": [{"column": "category", "values": ["Office"]}],
                "steps": [{"filter": [{"column": "visitors", "op": "gt", "value": "10"}]}],
                "limit": 100,
            })),
            &[dashboard_filter(
                &serde_json::json!({"column": "category", "values": ["Office", "Retail"]}),
            )],
            policy,
        );
    }
}

/// The nested statements, for a real-engine run.
#[test]
fn the_statements_with_steps_for_a_real_engine_run_are_built() {
    let time = TimeContext::default();
    let read = ReadContext::new(&time);
    for (label, c) in [
        (
            "steps AC2: by province and month, summarise by province, keep totals over 1000",
            ac2_chart(),
        ),
        ("steps: summarise twice", summarise_twice_chart()),
        ("steps: two filters", filter_filter_chart()),
    ] {
        let sql = sql_with_filters_report(&c, &[], &[], &columns(), &read).sql;
        assert!(sql.starts_with("SELECT * FROM ("), "{label}: {sql}");
        record_raw(label, &sql);
    }
}

/// `BI-6` part 2, T7: the builder's preview shows up to 2,000 rows of the last
/// step's result, read with a larger limit than the chart keeps, and says when
/// more exist; a chart without steps shows the rows its tile has.
#[tokio::test]
async fn the_preview_rows_are_read_past_the_charts_limit_and_say_when_they_are_cut() {
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    let rows: Vec<serde_json::Value> = (0..2001)
        .map(|i| serde_json::json!({ "provinsi": format!("p{i}"), "visitors": i }))
        .collect();
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "meta": [{"name": "provinsi", "type": "String"}, {"name": "visitors", "type": "UInt32"}],
            "data": rows, "rows": 2001,
        })))
        .mount(&server)
        .await;
    let ch = lakehouse_clickhouse::ChClient::new(server.uri(), "default".to_owned(), String::new());
    let obligations = crate::policy_engine::PolicyEngineObligations::new(None, &ch);
    let time = TimeContext::default();
    let mut stepped = filter_filter_chart();
    stepped.spec.sql =
        sql_with_filters_report(&stepped, &[], &[], &columns(), &ReadContext::new(&time)).sql;
    let shown_tile = serde_json::json!({ "columns": ["provinsi", "visitors"], "rows": [{"provinsi": "a", "visitors": 1}] });
    let shown = crate::routes::support::preview_rows(
        &ch,
        &stepped,
        &shown_tile,
        (&[], &PlaceholderValues::none(), &obligations),
    )
    .await
    .unwrap();
    assert_eq!(shown["rows"].as_array().unwrap().len(), 2000);
    assert_eq!(shown["truncated"], true);
    assert_eq!(shown["limit"], 2000);
    let sent: Vec<String> = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(|r| String::from_utf8_lossy(&r.body).into_owned())
        .collect();
    assert!(sent.iter().any(|s| s.contains("LIMIT 2001")), "{sent:?}");
    assert!(
        stepped.def.limit_value() == Some(100),
        "the saved chart's own limit is not changed"
    );

    // Without steps the rows are the shown_tile's own, and nothing more is read.
    let plain = chart(&serde_json::json!({"limit": 5}));
    let shown = crate::routes::support::preview_rows(
        &ch,
        &plain,
        &shown_tile,
        (&[], &PlaceholderValues::none(), &obligations),
    )
    .await
    .unwrap();
    assert_eq!(shown["rows"].as_array().unwrap().len(), 1);
    assert_eq!(shown["truncated"], false);
    assert_eq!(server.received_requests().await.unwrap().len(), sent.len());
    // A shown_tile that carries an error has no rows to show.
    assert!(
        crate::routes::support::preview_rows(
            &ch,
            &plain,
            &serde_json::json!({ "error": "x" }),
            (&[], &PlaceholderValues::none(), &obligations),
        )
        .await
        .is_none()
    );
}
