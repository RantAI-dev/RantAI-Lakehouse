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
//! Since `DATA-17` T2.2 (plan AD2) the API no longer converts a Parquet file
//! for a load: the job reads the file natively (`dagster/dispar_orchestrate/
//! parquet_ingest.py`), in batches, and writes the same text for every value
//! that [`convert`] writes here. What stays here is what the API needs to
//! SHOW: [`inspect`] (the footer: columns, types, row count) and [`convert`]
//! with `max_rows`, which writes the first rows of the first row group as the
//! preview's text. Both read through a `ChunkReader`.
//!
//! Since `SEC-22` P1 the API reads NOTHING of a Parquet file itself: the
//! child process does, and it is given the ranges it needs — the tail of the
//! file, which holds the footer, and the first row group — through the
//! sparse reader of `upload_parquet_parts`. That is [`inspect_parts`] and
//! [`convert_parts`], which are the same reads as [`inspect`] and [`convert`]
//! over a reader that has no other byte of the file. What the footer declares
//! (its length, where every column chunk lies, the size of every row group) is
//! bounded before it is trusted ([`MAX_FOOTER_BYTES`], `check_layout`).
//! `convert` is also the oracle of the text rules: `ops/fixtures/parquet/
//! types.expected.json` is what it writes for `types.parquet` and what the
//! job's reader must write too (a test on each side). Every column is loaded
//! as text, as for CSV and Excel; the file's own types are shown in the preview
//! ([`ColumnInfo`]) and are not carried into the table.
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
//!   the footer before any row is decoded, **only when the whole file is
//!   converted in memory** (`convert` with no row limit). A Parquet file is
//!   compressed and can hold far more than the bytes of the upload suggest, and
//!   that conversion holds it all. The footer read ([`inspect`]) and a preview
//!   ([`convert`] with a row limit) do not hold the file, so they have no cell
//!   cap: the load job streams a Parquet file by row group and the upload is
//!   bounded by its byte cap (`DATA-17` T6.2 item 3,
//!   `docs/plans/DATA-17-RESULT.md`). What still protects the API's own memory
//!   there is [`MAX_FOOTER_BYTES`] (the footer is bounded before it is
//!   read), [`MAX_PREVIEW_GROUP_BYTES`] (the first row group a preview fetches)
//!   and [`MAX_OUTPUT_BYTES`]. The number is a cap, not a measurement;
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
use parquet::arrow::arrow_reader::{
    ArrowReaderMetadata, ArrowReaderOptions, ParquetRecordBatchReaderBuilder,
};
use parquet::basic::Type as PhysicalType;
use parquet::file::metadata::{ParquetMetaData, RowGroupMetaData};
use parquet::file::reader::ChunkReader;
use serde::{Deserialize, Serialize};

use crate::upload_workbook::{finish_text, float_text, write_record};

/// The most cells a Parquet file may hold when it is converted whole in memory:
/// 5,000,000 rows times columns, the same cap as a workbook's sheet. A cap that
/// keeps that conversion's memory bounded, not a measured limit. It does not
/// apply to [`inspect`] or to a preview ([`cell_cap_for`]): the job streams a
/// Parquet file, and the upload is bounded by its bytes.
pub const MAX_CELLS: u64 = crate::upload_workbook::MAX_SHEET_CELLS;

/// The cell cap for a read: [`MAX_CELLS`] only when the whole file is converted
/// in memory (`max_rows` is `None`), none for the footer read and for a preview,
/// which hold a footer and a first row group and never the file. `DATA-17` T6.2
/// item 3; the API's memory is bounded there by [`MAX_FOOTER_BYTES`],
/// [`MAX_PREVIEW_GROUP_BYTES`] and [`MAX_OUTPUT_BYTES`], not by a cell count.
const fn cell_cap_for(max_rows: Option<usize>) -> u64 {
    match max_rows {
        None => MAX_CELLS,
        Some(_) => u64::MAX,
    }
}

/// The most text a conversion may produce: 512 MiB. The cell cap alone does not
/// bound it (see the module doc); a cap, not a measurement.
pub const MAX_OUTPUT_BYTES: usize = 512 * 1024 * 1024;

/// Rows decoded at a time.
const BATCH_ROWS: usize = 8192;

/// The longest footer the reader will take: 16 MiB. A footer lists every row
/// group and column chunk, so a wide file with many row groups has a large one;
/// a number above this is a file that was damaged or made to cost memory
/// (`DATA-17` T2.2, plan AD2: what the footer declares is bounded before it is
/// read). A cap, not a measurement.
pub const MAX_FOOTER_BYTES: u64 = 16 * 1024 * 1024;

/// The most compressed bytes of the first row group a preview will fetch to show
/// rows: 64 MiB. A preview decodes a row group's column chunks whole, so this
/// bounds its memory; a file whose first group is larger is previewed with its
/// columns and no rows (see [`convert`]).
pub const MAX_PREVIEW_GROUP_BYTES: i64 = 64 * 1024 * 1024;

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
    /// The file is past [`MAX_CELLS`] and was to be converted whole.
    #[error("That file is larger than the limit of 5,000,000 cells (rows times columns).")]
    TooLarge,
    /// The text of the file would be past [`MAX_OUTPUT_BYTES`].
    #[error("That file holds more text than can be converted (the limit is 512 MB of text).")]
    TooMuchText,
}

/// One column of a file, as the console shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParquetInfo {
    /// The columns in file order.
    pub columns: Vec<ColumnInfo>,
    /// The number of rows the footer counts. Not a measurement of the data:
    /// it is what the file claims.
    pub rows: u64,
}

/// The byte range of a Parquet file's first row group.
///
/// The row group is what a preview decodes, so the range is what the child
/// must be given (`SEC-22` P1: the file goes to the child by ranges, never
/// whole). The range is the span of the group's column chunks, which is
/// where every byte the preview reads lies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupRange {
    /// Where the first row group starts in the file.
    pub offset: u64,
    /// How many bytes it takes.
    pub len: u64,
}

/// What a file's footer says: its columns and its row count, and where its
/// first row group is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParquetFooter {
    /// The columns and the row count.
    pub info: ParquetInfo,
    /// The first row group's byte range, or `None` for a file with no row
    /// group (a file with no rows has none).
    pub group: Option<GroupRange>,
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
    /// The first row group's byte range, or `None` for a file with no row
    /// group.
    group: Option<GroupRange>,
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

/// Check what the footer declares against the file that holds it, before the
/// footer is read: the length it announces fits in the file and under
/// [`MAX_FOOTER_BYTES`]. A file shorter than a magic, a footer length and a
/// magic is not Parquet.
///
/// A footer whose magic is `PARE` is an ENCRYPTED footer, and that is said
/// as such: the file cannot be read, and the reason is the encryption, not a
/// damaged file.
fn check_footer_length<R: ChunkReader>(reader: &R) -> Result<u64, ParquetFileError> {
    let size = reader.len();
    if size < 12 {
        return Err(ParquetFileError::Unreadable);
    }
    let tail = reader
        .get_bytes(size - 8, 8)
        .map_err(|_err| ParquetFileError::Unreadable)?;
    match tail.get(4..8) {
        Some(magic) if magic == MAGIC => {}
        Some(magic) if magic == ENCRYPTED_MAGIC => return Err(ParquetFileError::Encrypted),
        _ => return Err(ParquetFileError::Unreadable),
    }
    let announced = tail
        .get(0..4)
        .and_then(|raw| <[u8; 4]>::try_from(raw).ok())
        .map(u32::from_le_bytes)
        .map_or(u64::MAX, u64::from);
    if announced > MAX_FOOTER_BYTES || announced.saturating_add(12) > size {
        return Err(ParquetFileError::Unreadable);
    }
    Ok(announced)
}

/// Check what the parsed footer declares about the row groups against the file:
/// every column chunk lies inside the data region (`data_end`: the byte where the
/// footer starts), and no row group is larger than the data.
fn check_layout(metadata: &ParquetMetaData, data_end: u64) -> Result<(), ParquetFileError> {
    let footer = u64::try_from(metadata.file_metadata().num_rows()).ok();
    if footer.is_none() {
        return Err(ParquetFileError::Unreadable);
    }
    for group in metadata.row_groups() {
        let too_big = u64::try_from(group.compressed_size()).map_or(true, |bytes| bytes > data_end);
        if too_big {
            return Err(ParquetFileError::Unreadable);
        }
        for column in group.columns() {
            let (start, length) = column.byte_range();
            if start.checked_add(length).is_none_or(|end| end > data_end) {
                return Err(ParquetFileError::Unreadable);
            }
        }
    }
    Ok(())
}

/// Open a Parquet file's metadata.
///
/// `head` says whether the reader has the file's FIRST bytes. It does when
/// the whole file is in memory, and it does not when only the file's tail is
/// (`SEC-22` P1: the child is given the tail of the file and the first row
/// group, never the whole file). Without the first bytes the magic cannot be
/// read, so the file's PARQUET-ness is decided by the footer's own magic —
/// which is the same four bytes — and the encrypted-footer case is said as
/// the encryption it is.
fn open<R: ChunkReader + 'static>(
    reader: &R,
    cap: u64,
    head: bool,
) -> Result<Opened, ParquetFileError> {
    if head {
        if reader.len() < 4 {
            return Err(ParquetFileError::Unreadable);
        }
        let first = reader
            .get_bytes(0, 4)
            .map_err(|_err| ParquetFileError::Unreadable)?;
        if has_encrypted_magic(&first) {
            return Err(ParquetFileError::Encrypted);
        }
        if !first.starts_with(MAGIC) {
            return Err(ParquetFileError::Unreadable);
        }
    }
    let footer = check_footer_length(reader)?;
    let loaded = ArrowReaderMetadata::load(reader, plain_options())
        .map_err(|_err| ParquetFileError::Unreadable)?;
    check_layout(loaded.metadata(), reader.len().saturating_sub(8 + footer))?;
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
    let group = first_group_range(meta.metadata());
    Ok(Opened { meta, info, group })
}

/// The byte range of the first row group, or `None` for a file with none.
///
/// The range is the span of the group's column chunks: every byte a preview
/// of this file reads lies inside it.
fn first_group_range(metadata: &ParquetMetaData) -> Option<GroupRange> {
    let group = metadata.row_groups().first()?;
    let mut start = u64::MAX;
    let mut end = 0_u64;
    for column in group.columns() {
        let (offset, length) = column.byte_range();
        let column_end = offset.checked_add(length)?;
        start = start.min(offset);
        end = end.max(column_end);
    }
    if start == u64::MAX {
        return None;
    }
    Some(GroupRange {
        offset: start,
        len: end - start,
    })
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
pub(crate) fn quoted_name(name: &str) -> String {
    let clean: String = name.chars().filter(|c| !c.is_control()).collect();
    if clean.chars().count() > MAX_QUOTED_NAME_CHARS {
        let head: String = clean.chars().take(MAX_QUOTED_NAME_CHARS).collect();
        format!("{head}...")
    } else {
        clean
    }
}

/// The word a refusal uses for a type that has no text form, when the word
/// comes from outside this module — the child process's answer (`SEC-22`).
///
/// Only the six words [`unsupported_word`] names are words; anything else is
/// `unsupported`, so a word that is not one of ours cannot end up in a
/// sentence.
pub(crate) fn type_word(word: &str) -> &'static str {
    match word {
        "binary" => "binary",
        "list" => "list",
        "map" => "map",
        "struct" => "struct",
        "interval" => "interval",
        _ => "unsupported",
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
/// [`ParquetFileError::Unsupported`] for a binary or nested column. There is no
/// cell cap here: only the footer is read ([`cell_cap_for`]).
pub fn inspect<R: ChunkReader + 'static>(reader: &R) -> Result<ParquetInfo, ParquetFileError> {
    inspect_capped(reader, u64::MAX)
}

fn inspect_capped<R: ChunkReader + 'static>(
    reader: &R,
    cap: u64,
) -> Result<ParquetInfo, ParquetFileError> {
    guarded(|| open(reader, cap, true).map(|opened| opened.info))
}

/// What the footer of a file says, read through a reader that holds only the
/// TAIL of the file and not its first bytes (`SEC-22` P1: the child is given
/// the ranges it needs, never the whole file).
///
/// The answer is the footer and the byte range of the first row group, which
/// is what a preview needs to be given next.
///
/// # Errors
///
/// As [`inspect`].
pub fn inspect_parts<R: ChunkReader + 'static>(
    reader: &R,
) -> Result<ParquetFooter, ParquetFileError> {
    guarded(|| {
        let opened = open(reader, u64::MAX, false)?;
        Ok(ParquetFooter {
            info: opened.info,
            group: opened.group,
        })
    })
}

/// The file in `bytes` as delimited text: a first record of column names, then
/// at most `max_rows` rows (all when `None`). See the module doc for every
/// rule.
///
/// The product no longer converts a whole Parquet file — the load job reads
/// the file natively (`DATA-17` T2.2) — so this is the ORACLE of the text
/// rules: the tests read it and compare with what the job's reader writes
/// (`ops/fixtures/parquet/types.expected.json`).
///
/// # Errors
///
/// Those of [`inspect`]; [`ParquetFileError::Unreadable`] also when a page of
/// the file cannot be decoded; [`ParquetFileError::TooMuchText`] past
/// [`MAX_OUTPUT_BYTES`].
#[allow(
    dead_code,
    reason = "the product converts no whole Parquet file; the tests read this as the oracle of the text rules"
)]
pub fn convert<R: ChunkReader + Clone + 'static>(
    reader: &R,
    max_rows: Option<usize>,
) -> Result<Converted, ParquetFileError> {
    convert_capped(
        reader,
        max_rows,
        cell_cap_for(max_rows),
        MAX_OUTPUT_BYTES,
        MAX_PREVIEW_GROUP_BYTES as u64,
        true,
    )
}

/// The rows of a file read through a reader that holds the file's TAIL and its
/// first row group, and nothing else (`SEC-22` P1).
///
/// `group_cap` is the most the first row group may take: a group over it is
/// shown as its columns and no rows, because the bytes it takes are not
/// worth moving (the reader decodes a group's column chunks whole, so the
/// bound is what keeps a preview's memory bounded).
///
/// # Errors
///
/// As [`convert`].
pub fn convert_parts<R: ChunkReader + Clone + 'static>(
    reader: &R,
    max_rows: Option<usize>,
    group_cap: u64,
) -> Result<Converted, ParquetFileError> {
    convert_capped(
        reader,
        max_rows,
        cell_cap_for(max_rows),
        MAX_OUTPUT_BYTES,
        group_cap,
        false,
    )
}

fn convert_capped<R: ChunkReader + Clone + 'static>(
    reader: &R,
    max_rows: Option<usize>,
    cell_cap: u64,
    byte_cap: usize,
    group_cap: u64,
    head: bool,
) -> Result<Converted, ParquetFileError> {
    guarded(|| {
        let Opened { meta, info, group } = open(reader, cell_cap, head)?;
        let mut out = Vec::new();
        let names: Vec<&str> = info.columns.iter().map(|c| c.name.as_str()).collect();
        write_record(&mut out, &names).map_err(|_err| ParquetFileError::Unreadable)?;

        // A preview (`max_rows` given) reads the FIRST row group only, and only
        // when it is small enough to fetch; a larger one is shown as its columns
        // with no rows. The reader decodes a group's column chunks whole, so the
        // bound is what keeps a preview's memory bounded.
        let mut builder = ParquetRecordBatchReaderBuilder::new_with_metadata(reader.clone(), meta)
            .with_batch_size(BATCH_ROWS);
        if let Some(limit) = max_rows {
            let group_fits = group.is_some_and(|range| range.len <= group_cap);
            if !group_fits || info.rows == 0 {
                return Ok(Converted {
                    info,
                    csv: finish_text(out),
                });
            }
            builder = builder.with_limit(limit).with_row_groups(vec![0]);
        }
        let batches = builder
            .build()
            .map_err(|_err| ParquetFileError::Unreadable)?;
        for batch in batches {
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
        write_record(out, &fields).map_err(|_err| ParquetFileError::Unreadable)?;
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
    use axum::body::Bytes;
    use parquet::arrow::ArrowWriter;
    use parquet::file::metadata::ParquetMetaDataReader;

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
            convert_capped(&bytes, None, 14, MAX_OUTPUT_BYTES, u64::MAX, true).unwrap_err(),
            ParquetFileError::TooLarge
        );
        // A file with no rows is no cells, whatever its columns.
        assert!(inspect_capped(&fixture(EMPTY), 0).is_ok());
    }

    /// `DATA-17` T6.2 item 3: the cell cap is for the whole-file conversion in
    /// memory; the footer read and a preview have none, so a Parquet file that
    /// the job streams is bounded by its bytes.
    #[test]
    fn the_cell_cap_applies_to_a_whole_conversion_and_not_to_a_footer_or_a_preview() {
        assert_eq!(cell_cap_for(None), MAX_CELLS);
        assert_eq!(cell_cap_for(Some(2000)), u64::MAX);
        // A file of 15 cells is read by `inspect` and by a preview whatever the cap
        // of a whole conversion would say.
        let bytes = fixture(ORDERS);
        assert!(inspect(&bytes).is_ok());
        assert!(convert(&bytes, Some(2)).is_ok());
    }

    #[test]
    fn a_conversion_past_the_text_cap_is_refused() {
        assert_eq!(
            convert_capped(&fixture(ORDERS), None, MAX_CELLS, 40, u64::MAX, true).unwrap_err(),
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

    // ── DATA-17 T2.2: bounded, ranged reading ───────────────────────────

    /// `types.parquet` as the job's reader must write it: the shared fixture of
    /// `ops/fixtures/parquet/types.expected.json`, read by a test on each side.
    #[test]
    fn the_shared_fixture_is_what_the_converter_writes_for_types_parquet() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../ops/fixtures/parquet/types.expected.json"
        );
        let shared: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let expected: Vec<Vec<String>> = shared["records"]
            .as_array()
            .unwrap()
            .iter()
            .map(|record| {
                record
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|cell| cell.as_str().unwrap().to_owned())
                    .collect()
            })
            .collect();
        assert_eq!(records(&convert(&fixture(TYPES), None).unwrap()), expected);
    }

    /// A reader that records every range the crate asks for.
    #[derive(Clone)]
    struct Recording {
        inner: Bytes,
        asked: Arc<std::sync::Mutex<Vec<(u64, u64)>>>,
    }

    impl Recording {
        fn of(inner: Bytes) -> Self {
            Self {
                inner,
                asked: Arc::default(),
            }
        }
        fn asked(&self) -> Vec<(u64, u64)> {
            self.asked.lock().unwrap().clone()
        }
    }

    impl parquet::file::reader::Length for Recording {
        fn len(&self) -> u64 {
            u64::try_from(self.inner.len()).unwrap()
        }
    }

    impl ChunkReader for Recording {
        type T = <Bytes as ChunkReader>::T;
        fn get_read(&self, start: u64) -> parquet::errors::Result<Self::T> {
            self.asked.lock().unwrap().push((start, u64::MAX));
            self.inner.get_read(start)
        }
        fn get_bytes(&self, start: u64, length: usize) -> parquet::errors::Result<Bytes> {
            self.asked
                .lock()
                .unwrap()
                .push((start, u64::try_from(length).unwrap()));
            self.inner.get_bytes(start, length)
        }
    }

    /// A file of `groups` row groups of `per_group` rows each.
    fn grouped(groups: usize, per_group: usize) -> Bytes {
        let rows = groups * per_group;
        let batch = batch_of(vec![
            (
                "id",
                Arc::new(Int64Array::from_iter_values(
                    0..i64::try_from(rows).unwrap(),
                )) as ArrayRef,
            ),
            (
                "text",
                Arc::new(StringArray::from_iter_values((0..rows).map(|n| {
                    format!("row number {n} with some padding to fill the page")
                }))) as ArrayRef,
            ),
        ]);
        let mut out = Vec::new();
        let props = parquet::file::properties::WriterProperties::builder()
            .set_max_row_group_row_count(Some(per_group))
            .build();
        let mut writer = ArrowWriter::try_new(&mut out, batch.schema(), Some(props)).unwrap();
        writer.write(&batch).unwrap();
        writer.close().unwrap();
        Bytes::from(out)
    }

    #[test]
    fn the_footer_alone_is_read_to_inspect_a_file_and_the_first_row_group_to_preview_it() {
        let file = grouped(4, 5_000);
        let size = u64::try_from(file.len()).unwrap();

        let recording = Recording::of(file.clone());
        let info = inspect(&recording).unwrap();
        assert_eq!(info.rows, 20_000);
        let asked = recording.asked();
        assert!(
            asked
                .iter()
                .all(|(start, _)| *start == 0 || *start >= size / 2),
            "inspect reads the magic and the end of the file: {asked:?}"
        );

        let recording = Recording::of(file.clone());
        let previewed = convert(&recording, Some(20)).unwrap();
        assert_eq!(records(&previewed).len(), 21);
        let footer = u64::try_from(footer_len(&file)).unwrap();
        let footer_start = size - 8 - footer;
        let metadata = ParquetMetaDataReader::decode_metadata(
            &file[usize::try_from(footer_start).unwrap()..usize::try_from(size - 8).unwrap()],
        )
        .unwrap();
        let first_end = metadata
            .row_group(0)
            .columns()
            .iter()
            .map(|c| {
                let (start, len) = c.byte_range();
                start + len
            })
            .max()
            .unwrap();
        let asked = recording.asked();
        assert!(
            asked
                .iter()
                .all(|(start, _)| *start < first_end || *start >= footer_start),
            "a preview reads the footer and the first row group, nothing between the \
             first group's end ({first_end}) and the footer ({footer_start}): {asked:?}"
        );
        assert!(
            first_end < size / 2,
            "the first of four groups is a part of the file"
        );
    }

    fn footer_len(file: &Bytes) -> usize {
        let tail = &file[file.len() - 8..file.len() - 4];
        usize::try_from(u32::from_le_bytes(tail.try_into().unwrap())).unwrap()
    }

    #[test]
    fn a_footer_that_announces_more_than_the_file_holds_is_unreadable_and_nothing_panics() {
        let mut broken = fixture(ORDERS).to_vec();
        let at = broken.len() - 8;
        broken[at..at + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(
            inspect(&Bytes::from(broken.clone())).unwrap_err(),
            ParquetFileError::Unreadable
        );
        let whole = u32::try_from(broken.len()).unwrap();
        broken[at..at + 4].copy_from_slice(&whole.to_le_bytes());
        assert_eq!(
            inspect(&Bytes::from(broken)).unwrap_err(),
            ParquetFileError::Unreadable
        );
        // A file shorter than a magic, a length and a magic.
        for len in 0..12 {
            assert_eq!(
                inspect(&Bytes::from(vec![b'P'; len])).unwrap_err(),
                ParquetFileError::Unreadable,
                "{len} bytes"
            );
        }
    }

    #[test]
    fn column_chunks_that_lie_outside_the_data_the_footer_describes_are_refused() {
        // Drop 100 bytes from the front of the data: the footer is intact and
        // every offset in it now points past where its data can be.
        let mut shifted = fixture(ORDERS).to_vec();
        shifted.drain(4..104);
        assert_eq!(
            inspect(&Bytes::from(shifted)).unwrap_err(),
            ParquetFileError::Unreadable
        );
    }

    #[test]
    fn a_first_row_group_over_the_preview_bound_shows_the_columns_and_no_rows() {
        let file = grouped(2, 1_000);
        let shown = convert_capped(&file, Some(20), MAX_CELLS, MAX_OUTPUT_BYTES, 1, true).unwrap();
        assert_eq!(records(&shown), rows(&[&["id", "text"]]));
        assert_eq!(
            shown.info.rows, 2_000,
            "the footer's count is still reported"
        );
        // Without the bound the same preview has its rows.
        let all = convert(&file, Some(20)).unwrap();
        assert_eq!(records(&all).len(), 21);
    }
}
