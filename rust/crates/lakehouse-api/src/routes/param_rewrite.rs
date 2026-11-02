//! `BI-11`: parameters under the real role rewrite.
//!
//! A parameter's value reaches a statement in exactly three ways: as a
//! literal inside a calculated field's formula, as the number of rows of a
//! chart, and as the name of a column a list parameter switches to. This file
//! builds those statements the way the dashboard does (`effective_chart`,
//! `stored_chart_sql`), passes each through the real [`enforce`] with real
//! obligations (a row filter and a mask), and asserts that every read of the
//! table became its governed projection and that nothing of a hostile value
//! is left outside a literal. The raw and the rewritten statements are
//! appended to the files named by `PARAM_STATEMENTS_OUT` and
//! `PARAM_REWRITE_OUT`, so the same statements can be run by hand against the
//! real engine (plan section 4).

#![cfg(test)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashMap;
use std::io::Write as _;

use lakehouse_bi::builder::{ReadContext, RelationColumns};
use lakehouse_bi::fields::{FieldCatalog, FieldDef, SourceKind};
use lakehouse_bi::filters::ColumnKind;
use lakehouse_bi::grain::TimeContext;
use lakehouse_bi::params::{ParamDef, ParamType, ParamValues, validate_def};
use lakehouse_bi::specs::ChartSource;
use lakehouse_bi::store::{ChartInput, ChartSpec, StoredChartSpec};
use serde_json::{Value, json};
use sqlparser::dialect::ClickHouseDialect;

use crate::routes::support::{effective_chart, stored_chart_sql};
use crate::sql_rewrite::{
    Engine, NoViews, ObligationsSource, PlaceholderValues, TableObligations, enforce,
};

const POINTS: &str = "mart_demo_map_points";
const CAPACITY: &str = "mart_demo_province_capacity";

struct Governed {
    label: &'static str,
    table: &'static str,
    columns: &'static [&'static str],
    mask: &'static [&'static str],
    row_filter: &'static str,
}

impl ObligationsSource for Governed {
    fn obligations_for(&self, table: &str, _roles: &[String]) -> Option<TableObligations> {
        (table == format!("serving.{}", self.table)).then(|| TableObligations {
            mask: self.mask.iter().map(|m| (*m).to_owned()).collect(),
            row_filter: Some(self.row_filter.to_owned()),
            real_columns: Some(self.columns.iter().map(|c| (*c).to_owned()).collect()),
        })
    }
    fn has_any_obligation(&self, _roles: &[String]) -> bool {
        true
    }
}

fn points(label: &'static str, mask: &'static [&'static str]) -> Governed {
    Governed {
        label,
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
        row_filter: "provinsi = 'Dki Jakarta'",
    }
}

fn capacity(label: &'static str, mask: &'static [&'static str]) -> Governed {
    Governed {
        label,
        table: CAPACITY,
        columns: &["provinsi", "hotels", "rooms"],
        mask,
        row_filter: "provinsi <> 'Bali'",
    }
}

fn marts() -> HashMap<String, RelationColumns> {
    let points: RelationColumns = [
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
    let capacity: RelationColumns = [
        ("provinsi", ColumnKind::Text),
        ("hotels", ColumnKind::Number),
        ("rooms", ColumnKind::Number),
    ]
    .into_iter()
    .map(|(n, k)| (n.to_owned(), k))
    .collect();
    HashMap::from([(POINTS.to_owned(), points), (CAPACITY.to_owned(), capacity)])
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

fn reads_of(sql: &str, table: &str) -> usize {
    let full = format!("serving.{table}");
    sql.match_indices(&full)
        .filter(|(at, _)| {
            !sql[at + full.len()..]
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
        })
        .count()
}

fn rewrite(label: &str, sql: &str, policy: &Governed) -> String {
    let label = format!("{label} [{}]", policy.label);
    append("PARAM_STATEMENTS_OUT", &label, sql);
    let out = enforce(
        sql,
        &ClickHouseDialect {},
        Engine::ClickHouse,
        &["analyst".to_owned()],
        &PlaceholderValues::none(),
        policy,
        &NoViews,
    )
    .unwrap_or_else(|failure| panic!("{label}: refused: {failure}\n{sql}"));
    append("PARAM_REWRITE_OUT", &label, &out);
    let reads = reads_of(sql, policy.table);
    assert!(reads >= 1, "{label}: the statement never reads the table");
    assert_eq!(reads_of(&out, policy.table), reads, "{label}: {out}");
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

fn def(name: &str, ty: ParamType, default: impl serde::Serialize, extra: &Value) -> ParamDef {
    let mut v = json!({ "name": name, "type": ty, "default": default });
    v.as_object_mut()
        .unwrap()
        .extend(extra.as_object().cloned().unwrap_or_default());
    validate_def(&serde_json::from_value(v).unwrap()).unwrap()
}

fn list(name: &str, values: &[&str], default: &str) -> ParamDef {
    let entries: Vec<Value> = values
        .iter()
        .map(|v| json!({ "value": v, "label": v }))
        .collect();
    validate_def(
        &serde_json::from_value(json!({
            "name": name, "type": "list", "default": default, "values": entries,
        }))
        .unwrap(),
    )
    .unwrap()
}

fn values(defs: &[ParamDef], request: &Value) -> ParamValues {
    ParamValues::resolve(defs, &[], request.as_object()).unwrap()
}

fn chart(mart: &str, extra: &Value) -> StoredChartSpec {
    let mut def = json!({
        "title": "t", "kind": "bar", "mart": mart,
        "dimension": "provinsi", "measures": ["visitors"], "aggregate": "sum",
        "limit": 20, "order": "desc",
    });
    def.as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    let def: ChartInput = serde_json::from_value(def).unwrap();
    let spec: ChartSpec = serde_json::from_value(json!({
        "id": "u_1", "title": "t", "kind": def.kind, "mart": mart, "sql": "SELECT 1",
        "x": "provinsi", "y": "visitors",
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

fn field(mart: &str, name: &str, formula: &str, ty: &str) -> FieldDef {
    FieldDef {
        id: format!("f_{name}"),
        source_kind: SourceKind::Mart,
        source_id: mart.to_owned(),
        name: name.to_owned(),
        formula: formula.to_owned(),
        level: "row".to_owned(),
        ty: ty.to_owned(),
        created_by: String::new(),
        updated_at: None,
    }
}

/// The tile statement of `chart` the way the dashboard builds it, or the
/// tile's sentence.
fn tile_sql(
    chart: &StoredChartSpec,
    params: ParamValues,
    fields: Vec<FieldDef>,
) -> Result<String, String> {
    let time = TimeContext::default().with_params(params);
    let catalog = FieldCatalog::from_fields(fields);
    let read = ReadContext {
        time: &time,
        grain: None,
        fields: &catalog,
    };
    let cols = marts();
    let sources = HashMap::new();
    let effective = effective_chart(chart, &cols, &sources, &read)?;
    stored_chart_sql(&effective, &[], &[], &cols, &sources, &read)
        .map(|f| f.sql)
        .map_err(str::to_owned)
}

/// `BI-11-AC1`: the same chart under two values of `target`, each statement
/// governed. The engine run of these two statements is in the plan.
#[test]
fn a_formula_parameter_is_a_literal_in_a_governed_statement() {
    let defs = [def("target", ParamType::Number, json!(1000), &json!({}))];
    let ratio = field(POINTS, "ratio", "[visitors] / {target}", "number");
    let c = chart(
        POINTS,
        &json!({ "measures": ["ratio"], "aggregate": "sum" }),
    );
    let mut seen = Vec::new();
    for t in [1000, 500] {
        let sql = tile_sql(
            &c,
            values(&defs, &json!({ "target": t })),
            vec![ratio.clone()],
        )
        .unwrap();
        assert!(sql.contains(&format!("nullIf({t}, 0)")), "{sql}");
        rewrite(
            &format!("target {t}"),
            &sql,
            &points("row-filter", &["place"]),
        );
        rewrite(
            &format!("target {t}"),
            &sql,
            &points("mask-visitors", &["visitors"]),
        );
        seen.push(sql);
    }
    assert_ne!(seen[0], seen[1]);
}

/// Hostile values of a text parameter used in a formula: the statement keeps
/// them inside one escaped literal, every read stays governed.
#[test]
fn a_hostile_text_value_stays_inside_one_literal_under_the_rewrite() {
    let defs = [def("who", ParamType::Text, json!("x"), &json!({}))];
    let tag = field(POINTS, "tag", "Concat({who}, provinsi)", "text");
    let c = chart(
        POINTS,
        &json!({ "dimension": "tag", "measures": ["visitors"] }),
    );
    for hostile in [
        "O'Brien",
        r"back\slash",
        "x') OR 1=1 --",
        "'; DROP TABLE t; --",
        "`backtick`",
        "a;b /* c */",
        "\u{65e5}\u{672c}",
    ] {
        let sql = tile_sql(
            &c,
            values(&defs, &json!({ "who": hostile })),
            vec![tag.clone()],
        )
        .unwrap();
        let literal = lakehouse_core::ident::SqlLiteral::from(hostile).to_string();
        assert!(sql.contains(&literal), "{hostile}: {sql}");
        assert_eq!(
            sql.matches(&literal).count(),
            1,
            "the value appears once, as that literal: {sql}"
        );
        rewrite(
            &format!("who {hostile}"),
            &sql,
            &points("row-filter", &["place"]),
        );
    }
    // A number or a date that is not one never gets as far as a statement.
    let numeric = [def("n", ParamType::Number, json!(1), &json!({}))];
    for bad in ["1; DROP TABLE t", "1 OR 1=1", "0x10", "1e999", "NaN", "--1"] {
        assert!(
            ParamValues::resolve(&numeric, &[], json!({ "n": bad }).as_object()).is_err(),
            "{bad}"
        );
    }
}

/// `BI-11-AC2`: a list parameter switching a measure between the two numeric
/// columns of the capacity table; a switch to a masked column shows the mask
/// (the engine then refuses the numeric aggregate, as for a plain chart).
#[test]
fn a_column_switch_reads_only_the_listed_column_and_stays_governed() {
    let defs = [list("pick", &["hotels", "rooms"], "hotels")];
    let c = chart(
        CAPACITY,
        &json!({ "measures": [{ "column": { "param": "pick" } }] }),
    );
    for choice in ["hotels", "rooms"] {
        let sql = tile_sql(&c, values(&defs, &json!({ "pick": choice })), vec![]).unwrap();
        assert!(
            sql.contains(&format!("sum({choice})")) || sql.contains(&format!("`{choice}`")),
            "{sql}"
        );
        rewrite(
            &format!("pick {choice}"),
            &sql,
            &capacity("row-filter", &[]),
        );
        rewrite(
            &format!("pick {choice}"),
            &sql,
            &capacity("mask-rooms", &["rooms"]),
        );
    }
    // A value that is not in the list is refused when the request is read.
    for outside in [
        "lat",
        "serving.mart_demo_map_points",
        "hotels; DROP TABLE x",
        "provinsi",
    ] {
        assert!(
            ParamValues::resolve(&defs, &[], json!({ "pick": outside }).as_object()).is_err(),
            "{outside}"
        );
    }
}

/// In the list but not a column of the source, or not an identifier: the tile's
/// sentence, never a statement.
#[test]
fn a_listed_value_that_is_not_a_column_of_the_source_is_never_written() {
    let defs = [list(
        "pick",
        &["hotels", "ghost", "serving.mart_demo_map_points", "a b"],
        "hotels",
    )];
    let c = chart(
        CAPACITY,
        &json!({ "measures": [{ "column": { "param": "pick" } }] }),
    );
    for choice in ["ghost", "serving.mart_demo_map_points", "a b"] {
        let sentence = tile_sql(&c, values(&defs, &json!({ "pick": choice })), vec![]).unwrap_err();
        assert!(sentence.contains("not a column"), "{choice}: {sentence}");
    }
    let d = chart(
        CAPACITY,
        &json!({ "dimension": { "param": "pick" }, "measures": ["hotels"] }),
    );
    assert!(tile_sql(&d, values(&defs, &json!({ "pick": "ghost" })), vec![]).is_err());
    // The same chart with a column that is there.
    assert!(tile_sql(&d, values(&defs, &json!({ "pick": "hotels" })), vec![]).is_ok());
}

/// The number of rows from a number parameter: a whole number in range is the
/// `LIMIT`; anything else is the tile's sentence.
#[test]
fn a_limit_parameter_is_the_limit_or_a_plain_sentence() {
    let defs = [def(
        "top",
        ParamType::Number,
        json!(5),
        &json!({ "min": 0, "max": 500 }),
    )];
    let c = chart(POINTS, &json!({ "limit": { "param": "top" } }));
    let sql = tile_sql(&c, values(&defs, &json!({ "top": 7 })), vec![]).unwrap();
    assert!(sql.ends_with("LIMIT 7"), "{sql}");
    rewrite("top 7", &sql, &points("row-filter", &["place"]));
    for bad in [json!(0), json!(101), json!(2.5)] {
        let sentence = tile_sql(&c, values(&defs, &json!({ "top": bad })), vec![]).unwrap_err();
        assert!(
            sentence.contains("whole number from 1 to 100"),
            "{sentence}"
        );
    }
}

/// Charts that name no parameter produce the statement they always did.
#[test]
fn a_chart_without_a_parameter_builds_the_same_statement_with_or_without_values() {
    let defs = [def("target", ParamType::Number, json!(1000), &json!({}))];
    let c = chart(POINTS, &json!({}));
    let with = tile_sql(&c, values(&defs, &json!({})), vec![]).unwrap();
    let without = tile_sql(&c, ParamValues::default(), vec![]).unwrap();
    assert_eq!(with, without);
}
