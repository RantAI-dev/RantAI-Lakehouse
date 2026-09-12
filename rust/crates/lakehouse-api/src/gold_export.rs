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
//!
//! # Why batches, a row cap, and `toTimeZone(..., 'UTC')` — three code-review fixes
//!
//! [`export_mart`] used to read an entire mart into memory in one
//! `ClickHouse` response, then build one Arrow `RecordBatch` from all of
//! it, then hand that whole batch to the Parquet writer — three full
//! copies of the mart alive at once, with no limit on how big the mart
//! could be. A large mart OOMs the whole `lakehouse-api` process, taking
//! down every other route it serves, not just this export. Three fixes,
//! all in this module:
//!
//! 1. **A hard row cap, checked before any row data is read.**
//!    [`export_mart`] first runs `SELECT count() FROM <mart>` and compares
//!    it against `GOLD_EXPORT_MAX_ROWS`
//!    (`crate::config::Config::gold_export_max_rows`); a mart over the cap
//!    fails with [`GoldExportError::RowCapExceeded`], naming both the mart
//!    and the cap, rather than silently truncating — truncating would
//!    publish a partial mart to Iceberg as though it were complete, which
//!    is a worse failure than refusing outright.
//! 2. **Bounded batches, not one whole-mart materialization.** Rows are
//!    read `GOLD_EXPORT_BATCH_SIZE`
//!    (`crate::config::Config::gold_export_batch_size`) at a time
//!    (`SELECT ... LIMIT n OFFSET n FORMAT JSON`) and appended to Iceberg
//!    per batch (one `GoldTable::append` — one new Parquet file/snapshot —
//!    per batch), so peak memory is bounded by one batch, not the whole
//!    mart. This is `LIMIT`/`OFFSET` pagination over repeated HTTP
//!    requests, not a single streaming response — a true single-pass
//!    streaming rewrite (reading `ClickHouse`'s `FORMAT JSONEachRow` as an
//!    incremental byte stream) would remove the pagination-stability
//!    caveat below, but requires a new streaming response mode on
//!    `lakehouse-clickhouse::ChClient` (which today always buffers a full
//!    response body — see that crate's `query`); this build accepts the
//!    caveat instead of taking on that broader change. Every batch query
//!    also carries an explicit `ORDER BY` over every source column (see
//!    [`export_mart`]'s "why `ORDER BY` every column" note) so that, absent
//!    concurrent writes to the SOURCE mart during the export (a separate
//!    concern from the single-flight guard on concurrent EXPORTS —
//!    `crate::gold_lock` — which only serializes callers of THIS API, not
//!    whatever else may be writing into `ClickHouse`), each `OFFSET` page
//!    reads a stable, non-overlapping slice.
//! 3. **`toTimeZone(col, 'UTC')` instead of blindly appending `Z`.** See
//!    [`select_projection`]'s doc comment.

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
    /// The source mart has more rows than `GOLD_EXPORT_MAX_ROWS` allows.
    /// Refusing outright — rather than exporting the first `max_rows` rows
    /// and calling it done — is deliberate: a silently truncated mart in
    /// Iceberg looks complete to every downstream reader, and there is no
    /// way for them to tell it isn't. See the module doc comment's "Why
    /// batches, a row cap, ..." section.
    #[error(
        "mart {mart:?} has {row_count} rows, exceeding GOLD_EXPORT_MAX_ROWS \
         ({cap}); refusing to export a partial mart silently — raise \
         GOLD_EXPORT_MAX_ROWS or export a smaller mart"
    )]
    RowCapExceeded {
        /// The mart name the cap was checked against.
        mart: String,
        /// The source mart's actual row count (`SELECT count()`).
        row_count: u64,
        /// The configured `GOLD_EXPORT_MAX_ROWS` value.
        cap: u64,
    },
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
///
/// # Why appending `Z` is safe HERE and was not before
///
/// This function still appends a literal `Z` when the string carries no
/// offset of its own — but by the time a value reaches this function, it
/// was read through [`select_projection`]'s `toString(toTimeZone(col,
/// 'UTC'))` wrapper for every `DateTime`/`DateTime64` column (see
/// [`export_mart`]). That is what makes appending `Z` correct: the string
/// genuinely IS UTC, converted by `ClickHouse` itself (which owns the
/// correct IANA timezone database for the column's declared zone, or its
/// server-default zone when the column has none), not merely assumed to
/// be. The prior version of this function appended `Z` directly to
/// whatever `ClickHouse` rendered a `DateTime`/`DateTime64` column as —
/// which, for a column with an explicit (or server-default) non-UTC
/// timezone, is a string in THAT timezone, not UTC — silently shifting
/// every value in the column by the zone's offset. Do not call this
/// function on a raw (non-`toTimeZone`-wrapped) `ClickHouse` `DateTime`
/// string and rely on its `Z`-appending fallback to be correct; it is only
/// correct because [`export_mart`] guarantees the projection upstream.
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

/// Parses a `ClickHouse` `UInt64`/`Int64` cell (rendered as a quoted
/// string in `FORMAT JSON`, to avoid the precision loss a JS/JSON number
/// would suffer past 2^53 — same reason `ClickHouse`'s JSON output always
/// quotes 64-bit integers). Used only for `SELECT count()`'s result.
fn cell_as_u64(row: &Map<String, Value>, name: &str) -> Result<u64, GoldExportError> {
    match row.get(name) {
        Some(Value::Number(n)) => n
            .as_u64()
            .ok_or_else(|| GoldExportError::Batch(format!("{name}: {n} does not fit in u64"))),
        Some(Value::String(s)) => s
            .parse::<u64>()
            .map_err(|e| GoldExportError::Batch(format!("{name}: {s:?} is not a valid u64: {e}"))),
        other => Err(GoldExportError::Batch(format!(
            "{name}: expected an unsigned integer, got {other:?}"
        ))),
    }
}

/// Builds one column's `SELECT` projection for the per-batch data query
/// [`export_mart`] issues.
///
/// # Why `toTimeZone(col, 'UTC')`, not appending `Z` in Rust
///
/// `ClickHouse` `DateTime`/`DateTime64` columns are either explicitly
/// typed with a timezone (`DateTime('Asia/Jakarta')`,
/// `DateTime64(3, 'Asia/Jakarta')`) or fall back to the server's default
/// timezone — there is no such thing as a genuinely timezone-free
/// `ClickHouse` `DateTime` value; `FORMAT JSON` always renders one as a
/// string in ITS zone. The code this replaces appended a literal `Z` to
/// that string regardless, asserting it was already UTC — for any column
/// whose zone is not UTC, that silently shifted every exported value by
/// the zone's offset (e.g. `Asia/Jakarta` is UTC+7: a row timestamped
/// `03:00` local reads as `03:00Z`, seven hours off). Rather than
/// reimplementing an IANA timezone database in Rust to compute the
/// correct offset ourselves (including the DST-transition cases a named
/// zone can have), this reprojects the column through `ClickHouse`'s OWN
/// `toTimeZone` function before reading it back — `ClickHouse` already
/// carries the full timezone database and does this conversion correctly
/// for any zone, named or server-default. After this projection the
/// string genuinely IS UTC, which is what makes
/// [`cell_as_timestamp_micros`]'s `Z`-append correct again.
///
/// Every other column type is selected unchanged (correctness for those
/// types was never in question — only `DateTime`/`DateTime64` carries an
/// ambiguous, non-UTC-by-default timezone). `Nullable(DateTime...)` is
/// handled the same way: `toTimeZone` on a `NULL` `DateTime` still
/// produces `NULL` (`ClickHouse`'s datetime functions are null-preserving
/// over `Nullable` arguments), so no separate branch is needed.
fn select_projection(col: &ChColumn) -> String {
    let (base, _nullable) = unwrap_nullable(&col.ty);
    let quoted = format!("`{}`", col.name);
    if base == "DateTime" || base.starts_with("DateTime64(") || base.starts_with("DateTime(") {
        format!("toString(toTimeZone({quoted}, 'UTC')) AS {quoted}")
    } else {
        quoted
    }
}

/// Builds the full `SELECT <projections> FROM <source_table>` prefix
/// shared by every batch query — see [`select_projection`] for what each
/// column's projection is.
fn select_clause(source_table: &str, columns: &[ChColumn]) -> String {
    let projections = columns
        .iter()
        .map(select_projection)
        .collect::<Vec<_>>()
        .join(", ");
    format!("SELECT {projections} FROM {source_table}")
}

/// Builds an `ORDER BY` clause over every source column, quoted, in
/// `ClickHouse`'s own column order.
///
/// # Why `ORDER BY` every column
///
/// [`export_mart`] pages through the mart with repeated `LIMIT n OFFSET n`
/// queries rather than one single-pass streaming read (see the module doc
/// comment's "Why batches..." section for why). Plain `LIMIT`/`OFFSET`
/// with no `ORDER BY` has no defined row order in `ClickHouse` (or any
/// SQL engine) — successive queries are not guaranteed to partition the
/// table consistently, which could read some rows twice and miss others
/// entirely across page boundaries. Ordering by every column, rather than
/// e.g. just a primary key the mart may not even have, guarantees a
/// deterministic, reproducible order for ANY mart shape as long as the
/// underlying data does not change between batch queries — the accepted
/// caveat this module doc comment names for the `LIMIT`/`OFFSET` fallback.
fn order_by_clause(columns: &[ChColumn]) -> String {
    columns
        .iter()
        .map(|c| format!("`{}`", c.name))
        .collect::<Vec<_>>()
        .join(", ")
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
/// Gold Iceberg table named `mart_name` (ADR 0010), in batches of at most
/// `batch_size` rows, refusing outright if the source has more than
/// `max_rows` rows — see the module doc comment's "Why batches, a row
/// cap, ..." section for why both exist, and [`crate::config::Config`]'s
/// `gold_export_max_rows`/`gold_export_batch_size` doc comments for where
/// callers should source these two values from (`routes::gold::export`
/// does, from `AppState::config`).
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
/// See [`GoldExportError`]'s variants. In particular,
/// [`GoldExportError::RowCapExceeded`] is returned (and no `ClickHouse` row
/// data is read at all) when `source_table` has more than `max_rows` rows.
pub async fn export_mart(
    ch: &ChClient,
    iceberg_config: &IcebergClientConfig,
    source_table: &str,
    mart_name: &str,
    max_rows: u64,
    batch_size: u64,
) -> Result<GoldExportResult, GoldExportError> {
    let namespace = gold::GOLD_NAMESPACE.to_owned();
    let sanitized_table =
        gold::sanitize_mart_name(mart_name).map_err(|e| GoldExportError::Batch(e.to_string()))?;

    // Zero is never a useful batch size (it would loop forever re-reading
    // offset 0) — clamp rather than fail the export over a misconfigured
    // env var, same "don't refuse to boot/run over a non-load-bearing
    // knob" posture `Config::gold_export_batch_size` documents.
    let batch_size = batch_size.max(1);

    // Fix (1/3): the row cap is checked BEFORE any row data is read — a
    // mart already over the cap must never start streaming batches only
    // to be cut off midway (which would look, from the caller's
    // perspective, exactly like the silent truncation this cap exists to
    // prevent).
    let count_result = ch
        .query(&format!("SELECT count() AS c FROM {source_table}"), None)
        .await?;
    let total_rows = match count_result.data.first() {
        Some(row) => cell_as_u64(row, "c")?,
        None => 0,
    };
    if total_rows > max_rows {
        return Err(GoldExportError::RowCapExceeded {
            mart: mart_name.to_owned(),
            row_count: total_rows,
            cap: max_rows,
        });
    }
    if total_rows == 0 {
        return Ok(GoldExportResult {
            namespace,
            table: sanitized_table,
            format_version: 2,
            rows_exported: 0,
        });
    }

    // Discover the column set/types with a zero-row query — `FORMAT
    // JSON`'s `meta` is populated regardless of how many rows `LIMIT`
    // allows through, so this is a schema-only round trip, not a second
    // whole-mart read.
    let schema_probe = ch
        .query(&format!("SELECT * FROM {source_table} LIMIT 0"), None)
        .await?;
    let plan = plan_columns(&schema_probe.meta)?;
    let domain_fields = plan_to_iceberg_fields(&plan);

    let client = IcebergClient::connect(iceberg_config).await?;
    let mut table = client
        .create_or_load_gold_table(mart_name, domain_fields)
        .await?;

    let arrow_schema = Arc::new(
        iceberg::arrow::schema_to_arrow_schema(table.schema())
            .map_err(|e| GoldExportError::Batch(format!("schema_to_arrow_schema failed: {e}")))?,
    );

    let select_clause = select_clause(source_table, &schema_probe.meta);
    let order_by = order_by_clause(&schema_probe.meta);

    let mut offset = 0u64;
    let mut rows_exported = 0usize;
    loop {
        // `ORDER BY` every column — see `order_by_clause`'s doc comment
        // for why this, not just a mart-specific key, is needed for
        // `LIMIT`/`OFFSET` pagination to be well-defined at all.
        let batch_sql = if order_by.is_empty() {
            format!("{select_clause} LIMIT {batch_size} OFFSET {offset} FORMAT JSON")
        } else {
            format!(
                "{select_clause} ORDER BY {order_by} LIMIT {batch_size} OFFSET {offset} \
                 FORMAT JSON"
            )
        };
        let batch_result = ch.query(&batch_sql, None).await?;
        if batch_result.data.is_empty() {
            break;
        }
        let batch_rows = batch_result.data.len();

        let mut arrays: Vec<ArrayRef> = Vec::with_capacity(plan.len() + 1);
        // `_exported_at` is the same instant for every row within THIS
        // batch — matches the "one partition value per batch" assumption
        // `gold::partition_key_for` documents (each batch is one Parquet
        // file/append, so each needs exactly one partition value derived
        // from it). Recomputed per batch rather than once for the whole
        // run: consecutive batches of one export may therefore carry
        // microseconds-apart `_exported_at` values rather than one
        // identical instant — an acceptable, cosmetic divergence from the
        // pre-batching behavior (a caller filtering `max(_exported_at)`
        // per day-partition still sees them as the same run).
        let now_micros = time::OffsetDateTime::now_utc().unix_timestamp() * 1_000_000;
        arrays.push(Arc::new(TimestampMicrosecondArray::from(vec![
            now_micros;
            batch_rows
        ])));
        for column in &plan {
            arrays.push(build_array(column, &batch_result.data)?);
        }

        let record_batch = RecordBatch::try_new(arrow_schema.clone(), arrays)
            .map_err(|e| GoldExportError::Batch(format!("RecordBatch::try_new failed: {e}")))?;

        table.append(client.as_catalog(), record_batch).await?;
        rows_exported += batch_rows;

        if (batch_rows as u64) < batch_size {
            // A short page means this was the last one — no need to issue
            // one more query that would just come back empty.
            break;
        }
        offset += batch_size;
    }

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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn row_with(name: &str, value: Value) -> Map<String, Value> {
        let mut row = Map::new();
        row.insert(name.to_owned(), value);
        row
    }

    fn ch_column(name: &str, ty: &str) -> ChColumn {
        ChColumn {
            name: name.to_owned(),
            ty: ty.to_owned(),
        }
    }

    // -- select_projection: the actual fix for defect (3) ------------------

    #[test]
    fn select_projection_wraps_datetime_in_to_time_zone_utc() {
        let col = ch_column("created_at", "DateTime('Asia/Jakarta')");
        assert_eq!(
            select_projection(&col),
            "toString(toTimeZone(`created_at`, 'UTC')) AS `created_at`"
        );
    }

    #[test]
    fn select_projection_wraps_datetime64_in_to_time_zone_utc() {
        let col = ch_column("created_at", "DateTime64(3, 'Asia/Jakarta')");
        assert_eq!(
            select_projection(&col),
            "toString(toTimeZone(`created_at`, 'UTC')) AS `created_at`"
        );
    }

    #[test]
    fn select_projection_wraps_bare_datetime_with_no_explicit_zone() {
        // No explicit zone still means "the server's default zone", not
        // "no zone" — see the doc comment on select_projection. It must
        // still be routed through toTimeZone.
        let col = ch_column("created_at", "DateTime");
        assert_eq!(
            select_projection(&col),
            "toString(toTimeZone(`created_at`, 'UTC')) AS `created_at`"
        );
    }

    #[test]
    fn select_projection_wraps_nullable_datetime() {
        let col = ch_column("created_at", "Nullable(DateTime('Asia/Jakarta'))");
        assert_eq!(
            select_projection(&col),
            "toString(toTimeZone(`created_at`, 'UTC')) AS `created_at`"
        );
    }

    #[test]
    fn select_projection_leaves_non_datetime_columns_unchanged() {
        assert_eq!(
            select_projection(&ch_column("region", "String")),
            "`region`"
        );
        assert_eq!(
            select_projection(&ch_column("amount", "Float64")),
            "`amount`"
        );
        assert_eq!(select_projection(&ch_column("day", "Date")), "`day`");
        assert_eq!(
            select_projection(&ch_column("count", "Nullable(UInt64)")),
            "`count`"
        );
    }

    #[test]
    fn select_clause_projects_every_column() {
        let columns = vec![ch_column("region", "String"), ch_column("ts", "DateTime")];
        assert_eq!(
            select_clause("serving.sales", &columns),
            "SELECT `region`, toString(toTimeZone(`ts`, 'UTC')) AS `ts` FROM serving.sales"
        );
    }

    #[test]
    fn order_by_clause_lists_every_column_quoted() {
        let columns = vec![ch_column("region", "String"), ch_column("ts", "DateTime")];
        assert_eq!(order_by_clause(&columns), "`region`, `ts`");
    }

    #[test]
    fn order_by_clause_is_empty_for_no_columns() {
        assert_eq!(order_by_clause(&[]), "");
    }

    // -- the tz-shift bug itself: proof the fix removes it ------------------

    /// The actual defect: appending `Z` directly to a `ClickHouse`
    /// `DateTime` string rendered in a non-UTC zone silently shifts every
    /// value by that zone's offset. `select_projection` closes this by
    /// making sure the string [`cell_as_timestamp_micros`] ever sees for a
    /// `DateTime`/`DateTime64` column has already been converted to UTC BY
    /// `ClickHouse` (`toTimeZone`) — this test proves what the shift would
    /// have been had that projection been skipped, by feeding
    /// [`cell_as_timestamp_micros`] both strings directly and diffing them.
    #[test]
    fn naive_z_append_on_a_non_utc_local_string_would_have_shifted_by_the_zone_offset() {
        // "Asia/Jakarta" is UTC+7: a row whose true instant is 03:00 UTC
        // renders as 10:00 in that zone. `select_projection` guarantees
        // only the FIRST string ever reaches `cell_as_timestamp_micros` in
        // this codebase; the second represents what the pre-fix code path
        // would have parsed instead, had it been given the raw
        // (non-`toTimeZone`-converted) local rendering.
        let correctly_projected_utc =
            row_with("ts", Value::String("2024-01-02 03:00:00".to_owned()));
        let raw_local_jakarta_rendering =
            row_with("ts", Value::String("2024-01-02 10:00:00".to_owned()));

        let correct = cell_as_timestamp_micros(&correctly_projected_utc, "ts")
            .unwrap()
            .unwrap();
        let would_have_been_wrong = cell_as_timestamp_micros(&raw_local_jakarta_rendering, "ts")
            .unwrap()
            .unwrap();

        let seven_hours_in_micros: i64 = 7 * 3600 * 1_000_000;
        assert_eq!(
            would_have_been_wrong - correct,
            seven_hours_in_micros,
            "this is exactly the corruption defect (3) describes: a whole \
             zone-offset shift from blindly appending Z to a non-UTC string"
        );
    }

    #[test]
    fn timestamp_parses_utc_projected_string_to_the_matching_instant() {
        let row = row_with("ts", Value::String("2024-01-02 03:04:05".to_owned()));
        let micros = cell_as_timestamp_micros(&row, "ts").unwrap().unwrap();
        let expected = time::OffsetDateTime::parse("2024-01-02T03:04:05Z", &Iso8601::DEFAULT)
            .unwrap()
            .unix_timestamp()
            * 1_000_000;
        assert_eq!(micros, expected);
    }

    // -- cell_as_u64: SELECT count() parsing --------------------------------

    #[test]
    fn cell_as_u64_parses_quoted_string_uint64() {
        let row = row_with("c", Value::String("123456789012".to_owned()));
        assert_eq!(cell_as_u64(&row, "c").unwrap(), 123_456_789_012);
    }

    #[test]
    fn cell_as_u64_parses_json_number() {
        let row = row_with("c", Value::Number(42.into()));
        assert_eq!(cell_as_u64(&row, "c").unwrap(), 42);
    }

    #[test]
    fn cell_as_u64_rejects_non_numeric_string() {
        let row = row_with("c", Value::String("nope".to_owned()));
        assert!(cell_as_u64(&row, "c").is_err());
    }

    // -- RowCapExceeded: the error must name both the mart and the cap ------

    #[test]
    fn row_cap_exceeded_error_names_mart_and_cap() {
        let err = GoldExportError::RowCapExceeded {
            mart: "sales_by_region".to_owned(),
            row_count: 10_000_000,
            cap: 5_000_000,
        };
        let message = err.to_string();
        assert!(message.contains("sales_by_region"));
        assert!(message.contains("5000000") || message.contains("5_000_000"));
        assert!(message.contains("GOLD_EXPORT_MAX_ROWS"));
    }
}
