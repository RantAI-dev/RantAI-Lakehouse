//! What an upload is when it is JSON, JSON Lines, Avro, gzip or zip, and what reading its
//! first rows produces (`DATA-17` slice 4, plan T4.1; decisions 2, 11, 12 and 13 of
//! `docs/core/features/upload-big-files-and-more-formats.md`).
//!
//! The load job reads these natively and in full (`dagster/dispar_orchestrate/
//! upload_formats.py`); the API reads only bounded samples (plan AD2): a prefix of at most
//! the caller's bound, a zip's central directory by range, the gzip stream's first bytes.
//! The two are readers of one set of rules, pinned by the fixtures in
//! `ops/fixtures/uploads/formats/`, which this module's tests and the job's tests both read.
//!
//! # The rules
//!
//! - **JSON** is a top-level array of objects (`.json`), **JSON Lines** one object per line
//!   (`.jsonl`, `.ndjson`; blank lines skipped). A UTF-8 byte order mark is dropped.
//! - **Columns** are the object keys in first-seen order. The preview sees the keys of the
//!   rows it samples; the load reads the whole file for its keys, so a key that first
//!   appears after the sample is a column of the table the preview did not show.
//! - **Cells** are text: a string as it is, `true` or `false`, a number in `serde_json`'s
//!   shortest form (the load keeps the digits the source wrote, `1.50`; this is the only
//!   difference), a missing key or `null` as empty, and a nested object or array as compact
//!   JSON text with its keys in the source's order (decision 11: no child tables).
//! - **Avro** is an object container file whose top-level type is a record, and it must be
//!   **uncompressed** (the null codec): `PR #186 review BLOCKER 1`. A compressed container
//!   is refused, because both readers inflate a whole block with no limit on the output
//!   (apache-avro 0.21 and fastavro 1.13.1 both hand the inflated block over whole), so a
//!   few MB of zeros becomes gigabytes in the API and in the job. A field that is
//!   a union with null may be empty; a record, array or map is JSON text (a map's keys are
//!   sorted here and in the file's order in the load); `bytes` and `fixed` columns, which
//!   have no text form, are refused, as a binary Parquet column is. The schema's own types
//!   are the starting guess of the Check step (decision 10 of `upload-column-types.md`).
//! - **gzip** holds one file of text, JSON or JSON Lines; the inner kind is classed after
//!   decoding. **zip** holds up to [`MAX_ZIP_FILES`] files of one supported format other than
//!   Excel and Parquet; folders (and macOS's `__MACOSX/` forks) are ignored; an archive
//!   inside is refused by name and by magic bytes; an encrypted file is refused.
//!
//! # The uncompressed byte counter
//!
//! Decision 2: the uncompressed size counts toward the cap, so a small archive cannot unpack
//! into more than was measured. [`count_unpacked_gzip`] and [`count_unpacked_zip`] decode a
//! whole file once, discarding the bytes, and refuse it ([`Refusal::TooLargeUnpacked`]) the
//! moment the count passes the cap: refused, never truncated. They run when the file arrives
//! (`routes::uploads`), and every preview read here is bounded by a limit far below the cap,
//! so a bomb can never be decoded past it by a preview. The job counts again as it reads
//! (`upload_formats.py`, `Budget`).

use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};

use serde_json::{Map, Value};

use crate::upload_types::{ColumnType, DateFormat, Declared, declared_from_parquet};

/// Decision 13: a zip holds up to ten files.
pub const MAX_ZIP_FILES: usize = 10;

/// The most entries a zip's central directory may declare (`PR #186 review BLOCKER 2`).
///
/// `zip::ZipArchive::new` reads the whole central directory into memory, one record per
/// entry (folders included), before the caller can look at any of it: an archive of
/// millions of folder entries would take the API's memory with it, in a blocking task
/// that outlives the request's timeout. The count is read from the end-of-central-directory
/// record instead, so such an archive is refused without being read. The cap is far above
/// [`MAX_ZIP_FILES`], which is the number of files a load reads: an archive of ten files
/// with a few folders beside them is well inside it.
pub const MAX_ZIP_ENTRIES: u64 = 1_000;

/// The most bytes a zip's central directory may take (`PR #186 review BLOCKER 2`).
///
/// The same read, bounded: a thousand entries with ordinary names take about 60 KiB of
/// central directory, so 4 MiB leaves room for long names and refuses an archive whose
/// directory is megabytes long.
pub const MAX_ZIP_CENTRAL_BYTES: u64 = 4 * 1024 * 1024;

/// How a stored object is wrapped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compression {
    /// Not wrapped.
    None,
    /// One gzip stream.
    Gzip,
    /// A zip archive.
    Zip,
}

impl Compression {
    /// The word the job's configuration uses.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Gzip => "gzip",
            Self::Zip => "zip",
        }
    }
}

/// What a file holds once unwrapped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// Delimited text (CSV, TSV, a text export).
    Delimited,
    /// A top-level array of objects.
    Json,
    /// One object per line.
    JsonLines,
    /// An Avro object container file.
    Avro,
}

impl Format {
    /// The word the job's configuration uses.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Delimited => "delimited",
            Self::Json => "json",
            Self::JsonLines => "jsonl",
            Self::Avro => "avro",
        }
    }

    /// The format a file extension (lower case, no dot) says, for the files of a zip and the
    /// inner name of a gzip file.
    #[must_use]
    pub fn of_extension(extension: &str) -> Option<Self> {
        match extension {
            "csv" | "tsv" | "txt" => Some(Self::Delimited),
            "json" => Some(Self::Json),
            "jsonl" | "ndjson" => Some(Self::JsonLines),
            "avro" => Some(Self::Avro),
            _ => None,
        }
    }
}

/// What an upload is, once its wrapper is taken off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layout {
    /// How it is wrapped.
    pub compression: Compression,
    /// What it holds.
    pub format: Format,
}

/// The last extension of a file name, lower case, without the dot.
#[must_use]
pub fn extension(name: &str) -> String {
    let last = name.rsplit(['/', '\\']).next().unwrap_or(name);
    last.rsplit_once('.')
        .map(|(_, ext)| ext.to_ascii_lowercase())
        .unwrap_or_default()
}

/// What a file's NAME says about it, for the kinds this module reads. Necessary, not
/// sufficient: the first bytes must agree ([`crate::upload_parse::sniff`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ByName {
    /// `.json`, `.jsonl`, `.ndjson` or `.avro`.
    Plain(Format),
    /// `.gz`, with the inner format its own name gives (`.csv.gz`), when it gives one.
    Gzip(Option<Format>),
    /// `.zip`.
    Zip,
    /// Anything else: the other readers decide.
    Other,
}

/// Classify a name by its extensions.
#[must_use]
pub fn by_name(name: &str) -> ByName {
    match extension(name).as_str() {
        "json" => ByName::Plain(Format::Json),
        "jsonl" | "ndjson" => ByName::Plain(Format::JsonLines),
        "avro" => ByName::Plain(Format::Avro),
        "gz" => {
            let inner = name.rsplit_once('.').map_or("", |(stem, _)| stem);
            ByName::Gzip(Format::of_extension(&extension(inner)).filter(|f| *f != Format::Avro))
        }
        "zip" => ByName::Zip,
        _ => ByName::Other,
    }
}

/// The format of decoded text that has no name to say: an array starts with `[`, a JSON Lines
/// file with `{`, anything else is delimited text.
#[must_use]
pub fn format_of_text(prefix: &[u8]) -> Format {
    let body = prefix.strip_prefix(b"\xef\xbb\xbf").unwrap_or(prefix);
    match body.iter().find(|byte| !byte.is_ascii_whitespace()) {
        Some(b'[') => Format::Json,
        Some(b'{') => Format::JsonLines,
        _ => Format::Delimited,
    }
}

/// Why a file was refused. Every message is fixed text: nothing from a decoder reaches a
/// response.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum Refusal {
    /// The file does not open as gzip.
    #[error("This file does not open as a gzip file.")]
    NotGzip,
    /// The gzip file holds an archive.
    #[error("A gzip file must hold one text or JSON file, not another archive.")]
    GzipHoldsArchive,
    /// A gzip file of a kind that is not text or JSON.
    #[error("A gzip file must hold a CSV, TSV, text, JSON or JSON Lines file.")]
    GzipHoldsOther,
    /// The file does not open as a zip.
    #[error("This file does not open as a zip archive.")]
    NotZip,
    /// A zip with no files.
    #[error("The zip archive holds no files.")]
    ZipEmpty,
    /// A zip with too many files.
    #[error("A zip archive may hold at most 10 files.")]
    ZipTooMany,
    /// A zip whose central directory is bigger than the API will read.
    ///
    /// `PR #186 review BLOCKER 2`: the central directory is read into memory before the
    /// archive can be looked at, so an archive with millions of entries is refused on the
    /// counts it declares, before the directory is built.
    #[error("This zip archive holds too many files to be read.")]
    ZipTooManyEntries,
    /// A zip with an encrypted file.
    #[error("A zip archive with a password cannot be read.")]
    ZipEncrypted,
    /// A zip holding an archive.
    #[error("A zip archive may not hold another archive.")]
    ZipNested,
    /// The files of a zip are not all of one supported format.
    #[error(
        "The files of a zip archive must all be of one supported format: CSV, TSV, text, JSON, JSON Lines or Avro."
    )]
    ZipMixed,
    /// The unpacked bytes passed the cap.
    #[error("The file is larger than the upload limit once unpacked.")]
    TooLargeUnpacked,
    /// A JSON file that is not an array.
    #[error("A JSON file must hold an array of objects.")]
    JsonNotArray,
    /// A JSON file that is not valid.
    #[error("This is not a valid JSON file.")]
    JsonInvalid,
    /// An element that is not an object.
    #[error("Every element of the array and every line must be a JSON object.")]
    JsonNotObjects,
    /// More than 1,000 distinct keys.
    #[error("The file has more than 1,000 columns.")]
    TooManyColumns,
    /// Nothing to load.
    #[error("That file has no rows, so there is nothing to load.")]
    NoRows,
    /// Not an Avro file.
    #[error("This is not a valid Avro file.")]
    AvroInvalid,
    /// An Avro file whose top level is not a record.
    #[error("An Avro file must hold records.")]
    AvroNotRecord,
    /// An Avro column with no text form.
    #[error("An Avro column holds binary data, which cannot be loaded.")]
    AvroBinary,
    /// An Avro codec the upload does not read.
    ///
    /// `PR #186 review BLOCKER 1`: only the null codec is read, on both sides. A
    /// compressed container is refused, because the readers inflate a whole block with no
    /// limit on the output (apache-avro 0.21 and fastavro 1.13.1), so a small file of
    /// zeros becomes gigabytes of memory.
    #[error("This Avro file is compressed. Save it with no compression.")]
    AvroCodec,
}

/// A bounded reading of a file's first rows.
#[derive(Debug, Clone, PartialEq)]
pub struct Parsed {
    /// The column names, as the file spells them.
    pub columns: Vec<String>,
    /// The rows read, as text, at most the bound asked for.
    pub rows: Vec<Vec<String>>,
    /// The types the file declares for its columns (Avro), when it does.
    pub declared: Option<Vec<Declared>>,
    /// Whether the file holds more than `rows` shows.
    pub more: bool,
}

const UTF8_BOM: &[u8] = b"\xef\xbb\xbf";

/// The text a JSON value is in a cell.
fn json_text(value: Option<&Value>) -> String {
    match value {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(text)) => text.clone(),
        Some(Value::Bool(flag)) => if *flag { "true" } else { "false" }.to_owned(),
        Some(Value::Number(number)) => number.to_string(),
        Some(nested) => nested.to_string(),
    }
}

fn skip_whitespace(bytes: &[u8], mut at: usize) -> usize {
    while bytes.get(at).is_some_and(u8::is_ascii_whitespace) {
        at += 1;
    }
    at
}

/// The array closed at `at`: nothing but whitespace may follow it.
fn closed(
    bytes: &[u8],
    at: usize,
    objects: Vec<Map<String, Value>>,
) -> Result<(Vec<Map<String, Value>>, bool), Refusal> {
    if skip_whitespace(bytes, at + 1) == bytes.len() {
        Ok((objects, false))
    } else {
        Err(Refusal::JsonInvalid)
    }
}

/// The objects of a JSON array in a prefix, at most `max`. The second value says the array
/// continues past what was read.
fn array_objects(
    bytes: &[u8],
    head_is_truncated: bool,
    max: usize,
) -> Result<(Vec<Map<String, Value>>, bool), Refusal> {
    let start = if bytes.starts_with(UTF8_BOM) {
        UTF8_BOM.len()
    } else {
        0
    };
    let mut at = skip_whitespace(bytes, start);
    if bytes.get(at) != Some(&b'[') {
        return Err(Refusal::JsonNotArray);
    }
    at += 1;
    let mut objects = Vec::new();
    let mut first = true;
    loop {
        at = skip_whitespace(bytes, at);
        match bytes.get(at) {
            Some(b']') if first => return closed(bytes, at, objects),
            Some(b']') => return Err(Refusal::JsonInvalid), // a trailing comma
            Some(_) => {}
            None if head_is_truncated => return Ok((objects, true)),
            None => return Err(Refusal::JsonInvalid),
        }
        if objects.len() == max {
            return Ok((objects, true));
        }
        let mut stream = serde_json::Deserializer::from_slice(&bytes[at..]).into_iter::<Value>();
        match stream.next() {
            Some(Ok(Value::Object(object))) => {
                at += stream.byte_offset();
                objects.push(object);
            }
            Some(Ok(_)) => return Err(Refusal::JsonNotObjects),
            Some(Err(err)) if err.is_eof() && head_is_truncated => return Ok((objects, true)),
            Some(Err(_)) | None => return Err(Refusal::JsonInvalid),
        }
        first = false;
        at = skip_whitespace(bytes, at);
        match bytes.get(at) {
            Some(b',') => at += 1,
            Some(b']') => return closed(bytes, at, objects),
            None if head_is_truncated => return Ok((objects, true)),
            _ => return Err(Refusal::JsonInvalid),
        }
    }
}

/// The objects of a JSON Lines prefix, at most `max`.
fn line_objects(
    bytes: &[u8],
    head_is_truncated: bool,
    max: usize,
) -> Result<(Vec<Map<String, Value>>, bool), Refusal> {
    let body = bytes.strip_prefix(UTF8_BOM).unwrap_or(bytes);
    let ends_whole = body.ends_with(b"\n");
    let mut lines: Vec<&[u8]> = body.split(|byte| *byte == b'\n').collect();
    if head_is_truncated && !ends_whole {
        lines.pop(); // the cut line
    }
    let mut objects = Vec::new();
    for line in lines {
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        if objects.len() == max {
            return Ok((objects, true));
        }
        match serde_json::from_slice::<Value>(line) {
            Ok(Value::Object(object)) => objects.push(object),
            Ok(_) => return Err(Refusal::JsonNotObjects),
            Err(_) => return Err(Refusal::JsonInvalid),
        }
    }
    Ok((objects, head_is_truncated))
}

/// Read the first rows of JSON (`lines == false`) or JSON Lines text.
///
/// `head_is_truncated` says the bytes are only the start of a longer file: a cut last
/// element or line is left out, not shown as a whole one.
///
/// # Errors
///
/// [`Refusal`] for text that is not a valid array (or lines) of objects, for no rows, for a
/// file with no key at all, and for more than [`crate::upload_parse::MAX_COLUMNS`] keys.
pub fn read_json(
    bytes: &[u8],
    head_is_truncated: bool,
    lines: bool,
    max_rows: usize,
) -> Result<Parsed, Refusal> {
    let (objects, more) = if lines {
        line_objects(bytes, head_is_truncated, max_rows)?
    } else {
        array_objects(bytes, head_is_truncated, max_rows)?
    };
    if objects.is_empty() {
        return Err(Refusal::NoRows);
    }
    let mut columns: Vec<String> = Vec::new();
    for object in &objects {
        for key in object.keys() {
            if !columns.iter().any(|known| known == key) {
                columns.push(key.clone());
                if columns.len() > crate::upload_parse::MAX_COLUMNS {
                    return Err(Refusal::TooManyColumns);
                }
            }
        }
    }
    if columns.is_empty() {
        return Err(Refusal::NoRows);
    }
    let rows = objects
        .iter()
        .map(|object| {
            columns
                .iter()
                .map(|key| json_text(object.get(key)))
                .collect()
        })
        .collect();
    Ok(Parsed {
        columns,
        rows,
        declared: None,
        more,
    })
}

// ── Avro ─────────────────────────────────────────────────────────────────

/// The `avro.codec` of a container's header, `None` when it has none (the null codec).
///
/// The header is the 4-byte magic and then a map of bytes to bytes: a zig-zag varint count of
/// pairs (a negative count is followed by the byte size of the block), each pair a
/// length-prefixed key and value.
fn avro_codec(bytes: &[u8]) -> Result<Option<String>, Refusal> {
    fn varint(bytes: &[u8], at: &mut usize) -> Option<i64> {
        let mut shift = 0u32;
        let mut raw = 0u64;
        loop {
            let byte = *bytes.get(*at)?;
            *at += 1;
            raw |= u64::from(byte & 0x7f).checked_shl(shift)?;
            if byte & 0x80 == 0 {
                break;
            }
            shift += 7;
            if shift > 63 {
                return None;
            }
        }
        let half = i64::try_from(raw >> 1).ok()?;
        Some(if raw & 1 == 1 { -half - 1 } else { half })
    }
    fn chunk<'a>(bytes: &'a [u8], at: &mut usize) -> Option<&'a [u8]> {
        let len = usize::try_from(varint(bytes, at)?).ok()?;
        let end = at.checked_add(len)?;
        let slice = bytes.get(*at..end)?;
        *at = end;
        Some(slice)
    }
    let mut at = 4;
    let mut codec = None;
    loop {
        let mut count = varint(bytes, &mut at).ok_or(Refusal::AvroInvalid)?;
        if count == 0 {
            return Ok(codec);
        }
        if count < 0 {
            count = -count;
            varint(bytes, &mut at).ok_or(Refusal::AvroInvalid)?;
        }
        for _ in 0..count {
            let key = chunk(bytes, &mut at).ok_or(Refusal::AvroInvalid)?;
            let value = chunk(bytes, &mut at).ok_or(Refusal::AvroInvalid)?;
            if key == b"avro.codec" {
                codec = Some(String::from_utf8_lossy(value).into_owned());
            }
        }
    }
}

mod avro {
    //! Avro values as the text the job writes.

    use apache_avro::Schema;
    use apache_avro::types::Value;
    use serde_json::{Number, Value as Json};
    use time::{Date, OffsetDateTime};

    use super::{ColumnType, DateFormat, Declared, Refusal, declared_from_parquet};

    /// The unscaled integer of a big-endian two's-complement number that fits `i128`.
    fn signed_be(bytes: &[u8]) -> Option<i128> {
        if bytes.len() > 16 {
            return None;
        }
        let negative = bytes.first().is_some_and(|byte| byte & 0x80 != 0);
        let mut raw: u128 = if negative { u128::MAX } else { 0 };
        for byte in bytes {
            raw = (raw << 8) | u128::from(*byte);
        }
        #[allow(
            clippy::cast_possible_wrap,
            reason = "two's complement reinterpretation of a value sign-extended to 128 bits"
        )]
        Some(raw as i128)
    }

    /// `1.50` from the unscaled `150` and the scale 2.
    fn decimal_text(unscaled: i128, scale: usize) -> String {
        let digits = unscaled.unsigned_abs().to_string();
        let padded = format!("{digits:0>width$}", width = scale + 1);
        let (whole, fraction) = padded.split_at(padded.len() - scale);
        let sign = if unscaled < 0 { "-" } else { "" };
        if scale == 0 {
            format!("{sign}{whole}")
        } else {
            format!("{sign}{whole}.{fraction}")
        }
    }

    fn clock(seconds: i64, micros: i64) -> String {
        let (h, m, s) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
        if micros == 0 {
            format!("{h:02}:{m:02}:{s:02}")
        } else {
            format!("{h:02}:{m:02}:{s:02}.{micros:06}")
        }
    }

    fn instant(micros: i64, zone: &str) -> Option<String> {
        let moment = OffsetDateTime::from_unix_timestamp_nanos(i128::from(micros) * 1000).ok()?;
        let base = format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
            moment.year(),
            u8::from(moment.month()),
            moment.day(),
            moment.hour(),
            moment.minute(),
            moment.second()
        );
        let fraction = micros.rem_euclid(1_000_000);
        Some(if fraction == 0 {
            format!("{base}{zone}")
        } else {
            format!("{base}.{fraction:06}{zone}")
        })
    }

    fn date(days: i32) -> Option<String> {
        let day = Date::from_julian_day(days.checked_add(2_440_588)?).ok()?;
        Some(format!(
            "{:04}-{:02}-{:02}",
            day.year(),
            u8::from(day.month()),
            day.day()
        ))
    }

    /// The branch of a union a value was read as, or the schema itself.
    fn branch(schema: &Schema, index: u32) -> &Schema {
        match schema {
            Schema::Union(union) => usize::try_from(index)
                .ok()
                .and_then(|at| union.variants().get(at))
                .unwrap_or(schema),
            other => other,
        }
    }

    /// A value as JSON (inside a record, array or map), numbers and decimals as the job writes
    /// them: a decimal is a JSON string here and in the job.
    fn json(value: &Value, schema: &Schema) -> Result<Json, Refusal> {
        Ok(match value {
            Value::Null => Json::Null,
            Value::Boolean(flag) => Json::Bool(*flag),
            Value::Int(n) => Json::Number(Number::from(*n)),
            Value::Long(n) => Json::Number(Number::from(*n)),
            Value::Float(n) => Number::from_f64(f64::from(*n)).map_or(Json::Null, Json::Number),
            Value::Double(n) => Number::from_f64(*n).map_or(Json::Null, Json::Number),
            Value::Union(index, inner) => json(inner, branch(schema, *index))?,
            Value::Array(items) => {
                let element = match schema {
                    Schema::Array(array) => &*array.items,
                    other => other,
                };
                Json::Array(
                    items
                        .iter()
                        .map(|item| json(item, element))
                        .collect::<Result<_, _>>()?,
                )
            }
            Value::Map(entries) => {
                let element = match schema {
                    Schema::Map(map) => &*map.types,
                    other => other,
                };
                let mut sorted: Vec<_> = entries.iter().collect();
                sorted.sort_by(|a, b| a.0.cmp(b.0));
                Json::Object(
                    sorted
                        .into_iter()
                        .map(|(key, item)| Ok((key.clone(), json(item, element)?)))
                        .collect::<Result<_, Refusal>>()?,
                )
            }
            Value::Record(fields) => {
                let schemas = match schema {
                    Schema::Record(record) => Some(&record.fields),
                    _ => None,
                };
                Json::Object(
                    fields
                        .iter()
                        .enumerate()
                        .map(|(at, (name, item))| {
                            let inner = schemas
                                .and_then(|list| list.get(at))
                                .map_or(schema, |field| &field.schema);
                            Ok((name.clone(), json(item, inner)?))
                        })
                        .collect::<Result<_, Refusal>>()?,
                )
            }
            Value::Bytes(_) | Value::Fixed(..) | Value::Duration(_) => {
                return Err(Refusal::AvroBinary);
            }
            other => Json::String(text(other, schema)?),
        })
    }

    /// A value as the text of a cell.
    pub(super) fn text(value: &Value, schema: &Schema) -> Result<String, Refusal> {
        Ok(match value {
            Value::Null => String::new(),
            Value::Boolean(flag) => if *flag { "true" } else { "false" }.to_owned(),
            Value::Int(n) => n.to_string(),
            Value::Long(n) => n.to_string(),
            Value::Float(n) => format!("{:?}", f64::from(*n)),
            Value::Double(n) => format!("{n:?}"),
            Value::String(s) | Value::Enum(_, s) => s.clone(),
            Value::Union(index, inner) => text(inner, branch(schema, *index))?,
            Value::Date(days) => date(*days).ok_or(Refusal::AvroInvalid)?,
            Value::TimestampMillis(n) => {
                instant(n.saturating_mul(1000), "+00:00").ok_or(Refusal::AvroInvalid)?
            }
            Value::TimestampMicros(n) => instant(*n, "+00:00").ok_or(Refusal::AvroInvalid)?,
            Value::TimestampNanos(n) => {
                instant(n.div_euclid(1000), "+00:00").ok_or(Refusal::AvroInvalid)?
            }
            Value::LocalTimestampMillis(n) => {
                instant(n.saturating_mul(1000), "").ok_or(Refusal::AvroInvalid)?
            }
            Value::LocalTimestampMicros(n) => instant(*n, "").ok_or(Refusal::AvroInvalid)?,
            Value::LocalTimestampNanos(n) => {
                instant(n.div_euclid(1000), "").ok_or(Refusal::AvroInvalid)?
            }
            Value::TimeMillis(n) => clock(i64::from(*n) / 1000, i64::from(*n) % 1000 * 1000),
            Value::TimeMicros(n) => clock(n / 1_000_000, n % 1_000_000),
            Value::Uuid(id) => id.to_string(),
            Value::Decimal(decimal) => {
                let scale = match schema {
                    Schema::Decimal(d) => d.scale,
                    _ => return Err(Refusal::AvroInvalid),
                };
                let bytes = <Vec<u8>>::try_from(decimal).map_err(|_err| Refusal::AvroInvalid)?;
                decimal_text(signed_be(&bytes).ok_or(Refusal::AvroInvalid)?, scale)
            }
            Value::Bytes(_) | Value::Fixed(..) | Value::Duration(_) | Value::BigDecimal(_) => {
                return Err(Refusal::AvroBinary);
            }
            nested @ (Value::Array(_) | Value::Map(_) | Value::Record(_)) => {
                json(nested, schema)?.to_string()
            }
        })
    }

    /// Whether a schema holds a value with no text form.
    pub(super) fn has_binary(schema: &Schema) -> bool {
        match schema {
            Schema::Bytes | Schema::Fixed(_) | Schema::Duration | Schema::BigDecimal => true,
            Schema::Array(array) => has_binary(&array.items),
            Schema::Map(map) => has_binary(&map.types),
            Schema::Union(union) => union.variants().iter().any(has_binary),
            Schema::Record(record) => record.fields.iter().any(|field| has_binary(&field.schema)),
            _ => false,
        }
    }

    /// The starting type of a column of this schema (decision 10).
    pub(super) fn declared(schema: &Schema) -> Declared {
        let plain = |column_type| Declared {
            column_type,
            date_format: None,
            timezone: None,
        };
        match schema {
            Schema::Union(union) => {
                let mut others = union
                    .variants()
                    .iter()
                    .filter(|v| !matches!(v, Schema::Null));
                match (others.next(), others.next()) {
                    (Some(only), None) => declared(only),
                    _ => plain(ColumnType::Text),
                }
            }
            Schema::Int | Schema::Long => plain(ColumnType::Integer),
            Schema::Float | Schema::Double => plain(ColumnType::Float),
            Schema::Boolean => plain(ColumnType::Boolean),
            Schema::Date => Declared {
                column_type: ColumnType::Date,
                date_format: Some(DateFormat::Iso),
                timezone: None,
            },
            Schema::TimestampMillis | Schema::TimestampMicros | Schema::TimestampNanos => {
                Declared {
                    column_type: ColumnType::Timestamp,
                    date_format: Some(DateFormat::Iso),
                    timezone: Some(true),
                }
            }
            Schema::LocalTimestampMillis
            | Schema::LocalTimestampMicros
            | Schema::LocalTimestampNanos => Declared {
                column_type: ColumnType::Timestamp,
                date_format: Some(DateFormat::Iso),
                timezone: Some(false),
            },
            Schema::Decimal(d) => {
                declared_from_parquet(&format!("decimal({}, {})", d.precision, d.scale))
            }
            _ => plain(ColumnType::Text),
        }
    }
}

/// Read the first rows of an Avro container.
///
/// `head_is_truncated` says the bytes are only the start of a longer file: a block cut by the
/// end of the bytes ends the sample, it is not an error.
///
/// # Errors
///
/// [`Refusal`] for bytes that are not an Avro container, a codec the upload does not read
/// (`PR #186 review BLOCKER 1`: only the null codec is read, because a compressed block is
/// inflated whole and without a limit), a top-level type that is not a record, a binary
/// column, no records, and more than [`crate::upload_parse::MAX_COLUMNS`] fields.
pub fn read_avro(
    bytes: &[u8],
    head_is_truncated: bool,
    max_rows: usize,
) -> Result<Parsed, Refusal> {
    use apache_avro::{Reader, Schema};

    if !bytes.starts_with(b"Obj\x01") {
        return Err(Refusal::AvroInvalid);
    }
    // `PR #186 review BLOCKER 1`: the null codec only. apache-avro 0.21 inflates a deflate
    // block with `miniz_oxide::inflate::decompress_to_vec`, which has no limit on the
    // output, so a 16 MiB head of a deflate file can ask for gigabytes of memory in the
    // API. The job's reader (fastavro) inflates the same way, so the file is refused on
    // both sides rather than read on one and refused on the other.
    if !matches!(avro_codec(bytes)?.as_deref(), None | Some("null")) {
        return Err(Refusal::AvroCodec);
    }
    let reader = Reader::new(bytes).map_err(|_err| Refusal::AvroInvalid)?;
    let schema = reader.writer_schema().clone();
    let Schema::Record(record) = &schema else {
        return Err(Refusal::AvroNotRecord);
    };
    if avro::has_binary(&schema) {
        return Err(Refusal::AvroBinary);
    }
    if record.fields.len() > crate::upload_parse::MAX_COLUMNS {
        return Err(Refusal::TooManyColumns);
    }
    let columns: Vec<String> = record
        .fields
        .iter()
        .map(|field| field.name.clone())
        .collect();
    if columns.is_empty() {
        return Err(Refusal::NoRows);
    }
    let declared: Vec<Declared> = record
        .fields
        .iter()
        .map(|field| avro::declared(&field.schema))
        .collect();
    let mut rows = Vec::new();
    let mut more = false;
    for item in reader {
        if rows.len() == max_rows {
            more = true;
            break;
        }
        match item {
            Ok(apache_avro::types::Value::Record(fields)) => {
                let mut by_name: HashMap<String, apache_avro::types::Value> =
                    fields.into_iter().collect();
                let mut row = Vec::with_capacity(columns.len());
                for field in &record.fields {
                    let value = by_name
                        .remove(&field.name)
                        .unwrap_or(apache_avro::types::Value::Null);
                    row.push(avro::text(&value, &field.schema)?);
                }
                rows.push(row);
            }
            Ok(_) => return Err(Refusal::AvroNotRecord),
            Err(_) if head_is_truncated => {
                more = true;
                break;
            }
            Err(_) => return Err(Refusal::AvroInvalid),
        }
    }
    if rows.is_empty() {
        return Err(Refusal::NoRows);
    }
    Ok(Parsed {
        columns,
        rows,
        declared: Some(declared),
        more: more || head_is_truncated,
    })
}

// ── gzip ─────────────────────────────────────────────────────────────────

/// The first `limit` decoded bytes of a gzip stream, and whether more follows. A compressed
/// prefix that ends inside the stream is fine as long as `limit` bytes (or the end of the
/// data) were reached.
///
/// # Errors
///
/// [`Refusal::NotGzip`] for a stream that does not decode before it yields anything, and
/// [`Refusal::GzipHoldsArchive`] for one that holds another archive.
pub fn gunzip_prefix<R: Read>(reader: R, limit: usize) -> Result<(Vec<u8>, bool), Refusal> {
    let mut decoder = flate2::read::MultiGzDecoder::new(reader);
    let mut out = Vec::new();
    let wanted = u64::try_from(limit).unwrap_or(u64::MAX).saturating_add(1);
    let result = (&mut decoder).take(wanted).read_to_end(&mut out);
    if result.is_err() && out.is_empty() {
        return Err(Refusal::NotGzip);
    }
    // An error after some bytes is a prefix cut inside the stream: what was decoded stands.
    let more = out.len() > limit || result.is_err();
    out.truncate(limit);
    if out.starts_with(b"PK\x03\x04") || out.starts_with(&[0x1f, 0x8b]) {
        return Err(Refusal::GzipHoldsArchive);
    }
    Ok((out, more))
}

/// Decode a whole gzip stream, discarding it, to count its bytes: refused the moment the
/// count passes `cap` (decision 2).
///
/// # Errors
///
/// [`Refusal::TooLargeUnpacked`] past the cap; [`Refusal::NotGzip`] for a stream that is
/// damaged or cut short.
pub fn count_unpacked_gzip<R: Read>(reader: R, cap: u64) -> Result<u64, Refusal> {
    count(
        flate2::read::MultiGzDecoder::new(reader),
        cap,
        Refusal::NotGzip,
    )
}

fn count<R: Read>(mut reader: R, cap: u64, damaged: Refusal) -> Result<u64, Refusal> {
    let mut buffer = vec![0u8; 64 * 1024];
    let mut total = 0u64;
    loop {
        let read = reader.read(&mut buffer).map_err(|_err| damaged)?;
        if read == 0 {
            return Ok(total);
        }
        total += u64::try_from(read).unwrap_or(u64::MAX);
        if total > cap {
            return Err(Refusal::TooLargeUnpacked);
        }
    }
}

// ── zip ──────────────────────────────────────────────────────────────────

/// A file of a zip that a load reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZipFileInfo {
    /// Its name inside the archive.
    pub name: String,
    /// Its uncompressed size as the archive declares it (not trusted: see
    /// [`count_unpacked_zip`]).
    pub size: u64,
}

/// The files of a zip and the one format they share.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZipListing {
    /// The files in the archive's order, folders and `__MACOSX/` forks left out.
    pub files: Vec<ZipFileInfo>,
    /// The format of every file.
    pub format: Format,
}

fn archive_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    [
        ".zip", ".gz", ".tgz", ".tar", ".bz2", ".xz", ".7z", ".rar", ".zst",
    ]
    .iter()
    .any(|suffix| lower.ends_with(suffix))
}

/// The end-of-central-directory record of a zip: how many entries its central directory
/// holds and how many bytes that directory takes.
struct ZipDirectory {
    entries: u64,
    bytes: u64,
}

/// `len` bytes of `reader` from `at`.
fn read_at<R: Read + Seek>(reader: &mut R, at: u64, len: usize) -> Result<Vec<u8>, Refusal> {
    reader
        .seek(SeekFrom::Start(at))
        .map_err(|_err| Refusal::NotZip)?;
    let mut out = vec![0u8; len];
    reader
        .read_exact(&mut out)
        .map_err(|_err| Refusal::NotZip)?;
    Ok(out)
}

/// The little-endian number at `at`.
fn u16_at(bytes: &[u8], at: usize) -> u64 {
    u64::from(u16::from_le_bytes([bytes[at], bytes[at + 1]]))
}

/// The little-endian number at `at`.
fn u32_at(bytes: &[u8], at: usize) -> u64 {
    u64::from(u32::from_le_bytes([
        bytes[at],
        bytes[at + 1],
        bytes[at + 2],
        bytes[at + 3],
    ]))
}

/// The little-endian number at `at`.
fn u64_at(bytes: &[u8], at: usize) -> u64 {
    let mut raw = [0u8; 8];
    raw.copy_from_slice(&bytes[at..at + 8]);
    u64::from_le_bytes(raw)
}

/// The counts a zip's central directory declares, read from the end of the file.
///
/// `PR #186 review BLOCKER 2`. `zip::ZipArchive::new` builds a record for every entry of
/// the central directory — folders included — before the caller can look at any of it, so
/// an archive of millions of folders would take the API's memory with it (in a blocking
/// task, which outlives the request's timeout). This reads the end-of-central-directory
/// record instead, and the zip64 one when the archive has it, so an archive that is too
/// big is refused before its directory is built.
///
/// # Errors
///
/// [`Refusal::NotZip`] for bytes that hold no end-of-central-directory record.
fn zip_directory<R: Read + Seek>(reader: &mut R) -> Result<ZipDirectory, Refusal> {
    const EOCD: &[u8; 4] = b"PK\x05\x06";
    const ZIP64_EOCD: &[u8; 4] = b"PK\x06\x06";
    const ZIP64_LOCATOR: &[u8; 4] = b"PK\x06\x07";
    const EOCD_LEN: usize = 22;
    const MAX_COMMENT: usize = 65_535;

    let len = reader
        .seek(SeekFrom::End(0))
        .map_err(|_err| Refusal::NotZip)?;
    if len < u64::try_from(EOCD_LEN).unwrap_or(u64::MAX) {
        return Err(Refusal::NotZip);
    }
    // The record is at the very end, after a comment of up to 64 KiB; the zip64 locator
    // that may point at the zip64 record sits just before it.
    let window = usize::try_from(len)
        .unwrap_or(usize::MAX)
        .min(EOCD_LEN + MAX_COMMENT + 20);
    let start = len.saturating_sub(u64::try_from(window).unwrap_or(u64::MAX));
    let tail = read_at(reader, start, window)?;
    // The last record whose own comment ends at the end of the file is the real one: a
    // comment may hold the signature itself.
    let mut found = None;
    let mut from = tail.len();
    while from >= 4 {
        from -= 1;
        if tail[from..].starts_with(EOCD) && from + EOCD_LEN <= tail.len() {
            let comment = usize::try_from(u16_at(&tail, from + 20)).unwrap_or_default();
            if from + EOCD_LEN + comment == tail.len() {
                found = Some(from);
                break;
            }
        }
    }
    let at = found.ok_or(Refusal::NotZip)?;
    let mut entries = u16_at(&tail, at + 10);
    let mut bytes = u32_at(&tail, at + 12);
    if entries == u64::from(u16::MAX) || bytes == u64::from(u32::MAX) {
        // A zip64 archive: the real counts are in the zip64 record, which the locator
        // before the end record points at. Without the locator the 32-bit counts stand,
        // and they are the sentinel: the archive is refused below.
        if at >= 20 && tail[at - 20..].starts_with(ZIP64_LOCATOR) {
            let record_at = u64_at(&tail, at - 20 + 8);
            let record = read_at(reader, record_at, 56)?;
            if record.starts_with(ZIP64_EOCD) {
                entries = u64_at(&record, 24);
                bytes = u64_at(&record, 32);
            }
        }
    }
    Ok(ZipDirectory { entries, bytes })
}

/// Refuse a zip whose central directory is bigger than the API will read.
///
/// `PR #186 review BLOCKER 2`: the one check every zip read goes through, before
/// `zip::ZipArchive::new` builds a record for each of its entries.
fn check_zip_directory<R: Read + Seek>(reader: &mut R) -> Result<(), Refusal> {
    let directory = zip_directory(reader)?;
    if directory.entries > MAX_ZIP_ENTRIES || directory.bytes > MAX_ZIP_CENTRAL_BYTES {
        return Err(Refusal::ZipTooManyEntries);
    }
    Ok(())
}

/// The indices of the files a load reads, in order.
fn file_indices<R: Read + Seek>(archive: &mut zip::ZipArchive<R>) -> Result<Vec<usize>, Refusal> {
    let mut found = Vec::new();
    for index in 0..archive.len() {
        let file = archive
            .by_index_raw(index)
            .map_err(|_err| Refusal::NotZip)?;
        if file.is_dir() || file.name().starts_with("__MACOSX/") {
            continue;
        }
        found.push(index);
    }
    Ok(found)
}

/// List a zip from its central directory (the reader fetches ranges; nothing is decoded).
///
/// # Errors
///
/// [`Refusal`] for a file that is not a zip, holds no files, more than [`MAX_ZIP_FILES`], an
/// encrypted file, an archive by name, files that are not all of one supported format, or a
/// central directory bigger than [`MAX_ZIP_ENTRIES`] entries ([`Refusal::ZipTooManyEntries`]).
pub fn zip_listing<R: Read + Seek>(mut reader: R) -> Result<ZipListing, Refusal> {
    check_zip_directory(&mut reader)?;
    let mut archive = zip::ZipArchive::new(reader).map_err(|_err| Refusal::NotZip)?;
    let indices = file_indices(&mut archive)?;
    if indices.is_empty() {
        return Err(Refusal::ZipEmpty);
    }
    if indices.len() > MAX_ZIP_FILES {
        return Err(Refusal::ZipTooMany);
    }
    let mut files = Vec::new();
    let mut format = None;
    for index in indices {
        let file = archive
            .by_index_raw(index)
            .map_err(|_err| Refusal::NotZip)?;
        if file.encrypted() {
            return Err(Refusal::ZipEncrypted);
        }
        if archive_name(file.name()) {
            return Err(Refusal::ZipNested);
        }
        let this = Format::of_extension(&extension(file.name())).ok_or(Refusal::ZipMixed)?;
        if *format.get_or_insert(this) != this {
            return Err(Refusal::ZipMixed);
        }
        files.push(ZipFileInfo {
            name: file.name().to_owned(),
            size: file.size(),
        });
    }
    Ok(ZipListing {
        files,
        format: format.ok_or(Refusal::ZipEmpty)?,
    })
}

/// The first `limit` decoded bytes of the `position`-th file of a listing, and whether more
/// follows.
///
/// # Errors
///
/// [`Refusal`] for a damaged archive or entry, [`Refusal::ZipNested`] for an entry that
/// starts like another archive, and [`Refusal::ZipTooManyEntries`] for a central directory
/// bigger than the API will read.
pub fn zip_file_prefix<R: Read + Seek>(
    mut reader: R,
    position: usize,
    limit: usize,
) -> Result<(Vec<u8>, bool), Refusal> {
    check_zip_directory(&mut reader)?;
    let mut archive = zip::ZipArchive::new(reader).map_err(|_err| Refusal::NotZip)?;
    let indices = file_indices(&mut archive)?;
    let index = *indices.get(position).ok_or(Refusal::ZipEmpty)?;
    let file = archive.by_index(index).map_err(|_err| Refusal::NotZip)?;
    let mut out = Vec::new();
    let wanted = u64::try_from(limit).unwrap_or(u64::MAX).saturating_add(1);
    file.take(wanted)
        .read_to_end(&mut out)
        .map_err(|_err| Refusal::NotZip)?;
    let more = out.len() > limit;
    out.truncate(limit);
    if out.starts_with(b"PK\x03\x04") || out.starts_with(&[0x1f, 0x8b]) {
        return Err(Refusal::ZipNested);
    }
    Ok((out, more))
}

/// The whole-file checks of an arriving gzip file: it decodes, and not past the cap.
///
/// # Errors
///
/// As [`count_unpacked_gzip`].
pub fn check_gzip<R: Read>(reader: R, cap: u64) -> Result<(), Refusal> {
    count_unpacked_gzip(reader, cap).map(|_total| ())
}

/// The whole-file checks of an arriving zip archive: its listing is acceptable and its files
/// decode to no more than the cap.
///
/// # Errors
///
/// As [`zip_listing`] and [`count_unpacked_zip`].
pub fn check_zip<R: Read + Seek>(mut reader: R, cap: u64) -> Result<(), Refusal> {
    zip_listing(&mut reader)?;
    count_unpacked_zip(&mut reader, cap).map(|_total| ())
}

/// Decode every file of a zip, discarding the bytes, to count them: refused when the
/// archive's declared sizes, or the bytes it really decodes, pass `cap` (decision 2).
///
/// # Errors
///
/// [`Refusal::TooLargeUnpacked`] past the cap; [`Refusal::NotZip`] for a damaged archive;
/// [`Refusal::ZipNested`] for an entry that starts like another archive;
/// [`Refusal::ZipTooManyEntries`] for a central directory bigger than the API will read.
pub fn count_unpacked_zip<R: Read + Seek>(mut reader: R, cap: u64) -> Result<u64, Refusal> {
    check_zip_directory(&mut reader)?;
    let mut archive = zip::ZipArchive::new(reader).map_err(|_err| Refusal::NotZip)?;
    let indices = file_indices(&mut archive)?;
    let mut declared = 0u64;
    for index in &indices {
        declared = declared.saturating_add(
            archive
                .by_index_raw(*index)
                .map_err(|_err| Refusal::NotZip)?
                .size(),
        );
    }
    if declared > cap {
        return Err(Refusal::TooLargeUnpacked);
    }
    let mut total = 0u64;
    for index in indices {
        let mut file = archive.by_index(index).map_err(|_err| Refusal::NotZip)?;
        let mut first = [0u8; 4];
        let seen = file.read(&mut first).map_err(|_err| Refusal::NotZip)?;
        if first[..seen].starts_with(b"PK\x03\x04") || first[..seen].starts_with(&[0x1f, 0x8b]) {
            return Err(Refusal::ZipNested);
        }
        total = total.saturating_add(u64::try_from(seen).unwrap_or(0));
        total = total.saturating_add(count(
            &mut file,
            cap.saturating_sub(total),
            Refusal::NotZip,
        )?);
        if total > cap {
            return Err(Refusal::TooLargeUnpacked);
        }
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::io::{Cursor, Write};
    use std::path::PathBuf;

    use super::*;

    fn fixture(name: &str) -> Vec<u8> {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../ops/fixtures/uploads/formats")
            .join(name);
        std::fs::read(path).unwrap()
    }

    /// The expected file of a fixture: `columns` and `rows` (`null` is an empty cell).
    fn expected(name: &str) -> (Vec<String>, Vec<Vec<String>>) {
        let text = String::from_utf8(fixture(&format!("{name}.expected.json"))).unwrap();
        let value: Value = serde_json::from_str(&text).unwrap();
        let columns = value["columns"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c.as_str().unwrap().to_owned())
            .collect();
        let rows = value["rows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| {
                row.as_array()
                    .unwrap()
                    .iter()
                    .map(|cell| cell.as_str().unwrap_or_default().to_owned())
                    .collect()
            })
            .collect();
        (columns, rows)
    }

    fn gzip(data: &[u8]) -> Vec<u8> {
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(data).unwrap();
        encoder.finish().unwrap()
    }

    #[test]
    fn json_and_json_lines_fixtures_give_the_expected_columns_and_rows() {
        for (name, lines) in [("orders.json", false), ("orders.jsonl", true)] {
            let parsed = read_json(&fixture(name), false, lines, 100).unwrap();
            let (columns, rows) = expected(name);
            assert_eq!(parsed.columns, columns, "{name}");
            assert_eq!(parsed.rows, rows, "{name}");
            assert!(!parsed.more);
        }
    }

    #[test]
    fn avro_fixture_gives_the_expected_rows_and_the_types_the_schema_declares() {
        let parsed = read_avro(&fixture("orders.avro"), false, 100).unwrap();
        let (columns, rows) = expected("orders.avro");
        assert_eq!(parsed.columns, columns);
        assert_eq!(parsed.rows, rows);
        let types: Vec<&str> = parsed
            .declared
            .unwrap()
            .iter()
            .map(|d| match d.column_type {
                ColumnType::Integer => "integer",
                ColumnType::Float => "float",
                ColumnType::Boolean => "boolean",
                ColumnType::Date => "date",
                _ => "text",
            })
            .collect();
        let want: Value = serde_json::from_slice(&fixture("orders.avro.expected.json")).unwrap();
        let want: Vec<&str> = want["declared"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t.as_str().unwrap())
            .collect();
        assert_eq!(types, want);
    }

    #[test]
    fn a_json_prefix_cut_in_an_element_or_a_line_shows_only_whole_ones() {
        let whole = fixture("orders.json");
        for cut in 1..whole.len() {
            match read_json(&whole[..cut], true, false, 100) {
                Ok(parsed) => assert!(
                    parsed.rows.len() <= 3 && (parsed.more || parsed.rows.len() == 3),
                    "cut at {cut}"
                ),
                Err(refusal) => assert!(
                    matches!(refusal, Refusal::JsonNotArray | Refusal::NoRows),
                    "cut at {cut}: {refusal:?}"
                ),
            }
        }
        let lines = fixture("orders.jsonl");
        for cut in 1..lines.len() {
            if let Ok(parsed) = read_json(&lines[..cut], true, true, 100) {
                // A row seen before a later row brought a new key is as wide as the keys known then.
                let (_, rows) = expected("orders.jsonl");
                for (got, want) in parsed.rows.iter().zip(&rows) {
                    assert_eq!(got[..], want[..got.len()], "cut at {cut}");
                }
            }
        }
    }

    #[test]
    fn the_sample_stops_at_the_bound_and_says_the_file_goes_on() {
        let parsed = read_json(&fixture("orders.json"), false, false, 2).unwrap();
        assert_eq!((parsed.rows.len(), parsed.more), (2, true));
        let parsed = read_json(&fixture("orders.jsonl"), false, true, 2).unwrap();
        assert_eq!((parsed.rows.len(), parsed.more), (2, true));
        let parsed = read_avro(&fixture("orders.avro"), false, 1).unwrap();
        assert_eq!((parsed.rows.len(), parsed.more), (1, true));
    }

    #[test]
    fn json_that_is_not_an_array_of_objects_is_refused_with_a_fixed_sentence() {
        let refuse = |bytes: &[u8], lines: bool| read_json(bytes, false, lines, 10).unwrap_err();
        assert_eq!(refuse(b"{\"a\": 1}", false), Refusal::JsonNotArray);
        assert_eq!(refuse(b"[1, 2]", false), Refusal::JsonNotObjects);
        assert_eq!(refuse(b"[{\"a\": 1}, ", false), Refusal::JsonInvalid);
        assert_eq!(refuse(b"[{\"a\": 1},]", false), Refusal::JsonInvalid);
        assert_eq!(refuse(b"[{\"a\": 1}] x", false), Refusal::JsonInvalid);
        assert_eq!(refuse(b"[]", false), Refusal::NoRows);
        assert_eq!(refuse(b"[{}]", false), Refusal::NoRows);
        assert_eq!(refuse(b"{\"a\": 1}\n[1]\n", true), Refusal::JsonNotObjects);
        assert_eq!(refuse(b"{\"a\": 1}\nnope\n", true), Refusal::JsonInvalid);
        assert_eq!(refuse(b"\n\n", true), Refusal::NoRows);
    }

    #[test]
    fn a_byte_order_mark_is_dropped_and_a_thousand_and_one_keys_are_refused() {
        let parsed = read_json(b"\xef\xbb\xbf[{\"a\": 1}]", false, false, 10).unwrap();
        assert_eq!(parsed.columns, ["a"]);
        let keys: Vec<String> = (0..1001).map(|i| format!("\"k{i}\": {i}")).collect();
        let wide = format!("[{{{}}}]", keys.join(","));
        assert_eq!(
            read_json(wide.as_bytes(), false, false, 10).unwrap_err(),
            Refusal::TooManyColumns
        );
    }

    #[test]
    fn nested_values_keep_their_key_order_as_compact_json() {
        let parsed = read_json(
            br#"[{"a": {"z": 1, "y": [1, {"k": null}]}, "b": true, "c": null}]"#,
            false,
            false,
            10,
        )
        .unwrap();
        assert_eq!(
            parsed.rows[0],
            [r#"{"z":1,"y":[1,{"k":null}]}"#, "true", ""]
        );
    }

    /// An Avro header (magic, then a map of two entries: the schema and the codec) and no blocks.
    fn avro_header(codec: &str) -> Vec<u8> {
        fn zigzag(n: usize) -> u8 {
            u8::try_from(n * 2).unwrap()
        }
        let mut out = b"Obj\x01".to_vec();
        out.push(zigzag(2));
        for (key, value) in [("avro.schema", "\"null\""), ("avro.codec", codec)] {
            out.push(zigzag(key.len()));
            out.extend_from_slice(key.as_bytes());
            out.push(zigzag(value.len()));
            out.extend_from_slice(value.as_bytes());
        }
        out.push(0);
        out.extend_from_slice(&[7u8; 16]);
        out
    }

    /// A zig-zag varint, as an Avro block header writes a count or a size.
    fn zigzag_varint(value: u64) -> Vec<u8> {
        let mut raw = value << 1;
        let mut out = Vec::new();
        loop {
            let byte = u8::try_from(raw & 0x7f).unwrap();
            raw >>= 7;
            if raw == 0 {
                out.push(byte);
                return out;
            }
            out.push(byte | 0x80);
        }
    }

    /// A deflate Avro container: the header names the codec, and its one block inflates to
    /// a megabyte of zeros out of a few hundred bytes (`PR #186 review BLOCKER 1`).
    fn deflate_avro() -> Vec<u8> {
        let mut out = avro_header("deflate");
        let mut encoder =
            flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(&vec![0u8; 1024 * 1024]).unwrap();
        let compressed = encoder.finish().unwrap();
        out.push(2); // one object
        out.extend_from_slice(&zigzag_varint(u64::try_from(compressed.len()).unwrap()));
        out.extend_from_slice(&compressed);
        out.extend_from_slice(&[7u8; 16]); // the sync marker again
        out
    }

    #[test]
    fn avro_that_is_not_readable_is_refused_and_only_the_null_codec_is_accepted() {
        assert_eq!(
            read_avro(b"hello", false, 10).unwrap_err(),
            Refusal::AvroInvalid
        );
        assert_eq!(read_avro(b"", false, 10).unwrap_err(), Refusal::AvroInvalid);
        // `PR #186 review BLOCKER 1`: deflate is refused with the rest, because the readers
        // inflate a whole block with no limit on the output.
        for codec in ["snappy", "zstandard", "bzip2", "xz", "deflate"] {
            assert_eq!(
                read_avro(&avro_header(codec), false, 10).unwrap_err(),
                Refusal::AvroCodec,
                "{codec}"
            );
        }
        // The codec the upload reads gets to the next check: this header's schema is not a
        // record.
        assert_eq!(
            read_avro(&avro_header("null"), false, 10).unwrap_err(),
            Refusal::AvroNotRecord
        );
        assert_eq!(
            avro_codec(&fixture("orders.avro")).unwrap().as_deref(),
            Some("null"),
            "fastavro writes the null codec by name"
        );
    }

    #[test]
    fn pr_186_review_blocker_1_a_deflate_avro_is_refused_and_never_inflated() {
        let file = deflate_avro();
        assert!(
            file.len() < 8 * 1024,
            "the file is a few KiB; its block would inflate to a megabyte"
        );
        assert_eq!(read_avro(&file, false, 10).unwrap_err(), Refusal::AvroCodec);
    }

    #[test]
    fn a_gzip_fixture_decodes_and_a_nested_archive_is_refused() {
        let (prefix, more) = gunzip_prefix(Cursor::new(fixture("orders.csv.gz")), 1000).unwrap();
        assert!(prefix.starts_with(b"id,name,price\n") && !more);
        let (cut, more) = gunzip_prefix(Cursor::new(fixture("orders.csv.gz")), 5).unwrap();
        assert_eq!((cut.as_slice(), more), (&b"id,na"[..], true));
        assert_eq!(
            gunzip_prefix(Cursor::new(b"not gzip".to_vec()), 10).unwrap_err(),
            Refusal::NotGzip
        );
        assert_eq!(
            gunzip_prefix(Cursor::new(gzip(&gzip(b"a\n1\n"))), 10).unwrap_err(),
            Refusal::GzipHoldsArchive
        );
        assert_eq!(
            gunzip_prefix(Cursor::new(gzip(&fixture("three.zip"))), 10).unwrap_err(),
            Refusal::GzipHoldsArchive
        );
    }

    #[test]
    fn a_gzip_bomb_is_counted_and_refused_never_truncated() {
        let bomb = gzip(&vec![b'a'; 5_000_000]);
        assert!(bomb.len() < 10_000);
        assert_eq!(
            count_unpacked_gzip(Cursor::new(bomb.clone()), 5_000_000).unwrap(),
            5_000_000
        );
        assert_eq!(
            count_unpacked_gzip(Cursor::new(bomb.clone()), 4_999_999).unwrap_err(),
            Refusal::TooLargeUnpacked
        );
        // The prefix reader decodes only what it was asked for, whatever the stream holds.
        let (prefix, more) = gunzip_prefix(Cursor::new(bomb), 100).unwrap();
        assert_eq!((prefix.len(), more), (100, true));
    }

    #[test]
    fn the_zip_fixture_lists_its_files_folders_left_out_and_reads_each() {
        let listing = zip_listing(Cursor::new(fixture("three.zip"))).unwrap();
        let (columns, _) = expected("three.zip");
        assert_eq!(columns, ["id", "name", "price"]);
        assert_eq!(listing.format, Format::Delimited);
        let names: Vec<&str> = listing.files.iter().map(|f| f.name.as_str()).collect();
        let want: Value = serde_json::from_slice(&fixture("three.zip.expected.json")).unwrap();
        let want: Vec<&str> = want["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n.as_str().unwrap())
            .collect();
        assert_eq!(names, want);
        let (second, more) = zip_file_prefix(Cursor::new(fixture("three.zip")), 1, 1000).unwrap();
        assert_eq!(
            (second.as_slice(), more),
            (&b"price,id,name\n0.25,3,Citra\n"[..], false)
        );
        assert_eq!(
            count_unpacked_zip(Cursor::new(fixture("three.zip")), 1000).unwrap(),
            35 + 27 + 23
        );
    }

    fn zipped(entries: &[(&str, &[u8])]) -> Vec<u8> {
        // The test archive is built by hand as stored entries: no writer feature is needed.
        let mut out = Vec::new();
        let mut central = Vec::new();
        for (name, data) in entries {
            let offset = u32::try_from(out.len()).unwrap();
            let crc = crc32(data);
            let size = u32::try_from(data.len()).unwrap();
            let name_len = u16::try_from(name.len()).unwrap();
            out.extend_from_slice(&[0x50, 0x4b, 3, 4, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
            out.extend_from_slice(&crc.to_le_bytes());
            out.extend_from_slice(&size.to_le_bytes());
            out.extend_from_slice(&size.to_le_bytes());
            out.extend_from_slice(&name_len.to_le_bytes());
            out.extend_from_slice(&[0, 0]);
            out.extend_from_slice(name.as_bytes());
            out.extend_from_slice(data);
            central.extend_from_slice(&[0x50, 0x4b, 1, 2, 20, 0, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
            central.extend_from_slice(&crc.to_le_bytes());
            central.extend_from_slice(&size.to_le_bytes());
            central.extend_from_slice(&size.to_le_bytes());
            central.extend_from_slice(&name_len.to_le_bytes());
            central.extend_from_slice(&[0; 12]);
            central.extend_from_slice(&offset.to_le_bytes());
            central.extend_from_slice(name.as_bytes());
        }
        let start = u32::try_from(out.len()).unwrap();
        let count = u16::try_from(entries.len()).unwrap();
        out.extend_from_slice(&central);
        out.extend_from_slice(&[0x50, 0x4b, 5, 6, 0, 0, 0, 0]);
        out.extend_from_slice(&count.to_le_bytes());
        out.extend_from_slice(&count.to_le_bytes());
        out.extend_from_slice(&u32::try_from(central.len()).unwrap().to_le_bytes());
        out.extend_from_slice(&start.to_le_bytes());
        out.extend_from_slice(&[0, 0]);
        out
    }

    fn crc32(data: &[u8]) -> u32 {
        let mut crc = 0xffff_ffffu32;
        for byte in data {
            crc ^= u32::from(*byte);
            for _ in 0..8 {
                crc = if crc & 1 == 1 {
                    (crc >> 1) ^ 0xedb8_8320
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }

    #[test]
    fn a_zip_holds_up_to_ten_files_of_one_format_and_nothing_nested() {
        let file = |name: String| (name, b"a\n1\n".to_vec());
        let ten: Vec<(String, Vec<u8>)> = (0..10).map(|i| file(format!("{i}.csv"))).collect();
        let pairs = |list: &[(String, Vec<u8>)]| {
            zipped(
                &list
                    .iter()
                    .map(|(n, d)| (n.as_str(), d.as_slice()))
                    .collect::<Vec<_>>(),
            )
        };
        assert_eq!(
            zip_listing(Cursor::new(pairs(&ten))).unwrap().files.len(),
            10
        );
        let mut eleven = ten;
        eleven.push(file("10.csv".to_owned()));
        assert_eq!(
            zip_listing(Cursor::new(pairs(&eleven))).unwrap_err(),
            Refusal::ZipTooMany
        );
        let mixed = zipped(&[("a.csv", b"a\n1\n"), ("b.json", b"[{}]")]);
        assert_eq!(
            zip_listing(Cursor::new(mixed)).unwrap_err(),
            Refusal::ZipMixed
        );
        let unknown = zipped(&[("a.csv", b"a\n1\n"), ("b.xlsx", b"PK")]);
        assert_eq!(
            zip_listing(Cursor::new(unknown)).unwrap_err(),
            Refusal::ZipMixed
        );
        let nested = zipped(&[("inner.zip", b"PK\x03\x04..")]);
        assert_eq!(
            zip_listing(Cursor::new(nested)).unwrap_err(),
            Refusal::ZipNested
        );
        let disguised = zipped(&[("disguised.csv", b"PK\x03\x04..")]);
        assert_eq!(
            count_unpacked_zip(Cursor::new(disguised), 1000).unwrap_err(),
            Refusal::ZipNested
        );
        let only_folder = zipped(&[("d/", b"")]);
        assert_eq!(
            zip_listing(Cursor::new(only_folder)).unwrap_err(),
            Refusal::ZipEmpty
        );
        assert_eq!(
            zip_listing(Cursor::new(b"not a zip".to_vec())).unwrap_err(),
            Refusal::NotZip
        );
        let forks = zipped(&[("__MACOSX/._a.csv", b"\0"), ("a.csv", b"a\n1\n")]);
        assert_eq!(zip_listing(Cursor::new(forks)).unwrap().files.len(), 1);
    }

    #[test]
    fn pr_186_review_blocker_2_a_zip_with_more_entries_than_the_cap_is_refused_before_it_is_read() {
        // `PR #186 review BLOCKER 2`: the central directory is built in memory before the
        // archive can be looked at, so an archive of folders is refused on the counts it
        // declares, without the directory being built.
        assert!(MAX_ZIP_ENTRIES >= u64::try_from(MAX_ZIP_FILES).unwrap());
        let folders: Vec<(String, Vec<u8>)> = (0..=MAX_ZIP_ENTRIES)
            .map(|i| (format!("d{i}/"), Vec::new()))
            .collect();
        let archive = zipped(
            &folders
                .iter()
                .map(|(name, data)| (name.as_str(), data.as_slice()))
                .collect::<Vec<_>>(),
        );
        assert_eq!(
            zip_listing(Cursor::new(archive.clone())).unwrap_err(),
            Refusal::ZipTooManyEntries
        );
        assert_eq!(
            count_unpacked_zip(Cursor::new(archive.clone()), 1_000_000).unwrap_err(),
            Refusal::ZipTooManyEntries
        );
        assert_eq!(
            zip_file_prefix(Cursor::new(archive), 0, 10).unwrap_err(),
            Refusal::ZipTooManyEntries
        );
        // One entry fewer is inside the cap: the archive is read and its folders are left
        // out, as a zip of files is.
        let fewer: Vec<(String, Vec<u8>)> = (0..MAX_ZIP_ENTRIES)
            .map(|i| (format!("d{i}/"), Vec::new()))
            .collect();
        let inside = zipped(
            &fewer
                .iter()
                .map(|(name, data)| (name.as_str(), data.as_slice()))
                .collect::<Vec<_>>(),
        );
        assert_eq!(
            zip_listing(Cursor::new(inside)).unwrap_err(),
            Refusal::ZipEmpty,
            "the folders are not files"
        );
    }

    #[test]
    fn a_zip_bomb_is_refused_by_the_bytes_it_really_decodes() {
        let big = vec![b'a'; 200_000];
        let archive = zipped(&[("a.csv", &big)]);
        assert_eq!(
            count_unpacked_zip(Cursor::new(archive.clone()), 200_000).unwrap(),
            200_000
        );
        assert_eq!(
            count_unpacked_zip(Cursor::new(archive), 199_999).unwrap_err(),
            Refusal::TooLargeUnpacked
        );
        let two = zipped(&[("a.csv", &big[..60_000]), ("b.csv", &big[..60_000])]);
        assert_eq!(
            count_unpacked_zip(Cursor::new(two), 100_000).unwrap_err(),
            Refusal::TooLargeUnpacked
        );
    }

    #[test]
    fn names_say_what_a_file_may_be() {
        assert_eq!(by_name("a.JSON"), ByName::Plain(Format::Json));
        assert_eq!(by_name("a.ndjson"), ByName::Plain(Format::JsonLines));
        assert_eq!(by_name("a.avro"), ByName::Plain(Format::Avro));
        assert_eq!(by_name("a.csv.gz"), ByName::Gzip(Some(Format::Delimited)));
        assert_eq!(by_name("a.json.gz"), ByName::Gzip(Some(Format::Json)));
        assert_eq!(by_name("a.jsonl.gz"), ByName::Gzip(Some(Format::JsonLines)));
        assert_eq!(
            by_name("a.avro.gz"),
            ByName::Gzip(None),
            "gzip holds text or JSON"
        );
        assert_eq!(by_name("data.gz"), ByName::Gzip(None));
        assert_eq!(by_name("a.zip"), ByName::Zip);
        assert_eq!(by_name("a.csv"), ByName::Other);
        assert_eq!(format_of_text(b"  [{}]"), Format::Json);
        assert_eq!(format_of_text(b"{\"a\":1}"), Format::JsonLines);
        assert_eq!(format_of_text(b"a,b\n"), Format::Delimited);
    }
}
