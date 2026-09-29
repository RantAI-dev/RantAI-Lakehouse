//! `GET /api/catalog/{id}/profile` — per-column profile of a catalog asset:
//! null count, approximate distinct count, min/max for ordered types, and
//! the most frequent values where they can be stated exactly.
//!
//! # What gets profiled
//!
//! Whatever table `catalog_source` resolves for the asset — the same one
//! the detail sample reads: a `silver.*`/`serving.*` table, a Bronze
//! dataset's Silver table, or, failing that, its Iceberg table through the
//! configured `DataLakeCatalog` database. With none of those, the route
//! answers `supported: false` with a reason rather than a guessed-at
//! profile.
//!
//! # Why the profile goes through the policy rewriter
//!
//! A profile is derived from the data (a `max` IS a value; a top value IS
//! a row's content), so it must never reveal more than a `SELECT` the
//! caller could run themselves. The aggregate query is therefore passed
//! through the exact rewrite `POST /api/query/run` uses
//! (`query::rewrite_sql_for_principal`): masked columns are profiled in
//! their masked form and row filters shrink the profiled rows. The route
//! requires `query:read` for the same reason.
//!
//! # Bounded cost
//!
//! The aggregate reads at most [`PROFILE_ROW_LIMIT`] rows and profiles at
//! most [`MAX_PROFILE_COLUMNS`] columns in one statement; the response says
//! when either cap applied.

use axum::Extension;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use lakehouse_auth::Principal;
use lakehouse_core::ApiError;
use lakehouse_core::ident::{Ident, SqlLiteral};
use serde_json::{Map, Value, json};

use crate::error::ApiResult;
use crate::json::ApiJson;
use crate::routes::catalog::{catalog_tenant_refusal, split_db_table};
use crate::routes::catalog_source::{self, ReadSource, SourceKind};
use crate::routes::query::rewrite_sql_for_principal;
use crate::routes::support::str_col;
use crate::state::AppState;

/// The most rows one profile reads. Enough for stable null fractions and
/// cardinality estimates; small enough that opening an asset page never
/// scans a whole large table.
const PROFILE_ROW_LIMIT: u64 = 100_000;

/// The most columns one profile aggregates, so a very wide table still
/// answers in one bounded statement.
const MAX_PROFILE_COLUMNS: usize = 60;

/// How many most-frequent values to report per column.
const TOP_VALUES: u32 = 5;

/// Counters `approx_top_k` keeps. While a column has no more distinct
/// values than this, its counts are exact (`error = 0`); past it the
/// counts are sketch estimates with large error, which this route drops
/// rather than presents as facts (see [`exact_top_values`]).
const TOP_K_RESERVED: u32 = 100;

/// What kind of statistics a column type supports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ColumnKind {
    /// Numbers and dates/times: nulls, distinct, min/max, top values.
    Ordered,
    /// Strings, booleans, enums, UUIDs, IPs: nulls, distinct, top values.
    Categorical,
    /// Arrays, maps, tuples, JSON, …: not aggregated at all.
    Unsupported,
}

/// Strips the `Nullable(…)`/`LowCardinality(…)` wrappers, which change
/// storage but not what statistics make sense.
fn base_type(ty: &str) -> &str {
    let mut t = ty.trim();
    loop {
        let inner = ["Nullable(", "LowCardinality("]
            .iter()
            .find_map(|w| t.strip_prefix(w).and_then(|r| r.strip_suffix(')')));
        match inner {
            Some(i) => t = i.trim(),
            None => return t,
        }
    }
}

fn column_kind(ty: &str) -> ColumnKind {
    let t = base_type(ty);
    let ordered = [
        "Int", "UInt", "Float", "Decimal", "Date", "DateTime", "BFloat16",
    ];
    let categorical = [
        "String",
        "FixedString",
        "Bool",
        "Enum",
        "UUID",
        "IPv4",
        "IPv6",
    ];
    if ordered.iter().any(|p| t.starts_with(p)) {
        ColumnKind::Ordered
    } else if categorical.iter().any(|p| t.starts_with(p)) {
        ColumnKind::Categorical
    } else {
        ColumnKind::Unsupported
    }
}

/// One column the aggregate will cover, at its position `idx` in the
/// statement's aliases (`c{idx}_nulls`, …).
#[derive(Debug, Clone)]
struct PlannedColumn {
    idx: usize,
    name: Ident,
    kind: ColumnKind,
}

/// The single aggregate statement over `from` (a [`ReadSource::from`],
/// already validated). Every column identifier is an [`Ident`], so the
/// interpolation cannot inject.
fn profile_sql(from: &str, columns: &[PlannedColumn]) -> String {
    let mut parts = vec!["count() AS __rows".to_owned()];
    for c in columns {
        let (i, col) = (c.idx, &c.name);
        parts.push(format!("countIf(isNull(`{col}`)) AS c{i}_nulls"));
        parts.push(format!("uniq(`{col}`) AS c{i}_distinct"));
        if c.kind == ColumnKind::Ordered {
            parts.push(format!("toString(min(`{col}`)) AS c{i}_min"));
            parts.push(format!("toString(max(`{col}`)) AS c{i}_max"));
        }
        parts.push(format!(
            "approx_top_k({TOP_VALUES}, {TOP_K_RESERVED})(`{col}`) AS c{i}_top"
        ));
    }
    format!(
        "SELECT {} FROM (SELECT * FROM {from} LIMIT {PROFILE_ROW_LIMIT})",
        parts.join(", ")
    )
}

/// A `ClickHouse` JSON cell as a count. 64-bit integers arrive quoted
/// (`output_format_json_quote_64bit_integers`), smaller ones as numbers.
fn cell_u64(row: &Map<String, Value>, key: &str) -> Option<u64> {
    match row.get(key)? {
        Value::Number(n) => n.as_u64(),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

/// `approx_top_k`'s `[{item, count, error}]`, keeping only the entries
/// whose count is exact. `run`'s string-everything row shape is not used
/// here, so the cell may be a real array or its JSON text.
fn exact_top_values(cell: Option<&Value>) -> Vec<Value> {
    let parsed;
    let items = match cell {
        Some(Value::Array(a)) => a,
        Some(Value::String(s)) => match serde_json::from_str::<Value>(s) {
            Ok(Value::Array(a)) => {
                parsed = a;
                &parsed
            }
            _ => return Vec::new(),
        },
        _ => return Vec::new(),
    };
    items
        .iter()
        .filter(|e| e.get("error").and_then(Value::as_u64) == Some(0))
        .filter_map(|e| {
            let count = e.get("count").and_then(Value::as_u64)?;
            let value = match e.get("item")? {
                Value::String(s) => s.clone(),
                Value::Null => return None,
                other => other.to_string(),
            };
            Some(json!({ "value": value, "count": count }))
        })
        .collect()
}

#[allow(
    clippy::cast_precision_loss,
    reason = "row counts are capped at PROFILE_ROW_LIMIT, far inside f64's exact range"
)]
fn fraction(part: u64, whole: u64) -> Value {
    if whole == 0 {
        Value::Null
    } else {
        json!(part as f64 / whole as f64)
    }
}

/// Turns the aggregate's single result row into the response's `columns`,
/// in the table's own column order, unprofiled columns included.
fn profile_columns(
    all: &[(String, String)],
    planned: &[PlannedColumn],
    row: &Map<String, Value>,
) -> (u64, Vec<Value>) {
    let rows = cell_u64(row, "__rows").unwrap_or(0);
    let columns = all
        .iter()
        .map(|(name, ty)| {
            let Some(c) = planned.iter().find(|c| c.name.as_str() == name) else {
                return json!({ "name": name, "dataType": ty, "profiled": false });
            };
            let i = c.idx;
            let nulls = cell_u64(row, &format!("c{i}_nulls"));
            let text = |k: String| row.get(&k).and_then(Value::as_str).map(str::to_owned);
            json!({
                "name": name,
                "dataType": ty,
                "profiled": true,
                "nullCount": nulls,
                "nullFraction": nulls.map_or(Value::Null, |n| fraction(n, rows)),
                "distinctCount": cell_u64(row, &format!("c{i}_distinct")),
                "min": text(format!("c{i}_min")),
                "max": text(format!("c{i}_max")),
                "topValues": exact_top_values(row.get(&format!("c{i}_top"))),
            })
        })
        .collect();
    (rows, columns)
}

/// Picks the columns to aggregate: supported types with safe names, in
/// table order, capped at [`MAX_PROFILE_COLUMNS`]. Returns whether the cap
/// cut any off.
fn plan_columns(all: &[(String, String)]) -> (Vec<PlannedColumn>, bool) {
    let eligible: Vec<(Ident, ColumnKind)> = all
        .iter()
        .filter_map(|(name, ty)| {
            let kind = column_kind(ty);
            if kind == ColumnKind::Unsupported {
                return None;
            }
            Ident::new(name.as_str()).ok().map(|n| (n, kind))
        })
        .collect();
    let capped = eligible.len() > MAX_PROFILE_COLUMNS;
    let planned = eligible
        .into_iter()
        .take(MAX_PROFILE_COLUMNS)
        .enumerate()
        .map(|(idx, (name, kind))| PlannedColumn { idx, name, kind })
        .collect();
    (planned, capped)
}

fn unsupported(reason: &str) -> Response {
    (
        StatusCode::OK,
        ApiJson(json!({ "supported": false, "reason": reason })),
    )
        .into_response()
}

/// The table behind `id` (see `catalog_source`), or `Ok(None)` when the
/// asset is real but nothing readable backs it.
async fn resolve_source(state: &AppState, id: &str) -> Result<Option<ReadSource>, ApiError> {
    if id.starts_with("silver.") || id.starts_with("serving.") {
        let (db, table) = split_db_table(id);
        return Ok(catalog_source::clickhouse_source(&state.clickhouse, &db, &table).await?);
    }
    let slug = SqlLiteral::from(id);
    let sql = format!(
        "SELECT table_name FROM lake.`bronze_meta.dataset_sync` WHERE slug = {slug}
         UNION ALL SELECT table_name FROM lake.`bronze_meta_sec.dataset_sync` WHERE slug = {slug}
         LIMIT 1"
    );
    let rows = state.clickhouse.rows(&sql, None).await?;
    let Some(row) = rows.first() else {
        return Err(ApiError::NotFound("Asset not found".to_owned()));
    };
    Ok(catalog_source::bronze_source(state, str_col(row, "table_name")).await?)
}

/// `GET /api/catalog/{id}/profile` — see the module doc.
///
/// # Errors
///
/// - 404 when `id` names no catalog asset.
/// - 422 when the policy rewriter refuses the profile statement (the same
///   fixed, non-leaking messages `POST /api/query/run` returns).
/// - 503 when `ClickHouse` is unreachable.
pub async fn profile(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> ApiResult<Response> {
    if let Some(reason) = catalog_tenant_refusal(&state, &principal, &headers).await? {
        return Ok(unsupported(reason));
    }
    let Some(source) = resolve_source(&state, &id).await? else {
        return Ok(unsupported(
            "No readable table backs this asset yet: no ClickHouse table, and no Iceberg \
             table reachable through this deployment's Iceberg query database.",
        ));
    };
    // Underscore-prefixed columns are loader bookkeeping, hidden from the
    // detail schema and sample too.
    let all: Vec<(String, String)> = source
        .columns
        .iter()
        .filter(|(name, _)| !name.starts_with('_'))
        .cloned()
        .collect();
    let (planned, columns_capped) = plan_columns(&all);
    if planned.is_empty() {
        return Ok(unsupported("None of this asset's columns can be profiled."));
    }

    let sql = rewrite_sql_for_principal(
        &state,
        &profile_sql(&source.from, &planned),
        "clickhouse",
        &principal,
    )
    .await?;
    let result = state.clickhouse.query(&sql, None).await?;
    let Some(row) = result.data.first() else {
        return Err(ApiError::Internal("profile returned no row".to_owned()).into());
    };
    let (rows, columns) = profile_columns(&all, &planned, row);

    Ok((
        StatusCode::OK,
        ApiJson(json!({
            "supported": true,
            "source": source.policy_key,
            "sourceKind": match source.kind {
                SourceKind::ClickHouse => "clickhouse",
                SourceKind::Iceberg => "iceberg",
            },
            "rowsProfiled": rows,
            "rowLimit": PROFILE_ROW_LIMIT,
            // Reading exactly the cap means the table may hold more.
            "sampled": rows >= PROFILE_ROW_LIMIT,
            "columnsCapped": columns_capped,
            "columns": columns,
        })),
    )
        .into_response())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn cols(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(n, t)| ((*n).to_owned(), (*t).to_owned()))
            .collect()
    }

    #[test]
    fn base_type_strips_wrappers() {
        assert_eq!(base_type("Nullable(String)"), "String");
        assert_eq!(base_type("LowCardinality(Nullable(String))"), "String");
        assert_eq!(base_type("DateTime64(3, 'UTC')"), "DateTime64(3, 'UTC')");
    }

    #[test]
    fn column_kind_classifies_types() {
        assert_eq!(column_kind("Nullable(Int64)"), ColumnKind::Ordered);
        assert_eq!(column_kind("Decimal(12, 2)"), ColumnKind::Ordered);
        assert_eq!(column_kind("DateTime64(6, 'UTC')"), ColumnKind::Ordered);
        assert_eq!(
            column_kind("LowCardinality(String)"),
            ColumnKind::Categorical
        );
        assert_eq!(column_kind("Array(String)"), ColumnKind::Unsupported);
        assert_eq!(column_kind("Map(String, UInt8)"), ColumnKind::Unsupported);
    }

    #[test]
    fn plan_skips_unsafe_names_and_complex_types() {
        let all = cols(&[
            ("id", "Int64"),
            ("tags", "Array(String)"),
            ("bad name", "String"),
            ("city", "Nullable(String)"),
        ]);
        let (planned, capped) = plan_columns(&all);
        let names: Vec<&str> = planned.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["id", "city"]);
        assert_eq!(planned[1].idx, 1);
        assert!(!capped);
    }

    #[test]
    fn plan_caps_wide_tables() {
        let all: Vec<(String, String)> = (0..MAX_PROFILE_COLUMNS + 5)
            .map(|i| (format!("c{i}"), "Int32".to_owned()))
            .collect();
        let (planned, capped) = plan_columns(&all);
        assert_eq!(planned.len(), MAX_PROFILE_COLUMNS);
        assert!(capped);
    }

    #[test]
    fn profile_sql_reads_a_bounded_subquery() {
        let (planned, _) = plan_columns(&cols(&[("amount", "Float64"), ("city", "String")]));
        let sql = profile_sql("silver.`orders`", &planned);
        assert!(sql.ends_with("FROM (SELECT * FROM silver.`orders` LIMIT 100000)"));
        assert!(sql.contains("toString(min(`amount`)) AS c0_min"));
        // Min/max of free text says nothing useful, so strings get none.
        assert!(!sql.contains("c1_min"));
        assert!(sql.contains("approx_top_k(5, 100)(`city`) AS c1_top"));
    }

    #[test]
    fn top_values_keep_only_exact_counts() {
        let cell =
            json!(r#"[{"item":"a","count":7,"error":0},{"item":"b","count":604,"error":603}]"#);
        assert_eq!(
            exact_top_values(Some(&cell)),
            vec![json!({ "value": "a", "count": 7 })]
        );
        let arr = json!([{ "item": 3, "count": 2, "error": 0 }]);
        assert_eq!(
            exact_top_values(Some(&arr)),
            vec![json!({ "value": "3", "count": 2 })]
        );
        assert!(exact_top_values(None).is_empty());
    }

    #[test]
    fn profile_columns_keeps_table_order_and_marks_skipped() {
        let all = cols(&[("id", "Int64"), ("tags", "Array(String)")]);
        let (planned, _) = plan_columns(&all);
        let row: Map<String, Value> = serde_json::from_value(json!({
            "__rows": "4",
            "c0_nulls": "1",
            "c0_distinct": "3",
            "c0_min": "1",
            "c0_max": "9",
            "c0_top": [],
        }))
        .unwrap();
        let (rows, out) = profile_columns(&all, &planned, &row);
        assert_eq!(rows, 4);
        assert_eq!(out[0]["nullFraction"], json!(0.25));
        assert_eq!(out[0]["max"], json!("9"));
        assert_eq!(
            out[1],
            json!({ "name": "tags", "dataType": "Array(String)", "profiled": false })
        );
    }
}
