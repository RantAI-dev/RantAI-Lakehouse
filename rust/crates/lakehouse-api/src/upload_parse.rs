//! What an uploaded file is, and what reading it would produce: sniffing,
//! decoding and parsing of its first bytes, as pure functions — T5 of
//! `docs/superpowers/plans/2026-10-02-upload-file.md` (ADR 0014, decisions 3
//! and 4).
//!
//! Nothing here reads a file, a clock or the network, and nothing panics: a
//! caller hands in bytes it already holds. The routes (T6) read the first
//! chunk of the stored object and call [`sniff`] on upload and [`preview`]
//! when the user asks what a file looks like.
//!
//! # One dialect, two readers
//!
//! The preview (this module, Rust) and the load (`file_ingest_job`, Python,
//! T7) are two readers of one dialect, and a detection that differs from
//! what the load does is a table that looks right and is wrong (ADR 0014,
//! decision 3). The dialect:
//!
//! - **Decoding.** UTF-8, or UTF-16 whose byte order mark picks the byte
//!   order (no mark: little endian). A UTF-8 byte order mark is dropped,
//!   because left in place it sits in front of the first quote and turns
//!   that cell's quoting into literal quote characters. Bytes that are not
//!   valid UTF-8 become U+FFFD, visible and never fatal.
//! - **Fields.** Separated by one delimiter character. `"` opens a quoted
//!   field only at the start of a field; inside it, `""` is a literal
//!   quote and the delimiter and line breaks are ordinary characters, kept
//!   byte for byte. A quote anywhere else is an ordinary character. Text
//!   right after a closing quote stays in the field (`"abc"def` is
//!   `abcdef`), and a quote that is never closed takes the rest of the text
//!   into its field. Nothing here is an error: a malformed file still has a
//!   reading, and the user sees it before anything is loaded.
//! - **Records.** Ended by `\r\n`, `\n` or a lone `\r`, outside quotes. A
//!   line break right after a delimiter ends the record with an empty last
//!   field. An empty line is an empty record (no fields), and counts as a
//!   record for the header row's index. The line break at the very end of
//!   the text does not start another record.
//! - **Cells are not trimmed.** Whitespace is data. A record is *blank*,
//!   and left out of a preview's rows, when every cell is empty or only
//!   whitespace (see [`is_space`]).
//! - **Rows are the records as parsed.** A short record is not padded and a
//!   long one is not cut: that is a rule of the load, not of the dialect.
//!
//! This is `csv.reader(io.StringIO(text, newline=""), delimiter=d)` of
//! Python's standard library, without strict mode. The fixture files in
//! `ops/fixtures/uploads/` pin it: the tests below read every one of them,
//! and the load's tests (T7) read the same files, so the two cannot drift
//! apart unnoticed. `newline=""` matters: with the default, a lone `\r` is
//! an error, and with `newline=None` a `\r\n` inside a quoted cell becomes
//! `\n`.
//!
//! # Proposals, not decisions
//!
//! Detection is a proposal the user can correct (ADR 0014, decision 3).
//! [`preview`] reports what was detected AND what is in force, and the load
//! uses what the user confirmed.

use std::iter::Peekable;
use std::str::Chars;

use serde::Serialize;

/// How many records delimiter and header-row detection look at.
const SAMPLE_RECORDS: usize = 50;

/// How many leading bytes [`sniff`] looks at, whatever the caller hands in.
const SNIFF_BYTES: usize = 64 * 1024;

/// The delimiters a delimited-text upload may use, in the order detection
/// prefers them when two score the same: tab, comma, semicolon, pipe.
pub const DELIMITERS: [char; 4] = ['\t', ',', ';', '|'];

// ── sniffing ────────────────────────────────────────────────────────────

/// What a file is, judged from its first bytes alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Text to be read as delimited records. The only kind accepted.
    DelimitedText,
    /// A spreadsheet workbook: a zip archive (`.xlsx`) or an OLE compound
    /// file (`.xls`). Refused; a workbook is its own feature (ADR 0014,
    /// decision 4). A zip that is not a workbook is also reported here: the
    /// first bytes cannot tell them apart.
    Workbook,
    /// A Parquet file. The routes accept it when it is also named `.parquet`
    /// and refuse it otherwise; `upload_parquet` converts it (ADR 0014,
    /// amendment of 2026-10-08).
    Parquet,
    /// Another binary format (a PDF, a gzip stream), or bytes that are not
    /// text (NUL bytes in something that is not UTF-16). Refused.
    OtherBinary,
}

/// Judge what a file is from its leading bytes. Only the first
/// [`SNIFF_BYTES`] are looked at, so the answer does not depend on how much
/// of the file the caller passes. An empty input is [`Kind::DelimitedText`]:
/// there is no evidence against it, and the routes refuse an empty file
/// before they get here.
///
/// The name of a file and the type the browser claimed are not evidence: the
/// export that motivated this feature was called `.xls` and was UTF-16 text.
#[must_use]
pub fn sniff(head: &[u8]) -> Kind {
    let head = &head[..head.len().min(SNIFF_BYTES)];
    if head.starts_with(b"PK\x03\x04") || head.starts_with(&[0xD0, 0xCF, 0x11, 0xE0]) {
        return Kind::Workbook;
    }
    // `PAR1` is also four ordinary letters a text file can start with (a
    // header `PAR1,PAR2`). A Parquet file goes on with a binary page header,
    // whose first byte is a control byte (0x15 in files written by pyarrow
    // with each of three codecs, with and without dictionaries), so the magic
    // alone is not enough.
    if head.starts_with(b"PAR1") && head.get(4).is_some_and(|b| is_control(*b)) {
        return Kind::Parquet;
    }
    if head.starts_with(b"%PDF-") || head.starts_with(&[0x1F, 0x8B]) {
        return Kind::OtherBinary;
    }
    if detect_encoding(head) == Encoding::Utf8 && head.contains(&0) {
        return Kind::OtherBinary;
    }
    Kind::DelimitedText
}

/// A control byte: below space but not tab, CR or LF, or DEL.
fn is_control(byte: u8) -> bool {
    (byte < 0x20 && !matches!(byte, b'\t' | b'\n' | b'\r')) || byte == 0x7F
}

// ── encoding ────────────────────────────────────────────────────────────

/// A text encoding an upload may be read as. The wire form is the lowercase
/// name, also what the load's run config carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Encoding {
    /// UTF-8, with or without a byte order mark.
    #[serde(rename = "utf-8")]
    Utf8,
    /// UTF-16, either byte order, with or without a byte order mark.
    #[serde(rename = "utf-16")]
    Utf16,
}

impl Encoding {
    /// The wire form: `"utf-8"` or `"utf-16"`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Utf8 => "utf-8",
            Self::Utf16 => "utf-16",
        }
    }

    /// Parse the wire form. Exactly the two lowercase names; anything else
    /// is `None`, so a handler refuses it instead of guessing.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "utf-8" => Some(Self::Utf8),
            "utf-16" => Some(Self::Utf16),
            _ => None,
        }
    }
}

/// UTF-16 when the bytes carry a UTF-16 byte order mark or look like
/// UTF-16LE (ASCII text interleaved with NUL), UTF-8 otherwise.
///
/// The SAP export that motivated this feature is UTF-16LE with a BOM and
/// a `.xls` name: neither its extension nor its declared content type
/// says so.
#[must_use]
pub fn detect_encoding(bytes: &[u8]) -> Encoding {
    if bytes.starts_with(&[0xFF, 0xFE]) || bytes.starts_with(&[0xFE, 0xFF]) {
        return Encoding::Utf16;
    }
    let sample = &bytes[..bytes.len().min(512)];
    // A plain count, not `bytecount`: this runs once per preview over at
    // most 512 bytes, and adding a dependency for that would cost more
    // than it saves.
    let nuls = sample.iter().fold(0_usize, |n, b| n + usize::from(*b == 0));
    if sample.len() > 16 && nuls * 3 > sample.len() {
        return Encoding::Utf16;
    }
    Encoding::Utf8
}

/// Decode `bytes` as `encoding`. Never fails: a byte that does not belong
/// becomes U+FFFD, which stays visible in the table instead of failing a
/// 35,000-row export for one bad byte.
///
/// A UTF-8 byte order mark is dropped (see the module doc for why). UTF-16
/// follows its own mark, and assumes little endian without one; a trailing
/// odd byte is dropped.
#[must_use]
pub fn decode(bytes: &[u8], encoding: Encoding) -> String {
    match encoding {
        Encoding::Utf16 => {
            let (bytes, big_endian) = match bytes {
                [0xFF, 0xFE, rest @ ..] => (rest, false),
                [0xFE, 0xFF, rest @ ..] => (rest, true),
                other => (other, false),
            };
            let units: Vec<u16> = bytes
                .chunks_exact(2)
                .map(|pair| {
                    if big_endian {
                        u16::from_be_bytes([pair[0], pair[1]])
                    } else {
                        u16::from_le_bytes([pair[0], pair[1]])
                    }
                })
                .collect();
            String::from_utf16_lossy(&units)
        }
        Encoding::Utf8 => {
            let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF][..]).unwrap_or(bytes);
            String::from_utf8_lossy(bytes).into_owned()
        }
    }
}

// ── delimiter ───────────────────────────────────────────────────────────

/// Parse the wire form of a delimiter: exactly one of [`DELIMITERS`], as a
/// one-character string (a tab is the tab character itself). Anything else
/// is `None`.
#[must_use]
pub fn parse_delimiter(text: &str) -> Option<char> {
    let mut chars = text.chars();
    let delimiter = chars.next()?;
    (chars.next().is_none() && DELIMITERS.contains(&delimiter)).then_some(delimiter)
}

// ── records ─────────────────────────────────────────────────────────────

/// Whether `c` counts as whitespace for deciding that a cell is blank.
///
/// The set Python's `str.isspace()` uses: Rust's `char::is_whitespace` plus
/// U+001C to U+001F, which Python also counts. Spelled out so the preview
/// and the load blank the same rows.
#[must_use]
pub fn is_space(c: char) -> bool {
    c.is_whitespace() || matches!(c, '\u{1c}'..='\u{1f}')
}

fn is_blank_cell(cell: &str) -> bool {
    cell.chars().all(is_space)
}

/// A record is blank when it has no cell with anything but whitespace in it.
/// An empty record (an empty line) is blank.
fn is_blank(record: &[String]) -> bool {
    record.iter().all(|cell| is_blank_cell(cell))
}

/// Split `text` into records of fields, in the dialect the module doc
/// describes. Quote-aware: the delimiter and line breaks inside a quoted
/// field belong to the field.
///
/// `delimiter` must be one of [`DELIMITERS`] (see [`parse_delimiter`]); it
/// must not be `"`, `\n` or `\r`. Every cell is returned as it is in the
/// text, untrimmed.
#[must_use]
pub fn split_records(text: &str, delimiter: char) -> Vec<Vec<String>> {
    Records::new(text, delimiter).collect()
}

/// Where the reader is inside a record.
#[derive(Clone, Copy)]
enum State {
    /// At the start of a field: a quote here opens a quoted field.
    StartField,
    /// Inside an unquoted field (or after a closing quote's trailing text).
    Unquoted,
    /// Inside a quoted field.
    Quoted,
    /// Just read a quote inside a quoted field: a second one is a literal
    /// quote, anything else ends the quoted part.
    QuoteInQuoted,
}

/// The records of a text, one at a time, so detection can stop after the
/// few it needs.
struct Records<'a> {
    chars: Peekable<Chars<'a>>,
    delimiter: char,
}

impl<'a> Records<'a> {
    fn new(text: &'a str, delimiter: char) -> Self {
        Self {
            chars: text.chars().peekable(),
            delimiter,
        }
    }

    /// Having read `c`, a line break, consume the `\n` of a `\r\n`.
    fn end_of_line(&mut self, c: char) {
        if c == '\r' && self.chars.peek() == Some(&'\n') {
            self.chars.next();
        }
    }
}

impl Iterator for Records<'_> {
    type Item = Vec<String>;

    fn next(&mut self) -> Option<Vec<String>> {
        let first = *self.chars.peek()?;
        if first == '\n' || first == '\r' {
            // An empty line is an empty record.
            self.chars.next();
            self.end_of_line(first);
            return Some(Vec::new());
        }
        let mut fields: Vec<String> = Vec::new();
        let mut field = String::new();
        let mut state = State::StartField;
        loop {
            let Some(c) = self.chars.next() else {
                // The end of the text ends the record, whatever state it is
                // in: an unclosed quote keeps what it has taken.
                fields.push(field);
                return Some(fields);
            };
            let line_break = c == '\n' || c == '\r';
            match state {
                State::StartField | State::Unquoted if line_break => {
                    fields.push(field);
                    self.end_of_line(c);
                    return Some(fields);
                }
                State::StartField if c == '"' => state = State::Quoted,
                State::StartField | State::Unquoted if c == self.delimiter => {
                    fields.push(std::mem::take(&mut field));
                    state = State::StartField;
                }
                State::StartField | State::Unquoted => {
                    field.push(c);
                    state = State::Unquoted;
                }
                State::Quoted if c == '"' => state = State::QuoteInQuoted,
                State::Quoted => field.push(c),
                State::QuoteInQuoted if c == '"' => {
                    field.push('"');
                    state = State::Quoted;
                }
                State::QuoteInQuoted if c == self.delimiter => {
                    fields.push(std::mem::take(&mut field));
                    state = State::StartField;
                }
                State::QuoteInQuoted if line_break => {
                    fields.push(field);
                    self.end_of_line(c);
                    return Some(fields);
                }
                State::QuoteInQuoted => {
                    // Text after a closing quote stays in the field.
                    field.push(c);
                    state = State::Unquoted;
                }
            }
        }
    }
}

// ── detection ───────────────────────────────────────────────────────────

/// The most frequent value; on a tie the larger one, so the answer does not
/// depend on the order of the input.
fn modal(values: &[usize]) -> Option<usize> {
    values
        .iter()
        .copied()
        .max_by_key(|v| (values.iter().filter(|w| *w == v).count(), *v))
}

/// The delimiter that splits the first records most consistently, comma
/// when none does.
///
/// Consistency, not frequency: a description column full of commas makes
/// `,` the most COMMON character in a tab-separated export, but only the
/// real delimiter yields the same field count on line after line. A
/// delimiter inside quotes is not counted, because it is not a delimiter
/// there.
///
/// Looks at the first [`SAMPLE_RECORDS`] records that are not blank, so a
/// truncated final record only matters for a file whose head holds fewer
/// than that.
#[must_use]
pub fn detect_delimiter(text: &str) -> char {
    let mut best = (',', 0_usize);
    for candidate in DELIMITERS {
        let counts: Vec<usize> = Records::new(text, candidate)
            .filter(|record| !is_blank(record))
            .take(SAMPLE_RECORDS)
            .map(|record| record.len().saturating_sub(1))
            .filter(|delimiters| *delimiters > 0)
            .collect();
        let Some(modal) = modal(&counts) else {
            continue;
        };
        let agreeing = counts.iter().filter(|n| **n == modal).count();
        let score = agreeing * modal;
        if score > best.1 {
            best = (candidate, score);
        }
    }
    best.0
}

/// Index, among `records`, of the one that most plausibly holds column
/// names.
///
/// The first record with the modal field count and more than one cell that
/// is not blank. Report exports (SAP's "Dynamic List Display" among them)
/// open with title and date lines that have one field or a handful of stray
/// tabs; taking record 0 as the header there produces a table whose columns
/// are a report title. An index counts RECORDS, blank ones and ones that
/// span several lines included, the way the load counts them.
///
/// `0` when no record has more than one field.
#[must_use]
pub fn detect_header_row(records: &[Vec<String>]) -> usize {
    let counts: Vec<(usize, usize)> = records
        .iter()
        .enumerate()
        .take(SAMPLE_RECORDS)
        .map(|(index, record)| (index, record.len()))
        .filter(|(_, fields)| *fields > 1)
        .collect();
    let field_counts: Vec<usize> = counts.iter().map(|(_, fields)| *fields).collect();
    let Some(modal) = modal(&field_counts) else {
        return 0;
    };
    counts
        .iter()
        .find(|(index, fields)| {
            *fields == modal
                && records.get(*index).is_some_and(|record| {
                    record.iter().filter(|cell| !is_blank_cell(cell)).count() > 1
                })
        })
        .map_or(0, |(index, _)| *index)
}

// ── preview ─────────────────────────────────────────────────────────────

/// What the caller may override, each as the user chose it.
#[derive(Debug, Clone, Copy, Default)]
pub struct Overrides {
    /// Read the file as this encoding instead of the detected one.
    pub encoding: Option<Encoding>,
    /// Split on this delimiter instead of the detected one.
    pub delimiter: Option<char>,
    /// Take the column names from this record (zero-based) instead of the
    /// detected one.
    pub header_row: Option<usize>,
}

/// How a file is read: the three settings a load is told.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Reading {
    /// The text encoding.
    pub encoding: Encoding,
    /// The delimiter, serialized as a one-character string.
    pub delimiter: char,
    /// Zero-based index of the record that holds the column names.
    pub header_row: usize,
}

/// What reading a file's first bytes produces.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    /// What was detected. The delimiter and the header row are detected
    /// under the settings in force: a header guess means nothing under a
    /// delimiter the user has just rejected.
    pub detected: Reading,
    /// What is in force: the detected value unless the user overrode it.
    pub using: Reading,
    /// The header record's cells, untouched. Empty when the header row is
    /// past the end of what was read.
    pub columns: Vec<String>,
    /// The records below the header that are not blank, as parsed (a short
    /// or long record is not padded or cut), at most as many as asked for.
    pub rows: Vec<Vec<String>>,
    /// Whether the file holds more than `rows` shows: the head was cut, or
    /// the head held more rows than were asked for. Never the size of the
    /// file: a preview does not count what it did not read.
    pub truncated: bool,
}

/// Build the preview of a file from its first bytes.
///
/// `head_is_truncated` says the bytes are only the start of a longer file.
/// Its last record may then end mid-way (or mid-character), so it is
/// dropped before anything is detected or shown, instead of presenting half
/// a row as a whole one. `max_rows` bounds `rows`.
#[must_use]
pub fn preview(
    head: &[u8],
    head_is_truncated: bool,
    overrides: Overrides,
    max_rows: usize,
) -> Preview {
    let detected_encoding = detect_encoding(head);
    let encoding = overrides.encoding.unwrap_or(detected_encoding);
    let text = decode(head, encoding);

    let detected_delimiter = detect_delimiter(&text);
    let delimiter = overrides.delimiter.unwrap_or(detected_delimiter);

    let mut records = split_records(&text, delimiter);
    if head_is_truncated {
        records.pop();
    }

    let detected_header_row = detect_header_row(&records);
    let header_row = overrides.header_row.unwrap_or(detected_header_row);

    let columns = records.get(header_row).cloned().unwrap_or_default();
    // `saturating_add`: the header row comes from a query string and can be
    // `usize::MAX`.
    let mut below = records
        .iter()
        .skip(header_row.saturating_add(1))
        .filter(|record| !is_blank(record));
    let rows: Vec<Vec<String>> = below.by_ref().take(max_rows).cloned().collect();
    let more_rows = below.next().is_some();

    Preview {
        detected: Reading {
            encoding: detected_encoding,
            delimiter: detected_delimiter,
            header_row: detected_header_row,
        },
        using: Reading {
            encoding,
            delimiter,
            header_row,
        },
        columns,
        rows,
        truncated: head_is_truncated || more_rows,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use serde_json::{Value, json};

    use super::*;

    // ── helpers ─────────────────────────────────────────────────────────

    fn records(rows: &[&[&str]]) -> Vec<Vec<String>> {
        rows.iter()
            .map(|row| row.iter().map(|cell| (*cell).to_owned()).collect())
            .collect()
    }

    fn utf16le_with_bom(text: &str) -> Vec<u8> {
        let mut bytes = vec![0xFF, 0xFE];
        bytes.extend(text.encode_utf16().flat_map(u16::to_le_bytes));
        bytes
    }

    fn read(text: &str, overrides: Overrides) -> Preview {
        preview(text.as_bytes(), false, overrides, 20)
    }

    // ── sniff ───────────────────────────────────────────────────────────

    #[test]
    fn a_zip_or_an_ole_header_is_a_workbook() {
        assert_eq!(sniff(b"PK\x03\x04\x14\x00\x06\x00"), Kind::Workbook);
        assert_eq!(
            sniff(&[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1]),
            Kind::Workbook
        );
    }

    #[test]
    fn a_parquet_header_is_parquet_but_text_that_starts_with_par1_is_not() {
        // The first twelve bytes of a file written by pyarrow with its defaults
        // (snappy, dictionary pages) for a two-column table: the magic, then a
        // binary page header.
        assert_eq!(
            sniff(&[
                0x50, 0x41, 0x52, 0x31, 0x15, 0x04, 0x15, 0x30, 0x15, 0x2e, 0x4c, 0x15
            ]),
            Kind::Parquet
        );
        assert_eq!(sniff(b"PAR1\x15\x00\x15\x10\x15\x12"), Kind::Parquet);
        assert_eq!(sniff(b"PAR1\x00\x00\x00\x00"), Kind::Parquet);
        // A header row that happens to begin with the same four letters.
        assert_eq!(sniff(b"PAR1,PAR2,PAR3\n1,2,3\n"), Kind::DelimitedText);
        assert_eq!(sniff(b"PAR1;x\n"), Kind::DelimitedText);
        assert_eq!(sniff(b"PAR1\n1\n"), Kind::DelimitedText);
        assert_eq!(sniff(b"PAR1"), Kind::DelimitedText);
    }

    #[test]
    fn a_pdf_or_a_gzip_stream_is_another_binary_format() {
        assert_eq!(sniff(b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n"), Kind::OtherBinary);
        assert_eq!(
            sniff(&[0x1F, 0x8B, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00]),
            Kind::OtherBinary
        );
    }

    #[test]
    fn nul_bytes_in_text_that_is_not_utf16_make_it_binary() {
        assert_eq!(sniff(b"id,name\n1,a\0b\n"), Kind::OtherBinary);
        assert_eq!(sniff(&[0_u8; 8]), Kind::OtherBinary);
    }

    #[test]
    fn utf16_text_is_text_with_or_without_a_byte_order_mark() {
        let text = "id\tname\n1\talpha\n2\tbeta\n";
        assert_eq!(sniff(&utf16le_with_bom(text)), Kind::DelimitedText);

        // No mark: ASCII interleaved with NUL.
        let bare: Vec<u8> = text.encode_utf16().flat_map(u16::to_le_bytes).collect();
        assert!(bare.len() > 16 && bare.contains(&0));
        assert_eq!(sniff(&bare), Kind::DelimitedText);

        let mut big_endian = vec![0xFE, 0xFF];
        big_endian.extend(text.encode_utf16().flat_map(u16::to_be_bytes));
        assert_eq!(sniff(&big_endian), Kind::DelimitedText);
    }

    #[test]
    fn plain_text_and_an_empty_head_are_delimited_text() {
        assert_eq!(sniff(b"id,name\n1,a\n"), Kind::DelimitedText);
        assert_eq!(sniff(b""), Kind::DelimitedText);
        // Latin-1 text is not binary: it reads with replacement characters.
        assert_eq!(sniff(b"name\nRen\xe9\n"), Kind::DelimitedText);
        // A name that merely starts like a magic number is not one.
        assert_eq!(sniff(b"PK,name\n1,a\n"), Kind::DelimitedText);
        assert_eq!(sniff(b"%PDF,x\n"), Kind::DelimitedText);
    }

    #[test]
    fn only_the_leading_bytes_are_judged() {
        let mut late_nul = vec![b'a'; SNIFF_BYTES];
        late_nul.push(0);
        assert_eq!(sniff(&late_nul), Kind::DelimitedText);
        let mut early_nul = vec![b'a'; SNIFF_BYTES - 1];
        early_nul.push(0);
        assert_eq!(sniff(&early_nul), Kind::OtherBinary);
        assert_eq!(sniff(b"id,PK\x03\x04,x\n"), Kind::DelimitedText);
    }

    // ── encoding ────────────────────────────────────────────────────────

    #[test]
    fn detects_utf16_from_a_bom() {
        let bytes = utf16le_with_bom("a\tb");
        assert_eq!(detect_encoding(&bytes), Encoding::Utf16);
        assert_eq!(decode(&bytes, Encoding::Utf16), "a\tb");
    }

    #[test]
    fn detects_utf8_for_plain_ascii() {
        assert_eq!(detect_encoding(b"id,name\n1,two\n"), Encoding::Utf8);
    }

    #[test]
    fn utf16_without_a_bom_is_detected_from_its_nul_bytes_and_read_as_little_endian() {
        let text = "id\tname\n1\talpha\n";
        let bare: Vec<u8> = text.encode_utf16().flat_map(u16::to_le_bytes).collect();
        assert_eq!(detect_encoding(&bare), Encoding::Utf16);
        assert_eq!(decode(&bare, Encoding::Utf16), text);
    }

    #[test]
    fn utf16_big_endian_is_read_by_its_byte_order_mark() {
        let text = "id\tname\n1\talpha\n";
        let mut bytes = vec![0xFE, 0xFF];
        bytes.extend(text.encode_utf16().flat_map(u16::to_be_bytes));
        assert_eq!(detect_encoding(&bytes), Encoding::Utf16);
        assert_eq!(decode(&bytes, Encoding::Utf16), text);
    }

    #[test]
    fn an_odd_trailing_byte_of_utf16_is_dropped() {
        let mut bytes = utf16le_with_bom("ab");
        bytes.push(0x63);
        assert_eq!(decode(&bytes, Encoding::Utf16), "ab");
    }

    /// A byte order mark in front of a quoted first cell would turn that
    /// cell's quotes into literal characters, so the mark never reaches the
    /// parser.
    #[test]
    fn a_utf8_byte_order_mark_is_dropped_so_it_cannot_sit_in_front_of_a_quote() {
        let bytes = b"\xef\xbb\xbf\"id\",\"name\"\n1,a\n";
        assert_eq!(decode(bytes, Encoding::Utf8), "\"id\",\"name\"\n1,a\n");
        let shown = preview(bytes, false, Overrides::default(), 20);
        assert_eq!(shown.columns, ["id", "name"]);
        assert_eq!(shown.rows, records(&[&["1", "a"]]));
    }

    #[test]
    fn invalid_utf8_becomes_replacement_characters_not_an_error() {
        assert_eq!(
            decode(b"Ren\xe9,Z\xfcrich", Encoding::Utf8),
            "Ren\u{fffd},Z\u{fffd}rich"
        );
    }

    #[test]
    fn encoding_wire_names_round_trip_and_nothing_else_parses() {
        for encoding in [Encoding::Utf8, Encoding::Utf16] {
            assert_eq!(Encoding::parse(encoding.as_str()), Some(encoding));
        }
        for refused in [
            "", "UTF-8", "utf8", "utf_16", " utf-8", "latin-1", "utf-16le",
        ] {
            assert_eq!(Encoding::parse(refused), None, "{refused:?}");
        }
        assert_eq!(
            serde_json::to_value(Encoding::Utf16).unwrap(),
            json!("utf-16")
        );
    }

    // ── delimiter ───────────────────────────────────────────────────────

    #[test]
    fn parse_delimiter_accepts_exactly_the_four_delimiters_as_one_character() {
        for (text, delimiter) in [(",", ','), (";", ';'), ("\t", '\t'), ("|", '|')] {
            assert_eq!(parse_delimiter(text), Some(delimiter));
        }
        for refused in ["", ",,", " ", "\"", "\n", "\r", "tab", ":", "\t\t", ", "] {
            assert_eq!(parse_delimiter(refused), None, "{refused:?}");
        }
    }

    // ── whitespace ──────────────────────────────────────────────────────

    /// Every code point Python's `str.isspace()` reports (Python 3.12,
    /// enumerated over all of Unicode when this was written).
    const PYTHON_SPACES: [char; 29] = [
        '\u{9}', '\u{a}', '\u{b}', '\u{c}', '\u{d}', '\u{1c}', '\u{1d}', '\u{1e}', '\u{1f}',
        '\u{20}', '\u{85}', '\u{a0}', '\u{1680}', '\u{2000}', '\u{2001}', '\u{2002}', '\u{2003}',
        '\u{2004}', '\u{2005}', '\u{2006}', '\u{2007}', '\u{2008}', '\u{2009}', '\u{200a}',
        '\u{2028}', '\u{2029}', '\u{202f}', '\u{205f}', '\u{3000}',
    ];

    /// The preview and the load must blank the same rows, so the whitespace
    /// that blanks a cell is the set Python uses, not Rust's slightly
    /// smaller one.
    #[test]
    fn the_whitespace_that_blanks_a_cell_is_exactly_the_set_python_uses() {
        for code in 0..=u32::from(char::MAX) {
            if let Some(c) = char::from_u32(code) {
                assert_eq!(is_space(c), PYTHON_SPACES.contains(&c), "U+{code:04X}");
            }
        }
        assert!(is_blank(&records(&[&["", " ", "\u{1f}\t"]])[0]));
        assert!(!is_blank(&records(&[&["", "x"]])[0]));
        assert!(is_blank(&[]));
    }

    // ── split_records ───────────────────────────────────────────────────

    struct SplitCase {
        why: &'static str,
        text: &'static str,
        delimiter: char,
        records: &'static [&'static [&'static str]],
    }

    /// The expected records of each case are what Python's
    /// `csv.reader(io.StringIO(text, newline=""), delimiter=d)` returns for
    /// the same text. Check a row against it before changing it.
    const SPLIT_CASES: &[SplitCase] = &[
        SplitCase {
            why: "a final line break does not start another record",
            text: "a,b\n1,2\n",
            delimiter: ',',
            records: &[&["a", "b"], &["1", "2"]],
        },
        SplitCase {
            why: "a last record needs no line break",
            text: "a,b\n1,2",
            delimiter: ',',
            records: &[&["a", "b"], &["1", "2"]],
        },
        SplitCase {
            why: "empty text has no records",
            text: "",
            delimiter: ',',
            records: &[],
        },
        SplitCase {
            why: "one line break is one empty record",
            text: "\n",
            delimiter: ',',
            records: &[&[]],
        },
        SplitCase {
            why: "an empty line is an empty record",
            text: "a\n\nb\n",
            delimiter: ',',
            records: &[&["a"], &[], &["b"]],
        },
        SplitCase {
            why: "a line break after a delimiter leaves an empty last field",
            text: "a,\n,\n",
            delimiter: ',',
            records: &[&["a", ""], &["", ""]],
        },
        SplitCase {
            why: "a lone delimiter is two empty cells",
            text: ",",
            delimiter: ',',
            records: &[&["", ""]],
        },
        SplitCase {
            why: "CRLF ends a record once",
            text: "a,b\r\n1,2\r\n",
            delimiter: ',',
            records: &[&["a", "b"], &["1", "2"]],
        },
        SplitCase {
            why: "a lone CR ends a record",
            text: "a,b\r1,2\r",
            delimiter: ',',
            records: &[&["a", "b"], &["1", "2"]],
        },
        SplitCase {
            why: "LF then CR are two line breaks",
            text: "a\n\rb",
            delimiter: ',',
            records: &[&["a"], &[], &["b"]],
        },
        SplitCase {
            why: "a delimiter inside quotes belongs to the cell",
            text: "\"a,b\",c\n",
            delimiter: ',',
            records: &[&["a,b", "c"]],
        },
        SplitCase {
            why: "line breaks inside quotes are kept byte for byte",
            text: "\"a\nb\",\"c\r\nd\",\"e\rf\"\n",
            delimiter: ',',
            records: &[&["a\nb", "c\r\nd", "e\rf"]],
        },
        SplitCase {
            why: "a doubled quote is a literal quote",
            text: "\"x \"\"q\"\" y\",z\n",
            delimiter: ',',
            records: &[&["x \"q\" y", "z"]],
        },
        SplitCase {
            why: "an empty quoted cell is an empty cell",
            text: "\"\",x\n",
            delimiter: ',',
            records: &[&["", "x"]],
        },
        SplitCase {
            why: "a quote that does not start a cell is literal",
            text: "5\" pipe,x\"\"y\n",
            delimiter: ',',
            records: &[&["5\" pipe", "x\"\"y"]],
        },
        SplitCase {
            why: "a quote after a space is literal",
            text: "a, \"b,c\"\n",
            delimiter: ',',
            records: &[&["a", " \"b", "c\""]],
        },
        SplitCase {
            why: "text after a closing quote stays in the cell",
            text: "\"abc\"def,x\n",
            delimiter: ',',
            records: &[&["abcdef", "x"]],
        },
        SplitCase {
            why: "a quote never closed takes the rest of the text",
            text: "a,\"b\nc,d",
            delimiter: ',',
            records: &[&["a", "b\nc,d"]],
        },
        SplitCase {
            why: "a closing quote at the end of the text ends the record",
            text: "a,\"b\"",
            delimiter: ',',
            records: &[&["a", "b"]],
        },
        SplitCase {
            why: "cells are not trimmed",
            text: "  a ,\t b\t\n",
            delimiter: ',',
            records: &[&["  a ", "\t b\t"]],
        },
        SplitCase {
            why: "a tab delimiter with a quoted tab",
            text: "a\t\"b\tc\"\td\n",
            delimiter: '\t',
            records: &[&["a", "b\tc", "d"]],
        },
        SplitCase {
            why: "semicolon with a comma and a quoted semicolon",
            text: "a;b,c;\"d;e\"\n",
            delimiter: ';',
            records: &[&["a", "b,c", "d;e"]],
        },
        SplitCase {
            why: "pipe with a quote after the delimiter",
            text: "a|\"b|c\"|d\n",
            delimiter: '|',
            records: &[&["a", "b|c", "d"]],
        },
        SplitCase {
            why: "non-ASCII text is kept",
            text: "\u{e9},\u{65e5}\u{672c}\u{8a9e},\u{1f600}\n",
            delimiter: ',',
            records: &[&["\u{e9}", "\u{65e5}\u{672c}\u{8a9e}", "\u{1f600}"]],
        },
    ];

    #[test]
    fn split_records_follows_the_dialect_on_every_case() {
        for case in SPLIT_CASES {
            assert_eq!(
                split_records(case.text, case.delimiter),
                records(case.records),
                "{}: {:?}",
                case.why,
                case.text
            );
        }
    }

    #[test]
    fn a_very_long_cell_is_kept_whole() {
        let cell = "x".repeat(300_000);
        let text = format!("a,\"{cell}\"\n");
        assert_eq!(split_records(&text, ','), vec![vec!["a".to_owned(), cell]]);
    }

    // ── detection ───────────────────────────────────────────────────────

    #[test]
    fn delimiter_is_the_consistent_one_not_the_common_one() {
        // Every description holds commas; only the tab count is stable.
        let text = [
            "id\tdescription\tqty",
            "1\tBOLT, HEX, M6\t10",
            "2\tTAPE, WHITE, 50MM\t4",
            "3\tGLUE, FAST, 20G\t7",
        ]
        .join("\n");
        assert_eq!(detect_delimiter(&text), '\t');
    }

    /// A delimiter inside quotes is not a delimiter. Counted per line, as
    /// the first version of this module did, the commas of the quoted
    /// descriptions below vary from line to line (2, 4, 5, 2, 3) while the
    /// semicolons of the last column are steady (2 a line), and `;` wins.
    /// Read in the dialect, every line has two commas, and `,` wins.
    #[test]
    fn a_delimiter_inside_quotes_is_not_counted() {
        let text = [
            "id,description,tags",
            "1,\"BOLT, HEX, M6\",a;b;c",
            "2,\"TAPE, WHITE, 50MM, ROLL\",d;e;f",
            "3,\"GLUE\",g;h;i",
            "4,\"CLIP, STEEL\",j;k;l",
            "",
        ]
        .join("\n");
        assert_eq!(detect_delimiter(&text), ',');
    }

    /// The same for a line break inside quotes. Two cells below span two
    /// lines, and the second line of each carries commas that are part of
    /// the cell. Counted per line, the commas are inconsistent (2, 1, 4, 1,
    /// 4, 2, 2) and `;` wins; read in the dialect, there are five records
    /// with two commas each.
    #[test]
    fn a_line_break_inside_quotes_does_not_make_a_record_of_its_own() {
        let text = [
            "id,note,tags",
            "1,\"first line",
            "second, third, fourth, fifth\",a;b;c",
            "2,\"first line",
            "second, third, fourth, fifth\",d;e;f",
            "3,\"y\",g;h;i",
            "4,\"z\",j;k;l",
            "",
        ]
        .join("\n");
        assert_eq!(detect_delimiter(&text), ',');
        assert_eq!(split_records(&text, ',').len(), 5);
    }

    #[test]
    fn delimiter_defaults_to_comma_when_nothing_splits_the_text() {
        assert_eq!(detect_delimiter("just one column\nand another\n"), ',');
        assert_eq!(detect_delimiter(""), ',');
    }

    #[test]
    fn a_tie_between_delimiters_goes_to_the_earlier_candidate() {
        assert_eq!(detect_delimiter("a,b;c\n1,2;3\n"), ',');
        assert_eq!(detect_delimiter("a\tb;c\n1\t2;3\n"), '\t');
    }

    #[test]
    fn the_modal_value_is_the_most_frequent_and_the_larger_on_a_tie() {
        assert_eq!(modal(&[]), None);
        assert_eq!(modal(&[1, 2, 2]), Some(2));
        assert_eq!(modal(&[2, 2, 3, 3]), Some(3));
        assert_eq!(modal(&[3, 3, 2, 2]), Some(3));
    }

    #[test]
    fn header_row_skips_a_report_preamble() {
        // The shape of the SAP export this feature was built for.
        let text = [
            "24.09.2025",
            "",
            "Material master Interface for MES",
            "",
            "Plnt\tMaterial\tDescription",
            "8250\t0250161\tADHESIVE",
            "8250\t0483912\tPP BAND",
        ]
        .join("\n");
        assert_eq!(detect_header_row(&split_records(&text, '\t')), 4);
    }

    /// The index counts records, as the load does: a preamble cell that
    /// spans two lines is one record, so the header is record 2, not line 3.
    #[test]
    fn a_preamble_record_that_spans_two_lines_counts_once() {
        let text = "\"Title\nsubtitle\"\n\nPlnt\tMat\tDesc\n8250\t1\tx\n";
        assert_eq!(detect_header_row(&split_records(text, '\t')), 2);
    }

    #[test]
    fn no_record_with_more_than_one_field_means_row_zero() {
        assert_eq!(detect_header_row(&split_records("a\nb\nc\n", ',')), 0);
        assert_eq!(detect_header_row(&[]), 0);
    }

    // ── preview ─────────────────────────────────────────────────────────

    #[test]
    fn overrides_win_and_the_preview_reports_both_what_was_detected_and_what_is_used() {
        let text = "id;name\n1;alpha\n2;beta\n";
        let plain = read(text, Overrides::default());
        assert_eq!(
            plain.detected,
            Reading {
                encoding: Encoding::Utf8,
                delimiter: ';',
                header_row: 0
            }
        );
        assert_eq!(plain.using, plain.detected);
        assert_eq!(plain.columns, ["id", "name"]);

        let wrong_delimiter = read(
            text,
            Overrides {
                delimiter: Some(','),
                ..Overrides::default()
            },
        );
        assert_eq!(wrong_delimiter.detected.delimiter, ';');
        assert_eq!(wrong_delimiter.using.delimiter, ',');
        assert_eq!(wrong_delimiter.columns, ["id;name"]);
        assert_eq!(wrong_delimiter.rows, records(&[&["1;alpha"], &["2;beta"]]));

        let second_row = read(
            text,
            Overrides {
                header_row: Some(1),
                ..Overrides::default()
            },
        );
        assert_eq!(second_row.detected.header_row, 0);
        assert_eq!(second_row.using.header_row, 1);
        assert_eq!(second_row.columns, ["1", "alpha"]);
        assert_eq!(second_row.rows, records(&[&["2", "beta"]]));

        let as_utf16 = read(
            text,
            Overrides {
                encoding: Some(Encoding::Utf16),
                ..Overrides::default()
            },
        );
        assert_eq!(as_utf16.detected.encoding, Encoding::Utf8);
        assert_eq!(as_utf16.using.encoding, Encoding::Utf16);
    }

    #[test]
    fn rows_are_limited_and_truncated_says_the_file_holds_more() {
        let text = "n\n1\n2\n3\n4\n5\n";
        let count = |max_rows| preview(text.as_bytes(), false, Overrides::default(), max_rows);

        let some = count(2);
        assert_eq!(some.rows, records(&[&["1"], &["2"]]));
        assert!(some.truncated);

        let all = count(5);
        assert_eq!(all.rows.len(), 5);
        assert!(
            !all.truncated,
            "exactly as many rows as asked for is not more"
        );

        assert!(count(100).rows.len() == 5 && !count(100).truncated);
        assert!(count(0).rows.is_empty() && count(0).truncated);

        // Blank records after the last real row are not "more".
        let trailing = preview(b"n\n1\n\n \n", false, Overrides::default(), 1);
        assert_eq!(trailing.rows, records(&[&["1"]]));
        assert!(!trailing.truncated);
    }

    #[test]
    fn blank_records_are_left_out_of_the_rows_but_counted_by_the_header_index() {
        let text = "Report\n\n\nid,name\n1,a\n\n \n,\n2,b\n";
        let shown = read(text, Overrides::default());
        assert_eq!(shown.using.header_row, 3);
        assert_eq!(shown.columns, ["id", "name"]);
        assert_eq!(shown.rows, records(&[&["1", "a"], &["2", "b"]]));
    }

    /// A head that is only the start of a longer file ends mid-record: that
    /// record is dropped, not shown as a short row.
    #[test]
    fn a_truncated_head_drops_its_last_record() {
        let head = "id,name\n1,alpha\n2,beta\n3,gam";
        let cut = preview(head.as_bytes(), true, Overrides::default(), 20);
        assert_eq!(cut.rows, records(&[&["1", "alpha"], &["2", "beta"]]));
        assert!(cut.truncated);

        // Told the head is the whole file, the half row is shown as a row.
        let whole = preview(head.as_bytes(), false, Overrides::default(), 20);
        assert_eq!(whole.rows.last().unwrap(), &["3", "gam"]);
        assert!(!whole.truncated);
    }

    #[test]
    fn a_head_cut_inside_a_quoted_cell_drops_the_record_it_opened() {
        let head = "id,note\n1,\"fine\"\n2,\"first line\nsecond li";
        let cut = preview(head.as_bytes(), true, Overrides::default(), 20);
        assert_eq!(cut.columns, ["id", "note"]);
        assert_eq!(cut.rows, records(&[&["1", "fine"]]));
    }

    #[test]
    fn a_head_cut_in_the_middle_of_a_character_does_not_show_the_damage() {
        let mut head = "id,name\n1,Ren".as_bytes().to_vec();
        head.extend("é".as_bytes());
        head.pop();
        let cut = preview(&head, true, Overrides::default(), 20);
        assert!(cut.rows.is_empty(), "{cut:?}");
        assert!(cut.truncated);
    }

    #[test]
    fn a_header_row_past_the_end_has_no_columns_and_no_rows() {
        for header_row in [5, 99, usize::MAX - 1, usize::MAX] {
            let shown = read(
                "id,name\n1,a\n",
                Overrides {
                    header_row: Some(header_row),
                    ..Overrides::default()
                },
            );
            assert!(shown.columns.is_empty(), "{header_row}");
            assert!(shown.rows.is_empty(), "{header_row}");
            assert_eq!(shown.using.header_row, header_row);
            assert!(!shown.truncated);
        }
    }

    #[test]
    fn nothing_to_read_gives_an_empty_preview() {
        let shown = preview(b"", false, Overrides::default(), 20);
        assert!(shown.columns.is_empty() && shown.rows.is_empty() && !shown.truncated);
        assert_eq!(shown.using.delimiter, ',');
        let shown = preview(b"", true, Overrides::default(), 20);
        assert!(shown.truncated);
    }

    /// What T6 puts on the wire: camelCase names, the encoding as its wire
    /// name, the delimiter as a one-character string.
    #[test]
    fn the_preview_serializes_with_camel_case_names_and_a_one_character_delimiter() {
        let shown = preview(
            &utf16le_with_bom("a\tb\n1\t2\n"),
            false,
            Overrides::default(),
            20,
        );
        let body = serde_json::to_value(&shown).unwrap();
        assert_eq!(
            body["using"],
            json!({ "encoding": "utf-16", "delimiter": "\t", "headerRow": 0 })
        );
        assert_eq!(body["detected"], body["using"]);
        assert_eq!(body["columns"], json!(["a", "b"]));
        assert_eq!(body["rows"], json!([["1", "2"]]));
        assert_eq!(body["truncated"], json!(false));
    }

    // ── the shared fixtures ─────────────────────────────────────────────
    //
    // `ops/fixtures/uploads/` is read by these tests and, in T7, by the
    // load's. Read here with `include_bytes!`/`include_str!` from test code
    // only, so the release build of the API depends on nothing outside
    // `rust/`.

    const FIXTURE_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../ops/fixtures/uploads");

    struct Fixture {
        file: &'static str,
        bytes: &'static [u8],
        expected: &'static str,
    }

    macro_rules! fixture {
        ($stem:literal, $ext:literal) => {
            Fixture {
                file: concat!($stem, ".", $ext),
                bytes: include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../../ops/fixtures/uploads/",
                    $stem,
                    ".",
                    $ext
                )),
                expected: include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../../ops/fixtures/uploads/",
                    $stem,
                    ".expected.json"
                )),
            }
        };
    }

    const FIXTURES: &[Fixture] = &[
        fixture!("quoted_comma_newline", "csv"),
        fixture!("crlf_utf8_bom", "csv"),
        fixture!("semicolon", "csv"),
        fixture!("pipe", "txt"),
        fixture!("sap_report_utf16", "xls"),
        fixture!("duplicate_blank_headers", "csv"),
        fixture!("ragged_rows", "csv"),
        fixture!("cr_only", "csv"),
        fixture!("latin1_bytes_in_utf8", "csv"),
        fixture!("malformed_quotes", "csv"),
        // What `upload_workbook` writes for the `Quirks` sheet of the
        // workbooks in `ops/fixtures/workbooks/` (X1 of the Excel plan).
        fixture!("converted_sheet", "csv"),
    ];

    struct Expected {
        reading: Reading,
        columns: Vec<String>,
        rows: Vec<Vec<String>>,
    }

    fn expected_of(fixture: &Fixture) -> Expected {
        let json: Value = serde_json::from_str(fixture.expected).unwrap();
        let mut keys: Vec<&str> = json
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            ["columns", "delimiter", "encoding", "headerRow", "rows"],
            "{}: the expected JSON has exactly these keys",
            fixture.file
        );
        let strings = |value: &Value| -> Vec<String> {
            value
                .as_array()
                .unwrap()
                .iter()
                .map(|cell| cell.as_str().unwrap().to_owned())
                .collect()
        };
        Expected {
            reading: Reading {
                encoding: Encoding::parse(json["encoding"].as_str().unwrap()).unwrap(),
                delimiter: parse_delimiter(json["delimiter"].as_str().unwrap()).unwrap(),
                header_row: usize::try_from(json["headerRow"].as_u64().unwrap()).unwrap(),
            },
            columns: strings(&json["columns"]),
            rows: json["rows"]
                .as_array()
                .unwrap()
                .iter()
                .map(strings)
                .collect(),
        }
    }

    fn told(reading: Reading) -> Overrides {
        Overrides {
            encoding: Some(reading.encoding),
            delimiter: Some(reading.delimiter),
            header_row: Some(reading.header_row),
        }
    }

    /// T5's acceptance: every fixture parses to its expected JSON, by
    /// detection alone and again with the detected settings given back as
    /// the user's choice, and is recognised as delimited text.
    #[test]
    fn every_fixture_parses_to_its_expected_json() {
        for fixture in FIXTURES {
            let expected = expected_of(fixture);
            let found = preview(fixture.bytes, false, Overrides::default(), usize::MAX);
            assert_eq!(
                found.detected, expected.reading,
                "{}: detection",
                fixture.file
            );
            assert_eq!(found.using, expected.reading, "{}: in force", fixture.file);
            assert_eq!(found.columns, expected.columns, "{}: columns", fixture.file);
            assert_eq!(found.rows, expected.rows, "{}: rows", fixture.file);
            assert!(!found.truncated, "{}", fixture.file);

            let confirmed = preview(fixture.bytes, false, told(expected.reading), usize::MAX);
            assert_eq!(confirmed, found, "{}: confirmed settings", fixture.file);

            assert_eq!(
                sniff(fixture.bytes),
                Kind::DelimitedText,
                "{}",
                fixture.file
            );
        }
    }

    /// Cut a fixture anywhere and call it a truncated head: nothing panics,
    /// and what is shown is a prefix of what the whole file shows, never a
    /// half row presented as a whole one.
    #[test]
    fn a_cut_head_shows_a_prefix_of_what_the_whole_file_shows() {
        for fixture in FIXTURES {
            let expected = expected_of(fixture);
            for len in 0..=fixture.bytes.len() {
                let head = &fixture.bytes[..len];
                let _ = sniff(head);
                let cut = preview(head, true, told(expected.reading), usize::MAX);
                let context = format!("{} cut at {len}", fixture.file);
                assert!(cut.truncated, "{context}");
                assert!(cut.rows.len() <= expected.rows.len(), "{context}");
                assert_eq!(cut.rows[..], expected.rows[..cut.rows.len()], "{context}");
                assert!(
                    cut.columns.is_empty() || cut.columns == expected.columns,
                    "{context}: {:?}",
                    cut.columns
                );
            }
        }
    }

    /// A fixture without its expected JSON, or an expected JSON without its
    /// fixture, would be skipped by the tests above without a word.
    #[test]
    fn the_fixture_directory_holds_exactly_the_listed_fixtures() {
        let mut listed: Vec<String> = FIXTURES
            .iter()
            .flat_map(|fixture| {
                let (stem, _) = fixture.file.rsplit_once('.').unwrap();
                [fixture.file.to_owned(), format!("{stem}.expected.json")]
            })
            .collect();
        listed.sort_unstable();
        let mut found: Vec<String> = std::fs::read_dir(FIXTURE_DIR)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .filter(|name| name != "README.md" && name != ".gitattributes")
            .collect();
        found.sort_unstable();
        assert_eq!(
            found, listed,
            "add the new file (and its .expected.json) to FIXTURES, or delete the stray one"
        );
    }
}
