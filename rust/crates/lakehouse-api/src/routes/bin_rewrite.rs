//! `BI-10`: the role rewrite on charts that bin a number or draw a grid.
//!
//! A bin reads its table more than once: the grouping, and the scalar
//! subquery that finds the column's range (`lakehouse_bi::bin`). This is the
//! place a permission rule could be lost, so these tests run the real
//! [`enforce`] on the statements the builder produces (each bin mode, a
//! binned breakdown, a binned grouping, a pivot, a grid, with and without the
//! dashboard's filters) under a row filter and under a mask on the binned
//! column, and assert that *every* read of the table became its governed
//! projection: the table's name appears exactly as many times in the
//! rewritten statement as in the original, and each read carries the row
//! filter (and the mask). The raw and the rewritten statements are written to
//! the files named by `BIN_STATEMENTS_OUT` and `BIN_REWRITE_OUT`, so they can
//! be run by hand against the real engine (plan section 5).

#![cfg(test)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashMap;
use std::io::Write as _;

use lakehouse_bi::builder::{FilteredSql, ReadContext, RelationColumns, sql_with_filters_report};
use lakehouse_bi::filters::{ColumnKind, FilterDef};
use lakehouse_bi::grain::TimeContext;
use lakehouse_bi::specs::ChartSource;
use lakehouse_bi::store::{ChartInput, ChartSpec, StoredChartSpec};
use sqlparser::dialect::ClickHouseDialect;

use crate::sql_rewrite::{
    NoViews, ObligationsSource, PlaceholderValues, TableObligations, enforce,
};

const VISITS: &str = "serving.mart_demo_map_points";
const NAME: &str = "mart_demo_map_points";

struct Governed {
    label: &'static str,
    mask: &'static [&'static str],
    row_filter: &'static str,
}

impl ObligationsSource for Governed {
    fn obligations_for(&self, table: &str, _roles: &[String]) -> Option<TableObligations> {
        (table == VISITS).then(|| TableObligations {
            mask: self.mask.iter().map(|m| (*m).to_owned()).collect(),
            row_filter: Some(self.row_filter.to_owned()),
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

fn only_jakarta() -> Governed {
    Governed {
        label: "row-filter",
        mask: &["place"],
        row_filter: "provinsi = 'Dki Jakarta'",
    }
}

fn masks_the_binned_column() -> Governed {
    Governed {
        label: "mask-visitors",
        mask: &["visitors"],
        row_filter: "provinsi = 'Dki Jakarta'",
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
    HashMap::from([(NAME.to_owned(), cols)])
}

/// A stored chart on the demo visits table: `extra` laid over a bar chart that
/// counts rows by the binned `visitors`.
fn chart(extra: &serde_json::Value) -> StoredChartSpec {
    let mut def = serde_json::json!({
        "title": "t", "kind": "bar", "mart": NAME,
        "dimension": "visitors", "measures": ["lat"], "aggregate": "count",
        "limit": 100, "order": "none",
        "bin": { "mode": "width", "width": 500 },
    });
    def.as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    let def: ChartInput = serde_json::from_value(def).unwrap();
    let spec: ChartSpec = serde_json::from_value(serde_json::json!({
        "id": "u_1", "title": "t", "kind": def.kind, "mart": NAME, "sql": "SELECT 1",
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

fn append(env: &str, label: &str, sql: &str) {
    let Ok(path) = std::env::var(env) else {
        return;
    };
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .unwrap();
    writeln!(file, "-- {label}\n{}", sql.replace('\n', " ")).unwrap();
}

/// Occurrences of the demo table as a table reference.
fn reads_of(sql: &str) -> usize {
    sql.match_indices(VISITS)
        .filter(|(at, _)| {
            !sql[at + VISITS.len()..]
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
        })
        .count()
}

fn built(chart: &StoredChartSpec, dashboard: &[FilterDef]) -> FilteredSql {
    let time = TimeContext::default();
    sql_with_filters_report(chart, &[], dashboard, &columns(), &ReadContext::new(&time))
}

fn rewrite(label: &str, sql: &str, policy: &Governed) -> String {
    let label = format!("{label} [{}]", policy.label);
    append("BIN_STATEMENTS_OUT", &label, sql);
    let out = enforce(
        sql,
        &ClickHouseDialect {},
        &["analyst".to_owned()],
        &PlaceholderValues::none(),
        policy,
        &NoViews,
    )
    .unwrap_or_else(|failure| panic!("{label}: refused: {failure}\n{sql}"));
    append("BIN_REWRITE_OUT", &label, &out);
    let reads = reads_of(sql);
    assert!(reads >= 1, "{label}: the statement never reads the table");
    assert_eq!(reads_of(&out), reads, "{label}: {out}");
    assert_eq!(
        out.matches(&format!("WHERE {}", policy.row_filter)).count(),
        reads,
        "{label}: a read is not row-filtered: {out}"
    );
    for column in policy.mask {
        assert_eq!(
            out.matches(&format!(
                "replaceRegexpOne(toString(`{column}`), '(?s)^.*$', '***') AS `{column}`"
            ))
            .count(),
            reads,
            "{label}: a read does not mask {column}: {out}"
        );
    }
    out
}

/// The tile statement and every width statement of `chart`, rewritten.
fn governed(label: &str, chart: &StoredChartSpec, dashboard: &[FilterDef], policy: &Governed) {
    let got = built(chart, dashboard);
    rewrite(label, &got.sql, policy);
    for (i, read) in got.bins.iter().enumerate() {
        rewrite(&format!("{label} width {i}"), &read.sql, policy);
    }
}

/// The statement of a chart over a value group "region" on `provinsi`, built
/// with the group in the field catalog as the dashboard builds it.
fn grouped_chart_sql(rest: &str) -> String {
    use lakehouse_bi::fields::{FieldCatalog, FieldDef, SourceKind};
    let java = [
        "Jawa Barat",
        "Jawa Tengah",
        "Jawa Timur",
        "Banten",
        "Dki Jakarta",
        "DI Yogyakarta",
    ];
    let group = lakehouse_bi::groups::GroupDef::from_stored(
        &serde_json::json!({
            "column": "provinsi", "rest": rest,
            "groups": [{ "name": "Jawa", "values": java }],
        })
        .to_string(),
    )
    .unwrap();
    let catalog = FieldCatalog::from_fields(vec![FieldDef {
        id: "f_1".to_owned(),
        source_kind: SourceKind::Mart,
        source_id: NAME.to_owned(),
        name: "region".to_owned(),
        formula: group.to_stored(),
        level: lakehouse_bi::groups::LEVEL.to_owned(),
        ty: "text".to_owned(),
        created_by: String::new(),
        updated_at: None,
    }]);
    let time = TimeContext::default();
    let read = ReadContext {
        time: &time,
        grain: None,
        fields: &catalog,
    };
    let chart = chart(&serde_json::json!({
        "dimension": "region", "bin": null, "measures": ["visitors"], "aggregate": "sum",
        "order": "desc",
    }));
    sql_with_filters_report(&chart, &[], &[], &columns(), &read).sql
}

#[test]
fn a_value_group_reads_the_table_through_its_governed_projection_and_sees_masked_values() {
    for rest in ["other", "keep"] {
        let sql = grouped_chart_sql(rest);
        assert!(sql.contains("multiIf(toString(provinsi) IN ("), "{sql}");
        append("BIN_STATEMENTS_OUT", &format!("group {rest}"), &sql);
        rewrite(&format!("group {rest}"), &sql, &only_jakarta());
        // A mask on the grouped column: the group reads the masked text, so no
        // value matches and every row falls to the rest.
        let masked = Governed {
            label: "mask-provinsi",
            mask: &["provinsi"],
            row_filter: "category = 'Office'",
        };
        let out = rewrite(&format!("group {rest}"), &sql, &masked);
        assert!(
            out.contains("replaceRegexpOne(toString(`provinsi`)"),
            "{out}"
        );
    }
}

/// The chart a drill builds (`routes::drill`): the saved chart one level down
/// with the click as one more filter. It is an ordinary tile statement, so the
/// role's row filter holds whatever the request says.
#[test]
fn a_drilled_level_is_an_ordinary_governed_statement_and_keeps_the_saved_filters() {
    let drilled = chart(&serde_json::json!({
        "dimension": "kab_kota", "bin": null, "measures": ["visitors"], "aggregate": "sum",
        "order": "desc",
        "filters": [
            { "column": "category", "values": ["Office", "Retail"] },
            { "column": "provinsi", "values": ["Jawa Barat"] },
        ],
    }));
    let sql = built(&drilled, &[]).sql;
    append("BIN_STATEMENTS_OUT", "drilled level", &sql);
    let out = rewrite("drilled level", &sql, &only_jakarta());
    // The saved chart's own filter is still in the statement, beside the click.
    assert!(
        out.contains("category IN ('Office', 'Retail')")
            || out.contains("category IN ('Office','Retail')"),
        "{out}"
    );
    assert!(out.contains("provinsi IN ('Jawa Barat')"), "{out}");
}

fn dashboard_filter(json: &str) -> FilterDef {
    serde_json::from_str(json).unwrap()
}

#[test]
fn every_bin_mode_reads_the_table_only_through_its_governed_projection() {
    for policy in [only_jakarta(), masks_the_binned_column()] {
        for bin in [
            serde_json::json!({ "mode": "width", "width": 500 }),
            serde_json::json!({ "mode": "auto" }),
            serde_json::json!({ "mode": "count", "count": 10 }),
            serde_json::json!({ "mode": "count", "count": 100 }),
        ] {
            governed(
                &format!("bin {bin}"),
                &chart(&serde_json::json!({ "bin": bin })),
                &[],
                &policy,
            );
        }
    }
}

#[test]
fn the_dashboards_filters_and_the_charts_own_filters_stay_under_the_bin_and_governed() {
    let own = serde_json::json!({
        "bin": { "mode": "auto" },
        "filters": [{ "column": "category", "values": ["Office"] }],
    });
    let dash = [
        dashboard_filter(r#"{"column":"visitors","op":"between","min":"10"}"#),
        dashboard_filter(r#"{"column":"category","values":["Office","Retail"]}"#),
    ];
    governed("filters", &chart(&own), &dash, &only_jakarta());
}

#[test]
fn a_binned_breakdown_a_binned_grouping_a_pivot_and_other_stay_governed() {
    let policy = only_jakarta();
    governed(
        "binned breakdown",
        &chart(&serde_json::json!({
            "dimension": "category", "bin": null,
            "breakdown": "visitors", "breakdownBin": { "mode": "count", "count": 10 },
            "measures": ["lat"], "aggregate": "avg",
        })),
        &[],
        &policy,
    );
    governed(
        "binned dimension and breakdown",
        &chart(&serde_json::json!({
            "bin": { "mode": "auto" },
            "breakdown": "lon", "breakdownBin": { "mode": "width", "width": 5 },
        })),
        &[],
        &policy,
    );
    governed(
        "grouped table",
        &chart(&serde_json::json!({
            "kind": "table", "dimension": "", "bin": null,
            "groupings": [
                { "column": "visitors", "bin": { "mode": "auto" } },
                { "column": "category" },
            ],
            "measures": ["lat"], "aggregate": "avg",
        })),
        &[],
        &policy,
    );
    governed(
        "pivot",
        &chart(&serde_json::json!({
            "kind": "pivot", "dimension": "", "measures": [],
            "rows": ["visitors"], "columns": ["category"],
            "values": [{ "column": "lat", "aggregate": "avg" }],
            "bin": { "mode": "auto" },
        })),
        &[],
        &policy,
    );
    governed(
        "other",
        &chart(&serde_json::json!({
            "bin": { "mode": "width", "width": 100 },
            "limit": 3, "sort": { "by": "lat", "dir": "desc" }, "other": true,
        })),
        &[],
        &policy,
    );
}

#[test]
fn a_grid_reads_the_points_only_through_the_governed_projection() {
    for size in [serde_json::json!(1), serde_json::json!("auto")] {
        let grid = chart(&serde_json::json!({
            "kind": "pointmap", "dimension": "", "bin": null,
            "lat": "lat", "lon": "lon", "measures": ["visitors"], "aggregate": "sum",
            "grid": { "size": size },
        }));
        for policy in [only_jakarta(), masks_the_binned_column()] {
            governed(
                &format!("grid {size}"),
                &grid,
                &[dashboard_filter(
                    r#"{"column":"category","values":["Office"]}"#,
                )],
                &policy,
            );
        }
    }
}

#[test]
fn a_chart_saved_before_bins_builds_no_range_statement() {
    let plain = chart(&serde_json::json!({
        "dimension": "category", "bin": null, "measures": ["visitors"], "aggregate": "sum",
    }));
    let got = built(
        &plain,
        &[dashboard_filter(
            r#"{"column":"visitors","op":"between","min":"10"}"#,
        )],
    );
    assert!(!got.sql.contains("rng"), "{}", got.sql);
    assert!(got.bins.is_empty());
}
