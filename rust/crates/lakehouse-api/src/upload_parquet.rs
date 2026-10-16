//! Reading a Parquet file (`.parquet`) for the file upload, as a pure function
//! of its bytes: its columns and their types, and its rows turned into
//! delimited text. Q1 of section 11 of
//! `docs/superpowers/plans/2026-10-07-upload-excel.md` (ADR 0014, amendment of
//! 2026-10-08).
//!
//! Nothing here reads a file, a clock or the network, and nothing panics: the
//! `parquet` crate runs on a file somebody chose, so it runs under
//! `catch_unwind` and a panic is the same refusal as a file that does not open.
//!
//! # Where this sits
//!
//! The load job reads delimited text and nothing else (ADR 0014, decision 3:
//! two readers, one dialect). A Parquet file is brought to that dialect here,
//! once, in the API, exactly as a workbook is ([`crate::upload_workbook`]):
//! [`convert`] writes UTF-8 text separated by commas (the first record is the
//! file's own column names), and the routes store that beside the original and
//! hand it to the unchanged job. Every column is loaded as text, as for CSV and
//! Excel; the file's own types are shown in the preview ([`ColumnInfo`]) and
//! are not carried into the table.
//!
//! # How the file is read
//!
//! Through the `parquet` crate's Arrow reader, with the Arrow schema that is
//! embedded in some files **ignored** (`with_skip_arrow_metadata`): the
//! columns are typed by what Parquet itself says (physical type, logical
//! type), so a file written by one tool reads the same as one written by
//! another, and a dictionary hint cannot change a type. The footer is read
//! first, and everything a refusal needs comes from it: the column list and
//! their types, the number of rows, whether any column is encrypted. Rows are
//! decoded only after that, in batches of [`BATCH_ROWS`], and for a preview
//! only as many as it shows.
//!
//! # How a value becomes text (every column is text)
//!
//! - **Null**: empty text. So is an empty string.
//! - **Text**: as stored.
//! - **Integers** (signed and unsigned): digits.
//! - **Floats** (`float`, `double`, and the rare half float): the shortest form
//!   that reads back to the same value, never a display format; the rule is the
//!   workbook's ([`crate::upload_workbook`]): a whole number has no decimal
//!   point, a value below `1e-6` or from `1e21` up is in exponent form
//!   (`1e-7`, `1e21`), negative zero is `0`, and `NaN` and infinities are
//!   `NaN`, `inf` and `-inf`. A `float` is not widened first: `0.1` stays `0.1`.
//! - **Decimal**: the exact digits with the scale applied, never through a
//!   float, with the trailing zeros the scale gives (`12.50` for
//!   `decimal(10,2)`, `0.0001` for `decimal(20,4)`).
//! - **Boolean**: `true` / `false`.
//! - **Date**: `YYYY-MM-DD`.
//! - **Timestamp**: ISO 8601 with a `T`, `YYYY-MM-DDTHH:MM:SS`, and the
//!   fraction of a second only when there is one (`.123456`, trailing zeros
//!   dropped, so nothing is rounded: milliseconds, microseconds and
//!   nanoseconds are all exact). A column whose logical type is adjusted to UTC
//!   (`isAdjustedToUTC`) holds instants, written in UTC with a trailing `Z`;
//!   any other timestamp is a local date and time, written without a zone. An
//!   instant is never converted to a zone the file does not name.
//! - **Legacy `INT96` timestamp** (the physical type Spark and Impala used
//!   before the logical types existed): read as microseconds, not as the
//!   nanoseconds the crate defaults to, because nanoseconds since 1970 only
//!   cover the years 1677 to 2262 and a date outside them would silently wrap.
//!   An `INT96` carries no `isAdjustedToUTC` flag, so it is written as a local
//!   date and time, without `Z`; the sub-microsecond part is dropped (the
//!   physical type has nanoseconds, and a microsecond is what is kept).
//! - **Time of day**: `HH:MM:SS`, with the fraction only when there is one.
//! - A date, timestamp or time outside what the text can say (a year before
//!   `0000` or past `9999`, a time of day outside one day) is written as the
//!   integer the file holds, not as a wrong date.
//!
//! # Refused, with a reason
//!
//! Each is a fixed sentence ([`ParquetFileError`]); nothing the library said
//! reaches a response.
//!
//! - a file that does not open (not Parquet, cut short, damaged);
//! - an encrypted file (a file whose footer is encrypted, or any of whose
//!   columns is);
//! - a file with no columns;
//! - a column of a type that has no honest text form: **binary** (including a
//!   `uuid` stored as 16 bytes) and **nested** (list, map, struct); the
//!   sentence names the first such column and its type, so the user knows
//!   what to remove;
//! - a file of more than [`MAX_CELLS`] cells (rows times columns), decided from
//!   the footer before any row is decoded. A Parquet file is compressed and can
//!   hold far more than the bytes of the upload suggest, and the API converts
//!   it in memory. The number is a cap, not a measurement;
//! - a conversion whose text would pass [`MAX_OUTPUT_BYTES`]: the cell cap
//!   does not bound the size of a cell, and a column of long repeated strings
//!   compresses to almost nothing.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

use arrow_array::cast::AsArray;
use arrow_array::types::{
    Date32Type, Decimal128Type, Decimal256Type, Float16Type, Float32Type, Float64Type, Int8Type,
    Int16Type, Int32Type, Int64Type, Time32MillisecondType, Time32SecondType,
    Time64MicrosecondType, Time64NanosecondType, TimestampMicrosecondType,
    TimestampMillisecondType, TimestampNanosecondType, TimestampSecondType, UInt8Type, UInt16Type,
    UInt32Type, UInt64Type,
};
use arrow_array::{Array, ArrayRef, ArrowPrimitiveType, PrimitiveArray, RecordBatch};
use arrow_schema::{DataType, Schema, TimeUnit};
use axum::body::Bytes;
use parquet::arrow::arrow_reader::{
    ArrowReaderMetadata, ArrowReaderOptions, ParquetRecordBatchReaderBuilder,
};
use parquet::basic::Type as PhysicalType;
use parquet::file::metadata::RowGroupMetaData;
use serde::Serialize;

use crate::upload_workbook::{finish_text, float_text, write_record};

/// The most cells a Parquet file may hold: 5,000,000 rows times columns, the
/// same cap as a workbook's sheet. A cap that keeps the conversion's memory
/// bounded, not a measured limit.
pub const MAX_CELLS: u64 = crate::upload_workbook::MAX_SHEET_CELLS;

/// The most text a conversion may produce: 512 MiB. The cell cap alone does not
/// bound it (see the module doc); a cap, not a measurement.
pub const MAX_OUTPUT_BYTES: usize = 512 * 1024 * 1024;

/// Rows decoded at a time.
const BATCH_ROWS: usize = 8192;

/// The first and last four bytes of a Parquet file.
const MAGIC: &[u8] = b"PAR1";

/// The first four bytes of a file whose footer is encrypted.
const ENCRYPTED_MAGIC: &[u8] = b"PARE";

/// The longest column name a refusal quotes, in characters.
const MAX_QUOTED_NAME_CHARS: usize = 64;

/// Why a Parquet file could not be read. Each is a fixed sentence (the column
/// name in [`Self::Unsupported`] is the user's own, cleaned of control
/// characters and shortened).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ParquetFileError {
    /// The bytes are not a Parquet file this reader can open: damaged, cut
    /// short, or not Parquet.
    #[error("This file could not be opened as a Parquet file.")]
    Unreadable,
    /// The file, or one of its columns, is encrypted.
    #[error("This Parquet file is encrypted, so it cannot be read. Upload an unencrypted copy.")]
    Encrypted,
    /// The file has no columns.
    #[error("This Parquet file has no columns.")]
    NoColumns,
    /// A column holds values that have no honest text form.
    #[error(
        "Column \"{column}\" holds {type_name} values, which cannot be loaded as text. Remove that column and upload the file again."
    )]
    Unsupported {
        /// The first such column's name, as the file spells it (cleaned).
        column: String,
        /// `binary`, `list`, `map`, `struct` or another fixed word.
        type_name: &'static str,
    },
    /// The file is past [`MAX_CELLS`].
    #[error("That file is larger than the limit of 5,000,000 cells (rows times columns).")]
    TooLarge,
    /// The text of the file would be past [`MAX_OUTPUT_BYTES`].
    #[error("That file holds more text than can be converted (the limit is 512 MB of text).")]
    TooMuchText,
}

/// One column of a file, as the console shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnInfo {
    /// The column's name: the header of the converted text.
    pub name: String,
    /// The type the file declares, in a short fixed vocabulary
    /// (`int64`, `decimal(10,2)`, `timestamp (us, UTC)`, `string` ...). Shown
    /// as "was: ..."; the table's column is text.
    #[serde(rename = "type")]
    pub type_name: String,
}

/// What the footer says about a file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParquetInfo {
    /// The columns in file order.
    pub columns: Vec<ColumnInfo>,
    /// The number of rows the footer counts. Not a measurement of the data:
    /// it is what the file claims.
    pub rows: u64,
}

/// A file converted to delimited text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Converted {
    /// The footer's account of the file.
    pub info: ParquetInfo,
    /// UTF-8, comma, `"` quoting, a line feed after every record; the first
    /// record is the column names.
    pub csv: Vec<u8>,
}

// ── opening ──────────────────────────────────────────────────────────────

/// Run `work`, and turn a panic of the parser into [`ParquetFileError::Unreadable`].
fn guarded<T>(work: impl FnOnce() -> Result<T, ParquetFileError>) -> Result<T, ParquetFileError> {
    catch_unwind(AssertUnwindSafe(work)).unwrap_or(Err(ParquetFileError::Unreadable))
}

/// A file that passed every refusal: the reader's metadata (with the
/// `INT96` columns retyped) and the footer's account of it.
struct Opened {
    meta: ArrowReaderMetadata,
    info: ParquetInfo,
}

/// The options both reads use: the Arrow schema some writers embed is ignored.
fn plain_options() -> ArrowReaderOptions {
    ArrowReaderOptions::new().with_skip_arrow_metadata(true)
}

/// Whether `bytes` start like a file whose footer is encrypted.
#[must_use]
pub fn has_encrypted_magic(bytes: &[u8]) -> bool {
    bytes.starts_with(ENCRYPTED_MAGIC)
}

fn open(bytes: &Bytes, cap: u64) -> Result<Opened, ParquetFileError> {
    if has_encrypted_magic(bytes) {
        return Err(ParquetFileError::Encrypted);
    }
    if !bytes.starts_with(MAGIC) {
        return Err(ParquetFileError::Unreadable);
    }
    let loaded = ArrowReaderMetadata::load(bytes, plain_options())
        .map_err(|_err| ParquetFileError::Unreadable)?;
    if loaded
        .metadata()
        .row_groups()
        .iter()
        .flat_map(RowGroupMetaData::columns)
        .any(|column| column.crypto_metadata().is_some())
    {
        return Err(ParquetFileError::Encrypted);
    }

    let schema = loaded.schema();
    if schema.fields().is_empty() {
        return Err(ParquetFileError::NoColumns);
    }
    for field in schema.fields() {
        if column_texts(&arrow_array::new_empty_array(field.data_type())).is_none() {
            return Err(ParquetFileError::Unsupported {
                column: quoted_name(field.name()),
                type_name: unsupported_word(field.data_type()),
            });
        }
    }
    let rows = u64::try_from(loaded.metadata().file_metadata().num_rows())
        .map_err(|_err| ParquetFileError::Unreadable)?;
    let columns = u64::try_from(schema.fields().len()).unwrap_or(u64::MAX);
    if rows.saturating_mul(columns) > cap {
        return Err(ParquetFileError::TooLarge);
    }

    // A flat schema has one leaf per field, in order; a nested one was refused
    // above, so the leaves line up here.
    let leaves = loaded.parquet_schema().columns();
    let legacy: Vec<bool> = (0..schema.fields().len())
        .map(|index| {
            leaves
                .get(index)
                .is_some_and(|leaf| leaf.physical_type() == PhysicalType::INT96)
        })
        .collect();
    let info = ParquetInfo {
        columns: schema
            .fields()
            .iter()
            .zip(&legacy)
            .map(|(field, is_legacy)| ColumnInfo {
                name: field.name().clone(),
                type_name: if *is_legacy {
                    "timestamp (INT96)".to_owned()
                } else {
                    type_label(field.data_type())
                },
            })
            .collect(),
        rows,
    };
    let meta = if legacy.contains(&true) {
        retyped_legacy(&loaded, &legacy)?
    } else {
        loaded
    };
    Ok(Opened { meta, info })
}

/// The same metadata with each `INT96` column read as microseconds (see the
/// module doc). The crate's own mechanism for it: a supplied schema whose type
/// for such a column is a coarser timestamp.
fn retyped_legacy(
    loaded: &ArrowReaderMetadata,
    legacy: &[bool],
) -> Result<ArrowReaderMetadata, ParquetFileError> {
    let fields: Vec<_> = loaded
        .schema()
        .fields()
        .iter()
        .zip(legacy)
        .map(|(field, is_legacy)| {
            if *is_legacy {
                Arc::new(
                    field
                        .as_ref()
                        .clone()
                        .with_data_type(DataType::Timestamp(TimeUnit::Microsecond, None)),
                )
            } else {
                Arc::clone(field)
            }
        })
        .collect();
    let schema = Schema::new_with_metadata(fields, loaded.schema().metadata().clone());
    ArrowReaderMetadata::try_new(
        Arc::clone(loaded.metadata()),
        ArrowReaderOptions::new().with_schema(Arc::new(schema)),
    )
    .map_err(|_err| ParquetFileError::Unreadable)
}

/// A column name fit to quote in a sentence: no control characters, at most
/// [`MAX_QUOTED_NAME_CHARS`] characters.
fn quoted_name(name: &str) -> String {
    let clean: String = name.chars().filter(|c| !c.is_control()).collect();
    if clean.chars().count() > MAX_QUOTED_NAME_CHARS {
        let head: String = clean.chars().take(MAX_QUOTED_NAME_CHARS).collect();
        format!("{head}...")
    } else {
        clean
    }
}

/// The word a refusal uses for a type that has no text form.
fn unsupported_word(data_type: &DataType) -> &'static str {
    match data_type {
        DataType::Binary
        | DataType::LargeBinary
        | DataType::BinaryView
        | DataType::FixedSizeBinary(_) => "binary",
        DataType::List(_)
        | DataType::LargeList(_)
        | DataType::ListView(_)
        | DataType::LargeListView(_)
        | DataType::FixedSizeList(_, _) => "list",
        DataType::Map(_, _) => "map",
        DataType::Struct(_) => "struct",
        DataType::Interval(_) | DataType::Duration(_) => "interval",
        _ => "unsupported",
    }
}

/// A type in the short vocabulary the console shows ("was: int64").
fn type_label(data_type: &DataType) -> String {
    let unit = |unit: &TimeUnit| match unit {
        TimeUnit::Second => "s",
        TimeUnit::Millisecond => "ms",
        TimeUnit::Microsecond => "us",
        TimeUnit::Nanosecond => "ns",
    };
    match data_type {
        DataType::Null => "null".to_owned(),
        DataType::Boolean => "boolean".to_owned(),
        DataType::Int8 => "int8".to_owned(),
        DataType::Int16 => "int16".to_owned(),
        DataType::Int32 => "int32".to_owned(),
        DataType::Int64 => "int64".to_owned(),
        DataType::UInt8 => "uint8".to_owned(),
        DataType::UInt16 => "uint16".to_owned(),
        DataType::UInt32 => "uint32".to_owned(),
        DataType::UInt64 => "uint64".to_owned(),
        DataType::Float16 => "float16".to_owned(),
        DataType::Float32 => "float32".to_owned(),
        DataType::Float64 => "float64".to_owned(),
        DataType::Decimal128(precision, scale) | DataType::Decimal256(precision, scale) => {
            format!("decimal({precision},{scale})")
        }
        DataType::Date32 | DataType::Date64 => "date".to_owned(),
        DataType::Timestamp(of, Some(_)) => format!("timestamp ({}, UTC)", unit(of)),
        DataType::Timestamp(of, None) => format!("timestamp ({})", unit(of)),
        DataType::Time32(of) | DataType::Time64(of) => format!("time ({})", unit(of)),
        DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View => "string".to_owned(),
        other => unsupported_word(other).to_owned(),
    }
}

// ── the public reads ─────────────────────────────────────────────────────

/// What the footer of the file in `bytes` says: its columns and their types and
/// its row count. No row is decoded.
///
/// # Errors
///
/// [`ParquetFileError::Unreadable`] when the bytes do not open as a Parquet
/// file, [`ParquetFileError::Encrypted`], [`ParquetFileError::NoColumns`],
/// [`ParquetFileError::Unsupported`] for a binary or nested column,
/// [`ParquetFileError::TooLarge`] past [`MAX_CELLS`].
pub fn inspect(bytes: &Bytes) -> Result<ParquetInfo, ParquetFileError> {
    inspect_capped(bytes, MAX_CELLS)
}

fn inspect_capped(bytes: &Bytes, cap: u64) -> Result<ParquetInfo, ParquetFileError> {
    guarded(|| open(bytes, cap).map(|opened| opened.info))
}

/// The file in `bytes` as delimited text: a first record of column names, then
/// at most `max_rows` rows (all when `None`). See the module doc for every
/// rule.
///
/// # Errors
///
/// Those of [`inspect`]; [`ParquetFileError::Unreadable`] also when a page of
/// the file cannot be decoded; [`ParquetFileError::TooMuchText`] past
/// [`MAX_OUTPUT_BYTES`].
pub fn convert(bytes: &Bytes, max_rows: Option<usize>) -> Result<Converted, ParquetFileError> {
    convert_capped(bytes, max_rows, MAX_CELLS, MAX_OUTPUT_BYTES)
}

fn convert_capped(
    bytes: &Bytes,
    max_rows: Option<usize>,
    cell_cap: u64,
    byte_cap: usize,
) -> Result<Converted, ParquetFileError> {
    guarded(|| {
        let Opened { meta, info } = open(bytes, cell_cap)?;
        let mut out = Vec::new();
        let names: Vec<&str> = info.columns.iter().map(|c| c.name.as_str()).collect();
        write_record(&mut out, &names);

        let mut builder = ParquetRecordBatchReaderBuilder::new_with_metadata(bytes.clone(), meta)
            .with_batch_size(BATCH_ROWS);
        if let Some(limit) = max_rows {
            builder = builder.with_limit(limit);
        }
        let reader = builder
            .build()
            .map_err(|_err| ParquetFileError::Unreadable)?;
        for batch in reader {
            let batch = batch.map_err(|_err| ParquetFileError::Unreadable)?;
            write_batch(&mut out, &batch)?;
            if out.len() > byte_cap {
                return Err(ParquetFileError::TooMuchText);
            }
        }
        Ok(Converted {
            info,
            csv: finish_text(out),
        })
    })
}

fn write_batch(out: &mut Vec<u8>, batch: &RecordBatch) -> Result<(), ParquetFileError> {
    let columns: Vec<Vec<String>> = batch
        .columns()
        .iter()
        .map(|column| column_texts(column).ok_or(ParquetFileError::Unreadable))
        .collect::<Result<_, _>>()?;
    let mut fields: Vec<&str> = Vec::with_capacity(columns.len());
    for row in 0..batch.num_rows() {
        fields.clear();
        fields.extend(
            columns
                .iter()
                .map(|column| column.get(row).map_or("", String::as_str)),
        );
        write_record(out, &fields);
    }
    Ok(())
}

// ── values to text ───────────────────────────────────────────────────────

/// The text of every value of a column, null as empty text; `None` for a type
/// that has no text form. The one place that says which types are supported:
/// [`open`] asks it of an empty array of each column's type.
fn column_texts(array: &ArrayRef) -> Option<Vec<String>> {
    let texts = match array.data_type() {
        DataType::Null => vec![String::new(); array.len()],
        DataType::Boolean => {
            let flags = array.as_boolean();
            (0..flags.len())
                .map(|row| {
                    if flags.is_null(row) {
                        String::new()
                    } else {
                        flags.value(row).to_string()
                    }
                })
                .collect()
        }
        DataType::Int8 => column_of::<Int8Type>(array, |a, r| a.value(r).to_string()),
        DataType::Int16 => column_of::<Int16Type>(array, |a, r| a.value(r).to_string()),
        DataType::Int32 => column_of::<Int32Type>(array, |a, r| a.value(r).to_string()),
        DataType::Int64 => column_of::<Int64Type>(array, |a, r| a.value(r).to_string()),
        DataType::UInt8 => column_of::<UInt8Type>(array, |a, r| a.value(r).to_string()),
        DataType::UInt16 => column_of::<UInt16Type>(array, |a, r| a.value(r).to_string()),
        DataType::UInt32 => column_of::<UInt32Type>(array, |a, r| a.value(r).to_string()),
        DataType::UInt64 => column_of::<UInt64Type>(array, |a, r| a.value(r).to_string()),
        DataType::Float16 => {
            column_of::<Float16Type>(array, |a, r| float_text(a.value(r).to_f32()))
        }
        DataType::Float32 => column_of::<Float32Type>(array, |a, r| float_text(a.value(r))),
        DataType::Float64 => column_of::<Float64Type>(array, |a, r| float_text(a.value(r))),
        DataType::Decimal128(_, _) => {
            column_of::<Decimal128Type>(array, PrimitiveArray::value_as_string)
        }
        DataType::Decimal256(_, _) => {
            column_of::<Decimal256Type>(array, PrimitiveArray::value_as_string)
        }
        DataType::Date32 => column_of::<Date32Type>(array, |a, r| {
            let days = i64::from(a.value(r));
            date_text(days).unwrap_or_else(|| days.to_string())
        }),
        DataType::Timestamp(unit, zone) => {
            let zoned = zone.is_some();
            match unit {
                TimeUnit::Second => column_of::<TimestampSecondType>(array, |a, r| {
                    timestamp_text(a.value(r), SECONDS, zoned)
                }),
                TimeUnit::Millisecond => column_of::<TimestampMillisecondType>(array, |a, r| {
                    timestamp_text(a.value(r), MILLIS, zoned)
                }),
                TimeUnit::Microsecond => column_of::<TimestampMicrosecondType>(array, |a, r| {
                    timestamp_text(a.value(r), MICROS, zoned)
                }),
                TimeUnit::Nanosecond => column_of::<TimestampNanosecondType>(array, |a, r| {
                    timestamp_text(a.value(r), NANOS, zoned)
                }),
            }
        }
        DataType::Time32(TimeUnit::Second) => {
            column_of::<Time32SecondType>(array, |a, r| time_text(i64::from(a.value(r)), SECONDS))
        }
        DataType::Time32(TimeUnit::Millisecond) => {
            column_of::<Time32MillisecondType>(array, |a, r| {
                time_text(i64::from(a.value(r)), MILLIS)
            })
        }
        DataType::Time64(TimeUnit::Microsecond) => {
            column_of::<Time64MicrosecondType>(array, |a, r| time_text(a.value(r), MICROS))
        }
        DataType::Time64(TimeUnit::Nanosecond) => {
            column_of::<Time64NanosecondType>(array, |a, r| time_text(a.value(r), NANOS))
        }
        DataType::Utf8 => {
            let strings = array.as_string::<i32>();
            (0..strings.len())
                .map(|row| {
                    if strings.is_null(row) {
                        String::new()
                    } else {
                        strings.value(row).to_owned()
                    }
                })
                .collect()
        }
        DataType::LargeUtf8 => {
            let strings = array.as_string::<i64>();
            (0..strings.len())
                .map(|row| {
                    if strings.is_null(row) {
                        String::new()
                    } else {
                        strings.value(row).to_owned()
                    }
                })
                .collect()
        }
        _ => return None,
    };
    Some(texts)
}

/// `show` applied to every non-null value of a primitive column.
fn column_of<T: ArrowPrimitiveType>(
    array: &ArrayRef,
    show: impl Fn(&PrimitiveArray<T>, usize) -> String,
) -> Vec<String> {
    let typed = array.as_primitive::<T>();
    (0..typed.len())
        .map(|row| {
            if typed.is_null(row) {
                String::new()
            } else {
                show(typed, row)
            }
        })
        .collect()
}

/// How a timestamp or time unit divides a second.
#[derive(Debug, Clone, Copy)]
struct Scale {
    per_second: i64,
    /// Digits of the fraction at this scale.
    digits: usize,
}

const SECONDS: Scale = Scale {
    per_second: 1,
    digits: 0,
};
const MILLIS: Scale = Scale {
    per_second: 1_000,
    digits: 3,
};
const MICROS: Scale = Scale {
    per_second: 1_000_000,
    digits: 6,
};
const NANOS: Scale = Scale {
    per_second: 1_000_000_000,
    digits: 9,
};

const SECONDS_PER_DAY: i64 = 86_400;

/// The civil date `days` after 1970-01-01 as `YYYY-MM-DD`, or `None` outside
/// the years 0000 to 9999. Howard Hinnant's `civil_from_days`, for the
/// proleptic Gregorian calendar.
fn date_text(days: i64) -> Option<String> {
    let shifted = days.checked_add(719_468)?;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (0..=9999)
        .contains(&year)
        .then(|| format!("{year:04}-{month:02}-{day:02}"))
}

/// `HH:MM:SS` for a second of the day, then the fraction when there is one.
fn clock_text(second_of_day: i64, fraction: i64, scale: Scale) -> String {
    let (hour, minute, second) = (
        second_of_day / 3600,
        second_of_day / 60 % 60,
        second_of_day % 60,
    );
    let mut text = format!("{hour:02}:{minute:02}:{second:02}");
    if fraction != 0 {
        let digits = format!("{fraction:0width$}", width = scale.digits);
        text.push('.');
        text.push_str(digits.trim_end_matches('0'));
    }
    text
}

fn timestamp_text(value: i64, scale: Scale, zoned: bool) -> String {
    let seconds = value.div_euclid(scale.per_second);
    let fraction = value.rem_euclid(scale.per_second);
    let Some(date) = date_text(seconds.div_euclid(SECONDS_PER_DAY)) else {
        return value.to_string();
    };
    let clock = clock_text(seconds.rem_euclid(SECONDS_PER_DAY), fraction, scale);
    let zone = if zoned { "Z" } else { "" };
    format!("{date}T{clock}{zone}")
}

fn time_text(value: i64, scale: Scale) -> String {
    if !(0..SECONDS_PER_DAY * scale.per_second).contains(&value) {
        return value.to_string();
    }
    clock_text(value / scale.per_second, value % scale.per_second, scale)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use arrow_array::{
        BinaryArray, BooleanArray, Int32Array, Int64Array, LargeStringArray, ListArray,
        StringArray, TimestampMicrosecondArray,
    };
    use arrow_schema::{Field, Fields};
    use parquet::arrow::ArrowWriter;

    use super::*;
    use crate::upload_parse::{Encoding, decode, split_records};

    fn fixture(bytes: &'static [u8]) -> Bytes {
        Bytes::from_static(bytes)
    }

    const ORDERS: &[u8] = include_bytes!("../../../../ops/fixtures/parquet/orders.parquet");
    const EMPTY: &[u8] = include_bytes!("../../../../ops/fixtures/parquet/empty.parquet");
    const TYPES: &[u8] = include_bytes!("../../../../ops/fixtures/parquet/types.parquet");
    const INT96: &[u8] = include_bytes!("../../../../ops/fixtures/parquet/legacy_int96.parquet");
    const BINARY: &[u8] = include_bytes!("../../../../ops/fixtures/parquet/binary_column.parquet");
    const NESTED: &[u8] = include_bytes!("../../../../ops/fixtures/parquet/nested_column.parquet");

    /// The converted text, read back with the load's own dialect.
    fn records(converted: &Converted) -> Vec<Vec<String>> {
        split_records(&decode(&converted.csv, Encoding::Utf8), ',')
    }

    fn rows(rows: &[&[&str]]) -> Vec<Vec<String>> {
        rows.iter()
            .map(|row| row.iter().map(|cell| (*cell).to_owned()).collect())
            .collect()
    }

    /// A file written in memory by the crate's own writer, for the cases no
    /// committed fixture covers.
    fn written(batch: &RecordBatch) -> Bytes {
        let mut out = Vec::new();
        let mut writer = ArrowWriter::try_new(&mut out, batch.schema(), None).unwrap();
        writer.write(batch).unwrap();
        writer.close().unwrap();
        Bytes::from(out)
    }

    fn batch_of(columns: Vec<(&str, ArrayRef)>) -> RecordBatch {
        RecordBatch::try_from_iter(columns).unwrap()
    }

    // ── what a file holds ───────────────────────────────────────────────

    #[test]
    fn a_plain_file_converts_to_its_column_names_and_rows_as_text() {
        let converted = convert(&fixture(ORDERS), None).unwrap();
        assert_eq!(
            records(&converted),
            rows(&[
                &["sku", "name", "qty", "price", "received"],
                &["A-100", "Bolt, hex M6", "10", "1.50", "2025-09-24"],
                &["A-200", "Washer", "250", "0.05", "2025-09-25"],
                &[
                    "A-300",
                    "Flange DN50 \u{2013} steel",
                    "4",
                    "12.00",
                    "2025-10-01"
                ],
            ])
        );
        assert_eq!(converted.info.rows, 3);
    }

    #[test]
    fn every_type_the_conversion_writes_comes_out_as_its_pinned_text() {
        let converted = convert(&fixture(TYPES), None).unwrap();
        let expected = rows(&[
            &[
                "id",
                "small",
                "count",
                "ratio",
                "tiny",
                "weight",
                "price",
                "ledger",
                "day",
                "at_utc_us",
                "at_local_ms",
                "at_local_ns",
                "at_utc_ns",
                "clock",
                "clock_ms",
                "flag",
                "note",
            ],
            &[
                "1",
                "-8",
                "7",
                "0.5",
                "1e-7",
                "0.1",
                "12.50",
                "123456789012345.6789",
                "2025-09-24",
                "2025-09-24T13:30:05.123456Z",
                "2025-09-24T13:30:05.12",
                "2025-09-24T13:30:05.000000001",
                "2025-09-24T13:30:05.000000001Z",
                "13:30:00",
                "23:59:59.999",
                "true",
                "plain",
            ],
            &[
                "9007199254740993",
                "127",
                "18446744073709551615",
                "0.30000000000000004",
                "1e21",
                "2",
                "-0.05",
                "0.0001",
                "1969-12-31",
                "2025-09-24T00:00:00Z",
                "2000-02-29T23:59:59",
                "1969-12-31T23:59:59.5",
                "1970-01-01T00:00:00Z",
                "00:00:00.5",
                "00:00:00",
                "false",
                "a, \"quoted\"\nline two",
            ],
            // A row of nothing but nulls: every cell is empty text.
            &[
                "", "", "", "", "", "", "", "", "", "", "", "", "", "", "", "", "",
            ],
        ]);
        assert_eq!(records(&converted), expected);
    }

    #[test]
    fn the_footer_lists_each_column_with_the_type_the_file_declares() {
        let info = inspect(&fixture(TYPES)).unwrap();
        let listed: Vec<(&str, &str)> = info
            .columns
            .iter()
            .map(|c| (c.name.as_str(), c.type_name.as_str()))
            .collect();
        assert_eq!(
            listed,
            [
                ("id", "int64"),
                ("small", "int8"),
                ("count", "uint64"),
                ("ratio", "float64"),
                ("tiny", "float64"),
                ("weight", "float32"),
                ("price", "decimal(10,2)"),
                ("ledger", "decimal(20,4)"),
                ("day", "date"),
                ("at_utc_us", "timestamp (us, UTC)"),
                ("at_local_ms", "timestamp (ms)"),
                ("at_local_ns", "timestamp (ns)"),
                ("at_utc_ns", "timestamp (ns, UTC)"),
                ("clock", "time (us)"),
                ("clock_ms", "time (ms)"),
                ("flag", "boolean"),
                ("note", "string"),
            ]
        );
        assert_eq!(info.rows, 3);
    }

    /// The legacy `INT96` timestamp is read as microseconds, so the year 1
    /// (outside nanoseconds since 1970) is the year 1 and not a wrapped date.
    #[test]
    fn a_legacy_int96_timestamp_is_a_local_time_and_survives_a_year_nanoseconds_cannot_hold() {
        let converted = convert(&fixture(INT96), None).unwrap();
        assert_eq!(
            records(&converted),
            rows(&[
                &["id", "seen"],
                &["1", "2025-09-24T13:30:05.123456"],
                &["2", "0001-01-01T00:00:00"],
                &["3", ""],
            ])
        );
        assert_eq!(
            converted.info.columns[1].type_name, "timestamp (INT96)",
            "the preview says what the file declared"
        );
    }

    #[test]
    fn a_file_with_no_rows_converts_to_its_header_alone() {
        let converted = convert(&fixture(EMPTY), None).unwrap();
        assert_eq!(
            records(&converted),
            rows(&[&["sku", "name", "qty", "price", "received"]])
        );
        assert_eq!(converted.info.rows, 0);
    }

    // ── how many rows are decoded ───────────────────────────────────────

    #[test]
    fn a_preview_decodes_only_the_rows_it_asks_for_across_batches() {
        let ids: Vec<i64> = (0..20_000).collect();
        let bytes = written(&batch_of(vec![(
            "id",
            Arc::new(Int64Array::from(ids)) as ArrayRef,
        )]));
        let all = convert(&bytes, None).unwrap();
        assert_eq!(records(&all).len(), 20_001, "the header and every row");
        let some = convert(&bytes, Some(9_000)).unwrap();
        let read = records(&some);
        assert_eq!(read.len(), 9_001, "the header and the first 9,000 rows");
        assert_eq!(read[9_000], ["8999"]);
        assert_eq!(some.info.rows, 20_000, "the footer still counts the file");
        let none = convert(&bytes, Some(0)).unwrap();
        assert_eq!(records(&none), rows(&[&["id"]]));
    }

    // ── the other types the crate can give ──────────────────────────────

    #[test]
    fn a_string_with_a_comma_a_quote_and_a_newline_reads_back_as_it_was() {
        let text = "a,b \"c\"\nline two\n";
        let bytes = written(&batch_of(vec![(
            "note",
            Arc::new(StringArray::from(vec![Some(text), None, Some("")])) as ArrayRef,
        )]));
        let converted = convert(&bytes, None).unwrap();
        assert_eq!(
            records(&converted),
            rows(&[&["note"], &[text], &[""], &[""]])
        );
    }

    #[test]
    fn a_single_column_of_nulls_is_written_so_it_reads_back_as_records() {
        let bytes = written(&batch_of(vec![(
            "only",
            Arc::new(Int32Array::from(vec![None, Some(5)])) as ArrayRef,
        )]));
        let converted = convert(&bytes, None).unwrap();
        assert_eq!(records(&converted), rows(&[&["only"], &[""], &["5"]]));
    }

    #[test]
    fn booleans_and_large_strings_are_text_too() {
        let flags = BooleanArray::from(vec![Some(true), Some(false), None]);
        let large = LargeStringArray::from(vec![Some("x"), None, Some("z")]);
        assert_eq!(
            column_texts(&(Arc::new(flags) as ArrayRef)).unwrap(),
            ["true", "false", ""]
        );
        assert_eq!(
            column_texts(&(Arc::new(large) as ArrayRef)).unwrap(),
            ["x", "", "z"]
        );
    }

    // ── dates, times and numbers, on their own ──────────────────────────

    #[test]
    fn dates_are_the_proleptic_gregorian_calendar_and_out_of_range_ones_are_not_invented() {
        assert_eq!(date_text(0).unwrap(), "1970-01-01");
        assert_eq!(date_text(-1).unwrap(), "1969-12-31");
        assert_eq!(date_text(11_017).unwrap(), "2000-03-01");
        assert_eq!(date_text(19_782).unwrap(), "2024-02-29");
        assert_eq!(date_text(-719_162).unwrap(), "0001-01-01");
        assert_eq!(date_text(2_932_896).unwrap(), "9999-12-31");
        assert_eq!(date_text(2_932_897), None, "year 10000");
        assert_eq!(date_text(-719_529), None, "year -1");
        assert_eq!(date_text(i64::MAX), None);
    }

    #[test]
    fn a_timestamp_before_1970_keeps_its_fraction_and_one_outside_the_calendar_is_its_integer() {
        assert_eq!(
            timestamp_text(-1, MICROS, true),
            "1969-12-31T23:59:59.999999Z"
        );
        assert_eq!(timestamp_text(0, SECONDS, false), "1970-01-01T00:00:00");
        assert_eq!(
            timestamp_text(1_500, MILLIS, false),
            "1970-01-01T00:00:01.5"
        );
        assert_eq!(
            timestamp_text(i64::MAX, SECONDS, true),
            i64::MAX.to_string()
        );
    }

    #[test]
    fn a_time_of_day_has_a_fraction_only_when_it_has_one_and_one_outside_a_day_is_its_integer() {
        assert_eq!(time_text(0, SECONDS), "00:00:00");
        assert_eq!(time_text(86_399, SECONDS), "23:59:59");
        assert_eq!(time_text(86_400, SECONDS), "86400");
        assert_eq!(time_text(-1, MILLIS), "-1");
        assert_eq!(time_text(45_000_000_001, NANOS), "00:00:45.000000001");
    }

    #[test]
    fn a_float_is_written_in_its_own_shortest_form() {
        assert_eq!(float_text(0.1_f32), "0.1");
        assert_eq!(float_text(f64::from(0.1_f32)), "0.10000000149011612");
        assert_eq!(float_text(-0.0_f64), "0");
        assert_eq!(float_text(1e21_f64), "1e21");
        assert_eq!(float_text(f64::NAN), "NaN");
        assert_eq!(float_text(f64::NEG_INFINITY), "-inf");
    }

    // ── refusals ────────────────────────────────────────────────────────

    #[test]
    fn a_binary_column_is_refused_by_name_and_type() {
        let refusal = inspect(&fixture(BINARY)).unwrap_err();
        assert_eq!(
            refusal,
            ParquetFileError::Unsupported {
                column: "blob".to_owned(),
                type_name: "binary"
            }
        );
        assert_eq!(
            refusal.to_string(),
            "Column \"blob\" holds binary values, which cannot be loaded as text. Remove that column and upload the file again."
        );
        assert_eq!(convert(&fixture(BINARY), None).unwrap_err(), refusal);
    }

    #[test]
    fn a_list_column_is_refused_by_name_and_type() {
        assert_eq!(
            inspect(&fixture(NESTED)).unwrap_err(),
            ParquetFileError::Unsupported {
                column: "tags".to_owned(),
                type_name: "list"
            }
        );
    }

    #[test]
    fn a_struct_and_a_map_column_are_refused_and_the_first_such_column_is_named() {
        let inner = Fields::from(vec![Field::new("x", DataType::Int32, true)]);
        let structs = arrow_array::StructArray::new(
            inner,
            vec![Arc::new(Int32Array::from(vec![1, 2])) as ArrayRef],
            None,
        );
        let bytes = written(&batch_of(vec![
            ("id", Arc::new(Int32Array::from(vec![1, 2])) as ArrayRef),
            ("point", Arc::new(structs) as ArrayRef),
        ]));
        assert_eq!(
            inspect(&bytes).unwrap_err(),
            ParquetFileError::Unsupported {
                column: "point".to_owned(),
                type_name: "struct"
            }
        );

        let mut builder = arrow_array::builder::MapBuilder::new(
            None,
            arrow_array::builder::StringBuilder::new(),
            arrow_array::builder::Int32Builder::new(),
        );
        builder.keys().append_value("k");
        builder.values().append_value(1);
        builder.append(true).unwrap();
        let bytes = written(&batch_of(vec![(
            "attrs",
            Arc::new(builder.finish()) as ArrayRef,
        )]));
        assert_eq!(
            inspect(&bytes).unwrap_err(),
            ParquetFileError::Unsupported {
                column: "attrs".to_owned(),
                type_name: "map"
            }
        );
    }

    #[test]
    fn a_list_array_type_is_reported_as_a_list() {
        let list = ListArray::from_iter_primitive::<Int32Type, _, _>(vec![Some(vec![Some(1)])]);
        assert!(column_texts(&(Arc::new(list) as ArrayRef)).is_none());
        let binary = BinaryArray::from(vec![b"a".as_slice()]);
        assert!(column_texts(&(Arc::new(binary) as ArrayRef)).is_none());
    }

    #[test]
    fn a_file_that_is_not_parquet_or_is_cut_short_is_unreadable() {
        let whole = fixture(TYPES);
        for (name, bytes) in [
            ("text", Bytes::from_static(b"id,name\n1,a\n")),
            ("magic only", Bytes::from_static(b"PAR1")),
            (
                "garbage",
                Bytes::from_static(b"PAR1\x15\x04junkjunkjunkPAR1"),
            ),
            ("cut in half", whole.slice(..whole.len() / 2)),
            ("cut short of its footer", whole.slice(..whole.len() - 3)),
            ("empty", Bytes::new()),
        ] {
            assert_eq!(
                inspect(&bytes).unwrap_err(),
                ParquetFileError::Unreadable,
                "{name}"
            );
            assert_eq!(
                convert(&bytes, None).unwrap_err(),
                ParquetFileError::Unreadable,
                "{name}"
            );
        }
    }

    /// A file cut anywhere is refused, never a panic or a loop. (A flipped
    /// byte is not tried: what the crate does with a damaged page header was
    /// not checked, and this test cannot be run where it was written.)
    #[test]
    fn a_file_cut_anywhere_never_panics() {
        let whole = TYPES.to_vec();
        for end in (0..whole.len()).step_by(37) {
            let _ = convert(&Bytes::copy_from_slice(&whole[..end]), None);
        }
    }

    #[test]
    fn a_file_whose_footer_is_encrypted_is_refused_as_encrypted() {
        let bytes = Bytes::from_static(b"PARE\x00\x01\x02\x03rest of an encrypted file PARE");
        assert_eq!(inspect(&bytes).unwrap_err(), ParquetFileError::Encrypted);
        assert_eq!(
            convert(&bytes, None).unwrap_err(),
            ParquetFileError::Encrypted
        );
        assert!(has_encrypted_magic(&bytes));
        assert!(!has_encrypted_magic(&fixture(ORDERS)));
    }

    #[test]
    fn the_cell_cap_is_decided_from_the_footer_before_any_row_is_decoded() {
        assert_eq!(MAX_CELLS, 5_000_000);
        // Three rows by five columns is 15 cells.
        let bytes = fixture(ORDERS);
        assert!(inspect_capped(&bytes, 15).is_ok(), "at the cap");
        assert_eq!(
            inspect_capped(&bytes, 14).unwrap_err(),
            ParquetFileError::TooLarge
        );
        assert_eq!(
            convert_capped(&bytes, None, 14, MAX_OUTPUT_BYTES).unwrap_err(),
            ParquetFileError::TooLarge
        );
        // A file with no rows is no cells, whatever its columns.
        assert!(inspect_capped(&fixture(EMPTY), 0).is_ok());
    }

    #[test]
    fn a_conversion_past_the_text_cap_is_refused() {
        assert_eq!(
            convert_capped(&fixture(ORDERS), None, MAX_CELLS, 40).unwrap_err(),
            ParquetFileError::TooMuchText
        );
    }

    #[test]
    fn a_column_name_in_a_sentence_has_no_control_characters_and_is_shortened() {
        assert_eq!(quoted_name("a\nb\u{0}c"), "abc");
        let long = "x".repeat(100);
        assert_eq!(quoted_name(&long), format!("{}...", "x".repeat(64)));
    }

    #[test]
    fn no_sentence_names_a_path_a_host_or_a_library() {
        let sentences = [
            ParquetFileError::Unreadable,
            ParquetFileError::Encrypted,
            ParquetFileError::NoColumns,
            ParquetFileError::Unsupported {
                column: "c".to_owned(),
                type_name: "binary",
            },
            ParquetFileError::TooLarge,
            ParquetFileError::TooMuchText,
        ];
        for sentence in sentences {
            let text = sentence.to_string();
            for forbidden in ["http", "://", "uploads/", "arrow", "parquet::", "thrift"] {
                assert!(!text.contains(forbidden), "{text:?} has {forbidden:?}");
            }
        }
    }

    #[test]
    fn a_timestamp_with_a_zone_in_memory_is_an_instant_and_without_one_is_local() {
        let zoned =
            TimestampMicrosecondArray::from(vec![Some(1_000_000), None]).with_timezone("UTC");
        let local = TimestampMicrosecondArray::from(vec![Some(1_000_000), None]);
        assert_eq!(
            column_texts(&(Arc::new(zoned) as ArrayRef)).unwrap(),
            ["1970-01-01T00:00:01Z", ""]
        );
        assert_eq!(
            column_texts(&(Arc::new(local) as ArrayRef)).unwrap(),
            ["1970-01-01T00:00:01", ""]
        );
    }
}
