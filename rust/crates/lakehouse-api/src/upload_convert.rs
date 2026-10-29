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
//! # A Parquet file goes by ranges, never whole (`SEC-22` P1)
//!
//! A Parquet file may be two gibibytes long (the chunked upload's cap), and
//! nothing but its footer and its first row group is read. So the two Parquet
//! operations take the file's SIZE on the command line and are fed only the
//! ranges they need:
//!
//! - `inspect-parquet <file_size>` is fed the file's TAIL — the last
//!   `min(file_size, MAX_FOOTER_BYTES + 8)` bytes, which hold the footer —
//!   and answers the footer's account of the file and the byte range of its
//!   first row group;
//! - `preview-parquet <max_rows> <file_size> <group_offset> <group_len>` is
//!   fed that tail and then the first row group's bytes.
//!
//! The child reads them through a sparse reader
//! ([`crate::upload_parquet_parts`]) that answers the ranges it was given and
//! refuses every other one. So the child's input no longer depends on the
//! file's size: a two-gibibyte Parquet file is read exactly as a small one
//! is.
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
//!
//! # The API side
//!
//! [`ConversionRunner`] is the other half of the protocol: it starts the
//! child, feeds it the file, reads the answer, and (for the operations that
//! convert) writes the text into the store. It holds the slots: at most
//! [`ConversionRunner::max_conversions`] conversions run at once per API
//! process, and a request that arrives when every slot is taken is refused
//! at once (429) rather than waiting.
//!
//! The child is started with an EMPTY environment and the variables it
//! needs ([`MEMORY_ENV`], [`OUTPUT_ENV`], [`CPU_ENV`], [`PARQUET_TAIL_ENV`]
//! and [`PARQUET_GROUP_ENV`]): a process that parses an untrusted file must
//! not inherit the API's secrets.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use axum::body::Bytes;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::upload_parquet::{self, GroupRange, ParquetFileError, ParquetInfo};
use crate::upload_parquet_parts::{Part, PartsReader};
use crate::upload_store::{UploadStore, UploadStoreError};
use crate::upload_workbook::{self, WorkbookError, WorkbookInfo};

/// The environment variable the child's memory limit comes from.
///
/// This is an INTERNAL variable, set by the API on the child it starts (see
/// [`ConversionRunner::start`]); it is not one an operator sets. The
/// operator-facing one is `UPLOAD_CONVERT_MEMORY_MB`
/// (`crate::config::Config::upload_convert_memory_mb`), which the API reads
/// and passes on as this one. The child reads this variable and nothing else:
/// it never resolves the whole configuration, so it needs no database, no
/// secret and no endpoint to read a file.
pub const MEMORY_ENV: &str = "LAKEHOUSE_CONVERT_MEMORY_MIB";

/// The environment variable the child's output cap comes from.
///
/// An INTERNAL variable, like [`MEMORY_ENV`]: the API sets it on the child it
/// starts. The cap is [`MAX_OUTPUT_BYTES`] unless this says otherwise, and
/// the API's own cap is the same number — the variable exists so that a test
/// can ask the child for a smaller cap than 512 MiB without a 512 MiB
/// fixture.
pub const OUTPUT_ENV: &str = "LAKEHOUSE_CONVERT_MAX_OUTPUT_BYTES";

/// The environment variable the child's CPU limit comes from.
///
/// An INTERNAL variable, like [`MEMORY_ENV`]. The limit is
/// [`DEFAULT_CPU_SECONDS`] unless this says otherwise; the variable exists
/// so that the measurement of `SEC-22` T5 can run without the limit ending
/// the run it is measuring.
pub const CPU_ENV: &str = "LAKEHOUSE_CONVERT_CPU_SECONDS";

/// The environment variable the child's Parquet tail bound comes from.
///
/// An INTERNAL variable, like [`MEMORY_ENV`]. It is how many bytes of a
/// Parquet file's end the child is given: the footer and the eight bytes
/// that follow it. The API sets it on the child it starts, so that both
/// sides agree on where the tail begins.
pub const PARQUET_TAIL_ENV: &str = "LAKEHOUSE_CONVERT_MAX_PARQUET_TAIL_BYTES";

/// The environment variable the child's Parquet row-group bound comes from.
///
/// An INTERNAL variable, like [`MEMORY_ENV`]. It is how many bytes of a
/// Parquet file's first row group the child is given.
pub const PARQUET_GROUP_ENV: &str = "LAKEHOUSE_CONVERT_MAX_PARQUET_GROUP_BYTES";

/// The most bytes of a Parquet file the child is given as its tail: the
/// footer, which is at most [`upload_parquet::MAX_FOOTER_BYTES`], and the
/// eight bytes that hold its length and its magic (`SEC-22` P1).
pub const PARQUET_TAIL_BYTES: u64 = upload_parquet::MAX_FOOTER_BYTES + 8;

/// The most bytes of a Parquet file's first row group the child is given: the
/// bound the reader already had on what a preview fetches
/// ([`upload_parquet::MAX_PREVIEW_GROUP_BYTES`]).
pub const PARQUET_GROUP_BYTES: u64 = upload_parquet::MAX_PREVIEW_GROUP_BYTES as u64;

/// The child's memory limit when nothing says otherwise: 896 MiB.
///
/// `SEC-22` D7: the limit is the measured peak times 1.5, rounded up to
/// 128 MiB. Measured on 2026-10-28 on the machine this was written on, with
/// the script of `ops/g9/sec22_measure.py` and the fixtures it builds (the
/// numbers are in `docs/plans/SEC-22-RESULT.md`, written by the planner
/// from the handoff of `SEC-22`):
///
/// - the baseline, the child with an empty input: 23.9 MiB;
/// - `convert-sheet` on a workbook of 5,000,000 cells: 549.2 MiB;
/// - `preview-parquet` on a Parquet file of 50 MiB: 136.2 MiB.
///
/// 549.2 MiB times 1.5 is 823.9 MiB, which rounds up to 896 MiB. Two
/// conversions at once are 1.75 GiB, which a machine with 12 GB of memory
/// affords.
pub const DEFAULT_MEMORY_MIB: u32 = 896;

/// The child's CPU limit, in seconds: 60 (`SEC-22` D7). The API's own
/// wall-clock limit is 90 s (`SEC-22` D3), so the CPU limit is the one that
/// ends a conversion that spins.
///
/// The measurement of `SEC-22` T5 took 82.9 s of wall-clock time for the
/// largest allowed workbook, in a BUILD WITHOUT OPTIMISATIONS: the
/// dependencies of the parser are unoptimised there, and the deployment's
/// binary is not. The release build was not measured, so the headroom of
/// this limit is not proven (`SEC-22` T5, handoff).
pub const DEFAULT_CPU_SECONDS: u64 = 60;

/// The most text a conversion may produce: 512 MiB (`SEC-22` D6, the number
/// `upload_parquet::MAX_OUTPUT_BYTES` already used).
pub const MAX_OUTPUT_BYTES: u64 = crate::upload_parquet::MAX_OUTPUT_BYTES as u64;

/// The most the child reads from its standard input: 64 MiB. The API's own
/// upload cap is 50 MiB (`routes::uploads::MAX_UPLOAD_BYTES`), so this is
/// the backstop for a caller that is not the API.
pub const MAX_INPUT_BYTES: u64 = 64 * 1024 * 1024;

/// The most the answer line may be, in bytes: 1 MiB.
///
/// The answer is one JSON line, and it says what the file holds: a workbook
/// with many sheets and long names makes it long. It must not be read
/// without a bound, because the API holds it, parses a second copy of it and
/// stores it — which is the memory failure `SEC-22` exists to stop
/// (`PR #187` review BLOCKER 4). An answer over the cap is refused as a
/// resource limit, and the child is killed with it.
pub const MAX_ANSWER_BYTES: usize = 1024 * 1024;

/// The most bytes the child is fed at a time: 8 MiB, the piece size the
/// chunked upload uses (`SEC-22` D1b). The API never holds the whole file.
pub const FEED_CHUNK_BYTES: u64 = 8 * 1024 * 1024;

/// The most of the child's standard error that is kept: 2 KiB. The rest is
/// read and dropped, so the child never blocks on a full pipe.
pub const MAX_STDERR_BYTES: usize = 2 * 1024;

/// The wall-clock limit of one conversion: 90 s (`SEC-22` D3). The console's
/// proxy gives up at 130 s, so the conversion answers first.
pub const CONVERSION_WALL: Duration = Duration::from_secs(90);

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
    /// The child could not set its own limits. That is an installation
    /// fault, not a bad file, and it is answered with a 500.
    pub const LIMIT_FAILED: i32 = 12;
}

/// What the child was asked to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Operation {
    /// The sheets of a workbook.
    ListSheets,
    /// One sheet of a workbook as text: the named sheet, or the default one
    /// when no name is given.
    ConvertSheet(Option<String>),
    /// What a Parquet file's footer says. The size is the size of the WHOLE
    /// file: the child is given the file's tail and reads the footer out of
    /// it (`SEC-22` P1).
    InspectParquet {
        /// The size of the file, in bytes.
        size: u64,
    },
    /// The first rows of a Parquet file, as text. The size and the row
    /// group's range are the file's own (`SEC-22` P1).
    PreviewParquet {
        /// How many rows to decode at most.
        rows: usize,
        /// The size of the whole file, in bytes.
        size: u64,
        /// Where the first row group starts in the file.
        group_offset: u64,
        /// How many bytes the first row group takes.
        group_len: u64,
    },
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
            "inspect-parquet" => {
                let size = rest.first()?.parse::<u64>().ok()?;
                Some(Self::InspectParquet { size })
            }
            "preview-parquet" => {
                let rows = rest.first()?.parse::<usize>().ok()?;
                let size = rest.get(1)?.parse::<u64>().ok()?;
                let group_offset = rest.get(2)?.parse::<u64>().ok()?;
                let group_len = rest.get(3)?.parse::<u64>().ok()?;
                Some(Self::PreviewParquet {
                    rows,
                    size,
                    group_offset,
                    group_len,
                })
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
            Self::InspectParquet { size } => vec!["inspect-parquet".to_owned(), size.to_string()],
            Self::PreviewParquet {
                rows,
                size,
                group_offset,
                group_len,
            } => vec![
                "preview-parquet".to_owned(),
                rows.to_string(),
                size.to_string(),
                group_offset.to_string(),
                group_len.to_string(),
            ],
        }
    }

    /// Whether this operation reads a Parquet file, and so is fed ranges of
    /// it rather than the whole file (`SEC-22` P1).
    #[must_use]
    pub fn is_parquet(&self) -> bool {
        matches!(
            self,
            Self::InspectParquet { .. } | Self::PreviewParquet { .. }
        )
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
        /// The first row group's byte range, when the answer is the footer's
        /// alone (`SEC-22` P1: the caller fetches that range and asks for the
        /// rows next).
        group: Option<GroupRange>,
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
    /// The most bytes of a Parquet file's end the child is given, in bytes.
    pub parquet_tail_bytes: u64,
    /// The most bytes of a Parquet file's first row group the child is given,
    /// in bytes.
    pub parquet_group_bytes: u64,
}

impl Default for ChildLimits {
    fn default() -> Self {
        Self {
            memory_bytes: u64::from(DEFAULT_MEMORY_MIB) * 1024 * 1024,
            cpu_seconds: DEFAULT_CPU_SECONDS,
            max_output_bytes: MAX_OUTPUT_BYTES,
            max_input_bytes: MAX_INPUT_BYTES,
            parquet_tail_bytes: PARQUET_TAIL_BYTES,
            parquet_group_bytes: PARQUET_GROUP_BYTES,
        }
    }
}

impl ChildLimits {
    /// The limits the child runs under, with the memory limit from the
    /// environment ([`MEMORY_ENV`]), the output cap from the environment
    /// ([`OUTPUT_ENV`]), the CPU limit from the environment ([`CPU_ENV`]) and
    /// the two Parquet bounds from the environment ([`PARQUET_TAIL_ENV`] and
    /// [`PARQUET_GROUP_ENV`]) when they say something usable.
    ///
    /// The child reads these five variables and nothing else: it never
    /// resolves the API's configuration, so it needs no environment of its
    /// own to read a file (`SEC-22` D1).
    #[must_use]
    pub fn from_env() -> Self {
        let memory = std::env::var(MEMORY_ENV).ok();
        let output = std::env::var(OUTPUT_ENV).ok();
        let cpu = std::env::var(CPU_ENV).ok();
        let tail = std::env::var(PARQUET_TAIL_ENV).ok();
        let group = std::env::var(PARQUET_GROUP_ENV).ok();
        Self::with_limits(memory.as_deref(), output.as_deref(), cpu.as_deref())
            .with_parquet(tail.as_deref(), group.as_deref())
    }

    /// The limits the child runs under, with the memory limit `raw` names,
    /// in mebibytes. Anything that is not a whole number of 1 or more keeps
    /// the default: a limit that cannot be read is no limit at all.
    #[allow(
        dead_code,
        reason = "the unit tests of this module read a limit from a string; the binary does not"
    )]
    #[must_use]
    pub fn with_memory(raw: Option<&str>) -> Self {
        Self::with_limits(raw, None, None)
    }

    /// The limits the child runs under, with the memory limit `memory` names
    /// in mebibytes, the output cap `output` names in bytes and the CPU
    /// limit `cpu` names in seconds. Anything that is not a whole number of
    /// 1 or more keeps the default: a limit that cannot be read is no limit
    /// at all.
    #[must_use]
    pub fn with_limits(memory: Option<&str>, output: Option<&str>, cpu: Option<&str>) -> Self {
        let mut limits = Self::default();
        if let Some(mib) = memory
            .map(str::trim)
            .and_then(|raw| raw.parse::<u64>().ok())
            .filter(|mib| *mib > 0)
        {
            limits.memory_bytes = mib.saturating_mul(1024 * 1024);
        }
        if let Some(bytes) = output
            .map(str::trim)
            .and_then(|raw| raw.parse::<u64>().ok())
            .filter(|bytes| *bytes > 0)
        {
            limits.max_output_bytes = bytes;
        }
        if let Some(seconds) = cpu
            .map(str::trim)
            .and_then(|raw| raw.parse::<u64>().ok())
            .filter(|seconds| *seconds > 0)
        {
            limits.cpu_seconds = seconds;
        }
        limits
    }

    /// The same limits, with the two Parquet bounds: how many bytes of the
    /// file's end the child is given, and how many bytes of its first row
    /// group. Anything that is not a whole number of 1 or more keeps the
    /// default.
    #[must_use]
    pub fn with_parquet(self, tail: Option<&str>, group: Option<&str>) -> Self {
        let mut limits = self;
        if let Some(bytes) = tail
            .map(str::trim)
            .and_then(|raw| raw.parse::<u64>().ok())
            .filter(|bytes| *bytes > 0)
        {
            limits.parquet_tail_bytes = bytes;
        }
        if let Some(bytes) = group
            .map(str::trim)
            .and_then(|raw| raw.parse::<u64>().ok())
            .filter(|bytes| *bytes > 0)
        {
            limits.parquet_group_bytes = bytes;
        }
        limits
    }

    /// The most the child reads from its standard input, for this operation.
    ///
    /// A workbook is fed whole, so the cap is [`MAX_INPUT_BYTES`]. A Parquet
    /// file is fed its tail and its first row group, so the cap is the two
    /// bounds together (`SEC-22` P1) and does not depend on the file's size.
    #[must_use]
    pub fn input_cap(&self, operation: &Operation) -> u64 {
        if operation.is_parquet() {
            self.parquet_tail_bytes
                .saturating_add(self.parquet_group_bytes)
        } else {
            self.max_input_bytes
        }
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
        // Its own limits are what keep a crafted file from ending the API,
        // so a child that could not set them is an installation fault and
        // says so with a code of its own (`PR #187` review SHOULD-FIX 4).
        eprintln!("[inspect-upload] the limits could not be set: {err}");
        return Some(exit::LIMIT_FAILED);
    }
    let operation = Operation::parse(args.get(2..).unwrap_or_default());
    // The operation decides how much the child reads: a Parquet file is fed
    // ranges of it, a workbook the whole of it (`SEC-22` P1).
    let cap = match &operation {
        Some(operation) => limits.input_cap(operation),
        None => limits.max_input_bytes,
    };
    let input = match read_input(cap) {
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
        Operation::InspectParquet { size } => {
            let reader = match parquet_reader(*size, None, input, limits) {
                Ok(reader) => reader,
                Err(refusal) => return refuse(out, FileKind::Parquet, &refusal),
            };
            match upload_parquet::inspect_parts(&reader) {
                Ok(footer) => {
                    let answer = Answer::Parquet {
                        info: footer.info,
                        group: footer.group,
                    };
                    match write_answer(out, &answer) {
                        Ok(()) => exit::OK,
                        Err(_) => exit::UNREADABLE,
                    }
                }
                Err(err) => refuse(out, FileKind::Parquet, &parquet_refusal(err)),
            }
        }
        Operation::PreviewParquet {
            rows,
            size,
            group_offset,
            group_len,
        } => {
            let reader =
                match parquet_reader(*size, Some((*group_offset, *group_len)), input, limits) {
                    Ok(reader) => reader,
                    Err(refusal) => return refuse(out, FileKind::Parquet, &refusal),
                };
            let converted = match upload_parquet::convert_parts(
                &reader,
                Some(*rows),
                limits.parquet_group_bytes,
            ) {
                Ok(converted) => converted,
                Err(err) => return refuse(out, FileKind::Parquet, &parquet_refusal(err)),
            };
            let answer = Answer::Parquet {
                info: converted.info.clone(),
                group: None,
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

/// The reader over the pieces of a Parquet file the child was given.
///
/// The file's TAIL comes first and is as long as the tail bound says, or as
/// long as the file itself when the file is shorter. The first row group's
/// bytes follow it, when the group is within the bound: a group over it is
/// shown as its columns and no rows, and its bytes are not moved at all.
///
/// The group's bytes are the ones the caller sent after the tail. They are
/// the same bytes the tail holds when the file is small enough that its first
/// row group is inside the tail, so the reader answers either way.
fn parquet_reader(
    size: u64,
    group: Option<(u64, u64)>,
    input: &[u8],
    limits: &ChildLimits,
) -> Result<PartsReader, Refusal> {
    let tail_len = size.min(limits.parquet_tail_bytes);
    let tail_len = usize::try_from(tail_len).unwrap_or(usize::MAX);
    if input.len() < tail_len {
        return Err(Refusal::Unreadable);
    }
    let mut parts: Vec<(u64, Bytes)> = Vec::new();
    if let Some((offset, len)) = group {
        let end = offset.checked_add(len).ok_or(Refusal::Unreadable)?;
        if end > size {
            return Err(Refusal::Unreadable);
        }
        if len > 0 && len <= limits.parquet_group_bytes {
            let from = tail_len;
            let to = tail_len.saturating_add(usize::try_from(len).unwrap_or(usize::MAX));
            if input.len() < to {
                return Err(Refusal::Unreadable);
            }
            parts.push((offset, Bytes::copy_from_slice(&input[from..to])));
        }
    }
    parts.push((
        size - tail_len as u64,
        Bytes::copy_from_slice(&input[..tail_len]),
    ));
    Ok(PartsReader::new(size, parts))
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

// ── the API side ──────────────────────────────────────────────────────────

/// Where the file the child reads comes from.
pub enum Source {
    /// The bytes, already in memory: an upload that was just received.
    Bytes(Bytes),
    /// A stored object, read in pieces of [`FEED_CHUNK_BYTES`]. The API
    /// never holds the whole file (`SEC-22` D1b).
    Stored {
        /// The store the object is in.
        store: UploadStore,
        /// The key the object is under.
        key: String,
    },
    /// Parts of a file that is already in memory, in order (`SEC-22` P1: a
    /// Parquet file is read by ranges, never whole).
    Parts {
        /// The whole file, in memory.
        bytes: Bytes,
        /// The parts to feed, in order.
        parts: Vec<Part>,
    },
    /// Parts of a stored object, in order. The API never holds more than one
    /// piece of it at a time.
    StoredParts {
        /// The store the object is in.
        store: UploadStore,
        /// The key the object is under.
        key: String,
        /// The parts to feed, in order.
        parts: Vec<Part>,
    },
}

/// Why a conversion did not answer.
///
/// Each is one answer, never a mixture: a busy installation says so, a
/// child that could not be started says so, and a file the child refused
/// says the file's own reason.
#[derive(Debug)]
pub enum ConversionError {
    /// Every slot is taken: 429, with `Retry-After: 5`.
    Busy,
    /// The child could not be started: 500.
    NotStarted,
    /// The child could not set its own limits: 500. That is an installation
    /// fault, not a bad file (`PR #187` review SHOULD-FIX 4).
    LimitFailed,
    /// The child was killed, ran out of time, or answered something the
    /// protocol does not allow: 400, with the one sentence for all of them.
    ResourceLimit,
    /// The file was refused: 400, with the refusal's own sentence.
    Refused(Refusal),
    /// The store failed while the text was written: 503.
    Store(UploadStoreError),
}

/// The API's side of the child's protocol: it starts the child, feeds it the
/// file, reads the answer, and writes the text of a conversion into the
/// store.
///
/// Cheap to clone (the slots are shared), so it can live in [`crate::state::AppState`]
/// like every other handle.
#[derive(Clone)]
pub struct ConversionRunner {
    /// The binary the child is: this API's own binary.
    binary: PathBuf,
    /// Whether the binary is checked to be this API's own binary before it is
    /// started. The API's own binary always is; a binary a caller sets with
    /// [`Self::with_binary`] is run as it is, and that caller vouches for it.
    check_binary: bool,
    /// The conversion slots, shared by every clone.
    slots: Arc<Semaphore>,
    /// The memory limit the child sets for itself, in mebibytes.
    memory_mib: u64,
    /// The most text the child may produce, in bytes.
    max_output_bytes: u64,
    /// The CPU limit the child sets for itself, in seconds.
    cpu_seconds: u64,
    /// The most bytes of a Parquet file's end the child is given.
    parquet_tail_bytes: u64,
    /// The most bytes of a Parquet file's first row group the child is given.
    parquet_group_bytes: u64,
    /// The wall-clock limit of one conversion.
    wall: Duration,
    /// How many children this process has started.
    runs: Arc<AtomicU64>,
}

impl std::fmt::Debug for ConversionRunner {
    /// Prints the type name and the limits, never the binary's path (which
    /// says where the API is installed).
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConversionRunner")
            .field("memory_mib", &self.memory_mib)
            .field("max_output_bytes", &self.max_output_bytes)
            .field("cpu_seconds", &self.cpu_seconds)
            .field("wall", &self.wall)
            .finish_non_exhaustive()
    }
}

impl ConversionRunner {
    /// A runner that starts `binary` as the child, with `max_conversions`
    /// conversions at once and the child's memory limit at `memory_mib`
    /// mebibytes.
    ///
    /// The binary is checked before it is started: it must be this API's own
    /// binary, whose name is `lakehouse-api`. A test binary would run the
    /// test suite again, so it is refused (500) rather than started.
    #[must_use]
    pub fn new(binary: PathBuf, max_conversions: usize, memory_mib: u64) -> Self {
        Self {
            binary,
            check_binary: true,
            slots: Arc::new(Semaphore::new(max_conversions.max(1))),
            memory_mib,
            max_output_bytes: MAX_OUTPUT_BYTES,
            cpu_seconds: DEFAULT_CPU_SECONDS,
            parquet_tail_bytes: PARQUET_TAIL_BYTES,
            parquet_group_bytes: PARQUET_GROUP_BYTES,
            wall: CONVERSION_WALL,
            runs: Arc::new(AtomicU64::new(0)),
        }
    }

    /// The same runner, with another binary as the child. For the tests: the
    /// route tests point the runner at the child Cargo built for them, and at
    /// a stand-in script that answers slowly.
    ///
    /// The binary is run as it is: the caller vouches for it, and the check
    /// that the child is this API's own binary does not apply.
    #[allow(
        dead_code,
        reason = "the route tests point the runner at the child they built; the binary does not"
    )]
    #[must_use]
    pub fn with_binary(&self, binary: PathBuf) -> Self {
        Self {
            binary,
            check_binary: false,
            slots: Arc::clone(&self.slots),
            memory_mib: self.memory_mib,
            max_output_bytes: self.max_output_bytes,
            cpu_seconds: self.cpu_seconds,
            parquet_tail_bytes: self.parquet_tail_bytes,
            parquet_group_bytes: self.parquet_group_bytes,
            wall: self.wall,
            runs: Arc::clone(&self.runs),
        }
    }

    /// The same runner, with another output cap for the child. For the tests:
    /// the cap is 512 MiB, and a test that proves the cap is reached does not
    /// write 512 MiB.
    #[allow(
        dead_code,
        reason = "the route tests set a smaller cap for the child; the binary does not"
    )]
    #[must_use]
    pub fn with_max_output_bytes(&self, max_output_bytes: u64) -> Self {
        Self {
            binary: self.binary.clone(),
            check_binary: self.check_binary,
            slots: Arc::clone(&self.slots),
            memory_mib: self.memory_mib,
            max_output_bytes,
            cpu_seconds: self.cpu_seconds,
            parquet_tail_bytes: self.parquet_tail_bytes,
            parquet_group_bytes: self.parquet_group_bytes,
            wall: self.wall,
            runs: Arc::clone(&self.runs),
        }
    }

    /// The same runner, with other bounds for what a Parquet file costs the
    /// child: how many bytes of the file's end it is given, and how many
    /// bytes of its first row group.
    ///
    /// For the tests: the real bounds are 16 MiB and 64 MiB, and a test that
    /// proves a Parquet file is read by ranges must not write 80 MiB.
    #[allow(
        dead_code,
        reason = "the route tests set smaller bounds for the child; the binary does not"
    )]
    #[must_use]
    pub fn with_parquet_limits(&self, tail_bytes: u64, group_bytes: u64) -> Self {
        Self {
            binary: self.binary.clone(),
            check_binary: self.check_binary,
            slots: Arc::clone(&self.slots),
            memory_mib: self.memory_mib,
            max_output_bytes: self.max_output_bytes,
            cpu_seconds: self.cpu_seconds,
            parquet_tail_bytes: tail_bytes,
            parquet_group_bytes: group_bytes,
            wall: self.wall,
            runs: Arc::clone(&self.runs),
        }
    }

    /// The same runner, with another wall-clock limit. For the tests.
    #[allow(
        dead_code,
        reason = "the route tests shorten the wall clock; the binary does not"
    )]
    #[must_use]
    pub fn with_wall(&self, wall: Duration) -> Self {
        Self {
            binary: self.binary.clone(),
            check_binary: self.check_binary,
            slots: Arc::clone(&self.slots),
            memory_mib: self.memory_mib,
            max_output_bytes: self.max_output_bytes,
            cpu_seconds: self.cpu_seconds,
            parquet_tail_bytes: self.parquet_tail_bytes,
            parquet_group_bytes: self.parquet_group_bytes,
            wall,
            runs: Arc::clone(&self.runs),
        }
    }

    /// How many children this process has started. The route tests read it
    /// to prove that a second preview runs no child.
    #[allow(
        dead_code,
        reason = "the route tests count the child's runs; the binary does not"
    )]
    #[must_use]
    pub fn runs(&self) -> u64 {
        self.runs.load(Ordering::Relaxed)
    }

    /// How many bytes of a Parquet file's end the child is given (`SEC-22`
    /// P1). The API fetches exactly that many bytes and hands them over.
    #[must_use]
    pub fn parquet_tail_bytes(&self) -> u64 {
        self.parquet_tail_bytes
    }

    /// How many bytes of a Parquet file's first row group the child is given
    /// (`SEC-22` P1). A group over this is shown as its columns and no rows.
    #[must_use]
    pub fn parquet_group_bytes(&self) -> u64 {
        self.parquet_group_bytes
    }

    /// Take a place, or `None` when every slot is taken.
    fn try_slot(&self) -> Option<OwnedSemaphorePermit> {
        Arc::clone(&self.slots).try_acquire_owned().ok()
    }

    /// Run the operation on the file and answer with what the child said.
    ///
    /// # Errors
    ///
    /// [`ConversionError`]: the installation is busy, the child could not be
    /// started, the child was killed or ran out of time, or the file was
    /// refused.
    pub async fn answer(
        &self,
        operation: Operation,
        source: Source,
    ) -> Result<Answer, ConversionError> {
        let Some(_slot) = self.try_slot() else {
            return Err(ConversionError::Busy);
        };
        // ONE deadline for the whole conversion: the answer, the text and
        // the child's ending. Every step is bounded by it, so the child is
        // ours when the time is up: it is killed and its ending is logged
        // (`PR #187` review SHOULD-FIX 4).
        let deadline = tokio::time::Instant::now() + self.wall;
        // Boxed: the future of the conversion holds the child's handles and
        // its buffers, and the caller's future would otherwise be that big.
        Box::pin(self.answer_inner(operation, source, deadline)).await
    }

    async fn answer_inner(
        &self,
        operation: Operation,
        source: Source,
        deadline: tokio::time::Instant,
    ) -> Result<Answer, ConversionError> {
        let started = self.start(&operation, source, deadline).await?;
        let Started {
            answer,
            child,
            mut stdout,
            stderr,
            feed,
        } = started;
        if let Answer::Refusal { refusal } = &answer {
            // The child refused the file: it is finished, and what it said
            // on its standard error is logged (`PR #187` review SHOULD-FIX
            // 4).
            let _ = self.wait(child, stderr, feed, deadline).await;
            return Err(ConversionError::Refused(refusal.clone()));
        }
        // The rest of the output is text this operation does not produce; it
        // is read and dropped, so the child is never blocked on a full pipe.
        let mut piece = [0_u8; 8192];
        let read_to_the_end = async {
            while let Ok(read) = stdout.read(&mut piece).await {
                if read == 0 {
                    break;
                }
            }
        };
        if tokio::time::timeout_at(deadline, read_to_the_end)
            .await
            .is_err()
        {
            // Past the wall clock: the child is killed, and its ending is
            // logged.
            let _ = self.wait(child, stderr, feed, deadline).await;
            return Err(ConversionError::ResourceLimit);
        }
        self.wait(child, stderr, feed, deadline).await?;
        Ok(answer)
    }

    /// Run the operation on the file and answer with what the child said and
    /// the text it produced. The text is read into memory, so this is for
    /// the small ones: a Parquet preview.
    ///
    /// # Errors
    ///
    /// As [`Self::answer`].
    pub async fn answer_with_text(
        &self,
        operation: Operation,
        source: Source,
    ) -> Result<(Answer, Bytes), ConversionError> {
        let Some(_slot) = self.try_slot() else {
            return Err(ConversionError::Busy);
        };
        let deadline = tokio::time::Instant::now() + self.wall;
        Box::pin(self.text_inner(operation, source, deadline)).await
    }

    async fn text_inner(
        &self,
        operation: Operation,
        source: Source,
        deadline: tokio::time::Instant,
    ) -> Result<(Answer, Bytes), ConversionError> {
        let started = self.start(&operation, source, deadline).await?;
        let Started {
            answer,
            child,
            mut stdout,
            stderr,
            feed,
        } = started;
        if let Answer::Refusal { refusal } = &answer {
            let _ = self.wait(child, stderr, feed, deadline).await;
            return Err(ConversionError::Refused(refusal.clone()));
        }
        let mut text = Vec::new();
        let mut piece = [0_u8; 16 * 1024];
        let read_to_the_end = async {
            loop {
                let read = stdout
                    .read(&mut piece)
                    .await
                    .map_err(|_err| ConversionError::Refused(Refusal::ResourceLimit))?;
                if read == 0 {
                    break;
                }
                text.extend_from_slice(&piece[..read]);
                if u64::try_from(text.len()).unwrap_or(u64::MAX) > MAX_OUTPUT_BYTES {
                    return Err(ConversionError::Refused(Refusal::TooMuchText));
                }
            }
            Ok::<(), ConversionError>(())
        };
        match tokio::time::timeout_at(deadline, read_to_the_end).await {
            Ok(Ok(())) => {}
            Ok(Err(err)) => {
                let _ = self.wait(child, stderr, feed, deadline).await;
                return Err(err);
            }
            Err(_elapsed) => {
                let _ = self.wait(child, stderr, feed, deadline).await;
                return Err(ConversionError::ResourceLimit);
            }
        }
        self.wait(child, stderr, feed, deadline).await?;
        Ok((answer, Bytes::from(text)))
    }

    /// Run the operation on the file and store the text it produced at
    /// `text_key`, and the answer at `answer_key`.
    ///
    /// The text is written as one multipart upload that is NOT assembled
    /// until the child has exited 0, and the answer is written after it, as
    /// the MARKER of a finished conversion: a text without its answer is a
    /// conversion that did not finish, and is converted again
    /// (`SEC-22` P2).
    ///
    /// # Errors
    ///
    /// As [`Self::answer`], and [`ConversionError::Store`] when the store
    /// refuses the write.
    pub async fn convert_and_store(
        &self,
        operation: Operation,
        source: Source,
        store: &UploadStore,
        text_key: &str,
        answer_key: &str,
    ) -> Result<Answer, ConversionError> {
        let Some(_slot) = self.try_slot() else {
            return Err(ConversionError::Busy);
        };
        let deadline = tokio::time::Instant::now() + self.wall;
        Box::pin(self.store_inner(operation, source, store, text_key, answer_key, deadline)).await
    }

    async fn store_inner(
        &self,
        operation: Operation,
        source: Source,
        store: &UploadStore,
        text_key: &str,
        answer_key: &str,
        deadline: tokio::time::Instant,
    ) -> Result<Answer, ConversionError> {
        let started = self.start(&operation, source, deadline).await?;
        let Started {
            answer,
            child,
            mut stdout,
            stderr,
            feed,
        } = started;
        if let Answer::Refusal { refusal } = &answer {
            return Err(ConversionError::Refused(refusal.clone()));
        }
        // The text is written as a multipart upload that is NOT assembled
        // yet (`SEC-22` P2): the object at the key does not exist until the
        // child has exited 0, so a truncated text is never visible.
        let written = write_text_to_store(store, text_key, &mut stdout);
        let pending = match tokio::time::timeout_at(deadline, written).await {
            Ok(Ok(pending)) => pending,
            Ok(Err(err)) => {
                // The stream stopped early: read what is left of the child's
                // output, so the child is never left writing into a pipe
                // nobody reads.
                let mut sink = [0_u8; 8192];
                while let Ok(read) = stdout.read(&mut sink).await {
                    if read == 0 {
                        break;
                    }
                }
                let _ = self.wait(child, stderr, feed, deadline).await;
                return Err(ConversionError::Store(err));
            }
            Err(_elapsed) => {
                // Past the wall clock: the child is killed, and its ending
                // is logged.
                let _ = self.wait(child, stderr, feed, deadline).await;
                return Err(ConversionError::ResourceLimit);
            }
        };
        // The child's own ending decides the answer: it may have stopped
        // (refused, killed, out of time) even though the store took what
        // came of it.
        match self.wait(child, stderr, feed, deadline).await {
            Ok(()) => {
                // The child exited 0: the text is the whole text, and it is
                // assembled now.
                if let Err(err) = store.complete_pending(&pending).await {
                    let _aborted = store.abort_pending(&pending).await;
                    return Err(ConversionError::Store(err));
                }
                // The answer is written LAST, and it is the MARKER: a text
                // without it is a conversion that did not finish, and is
                // converted again (`SEC-22` P2).
                let line = serde_json::to_vec(&answer)
                    .map_err(|_err| ConversionError::Refused(Refusal::Unreadable))?;
                if let Err(err) = store.put(answer_key, Bytes::from(line)).await {
                    // The text is there but the marker is not, so the text is
                    // not a conversion: it goes. Only what this run wrote is
                    // removed (`SEC-22` P2).
                    if let Err(cleanup) = store.delete(text_key).await {
                        tracing::warn!(%cleanup, key = %text_key, "the text of a conversion that could not be marked could not be removed");
                    }
                    return Err(ConversionError::Store(err));
                }
                Ok(answer)
            }
            Err(err) => {
                // The child did not finish: nothing was assembled, so there
                // is no object to take back out, and the objects of an
                // earlier run are left alone (`SEC-22` P2).
                let _aborted = store.abort_pending(&pending).await;
                Err(err)
            }
        }
    }

    /// Start the child, feed it the file, and read its answer.
    async fn start(
        &self,
        operation: &Operation,
        source: Source,
        deadline: tokio::time::Instant,
    ) -> Result<Started, ConversionError> {
        if self.check_binary && !is_our_binary(&self.binary) {
            tracing::warn!(
                binary = %self.binary.display(),
                "the child binary is not this API's own binary; no file is read"
            );
            return Err(ConversionError::NotStarted);
        }
        self.runs.fetch_add(1, Ordering::Relaxed);
        let mut command = Command::new(&self.binary);
        command
            .arg("inspect-upload")
            .args(operation.arguments())
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            // The child parses a file somebody chose, so it must not inherit
            // the API's environment: no secret, no endpoint, no password.
            // It gets the variables it needs and nothing else.
            .env_clear()
            .env(MEMORY_ENV, self.memory_mib.to_string())
            .env(OUTPUT_ENV, self.max_output_bytes.to_string())
            .env(CPU_ENV, self.cpu_seconds.to_string())
            .env(PARQUET_TAIL_ENV, self.parquet_tail_bytes.to_string())
            .env(PARQUET_GROUP_ENV, self.parquet_group_bytes.to_string());
        let mut child = command.spawn().map_err(|err| {
            tracing::warn!(%err, "the child that reads an uploaded file could not be started");
            ConversionError::NotStarted
        })?;
        let Some(stdin) = child.stdin.take() else {
            return Err(ConversionError::NotStarted);
        };
        let Some(stdout) = child.stdout.take() else {
            return Err(ConversionError::NotStarted);
        };
        let Some(mut stderr) = child.stderr.take() else {
            return Err(ConversionError::NotStarted);
        };
        // The file is fed in another task: the child reads all of it before
        // it answers, and the answer would never come if the feeding
        // waited for it. The task is kept, so that a store failure in it can
        // be told from the file's own refusal (`PR #187` review SHOULD-FIX
        // 3).
        let feed = tokio::spawn(async move { feed(stdin, source).await });
        // The standard error is drained in another task, so the child never
        // blocks on a full pipe; only the first [`MAX_STDERR_BYTES`] are kept.
        let stderr = tokio::spawn(async move {
            let mut kept: Vec<u8> = Vec::new();
            let mut piece = [0_u8; 4096];
            while let Ok(read) = stderr.read(&mut piece).await {
                if read == 0 {
                    break;
                }
                if kept.len() < MAX_STDERR_BYTES {
                    let room = MAX_STDERR_BYTES - kept.len();
                    kept.extend_from_slice(&piece[..read.min(room)]);
                }
            }
            kept
        });
        let mut stdout = BufReader::new(stdout);
        let mut line = Vec::new();
        // The answer is read through a cap: it is one JSON line, and a child
        // that writes an endless one must not fill the API's memory
        // (`PR #187` review BLOCKER 4).
        let read_answer = async {
            let mut limited =
                (&mut stdout).take(u64::try_from(MAX_ANSWER_BYTES).unwrap_or(u64::MAX) + 1);
            limited.read_until(b'\n', &mut line).await
        };
        let read = match tokio::time::timeout_at(deadline, read_answer).await {
            Ok(result) => {
                result.map_err(|_err| ConversionError::Refused(Refusal::ResourceLimit))?
            }
            Err(_elapsed) => {
                // Past the wall clock: the child is killed, and its ending
                // is logged (`PR #187` review SHOULD-FIX 4).
                return Err(self.refuse_the_child(child, stderr, feed, deadline).await);
            }
        };
        if read == 0 {
            // The child said nothing: it died before its answer. Its ending
            // is logged, and that is where the out-of-memory abort shows up.
            return Err(self.refuse_the_child(child, stderr, feed, deadline).await);
        }
        if !answer_line_fits(&line) {
            // The answer is over the cap, or the child stopped before the
            // line was finished. The child is killed with this request
            // (`kill_on_drop`), its ending is logged, and the answer is a
            // resource limit.
            tracing::warn!(
                bytes = line.len(),
                "the answer of the child that reads an uploaded file is over its cap"
            );
            return Err(self.refuse_the_child(child, stderr, feed, deadline).await);
        }
        let answer: Answer = match serde_json::from_slice(&line) {
            Ok(answer) => answer,
            Err(err) => {
                tracing::warn!(%err, "the answer of the child that reads an uploaded file is not JSON");
                // The child's own ending decides (`PR #187` review
                // SHOULD-FIX 5): a signal means it was killed — out of
                // memory, out of time — and that is a resource limit; any
                // other ending is the file's own unreadability.
                let ending = self.wait(child, stderr, feed, deadline).await;
                return Err(match ending {
                    Err(ConversionError::ResourceLimit) => ConversionError::ResourceLimit,
                    Err(ConversionError::Store(err)) => ConversionError::Store(err),
                    _ => ConversionError::Refused(Refusal::Unreadable),
                });
            }
        };
        Ok(Started {
            answer,
            child,
            stdout,
            stderr,
            feed,
        })
    }

    /// The child is not to be trusted with the answer: it is killed, its
    /// ending is logged, and the answer is a resource limit.
    async fn refuse_the_child(
        &self,
        mut child: Child,
        stderr: tokio::task::JoinHandle<Vec<u8>>,
        feed: tokio::task::JoinHandle<Result<(), ConversionError>>,
        deadline: tokio::time::Instant,
    ) -> ConversionError {
        let _killed = child.start_kill();
        let _ = self.wait(child, stderr, feed, deadline).await;
        ConversionError::ResourceLimit
    }

    /// Wait for the child to end, and read what it said on its standard
    /// error.
    ///
    /// The task that feeds the child is joined here: a store failure in it
    /// is the REAL reason the child did not finish, and it must not be
    /// reported as the file's own (`PR #187` review SHOULD-FIX 3).
    ///
    /// Every failure path logs the child's status and the first
    /// [`MAX_STDERR_BYTES`] of its standard error (`PR #187` review
    /// SHOULD-FIX 4): a child that was killed by a signal says why there, and
    /// the log is the only place that says it.
    async fn wait(
        &self,
        mut child: Child,
        stderr: tokio::task::JoinHandle<Vec<u8>>,
        feed: tokio::task::JoinHandle<Result<(), ConversionError>>,
        deadline: tokio::time::Instant,
    ) -> Result<(), ConversionError> {
        let feeding = match tokio::time::timeout_at(deadline, feed).await {
            Ok(Ok(result)) => result,
            Ok(Err(_join)) => Err(ConversionError::Refused(Refusal::ResourceLimit)),
            Err(_elapsed) => Err(ConversionError::Refused(Refusal::ResourceLimit)),
        };
        let status = match tokio::time::timeout_at(deadline, child.wait()).await {
            Ok(Ok(status)) => status,
            Ok(Err(_err)) => {
                tracing::warn!("the child that reads an uploaded file could not be waited for");
                return Err(ConversionError::ResourceLimit);
            }
            Err(_elapsed) => {
                // Past the wall clock: the child is killed, and what it said
                // is logged.
                let _killed = child.start_kill();
                let status = child.wait().await;
                let kept = match tokio::time::timeout_at(deadline, stderr).await {
                    Ok(Ok(kept)) => kept,
                    Ok(Err(_join)) => Vec::new(),
                    Err(_elapsed) => Vec::new(),
                };
                let text = String::from_utf8_lossy(&kept);
                tracing::warn!(
                    status = ?status,
                    stderr = %text.trim(),
                    "the child that reads an uploaded file did not finish in time"
                );
                return Err(ConversionError::ResourceLimit);
            }
        };
        // The standard error is read through the deadline as well: a child
        // that leaves a grandchild holding the pipe open must not hold the
        // request (`PR #187` review SHOULD-FIX 4).
        let kept = match tokio::time::timeout_at(deadline, stderr).await {
            Ok(Ok(kept)) => kept,
            Ok(Err(_join)) => Vec::new(),
            Err(_elapsed) => Vec::new(),
        };
        let text = String::from_utf8_lossy(&kept);
        let text = text.trim().to_owned();
        // A store failure while the file was fed is the real reason, and it
        // is a 503: the file may well read once the store is back.
        if let Err(ConversionError::Store(err)) = &feeding {
            let err = *err;
            tracing::warn!(%err, status = ?status, stderr = %text, "the store could not be read while the child was fed");
            return Err(ConversionError::Store(err));
        }
        match status.code() {
            Some(exit::OK) => {
                if !text.is_empty() {
                    tracing::warn!(stderr = %text, "the child that reads an uploaded file said something");
                }
                Ok(())
            }
            Some(exit::LIMIT_FAILED) => {
                // The child could not set its own limits: an installation
                // fault, not a bad file (`PR #187` review SHOULD-FIX 4).
                tracing::warn!(stderr = %text, "the child could not set its own limits");
                Err(ConversionError::LimitFailed)
            }
            Some(code) => {
                tracing::warn!(code, stderr = %text, "the child that reads an uploaded file did not finish");
                Err(ConversionError::Refused(
                    Refusal::for_exit_code(code).unwrap_or(Refusal::Unreadable),
                ))
            }
            // Killed by a signal: out of memory, out of time, or killed.
            None => {
                tracing::warn!(status = ?status, stderr = %text, "the child that reads an uploaded file was killed");
                Err(ConversionError::ResourceLimit)
            }
        }
    }
}

/// A started child: its answer, its output, and the process itself.
struct Started {
    /// What the child said.
    answer: Answer,
    /// The child itself.
    child: Child,
    /// The rest of its output: the text, for the operations that convert.
    stdout: BufReader<ChildStdout>,
    /// Its standard error, read in another task.
    stderr: tokio::task::JoinHandle<Vec<u8>>,
    /// The task that feeds it the file: a store failure in it is the real
    /// reason the child did not finish (`PR #187` review SHOULD-FIX 3).
    feed: tokio::task::JoinHandle<Result<(), ConversionError>>,
}

/// Write the text the child produced into the store, as one multipart
/// upload that is NOT assembled: the caller assembles it once the child's
/// ending is known (`SEC-22` P2).
async fn write_text_to_store(
    store: &UploadStore,
    key: &str,
    stdout: &mut BufReader<ChildStdout>,
) -> Result<crate::upload_store::PendingText, UploadStoreError> {
    // Boxed: the multipart write holds a good deal of state, and the future
    // of the caller would otherwise be that big.
    Box::pin(store.put_stream(key, stdout)).await
}

/// Feed the child its file.
async fn feed(mut stdin: ChildStdin, source: Source) -> Result<(), ConversionError> {
    let result = match source {
        Source::Bytes(bytes) => stdin.write_all(&bytes).await,
        Source::Stored { store, key } => {
            let size = store
                .object_size(&key)
                .await
                .map_err(ConversionError::Store)?;
            let Some(size) = size else {
                return Err(ConversionError::Store(UploadStoreError::Unavailable(
                    "not found",
                )));
            };
            let mut offset: u64 = 0;
            let mut result = Ok(());
            while offset < size {
                let len = FEED_CHUNK_BYTES.min(size - offset);
                let chunk = store
                    .read_range(&key, offset, len)
                    .await
                    .map_err(ConversionError::Store)?;
                if chunk.is_empty() {
                    break;
                }
                if let Err(err) = stdin.write_all(&chunk).await {
                    result = Err(err);
                    break;
                }
                offset = offset.saturating_add(chunk.len() as u64);
            }
            result
        }
        Source::Parts { bytes, parts } => {
            let mut result = Ok(());
            for part in parts {
                let from = usize::try_from(part.offset).unwrap_or(usize::MAX);
                let to = from.saturating_add(usize::try_from(part.length).unwrap_or(usize::MAX));
                let Some(slice) = bytes.get(from..to) else {
                    result = Err(std::io::Error::other("a part outside the file"));
                    break;
                };
                if let Err(err) = stdin.write_all(slice).await {
                    result = Err(err);
                    break;
                }
            }
            result
        }
        Source::StoredParts { store, key, parts } => {
            let mut result = Ok(());
            for part in parts {
                if let Err(err) = feed_range(&store, &key, part, &mut stdin).await {
                    result = Err(err);
                    break;
                }
            }
            result
        }
    };
    // The child's input ends here, so it can read the whole file.
    let _closed = stdin.shutdown().await;
    result.map_err(|_err| ConversionError::Refused(Refusal::ResourceLimit))
}

/// Feed the child one part of a stored object, in pieces of
/// [`FEED_CHUNK_BYTES`]: the API never holds more than one piece at a time.
async fn feed_range(
    store: &UploadStore,
    key: &str,
    part: Part,
    stdin: &mut ChildStdin,
) -> std::io::Result<()> {
    let mut done: u64 = 0;
    while done < part.length {
        let len = FEED_CHUNK_BYTES.min(part.length - done);
        let chunk = store
            .read_range(key, part.offset.saturating_add(done), len)
            .await
            .map_err(|_err| std::io::Error::other("the store could not be read"))?;
        if chunk.is_empty() {
            break;
        }
        stdin.write_all(&chunk).await?;
        done = done.saturating_add(chunk.len() as u64);
    }
    Ok(())
}

/// Whether `binary` is this API's own binary, which is what the child must
/// be.
///
/// The child is this binary started again as `inspect-upload`, so its name is
/// `lakehouse-api`. The name is what is checked, not the path: a TEST binary
/// would run the test suite again, and a path is not a proof of anything
/// else. In a deployment the path is [`std::env::current_exe`], which is this
/// binary; in a test it is the child Cargo built (`CARGO_BIN_EXE_lakehouse-api`).
fn is_our_binary(binary: &Path) -> bool {
    binary
        .file_stem()
        .is_some_and(|stem| stem == "lakehouse-api")
}

/// Whether the answer line fits its cap: it ends with a newline, and it is
/// at most [`MAX_ANSWER_BYTES`] bytes of content.
///
/// A line that does not end with a newline is a child that stopped in the
/// middle of it, or a line that is over the cap: the caller cannot tell the
/// two apart, and answers both the same way.
fn answer_line_fits(line: &[u8]) -> bool {
    let Some(last) = line.last() else {
        return false;
    };
    if *last != b'\n' {
        return false;
    }
    line.len() - 1 <= MAX_ANSWER_BYTES
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
            Operation::parse(&["inspect-parquet".to_owned(), "1024".to_owned()]),
            Some(Operation::InspectParquet { size: 1024 })
        );
        assert_eq!(
            Operation::parse(&[
                "preview-parquet".to_owned(),
                "10".to_owned(),
                "1024".to_owned(),
                "16".to_owned(),
                "64".to_owned(),
            ]),
            Some(Operation::PreviewParquet {
                rows: 10,
                size: 1024,
                group_offset: 16,
                group_len: 64,
            })
        );
        assert_eq!(Operation::parse(&["inspect-parquet".to_owned()]), None);
        assert_eq!(
            Operation::parse(&["preview-parquet".to_owned(), "10".to_owned()]),
            None,
            "a preview needs the file's size and its row group's range"
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
    ///
    /// The child is given the file's TAIL for the footer, and that tail and
    /// the first row group for the rows (`SEC-22` P1).
    #[test]
    fn the_child_answers_with_a_parquet_file_and_its_rows() {
        let size = u64::try_from(ORDERS.len()).unwrap();
        let mut out: Vec<u8> = Vec::new();
        let code = run(
            &Operation::InspectParquet { size },
            &tail_of(ORDERS),
            &limits(),
            &mut out,
        );
        assert_eq!(code, exit::OK);
        let (answer, text) = split(&out);
        let Answer::Parquet { info, group } = answer else {
            panic!("the answer is the footer: {answer:?}");
        };
        assert_eq!(info.rows, 3);
        assert!(text.is_empty(), "the footer is no text");
        let group = group.expect("the file has a row group");

        let mut out: Vec<u8> = Vec::new();
        let code = run(
            &Operation::PreviewParquet {
                rows: 10,
                size,
                group_offset: group.offset,
                group_len: group.len,
            },
            &preview_input(ORDERS, group),
            &limits(),
            &mut out,
        );
        assert_eq!(code, exit::OK);
        let (answer, text) = split(&out);
        let Answer::Parquet { info, .. } = answer else {
            panic!("the answer is the footer: {answer:?}");
        };
        assert_eq!(info.rows, 3);
        assert!(
            String::from_utf8(text)
                .unwrap()
                .starts_with("sku,name,qty,price,received\n")
        );
    }

    /// The tail of a Parquet file: the bytes the child is given to read the
    /// footer out of.
    fn tail_of(bytes: &[u8]) -> Vec<u8> {
        let bound = usize::try_from(PARQUET_TAIL_BYTES).unwrap_or(usize::MAX);
        let len = bytes.len().min(bound);
        bytes[bytes.len() - len..].to_vec()
    }

    /// The bytes the child is given for a preview: the file's tail, then its
    /// first row group's bytes.
    fn preview_input(bytes: &[u8], group: GroupRange) -> Vec<u8> {
        let mut input = tail_of(bytes);
        let from = usize::try_from(group.offset).unwrap();
        let to = from + usize::try_from(group.len).unwrap();
        input.extend_from_slice(&bytes[from..to]);
        input
    }

    /// A column with no text form is refused, and the refusal names it.
    #[test]
    fn a_column_with_no_text_form_is_refused_by_name() {
        let size = u64::try_from(BINARY.len()).unwrap();
        let mut out: Vec<u8> = Vec::new();
        let code = run(
            &Operation::InspectParquet { size },
            &tail_of(BINARY),
            &limits(),
            &mut out,
        );
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
        assert_eq!(
            limits.memory_bytes,
            u64::from(DEFAULT_MEMORY_MIB) * 1024 * 1024
        );
        assert_eq!(limits.cpu_seconds, DEFAULT_CPU_SECONDS);
        assert_eq!(
            ChildLimits::with_memory(Some("512")).memory_bytes,
            512 * 1024 * 1024
        );
        // A limit that is not a number is no limit: the default stands.
        for raw in ["not a number", "0", "", "  "] {
            assert_eq!(
                ChildLimits::with_memory(Some(raw)).memory_bytes,
                u64::from(DEFAULT_MEMORY_MIB) * 1024 * 1024,
                "{raw:?}"
            );
        }
    }
}
