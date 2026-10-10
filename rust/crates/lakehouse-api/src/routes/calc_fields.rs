//! `GET/POST/PUT/DELETE /api/dashboard/calc-fields`, `POST
//! /api/dashboard/calc-fields/validate`, `GET /api/dashboard/calc-fields/functions`
//! — calculated fields (`BI-8`, with `AI-4`): a named formula on a mart or a
//! dashboard SQL source that a chart picks like a column.
//!
//! # Posture
//!
//! - The formula language, its checker and its SQL are
//!   `lakehouse_bi::formula`; nothing here builds SQL from a formula's text.
//!   These routes never run a statement over data: they read a mart's or a
//!   source's *column list* (schema metadata, like `GET /api/dashboard/fields`)
//!   and write `console.bi_field`.
//! - Reading and validating need `dashboard:read`; creating, changing and
//!   deleting need `dashboard:write`. A field is visible to everyone who can
//!   read the source (feature page, decision 2).
//! - What a field *shows* is decided where a chart is built: the statement it
//!   ends up in goes through the same guard and role rewrite as every tile,
//!   so a formula over a masked column sees the masked value.
//! - A mistake in a formula is the person's own text and gets our sentence
//!   with a character position; a `ClickHouse` failure is classified and its
//!   text never leaves the log (`AGENTS.md` principle 4).

use axum::Extension;
use axum::body::Bytes;
use axum::extract::{Query, State};
use lakehouse_auth::Principal;
use lakehouse_bi::builder::RelationColumns;
use lakehouse_bi::fields::{self, FieldDef, SourceKind};
use lakehouse_bi::formula::FormulaError;
use lakehouse_bi::formula::catalog::CATALOG;
use lakehouse_bi::formula::compile::Level;
use lakehouse_bi::store;
use lakehouse_clickhouse::{ChClient, ChError};
use lakehouse_core::ApiError;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::error::ApiResult;
use crate::json::ApiJson;
use crate::state::AppState;
use crate::upstream_error;

/// Fixed words for a calculated-field read or write that failed (SEC-11).
const CALC_FIELD: upstream_error::Context = upstream_error::Context::new(
    "calculated field",
    "The calculated field could not be read or saved.",
    "ClickHouse is unavailable.",
);

fn classify_ch(failure: &ChError) -> ApiError {
    upstream_error::ch_error(&CALC_FIELD, failure)
}

/// `GET` query: exactly one of `mart` and `source`.
#[derive(Debug, Deserialize)]
pub struct ListQuery {
    #[serde(default)]
    mart: Option<String>,
    #[serde(default)]
    source: Option<String>,
}

/// `DELETE` query.
#[derive(Debug, Deserialize)]
pub struct IdQuery {
    #[serde(default)]
    id: Option<String>,
}

/// `POST` / `PUT` / `validate` body.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldBody {
    /// `PUT`: the field to change.
    #[serde(default)]
    id: Option<String>,
    /// `POST` and `validate`: the mart, or ...
    #[serde(default)]
    mart: Option<String>,
    /// ... the SQL source id.
    #[serde(default)]
    source: Option<String>,
    /// `POST`: the field's name (kept for good once saved).
    #[serde(default)]
    name: String,
    #[serde(default)]
    formula: String,
}

fn parse_body(body: &Bytes) -> Result<FieldBody, ApiError> {
    serde_json::from_slice(body)
        .map_err(|_unparsed| ApiError::BadRequest("body JSON is invalid".to_owned()))
}

/// The source a field belongs to, with its current columns.
struct Resolved {
    kind: SourceKind,
    id: String,
    cols: RelationColumns,
}

async fn resolve(
    ch: &ChClient,
    mart: Option<&str>,
    source: Option<&str>,
) -> Result<Resolved, ApiError> {
    let blank = |s: Option<&str>| {
        s.map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    };
    match (blank(mart), blank(source)) {
        (Some(_), Some(_)) | (None, None) => Err(ApiError::BadRequest(
            "choose either a mart or a SQL source.".to_owned(),
        )),
        (Some(raw), None) => {
            let id = raw.strip_prefix("serving.").unwrap_or(&raw).to_owned();
            lakehouse_core::ident::Ident::new(id.as_str())
                .map_err(|_bad| ApiError::BadRequest(format!("invalid mart name: {raw}")))?;
            let cols = store::validated_mart_columns(ch, &id)
                .await
                .map_err(|failure| crate::routes::dashboard_folders::classify_bi_error(&failure))?;
            Ok(Resolved {
                kind: SourceKind::Mart,
                id,
                cols,
            })
        }
        (None, Some(id)) => {
            let found = lakehouse_bi::sources::get_source(ch, &id)
                .await
                .map_err(|failure| classify_ch(&failure))?
                .ok_or_else(|| ApiError::NotFound(format!("SQL source '{id}' not found.")))?;
            Ok(Resolved {
                kind: SourceKind::SqlSource,
                id: found.id.clone(),
                cols: found.column_kinds(),
            })
        }
    }
}

/// The `{ message, position, length }` object a formula mistake is sent as.
fn problem_json(problem: &FormulaError) -> Value {
    json!({
        "message": problem.message,
        "position": problem.position,
        "length": problem.length,
    })
}

fn author_name(principal: Option<&Extension<Principal>>) -> String {
    principal.map_or_else(String::new, |Extension(p)| p.display_name.clone())
}

/// `GET /api/dashboard/calc-fields?mart=|source=` — the fields of one source.
///
/// # Errors
///
/// 400 unless exactly one of `mart` and `source` is given, 404 for an unknown
/// SQL source, a fixed 422/503 for a database failure.
pub async fn list(
    State(state): State<AppState>,
    Query(q): Query<ListQuery>,
) -> ApiResult<ApiJson<Value>> {
    let ch = &state.clickhouse;
    // A source the caller names must exist, but its columns are not needed to
    // list; the mart branch skips the column lookup the same way.
    let (kind, id) = match (q.mart.as_deref(), q.source.as_deref()) {
        (Some(m), None) if !m.trim().is_empty() => (
            SourceKind::Mart,
            m.trim()
                .strip_prefix("serving.")
                .unwrap_or(m.trim())
                .to_owned(),
        ),
        (None, Some(s)) if !s.trim().is_empty() => (SourceKind::SqlSource, s.trim().to_owned()),
        _ => {
            return Err(
                ApiError::BadRequest("choose either a mart or a SQL source.".to_owned()).into(),
            );
        }
    };
    let found = fields::list_fields_for(ch, kind, &id)
        .await
        .map_err(|failure| classify_ch(&failure))?;
    Ok(ApiJson(json!({
        "sourceKind": kind.as_str(),
        "sourceId": id,
        "fields": found,
    })))
}

/// `GET /api/dashboard/calc-fields/functions` — the function catalog: the one
/// list the console's suggestions and the assistant read.
#[allow(
    clippy::unused_async,
    reason = "an axum handler is an async fn even when it awaits nothing"
)]
pub async fn functions() -> ApiJson<Value> {
    ApiJson(json!({ "functions": CATALOG }))
}

/// `POST /api/dashboard/calc-fields/validate` — check a formula against a
/// source and say what it would be (`level`, `type`), or where it is wrong.
/// Writes nothing and runs nothing.
///
/// # Errors
///
/// 400 for a bad body or source; a mistake in the formula itself is a normal
/// `200` with `ok: false`, because finding it is the route's job.
pub async fn validate(State(state): State<AppState>, body: Bytes) -> ApiResult<ApiJson<Value>> {
    let parsed = parse_body(&body)?;
    let ch = &state.clickhouse;
    let source = resolve(ch, parsed.mart.as_deref(), parsed.source.as_deref()).await?;
    let time = crate::routes::settings::time_context(&state).await?;
    if let Err(too_long) = fields::validate_formula_size(&parsed.formula) {
        return Ok(ApiJson(json!({
            "ok": false,
            "error": { "message": too_long, "position": 0, "length": 1 },
        })));
    }
    let siblings = fields::list_fields_for(ch, source.kind, &source.id)
        .await
        .map_err(|failure| classify_ch(&failure))?;
    let own = parsed.name.trim();
    let own = if lakehouse_core::ident::Ident::new(own).is_ok() {
        own
    } else {
        ""
    };
    Ok(ApiJson(
        match fields::check(&parsed.formula, own, &siblings, &source.cols, &time) {
            Ok((level, ty)) => json!({ "ok": true, "level": level.as_str(), "type": ty.as_str() }),
            Err(problem) => json!({ "ok": false, "error": problem_json(&problem) }),
        },
    ))
}

/// A formula mistake as the sentence a save answers with: ours, with the
/// character position (the editor shows the same through `validate`).
fn unprocessable(problem: &FormulaError) -> ApiError {
    ApiError::Unprocessable(format!("{problem}"))
}

/// `POST /api/dashboard/calc-fields` — save a new field.
///
/// # Errors
///
/// 400 for a bad body, name or source; 409 for a name already used by a field
/// of that source; 422 for a formula that does not check out; a fixed 422/503
/// for a database failure.
pub async fn create(
    State(state): State<AppState>,
    principal: Option<Extension<Principal>>,
    body: Bytes,
) -> ApiResult<ApiJson<Value>> {
    let parsed = parse_body(&body)?;
    let ch = &state.clickhouse;
    let source = resolve(ch, parsed.mart.as_deref(), parsed.source.as_deref()).await?;
    let time = crate::routes::settings::time_context(&state).await?;
    let name = parsed.name.trim().to_owned();
    fields::validate_name(&name, &source.cols).map_err(ApiError::BadRequest)?;
    fields::validate_formula_size(&parsed.formula).map_err(ApiError::BadRequest)?;
    let siblings = fields::list_fields_for(ch, source.kind, &source.id)
        .await
        .map_err(|failure| classify_ch(&failure))?;
    if siblings.iter().any(|f| f.name == name) {
        return Err(ApiError::Conflict(format!(
            "a field called '{name}' already exists on this source."
        ))
        .into());
    }
    if siblings.len() >= fields::MAX_FIELDS_PER_SOURCE {
        return Err(ApiError::BadRequest(format!(
            "a source has at most {} calculated fields.",
            fields::MAX_FIELDS_PER_SOURCE
        ))
        .into());
    }
    let (level, ty) = fields::check(&parsed.formula, &name, &siblings, &source.cols, &time)
        .map_err(|problem| unprocessable(&problem))?;
    let field = FieldDef {
        id: fields::new_field_id(),
        source_kind: source.kind,
        source_id: source.id,
        name,
        formula: parsed.formula,
        level: level.as_str().to_owned(),
        ty: ty.as_str().to_owned(),
        created_by: author_name(principal.as_ref()),
        updated_at: None,
    };
    fields::save_field(ch, &field)
        .await
        .map_err(|failure| classify_ch(&failure))?;
    Ok(ApiJson(json!({ "ok": true, "field": field })))
}

/// The charts and the other fields that use `name`, as the sentence a refusal
/// carries; empty when nothing does.
async fn dependents(
    ch: &ChClient,
    field: &FieldDef,
    source: &Resolved,
    siblings: &[FieldDef],
    time: &lakehouse_bi::grain::TimeContext,
) -> Result<Vec<String>, ApiError> {
    let charts = store::list_stored_charts(ch)
        .await
        .map_err(|failure| classify_ch(&failure))?;
    let boards = store::list_boards(ch)
        .await
        .map_err(|failure| classify_ch(&failure))?;
    let using = fields::charts_using(&charts, field.source_kind, &field.source_id, &field.name);
    let mut out = fields::name_charts(&charts, &boards, &using);
    out.extend(
        fields::fields_using(siblings, &source.cols, time, &field.name)
            .into_iter()
            .map(|n| format!("the field {n}")),
    );
    Ok(out)
}

/// `PUT /api/dashboard/calc-fields` — change a field's formula (`id`,
/// `formula`). The name stays: charts refer to it. A change that would turn a
/// row-level field into an aggregate (or back) while something uses it is
/// refused.
///
/// # Errors
///
/// 400 for a bad body, 404 for an unknown id, 409 for a level change that
/// would break a chart or a field, 422 for a formula that does not check out.
pub async fn update(
    State(state): State<AppState>,
    principal: Option<Extension<Principal>>,
    body: Bytes,
) -> ApiResult<ApiJson<Value>> {
    let parsed = parse_body(&body)?;
    let ch = &state.clickhouse;
    let id = parsed
        .id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ApiError::BadRequest("id is required.".to_owned()))?;
    let current = fields::get_field(ch, id)
        .await
        .map_err(|failure| classify_ch(&failure))?
        .ok_or_else(|| ApiError::NotFound(format!("field '{id}' not found.")))?;
    let source = match current.source_kind {
        SourceKind::Mart => resolve(ch, Some(&current.source_id), None).await?,
        SourceKind::SqlSource => resolve(ch, None, Some(&current.source_id)).await?,
    };
    let time = crate::routes::settings::time_context(&state).await?;
    fields::validate_formula_size(&parsed.formula).map_err(ApiError::BadRequest)?;
    let siblings = fields::list_fields_for(ch, source.kind, &source.id)
        .await
        .map_err(|failure| classify_ch(&failure))?;
    let (level, ty) = fields::check(
        &parsed.formula,
        &current.name,
        &siblings,
        &source.cols,
        &time,
    )
    .map_err(|problem| unprocessable(&problem))?;
    if level != Level::Row && !current.is_aggregate()
        || level == Level::Row && current.is_aggregate()
    {
        let users = dependents(ch, &current, &source, &siblings, &time).await?;
        if !users.is_empty() {
            return Err(ApiError::Conflict(format!(
                "this change turns the field into {} one, but it is used by {}.",
                if level == Level::Row {
                    "a row-level"
                } else {
                    "an aggregate"
                },
                users.join(", ")
            ))
            .into());
        }
    }
    let field = FieldDef {
        formula: parsed.formula,
        level: level.as_str().to_owned(),
        ty: ty.as_str().to_owned(),
        created_by: author_name(principal.as_ref()),
        updated_at: None,
        ..current
    };
    fields::save_field(ch, &field)
        .await
        .map_err(|failure| classify_ch(&failure))?;
    Ok(ApiJson(json!({ "ok": true, "field": field })))
}

/// `DELETE /api/dashboard/calc-fields?id=` — remove a field nothing uses.
///
/// # Errors
///
/// 400 without an id, 404 for an unknown one, 409 (naming them) while a chart
/// or another field uses it.
pub async fn delete(
    State(state): State<AppState>,
    Query(q): Query<IdQuery>,
) -> ApiResult<ApiJson<Value>> {
    let ch = &state.clickhouse;
    let id =
        q.id.as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| ApiError::BadRequest("id is required.".to_owned()))?;
    let current = fields::get_field(ch, id)
        .await
        .map_err(|failure| classify_ch(&failure))?
        .ok_or_else(|| ApiError::NotFound(format!("field '{id}' not found.")))?;
    let source = match current.source_kind {
        SourceKind::Mart => resolve(ch, Some(&current.source_id), None).await,
        SourceKind::SqlSource => resolve(ch, None, Some(&current.source_id)).await,
    };
    // A source that no longer exists leaves nothing for a field to be used on:
    // its columns are not needed, only the charts that name it.
    let source = source.unwrap_or(Resolved {
        kind: current.source_kind,
        id: current.source_id.clone(),
        cols: RelationColumns::default(),
    });
    let time = crate::routes::settings::time_context(&state).await?;
    let siblings = fields::list_fields_for(ch, source.kind, &source.id)
        .await
        .map_err(|failure| classify_ch(&failure))?;
    let users = dependents(ch, &current, &source, &siblings, &time).await?;
    if !users.is_empty() {
        return Err(ApiError::Conflict(format!(
            "the field is still used by {}.",
            users.join(", ")
        ))
        .into());
    }
    fields::delete_field(ch, &current.id)
        .await
        .map_err(|failure| classify_ch(&failure))?;
    Ok(ApiJson(json!({ "ok": true })))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::io::Write as _;

    use lakehouse_bi::builder::{QueryBuilder, Relation, build_kpi_sql};
    use lakehouse_bi::filters::ColumnKind;
    use lakehouse_bi::formula::compile::{Scope, compile};
    use lakehouse_bi::grain::TimeContext;
    use lakehouse_bi::specs::Aggregate;
    use lakehouse_bi::store::ChartInput;
    use lakehouse_core::ident::Ident;
    use sqlparser::dialect::ClickHouseDialect;

    use super::*;
    use crate::sql_rewrite::{
        NoViews, ObligationsSource, PlaceholderValues, RewriteError, TableObligations, enforce,
    };

    /// One governed table, `serving.<table>`, with a fixed obligation set for
    /// every role: the role rewrite as `routes::support::run_spec_sql` sends
    /// a tile through it, minus the Postgres lookup.
    struct OneTable {
        table: &'static str,
        mask: Vec<&'static str>,
        row_filter: Option<&'static str>,
        real_columns: Option<Vec<&'static str>>,
    }

    impl ObligationsSource for OneTable {
        fn obligations_for(&self, table: &str, _roles: &[String]) -> Option<TableObligations> {
            (table == self.table).then(|| TableObligations {
                mask: self.mask.iter().map(|m| (*m).to_owned()).collect(),
                row_filter: self.row_filter.map(str::to_owned),
                real_columns: self
                    .real_columns
                    .as_ref()
                    .map(|c| c.iter().map(|c| (*c).to_owned()).collect()),
            })
        }
        fn has_any_obligation(&self, _roles: &[String]) -> bool {
            true
        }
    }

    fn rewrite(sql: &str, policy: &OneTable) -> Result<String, RewriteError> {
        enforce(
            sql,
            &ClickHouseDialect {},
            &["analyst".to_owned()],
            &PlaceholderValues::none(),
            policy,
            &NoViews,
        )
    }

    // ── the demo table, as the engine reports it ─────────────────────────

    const MART: &str = "mart_demo_map_points";

    fn demo_columns() -> RelationColumns {
        [
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
        .collect()
    }

    const DEMO_REAL_COLUMNS: [&str; 8] = [
        "place",
        "kab_kota",
        "provinsi",
        "lat",
        "lon",
        "visitors",
        "category",
        "visit_date",
    ];

    fn demo_field(name: &str, formula: &str) -> FieldDef {
        let cols = demo_columns();
        let time = TimeContext::default();
        let (level, ty) = fields::check(formula, name, &[], &cols, &time).unwrap();
        FieldDef {
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

    /// The statement a chart of this shape would run, built the way the
    /// builder builds it for a stored chart that names `fields`.
    fn chart_sql(fields: &[FieldDef], chart: &serde_json::Value) -> String {
        let mut def: serde_json::Value = serde_json::json!({ "title": "t", "mart": MART });
        def.as_object_mut()
            .unwrap()
            .extend(chart.as_object().unwrap().clone());
        let def: ChartInput = serde_json::from_value(def).unwrap();
        let time = TimeContext::default();
        let from = Relation::Mart(Ident::new(MART).unwrap());
        let p = fields::prepare(fields, &def, from, &demo_columns(), &time)
            .unwrap()
            .expect("the chart names a field");
        let measure = Ident::new(def.measures[0].as_str()).unwrap();
        if def.kind == lakehouse_bi::specs::ChartKind::Kpi {
            return build_kpi_sql(&p.from, &measure, Aggregate::Sum, &[]);
        }
        QueryBuilder::over(p.from)
            .dimension(Ident::new(def.dimension.as_str()).unwrap())
            .aggregate(Aggregate::Sum)
            .limit(5)
            .measures(vec![measure])
            .build()
    }

    /// With `CALC_REWRITE_OUT=<file>` set, every statement the tests below
    /// rewrite is appended there (label, then the rewritten statement), so
    /// the developer can run them by hand against the real engine.
    fn record(label: &str, rewritten: &str) {
        let Ok(path) = std::env::var("CALC_REWRITE_OUT") else {
            return;
        };
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap();
        writeln!(file, "-- {label}\n{}", rewritten.replace('\n', " ")).unwrap();
    }

    /// The role rewrite replaces the *table* with its masked, filtered
    /// projection, so a formula compiled into a chart's statement reads the
    /// masked value: the raw table name appears only inside that projection.
    #[test]
    fn a_formula_over_a_masked_column_reads_the_masked_projection_and_the_row_filter_holds() {
        let policy = OneTable {
            table: "serving.mart_demo_map_points",
            mask: vec!["place"],
            row_filter: Some("provinsi = 'Bali'"),
            real_columns: Some(DEMO_REAL_COLUMNS.to_vec()),
        };
        let cases = [
            (
                "row-level text over the masked column, as the dimension",
                vec![demo_field("shout", "Upper(place)")],
                serde_json::json!({ "kind": "bar", "dimension": "shout", "measures": ["visitors"] }),
            ),
            (
                "row-level number from the masked column, as the measure",
                vec![demo_field("name_len", "Length(place)")],
                serde_json::json!({ "kind": "bar", "dimension": "category", "measures": ["name_len"] }),
            ),
            (
                "aggregate over the masked column",
                vec![demo_field("places", "CountDistinct(place)")],
                serde_json::json!({ "kind": "bar", "dimension": "category", "measures": ["places"] }),
            ),
            (
                "aggregate mixing a masked and a plain column",
                vec![demo_field(
                    "per_place",
                    "Sum(visitors) / CountDistinct(place)",
                )],
                serde_json::json!({ "kind": "kpi", "measures": ["per_place"] }),
            ),
            (
                "row-level text compared with a clear value",
                vec![demo_field("is_a", "StartsWith(place, 'A')")],
                serde_json::json!({ "kind": "bar", "dimension": "is_a", "measures": ["visitors"] }),
            ),
            (
                "aggregate count over the row-filtered rows",
                vec![demo_field("n", "Count()")],
                serde_json::json!({ "kind": "kpi", "measures": ["n"] }),
            ),
        ];
        for (label, fields, chart) in cases {
            let sql = chart_sql(&fields, &chart);
            let out = rewrite(&sql, &policy).unwrap_or_else(|failure| panic!("{label}: {failure}"));
            record(label, &out);
            // The table is read once, inside the masked projection; nothing
            // outside it names the real table.
            assert_eq!(
                out.matches("serving.mart_demo_map_points").count(),
                sql.matches("serving.mart_demo_map_points").count(),
                "{label}: {out}"
            );
            assert!(
                out.contains("replaceRegexpOne(toString(`place`), '(?s)^.*$', '***') AS `place`"),
                "{label}: {out}"
            );
            assert!(out.contains("WHERE provinsi = 'Bali'"), "{label}: {out}");
            // The masked projection is the innermost read: the calculated
            // expression sits OUTSIDE it.
            let masked_at = out.find("replaceRegexpOne").unwrap();
            let table_at = out.find("serving.mart_demo_map_points").unwrap();
            let expr_at = out.find("place").unwrap();
            assert!(expr_at < table_at && masked_at < table_at, "{label}: {out}");
        }
    }

    #[test]
    fn a_formula_over_a_masked_number_has_nothing_clear_to_compute_with() {
        let policy = OneTable {
            table: "serving.mart_demo_map_points",
            mask: vec!["visitors", "visit_date"],
            row_filter: None,
            real_columns: Some(DEMO_REAL_COLUMNS.to_vec()),
        };
        let cases = [
            (
                "masked number, row-level arithmetic",
                vec![demo_field("double", "visitors * 2")],
                serde_json::json!({ "kind": "bar", "dimension": "category", "measures": ["double"] }),
            ),
            (
                "masked number, aggregate",
                vec![demo_field("total", "Sum(visitors)")],
                serde_json::json!({ "kind": "kpi", "measures": ["total"] }),
            ),
            (
                "masked date, date part",
                vec![demo_field("yr", "Year(visit_date)")],
                serde_json::json!({ "kind": "bar", "dimension": "yr", "measures": ["lat"] }),
            ),
            (
                "plain chart on the masked number, for comparison",
                vec![demo_field("unused", "lat + 1")],
                serde_json::json!({ "kind": "bar", "dimension": "category", "measures": ["visitors", "unused"] }),
            ),
        ];
        for (label, fields, chart) in cases {
            let sql = chart_sql(&fields, &chart);
            let out = rewrite(&sql, &policy).unwrap_or_else(|failure| panic!("{label}: {failure}"));
            record(label, &out);
            assert!(
                out.contains(
                    "replaceRegexpOne(toString(`visitors`), '(?s)^.*$', '***') AS `visitors`"
                ),
                "{label}: {out}"
            );
        }
    }

    /// What the rewrite cannot prove it fails closed on, with a calculated
    /// field in the statement exactly as without one.
    #[test]
    fn an_obligation_the_rewrite_cannot_prove_refuses_the_statement() {
        let field = demo_field("double", "visitors * 2");
        let sql = chart_sql(
            std::slice::from_ref(&field),
            &serde_json::json!({ "kind": "bar", "dimension": "category", "measures": ["double"] }),
        );
        let no_columns = OneTable {
            table: "serving.mart_demo_map_points",
            mask: vec!["visitors"],
            row_filter: None,
            real_columns: None,
        };
        assert!(matches!(
            rewrite(&sql, &no_columns),
            Err(RewriteError::UnprovableSubstitution { .. })
        ));
        let bad_filter = OneTable {
            table: "serving.mart_demo_map_points",
            mask: vec![],
            row_filter: Some("secret_column = 1"),
            real_columns: Some(DEMO_REAL_COLUMNS.to_vec()),
        };
        assert!(matches!(
            rewrite(&sql, &bad_filter),
            Err(RewriteError::InvalidRowFilter { .. })
        ));
    }

    /// A formula cannot introduce a second table, a table function or a
    /// dictionary call: the compiler has no such construct, and the rewrite's
    /// own classifier would refuse it. Checked on every catalog example over
    /// the sample columns: each compiled expression is valid for the rewrite's
    /// parser, adds no table, and survives rewriting under a mask on every
    /// column.
    #[test]
    fn every_catalog_example_parses_for_the_rewrite_and_reads_only_its_own_table() {
        let columns: RelationColumns = [
            ("amount", ColumnKind::Number),
            ("qty", ColumnKind::Number),
            ("label", ColumnKind::Text),
            ("day", ColumnKind::Date),
            ("ts", ColumnKind::DateTime),
        ]
        .into_iter()
        .map(|(n, k)| (n.to_owned(), k))
        .collect();
        let time = TimeContext::default();
        let policy = OneTable {
            table: "serving.t",
            mask: vec!["label"],
            row_filter: Some("qty >= 0"),
            real_columns: Some(vec!["amount", "qty", "label", "day", "ts"]),
        };
        for f in lakehouse_bi::formula::catalog::CATALOG {
            let c = compile(f.example, &Scope::new(&columns, &time), None).unwrap();
            let sql = format!("SELECT {} AS v FROM serving.t", c.sql);
            let out = rewrite(&sql, &policy).unwrap_or_else(|failure| {
                panic!("{}: {failure}\n{sql}", f.name);
            });
            assert_eq!(out.matches("serving.t").count(), 1, "{}: {out}", f.name);
            let tables =
                crate::sql_rewrite::referenced_tables(&out, &ClickHouseDialect {}).unwrap();
            assert_eq!(tables, vec!["serving.t".to_owned()], "{}", f.name);
            // The output is a fixed point: rewriting it again (the way a
            // stored statement is rewritten on every read) changes nothing
            // that matters and still parses.
            assert!(rewrite(&out, &policy).is_ok(), "{}: {out}", f.name);
        }
    }
}
