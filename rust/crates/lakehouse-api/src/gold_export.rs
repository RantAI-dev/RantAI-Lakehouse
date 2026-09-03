//! ADR 0010 — Gold export to Iceberg, from Rust.
//!
//! Reads a Gold mart from a `ClickHouse` `MergeTree` table
//! (`lakehouse-clickhouse::ChClient`) and appends it to an Iceberg table in
//! the `gold` namespace through Lakekeeper (`lakehouse-iceberg`), using the
//! exact write path G1(a) proved end to end: `iceberg-rust` +
//! `iceberg-catalog-rest`, vended credentials, format-version 2.
//!
//! This module owns exactly the glue G1's own crate boundary left out on
//! purpose (`lakehouse-iceberg`'s doc comment: "Console/route wiring" is
//! not that crate's job) — `ClickHouse`'s `FORMAT JSON` row/column shape
//! in, an Arrow `RecordBatch` matching a Gold Iceberg schema out. Nothing
//! here is Iceberg- or `ClickHouse`-specific in a way that couldn't move
//! into either crate later; it stays here because it is the one piece that
//! genuinely needs both.
//!
//! # Why a fresh `RecordBatch` schema every export, not a stored one
//!
//! A Gold mart's column set is discovered from `ClickHouse`'s `FORMAT
//! JSON` response (`meta`), not declared anywhere in this codebase ahead
//! of time — the same "no hand-authored schema" stance `lakehouse-iceberg`
//! takes internally (`BronzeTable::schema`'s doc comment: derive the Arrow
//! schema from the Iceberg one, never hand-write it). [`ch_type_to_field`]
//! is the one place a `ClickHouse` type name is translated to an Iceberg
//! [`iceberg::spec::Type`]; unsupported types fail the export loudly
//! rather than silently dropping or mis-typing a column.

use std::sync::Arc;

use arrow_array::{
    ArrayRef, BooleanArray, Date32Array, Float64Array, Int64Array, RecordBatch, StringArray,
    TimestampMicrosecondArray,
};
use iceberg::spec::{NestedField, PrimitiveType, Type};
use lakehouse_clickhouse::{ChClient, ChColumn, ChError};
use lakehouse_core::secret::SecretValue;
use lakehouse_iceberg::{GoldTable, IcebergClient, IcebergClientConfig, IcebergError, gold};
use serde_json::{Map, Value};
use thiserror::Error;
use time::Date;
use time::format_description::well_known::Iso8601;

/// Errors exporting a Gold mart to Iceberg.
#[derive(Debug, Error)]
pub enum GoldExportError {
    /// Reading the source mart from `ClickHouse` failed.
    #[error("reading Gold mart from ClickHouse failed: {0}")]
    ClickHouse(#[from] ChError),
    /// A `ClickHouse` column type has no supported Iceberg/Arrow mapping —
    /// see [`ch_type_to_field`].
    #[error("column {column:?} has unsupported ClickHouse type {ch_type:?}: {reason}")]
    UnsupportedColumn {
        /// The offending column's name.
        column: String,
        /// The offending column's `ClickHouse` type string, verbatim.
        ch_type: String,
        /// Why the mapping failed.
        reason: String,
    },
    /// Building the Arrow batch from `ClickHouse` row data failed (a value
    /// did not parse as its declared column type).
    #[error("failed to build export batch: {0}")]
    Batch(String),
    /// The Iceberg catalog/write path failed.
    #[error("Iceberg export failed: {0}")]
    Iceberg(#[from] IcebergError),
}

/// One column's resolved Iceberg + Arrow shape, plus enough to parse a
/// `ClickHouse` JSON cell into the right Arrow array.
struct ColumnPlan {
    name: String,
    field_id: i32,
    primitive: PrimitiveType,
    nullable: bool,
}

/// Strips a `ClickHouse` `Nullable(X)` wrapper, returning `(inner_type,
/// nullable)`.
fn unwrap_nullable(ch_type: &str) -> (&str, bool) {
    ch_type
        .strip_prefix("Nullable(")
        .and_then(|rest| rest.strip_suffix(')'))
        .map_or((ch_type, false), |inner| (inner, true))
}

/// Maps one `ClickHouse` column to its Iceberg [`PrimitiveType`].
///
/// Deliberately narrow: only the types a `serving.*` aggregate mart
/// realistically carries (strings, integers, floats, booleans, dates,
/// timestamps). Anything else — `Array(...)`, `Map(...)`, `Tuple(...)`,
/// `AggregateFunction(...)`, `UUID`, `Decimal`, ... — is rejected with
/// [`GoldExportError::UnsupportedColumn`] rather than silently coerced to
/// a string, so a mart with an unsupported column fails the export
/// visibly instead of exporting corrupted data.
///
/// # Errors
///
/// Returns an error message (wrapped by the caller into
/// [`GoldExportError::UnsupportedColumn`]) for any type not in the list
/// above.
fn ch_type_to_primitive(ch_type: &str) -> Result<(PrimitiveType, bool), String> {
    let (base, nullable) = unwrap_nullable(ch_type);
    let primitive = match base {
        "String" | "UUID" => PrimitiveType::String,
        s if s.starts_with("FixedString(") => PrimitiveType::String,
        "Int8" | "Int16" | "Int32" | "Int64" | "UInt8" | "UInt16" | "UInt32" | "UInt64" => {
            PrimitiveType::Long
        }
        "Float32" | "Float64" => PrimitiveType::Double,
        "Bool" | "Boolean" => PrimitiveType::Boolean,
        "Date" | "Date32" => PrimitiveType::Date,
        "DateTime" => PrimitiveType::Timestamp,
        s if s.starts_with("DateTime64(") || s.starts_with("DateTime(") => PrimitiveType::Timestamp,
        other => return Err(format!("no Iceberg mapping for ClickHouse type {other:?}")),
    };
    Ok((primitive, nullable))
}

/// Builds the per-column [`ColumnPlan`]s for `columns`, field ids starting
/// at [`gold::FIRST_DOMAIN_FIELD_ID`] in `ClickHouse`'s own column order.
///
/// # Errors
///
/// Returns [`GoldExportError::UnsupportedColumn`] for the first column
/// whose type has no mapping.
fn plan_columns(columns: &[ChColumn]) -> Result<Vec<ColumnPlan>, GoldExportError> {
    columns
        .iter()
        .enumerate()
        .map(|(i, col)| {
            let (primitive, nullable) = ch_type_to_primitive(&col.ty).map_err(|reason| {
                GoldExportError::UnsupportedColumn {
                    column: col.name.clone(),
                    ch_type: col.ty.clone(),
                    reason,
                }
            })?;
            Ok(ColumnPlan {
                name: col.name.clone(),
                #[allow(
                    clippy::cast_possible_truncation,
                    clippy::cast_possible_wrap,
                    reason = "a Gold mart has nowhere near i32::MAX columns"
                )]
                field_id: gold::FIRST_DOMAIN_FIELD_ID + i as i32,
                primitive,
                nullable,
            })
        })
        .collect()
}

/// Builds the Iceberg [`NestedField`]s for [`gold::gold_schema`] from a
/// resolved column plan.
fn plan_to_iceberg_fields(plan: &[ColumnPlan]) -> Vec<NestedField> {
    plan.iter()
        .map(|c| {
            let ty = Type::Primitive(c.primitive.clone());
            if c.nullable {
                NestedField::optional(c.field_id, &c.name, ty)
            } else {
                NestedField::required(c.field_id, &c.name, ty)
            }
        })
        .collect()
}

fn cell_as_str<'a>(row: &'a Map<String, Value>, name: &str) -> Option<&'a str> {
    match row.get(name) {
        Some(Value::String(s)) => Some(s.as_str()),
        _ => None,
    }
}

fn cell_as_i64(row: &Map<String, Value>, name: &str) -> Result<Option<i64>, GoldExportError> {
    match row.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(n)) => n
            .as_i64()
            .map(Some)
            .ok_or_else(|| GoldExportError::Batch(format!("{name}: {n} does not fit in i64"))),
        Some(Value::String(s)) => s.parse::<i64>().map(Some).map_err(|e| {
            GoldExportError::Batch(format!("{name}: {s:?} is not a valid integer: {e}"))
        }),
        Some(other) => Err(GoldExportError::Batch(format!(
            "{name}: expected an integer, got {other}"
        ))),
    }
}

fn cell_as_f64(row: &Map<String, Value>, name: &str) -> Result<Option<f64>, GoldExportError> {
    match row.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(n)) => Ok(n.as_f64()),
        Some(Value::String(s)) => s.parse::<f64>().map(Some).map_err(|e| {
            GoldExportError::Batch(format!("{name}: {s:?} is not a valid float: {e}"))
        }),
        Some(other) => Err(GoldExportError::Batch(format!(
            "{name}: expected a float, got {other}"
        ))),
    }
}

fn cell_as_bool(row: &Map<String, Value>, name: &str) -> Result<Option<bool>, GoldExportError> {
    match row.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(b)) => Ok(Some(*b)),
        Some(Value::Number(n)) => Ok(Some(n.as_i64().unwrap_or(0) != 0)),
        Some(Value::String(s)) => Ok(Some(s == "1" || s.eq_ignore_ascii_case("true"))),
        Some(other) => Err(GoldExportError::Batch(format!(
            "{name}: expected a boolean, got {other}"
        ))),
    }
}

/// Days since the Unix epoch for a `ClickHouse` `Date`/`Date32` string
/// (`"YYYY-MM-DD"`).
fn cell_as_date_days(row: &Map<String, Value>, name: &str) -> Result<Option<i32>, GoldExportError> {
    let Some(raw) = cell_as_str(row, name) else {
        return Ok(None);
    };
    let format =
        time::macros::format_description!("[year]-[month padding:zero]-[day padding:zero]");
    let date = Date::parse(raw, &format)
        .map_err(|e| GoldExportError::Batch(format!("{name}: {raw:?} is not a valid Date: {e}")))?;
    let epoch = Date::from_ordinal_date(1970, 1)
        .map_err(|e| GoldExportError::Batch(format!("internal epoch date error: {e}")))?;
    Ok(Some((date - epoch).whole_days().try_into().map_err(
        |e| GoldExportError::Batch(format!("{name}: date out of i32 range: {e}")),
    )?))
}

/// Microseconds since the Unix epoch for a `ClickHouse`
/// `DateTime`/`DateTime64` string (`"YYYY-MM-DD HH:MM:SS[.ffffff]"`).
fn cell_as_timestamp_micros(
    row: &Map<String, Value>,
    name: &str,
) -> Result<Option<i64>, GoldExportError> {
    let Some(raw) = cell_as_str(row, name) else {
        return Ok(None);
    };
    // Accept both the plain ClickHouse rendering ("2024-01-02 03:04:05" /
    // "...05.123456") and a full ISO 8601 timestamp, since either can
    // appear depending on the source table's exact type and ClickHouse
    // version.
    let normalized = raw.replacen(' ', "T", 1);
    let with_offset = if normalized.contains('Z') || normalized.contains('+') {
        normalized
    } else {
        format!("{normalized}Z")
    };
    let odt = time::OffsetDateTime::parse(&with_offset, &Iso8601::DEFAULT).map_err(|e| {
        GoldExportError::Batch(format!("{name}: {raw:?} is not a valid DateTime: {e}"))
    })?;
    Ok(Some(
        odt.unix_timestamp() * 1_000_000 + i64::from(odt.microsecond()),
    ))
}

/// Builds one Arrow [`ArrayRef`] for `plan` from every row in `rows`.
fn build_array(
    plan: &ColumnPlan,
    rows: &[Map<String, Value>],
) -> Result<ArrayRef, GoldExportError> {
    Ok(match plan.primitive {
        PrimitiveType::String => Arc::new(StringArray::from(
            rows.iter()
                .map(|r| cell_as_str(r, &plan.name).map(str::to_owned))
                .collect::<Vec<_>>(),
        )),
        PrimitiveType::Long => {
            let values = rows
                .iter()
                .map(|r| cell_as_i64(r, &plan.name))
                .collect::<Result<Vec<_>, _>>()?;
            Arc::new(Int64Array::from(values))
        }
        PrimitiveType::Double => {
            let values = rows
                .iter()
                .map(|r| cell_as_f64(r, &plan.name))
                .collect::<Result<Vec<_>, _>>()?;
            Arc::new(Float64Array::from(values))
        }
        PrimitiveType::Boolean => {
            let values = rows
                .iter()
                .map(|r| cell_as_bool(r, &plan.name))
                .collect::<Result<Vec<_>, _>>()?;
            Arc::new(BooleanArray::from(values))
        }
        PrimitiveType::Date => {
            let values = rows
                .iter()
                .map(|r| cell_as_date_days(r, &plan.name))
                .collect::<Result<Vec<_>, _>>()?;
            Arc::new(Date32Array::from(values))
        }
        PrimitiveType::Timestamp => {
            let values = rows
                .iter()
                .map(|r| cell_as_timestamp_micros(r, &plan.name))
                .collect::<Result<Vec<_>, _>>()?;
            Arc::new(TimestampMicrosecondArray::from(values))
        }
        ref other => {
            return Err(GoldExportError::Batch(format!(
                "internal error: no array builder for {other:?}"
            )));
        }
    })
}

/// The result of one export run.
pub struct GoldExportResult {
    /// The Gold namespace exported into (always [`gold::GOLD_NAMESPACE`]).
    pub namespace: String,
    /// The sanitized Iceberg table name exported into.
    pub table: String,
    /// The Iceberg format version of the target table (always 2).
    pub format_version: u8,
    /// How many rows were read from `ClickHouse` and appended. `0` means
    /// the source query returned no rows — no snapshot is committed in
    /// that case (see [`export_mart`]'s doc comment).
    pub rows_exported: usize,
}

/// Reads every row of `source_table` (a fully-qualified `ClickHouse`
/// table name, e.g. `"serving.sales_by_region"`) and appends it to the
/// Gold Iceberg table named `mart_name` (ADR 0010).
///
/// If the source query returns zero rows, this is a no-op: no namespace
/// or table is created, no snapshot is committed. `iceberg-rust`'s
/// partition-key derivation needs at least one non-null `_exported_at`
/// value (see `gold::partition_key_for`'s doc comment), and there is
/// nothing useful to export from an empty mart anyway — a caller that
/// wants the table to exist ahead of the mart having data should call
/// `IcebergClient::create_gold_table` directly.
///
/// # Errors
///
/// See [`GoldExportError`]'s variants.
pub async fn export_mart(
    ch: &ChClient,
    iceberg_config: &IcebergClientConfig,
    source_table: &str,
    mart_name: &str,
) -> Result<GoldExportResult, GoldExportError> {
    let result = ch
        .query(&format!("SELECT * FROM {source_table} FORMAT JSON"), None)
        .await?;

    let namespace = gold::GOLD_NAMESPACE.to_owned();
    let sanitized_table =
        gold::sanitize_mart_name(mart_name).map_err(|e| GoldExportError::Batch(e.to_string()))?;

    if result.data.is_empty() {
        return Ok(GoldExportResult {
            namespace,
            table: sanitized_table,
            format_version: 2,
            rows_exported: 0,
        });
    }

    let plan = plan_columns(&result.meta)?;
    let domain_fields = plan_to_iceberg_fields(&plan);

    let client = IcebergClient::connect(iceberg_config).await?;
    let mut table = client
        .create_or_load_gold_table(mart_name, domain_fields)
        .await?;

    let arrow_schema = iceberg::arrow::schema_to_arrow_schema(table.schema())
        .map_err(|e| GoldExportError::Batch(format!("schema_to_arrow_schema failed: {e}")))?;

    let mut arrays: Vec<ArrayRef> = Vec::with_capacity(plan.len() + 1);
    // `_exported_at` is the same instant for every row in this batch —
    // matches the "one partition value per batch" assumption
    // `gold::partition_key_for` documents.
    let now_micros = time::OffsetDateTime::now_utc().unix_timestamp() * 1_000_000;
    arrays.push(Arc::new(TimestampMicrosecondArray::from(vec![
        now_micros;
        result
            .data
            .len()
    ])));
    for column in &plan {
        arrays.push(build_array(column, &result.data)?);
    }

    let batch = RecordBatch::try_new(Arc::new(arrow_schema), arrays)
        .map_err(|e| GoldExportError::Batch(format!("RecordBatch::try_new failed: {e}")))?;

    let rows_exported = result.data.len();
    table.append(client.as_catalog(), batch).await?;

    Ok(GoldExportResult {
        namespace,
        table: sanitized_table,
        format_version: 2,
        rows_exported,
    })
}

/// Loads the Gold table named `mart_name` and reads back every row
/// currently visible, returning just the row count — used by
/// `routes::gold::read_back` to prove the export round trip without
/// exposing the Gold data itself over the API.
///
/// # Errors
///
/// Returns [`GoldExportError::Iceberg`] if the table does not exist or the
/// read fails.
pub async fn read_back_row_count(
    iceberg_config: &IcebergClientConfig,
    mart_name: &str,
) -> Result<(u8, usize), GoldExportError> {
    let client = IcebergClient::connect(iceberg_config).await?;
    let table: GoldTable = client.load_gold_table(mart_name).await?;
    let format_version = table.format_version() as u8;
    let batches = table.read_all().await?;
    let rows = batches.iter().map(RecordBatch::num_rows).sum();
    Ok((format_version, rows))
}

/// Builds an [`IcebergClientConfig`] for the Gold export path from
/// resolved config + secret values.
#[must_use]
pub fn iceberg_config(
    catalog_uri: String,
    warehouse: String,
    catalog_token: Option<SecretValue>,
) -> IcebergClientConfig {
    IcebergClientConfig {
        catalog_uri,
        warehouse,
        catalog_credential: None,
        catalog_token,
    }
}
