//! Reading an Excel workbook (`.xls`, `.xlsx`) for the file upload, as a pure
//! function of its bytes: which sheets it has, and one sheet turned into
//! delimited text. X1 of `docs/superpowers/plans/2026-10-07-upload-excel.md`
//! (ADR 0014, amendment of 2026-10-07).
//!
//! Nothing here reads a file, a clock or the network, and nothing panics: the
//! parser of the `calamine` crate runs on a file somebody chose, so it runs
//! under `catch_unwind` and a panic is the same refusal as a file that does not
//! open.
//!
//! # Where this sits
//!
//! The load job reads delimited text and nothing else (ADR 0014, decision 3:
//! two readers, one dialect). A workbook is brought to that dialect here, once,
//! in the API: [`read_sheet`] turns the chosen sheet into text, [`SheetData::to_csv`]
//! writes it as UTF-8 text separated by commas, and the routes store that beside
//! the original and hand it to the unchanged job. The output is pinned by
//! `ops/fixtures/uploads/converted_sheet.csv`, which `upload_parse` and the
//! load's own tests read as well.
//!
//! # How a cell becomes text (every column is text)
//!
//! - **Empty**: empty text. A cell with an empty string is empty too.
//! - **Text**: as stored.
//! - **Number**: never through the cell's display format (`1234.5`, not
//!   `1.234,50`; `0.25` for a cell shown as `25%`). A whole number has no
//!   decimal point. Any other number is written in the shortest form that
//!   reads back to the same value (Rust's `Display` for `f64`), so `0.1` is
//!   `0.1`. A number below `1e-6` or from `1e21` up is written in exponent form
//!   (`1e21`, `1.5e-7`), as JSON does, so a huge value is not a hundred
//!   digits.
//! - **Date and date-time**: ISO 8601, `YYYY-MM-DD` when the time is midnight,
//!   otherwise `YYYY-MM-DD HH:MM:SS` (with `.mmm` appended only when the cell
//!   holds milliseconds, so nothing is silently dropped). The date system of
//!   the workbook (1900 or 1904) is the library's to apply, including Excel's
//!   non-existent 1900-02-29. Whether a number IS a date is decided by the
//!   cell's number format, which is the one use of the display format; a
//!   number whose format is not a date format is a number.
//! - **Time of day**: a date cell with a value below 1 (no date part) is
//!   `HH:MM:SS`. A date cell holding exactly 1904-01-01 in a 1904 workbook has
//!   the value 0 and is written as `00:00:00`: the file cannot tell the two
//!   apart, and a time of day is far the commoner meaning.
//! - **Duration** (a `[h]:mm:ss` cell): `HH:MM:SS`, the hours not wrapped at 24
//!   (`36:00:00`).
//! - **Boolean**: `true` / `false`.
//! - **Error**: the error's own text (`#DIV/0!`).
//! - **Formula**: its stored result, by the rules above. A formula the file
//!   stored no result for is empty: nothing here calculates.
//! - **Merged range**: the value in the top-left cell and the rest empty, as the
//!   file stores it.
//! - A value outside what a date can be (negative, or past the year 9990) is
//!   written as the number it is.
//!
//! # The used range
//!
//! A sheet's records begin at its first non-empty cell and end at its last:
//! empty rows and columns before the first and after the last are not part of
//! the data, a cell that is only formatted is not a cell, and a row or column
//! of nothing between two cells is kept (as an empty record, or as empty cells
//! of every record). The header-row number the user picks counts records of
//! THAT range, which is not always the row number Excel shows when the sheet
//! starts below row 1.
//!
//! # Limits
//!
//! A workbook is compressed and can hold far more than the bytes of the upload
//! suggest, and the API converts it in memory. A sheet whose used range is
//! more than [`MAX_SHEET_CELLS`] cells (rows times columns, empty cells
//! included) is refused ([`WorkbookError::TooLarge`]). The number is a cap, not
//! a measurement. For `.xlsx` the cap is checked while the sheet streams in, so
//! a sheet that only claims a huge range costs no more than the cells it has;
//! for `.xls` the library builds the sheet before this module sees it, and the
//! format itself bounds it (65,536 rows by 256 columns).
//!
//! # What is accepted
//!
//! Only `.xlsx` (a zip) and `.xls` (an OLE compound file) are opened, chosen by
//! the file's first bytes, never by its name; the name's job is the routes'.
//! `.xlsb`, `.ods` and a macro-enabled `.xlsm` are not opened here by a
//! dedicated reader: a zip that is not an `.xlsx` workbook does not open, and
//! one that does is read for its cell values only (no macro is ever run).
//! A password-protected workbook is refused ([`WorkbookError::Encrypted`]).

use std::io::Cursor;
use std::num::FpCategory;
use std::panic::{AssertUnwindSafe, catch_unwind};

use calamine::{Data, ExcelDateTime, Reader, Sheet, SheetType, SheetVisible, Xls, XlsError, Xlsx};
use serde::Serialize;

/// The most cells a sheet's used range may hold: 5,000,000. A cap that keeps
/// the conversion's memory bounded, not a measured limit.
pub const MAX_SHEET_CELLS: u64 = 5_000_000;

/// Why a workbook could not be read. Each is a fixed sentence: nothing the
/// library said reaches a response.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum WorkbookError {
    /// The bytes are not a workbook this reader can open: not an `.xls` or
    /// `.xlsx`, damaged, or cut short.
    #[error("This file could not be opened as an Excel workbook (.xls or .xlsx).")]
    Unreadable,
    /// The workbook is password protected.
    #[error(
        "This workbook is password protected, so it cannot be read. Remove the password and upload it again."
    )]
    Encrypted,
    /// The workbook has no sheet of cells.
    #[error("This workbook has no sheet that can be read.")]
    NoSheets,
    /// The sheet asked for is not in the workbook.
    #[error("The workbook has no sheet with that name.")]
    UnknownSheet,
    /// The sheet's used range is past [`MAX_SHEET_CELLS`].
    #[error("That sheet is larger than the limit of 5,000,000 cells.")]
    TooLarge,
}

/// One sheet of a workbook, as the console lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SheetInfo {
    /// The sheet's name, as the workbook spells it.
    pub name: String,
    /// `false` for a hidden sheet (including one that is "very hidden").
    pub visible: bool,
}

/// The sheets of a workbook and which one is read when none is asked for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkbookInfo {
    /// Every sheet of cells, in workbook order. Chart sheets, dialog sheets
    /// and macro sheets hold no table and are not listed.
    pub sheets: Vec<SheetInfo>,
    /// The first visible sheet, or the first sheet when none is visible.
    pub default_sheet: String,
}

/// One sheet's non-empty cells, ready to be written as text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SheetData {
    name: String,
    rows: usize,
    cols: usize,
    /// Sorted by row, then column; positions are relative to the used range.
    cells: Vec<Cell>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Cell {
    row: usize,
    col: usize,
    text: String,
}

/// What reading a sheet gives: the workbook's sheets and the one read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkbookSheet {
    /// The workbook's sheets and the default among them.
    pub info: WorkbookInfo,
    /// The sheet that was read.
    pub data: SheetData,
}

// ── opening ──────────────────────────────────────────────────────────────

/// The first bytes of a zip archive (`.xlsx`).
const ZIP_MAGIC: &[u8] = b"PK\x03\x04";

/// The first bytes of an OLE compound file (`.xls`, and the container of a
/// password-protected `.xlsx`).
const OLE_MAGIC: &[u8] = &[0xD0, 0xCF, 0x11, 0xE0];

/// The name of the stream that holds an encrypted OOXML package. An OLE
/// directory stores names as UTF-16LE.
const ENCRYPTED_PACKAGE: &str = "EncryptedPackage";

/// Run `work`, and turn a panic of the parser into [`WorkbookError::Unreadable`].
fn guarded<T>(work: impl FnOnce() -> Result<T, WorkbookError>) -> Result<T, WorkbookError> {
    catch_unwind(AssertUnwindSafe(work)).unwrap_or(Err(WorkbookError::Unreadable))
}

/// A workbook opened by the reader its first bytes call for.
enum Book<'a> {
    // Boxed: the two readers differ in size by more than `large_enum_variant`
    // allows.
    Xlsx(Box<Xlsx<Cursor<&'a [u8]>>>),
    Xls(Box<Xls<Cursor<&'a [u8]>>>),
}

/// Whether `bytes` hold the stream an encrypted OOXML package has. Looked for
/// only after an OLE file failed to open as a workbook: an encrypted `.xlsx`
/// is an OLE file that has no `Workbook` stream.
fn holds_encrypted_package(bytes: &[u8]) -> bool {
    let marker: Vec<u8> = ENCRYPTED_PACKAGE
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    bytes.windows(marker.len()).any(|window| window == marker)
}

fn open(bytes: &[u8]) -> Result<Book<'_>, WorkbookError> {
    if bytes.starts_with(ZIP_MAGIC) {
        return Xlsx::new(Cursor::new(bytes))
            .map(|book| Book::Xlsx(Box::new(book)))
            .map_err(|_err| WorkbookError::Unreadable);
    }
    if bytes.starts_with(OLE_MAGIC) {
        return Xls::new(Cursor::new(bytes))
            .map(|book| Book::Xls(Box::new(book)))
            .map_err(|err| xls_failure(&err, bytes));
    }
    Err(WorkbookError::Unreadable)
}

/// What an `.xls` failure means to the user.
fn xls_failure(err: &XlsError, bytes: &[u8]) -> WorkbookError {
    if matches!(err, XlsError::Password) || holds_encrypted_package(bytes) {
        WorkbookError::Encrypted
    } else {
        WorkbookError::Unreadable
    }
}

impl Book<'_> {
    /// The sheets of cells, in workbook order.
    fn worksheets(&self) -> Vec<Sheet> {
        let all = match self {
            Self::Xlsx(book) => book.sheets_metadata(),
            Self::Xls(book) => book.sheets_metadata(),
        };
        all.iter()
            .filter(|sheet| matches!(sheet.typ, SheetType::WorkSheet))
            .cloned()
            .collect()
    }

    /// The non-empty cells of the sheet `name`, under `cap`.
    fn collect(&mut self, bytes: &[u8], name: &str, cap: u64) -> Result<Collector, WorkbookError> {
        let mut collector = Collector::new(cap);
        match self {
            Self::Xlsx(book) => {
                let mut reader = book
                    .worksheet_cells_reader(name)
                    .map_err(|_err| WorkbookError::Unreadable)?;
                // Cell by cell, so the cap is enforced before a range the file
                // merely claims is allocated (`worksheet_range` would build
                // every cell of the bounding box first).
                while let Some(cell) = reader
                    .next_cell()
                    .map_err(|_err| WorkbookError::Unreadable)?
                {
                    let (row, col) = cell.get_position();
                    collector.push(row, col, &Data::from(cell.get_value().clone()))?;
                }
            }
            Self::Xls(book) => {
                let range = book
                    .worksheet_range(name)
                    .map_err(|err| xls_failure(&err, bytes))?;
                for (row, col, value) in range.used_cells() {
                    collector.push(to_u32(row), to_u32(col), value)?;
                }
            }
        }
        Ok(collector)
    }
}

fn to_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

fn to_usize(value: u32) -> usize {
    usize::try_from(value).unwrap_or(usize::MAX)
}

// ── sheets ───────────────────────────────────────────────────────────────

fn info_of(sheets: &[Sheet]) -> Result<WorkbookInfo, WorkbookError> {
    let first = sheets.first().ok_or(WorkbookError::NoSheets)?;
    let default = sheets
        .iter()
        .find(|sheet| matches!(sheet.visible, SheetVisible::Visible))
        .unwrap_or(first);
    Ok(WorkbookInfo {
        sheets: sheets
            .iter()
            .map(|sheet| SheetInfo {
                name: sheet.name.clone(),
                visible: matches!(sheet.visible, SheetVisible::Visible),
            })
            .collect(),
        default_sheet: default.name.clone(),
    })
}

/// The sheets of the workbook in `bytes`, and the default one.
///
/// # Errors
///
/// [`WorkbookError::Unreadable`] when the bytes do not open as an `.xls` or
/// `.xlsx` (a zip of another kind, a damaged or cut file, text),
/// [`WorkbookError::Encrypted`] for a password-protected workbook,
/// [`WorkbookError::NoSheets`] when it has no sheet of cells.
pub fn list_sheets(bytes: &[u8]) -> Result<WorkbookInfo, WorkbookError> {
    guarded(|| info_of(&open(bytes)?.worksheets()))
}

/// Read one sheet of the workbook in `bytes`: the sheet called `sheet`, or the
/// default one when it is `None`.
///
/// # Errors
///
/// Those of [`list_sheets`]; [`WorkbookError::UnknownSheet`] when `sheet` is
/// not the name of a sheet of cells; [`WorkbookError::TooLarge`] when its used
/// range is past [`MAX_SHEET_CELLS`].
pub fn read_sheet(bytes: &[u8], sheet: Option<&str>) -> Result<WorkbookSheet, WorkbookError> {
    read_sheet_capped(bytes, sheet, MAX_SHEET_CELLS)
}

fn read_sheet_capped(
    bytes: &[u8],
    sheet: Option<&str>,
    cap: u64,
) -> Result<WorkbookSheet, WorkbookError> {
    guarded(|| {
        let mut book = open(bytes)?;
        let info = info_of(&book.worksheets())?;
        let name = match sheet {
            None => info.default_sheet.clone(),
            Some(asked) => info
                .sheets
                .iter()
                .find(|listed| listed.name == asked)
                .map(|listed| listed.name.clone())
                .ok_or(WorkbookError::UnknownSheet)?,
        };
        let data = book.collect(bytes, &name, cap)?.finish(name);
        Ok(WorkbookSheet { info, data })
    })
}

/// The used range's bounds, as absolute positions.
#[derive(Debug, Clone, Copy)]
struct Bounds {
    first_row: u32,
    last_row: u32,
    first_col: u32,
    last_col: u32,
}

impl Bounds {
    fn including(self, row: u32, col: u32) -> Self {
        Self {
            first_row: self.first_row.min(row),
            last_row: self.last_row.max(row),
            first_col: self.first_col.min(col),
            last_col: self.last_col.max(col),
        }
    }

    fn area(self) -> u64 {
        (u64::from(self.last_row - self.first_row) + 1)
            * (u64::from(self.last_col - self.first_col) + 1)
    }
}

/// Collects the non-empty cells of a sheet and keeps the cap.
struct Collector {
    cells: Vec<(u32, u32, String)>,
    bounds: Option<Bounds>,
    cap: u64,
}

impl Collector {
    fn new(cap: u64) -> Self {
        Self {
            cells: Vec::new(),
            bounds: None,
            cap,
        }
    }

    /// Take a cell at an absolute position. An empty cell is not taken: it
    /// does not widen the used range.
    fn push(&mut self, row: u32, col: u32, value: &Data) -> Result<(), WorkbookError> {
        let Some(text) = cell_text(value) else {
            return Ok(());
        };
        let bounds = self.bounds.map_or(
            Bounds {
                first_row: row,
                last_row: row,
                first_col: col,
                last_col: col,
            },
            |bounds| bounds.including(row, col),
        );
        if bounds.area() > self.cap {
            return Err(WorkbookError::TooLarge);
        }
        self.bounds = Some(bounds);
        self.cells.push((row, col, text));
        Ok(())
    }

    fn finish(self, name: String) -> SheetData {
        let Some(bounds) = self.bounds else {
            return SheetData {
                name,
                rows: 0,
                cols: 0,
                cells: Vec::new(),
            };
        };
        let mut cells: Vec<Cell> = self
            .cells
            .into_iter()
            .map(|(row, col, text)| Cell {
                row: to_usize(row - bounds.first_row),
                col: to_usize(col - bounds.first_col),
                text,
            })
            .collect();
        cells.sort_by_key(|cell| (cell.row, cell.col));
        SheetData {
            name,
            rows: to_usize(bounds.last_row - bounds.first_row) + 1,
            cols: to_usize(bounds.last_col - bounds.first_col) + 1,
            cells,
        }
    }
}

impl SheetData {
    /// The sheet's name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Records in the used range (0 for a sheet with no cells).
    #[must_use]
    pub fn rows(&self) -> usize {
        self.rows
    }

    /// Whether the sheet has no cells at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }

    /// The records of the used range, each as wide as the range, at most
    /// `max_records` of them. The same records `to_csv` writes. Tests only: the
    /// routes never need the records, only the text.
    #[cfg(test)]
    #[must_use]
    pub fn records(&self, max_records: Option<usize>) -> Vec<Vec<String>> {
        let mut out = Vec::new();
        self.each_record(max_records, |fields| {
            out.push(fields.iter().map(|field| (*field).to_owned()).collect());
        });
        out
    }

    /// The used range as delimited text: UTF-8, comma, `"` quoting, a line
    /// feed after every record, at most `max_records` records (all when
    /// `None`). One writer, [`write_record`], for every caller; what it writes
    /// reads back through `upload_parse::split_records` to exactly
    /// `records`.
    #[must_use]
    pub fn to_csv(&self, max_records: Option<usize>) -> Vec<u8> {
        let mut out = Vec::new();
        self.each_record(max_records, |fields| write_record(&mut out, fields));
        finish_text(out)
    }

    fn each_record(&self, max_records: Option<usize>, mut each: impl FnMut(&[&str])) {
        let rows = max_records.map_or(self.rows, |max| max.min(self.rows));
        let mut next = 0;
        let mut fields: Vec<&str> = vec![""; self.cols];
        for row in 0..rows {
            fields.fill("");
            while let Some(cell) = self.cells.get(next)
                && cell.row == row
            {
                if let Some(slot) = fields.get_mut(cell.col) {
                    *slot = &cell.text;
                }
                next += 1;
            }
            each(&fields);
        }
    }
}

// ── cells to text ────────────────────────────────────────────────────────

/// The text of one cell, or `None` for an empty one. See the module doc.
fn cell_text(value: &Data) -> Option<String> {
    match value {
        Data::Empty => None,
        Data::String(text) => (!text.is_empty()).then(|| text.clone()),
        Data::Int(number) => Some(number.to_string()),
        Data::Float(number) => Some(number_text(*number)),
        Data::Bool(flag) => Some(flag.to_string()),
        Data::DateTime(moment) => Some(datetime_text(moment)),
        Data::DateTimeIso(text) | Data::DurationIso(text) => Some(text.clone()),
        Data::Error(error) => Some(error.to_string()),
    }
}

/// A number as text: shortest form that reads back to the same value, in
/// plain digits between `1e-6` and `1e21`, in exponent form outside.
fn number_text(value: f64) -> String {
    if value.classify() == FpCategory::Zero {
        // Also negative zero, which `Display` would write as `-0`.
        return "0".to_owned();
    }
    if value.is_finite() && !(1e-6..1e21).contains(&value.abs()) {
        return format!("{value:e}");
    }
    format!("{value}")
}

/// The latest serial this module writes as a date: a little before the year
/// 9990 in the 1900 system, so that a date in the 1904 system, which counts
/// 1,462 days less, cannot run past the year 9999 either.
const LATEST_DATE_SERIAL: f64 = 2_957_003.0;

fn datetime_text(value: &ExcelDateTime) -> String {
    let serial = value.as_f64();
    // `contains` is false for NaN too.
    if !(0.0..LATEST_DATE_SERIAL).contains(&serial) {
        return number_text(serial);
    }
    if value.is_duration() {
        return duration_text(serial);
    }
    let (year, month, day, hour, minute, second, milli) = value.to_ymd_hms_milli();
    let clock = if milli == 0 {
        format!("{hour:02}:{minute:02}:{second:02}")
    } else {
        format!("{hour:02}:{minute:02}:{second:02}.{milli:03}")
    };
    if serial < 1.0 {
        clock
    } else if hour == 0 && minute == 0 && second == 0 && milli == 0 {
        format!("{year:04}-{month:02}-{day:02}")
    } else {
        format!("{year:04}-{month:02}-{day:02} {clock}")
    }
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the caller passes a value in 0..2_957_003 (days), so the rounded seconds are in 0..2.6e11, far inside u64"
)]
fn duration_text(days: f64) -> String {
    let seconds = (days * 86_400.0).round() as u64;
    let (hours, minutes, rest) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
    format!("{hours:02}:{minutes:02}:{rest:02}")
}

// ── text to delimited text ───────────────────────────────────────────────

/// Write one record in the dialect `upload_parse` reads: fields separated by
/// commas, a field quoted (with `""` for a quote) when it holds a comma, a
/// quote or a line break, a line feed at the end. Cells are never trimmed
/// (neither reader trims), so leading and trailing spaces need no quotes.
///
/// A record of one empty field is written `""`: as an empty line it would read
/// back as a record with no fields.
pub fn write_record(out: &mut Vec<u8>, fields: &[&str]) {
    if matches!(fields, [""]) {
        out.extend_from_slice(b"\"\"");
    } else {
        for (index, field) in fields.iter().enumerate() {
            if index > 0 {
                out.push(b',');
            }
            write_field(out, field);
        }
    }
    out.push(b'\n');
}

fn write_field(out: &mut Vec<u8>, field: &str) {
    if field.contains([',', '"', '\n', '\r']) {
        out.push(b'"');
        out.extend_from_slice(field.replace('"', "\"\"").as_bytes());
        out.push(b'"');
    } else {
        out.extend_from_slice(field.as_bytes());
    }
}

/// The byte order mark of UTF-8.
const BOM: &[u8] = &[0xEF, 0xBB, 0xBF];

/// Both readers drop a byte order mark at the start of the text, so text that
/// really starts with U+FEFF (an unquoted first cell that begins with it) gets
/// a mark of its own in front, and the cell keeps its character.
fn finish_text(mut out: Vec<u8>) -> Vec<u8> {
    if out.starts_with(BOM) {
        let mut marked = BOM.to_vec();
        marked.append(&mut out);
        return marked;
    }
    out
}

/// Write `records` as `SheetData::to_csv` does, for tests that hold records and
/// not a sheet.
#[cfg(test)]
#[must_use]
pub fn encode_csv(records: &[Vec<String>]) -> Vec<u8> {
    let mut out = Vec::new();
    for record in records {
        let fields: Vec<&str> = record.iter().map(String::as_str).collect();
        write_record(&mut out, &fields);
    }
    finish_text(out)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use calamine::ExcelDateTimeType;

    use super::*;
    use crate::upload_parse::{Encoding, decode, split_records};

    const STOCK_XLSX: &[u8] = include_bytes!("../../../../ops/fixtures/workbooks/stock.xlsx");
    const STOCK_XLS: &[u8] = include_bytes!("../../../../ops/fixtures/workbooks/stock.xls");
    const FORMULA_XLSX: &[u8] = include_bytes!("../../../../ops/fixtures/workbooks/formula.xlsx");
    const DATE1904_XLSX: &[u8] = include_bytes!("../../../../ops/fixtures/workbooks/date1904.xlsx");
    const PLAIN_ZIP: &[u8] = include_bytes!("../../../../ops/fixtures/workbooks/plain.zip");
    const CONVERTED_QUIRKS: &[u8] =
        include_bytes!("../../../../ops/fixtures/uploads/converted_sheet.csv");

    /// Both formats hold the same five sheets.
    fn both() -> [(&'static str, &'static [u8]); 2] {
        [("stock.xlsx", STOCK_XLSX), ("stock.xls", STOCK_XLS)]
    }

    fn records_of(bytes: &[u8], sheet: &str) -> Vec<Vec<String>> {
        read_sheet(bytes, Some(sheet)).unwrap().data.records(None)
    }

    fn rows(rows: &[&[&str]]) -> Vec<Vec<String>> {
        rows.iter()
            .map(|row| row.iter().map(|cell| (*cell).to_owned()).collect())
            .collect()
    }

    // ── sheets ──────────────────────────────────────────────────────────

    #[test]
    fn every_sheet_is_listed_in_workbook_order_with_its_visibility() {
        for (name, bytes) in both() {
            let info = list_sheets(bytes).unwrap();
            let listed: Vec<(&str, bool)> = info
                .sheets
                .iter()
                .map(|sheet| (sheet.name.as_str(), sheet.visible))
                .collect();
            assert_eq!(
                listed,
                [
                    ("Stock", true),
                    ("Quirks", true),
                    ("Offset", true),
                    ("Hidden notes", false),
                    ("Empty", true),
                ],
                "{name}"
            );
            assert_eq!(info.default_sheet, "Stock", "{name}");
        }
    }

    #[test]
    fn the_default_is_the_first_visible_sheet_and_the_first_when_none_is_visible() {
        let sheet = |name: &str, visible| Sheet {
            name: name.to_owned(),
            typ: SheetType::WorkSheet,
            visible,
        };
        let info = info_of(&[
            sheet("Hidden", SheetVisible::Hidden),
            sheet("Secret", SheetVisible::VeryHidden),
            sheet("Shown", SheetVisible::Visible),
        ])
        .unwrap();
        assert_eq!(info.default_sheet, "Shown");
        assert_eq!(
            info.sheets.iter().map(|s| s.visible).collect::<Vec<_>>(),
            [false, false, true]
        );
        let info = info_of(&[
            sheet("A", SheetVisible::Hidden),
            sheet("B", SheetVisible::Hidden),
        ])
        .unwrap();
        assert_eq!(info.default_sheet, "A");
        assert_eq!(info_of(&[]).unwrap_err(), WorkbookError::NoSheets);
    }

    #[test]
    fn the_default_sheet_is_read_when_none_is_named_and_an_unknown_name_is_refused() {
        for (name, bytes) in both() {
            let read = read_sheet(bytes, None).unwrap();
            assert_eq!(read.data.name(), "Stock", "{name}");
            assert_eq!(read.info.default_sheet, "Stock", "{name}");
            assert_eq!(
                read_sheet(bytes, Some("stock")).unwrap_err(),
                WorkbookError::UnknownSheet,
                "{name}: names match exactly"
            );
            assert_eq!(
                read_sheet(bytes, Some("")).unwrap_err(),
                WorkbookError::UnknownSheet,
                "{name}"
            );
        }
    }

    #[test]
    fn a_hidden_sheet_can_still_be_chosen() {
        for (name, bytes) in both() {
            assert_eq!(
                records_of(bytes, "Hidden notes"),
                rows(&[&["note"], &["internal"]]),
                "{name}"
            );
        }
    }

    #[test]
    fn a_sheet_with_no_cells_reads_as_nothing() {
        for (name, bytes) in both() {
            let read = read_sheet(bytes, Some("Empty")).unwrap();
            assert!(read.data.is_empty(), "{name}");
            assert_eq!(read.data.rows(), 0, "{name}");
            assert!(read.data.to_csv(None).is_empty(), "{name}");
            assert!(read.data.records(None).is_empty(), "{name}");
        }
    }

    // ── what a sheet holds ──────────────────────────────────────────────

    #[test]
    fn a_plain_sheet_reads_as_its_cells_with_dates_and_numbers_as_text() {
        let expected = rows(&[
            &["sku", "name", "qty", "price", "received"],
            &["A-100", "Bolt, hex M6", "10", "1.5", "2025-09-24"],
            &["A-200", "Washer", "250", "0.05", "2025-09-25"],
            &[
                "A-300",
                "Flange DN50 \u{2013} steel",
                "4",
                "12",
                "2025-10-01",
            ],
        ]);
        for (name, bytes) in both() {
            assert_eq!(records_of(bytes, "Stock"), expected, "{name}");
        }
    }

    /// The sheet starts at C3 and has an empty row inside: the records begin
    /// at the first cell, and the empty row stays as a record of empty cells.
    #[test]
    fn the_records_begin_at_the_first_cell_and_an_empty_row_inside_stays() {
        let expected = rows(&[
            &["id", "label"],
            &["1", "first"],
            &["", ""],
            &["3", "third"],
        ]);
        for (name, bytes) in both() {
            assert_eq!(records_of(bytes, "Offset"), expected, "{name}");
        }
    }

    /// Every rule of the conversion on one sheet, pinned by the fixture that
    /// the preview's and the load's tests read too.
    #[test]
    fn the_quirks_sheet_converts_to_the_pinned_text_in_both_formats() {
        for (name, bytes) in both() {
            let csv = read_sheet(bytes, Some("Quirks")).unwrap().data.to_csv(None);
            assert_eq!(
                String::from_utf8_lossy(&csv),
                String::from_utf8_lossy(CONVERTED_QUIRKS),
                "{name}"
            );
            assert_eq!(csv, CONVERTED_QUIRKS, "{name}");
        }
    }

    #[test]
    fn what_is_written_reads_back_as_the_records_that_were_written() {
        for (name, bytes) in both() {
            for sheet in ["Stock", "Quirks", "Offset", "Hidden notes"] {
                let data = read_sheet(bytes, Some(sheet)).unwrap().data;
                let text = decode(&data.to_csv(None), Encoding::Utf8);
                assert_eq!(
                    split_records(&text, ','),
                    data.records(None),
                    "{name} {sheet}"
                );
            }
        }
    }

    #[test]
    fn a_prefix_of_the_records_is_a_prefix_of_the_text() {
        let data = read_sheet(STOCK_XLSX, Some("Quirks")).unwrap().data;
        let all = data.records(None);
        let head = data.records(Some(3));
        assert_eq!(head, all[..3]);
        let text = decode(&data.to_csv(Some(3)), Encoding::Utf8);
        assert_eq!(split_records(&text, ','), head);
        assert_eq!(data.records(Some(10_000)), all);
        assert!(data.to_csv(Some(0)).is_empty());
    }

    #[test]
    fn a_formula_is_its_stored_result_and_an_error_result_is_its_text() {
        assert_eq!(
            records_of(FORMULA_XLSX, "Formula"),
            rows(&[&["sum", "3"], &["text", "ab"], &["error", "#DIV/0!"]])
        );
    }

    #[test]
    fn dates_of_the_1904_date_system_are_the_same_dates() {
        assert_eq!(
            records_of(DATE1904_XLSX, "Dates"),
            rows(&[
                &["when"],
                &["2025-09-24"],
                &["2025-09-24 13:30:05"],
                &["13:30:00"],
            ])
        );
    }

    // ── limits and refusals ─────────────────────────────────────────────

    /// `Quirks` is 2 columns by 28 rows.
    #[test]
    fn a_sheet_over_the_cap_is_refused_and_one_at_it_is_not() {
        for (name, bytes) in both() {
            assert_eq!(
                read_sheet_capped(bytes, Some("Quirks"), 56)
                    .unwrap()
                    .data
                    .rows(),
                28,
                "{name}"
            );
            assert_eq!(
                read_sheet_capped(bytes, Some("Quirks"), 55).unwrap_err(),
                WorkbookError::TooLarge,
                "{name}"
            );
            assert_eq!(
                read_sheet_capped(bytes, Some("Quirks"), 1).unwrap_err(),
                WorkbookError::TooLarge,
                "{name}"
            );
        }
    }

    #[test]
    fn the_cap_is_five_million_cells_and_the_sentence_says_so() {
        assert_eq!(MAX_SHEET_CELLS, 5_000_000);
        assert!(WorkbookError::TooLarge.to_string().contains("5,000,000"));
    }

    #[test]
    fn a_zip_that_is_not_a_workbook_is_unreadable() {
        assert_eq!(
            list_sheets(PLAIN_ZIP).unwrap_err(),
            WorkbookError::Unreadable
        );
        assert_eq!(
            read_sheet(PLAIN_ZIP, None).unwrap_err(),
            WorkbookError::Unreadable
        );
    }

    #[test]
    fn a_truncated_file_garbage_text_and_nothing_are_unreadable_and_nothing_panics() {
        let half = STOCK_XLSX.len() / 2;
        for (what, bytes) in [
            ("half an xlsx", &STOCK_XLSX[..half]),
            ("the first bytes of an xlsx", &STOCK_XLSX[..4]),
            ("the head of an xls", &STOCK_XLS[..600]),
            ("the magic of an xls", &STOCK_XLS[..4]),
            ("text", &b"id,name\n1,a\n"[..]),
            ("nothing", &b""[..]),
            ("zeros", &[0_u8; 512][..]),
        ] {
            assert_eq!(
                list_sheets(bytes).unwrap_err(),
                WorkbookError::Unreadable,
                "{what}"
            );
            assert_eq!(
                read_sheet(bytes, None).unwrap_err(),
                WorkbookError::Unreadable,
                "{what}"
            );
        }
    }

    /// Cutting a real file anywhere gives a refusal or a reading, never a
    /// panic that escapes.
    #[test]
    fn a_file_cut_anywhere_never_panics() {
        for bytes in [STOCK_XLSX, STOCK_XLS] {
            for len in (0..bytes.len()).step_by(97) {
                let _ = list_sheets(&bytes[..len]);
            }
        }
    }

    /// A password-protected `.xlsx` is an OLE file with a stream named
    /// `EncryptedPackage` and no `Workbook` stream. This is not a real one
    /// (none can be made without an Office tool); it has what the detection
    /// looks for: the OLE magic and the stream's name in UTF-16LE.
    #[test]
    fn a_file_that_holds_an_encrypted_package_is_encrypted_not_unreadable() {
        let mut bytes = OLE_MAGIC.to_vec();
        bytes.extend_from_slice(&[0_u8; 600]);
        bytes.extend(ENCRYPTED_PACKAGE.encode_utf16().flat_map(u16::to_le_bytes));
        bytes.extend_from_slice(&[0_u8; 600]);
        assert_eq!(list_sheets(&bytes).unwrap_err(), WorkbookError::Encrypted);
        assert!(!holds_encrypted_package(STOCK_XLS));
    }

    #[test]
    fn no_refusal_carries_anything_but_its_own_sentence() {
        for (err, sentence) in [
            (
                WorkbookError::Unreadable,
                "This file could not be opened as an Excel workbook (.xls or .xlsx).",
            ),
            (
                WorkbookError::Encrypted,
                "This workbook is password protected, so it cannot be read. Remove the password and upload it again.",
            ),
            (
                WorkbookError::NoSheets,
                "This workbook has no sheet that can be read.",
            ),
            (
                WorkbookError::UnknownSheet,
                "The workbook has no sheet with that name.",
            ),
            (
                WorkbookError::TooLarge,
                "That sheet is larger than the limit of 5,000,000 cells.",
            ),
        ] {
            assert_eq!(err.to_string(), sentence);
        }
    }

    // ── cells to text, one rule at a time ───────────────────────────────

    #[test]
    fn numbers_are_whole_or_shortest_and_never_the_display_format() {
        for (value, text) in [
            (1.0, "1"),
            (-3.0, "-3"),
            (0.0, "0"),
            (-0.0, "0"),
            (0.1, "0.1"),
            (1234.5, "1234.5"),
            (0.25, "0.25"),
            (-42.5, "-42.5"),
            (0.1 + 0.2, "0.30000000000000004"),
            (1.0 / 3.0, "0.3333333333333333"),
            (123_456_789_012_345.0, "123456789012345"),
            (1e20, "100000000000000000000"),
            (1e21, "1e21"),
            (1.5e300, "1.5e300"),
            (0.000_001, "0.000001"),
            (1e-7, "1e-7"),
            (-1.5e-7, "-1.5e-7"),
        ] {
            assert_eq!(number_text(value), text, "{value:e}");
        }
        for value in [0.1 + 0.2, 1e21, 1e-7, 123.456, 1.0 / 3.0, -2.5e-9] {
            let read_back = number_text(value).parse::<f64>().unwrap();
            assert_eq!(read_back.to_bits(), value.to_bits(), "{value:e}");
        }
    }

    #[test]
    fn whole_numbers_and_integers_cells_are_written_alike() {
        assert_eq!(cell_text(&Data::Int(7)).unwrap(), "7");
        assert_eq!(cell_text(&Data::Float(7.0)).unwrap(), "7");
        assert_eq!(
            cell_text(&Data::Int(-9_007_199_254_740_993)).unwrap(),
            "-9007199254740993"
        );
    }

    #[test]
    fn empty_cells_and_empty_text_are_nothing_and_other_text_is_kept_as_it_is() {
        assert_eq!(cell_text(&Data::Empty), None);
        assert_eq!(cell_text(&Data::String(String::new())), None);
        assert_eq!(cell_text(&Data::String("  ".to_owned())).unwrap(), "  ");
        assert_eq!(cell_text(&Data::String("007".to_owned())).unwrap(), "007");
        assert_eq!(cell_text(&Data::Bool(true)).unwrap(), "true");
        assert_eq!(cell_text(&Data::Bool(false)).unwrap(), "false");
        assert_eq!(
            cell_text(&Data::DateTimeIso("2025-09-24T13:30:05".to_owned())).unwrap(),
            "2025-09-24T13:30:05"
        );
    }

    fn moment(serial: f64, is_1904: bool) -> String {
        datetime_text(&ExcelDateTime::new(
            serial,
            ExcelDateTimeType::DateTime,
            is_1904,
        ))
    }

    #[test]
    fn dates_and_times_are_iso_in_either_date_system() {
        // 2025-09-24 is serial 45924 in the 1900 system and 44462 in the 1904.
        assert_eq!(moment(45_924.0, false), "2025-09-24");
        assert_eq!(moment(44_462.0, true), "2025-09-24");
        // 13:30:05 is 48,605 of the day's 86,400 seconds.
        let fraction = 48_605.0 / 86_400.0;
        assert_eq!(moment(45_924.0 + fraction, false), "2025-09-24 13:30:05");
        assert_eq!(moment(44_462.0 + fraction, true), "2025-09-24 13:30:05");
        assert_eq!(moment(fraction, false), "13:30:05");
        assert_eq!(moment(0.5, false), "12:00:00");
        assert_eq!(moment(45_924.5, false), "2025-09-24 12:00:00");
        // Excel counts a 29 February 1900 that never was; the next day is
        // 1 March.
        assert_eq!(moment(60.0, false), "1900-02-29");
        assert_eq!(moment(61.0, false), "1900-03-01");
        assert_eq!(moment(36_526.0, false), "2000-01-01");
        assert_eq!(moment(36_550.0, false), "2000-01-25");
    }

    #[test]
    fn milliseconds_are_written_only_when_the_cell_has_them() {
        assert_eq!(
            moment(45_924.0 + 45_000.5 / 86_400.0, false),
            "2025-09-24 12:30:00.500"
        );
    }

    #[test]
    fn a_duration_does_not_wrap_at_24_hours() {
        let duration = |days: f64| {
            datetime_text(&ExcelDateTime::new(
                days,
                ExcelDateTimeType::TimeDelta,
                false,
            ))
        };
        assert_eq!(duration(1.5), "36:00:00");
        assert_eq!(duration(0.0), "00:00:00");
        assert_eq!(duration(10.0 / 24.0 + 5.0 / 1440.0), "10:05:00");
    }

    #[test]
    fn a_value_a_date_cannot_be_is_written_as_the_number_it_is() {
        assert_eq!(moment(-1.0, false), "-1");
        assert_eq!(moment(3_000_000.0, false), "3000000");
        assert_eq!(moment(f64::NAN, false), "NaN");
    }

    #[test]
    fn an_error_cell_is_its_own_text() {
        use calamine::CellErrorType;
        for (error, text) in [
            (CellErrorType::Div0, "#DIV/0!"),
            (CellErrorType::NA, "#N/A"),
            (CellErrorType::Name, "#NAME?"),
            (CellErrorType::Null, "#NULL!"),
            (CellErrorType::Num, "#NUM!"),
            (CellErrorType::Ref, "#REF!"),
            (CellErrorType::Value, "#VALUE!"),
        ] {
            assert_eq!(cell_text(&Data::Error(error)).unwrap(), text);
        }
    }

    // ── records to text ─────────────────────────────────────────────────

    /// What the writer must survive, through the reader the preview uses.
    #[test]
    fn awkward_records_read_back_exactly() {
        let records = rows(&[
            &["plain", "with,comma", "with \"quote\""],
            &["line\nbreak", "carriage\rreturn", "both\r\nbreaks"],
            &["  leading", "trailing  ", "  both  "],
            &["", "", ""],
            &["\"", "\"\"", ","],
            &["", "x", ""],
            &["tab\there", "semi;colon", "pipe|bar"],
            &["\u{1F600}", "caf\u{e9}", "\u{65e5}\u{672c}"],
        ]);
        let text = decode(&encode_csv(&records), Encoding::Utf8);
        assert_eq!(split_records(&text, ','), records);
    }

    #[test]
    fn a_record_of_one_empty_cell_is_not_an_empty_line() {
        let records = rows(&[&["a"], &[""], &["b"], &[""]]);
        let bytes = encode_csv(&records);
        assert_eq!(
            String::from_utf8(bytes.clone()).unwrap(),
            "a\n\"\"\nb\n\"\"\n"
        );
        assert_eq!(split_records(&decode(&bytes, Encoding::Utf8), ','), records);
    }

    #[test]
    fn a_first_cell_that_starts_with_a_byte_order_mark_keeps_it() {
        let records = rows(&[&["\u{feff}x", "y"], &["1", "2"]]);
        let bytes = encode_csv(&records);
        assert!(bytes.starts_with(&[0xEF, 0xBB, 0xBF, 0xEF, 0xBB, 0xBF]));
        assert_eq!(split_records(&decode(&bytes, Encoding::Utf8), ','), records);
        // A quoted one needs no help: the mark is not at the start.
        let quoted = rows(&[&["\u{feff},x"]]);
        let bytes = encode_csv(&quoted);
        assert!(bytes.starts_with(b"\""));
        assert_eq!(split_records(&decode(&bytes, Encoding::Utf8), ','), quoted);
    }

    #[test]
    fn nothing_is_written_for_no_records() {
        assert!(encode_csv(&[]).is_empty());
    }
}
