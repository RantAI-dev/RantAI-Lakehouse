//! Whole-result downloads, CSV and Excel (`BI-1`, plan decisions D6 and D8).
//!
//! The rows come from `ClickHouse` one line at a time ([`RowSource`]) and are
//! written to a file on disk as they arrive, so a million rows never sit in
//! memory (feature decision 8). The file is on disk, not in the response,
//! because the response must say how many rows it holds and whether the result
//! was cut (`X-Export-Rows`, `X-Export-Cut`) and the audit record has to carry
//! the same counts before the file is sent (D8): both are known only when the
//! last row has been read. The file is unlinked the moment it is open, so it
//! is gone after the download ends and on every error path alike. What is true
//! of a killed process: a CSV file is already unlinked, but an `.xlsx` is named
//! in the temp directory from the end of the write until it is open (and the
//! writer's own working files sit there while it runs), so a process killed in
//! that window leaves them behind (BI-1 review fix (SHOULD-FIX) R10).
//!
//! One rule per concern, shared with the older export: formula
//! neutralising is `csv_export::neutralise` for a CSV cell and an Excel text
//! cell alike. Excel keeps numbers as numbers (feature decision 7): with
//! "formatted", a column whose chart has a value format gets the Excel number
//! format `lakehouse_bi::export_format::excel_number_format` returns; with
//! "raw" nothing is formatted. A CSV "formatted" file holds the text the chart
//! shows (`format_text`).

use std::collections::HashMap;
use std::future::Future;
use std::io::SeekFrom;
use std::path::PathBuf;

use lakehouse_bi::export_format::{excel_number_format, format_text};
use lakehouse_bi::style::ValueFormat;
use lakehouse_clickhouse::ChError;
use serde_json::Value;
use tokio::io::{AsyncSeekExt, AsyncWriteExt, BufWriter};

use crate::csv_export;

/// The most rows a download holds (Metabase's limit, feature decision 8).
/// One more is read so a result of exactly this size is not reported as cut.
pub(crate) const EXPORT_ROW_CAP: u32 = 1_000_000;

/// The longest a download may take: the read timeout of the stream, 300
/// seconds (D6).
pub(crate) const EXPORT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);

/// The file kinds a chart can be downloaded as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FileFormat {
    /// UTF-8 text with a byte-order mark.
    Csv,
    /// An Excel workbook, one sheet.
    Xlsx,
}

impl FileFormat {
    /// The parameter value (`csv`, `xlsx`).
    pub(crate) fn parse(raw: &str) -> Option<Self> {
        match raw {
            "csv" => Some(Self::Csv),
            "xlsx" => Some(Self::Xlsx),
            _ => None,
        }
    }

    /// The file name's extension.
    pub(crate) const fn extension(self) -> &'static str {
        match self {
            Self::Csv => "csv",
            Self::Xlsx => "xlsx",
        }
    }

    /// The response's `Content-Type`.
    pub(crate) const fn content_type(self) -> &'static str {
        match self {
            Self::Csv => "text/csv; charset=utf-8",
            Self::Xlsx => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        }
    }
}

/// Whether values are written as the chart shows them or as the engine sent them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Values {
    /// With the chart's value formats.
    Formatted,
    /// Plain values.
    Raw,
}

impl Values {
    /// The parameter value (`formatted`, `raw`).
    pub(crate) fn parse(raw: &str) -> Option<Self> {
        match raw {
            "formatted" => Some(Self::Formatted),
            "raw" => Some(Self::Raw),
            _ => None,
        }
    }
}

/// Why a file could not be made. The text is for the log; a caller answers
/// with a fixed sentence (`upstream_error`, principle 4).
#[derive(Debug)]
pub(crate) enum ExportError {
    /// `ClickHouse` failed.
    Source(ChError),
    /// The disk failed.
    Io(std::io::Error),
    /// The workbook writer refused (a cell longer than Excel allows, more
    /// columns than a sheet has).
    Xlsx(String),
}

impl std::fmt::Display for ExportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Source(e) => write!(f, "export source: {e}"),
            Self::Io(e) => write!(f, "export file: {e}"),
            Self::Xlsx(e) => write!(f, "export workbook: {e}"),
        }
    }
}

impl From<ChError> for ExportError {
    fn from(e: ChError) -> Self {
        Self::Source(e)
    }
}

impl From<std::io::Error> for ExportError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

/// Rows read one at a time. The real source is
/// [`lakehouse_clickhouse::RowStream`]; the tests use a generator, so a
/// million rows can be counted without being held.
pub(crate) trait RowSource {
    /// The column names, in the order a row lists its values.
    fn names(&self) -> &[String];
    /// The column types, same order.
    fn types(&self) -> &[String];
    /// The next row, or `None` at the end.
    fn next_row(&mut self) -> impl Future<Output = Result<Option<Vec<Value>>, ChError>> + Send;
}

impl RowSource for lakehouse_clickhouse::RowStream {
    fn names(&self) -> &[String] {
        Self::names(self)
    }
    fn types(&self) -> &[String] {
        Self::types(self)
    }
    fn next_row(&mut self) -> impl Future<Output = Result<Option<Vec<Value>>, ChError>> + Send {
        Self::next_row(self)
    }
}

/// What to write and how.
pub(crate) struct ExportOptions<'a> {
    pub(crate) format: FileFormat,
    pub(crate) values: Values,
    /// Rows written at most; one more is read to see a cut.
    pub(crate) cap: usize,
    /// The deployment's number locale (`en-US`, `id-ID`).
    pub(crate) locale: &'a str,
    /// The header label of a column, by name; a column without one is headed by its name.
    pub(crate) labels: &'a HashMap<String, String>,
    /// The value format of a column, by name (used with [`Values::Formatted`]).
    pub(crate) formats: &'a HashMap<String, ValueFormat>,
}

/// A finished file, open for reading from its start.
pub(crate) struct Written {
    pub(crate) file: tokio::fs::File,
    pub(crate) len: u64,
    pub(crate) rows: usize,
    pub(crate) cut: bool,
}

/// What reading the next row came to.
enum Next {
    Row(Vec<Value>),
    End,
    /// The probe row past the cap was read: the result is longer.
    Cut,
}

async fn next_capped<S: RowSource>(
    source: &mut S,
    written: usize,
    cap: usize,
) -> Result<Next, ChError> {
    match source.next_row().await? {
        None => Ok(Next::End),
        Some(_) if written >= cap => Ok(Next::Cut),
        Some(row) => Ok(Next::Row(row)),
    }
}

/// A number the engine sent as a quoted string (64-bit integers are quoted
/// unless the server is set otherwise), when it is exact as a float.
fn numeric_string(s: &str) -> Option<f64> {
    let n: f64 = s.trim().parse().ok().filter(|n: &f64| n.is_finite())?;
    let digits = s.trim().trim_start_matches('-').len();
    // More than 15 digits does not survive being a float: keep the text.
    (digits <= 15 || s.contains(['.', 'e', 'E'])).then_some(n)
}

/// The number a cell holds, if it is one.
fn cell_number(value: &Value, numeric_column: bool) -> Option<f64> {
    match value {
        Value::Number(n) => n.as_f64(),
        Value::String(s) if numeric_column => numeric_string(s),
        _ => None,
    }
}

/// A cell of a CSV file.
fn csv_cell(
    value: &Value,
    numeric_column: bool,
    format: Option<&ValueFormat>,
    locale: &str,
) -> String {
    if let (Some(fmt), Some(n)) = (format, cell_number(value, numeric_column))
        && let Some(text) = format_text(n, fmt, locale)
    {
        return csv_export::neutralise(&text);
    }
    // BI-1 review fix (SHOULD-FIX) R5: a number the engine sent as text (a 64-bit
    // integer) is a number, not a formula: `"-5"` is written as `-5`, with no
    // apostrophe, as a JSON number is.
    if numeric_column
        && let Value::String(text) = value
        && numeric_string(text).is_some()
    {
        return text.clone();
    }
    csv_export::cell_text(Some(value))
}

/// A file on disk that is removed when this is dropped, however the function
/// that made it ended.
struct TempPath(PathBuf);

impl TempPath {
    fn new(extension: &str) -> Self {
        Self(std::env::temp_dir().join(format!(
            "lakehouse-export-{}.{extension}",
            uuid::Uuid::new_v4()
        )))
    }
}

impl Drop for TempPath {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Read `source` to its end (or the cap) and write it as `options` say.
///
/// # Errors
///
/// [`ExportError`]: the source failed, the disk failed, or the workbook writer
/// refused a cell.
pub(crate) async fn write_export<S: RowSource>(
    source: &mut S,
    options: &ExportOptions<'_>,
) -> Result<Written, ExportError> {
    match options.format {
        FileFormat::Csv => write_csv_file(source, options).await,
        FileFormat::Xlsx => write_xlsx_file(source, options).await,
    }
}

/// The per-column facts both writers need.
struct Columns {
    labels: Vec<String>,
    numeric: Vec<bool>,
    formats: Vec<Option<ValueFormat>>,
}

fn columns<S: RowSource>(source: &S, options: &ExportOptions<'_>) -> Columns {
    let names = source.names();
    Columns {
        labels: names
            .iter()
            .map(|n| options.labels.get(n).unwrap_or(n).clone())
            .collect(),
        numeric: source
            .types()
            .iter()
            .map(|t| crate::routes::support::is_numeric_type(t))
            .collect(),
        formats: names
            .iter()
            .map(|n| match options.values {
                Values::Formatted => options.formats.get(n).cloned(),
                Values::Raw => None,
            })
            .collect(),
    }
}

async fn write_csv_file<S: RowSource>(
    source: &mut S,
    options: &ExportOptions<'_>,
) -> Result<Written, ExportError> {
    let cols = columns(source, options);
    let path = TempPath::new("csv");
    let file = tokio::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&path.0)
        .await?;
    // Unlinked at once: the open file stays usable, nothing is left to clean up.
    drop(tokio::fs::remove_file(&path.0).await);
    let mut out = BufWriter::new(file);
    out.write_all(csv_export::header(&cols.labels).as_bytes())
        .await?;
    let mut rows = 0;
    let mut cut = false;
    loop {
        match next_capped(source, rows, options.cap).await? {
            Next::End => break,
            Next::Cut => {
                cut = true;
                break;
            }
            Next::Row(row) => {
                let fields: Vec<String> = row
                    .iter()
                    .enumerate()
                    .map(|(i, v)| {
                        csv_cell(
                            v,
                            cols.numeric.get(i).copied().unwrap_or(false),
                            cols.formats.get(i).and_then(Option::as_ref),
                            options.locale,
                        )
                    })
                    .collect();
                out.write_all(csv_export::line(&fields).as_bytes()).await?;
                rows += 1;
            }
        }
    }
    out.flush().await?;
    let mut file = out.into_inner();
    let len = file.seek(SeekFrom::End(0)).await?;
    file.seek(SeekFrom::Start(0)).await?;
    Ok(Written {
        file,
        len,
        rows,
        cut,
    })
}

/// Excel's longest cell text.
const XLSX_CELL_CHARS: usize = 32_767;

fn xlsx_err(e: &rust_xlsxwriter::XlsxError) -> ExportError {
    ExportError::Xlsx(e.to_string())
}

async fn write_xlsx_file<S: RowSource>(
    source: &mut S,
    options: &ExportOptions<'_>,
) -> Result<Written, ExportError> {
    use rust_xlsxwriter::{Format, Workbook};

    let cols = columns(source, options);
    let mut workbook = Workbook::new();
    // Constant-memory mode: each row goes to a temporary file as it is written.
    let sheet = workbook.add_worksheet_with_constant_memory();
    sheet.set_name("Data").map_err(|e| xlsx_err(&e))?;
    let bold = Format::new().set_bold();
    let number_formats: Vec<Option<Format>> = cols
        .formats
        .iter()
        .map(|f| {
            f.as_ref()
                .and_then(|f| excel_number_format(f, options.locale))
                .map(|code| Format::new().set_num_format(code))
        })
        .collect();
    for (i, label) in cols.labels.iter().enumerate() {
        let col = u16::try_from(i)
            .map_err(|_| ExportError::Xlsx("more columns than a sheet has".to_owned()))?;
        sheet
            .write_string_with_format(0, col, csv_export::neutralise(label), &bold)
            .map_err(|e| xlsx_err(&e))?;
    }
    let mut rows = 0_usize;
    let mut cut = false;
    loop {
        match next_capped(source, rows, options.cap).await? {
            Next::End => break,
            Next::Cut => {
                cut = true;
                break;
            }
            Next::Row(row) => {
                let r = u32::try_from(rows + 1)
                    .map_err(|_| ExportError::Xlsx("more rows than a sheet has".to_owned()))?;
                for (i, value) in row.iter().enumerate() {
                    let col = u16::try_from(i).map_err(|_| {
                        ExportError::Xlsx("more columns than a sheet has".to_owned())
                    })?;
                    let numeric = cols.numeric.get(i).copied().unwrap_or(false);
                    let written = match (value, cell_number(value, numeric)) {
                        (Value::Null, _) => Ok(&mut *sheet),
                        (_, Some(n)) => match number_formats.get(i).and_then(Option::as_ref) {
                            Some(f) => sheet.write_number_with_format(r, col, n, f),
                            None => sheet.write_number(r, col, n),
                        },
                        (Value::Bool(b), None) => sheet.write_boolean(r, col, *b),
                        (other, None) => {
                            let text = csv_export::cell_text(Some(other));
                            if text.chars().count() > XLSX_CELL_CHARS {
                                return Err(ExportError::Xlsx(
                                    "a cell is longer than Excel allows".to_owned(),
                                ));
                            }
                            sheet.write_string(r, col, text)
                        }
                    };
                    written.map_err(|e| xlsx_err(&e))?;
                }
                rows += 1;
            }
        }
    }
    let path = TempPath::new("xlsx");
    let target = path.0.clone();
    tokio::task::spawn_blocking(move || workbook.save(&target))
        .await
        .map_err(|e| ExportError::Xlsx(e.to_string()))?
        .map_err(|e| xlsx_err(&e))?;
    let mut file = tokio::fs::File::open(&path.0).await?;
    drop(tokio::fs::remove_file(&path.0).await);
    let len = file.seek(SeekFrom::End(0)).await?;
    file.seek(SeekFrom::Start(0)).await?;
    Ok(Written {
        file,
        len,
        rows,
        cut,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::io::Read;

    use serde_json::json;
    use tokio::io::AsyncReadExt;

    use super::*;

    /// Rows made on demand, so no test holds the result it counts.
    struct Generated {
        names: Vec<String>,
        types: Vec<String>,
        total: usize,
        next: usize,
        make: fn(usize) -> Vec<Value>,
    }

    impl RowSource for Generated {
        fn names(&self) -> &[String] {
            &self.names
        }
        fn types(&self) -> &[String] {
            &self.types
        }
        async fn next_row(&mut self) -> Result<Option<Vec<Value>>, ChError> {
            if self.next >= self.total {
                return Ok(None);
            }
            self.next += 1;
            Ok(Some((self.make)(self.next)))
        }
    }

    fn source(total: usize, make: fn(usize) -> Vec<Value>) -> Generated {
        Generated {
            names: vec!["name".to_owned(), "amount".to_owned()],
            types: vec!["String".to_owned(), "Int64".to_owned()],
            total,
            next: 0,
            make,
        }
    }

    fn options<'a>(
        format: FileFormat,
        values: Values,
        cap: usize,
        labels: &'a HashMap<String, String>,
        formats: &'a HashMap<String, ValueFormat>,
    ) -> ExportOptions<'a> {
        ExportOptions {
            format,
            values,
            cap,
            locale: "id-ID",
            labels,
            formats,
        }
    }

    async fn read_all(mut w: Written) -> Vec<u8> {
        let mut bytes = Vec::new();
        w.file.read_to_end(&mut bytes).await.unwrap();
        assert_eq!(bytes.len() as u64, w.len);
        bytes
    }

    fn idr() -> HashMap<String, ValueFormat> {
        HashMap::from([(
            "amount".to_owned(),
            serde_json::from_value(json!({"style":"compact","currency":"IDR","decimals":2}))
                .unwrap(),
        )])
    }

    #[tokio::test]
    async fn csv_formatted_writes_the_text_the_chart_shows_and_raw_the_plain_value() {
        let formats = idr();
        let labels = HashMap::from([("amount".to_owned(), "Amount".to_owned())]);
        let make = |i: usize| vec![json!(format!("row {i}")), json!(1_234_567)];
        let w = write_export(
            &mut source(1, make),
            &options(FileFormat::Csv, Values::Formatted, 10, &labels, &formats),
        )
        .await
        .unwrap();
        let text = String::from_utf8(read_all(w).await).unwrap();
        assert_eq!(
            text,
            "\u{feff}name,Amount\r\nrow 1,Rp 1,23 M\r\n".replace("Rp 1,23 M", "\"Rp 1,23 M\"")
        );
        let w = write_export(
            &mut source(1, make),
            &options(FileFormat::Csv, Values::Raw, 10, &labels, &formats),
        )
        .await
        .unwrap();
        let text = String::from_utf8(read_all(w).await).unwrap();
        assert_eq!(text, "\u{feff}name,Amount\r\nrow 1,1234567\r\n");
    }

    #[tokio::test]
    async fn a_quoted_64_bit_integer_is_a_number_and_a_formula_is_neutralised() {
        let (labels, formats) = (HashMap::new(), idr());
        let make = |_| vec![json!("=1+1"), json!("1234567")];
        let w = write_export(
            &mut source(1, make),
            &options(FileFormat::Csv, Values::Formatted, 10, &labels, &formats),
        )
        .await
        .unwrap();
        let text = String::from_utf8(read_all(w).await).unwrap();
        assert!(text.contains("'=1+1,\"Rp 1,23 M\""), "{text}");
    }

    #[tokio::test]
    async fn a_result_past_the_cap_is_cut_and_the_probe_row_is_not_written() {
        let (labels, formats) = (HashMap::new(), HashMap::new());
        let make = |i: usize| vec![json!("x"), json!(i)];
        let w = write_export(
            &mut source(6, make),
            &options(FileFormat::Csv, Values::Raw, 5, &labels, &formats),
        )
        .await
        .unwrap();
        assert!(w.cut);
        assert_eq!(w.rows, 5);
        let text = String::from_utf8(read_all(w).await).unwrap();
        assert_eq!(text.lines().count(), 6, "the header and five rows: {text}");
        // Exactly the cap is not a cut.
        let w = write_export(
            &mut source(5, make),
            &options(FileFormat::Csv, Values::Raw, 5, &labels, &formats),
        )
        .await
        .unwrap();
        assert!(!w.cut);
        assert_eq!(w.rows, 5);
    }

    /// 1,000,001 generated rows against the real cap: the file is cut at
    /// 1,000,000 and the source was asked for exactly one row more. The rows
    /// are made one at a time and written as they come (the source is the
    /// counter, there is no list of them).
    #[tokio::test]
    async fn a_million_and_one_rows_give_a_cut_file_of_a_million_without_holding_them() {
        let (labels, formats) = (HashMap::new(), HashMap::new());
        let make = |i: usize| vec![json!("x"), json!(i)];
        let mut src = source(EXPORT_ROW_CAP as usize + 1, make);
        let w = write_export(
            &mut src,
            &options(
                FileFormat::Csv,
                Values::Raw,
                EXPORT_ROW_CAP as usize,
                &labels,
                &formats,
            ),
        )
        .await
        .unwrap();
        assert!(w.cut);
        assert_eq!(w.rows, EXPORT_ROW_CAP as usize);
        assert_eq!(
            src.next,
            EXPORT_ROW_CAP as usize + 1,
            "one probe row, no more"
        );
        assert!(w.len > 0);
    }

    #[tokio::test]
    async fn xlsx_keeps_numbers_as_numbers_with_the_charts_number_format() {
        let (labels, formats) = (HashMap::new(), idr());
        let make = |_| vec![json!("=HYPERLINK(\"http://x\")"), json!(1_234_567)];
        let w = write_export(
            &mut source(2, make),
            &options(FileFormat::Xlsx, Values::Formatted, 10, &labels, &formats),
        )
        .await
        .unwrap();
        assert_eq!(w.rows, 2);
        let bytes = read_all(w).await;
        assert_eq!(&bytes[..2], b"PK");
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        let mut sheet = String::new();
        zip.by_name("xl/worksheets/sheet1.xml")
            .unwrap()
            .read_to_string(&mut sheet)
            .unwrap();
        // The amount is a numeric cell, not a string.
        assert!(sheet.contains("<v>1234567</v>"), "{sheet}");
        // The formula-looking text is a string cell with an apostrophe in front.
        assert!(!sheet.contains("<f>"), "{sheet}");
        let mut shared = String::new();
        zip.by_name("xl/sharedStrings.xml")
            .map_or_else(|_| Ok(0), |mut f| f.read_to_string(&mut shared))
            .unwrap();
        let mut styles = String::new();
        zip.by_name("xl/styles.xml")
            .unwrap()
            .read_to_string(&mut styles)
            .unwrap();
        assert!(styles.contains("Rp"), "{styles}");
        assert!(
            sheet.contains("'=HYPERLINK") || shared.contains("'=HYPERLINK"),
            "{sheet} {shared}"
        );
    }

    #[tokio::test]
    async fn raw_xlsx_has_no_number_format_and_a_cell_longer_than_excel_allows_is_refused() {
        let (labels, formats) = (HashMap::new(), idr());
        let make = |_| vec![json!("a"), json!(5)];
        let w = write_export(
            &mut source(1, make),
            &options(FileFormat::Xlsx, Values::Raw, 10, &labels, &formats),
        )
        .await
        .unwrap();
        let bytes = read_all(w).await;
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        let mut styles = String::new();
        zip.by_name("xl/styles.xml")
            .unwrap()
            .read_to_string(&mut styles)
            .unwrap();
        assert!(!styles.contains("Rp"), "{styles}");

        let long = |_| vec![json!("a".repeat(40_000)), json!(1)];
        let err = write_export(
            &mut source(1, long),
            &options(FileFormat::Xlsx, Values::Raw, 10, &labels, &formats),
        )
        .await;
        assert!(matches!(err, Err(ExportError::Xlsx(_))));
    }

    /// R5: a quoted 64-bit integer in a numeric column keeps its sign unchanged
    /// in "raw" CSV; a text column starting with `-` is still neutralised.
    #[tokio::test]
    async fn a_negative_quoted_integer_is_written_unchanged_and_text_is_still_neutralised() {
        let (labels, formats) = (HashMap::new(), HashMap::new());
        let make = |_| vec![json!("-5"), json!("-5")];
        let mut src = Generated {
            names: vec!["code".to_owned(), "amount".to_owned()],
            types: vec!["String".to_owned(), "Int64".to_owned()],
            total: 1,
            next: 0,
            make,
        };
        let w = write_export(
            &mut src,
            &options(FileFormat::Csv, Values::Raw, 10, &labels, &formats),
        )
        .await
        .unwrap();
        let text = String::from_utf8(read_all(w).await).unwrap();
        assert!(text.ends_with("code,amount\r\n'-5,-5\r\n"), "{text:?}");
    }

    #[test]
    fn a_long_integer_stays_text_and_a_short_one_is_a_number() {
        assert_eq!(numeric_string("12345"), Some(12345.0));
        assert_eq!(numeric_string("1234567890123456789"), None);
        assert_eq!(numeric_string("abc"), None);
    }
}
