//! Column types on upload (`DATA-18`, plan AD3): what the preview offers per
//! column, the check of a person's choices before Load, and what a load is told.
//! The rules themselves are `crate::upload_types`.
//!
//! # The three places a column list matters
//!
//! - **Preview** (`GET /api/uploads/{id}/preview`): every column comes back with
//!   a name (the load's own cleaning, so a person who edits nothing gets what
//!   loading gave before), a starting type (the guess over the first
//!   [`SAMPLE_ROWS`] rows or [`SAMPLE_BYTES`], or the file's own type for Parquet,
//!   decision 10) and, for a date that could be read either way, a question.
//!   With `?table=` naming a table of the caller's tenant, the names and types are
//!   the table's: adding to it cannot change them (decision 9).
//! - **Check** (`POST /api/uploads/{id}/check`): the list a person has so far,
//!   applied to the sample: how many values do not fit and the first 100. Nothing
//!   is stored.
//! - **Ingest**: the list is validated against the file's width and, when adding
//!   to a table that exists, against the table; then it is sent to the job
//!   (`run_config`) and kept on the upload row for "Try again", the Review step
//!   and the run view.
//!
//! A load with no list is every column text, as before (decision 11). The one
//! exception is adding to a table that holds typed columns: the text would not fit
//! them, so the request is refused with the table's columns in the sentence's
//! place, rather than loaded and failed.

use axum::body::Bytes;
use axum::extract::{Path, State};
use lakehouse_core::ApiError;
use lakehouse_store::uploads::{self, LoadMode, TableClaim, Upload};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::super::catalog_source::iceberg_source;
use super::{Conversion, IngestRequest, PreviewQuery, not_found, pool, preview_with_sample};
use crate::error::ApiResult;
use crate::json::ApiJson;
use crate::state::AppState;
use crate::upload_types::{
    BadValues, ColumnSpec, ColumnType, Declared, check_rows, clean_names, declared_from_clickhouse,
    guess_column, validate_specs,
};

pub(super) use crate::upload_types::{SAMPLE_BYTES, SAMPLE_ROWS};

const COLUMNS_FIELD: &str =
    "columns must be a list with one entry per column: {source, name, type, dateFormat, timezone}.";
const ON_BAD_RULE: &str = "onBadValue must be null or stop, and is required with a column list.";
const ON_BAD_WITHOUT_COLUMNS: &str = "onBadValue is only for a request that sends a column list.";
const TYPED_TABLE: &str = "That table holds typed columns, so a file added to it needs the column list that matches it. Open the file in the console, which fills it in.";

/// The data a preview or a check works on: the header, the sample rows, and the
/// types the file declares for its columns, when it does (Parquet).
#[derive(Debug, Clone)]
pub(super) struct Sample {
    pub(super) header: Vec<String>,
    pub(super) rows: Vec<Vec<String>>,
    pub(super) declared: Option<Vec<Declared>>,
}

/// One column in a preview, as the console shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpecView {
    source: String,
    name: String,
    #[serde(rename = "type")]
    column_type: ColumnType,
    /// What the guess (or the file's own type) said, so the console can show a
    /// change from it.
    guessed_type: ColumnType,
    #[serde(skip_serializing_if = "Option::is_none")]
    date_format: Option<crate::upload_types::DateFormat>,
    #[serde(skip_serializing_if = "Option::is_none")]
    timezone: Option<bool>,
    /// Every sampled value could be read day first or month first.
    ask_date_order: bool,
}

/// What a table holds, as the types of its columns, without the system columns.
pub(super) type TableColumns = Vec<(String, Declared)>;

/// The columns of the Bronze table `table`, when it exists. Names starting with
/// `_` are the loader's (`_ingested_at`, `_dlt_*`) and are left out.
pub(super) async fn table_columns(state: &AppState, table: &str) -> Option<TableColumns> {
    let source = iceberg_source(state, table).await?;
    Some(
        source
            .columns
            .iter()
            .filter(|(name, _)| !name.starts_with('_'))
            .map(|(name, type_name)| (name.clone(), declared_from_clickhouse(type_name)))
            .collect(),
    )
}

/// The table of this tenant a preview or an ingest may take columns from: the
/// caller's own claim only (another tenant's table must not be readable through
/// this route, and the answer does not say which it is).
pub(super) async fn own_table_columns(
    state: &AppState,
    tenant_id: uuid::Uuid,
    table: &str,
) -> Option<TableColumns> {
    let pool = state.pg.as_deref()?;
    if super::table_name_problem(table).is_some() {
        return None;
    }
    match uploads::table_claim(pool, tenant_id, table).await {
        Ok(TableClaim::Ours) => table_columns(state, table).await,
        _ => None,
    }
}

/// The columns of a preview: names from the load's own cleaning, types from the
/// guess, the file's declared types, or the table.
pub(super) fn spec_views(sample: &Sample, table: Option<&TableColumns>) -> Vec<SpecView> {
    let names = clean_names(&sample.header);
    names
        .into_iter()
        .enumerate()
        .map(|(index, name)| {
            let values: Vec<&str> = sample
                .rows
                .iter()
                .map(|record| record.get(index).map_or("", String::as_str))
                .collect();
            let source = sample.header.get(index).cloned().unwrap_or_default();
            let guess = guess_column(&values);
            let declared = sample.declared.as_ref().and_then(|d| d.get(index)).copied();
            // The file's own type is the starting guess; a table's type is not a
            // guess at all, it is the table.
            let (guessed_type, mut column_type, mut date_format, mut timezone, ask) = match declared
            {
                Some(d) => (
                    d.column_type,
                    d.column_type,
                    d.date_format,
                    d.timezone,
                    false,
                ),
                None => (
                    guess.column_type,
                    guess.column_type,
                    guess.date_format,
                    guess.timezone,
                    guess.ask_date_order,
                ),
            };
            let mut name = name;
            let mut ask = ask;
            if let Some((table_name, held)) = table.and_then(|t| t.get(index)) {
                name.clone_from(table_name);
                column_type = held.column_type;
                timezone = held.timezone;
                date_format = match (held.column_type, guess.date_format) {
                    (ColumnType::Date | ColumnType::Timestamp, Some(read)) => Some(read),
                    _ => held.date_format,
                };
                ask = held.column_type == ColumnType::Date && guess.ask_date_order;
            }
            SpecView {
                source,
                name,
                column_type,
                guessed_type,
                date_format,
                timezone,
                ask_date_order: ask,
            }
        })
        .collect()
}

// ── the check route ──────────────────────────────────────────────────────

/// `POST /api/uploads/{id}/check`'s body: the reading in force and the column
/// list so far.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CheckRequest {
    encoding: Option<String>,
    delimiter: Option<String>,
    header_row: Option<u64>,
    sheet: Option<String>,
    columns: Vec<ColumnSpec>,
}

/// `POST /api/uploads/{id}/check`'s response.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckResponse {
    /// How many rows were read.
    sampled_rows: usize,
    /// The values that do not fit: all of them counted, the first 100 listed.
    #[serde(flatten)]
    bad: BadValues,
}

/// `POST /api/uploads/{id}/check` — apply a column list to the sample (the first
/// 10,000 rows) and report the values that do not fit. Stores nothing.
///
/// # Errors
///
/// 400 for a body that is not that object, a reading that is not valid, or a
/// column list that breaks the rules (`upload_types::validate_specs`); 409 for an
/// upload still being received; 404 for an unknown upload or another tenant's;
/// 503 when Postgres or the object store is unavailable.
pub async fn check(
    State(state): State<AppState>,
    Path(id): Path<String>,
    body: Bytes,
) -> ApiResult<ApiJson<CheckResponse>> {
    let request: CheckRequest = serde_json::from_slice(&body)
        .map_err(|_err| ApiError::BadRequest(COLUMNS_FIELD.to_owned()))?;
    let row = uploads::get(pool(&state)?, &id)
        .await?
        .ok_or_else(not_found)?;
    let query = PreviewQuery {
        encoding: request.encoding,
        delimiter: request.delimiter,
        header_row: request.header_row.map(|n| n.to_string()),
        sheet: request.sheet,
        table: None,
    };
    let (_, sample) = preview_with_sample(&state, &row, &query).await?;
    validate_specs(&request.columns, sample.header.len()).map_err(ApiError::BadRequest)?;
    Ok(ApiJson(CheckResponse {
        sampled_rows: sample.rows.len(),
        bad: check_rows(&request.columns, &sample.rows),
    }))
}

// ── ingest ───────────────────────────────────────────────────────────────

/// What to do with a value that does not fit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum OnBadValue {
    /// Load it as empty.
    Null,
    /// Fail the load, writing nothing.
    Stop,
}

impl OnBadValue {
    pub(super) const fn as_str(self) -> &'static str {
        match self {
            Self::Null => "null",
            Self::Stop => "stop",
        }
    }
}

/// The column list of an ingest request, parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ColumnPlan {
    pub(super) columns: Vec<ColumnSpec>,
    pub(super) on_bad_value: OnBadValue,
}

/// Read `columns` and `onBadValue` out of an ingest body. Neither: no plan (every
/// column text). One without the other is refused.
///
/// # Errors
///
/// 400 with a fixed sentence.
pub(super) fn parse_column_plan(
    fields: &serde_json::Map<String, Value>,
) -> Result<Option<ColumnPlan>, ApiError> {
    let bad = |message: &str| ApiError::BadRequest(message.to_owned());
    let on_bad = match fields.get("onBadValue") {
        None | Some(Value::Null) => None,
        Some(Value::String(word)) => Some(match word.as_str() {
            "null" => OnBadValue::Null,
            "stop" => OnBadValue::Stop,
            _ => return Err(bad(ON_BAD_RULE)),
        }),
        Some(_) => return Err(bad(ON_BAD_RULE)),
    };
    match (fields.get("columns"), on_bad) {
        (None | Some(Value::Null), None) => Ok(None),
        (None | Some(Value::Null), Some(_)) => Err(bad(ON_BAD_WITHOUT_COLUMNS)),
        (Some(_), None) => Err(bad(ON_BAD_RULE)),
        (Some(list), Some(on_bad_value)) => {
            let columns: Vec<ColumnSpec> =
                serde_json::from_value(list.clone()).map_err(|_err| bad(COLUMNS_FIELD))?;
            Ok(Some(ColumnPlan {
                columns,
                on_bad_value,
            }))
        }
    }
}

/// A fixed sentence listing how a column list differs from a table's columns, or
/// `None` when they are the same names and types in the same order.
fn table_difference(columns: &[ColumnSpec], table: &TableColumns) -> Option<String> {
    let mut differences = Vec::new();
    if columns.len() != table.len() {
        differences.push(format!(
            "the file has {} columns and the table has {}",
            columns.len(),
            table.len()
        ));
    }
    for (spec, (name, held)) in columns.iter().zip(table) {
        if spec.name != *name {
            differences.push(format!("column {name} is named {} in the file", spec.name));
        } else if spec.column_type != held.column_type {
            differences.push(format!(
                "column {name} is {:?} in the table and {:?} in the file",
                held.column_type, spec.column_type
            ));
        }
    }
    (!differences.is_empty()).then(|| {
        let listed: Vec<String> = differences.into_iter().take(10).collect();
        format!(
            "Adding to an existing table keeps its names and types, and this file differs: {}.",
            listed.join("; ")
        )
    })
}

/// Validate what an ingest request asked for about columns, before any claim:
/// the list against the file's width and, when adding to a table that exists,
/// against the table. Returns the plan to send to the job.
///
/// # Errors
///
/// 400 with a fixed sentence (the rules of `validate_specs`, a difference from the
/// table, or a typed table with no list); 503 when the object store is
/// unavailable.
pub(super) async fn plan_columns(
    state: &AppState,
    row: &Upload,
    request: &IngestRequest,
    _conversion: Option<Conversion>,
) -> ApiResult<Option<ColumnPlan>> {
    let appending = request.mode == LoadMode::Append;
    let table = if appending {
        table_columns(state, &request.table).await
    } else {
        None
    };
    let Some(plan) = request.columns.clone() else {
        // No list: every column text. Added to a typed table that would not fit.
        if let Some(held) = &table
            && held.iter().any(|(_, d)| d.column_type != ColumnType::Text)
        {
            return Err(ApiError::BadRequest(TYPED_TABLE.to_owned()).into());
        }
        return Ok(None);
    };
    // The width of the file under the reading the request confirmed.
    let query = PreviewQuery {
        encoding: Some(request.encoding.as_str().to_owned()),
        delimiter: Some(request.delimiter.to_string()),
        header_row: Some(request.header_row.to_string()),
        sheet: request.sheet.clone(),
        table: None,
    };
    let (_, sample) = preview_with_sample(state, row, &query).await?;
    validate_specs(&plan.columns, sample.header.len()).map_err(ApiError::BadRequest)?;
    if let Some(held) = &table
        && let Some(sentence) = table_difference(&plan.columns, held)
    {
        return Err(ApiError::BadRequest(sentence).into());
    }
    Ok(Some(plan))
}

/// The column part of the job's run configuration. A load with no list sends
/// `columns: []`, which the job reads as "every column text, named by its own
/// rules"; every field is present either way (the job's schema is closed).
pub(super) fn run_config_columns(plan: Option<&ColumnPlan>) -> (Value, &'static str) {
    let Some(plan) = plan else {
        return (json!([]), "stop");
    };
    let columns = plan
        .columns
        .iter()
        .map(|spec| {
            json!({
                "source": spec.source,
                "name": spec.name,
                "type": spec.column_type,
                "date_format": spec.date_format,
                "timezone": spec.timezone.unwrap_or(false),
            })
        })
        .collect::<Vec<_>>();
    (Value::Array(columns), plan.on_bad_value.as_str())
}

/// What the claim of a load writes on the upload row: the list (the wire form,
/// `dateFormat` and all) and what to do with a value that does not fit, or
/// nothing for a load with no list.
pub(super) fn claim_fields(plan: Option<&ColumnPlan>) -> (Option<Value>, Option<&'static str>) {
    plan.map_or((None, None), |plan| {
        (
            Some(serde_json::to_value(&plan.columns).unwrap_or(Value::Null)),
            Some(plan.on_bad_value.as_str()),
        )
    })
}
