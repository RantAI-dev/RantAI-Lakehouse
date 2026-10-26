//! The column types of an uploaded file (`DATA-18`, plan AD3): the guess the
//! preview makes, the check of a person's choices, and the names the load will
//! give the columns. A pure function of its input: no file, clock or network.
//!
//! # Two readers of one set of rules
//!
//! The preview (here, Rust) and the load (`dagster/dispar_orchestrate/
//! upload_types.py`, Python, vectorised with `pyarrow`) apply the same rules,
//! pinned by the cases of `ops/fixtures/uploads/types/` which a test on each
//! side reads (ADR 0014 decision 3, extended to types). This side only ever asks
//! "does this value fit this type" ([`fits`]); the stored value is the job's.
//!
//! # The rules (feature page `upload-column-types.md`, decisions 2 to 5, 7, 8)
//!
//! - Seven types go on the wire: `text`, `integer` (64-bit), `decimal` (38
//!   digits, 9 after the point), `boolean`, `date`, `timestamp`, and `float`,
//!   which only a file that carries it (Parquet, Avro) starts with (decision 10).
//! - **Empty**: an empty or all-space cell is empty (null) in every typed column
//!   and never decides a guess. Text keeps an empty text.
//! - **Trimmed**: spaces around a typed value do not count (` 12 ` is `12`).
//! - **Integer**: a sign and digits. The GUESS also wants no leading zero, so
//!   `007` and postcodes stay text; a column a person makes Integer accepts
//!   `007` as 7. One past 64 bits does not fit.
//! - **Decimal**: digits with one `.` (`.5` is allowed, `5.` is not), at most 29
//!   before the point and 9 after it; `1,234` and `1e3` are not decimals. A whole
//!   number that does not fit 64 bits guesses Decimal.
//! - **Boolean**: `true`/`false`, `yes`/`no`, `y`/`n`, `t`/`f` in any case; a
//!   column a person makes Boolean also accepts `1` and `0`, which the guess calls
//!   integers.
//! - **Date**: ISO `YYYY-MM-DD`, or `d/m/yyyy` / `m/d/yyyy` with `/`, `-` or `.`
//!   (one kind in a value). When every value could be read either way, the guess
//!   asks ([`Guess::ask_date_order`]); values that contradict each other make the
//!   column text.
//! - **Timestamp**: a date as above, `T` or a space, `HH:MM`, optional `:SS`, an
//!   optional fraction of one to six digits, and an optional zone (`Z`, `+HH:MM`,
//!   `+HHMM`). A column whose values all have a zone holds instants (stored in
//!   UTC); none, local times; a mix is text.
//! - **Float**: the syntax of a floating-point number, `NaN` and `inf` included.
//!
//! # Names (decision 8)
//!
//! [`clean_names`] is `_column_names` of the job, so a person who edits nothing
//! gets what loading gave before. A name a person types must be lower-case
//! letters and digits in groups joined by single underscores, starting with a
//! letter, at most 128 characters, unique: stricter than the page's "letters,
//! digits and `_`" in one way, no doubled or trailing `_`, because dlt writes such
//! a name under another one (the rule the table names already follow).

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

/// Rows the guess reads (decision 1, *proposed*).
pub const SAMPLE_ROWS: usize = 10_000;

/// Bytes the guess reads if the rows do not come first (decision 1).
pub const SAMPLE_BYTES: usize = 16 * 1024 * 1024;

/// The failing values a check lists (decision 6) and a report keeps.
pub const MAX_LISTED: usize = 100;

/// The longest value a listing keeps, in characters.
pub const MAX_VALUE_CHARS: usize = 200;

/// The most digits before the point of a decimal (`decimal(38, 9)`).
const DECIMAL_INTEGER_DIGITS: usize = 29;

/// The most digits after the point of a decimal.
const DECIMAL_FRACTION_DIGITS: usize = 9;

/// The most columns a table may have (`SEC-17`); also `upload_parse::MAX_COLUMNS`.
const MAX_COLUMNS: usize = 1_000;

/// The longest column name, in characters.
const MAX_NAME_CHARS: usize = 128;

/// A column's type, as the wire and the job name it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ColumnType {
    /// Text, as the file has it.
    Text,
    /// A 64-bit whole number.
    Integer,
    /// An exact number: 38 digits, 9 of them after the point.
    Decimal,
    /// Yes or no.
    Boolean,
    /// A calendar date.
    Date,
    /// A date and a time, with or without a zone.
    Timestamp,
    /// A floating-point number, from a file that carries one.
    Float,
}

/// How a date is read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DateFormat {
    /// `YYYY-MM-DD`.
    Iso,
    /// Day first: `16/10/2026`.
    Dmy,
    /// Month first: `10/16/2026`.
    Mdy,
}

/// One column of a load, as a person confirmed it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ColumnSpec {
    /// The file's own name for the column (its header cell), for display and for
    /// the report. The load matches columns by position, not by this.
    pub source: String,
    /// The column's name in the table.
    pub name: String,
    /// Its type.
    #[serde(rename = "type")]
    pub column_type: ColumnType,
    /// How its dates are read: required for `date` and `timestamp`, refused
    /// for the others.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub date_format: Option<DateFormat>,
    /// For a `timestamp`: whether its values carry a zone (and are stored as
    /// instants in UTC). Required for `timestamp`, refused for the others.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timezone: Option<bool>,
}

/// What the guess concludes about one column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Guess {
    /// The type.
    #[serde(rename = "type")]
    pub column_type: ColumnType,
    /// How its dates are read, when the guess settled it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date_format: Option<DateFormat>,
    /// Every value could be read day first or month first: the person must say.
    pub ask_date_order: bool,
    /// For a timestamp: whether its values carry a zone.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timezone: Option<bool>,
}

impl Guess {
    const fn plain(column_type: ColumnType) -> Self {
        Self {
            column_type,
            date_format: None,
            ask_date_order: false,
            timezone: None,
        }
    }
}

// ── scanning helpers ─────────────────────────────────────────────────────

fn all_digits(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit())
}

/// A sign and digits: the integer syntax. Returns the value if it fits 64 bits.
fn parse_integer(value: &str) -> Option<i64> {
    let digits = value.strip_prefix(['+', '-']).unwrap_or(value);
    if !all_digits(digits) {
        return None;
    }
    value.strip_prefix('+').unwrap_or(value).parse::<i64>().ok()
}

fn has_leading_zero(digits: &str) -> bool {
    digits.len() > 1 && digits.starts_with('0')
}

/// The parts of a decimal: (integer digits, fraction digits), if the syntax is
/// a sign, digits, and optionally a `.` followed by digits (or a leading `.`).
fn decimal_parts(value: &str) -> Option<(&str, &str)> {
    let body = value.strip_prefix(['+', '-']).unwrap_or(value);
    match body.split_once('.') {
        None => all_digits(body).then_some((body, "")),
        Some((whole, fraction)) => {
            let whole_ok = whole.is_empty() || all_digits(whole);
            (whole_ok && all_digits(fraction)).then_some((whole, fraction))
        }
    }
}

fn decimal_fits(whole: &str, fraction: &str) -> bool {
    whole.trim_start_matches('0').len() <= DECIMAL_INTEGER_DIGITS
        && fraction.len() <= DECIMAL_FRACTION_DIGITS
}

fn boolean_word(value: &str) -> Option<bool> {
    match value.to_ascii_lowercase().as_str() {
        "true" | "yes" | "y" | "t" => Some(true),
        "false" | "no" | "n" | "f" => Some(false),
        _ => None,
    }
}

/// Floating-point syntax: digits with an optional `.` and exponent, or a
/// leading `.`; `inf`, `infinity` and `nan`, any case, with an optional sign.
fn is_float(value: &str) -> bool {
    let body = value.strip_prefix(['+', '-']).unwrap_or(value);
    let lower = body.to_ascii_lowercase();
    if matches!(lower.as_str(), "inf" | "infinity" | "nan") {
        return true;
    }
    let (mantissa, exponent) = match lower.split_once('e') {
        Some((m, e)) => (m, Some(e)),
        None => (lower.as_str(), None),
    };
    let mantissa_ok = match mantissa.split_once('.') {
        None => all_digits(mantissa),
        Some((whole, fraction)) => {
            (all_digits(whole) || whole.is_empty())
                && (all_digits(fraction) || fraction.is_empty())
                && !(whole.is_empty() && fraction.is_empty())
        }
    };
    let exponent_ok = exponent.is_none_or(|e| all_digits(e.strip_prefix(['+', '-']).unwrap_or(e)));
    mantissa_ok && exponent_ok
}

// ── dates and timestamps ─────────────────────────────────────────────────

fn days_in_month(year: u32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ => {
            if (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400) {
                29
            } else {
                28
            }
        }
    }
}

fn real_date(year: u32, month: u32, day: u32) -> bool {
    (1..=9999).contains(&year)
        && (1..=12).contains(&month)
        && day >= 1
        && day <= days_in_month(year, month)
}

fn number(text: &str, min_len: usize, max_len: usize) -> Option<u32> {
    (all_digits(text) && (min_len..=max_len).contains(&text.len()))
        .then(|| text.parse().ok())
        .flatten()
}

/// Whether `text` is a real date in this reading.
fn date_in(text: &str, format: DateFormat) -> bool {
    match format {
        DateFormat::Iso => {
            let mut parts = text.split('-');
            let (Some(y), Some(m), Some(d), None) =
                (parts.next(), parts.next(), parts.next(), parts.next())
            else {
                return false;
            };
            matches!(
                (number(y, 4, 4), number(m, 2, 2), number(d, 2, 2)),
                (Some(y), Some(m), Some(d)) if real_date(y, m, d)
            )
        }
        DateFormat::Dmy | DateFormat::Mdy => {
            let Some(separator) = text.chars().find(|c| matches!(c, '/' | '-' | '.')) else {
                return false;
            };
            let mut parts = text.split(separator);
            let (Some(a), Some(b), Some(y), None) =
                (parts.next(), parts.next(), parts.next(), parts.next())
            else {
                return false;
            };
            let (Some(a), Some(b), Some(y)) = (number(a, 1, 2), number(b, 1, 2), number(y, 4, 4))
            else {
                return false;
            };
            if format == DateFormat::Dmy {
                real_date(y, b, a)
            } else {
                real_date(y, a, b)
            }
        }
    }
}

/// The readings in which `text` is a date (a subset of the three).
fn date_readings(text: &str) -> Vec<DateFormat> {
    [DateFormat::Iso, DateFormat::Dmy, DateFormat::Mdy]
        .into_iter()
        .filter(|format| date_in(text, *format))
        .collect()
}

/// Split a timestamp into its date text, and whether it carries a zone, if its
/// time and zone are well formed.
fn split_timestamp(value: &str) -> Option<(&str, bool)> {
    let at = value.find(['T', ' '])?;
    let (date, rest) = (&value[..at], &value[at + 1..]);
    let rest = rest.trim_start();
    // The zone is the tail: `Z`, or a sign and `HH:MM` or `HHMM`.
    let (clock, zoned) = if let Some(clock) = rest.strip_suffix('Z') {
        (clock, true)
    } else if let Some(sign_at) = rest.rfind(['+', '-']) {
        let zone = &rest[sign_at + 1..];
        let zone_ok = match zone.len() {
            5 => {
                zone.as_bytes().get(2) == Some(&b':')
                    && number(&zone[..2], 2, 2).is_some_and(|h| h < 24)
                    && number(&zone[3..], 2, 2).is_some_and(|m| m < 60)
            }
            4 => {
                number(&zone[..2], 2, 2).is_some_and(|h| h < 24)
                    && number(&zone[2..], 2, 2).is_some_and(|m| m < 60)
            }
            _ => false,
        };
        if !zone_ok {
            return None;
        }
        (&rest[..sign_at], true)
    } else {
        (rest, false)
    };
    let (hm_s, fraction) = match clock.split_once('.') {
        Some((head, fraction)) => (head, Some(fraction)),
        None => (clock, None),
    };
    let mut parts = hm_s.split(':');
    let (Some(h), Some(m), s, None) = (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return None;
    };
    if !(number(h, 2, 2).is_some_and(|h| h < 24) && number(m, 2, 2).is_some_and(|m| m < 60)) {
        return None;
    }
    match (s, fraction) {
        (Some(s), _) if number(s, 2, 2).is_none_or(|s| s >= 60) => return None,
        (None, Some(_)) => return None,
        _ => {}
    }
    if let Some(fraction) = fraction
        && !(all_digits(fraction) && fraction.len() <= 6)
    {
        return None;
    }
    Some((date, zoned))
}

// ── the guess ────────────────────────────────────────────────────────────

/// Guess one column's type from its sample values (empty cells ignored).
#[must_use]
pub fn guess_column(values: &[&str]) -> Guess {
    let present: Vec<&str> = values
        .iter()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .collect();
    if present.is_empty() {
        return Guess::plain(ColumnType::Text);
    }
    let strict_integer = |value: &&str| {
        parse_integer(value).is_some()
            && !has_leading_zero(value.strip_prefix(['+', '-']).unwrap_or(value))
    };
    if present.iter().all(strict_integer) {
        return Guess::plain(ColumnType::Integer);
    }
    let strict_decimal = |value: &&str| {
        decimal_parts(value).is_some_and(|(whole, fraction)| {
            !has_leading_zero(whole) && decimal_fits(whole, fraction)
        })
    };
    if present.iter().all(strict_decimal) {
        return Guess::plain(ColumnType::Decimal);
    }
    if present.iter().all(|value| boolean_word(value).is_some()) {
        return Guess::plain(ColumnType::Boolean);
    }
    if let Some(found) = guess_dates(&present) {
        return found;
    }
    Guess::plain(ColumnType::Text)
}

/// Settle the date reading of `date_texts` (the one every value allows), or ask.
fn settle(readings: &[Vec<DateFormat>]) -> Option<(Option<DateFormat>, bool)> {
    let mut common: Vec<DateFormat> = vec![DateFormat::Iso, DateFormat::Dmy, DateFormat::Mdy];
    for each in readings {
        common.retain(|format| each.contains(format));
    }
    match common.as_slice() {
        [] => None,
        [only] => Some((Some(*only), false)),
        _ => Some((None, true)),
    }
}

fn guess_dates(present: &[&str]) -> Option<Guess> {
    // Dates alone.
    let readings: Vec<Vec<DateFormat>> = present.iter().map(|v| date_readings(v)).collect();
    if let Some((format, ask)) = settle(&readings) {
        return Some(Guess {
            column_type: ColumnType::Date,
            date_format: format,
            ask_date_order: ask,
            timezone: None,
        });
    }
    // Timestamps.
    let mut zone_flags = Vec::with_capacity(present.len());
    let mut readings = Vec::with_capacity(present.len());
    for value in present {
        let (date, zoned) = split_timestamp(value)?;
        zone_flags.push(zoned);
        readings.push(date_readings(date));
    }
    let zoned = zone_flags.first().copied()?;
    if zone_flags.iter().any(|flag| *flag != zoned) {
        return None;
    }
    let (format, ask) = settle(&readings)?;
    Some(Guess {
        column_type: ColumnType::Timestamp,
        date_format: format,
        ask_date_order: ask,
        timezone: Some(zoned),
    })
}

// ── does a value fit ─────────────────────────────────────────────────────

/// Whether `value` fits a column of this type, reading dates as `date_format`
/// and timestamps with or without a zone as `timezone`. An empty cell fits every
/// type (it is null); text fits everything.
#[must_use]
pub fn fits(
    column_type: ColumnType,
    date_format: Option<DateFormat>,
    timezone: bool,
    value: &str,
) -> bool {
    if column_type == ColumnType::Text {
        return true;
    }
    let value = value.trim();
    if value.is_empty() {
        return true;
    }
    match column_type {
        ColumnType::Text => true,
        ColumnType::Integer => parse_integer(value).is_some(),
        ColumnType::Decimal => {
            decimal_parts(value).is_some_and(|(whole, fraction)| decimal_fits(whole, fraction))
        }
        ColumnType::Boolean => boolean_word(value).is_some() || matches!(value, "1" | "0"),
        ColumnType::Float => is_float(value),
        ColumnType::Date => date_format.is_some_and(|format| date_in(value, format)),
        ColumnType::Timestamp => {
            let Some(format) = date_format else {
                return false;
            };
            split_timestamp(value)
                .is_some_and(|(date, zoned)| zoned == timezone && date_in(date, format))
        }
    }
}

// ── types that come with a file or a table ───────────────────────────────

/// A type a file or a table declares, with the reading its dates need.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Declared {
    /// The type.
    pub column_type: ColumnType,
    /// For a date or a timestamp: ISO, which is what Parquet's text and an Excel
    /// conversion write.
    pub date_format: Option<DateFormat>,
    /// For a timestamp: whether it holds instants.
    pub timezone: Option<bool>,
}

impl Declared {
    const fn of(column_type: ColumnType) -> Self {
        Self {
            column_type,
            date_format: None,
            timezone: None,
        }
    }
}

/// The starting type of a Parquet column, from the label the footer reading
/// gives it (`int64`, `decimal(10,2)`, `date`, `timestamp (us, UTC)` ...;
/// `upload_parquet::type_label`). Integers are integer (a `uint64` past 64 bits
/// will not fit and the check says so); a decimal that does not fit 38 digits
/// with 9 after the point, a time of day and a string are text; a float keeps its
/// floating-point type (decision 10).
#[must_use]
pub fn declared_from_parquet(label: &str) -> Declared {
    let label = label.trim();
    match label {
        "int8" | "int16" | "int32" | "int64" | "uint8" | "uint16" | "uint32" | "uint64" => {
            Declared::of(ColumnType::Integer)
        }
        "boolean" => Declared::of(ColumnType::Boolean),
        "float16" | "float32" | "float64" => Declared::of(ColumnType::Float),
        "date" => Declared {
            column_type: ColumnType::Date,
            date_format: Some(DateFormat::Iso),
            timezone: None,
        },
        _ if label.starts_with("timestamp") => Declared {
            column_type: ColumnType::Timestamp,
            date_format: Some(DateFormat::Iso),
            timezone: Some(label.contains("UTC")),
        },
        _ if label.starts_with("decimal(") => {
            let inner = label.trim_start_matches("decimal(").trim_end_matches(')');
            let mut parts = inner.split(',').map(|part| part.trim().parse::<usize>());
            match (parts.next(), parts.next()) {
                (Some(Ok(precision)), Some(Ok(scale)))
                    if scale <= DECIMAL_FRACTION_DIGITS
                        && precision.saturating_sub(scale) <= DECIMAL_INTEGER_DIGITS =>
                {
                    Declared::of(ColumnType::Decimal)
                }
                _ => Declared::of(ColumnType::Text),
            }
        }
        _ => Declared::of(ColumnType::Text),
    }
}

/// The type of a table's column, from the name `ClickHouse` gives it
/// (`Nullable(Int64)`, `Decimal(38, 9)`, `DateTime64(6, 'UTC')` ...). A type
/// outside the seven is text: adding to such a column is as text.
#[must_use]
pub fn declared_from_clickhouse(name: &str) -> Declared {
    let mut inner = name.trim();
    for wrapper in ["Nullable(", "LowCardinality("] {
        if let Some(rest) = inner.strip_prefix(wrapper) {
            inner = rest.strip_suffix(')').unwrap_or(rest).trim();
        }
    }
    if inner.starts_with("Int") || inner.starts_with("UInt") {
        return Declared::of(ColumnType::Integer);
    }
    if inner.starts_with("Float") {
        return Declared::of(ColumnType::Float);
    }
    if inner == "Bool" {
        return Declared::of(ColumnType::Boolean);
    }
    if inner == "Date" || inner == "Date32" {
        return Declared {
            column_type: ColumnType::Date,
            date_format: Some(DateFormat::Iso),
            timezone: None,
        };
    }
    if inner.starts_with("DateTime") {
        return Declared {
            column_type: ColumnType::Timestamp,
            date_format: Some(DateFormat::Iso),
            timezone: Some(inner.contains('\'')),
        };
    }
    if inner.starts_with("Decimal") {
        return Declared::of(ColumnType::Decimal);
    }
    Declared::of(ColumnType::Text)
}

// ── names ────────────────────────────────────────────────────────────────

/// The column names a load makes of a header: `_column_names` of the job
/// (`file_ingest.py`), proven by `names.json`. A blank cell is `col_<index>`, a
/// name starting with a digit gets `col_`, a repeat gets `_2`, `_3` (moving past
/// any name taken); only ASCII letters and digits are kept.
#[must_use]
pub fn clean_names(header: &[String]) -> Vec<String> {
    let mut names = Vec::with_capacity(header.len());
    let mut taken: HashSet<String> = HashSet::new();
    let mut seen: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for (index, cell) in header.iter().enumerate() {
        let mut base: String = cell
            .trim()
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() {
                    c.to_ascii_lowercase()
                } else {
                    '_'
                }
            })
            .collect();
        base = base.trim_matches('_').to_owned();
        while base.contains("__") {
            base = base.replace("__", "_");
        }
        if base.is_empty() {
            base = format!("col_{index}");
        } else if base.starts_with(|c: char| c.is_ascii_digit()) {
            base = format!("col_{base}");
        }
        let count = seen.entry(base.clone()).or_insert(0);
        *count += 1;
        let mut name = if *count == 1 {
            base.clone()
        } else {
            format!("{base}_{count}")
        };
        while taken.contains(&name) {
            let count = seen.entry(base.clone()).or_insert(0);
            *count += 1;
            name = format!("{base}_{count}");
        }
        taken.insert(name.clone());
        names.push(name);
    }
    names
}

/// The sentence for a name that breaks the rule.
pub const NAME_RULE: &str = "Column names start with a lower-case letter and use lower-case letters and digits joined by single underscores, with at most 128 characters, and each name is used once.";

fn name_ok(name: &str) -> bool {
    if name.is_empty() || name.chars().count() > MAX_NAME_CHARS || name.starts_with("_dlt") {
        return false;
    }
    let mut groups = name.split('_');
    let first_ok = groups.next().is_some_and(|g| {
        g.starts_with(|c: char| c.is_ascii_lowercase())
            && g.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
    });
    first_ok
        && groups.all(|g| {
            !g.is_empty()
                && g.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        })
}

/// Check a column list against a file of `width` columns.
///
/// # Errors
///
/// A fixed sentence: the wrong number of columns, a name that breaks
/// [`NAME_RULE`] or is used twice, a date type with no reading, a timestamp with
/// no zone choice, or a reading or zone choice on a type that has none.
pub fn validate_specs(specs: &[ColumnSpec], width: usize) -> Result<(), String> {
    if specs.len() != width || specs.len() > MAX_COLUMNS {
        return Err(format!(
            "The column list has {} columns but the file has {width}.",
            specs.len()
        ));
    }
    let mut seen: HashSet<&str> = HashSet::new();
    for spec in specs {
        if !name_ok(&spec.name) || !seen.insert(spec.name.as_str()) {
            return Err(NAME_RULE.to_owned());
        }
        if spec.source.chars().count() > 1_000 {
            return Err("A column's source name is too long.".to_owned());
        }
        let dated = matches!(spec.column_type, ColumnType::Date | ColumnType::Timestamp);
        if dated != spec.date_format.is_some() {
            return Err(
                "Date and timestamp columns need a date reading, and other columns have none."
                    .to_owned(),
            );
        }
        if (spec.column_type == ColumnType::Timestamp) != spec.timezone.is_some() {
            return Err(
                "Timestamp columns say whether they carry a time zone, and other columns do not."
                    .to_owned(),
            );
        }
    }
    Ok(())
}

// ── checking rows ────────────────────────────────────────────────────────

/// One value that does not fit its column.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BadCell {
    /// The data row, counted from 1 below the header.
    pub row: u64,
    /// The column's name in the table.
    pub column: String,
    /// The value, cut to [`MAX_VALUE_CHARS`] characters.
    pub value: String,
}

/// How many values fail in one column.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnCount {
    /// The column's name in the table.
    pub column: String,
    /// Failing values in it.
    pub count: u64,
}

/// The values of a sample that do not fit, as the check route and the load's
/// report both say them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BadValues {
    /// Failing values in all.
    pub count: u64,
    /// Failing values per column that has any.
    pub by_column: Vec<ColumnCount>,
    /// The first [`MAX_LISTED`] failing values, in file order.
    pub rows: Vec<BadCell>,
}

/// Apply a column list to the rows of a sample. A record shorter than the list
/// has empty cells for the rest, as the load pads it.
#[must_use]
pub fn check_rows(specs: &[ColumnSpec], rows: &[Vec<String>]) -> BadValues {
    let mut counts = vec![0_u64; specs.len()];
    let mut listed = Vec::new();
    for (index, record) in rows.iter().enumerate() {
        for (column, spec) in specs.iter().enumerate() {
            let value = record.get(column).map_or("", String::as_str);
            let fit = fits(
                spec.column_type,
                spec.date_format,
                spec.timezone.unwrap_or(false),
                value,
            );
            if fit {
                continue;
            }
            counts[column] += 1;
            if listed.len() < MAX_LISTED {
                listed.push(BadCell {
                    row: u64::try_from(index).unwrap_or(u64::MAX).saturating_add(1),
                    column: spec.name.clone(),
                    value: value.chars().take(MAX_VALUE_CHARS).collect(),
                });
            }
        }
    }
    BadValues {
        count: counts.iter().sum(),
        by_column: specs
            .iter()
            .zip(&counts)
            .filter(|(_, count)| **count > 0)
            .map(|(spec, count)| ColumnCount {
                column: spec.name.clone(),
                count: *count,
            })
            .collect(),
        rows: listed,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use serde_json::Value;

    use super::*;

    fn fixture(name: &str) -> Value {
        let path = format!(
            "{}/../../../ops/fixtures/uploads/types/{name}",
            env!("CARGO_MANIFEST_DIR")
        );
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    fn type_of(word: &str) -> ColumnType {
        serde_json::from_value(Value::String(word.to_owned())).unwrap()
    }

    fn format_of(value: &Value) -> Option<DateFormat> {
        value
            .as_str()
            .map(|word| serde_json::from_value(Value::String(word.to_owned())).unwrap())
    }

    #[test]
    fn the_guess_reaches_every_case_of_the_shared_fixture() {
        let cases = fixture("guess.json");
        for case in cases["cases"].as_array().unwrap() {
            let values: Vec<&str> = case["values"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap())
                .collect();
            let guess = guess_column(&values);
            let name = case["name"].as_str().unwrap();
            assert_eq!(
                guess.column_type,
                type_of(case["type"].as_str().unwrap()),
                "{name}"
            );
            assert_eq!(guess.date_format, format_of(&case["dateFormat"]), "{name}");
            assert_eq!(
                guess.ask_date_order,
                case["ask"].as_bool().unwrap(),
                "{name}"
            );
            assert_eq!(guess.timezone, case["timezone"].as_bool(), "{name}");
        }
    }

    #[test]
    fn a_value_fits_exactly_when_the_shared_fixture_says_so() {
        let cases = fixture("cast.json");
        let mut checked = 0;
        for case in cases["cases"].as_array().unwrap() {
            let column_type = type_of(case["type"].as_str().unwrap());
            let value = case["value"].as_str().unwrap();
            let got = fits(
                column_type,
                format_of(&case["dateFormat"]),
                case["timezone"].as_bool().unwrap_or(false),
                value,
            );
            assert_eq!(
                got,
                case["ok"].as_bool().unwrap(),
                "{column_type:?} {value:?}"
            );
            checked += 1;
        }
        assert!(checked > 80);
    }

    #[test]
    fn the_names_are_those_the_load_makes_of_the_header() {
        let cases = fixture("names.json");
        for case in cases["cases"].as_array().unwrap() {
            let header: Vec<String> = case["header"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap().to_owned())
                .collect();
            let names: Vec<&str> = case["names"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap())
                .collect();
            assert_eq!(clean_names(&header), names, "{header:?}");
        }
    }

    fn spec(name: &str, column_type: ColumnType) -> ColumnSpec {
        ColumnSpec {
            source: name.to_owned(),
            name: name.to_owned(),
            column_type,
            date_format: None,
            timezone: None,
        }
    }

    #[test]
    fn a_column_list_is_checked_for_count_names_and_the_readings_of_dates() {
        let ok = vec![
            spec("id", ColumnType::Integer),
            spec("note", ColumnType::Text),
        ];
        assert_eq!(validate_specs(&ok, 2), Ok(()));
        assert!(
            validate_specs(&ok, 3)
                .unwrap_err()
                .contains("2 columns but the file has 3")
        );
        for bad in [
            "",
            "Id",
            "1id",
            "a__b",
            "a_",
            "_a",
            "a-b",
            "_dlt_x",
            &"a".repeat(129),
        ] {
            let list = vec![spec(bad, ColumnType::Text)];
            assert_eq!(validate_specs(&list, 1).unwrap_err(), NAME_RULE, "{bad:?}");
        }
        let twice = vec![spec("a", ColumnType::Text), spec("a", ColumnType::Text)];
        assert_eq!(validate_specs(&twice, 2).unwrap_err(), NAME_RULE);
        let mut dated = spec("d", ColumnType::Date);
        assert!(
            validate_specs(&[dated.clone()], 1).is_err(),
            "a date with no reading"
        );
        dated.date_format = Some(DateFormat::Iso);
        assert_eq!(validate_specs(&[dated], 1), Ok(()));
        let mut stamp = spec("t", ColumnType::Timestamp);
        stamp.date_format = Some(DateFormat::Iso);
        assert!(
            validate_specs(&[stamp.clone()], 1).is_err(),
            "a timestamp with no zone choice"
        );
        stamp.timezone = Some(true);
        assert_eq!(validate_specs(&[stamp], 1), Ok(()));
        let mut stray = spec("n", ColumnType::Integer);
        stray.date_format = Some(DateFormat::Iso);
        assert!(
            validate_specs(&[stray], 1).is_err(),
            "a reading on an integer"
        );
    }

    #[test]
    fn the_check_counts_every_failing_value_and_lists_the_first_hundred() {
        let specs = vec![spec("n", ColumnType::Integer), spec("t", ColumnType::Text)];
        let mut rows: Vec<Vec<String>> = (0..150)
            .map(|i| vec![format!("x{i}"), "ok".to_owned()])
            .collect();
        rows.push(vec!["5".to_owned()]); // short record: the rest is empty
        rows.push(vec![String::new(), String::new()]);
        let bad = check_rows(&specs, &rows);
        assert_eq!(bad.count, 150);
        assert_eq!(
            bad.by_column,
            vec![ColumnCount {
                column: "n".into(),
                count: 150
            }]
        );
        assert_eq!(bad.rows.len(), MAX_LISTED);
        assert_eq!(
            bad.rows[0],
            BadCell {
                row: 1,
                column: "n".into(),
                value: "x0".into()
            }
        );
        let long = vec![vec!["y".repeat(500)]];
        assert_eq!(
            check_rows(&specs[..1], &long).rows[0].value.chars().count(),
            MAX_VALUE_CHARS
        );
    }

    #[test]
    fn a_files_and_a_tables_own_types_become_the_starting_types() {
        for (label, expected, zone) in [
            ("int64", ColumnType::Integer, None),
            ("uint8", ColumnType::Integer, None),
            ("boolean", ColumnType::Boolean, None),
            ("float64", ColumnType::Float, None),
            ("date", ColumnType::Date, None),
            ("timestamp (us, UTC)", ColumnType::Timestamp, Some(true)),
            ("timestamp (ms)", ColumnType::Timestamp, Some(false)),
            ("decimal(10,2)", ColumnType::Decimal, None),
            ("decimal(38,9)", ColumnType::Decimal, None),
            ("decimal(38,18)", ColumnType::Text, None),
            ("decimal(20,0)", ColumnType::Decimal, None),
            ("string", ColumnType::Text, None),
            ("time (us)", ColumnType::Text, None),
        ] {
            let got = declared_from_parquet(label);
            assert_eq!((got.column_type, got.timezone), (expected, zone), "{label}");
        }
        for (name, expected, zone) in [
            ("Nullable(Int64)", ColumnType::Integer, None),
            ("Nullable(Decimal(38, 9))", ColumnType::Decimal, None),
            ("Bool", ColumnType::Boolean, None),
            ("Nullable(Date32)", ColumnType::Date, None),
            (
                "Nullable(DateTime64(6, 'UTC'))",
                ColumnType::Timestamp,
                Some(true),
            ),
            (
                "Nullable(DateTime64(6))",
                ColumnType::Timestamp,
                Some(false),
            ),
            ("Nullable(Float64)", ColumnType::Float, None),
            ("Nullable(String)", ColumnType::Text, None),
            ("String", ColumnType::Text, None),
        ] {
            let got = declared_from_clickhouse(name);
            assert_eq!((got.column_type, got.timezone), (expected, zone), "{name}");
        }
    }

    #[test]
    fn a_wire_spec_round_trips_and_refuses_a_field_it_does_not_know() {
        let json = serde_json::json!({"source":"Due","name":"due","type":"timestamp","dateFormat":"dmy","timezone":false});
        let parsed: ColumnSpec = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(parsed.column_type, ColumnType::Timestamp);
        assert_eq!(serde_json::to_value(&parsed).unwrap(), json);
        let extra = serde_json::json!({"source":"a","name":"a","type":"text","cast":"x"});
        assert!(serde_json::from_value::<ColumnSpec>(extra).is_err());
        let unknown = serde_json::json!({"source":"a","name":"a","type":"time"});
        assert!(serde_json::from_value::<ColumnSpec>(unknown).is_err());
    }
}
