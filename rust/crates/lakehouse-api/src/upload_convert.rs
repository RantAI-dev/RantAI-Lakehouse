//! The child process that reads an uploaded workbook or a Parquet file
//! (`SEC-22`).
//!
//! # Why a child process and not a function
//!
//! Reading a workbook or a Parquet file is the one thing an upload can ask
//! the API to do that the API cannot bound from the inside. A file DECLARES
//! how much memory its reader will want — a workbook's shared-strings header,
//! a Parquet page's size — and the reader believes the declaration. When the
//! declaration is a lie, the allocation fails, and Rust ABORTS the process
//! on an allocation failure: `catch_unwind` cannot catch that, because an
//! abort is not a panic. One crafted upload could end the API for every
//! tenant (`SEC-22`; the BLOCKER of `PR #76`'s review, which merged without
//! the fix).
//!
//! So the API never parses an uploaded workbook or Parquet file in its own
//! process. It starts this binary again as a child, writes the file to the
//! child's standard input, and reads the answer from the child's standard
//! output. The child sets its own resource limits BEFORE it reads anything,
//! so an allocation failure, a stack overflow or a CPU-limit signal ends the
//! CHILD and never the API.
//!
//! # The protocol
//!
//! `lakehouse-api inspect-upload <operation> [argument]`:
//!
//! - the file arrives on the child's standard input;
//! - the child writes ONE JSON line to its standard output: the answer, or
//!   the refusal;
//! - for the two operations that convert, the text follows that line, to
//!   the end of the output;
//! - the child exits with a code from [`exit`].
//!
//! The JSON line comes first so that the caller learns the answer (which
//! sheet, which columns, whether the sheet was empty) before it reads the
//! text, and so that a refusal is a complete answer on its own.
//!
//! # The limits
//!
//! The child sets [`Resource::RLIMIT_DATA`] and [`Resource::RLIMIT_CPU`] on
//! itself, so the limit holds even if the caller forgets. `RLIMIT_DATA` and
//! not `RLIMIT_AS`: the address-space limit counts the binary's own
//! mappings, the stacks and the allocator's arenas, which are not the
//! parser's memory, and at a limit a real file needs the child cannot even
//! start (the measurement is in [`set_limits`]). The memory limit comes from
//! the environment ([`MEMORY_ENV`]), so the API can set it per deployment;
//! the CPU limit is [`DEFAULT_CPU_SECONDS`].
//!
//! The text a conversion produces is capped at [`MAX_OUTPUT_BYTES`]: the
//! child stops writing and exits with [`exit::TOO_MUCH_TEXT`], and the caller
//! throws the partial object away rather than keeping it.

// The API side of this protocol — the runner that starts the child, and the
// sentences it answers a refusal with — lands in `SEC-22` T3. Until then the
// binary compiles the parts of this module that only the API side uses, and
// its `dead_code` lint would say so on every build. The allow goes with T3.
#![allow(
    dead_code,
    reason = "the API side of the protocol (the runner and the sentences) lands in SEC-22 T3"
)]

use std::io::{Read, Write};

use axum::body::Bytes;
use serde::{Deserialize, Serialize};

use crate::upload_parquet::{self, ParquetFileError, ParquetInfo};
use crate::upload_workbook::{self, WorkbookError, WorkbookInfo};

/// The environment variable the child's memory limit comes from. The API
/// sets it on the child it starts; the child reads it for itself, so the
/// limit holds whatever the caller does (`SEC-22` D1).
pub const MEMORY_ENV: &str = "LAKEHOUSE_CONVERT_MEMORY_MIB";

/// The child's memory limit when nothing says otherwise: 1 GiB, the number
/// `SEC-22` D7 starts from. `SEC-22` T5 measures the real peak and sets the
/// default from it.
pub const DEFAULT_MEMORY_MIB: u64 = 1024;

/// The child's CPU limit, in seconds: 60. The API's own wall-clock limit is
/// 90 s (`SEC-22` D3), so the CPU limit is the one that ends a conversion
/// that spins.
pub const DEFAULT_CPU_SECONDS: u64 = 60;

/// The most text a conversion may produce: 512 MiB (`SEC-22` D6, the number
/// `upload_parquet::MAX_OUTPUT_BYTES` already used).
pub const MAX_OUTPUT_BYTES: u64 = crate::upload_parquet::MAX_OUTPUT_BYTES as u64;

/// The most the child reads from its standard input: 64 MiB. The API's own
/// upload cap is 50 MiB (`routes::uploads::MAX_UPLOAD_BYTES`), so this is
/// the backstop for a caller that is not the API.
pub const MAX_INPUT_BYTES: u64 = 64 * 1024 * 1024;

/// The sentence for a file that needed more memory or more time than a read
/// is allowed (`SEC-22` D4). The cases cannot be told apart reliably — a
/// signal, an allocation failure and a timeout look the same from here — so
/// there is one sentence for all of them.
pub const NEEDS_MORE_MEMORY: &str = "This file needs more memory or time to read than an upload is allowed. Save it as CSV and upload that instead.";

/// The sentence for a request that arrives while every conversion slot is
/// taken (`SEC-22` D4). 429, with `Retry-After: 5`.
pub const TOO_MANY_CONVERSIONS: &str =
    "Too many files are being read right now. Try again in a few seconds.";

/// The exit codes of the child (`SEC-22` D2: a fixed table). Every code
/// says what happened, so the caller never has to read the child's output to
/// know why it failed.
pub mod exit {
    /// The operation succeeded.
    pub const OK: i32 = 0;
    /// The file could not be read.
    pub const UNREADABLE: i32 = 2;
    /// The file is encrypted.
    pub const ENCRYPTED: i32 = 3;
    /// The workbook has no sheet of cells.
    pub const NO_SHEETS: i32 = 4;
    /// The Parquet file has no columns.
    pub const NO_COLUMNS: i32 = 5;
    /// The workbook has no sheet with that name.
    pub const UNKNOWN_SHEET: i32 = 6;
    /// The file is past the cell cap.
    pub const TOO_LARGE: i32 = 7;
    /// The text would be past the output cap.
    pub const TOO_MUCH_TEXT: i32 = 8;
    /// A column holds values that have no text form.
    pub const UNSUPPORTED_COLUMN: i32 = 9;
    /// The input is past the cap.
    pub const INPUT_TOO_LARGE: i32 = 10;
    /// The child ran out of memory or out of time.
    pub const RESOURCE_LIMIT: i32 = 11;

    /// Every code, for the tests that pin the table.
    pub const ALL: [i32; 11] = [
        OK,
        UNREADABLE,
        ENCRYPTED,
        NO_SHEETS,
        NO_COLUMNS,
        UNKNOWN_SHEET,
        TOO_LARGE,
        TOO_MUCH_TEXT,
        UNSUPPORTED_COLUMN,
        INPUT_TOO_LARGE,
        RESOURCE_LIMIT,
    ];
}

/// What the child was asked to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Operation {
    /// The sheets of a workbook.
    ListSheets,
    /// One sheet of a workbook as text: the named sheet, or the default one
    /// when no name is given.
    ConvertSheet(Option<String>),
    /// What a Parquet file's footer says.
    InspectParquet,
    /// The first rows of a Parquet file, as text.
    PreviewParquet(usize),
}

impl Operation {
    /// The operation the arguments name, or `None` when they name none.
    ///
    /// An unknown operation is not an error of its own: the caller reads it
    /// as "the file could not be read" (`SEC-22` D2), so a mistyped command
    /// cannot be mistaken for a successful read.
    #[must_use]
    pub fn parse(args: &[String]) -> Option<Self> {
        let (name, rest) = args.split_first()?;
        match name.as_str() {
            "list-sheets" => Some(Self::ListSheets),
            "convert-sheet" => Some(Self::ConvertSheet(rest.first().cloned())),
            "inspect-parquet" => Some(Self::InspectParquet),
            "preview-parquet" => {
                let rows = rest.first()?.parse::<usize>().ok()?;
                Some(Self::PreviewParquet(rows))
            }
            _ => None,
        }
    }

    /// The arguments this operation is started with.
    #[must_use]
    pub fn arguments(&self) -> Vec<String> {
        match self {
            Self::ListSheets => vec!["list-sheets".to_owned()],
            Self::ConvertSheet(None) => vec!["convert-sheet".to_owned()],
            Self::ConvertSheet(Some(name)) => vec!["convert-sheet".to_owned(), name.clone()],
            Self::InspectParquet => vec!["inspect-parquet".to_owned()],
            Self::PreviewParquet(rows) => {
                vec!["preview-parquet".to_owned(), rows.to_string()]
            }
        }
    }
}

/// Why a file could not be read, as the child says it. The API turns one of
/// these into the sentence the user sees (`SEC-22` D4: a fixed sentence, and
/// nothing the library wrote).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Refusal {
    /// The bytes are not a file this reader can open.
    Unreadable,
    /// The file is encrypted.
    Encrypted,
    /// The workbook has no sheet of cells.
    NoSheets,
    /// The Parquet file has no columns.
    NoColumns,
    /// The workbook has no sheet with that name.
    UnknownSheet,
    /// The file is past the cell cap.
    TooLarge,
    /// The text would be past the output cap.
    TooMuchText,
    /// A column holds values that have no text form. The name and the type
    /// are the file's own, cleaned by the reader that named them.
    UnsupportedColumn {
        /// The first such column's name.
        column: String,
        /// The type it holds: `binary`, `list`, `map`, `struct`, ...
        type_name: String,
    },
    /// The input is past the cap.
    InputTooLarge,
    /// The child ran out of memory or out of time.
    ResourceLimit,
}

impl Refusal {
    /// The exit code this refusal is reported with.
    #[must_use]
    pub fn exit_code(&self) -> i32 {
        match self {
            Self::Unreadable => exit::UNREADABLE,
            Self::Encrypted => exit::ENCRYPTED,
            Self::NoSheets => exit::NO_SHEETS,
            Self::NoColumns => exit::NO_COLUMNS,
            Self::UnknownSheet => exit::UNKNOWN_SHEET,
            Self::TooLarge => exit::TOO_LARGE,
            Self::TooMuchText => exit::TOO_MUCH_TEXT,
            Self::UnsupportedColumn { .. } => exit::UNSUPPORTED_COLUMN,
            Self::InputTooLarge => exit::INPUT_TOO_LARGE,
            Self::ResourceLimit => exit::RESOURCE_LIMIT,
        }
    }

    /// The refusal an exit code stands for, or `None` for a code that is
    /// not a refusal (`exit::OK`).
    #[must_use]
    pub fn for_exit_code(code: i32) -> Option<Self> {
        Some(match code {
            exit::UNREADABLE => Self::Unreadable,
            exit::ENCRYPTED => Self::Encrypted,
            exit::NO_SHEETS => Self::NoSheets,
            exit::NO_COLUMNS => Self::NoColumns,
            exit::UNKNOWN_SHEET => Self::UnknownSheet,
            exit::TOO_LARGE => Self::TooLarge,
            exit::TOO_MUCH_TEXT => Self::TooMuchText,
            exit::UNSUPPORTED_COLUMN => Self::UnsupportedColumn {
                column: String::new(),
                type_name: String::new(),
            },
            exit::INPUT_TOO_LARGE => Self::InputTooLarge,
            exit::RESOURCE_LIMIT => Self::ResourceLimit,
            _ => return None,
        })
    }
}

/// What kind of file the API is reading. The sentences for a refusal are
/// the file's own: a workbook that does not open and a Parquet file that
/// does not open are two different sentences.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    /// An `.xls` or `.xlsx` workbook.
    Workbook,
    /// A `.parquet` file.
    Parquet,
}

impl FileKind {
    /// The sentence this refusal is shown as (`SEC-22` D4: a fixed sentence,
    /// and nothing the child or the library wrote).
    #[must_use]
    pub fn sentence(self, refusal: &Refusal) -> String {
        match refusal {
            Refusal::Unreadable => match self {
                Self::Workbook => WorkbookError::Unreadable.to_string(),
                Self::Parquet => ParquetFileError::Unreadable.to_string(),
            },
            Refusal::Encrypted => match self {
                Self::Workbook => WorkbookError::Encrypted.to_string(),
                Self::Parquet => ParquetFileError::Encrypted.to_string(),
            },
            Refusal::NoSheets => WorkbookError::NoSheets.to_string(),
            Refusal::NoColumns => ParquetFileError::NoColumns.to_string(),
            Refusal::UnknownSheet => WorkbookError::UnknownSheet.to_string(),
            Refusal::TooLarge => match self {
                Self::Workbook => WorkbookError::TooLarge.to_string(),
                Self::Parquet => ParquetFileError::TooLarge.to_string(),
            },
            Refusal::TooMuchText => ParquetFileError::TooMuchText.to_string(),
            Refusal::UnsupportedColumn { column, type_name } => ParquetFileError::Unsupported {
                column: upload_parquet::quoted_name(column),
                type_name: upload_parquet::type_word(type_name),
            }
            .to_string(),
            Refusal::InputTooLarge => crate::routes::uploads::TOO_LARGE.to_owned(),
            Refusal::ResourceLimit => NEEDS_MORE_MEMORY.to_owned(),
        }
    }
}

/// What the child writes as its JSON line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Answer {
    /// The sheets of a workbook.
    Sheets {
        /// The workbook's sheets and the one read when none is asked for.
        info: WorkbookInfo,
    },
    /// One sheet of a workbook, read.
    Sheet {
        /// The sheet that was read, as the workbook spells it.
        sheet: String,
        /// The workbook's sheets and the one read when none is asked for.
        info: WorkbookInfo,
        /// Whether the sheet has no cells at all.
        empty: bool,
    },
    /// What a Parquet file's footer says.
    Parquet {
        /// The file's columns and its row count.
        info: ParquetInfo,
    },
    /// The file could not be read.
    Refusal {
        /// Why.
        refusal: Refusal,
    },
}

/// The limits the child runs under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChildLimits {
    /// The memory limit, in bytes.
    pub memory_bytes: u64,
    /// The CPU limit, in seconds.
    pub cpu_seconds: u64,
    /// The most text a conversion may produce, in bytes.
    pub max_output_bytes: u64,
    /// The most the child reads from its standard input, in bytes.
    pub max_input_bytes: u64,
}

impl Default for ChildLimits {
    fn default() -> Self {
        Self {
            memory_bytes: DEFAULT_MEMORY_MIB * 1024 * 1024,
            cpu_seconds: DEFAULT_CPU_SECONDS,
            max_output_bytes: MAX_OUTPUT_BYTES,
            max_input_bytes: MAX_INPUT_BYTES,
        }
    }
}

impl ChildLimits {
    /// The limits the child runs under, with the memory limit from the
    /// environment ([`MEMORY_ENV`]) when it says something usable.
    #[must_use]
    pub fn from_env() -> Self {
        let raw = std::env::var(MEMORY_ENV).ok();
        Self::with_memory(raw.as_deref())
    }

    /// The limits the child runs under, with the memory limit `raw` names,
    /// in mebibytes. Anything that is not a whole number of 1 or more keeps
    /// the default: a limit that cannot be read is no limit at all.
    #[must_use]
    pub fn with_memory(raw: Option<&str>) -> Self {
        let mut limits = Self::default();
        if let Some(mib) = raw
            .map(str::trim)
            .and_then(|raw| raw.parse::<u64>().ok())
            .filter(|mib| *mib > 0)
        {
            limits.memory_bytes = mib.saturating_mul(1024 * 1024);
        }
        limits
    }
}

/// Run the child, if this process was started as one.
///
/// Returns the exit code when the arguments name the child's command, and
/// `None` when they do not — which is how the API's own start-up tells the
/// two apart (`SEC-22` D1: the child branch comes before the runtime and
/// before the configuration, so the child needs no environment, starts no
/// threads and opens no connection).
#[must_use]
pub fn run_child_if_asked(args: &[String]) -> Option<i32> {
    if args.get(1).map(String::as_str) != Some("inspect-upload") {
        return None;
    }
    let limits = ChildLimits::from_env();
    if let Err(err) = set_limits(&limits) {
        eprintln!("[inspect-upload] the limits could not be set: {err}");
        return Some(exit::RESOURCE_LIMIT);
    }
    let input = match read_input(limits.max_input_bytes) {
        Ok(bytes) => bytes,
        Err(refusal) => {
            let mut out = std::io::stdout().lock();
            let _written = write_answer(
                &mut out,
                &Answer::Refusal {
                    refusal: refusal.clone(),
                },
            );
            return Some(refusal.exit_code());
        }
    };
    let operation = Operation::parse(args.get(2..).unwrap_or_default());
    let mut out = std::io::stdout().lock();
    let mut buffered = std::io::BufWriter::new(&mut out);
    let code = if let Some(operation) = operation {
        run(&operation, &input, &limits, &mut buffered)
    } else {
        // An unknown operation is the "unreadable" code (`SEC-22` D2).
        let _written = write_answer(
            &mut buffered,
            &Answer::Refusal {
                refusal: Refusal::Unreadable,
            },
        );
        exit::UNREADABLE
    };
    let _flushed = buffered.flush();
    Some(code)
}

/// Do the operation on `input` and write the answer, and the text for the
/// operations that convert, to `out`. Returns the exit code.
///
/// The answer is written first, so a caller learns what it is reading
/// before it reads the text, and so that a refusal is a complete answer.
pub fn run(operation: &Operation, input: &[u8], limits: &ChildLimits, out: &mut dyn Write) -> i32 {
    match operation {
        Operation::ListSheets => match upload_workbook::list_sheets(input) {
            Ok(info) => match write_answer(out, &Answer::Sheets { info }) {
                Ok(()) => exit::OK,
                Err(_) => exit::UNREADABLE,
            },
            Err(err) => refuse(out, FileKind::Workbook, &workbook_refusal(err)),
        },
        Operation::ConvertSheet(sheet) => {
            let read = match upload_workbook::read_sheet(input, sheet.as_deref()) {
                Ok(read) => read,
                Err(err) => return refuse(out, FileKind::Workbook, &workbook_refusal(err)),
            };
            let answer = Answer::Sheet {
                sheet: read.data.name().to_owned(),
                info: read.info,
                empty: read.data.is_empty(),
            };
            if write_answer(out, &answer).is_err() {
                return exit::UNREADABLE;
            }
            match write_sheet(out, &read.data, limits.max_output_bytes) {
                Ok(()) => exit::OK,
                Err(WriteFailure::TooMuchText) => exit::TOO_MUCH_TEXT,
                Err(WriteFailure::Failed) => exit::UNREADABLE,
            }
        }
        Operation::InspectParquet => {
            let bytes = Bytes::copy_from_slice(input);
            match upload_parquet::inspect(&bytes) {
                Ok(info) => match write_answer(out, &Answer::Parquet { info }) {
                    Ok(()) => exit::OK,
                    Err(_) => exit::UNREADABLE,
                },
                Err(err) => refuse(out, FileKind::Parquet, &parquet_refusal(err)),
            }
        }
        Operation::PreviewParquet(rows) => {
            let bytes = Bytes::copy_from_slice(input);
            let converted = match upload_parquet::convert(&bytes, Some(*rows)) {
                Ok(converted) => converted,
                Err(err) => return refuse(out, FileKind::Parquet, &parquet_refusal(err)),
            };
            let answer = Answer::Parquet {
                info: converted.info.clone(),
            };
            if write_answer(out, &answer).is_err() {
                return exit::UNREADABLE;
            }
            match write_bytes(out, &converted.csv, limits.max_output_bytes) {
                Ok(()) => exit::OK,
                Err(WriteFailure::TooMuchText) => exit::TOO_MUCH_TEXT,
                Err(WriteFailure::Failed) => exit::UNREADABLE,
            }
        }
    }
}

/// Write the refusal and return its exit code.
///
/// The sentence the user will see goes to the standard error as well: it is
/// the one line of the child's output that says what happened, and the API
/// logs it (`SEC-22` D3).
fn refuse(out: &mut dyn Write, kind: FileKind, refusal: &Refusal) -> i32 {
    let _written = write_answer(
        out,
        &Answer::Refusal {
            refusal: refusal.clone(),
        },
    );
    eprintln!("[inspect-upload] refused: {}", kind.sentence(refusal));
    refusal.exit_code()
}

/// The refusal a workbook error is.
fn workbook_refusal(err: WorkbookError) -> Refusal {
    match err {
        WorkbookError::Unreadable => Refusal::Unreadable,
        WorkbookError::Encrypted => Refusal::Encrypted,
        WorkbookError::NoSheets => Refusal::NoSheets,
        WorkbookError::UnknownSheet => Refusal::UnknownSheet,
        WorkbookError::TooLarge => Refusal::TooLarge,
    }
}

/// The refusal a Parquet error is.
fn parquet_refusal(err: ParquetFileError) -> Refusal {
    match err {
        ParquetFileError::Unreadable => Refusal::Unreadable,
        ParquetFileError::Encrypted => Refusal::Encrypted,
        ParquetFileError::NoColumns => Refusal::NoColumns,
        ParquetFileError::Unsupported { column, type_name } => Refusal::UnsupportedColumn {
            column,
            type_name: type_name.to_owned(),
        },
        ParquetFileError::TooLarge => Refusal::TooLarge,
        ParquetFileError::TooMuchText => Refusal::TooMuchText,
    }
}

/// Write the answer as one JSON line.
fn write_answer(out: &mut dyn Write, answer: &Answer) -> std::io::Result<()> {
    serde_json::to_writer(&mut *out, answer)?;
    out.write_all(b"\n")
}

/// Why a write of the converted text failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WriteFailure {
    /// The text is past the cap.
    TooMuchText,
    /// The output could not be written.
    Failed,
}

/// Write the sheet's text, stopping at `cap` bytes.
fn write_sheet(
    out: &mut dyn Write,
    data: &upload_workbook::SheetData,
    cap: u64,
) -> Result<(), WriteFailure> {
    let mut capped = Capped::new(out, cap);
    data.write_csv(&mut capped, None)
        .map_err(|err| match err.kind() {
            std::io::ErrorKind::Other => WriteFailure::TooMuchText,
            _ => WriteFailure::Failed,
        })
}

/// Write the bytes, stopping at `cap` bytes.
fn write_bytes(out: &mut dyn Write, bytes: &[u8], cap: u64) -> Result<(), WriteFailure> {
    let mut capped = Capped::new(out, cap);
    capped.write_all(bytes).map_err(|err| match err.kind() {
        std::io::ErrorKind::Other => WriteFailure::TooMuchText,
        _ => WriteFailure::Failed,
    })
}

/// A writer that stops at a byte cap: the child writes no more than the cap
/// allows, and says so (`SEC-22` D6) rather than producing an object nobody
/// can use.
struct Capped<'a, W: Write + ?Sized> {
    inner: &'a mut W,
    written: u64,
    cap: u64,
}

impl<'a, W: Write + ?Sized> Capped<'a, W> {
    fn new(inner: &'a mut W, cap: u64) -> Self {
        Self {
            inner,
            written: 0,
            cap,
        }
    }
}

impl<W: Write + ?Sized> Write for Capped<'_, W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let wanted = u64::try_from(buf.len()).unwrap_or(u64::MAX);
        if self.written.saturating_add(wanted) > self.cap {
            return Err(std::io::Error::other("the converted text is past its cap"));
        }
        self.written = self.written.saturating_add(wanted);
        self.inner.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

/// Read the file from the standard input, at most `cap` bytes.
fn read_input(cap: u64) -> Result<Vec<u8>, Refusal> {
    let mut bytes = Vec::new();
    let stdin = std::io::stdin();
    let mut limited = stdin.lock().take(cap.saturating_add(1));
    limited
        .read_to_end(&mut bytes)
        .map_err(|_err| Refusal::Unreadable)?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > cap {
        return Err(Refusal::InputTooLarge);
    }
    Ok(bytes)
}

/// Set the limits the child runs under.
///
/// The child sets them on ITSELF, so the limit holds even if the caller
/// forgets: `Command::pre_exec` would be the other way, and it is `unsafe`.
///
/// `RLIMIT_DATA`, not `RLIMIT_AS` (`SEC-22` D1: "or `RLIMIT_DATA` if T2
/// shows `AS` counts reservations it should not"). Measured on this VM on
/// 2026-10-28 (kernel 6.8), with the T1 hostile workbook:
///
/// - `RLIMIT_AS` of 64 MiB: the child dies of `SIGSEGV` (exit 139) before
///   it reads anything, even on a valid workbook — the limit counts the
///   binary's own mappings, the stacks and the allocator's arenas, none of
///   which are the parser's memory.
/// - `RLIMIT_DATA` of 64 MiB: the child reads a valid workbook and refuses
///   the hostile one, and the hostile one ends it with
///   `memory allocation of 48000000000 bytes failed` — the same abort the
///   `RLIMIT_AS` limit produced, at a limit the child can actually run
///   under.
///
/// So the limit bounds the HEAP, which is what a crafted file asks for, and
/// leaves the process's own address space alone.
fn set_limits(limits: &ChildLimits) -> Result<(), String> {
    use nix::sys::resource::{Resource, setrlimit};

    let memory = limits.memory_bytes;
    let cpu = limits.cpu_seconds;
    setrlimit(Resource::RLIMIT_DATA, memory, memory)
        .map_err(|err| format!("RLIMIT_DATA: {err}"))?;
    setrlimit(Resource::RLIMIT_CPU, cpu, cpu).map_err(|err| format!("RLIMIT_CPU: {err}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    const STOCK_XLSX: &[u8] = include_bytes!("../../../../ops/fixtures/workbooks/stock.xlsx");
    const ORDERS: &[u8] = include_bytes!("../../../../ops/fixtures/parquet/orders.parquet");
    const BINARY: &[u8] = include_bytes!("../../../../ops/fixtures/parquet/binary_column.parquet");

    fn limits() -> ChildLimits {
        ChildLimits::default()
    }

    /// The answer line of an output, and the text after it.
    fn split(output: &[u8]) -> (Answer, Vec<u8>) {
        let newline = output
            .iter()
            .position(|byte| *byte == b'\n')
            .expect("the answer is one line");
        let answer: Answer =
            serde_json::from_slice(&output[..newline]).expect("the answer is JSON");
        (answer, output[newline + 1..].to_vec())
    }

    /// Every code in the table, with the refusal it stands for.
    const TABLE: [(i32, Refusal); 10] = [
        (exit::UNREADABLE, Refusal::Unreadable),
        (exit::ENCRYPTED, Refusal::Encrypted),
        (exit::NO_SHEETS, Refusal::NoSheets),
        (exit::NO_COLUMNS, Refusal::NoColumns),
        (exit::UNKNOWN_SHEET, Refusal::UnknownSheet),
        (exit::TOO_LARGE, Refusal::TooLarge),
        (exit::TOO_MUCH_TEXT, Refusal::TooMuchText),
        (
            exit::UNSUPPORTED_COLUMN,
            Refusal::UnsupportedColumn {
                column: String::new(),
                type_name: String::new(),
            },
        ),
        (exit::INPUT_TOO_LARGE, Refusal::InputTooLarge),
        (exit::RESOURCE_LIMIT, Refusal::ResourceLimit),
    ];

    /// Every code names one refusal, and the refusal names the code back:
    /// the table cannot have a code without a refusal or two refusals with
    /// one code.
    #[test]
    fn every_code_maps_to_its_refusal_and_back() {
        for (code, refusal) in TABLE {
            assert_eq!(refusal.exit_code(), code, "{refusal:?}");
            let back = Refusal::for_exit_code(code).expect("the code is in the table");
            assert_eq!(back.exit_code(), code, "{refusal:?}");
        }
        assert_eq!(Refusal::for_exit_code(exit::OK), None);
        assert_eq!(Refusal::for_exit_code(99), None);
        let mut codes: Vec<i32> = TABLE.iter().map(|(code, _)| *code).collect();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), TABLE.len(), "no code stands for two refusals");
    }

    /// Every refusal has the sentence it is shown as, and the sentence is
    /// the file's own (`SEC-22` D4).
    #[test]
    fn every_refusal_has_its_sentence() {
        assert_eq!(
            FileKind::Workbook.sentence(&Refusal::Unreadable),
            "This file could not be opened as an Excel workbook (.xls or .xlsx)."
        );
        assert_eq!(
            FileKind::Parquet.sentence(&Refusal::Unreadable),
            "This file could not be opened as a Parquet file."
        );
        assert_eq!(
            FileKind::Workbook.sentence(&Refusal::Encrypted),
            "This workbook is password protected, so it cannot be read. Remove the password and upload it again."
        );
        assert_eq!(
            FileKind::Parquet.sentence(&Refusal::Encrypted),
            "This Parquet file is encrypted, so it cannot be read. Upload an unencrypted copy."
        );
        assert_eq!(
            FileKind::Workbook.sentence(&Refusal::NoSheets),
            "This workbook has no sheet that can be read."
        );
        assert_eq!(
            FileKind::Parquet.sentence(&Refusal::NoColumns),
            "This Parquet file has no columns."
        );
        assert_eq!(
            FileKind::Workbook.sentence(&Refusal::UnknownSheet),
            "The workbook has no sheet with that name."
        );
        assert_eq!(
            FileKind::Workbook.sentence(&Refusal::TooLarge),
            "That sheet is larger than the limit of 5,000,000 cells."
        );
        assert_eq!(
            FileKind::Parquet.sentence(&Refusal::TooLarge),
            "That file is larger than the limit of 5,000,000 cells (rows times columns)."
        );
        assert_eq!(
            FileKind::Parquet.sentence(&Refusal::TooMuchText),
            "That file holds more text than can be converted (the limit is 512 MB of text)."
        );
        assert_eq!(
            FileKind::Parquet.sentence(&Refusal::UnsupportedColumn {
                column: "blob".to_owned(),
                type_name: "binary".to_owned(),
            }),
            "Column \"blob\" holds binary values, which cannot be loaded as text. Remove that column and upload the file again."
        );
        assert_eq!(
            FileKind::Workbook.sentence(&Refusal::InputTooLarge),
            "The file is larger than the 50 MB limit."
        );
        assert_eq!(
            FileKind::Workbook.sentence(&Refusal::ResourceLimit),
            "This file needs more memory or time to read than an upload is allowed. Save it as CSV and upload that instead."
        );
    }

    /// A type word that is not one of ours is not quoted as one of ours.
    #[test]
    fn a_type_word_that_is_not_ours_is_not_quoted_as_one() {
        assert_eq!(
            FileKind::Parquet.sentence(&Refusal::UnsupportedColumn {
                column: "x".to_owned(),
                type_name: "evil\"] was loaded".to_owned(),
            }),
            "Column \"x\" holds unsupported values, which cannot be loaded as text. Remove that column and upload the file again."
        );
    }

    /// An operation that is not one of ours is the "unreadable" code
    /// (`SEC-22` D2): a mistyped command cannot be read as a successful read.
    #[test]
    fn an_unknown_operation_exits_with_the_unreadable_code() {
        let mut out: Vec<u8> = Vec::new();
        let code = run(
            &Operation::ListSheets,
            b"not a workbook",
            &limits(),
            &mut out,
        );
        assert_eq!(code, exit::UNREADABLE);
        let (answer, text) = split(&out);
        assert_eq!(
            answer,
            Answer::Refusal {
                refusal: Refusal::Unreadable
            }
        );
        assert!(text.is_empty(), "a refusal writes no text");
        assert_eq!(Operation::parse(&["nope".to_owned()]), None);
        assert_eq!(Operation::parse(&["preview-parquet".to_owned()]), None);
    }

    /// The operation the arguments name is the operation that runs.
    #[test]
    fn the_operations_are_the_ones_the_arguments_name() {
        assert_eq!(
            Operation::parse(&["list-sheets".to_owned()]),
            Some(Operation::ListSheets)
        );
        assert_eq!(
            Operation::parse(&["convert-sheet".to_owned()]),
            Some(Operation::ConvertSheet(None))
        );
        assert_eq!(
            Operation::parse(&["convert-sheet".to_owned(), "Quirks".to_owned()]),
            Some(Operation::ConvertSheet(Some("Quirks".to_owned())))
        );
        assert_eq!(
            Operation::parse(&["inspect-parquet".to_owned()]),
            Some(Operation::InspectParquet)
        );
        assert_eq!(
            Operation::parse(&["preview-parquet".to_owned(), "10".to_owned()]),
            Some(Operation::PreviewParquet(10))
        );
    }

    /// The sheets of a workbook, and the sheet that was read.
    #[test]
    fn the_child_answers_with_the_sheets_and_the_text() {
        let mut out: Vec<u8> = Vec::new();
        let code = run(&Operation::ListSheets, STOCK_XLSX, &limits(), &mut out);
        assert_eq!(code, exit::OK);
        let (answer, text) = split(&out);
        let Answer::Sheets { info } = answer else {
            panic!("the answer is the sheets: {answer:?}");
        };
        assert_eq!(info.default_sheet, "Stock");
        assert_eq!(info.sheets.len(), 5);
        assert!(text.is_empty(), "listing the sheets writes no text");

        let mut out: Vec<u8> = Vec::new();
        let code = run(
            &Operation::ConvertSheet(Some("Quirks".to_owned())),
            STOCK_XLSX,
            &limits(),
            &mut out,
        );
        assert_eq!(code, exit::OK);
        let (answer, text) = split(&out);
        let Answer::Sheet { sheet, empty, .. } = answer else {
            panic!("the answer is the sheet: {answer:?}");
        };
        assert_eq!(sheet, "Quirks");
        assert!(!empty);
        assert!(String::from_utf8(text).unwrap().starts_with("case,value\n"));
    }

    /// A sheet the workbook does not have is the workbook's own refusal.
    #[test]
    fn a_sheet_that_is_not_there_is_refused() {
        let mut out: Vec<u8> = Vec::new();
        let code = run(
            &Operation::ConvertSheet(Some("Nope".to_owned())),
            STOCK_XLSX,
            &limits(),
            &mut out,
        );
        assert_eq!(code, exit::UNKNOWN_SHEET);
        let (answer, _) = split(&out);
        assert_eq!(
            answer,
            Answer::Refusal {
                refusal: Refusal::UnknownSheet
            }
        );
    }

    /// A Parquet file's footer, and its first rows as text.
    #[test]
    fn the_child_answers_with_a_parquet_file_and_its_rows() {
        let mut out: Vec<u8> = Vec::new();
        let code = run(&Operation::InspectParquet, ORDERS, &limits(), &mut out);
        assert_eq!(code, exit::OK);
        let (answer, text) = split(&out);
        let Answer::Parquet { info } = answer else {
            panic!("the answer is the footer: {answer:?}");
        };
        assert_eq!(info.rows, 3);
        assert!(text.is_empty(), "the footer is no text");

        let mut out: Vec<u8> = Vec::new();
        let code = run(&Operation::PreviewParquet(10), ORDERS, &limits(), &mut out);
        assert_eq!(code, exit::OK);
        let (answer, text) = split(&out);
        let Answer::Parquet { info } = answer else {
            panic!("the answer is the footer: {answer:?}");
        };
        assert_eq!(info.rows, 3);
        assert!(
            String::from_utf8(text)
                .unwrap()
                .starts_with("sku,name,qty,price,received\n")
        );
    }

    /// A column with no text form is refused, and the refusal names it.
    #[test]
    fn a_column_with_no_text_form_is_refused_by_name() {
        let mut out: Vec<u8> = Vec::new();
        let code = run(&Operation::InspectParquet, BINARY, &limits(), &mut out);
        assert_eq!(code, exit::UNSUPPORTED_COLUMN);
        let (answer, _) = split(&out);
        assert_eq!(
            answer,
            Answer::Refusal {
                refusal: Refusal::UnsupportedColumn {
                    column: "blob".to_owned(),
                    type_name: "binary".to_owned(),
                }
            }
        );
    }

    /// The output cap stops the text: the child writes what fits and says
    /// the text is too much (`SEC-22` D6).
    #[test]
    fn the_output_cap_stops_the_stream() {
        let mut limits = limits();
        limits.max_output_bytes = 16;
        let mut out: Vec<u8> = Vec::new();
        let code = run(
            &Operation::ConvertSheet(Some("Stock".to_owned())),
            STOCK_XLSX,
            &limits,
            &mut out,
        );
        assert_eq!(code, exit::TOO_MUCH_TEXT);
        let (_, text) = split(&out);
        assert!(
            text.len() <= 16 + 64,
            "the text stops near the cap: {} bytes",
            text.len()
        );
    }

    /// The limits the child runs under are the ones the environment names.
    #[test]
    fn the_memory_limit_comes_from_the_environment() {
        let limits = ChildLimits::from_env();
        assert_eq!(limits.memory_bytes, DEFAULT_MEMORY_MIB * 1024 * 1024);
        assert_eq!(limits.cpu_seconds, DEFAULT_CPU_SECONDS);
        assert_eq!(
            ChildLimits::with_memory(Some("512")).memory_bytes,
            512 * 1024 * 1024
        );
        // A limit that is not a number is no limit: the default stands.
        for raw in ["not a number", "0", "", "  "] {
            assert_eq!(
                ChildLimits::with_memory(Some(raw)).memory_bytes,
                DEFAULT_MEMORY_MIB * 1024 * 1024,
                "{raw:?}"
            );
        }
    }
}
