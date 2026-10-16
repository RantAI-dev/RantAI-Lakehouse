//! `BI-7`: the role rewrite on charts that join tables.
//!
//! A join is exactly where a permission rule on one table could be lost, so
//! these tests run the real [`enforce`] on the statements the builder produces
//! for joined charts (each join type, a table joined twice, a grain, a pivot,
//! the dashboard's and the chart's own filters) with a mask and a row filter on
//! the joined table, and separately on the base table, and assert that
//! *every* read of *each* governed table became its masked, row-filtered
//! projection: the table's name appears exactly as many times in the rewritten
//! statement as in the original, and each read carries the mask and the row
//! filter. The raw and the rewritten statements are written to the files named
//! by `JOIN_STATEMENTS_OUT` and `JOIN_REWRITE_OUT`, so they can be run by hand
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
    NoViews, ObligationsSource, PlaceholderValues, TableObligations, enforce,
};

const VISITS: &str = "serving.mart_demo_map_points";
const CAPACITY: &str = "serving.mart_demo_province_capacity";
const VISITS_NAME: &str = "mart_demo_map_points";
const CAPACITY_NAME: &str = "mart_demo_province_capacity";

/// The obligations of one table.
#[derive(Clone)]
struct Rule {
    mask: &'static [&'static str],
    row_filter: &'static str,
}

/// Obligations by table (canonical `serving.<table>` name).
struct Governed(&'static str, HashMap<&'static str, Rule>);

fn real_columns(table: &str) -> Vec<String> {
    let cols: &[&str] = if table == VISITS {
        &[
            "place",
            "kab_kota",
            "provinsi",
            "lat",
            "lon",
            "visitors",
            "category",
            "visit_date",
        ]
    } else {
        &["provinsi", "hotels", "rooms"]
    };
    cols.iter().map(|c| (*c).to_owned()).collect()
}

impl ObligationsSource for Governed {
    fn obligations_for(&self, table: &str, _roles: &[String]) -> Option<TableObligations> {
        self.1.get(table).map(|rule| TableObligations {
            mask: rule.mask.iter().map(|m| (*m).to_owned()).collect(),
            row_filter: Some(rule.row_filter.to_owned()),
            real_columns: Some(real_columns(table)),
        })
    }
    fn has_any_obligation(&self, _roles: &[String]) -> bool {
        true
    }
}

/// A mask and a row filter on the joined table only.
fn on_the_joined_table() -> Governed {
    Governed(
        "joined-table",
        HashMap::from([(
            CAPACITY,
            Rule {
                mask: &["rooms"],
                row_filter: "hotels > 100",
            },
        )]),
    )
}

/// A mask and a row filter on the base table only.
fn on_the_base_table() -> Governed {
    Governed(
        "base-table",
        HashMap::from([(
            VISITS,
            Rule {
                mask: &["place", "category"],
                row_filter: "provinsi = 'Dki Jakarta'",
            },
        )]),
    )
}

/// Both tables governed.
fn on_both_tables() -> Governed {
    Governed(
        "both-tables",
        HashMap::from([
            (
                VISITS,
                Rule {
                    mask: &["place", "category"],
                    row_filter: "provinsi = 'Dki Jakarta'",
                },
            ),
            (
                CAPACITY,
                Rule {
                    mask: &["rooms"],
                    row_filter: "hotels > 100",
                },
            ),
        ]),
    )
}

fn tables() -> HashMap<String, RelationColumns> {
    let cols = |pairs: &[(&str, ColumnKind)]| -> RelationColumns {
        pairs.iter().map(|(n, k)| ((*n).to_owned(), *k)).collect()
    };
    HashMap::from([
        (
            VISITS_NAME.to_owned(),
            cols(&[
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
            CAPACITY_NAME.to_owned(),
            cols(&[
                ("provinsi", ColumnKind::Text),
                ("hotels", ColumnKind::Number),
                ("rooms", ColumnKind::Number),
            ]),
        ),
    ])
}

fn join_to_capacity(join_type: &str) -> serde_json::Value {
    serde_json::json!({
        "mart": CAPACITY_NAME, "type": join_type,
        "on": [{ "left": "provinsi", "op": "=", "right": "provinsi" }],
    })
}

/// A stored chart on the demo visits table with `extra` laid over a bar chart
/// that joins the capacity table.
fn chart(join_type: &str, extra: &serde_json::Value) -> StoredChartSpec {
    let mut def = serde_json::json!({
        "title": "t", "kind": "bar", "mart": VISITS_NAME,
        "dimension": format!("{CAPACITY_NAME}__provinsi"),
        "measures": ["visitors", { "column": format!("{CAPACITY_NAME}__rooms"), "aggregate": "max" }],
        "aggregate": "sum", "limit": 100, "order": "desc",
        "joins": [join_to_capacity(join_type)],
    });
    def.as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    let def: ChartInput = serde_json::from_value(def).unwrap();
    let spec: ChartSpec = serde_json::from_value(serde_json::json!({
        "id": "u_1", "title": "t", "kind": def.kind, "mart": VISITS_NAME, "sql": "SELECT 1",
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

/// Occurrences of `table` in `sql` as a table reference (`serving.x` not
/// followed by more identifier characters, so `serving.mart_demo_map_points`
/// is not counted inside a longer name).
fn reads_of(sql: &str, table: &str) -> usize {
    sql.match_indices(table)
        .filter(|(at, _)| {
            !sql[at + table.len()..]
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
        })
        .count()
}

/// The statement the dashboard builds for `chart`, rewritten for a role under
/// `policy`, checked: every read of every governed table is its masked,
/// row-filtered projection.
fn rewritten(
    label: &str,
    chart: &StoredChartSpec,
    dashboard: &[FilterDef],
    policy: &Governed,
) -> String {
    let time = TimeContext::default();
    let sql =
        sql_with_filters_report(chart, &[], dashboard, &tables(), &ReadContext::new(&time)).sql;
    let label = format!("{label} [{}]", policy.0);
    let label = label.as_str();
    append("JOIN_STATEMENTS_OUT", label, &sql);
    let out = enforce(
        &sql,
        &ClickHouseDialect {},
        &["analyst".to_owned()],
        &PlaceholderValues::none(),
        policy,
        &NoViews,
    )
    .unwrap_or_else(|failure| panic!("{label}: refused: {failure}\n{sql}"));
    append("JOIN_REWRITE_OUT", label, &out);
    for (table, rule) in &policy.1 {
        let reads = reads_of(&sql, table);
        assert!(
            reads >= 1,
            "{label}: the statement never reads {table}: {sql}"
        );
        // Each read is replaced by a derived table over the same table, so the
        // name appears once per read, inside the projection, never bare.
        assert_eq!(reads_of(&out, table), reads, "{label}: {out}");
        for column in rule.mask {
            // No other table has a column of this name in the demo tables, so
            // the mask appears once per read of its own table.
            assert_eq!(
                out.matches(&format!(
                    "replaceRegexpOne(toString(`{column}`), '(?s)^.*$', '***') AS `{column}`"
                ))
                .count(),
                reads,
                "{label}: a read of {table} does not mask {column}: {out}"
            );
        }
        assert_eq!(
            out.matches(&format!("WHERE {}", rule.row_filter)).count(),
            reads,
            "{label}: a read of {table} is not row-filtered: {out}"
        );
    }
    out
}

#[test]
fn every_join_type_reads_each_governed_table_only_through_its_projection() {
    for policy in [on_the_joined_table(), on_the_base_table(), on_both_tables()] {
        for join_type in ["inner", "left", "right", "full"] {
            rewritten(
                &format!("{join_type} join"),
                &chart(join_type, &serde_json::json!({})),
                &[],
                &policy,
            );
        }
    }
}

#[test]
fn the_same_table_joined_twice_is_governed_at_both_reads() {
    let twice = serde_json::json!({
        "joins": [
            join_to_capacity("left"),
            {
                "mart": CAPACITY_NAME, "type": "left",
                "on": [{ "left": "provinsi", "op": "=", "right": "provinsi" }],
            },
        ],
        "dimension": "provinsi",
        "measures": [
            "visitors",
            { "column": format!("{CAPACITY_NAME}__rooms"), "aggregate": "max" },
            { "column": format!("{CAPACITY_NAME}2__hotels"), "aggregate": "max" },
        ],
    });
    for policy in [on_the_joined_table(), on_both_tables()] {
        let out = rewritten(
            "capacity joined twice",
            &chart("left", &twice),
            &[],
            &policy,
        );
        assert_eq!(reads_of(&out, CAPACITY), 2, "{out}");
    }
}

#[test]
fn a_comparison_condition_a_grain_a_pivot_and_the_filters_stay_governed() {
    let policy = on_both_tables();
    let compare = serde_json::json!({
        "joins": [{
            "mart": CAPACITY_NAME, "type": "left",
            "on": [
                { "left": "provinsi", "op": "=", "right": "provinsi" },
                { "left": "visitors", "op": "<=", "right": "rooms" },
            ],
        }],
    });
    rewritten("comparison pair", &chart("left", &compare), &[], &policy);

    let grain = serde_json::json!({
        "kind": "line", "dimension": "visit_date", "grain": "month", "order": "none",
        "measures": ["visitors", { "column": format!("{CAPACITY_NAME}__hotels"), "aggregate": "max" }],
    });
    rewritten("monthly grain", &chart("left", &grain), &[], &policy);

    let pivot = serde_json::json!({
        "kind": "pivot", "dimension": "", "measures": [],
        "rows": ["provinsi"], "columns": [format!("{CAPACITY_NAME}__hotels")],
        "values": [{ "column": "visitors", "aggregate": "sum" }],
    });
    rewritten("pivot", &chart("left", &pivot), &[], &policy);

    let own = serde_json::json!({
        "filters": [{ "column": format!("{CAPACITY_NAME}__provinsi"), "values": ["Dki Jakarta"] }],
    });
    let dashboard: Vec<FilterDef> = serde_json::from_value(serde_json::json!([
        { "column": "category", "values": ["Office"] },
        { "column": format!("{CAPACITY_NAME}__hotels"), "op": "between", "min": "10" },
    ]))
    .unwrap();
    let out = rewritten(
        "own and dashboard filters on both tables",
        &chart("left", &own),
        &dashboard,
        &policy,
    );
    // The filters narrow what the policy already left; they never replace it.
    assert!(out.contains("category IN ('Office')"), "{out}");
}

#[test]
fn a_masked_column_of_the_joined_table_is_masked_before_it_is_compared_or_aggregated() {
    // `rooms` is masked for this role: a join pair on it compares the mask
    // (text), not the number, and a measure over it reads text.
    let on_rooms = serde_json::json!({
        "joins": [{
            "mart": CAPACITY_NAME, "type": "left",
            "on": [
                { "left": "provinsi", "op": "=", "right": "provinsi" },
                { "left": "visitors", "op": "<", "right": "rooms" },
            ],
        }],
    });
    let out = rewritten(
        "pair on a masked column",
        &chart("left", &on_rooms),
        &[],
        &on_the_joined_table(),
    );
    let mask_at = out.find("replaceRegexpOne").unwrap();
    let on_at = out.find(" ON ").unwrap();
    assert!(mask_at < on_at, "{out}");
}

/// Readable shapes for the real-engine run: the measures are columns no rule
/// masks, so the numbers come back and show the row filter and the mask on the
/// joined table's text column at work.
#[test]
fn the_statements_for_the_readable_engine_runs_are_governed() {
    let readable = serde_json::json!({
        "measures": ["visitors", { "column": format!("{CAPACITY_NAME}__hotels"), "aggregate": "max" }],
    });
    let mask_key = Governed(
        "mask-on-joined-key",
        HashMap::from([(
            CAPACITY,
            Rule {
                mask: &["provinsi"],
                row_filter: "hotels > 100",
            },
        )]),
    );
    for join_type in ["inner", "left", "right", "full"] {
        for policy in [
            on_the_joined_table(),
            on_the_base_table(),
            mask_key_clone(&mask_key),
        ] {
            rewritten(
                &format!("readable {join_type}"),
                &chart(join_type, &readable),
                &[],
                &policy,
            );
        }
    }
}

fn mask_key_clone(g: &Governed) -> Governed {
    Governed(g.0, g.1.clone())
}

/// Against the real engine, when `LIVE_CH_URL` names one (a no-op otherwise, so
/// CI and a plain `cargo test` never need it): the suggestions route's answer
/// for the demo tables, `fields` over a join, and what a join on a column that
/// is not unique does to a sum. The numbers are printed with `--nocapture`.
#[tokio::test]
async fn the_demo_tables_against_the_live_engine() {
    let Ok(url) = std::env::var("LIVE_CH_URL") else {
        return;
    };
    let ch = lakehouse_clickhouse::ChClient::new(url, "default".to_owned(), String::new());
    let key_join = super::join_suggestions_value(&ch, Some(VISITS_NAME), Some(CAPACITY_NAME))
        .await
        .unwrap();
    println!("SUGGEST visits->capacity: {key_join}");
    assert_eq!(key_join["coversRightKey"], true);
    assert_eq!(key_join["pairs"][0]["rank"], "keys");
    let by_province = super::join_suggestions_value(&ch, Some(VISITS_NAME), Some(VISITS_NAME))
        .await
        .unwrap();
    println!("SUGGEST visits->visits: {by_province}");
    // The three key columns are all shared, so the suggested pairs cover the key;
    // the multiplication below comes from a pair set that leaves part of it out,
    // which the builder judges with `coversKey` (a TypeScript test pins that rule).
    assert_eq!(by_province["coversRightKey"], true);
    assert_eq!(
        by_province["rightKey"].as_array().map(Vec::len),
        Some(3),
        "{by_province}"
    );
    let missing = super::join_suggestions_value(&ch, Some(VISITS_NAME), Some("no_such_mart"))
        .await
        .unwrap_err();
    println!("SUGGEST unknown table: {missing:?}");
    // A join on a column that is not unique in the joined table multiplies sums.
    let multiplied = chart(
        "inner",
        &serde_json::json!({
            "joins": [{
                "mart": VISITS_NAME, "type": "inner",
                "on": [{ "left": "provinsi", "op": "=", "right": "provinsi" }],
            }],
            "dimension": "provinsi", "measures": ["visitors"], "limit": 100,
        }),
    );
    let time = TimeContext::default();
    let mut tables = tables();
    let visits = tables[VISITS_NAME].clone();
    tables.insert(VISITS_NAME.to_owned(), visits);
    let sql = sql_with_filters_report(&multiplied, &[], &[], &tables, &ReadContext::new(&time)).sql;
    let total = ch
        .rows(
            &format!(
                "SELECT sum(visitors) AS s FROM ({})",
                sql.replace(" SETTINGS join_use_nulls = 1", "")
            ),
            None,
        )
        .await
        .unwrap();
    println!("MULTIPLIED sum(visitors) over the self-join on provinsi: {total:?}");
    println!("MULTIPLIED statement: {}", sql.replace('\n', " "));
}
