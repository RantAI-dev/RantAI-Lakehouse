/**
 * Pure rules of the "Upload file" screens (`src/features/connectors/upload-*`),
 * kept here so they are testable without a DOM (CODE-STANDARD 4.6). Plan T10
 * of `docs/superpowers/plans/2026-10-02-upload-file.md`.
 *
 * Every rule here repeats one the API enforces (`rust/crates/lakehouse-api/
 * src/routes/uploads.rs`). The API is the authority; these exist so the
 * screen can say no BEFORE a request, in the API's own words, and never say
 * yes to something the API will refuse.
 */

import { formatBytes } from "@/lib/format"
import type { Candidate } from "@/lib/upload-queue"
import type {
  IngestUploadInput,
  Upload,
  UploadColumnSpec,
  UploadColumnType,
  UploadDateFormat,
  UploadArchive,
  UploadEncoding,
  UploadLayout,
  UploadMismatch,
  UploadPreviewColumn,
  UploadStatus,
} from "@/services/contracts/uploads"

/**
 * The largest file the API accepts (its `MAX_UPLOAD_BYTES`): 50 MiB, which
 * its sentence calls "50 MB". A bigger file is refused in the browser, before
 * a byte of it is sent.
 */
export const MAX_UPLOAD_BYTES = 50 * 1024 * 1024

/** The most files one upload takes (decision 5 of `docs/core/features/upload-big-files-and-more-formats.md`). */
export const MAX_FILES = 10

/** The longest raw table name the API accepts (its `MAX_TABLE_NAME_CHARS`). */
export const MAX_TABLE_NAME_CHARS = 128

/**
 * The sentence for a table name that breaks the rule. Word for word the
 * API's `TABLE_NAME_RULE`, which is what `POST /api/uploads/{id}/ingest`
 * answers (400); a test pins it here. The screen shows this sentence as the
 * hint under the field and, when the name breaks the rule, as the error.
 */
export const TABLE_NAME_RULE =
  "Table names start with a lower-case letter and use lower-case letters and digits joined by single underscores, with at most 128 characters."

/**
 * The rule behind it: a lower-case letter first, then lower-case letters and
 * digits in groups joined by single underscores (no leading, trailing or
 * doubled `_`). Stricter than a connector's target, which keeps
 * `^[a-z_][a-z0-9_]*$`: the writer renames names that rule admits (`x_` is
 * written as `xx`, `__x` as `x`), so the rows would land in a table nobody
 * asked for (review finding C1, measured over 59,052 names the stricter rule
 * admits, none renamed). `$` without the `m` flag matches only at the very
 * end, so a name ending in a line break is refused, as the API refuses it.
 */
const TABLE_NAME_PATTERN = /^[a-z][a-z0-9]*(_[a-z0-9]+)*$/

/** Why `name` cannot be a raw table's name, or `null`. The sentence is the API's. */
export function tableNameProblem(name: string): string | null {
  // The length first, so a huge paste never reaches the pattern.
  if (name.length > MAX_TABLE_NAME_CHARS || !TABLE_NAME_PATTERN.test(name)) return TABLE_NAME_RULE
  return null
}

/**
 * The specific reason `name` breaks the table-name rule, in a sentence, or
 * `null` when it does not break it. For the hint under the field, which turns
 * from the rule into this; `tableNameProblem` stays the judge (this returns
 * `null` exactly when it does) and its rule sentence is the fallback for a
 * break none of the checks below names.
 */
export function tableNameFault(name: string): string | null {
  if (tableNameProblem(name) === null) return null
  if (name.length > MAX_TABLE_NAME_CHARS) {
    return `The name has ${name.length} characters; the most is ${MAX_TABLE_NAME_CHARS}.`
  }
  if (/^[0-9]/.test(name)) return "A name cannot start with a digit; it starts with a lower-case letter."
  if (name.startsWith("_")) return "A name cannot start with an underscore; it starts with a lower-case letter."
  if (/[A-Z]/.test(name)) return "Upper-case letters are not allowed: use lower case."
  if (/\s/.test(name)) return "Spaces are not allowed: use an underscore."
  const odd = [...name].find((ch) => !/[a-z0-9_]/.test(ch))
  if (odd !== undefined) return `"${odd}" is not allowed: use lower-case letters, digits and underscores.`
  if (name.endsWith("_")) return "A name cannot end with an underscore."
  if (name.includes("__")) return "Underscores are single: a name cannot have two in a row."
  return TABLE_NAME_RULE
}

/** What a table is called when a file's name leaves nothing to build one from. */
const FALLBACK_TABLE_NAME = "uploaded_file"

/**
 * A table name for a file, always one `tableNameProblem` accepts.
 *
 * The extension goes (the same shape the API gives a stored object: up to
 * eight letters or digits after the last dot), the rest is lower-cased, every
 * run of anything but a lower-case ASCII letter or digit becomes one `_`
 * (so `Größe` is `gr_e`, the name the writer would give it anyway), and
 * leading and trailing `_` go. A name that would start with a digit gets `t_`
 * in front. It is cut to 128 characters, and a cut that ends on `_` loses it.
 * When nothing is left (`!!!.csv`, `データ.csv`), the fixed `uploaded_file`.
 */
export function suggestTableName(fileName: string): string {
  const base = fileName.replace(/\.[A-Za-z0-9]{1,8}$/, "")
  const slug = base
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "_")
    .replace(/^_+|_+$/g, "")
  if (slug === "") return FALLBACK_TABLE_NAME
  const named = /^[0-9]/.test(slug) ? `t_${slug}` : slug
  return named.slice(0, MAX_TABLE_NAME_CHARS).replace(/_+$/, "")
}

/** What a status is called on screen. A status this version does not know is shown as received. */
export function statusLabel(status: UploadStatus): string {
  switch (status) {
    case "receiving":
      return "Receiving"
    case "uploaded":
      return "Uploaded"
    case "ingesting":
      return "Loading"
    case "ingested":
      return "Loaded"
    case "failed":
      return "Failed"
    default:
      return status
  }
}

/** What a delimiter is called on screen. A character the API does not offer is shown as it is. */
export function delimiterLabel(delimiter: string): string {
  switch (delimiter) {
    case ",":
      return "Comma"
    case ";":
      return "Semicolon"
    case "\t":
      return "Tab"
    case "|":
      return "Pipe"
    default:
      return delimiter
  }
}

/** What an encoding is called on screen. */
export function encodingLabel(encoding: UploadEncoding | string): string {
  switch (encoding) {
    case "utf-8":
      return "UTF-8"
    case "utf-16":
      return "UTF-16"
    default:
      return encoding
  }
}

/** The encodings the API reads, in the order a menu lists them. */
export const UPLOAD_ENCODINGS: readonly { value: UploadEncoding; label: string; disabled?: boolean }[] = [
  { value: "utf-8", label: encodingLabel("utf-8") },
  { value: "utf-16", label: encodingLabel("utf-16") },
]

/**
 * The delimiters the API reads (its `DELIMITERS`), in the order a menu lists
 * them. The tab is the tab character itself, which is what the API sends and
 * takes.
 */
export const UPLOAD_DELIMITERS: readonly { value: string; label: string; disabled?: boolean }[] = [",", ";", "\t", "|"].map(
  (value) => ({ value, label: delimiterLabel(value) })
)

/**
 * The header row as a person counts it: from 1. The API counts records from
 * 0. These two functions are the only place the two are converted; a screen
 * never adds or subtracts 1 itself.
 */
export function headerRowDisplay(headerRow: number): number {
  return headerRow + 1
}

/**
 * What a person typed in the header-row field, as the API's zero-based index,
 * or `null` when it is not a whole number of 1 or more (empty, `0`, `-1`,
 * `1.5`, `2e3`, text).
 */
export function headerRowFromDisplay(text: string): number | null {
  const trimmed = text.trim()
  if (!/^[0-9]+$/.test(trimmed)) return null
  const row = Number(trimmed)
  return Number.isSafeInteger(row) && row >= 1 ? row - 1 : null
}

/**
 * Whether `name` is a table an earlier upload of this tenant created: one a
 * live upload has loaded. Only then does a load into it have a choice to make
 * (replace its rows, or add to them); a name nobody has loaded into is a new
 * table.
 *
 * The API has no read for who owns a name, so this goes by the list. A table
 * whose upload was later deleted is still the tenant's and loads, but cannot
 * be seen from here, so no choice is offered for it and the load replaces.
 */
export function isUploadedTable(name: string, uploads: readonly Upload[]): boolean {
  return uploads.some((upload) => upload.status === "ingested" && upload.bronzeTable === name)
}

/**
 * The reason `file_ingest_job` records when the rows were written and only the
 * catalog entry failed (`JOB_NOT_REGISTERED` in `routes/uploads.rs`, one of
 * the seven in `ops/fixtures/upload_load_failure_reasons.json`, which a test
 * reads). It is the one failure after which the data IS in the table, so a
 * retry that adds rows would add them a second time.
 */
export const REGISTRATION_FAILED_REASON = "The table was loaded but could not be registered in the catalog."

/**
 * Why a chosen file cannot be sent, or `null`. Only the size is judged here
 * (it is what the browser knows before a byte leaves); an empty file, a
 * workbook that does not open or a file that is not delimited text is the
 * API's to refuse, in its own sentence. The sentence names the limit and says
 * what to do next.
 */
export function fileSizeProblem(name: string, sizeBytes: number): string | null {
  if (sizeBytes <= MAX_UPLOAD_BYTES) return null
  return `${name} is ${formatBytes(sizeBytes)}, over the 50 MB limit. Split it into smaller files or leave out columns you do not need, then choose a file again.`
}

/**
 * The extensions of files a person must not send. The words follow the API's
 * `WORKBOOK` and `OTHER_BINARY` (`routes/uploads.rs`); the API still decides by
 * content, so a file that passes here can be refused there (a `.parquet` with a
 * binary column, say), and only a name that is clearly not a kind it reads is
 * refused here. The workbook kinds the API does not open (`.xlsm`, `.xlsb`,
 * `.ods`) and the archives it does not open (anything but `.gz` and `.zip`)
 * are refused with the workbook-or-archive sentence.
 */
const WORKBOOK_EXTENSIONS = ["xlsm", "xlsb", "ods"]
const ARCHIVE_EXTENSIONS = ["tgz", "bz2", "xz", "7z", "rar", "tar"]
const OTHER_BINARY_EXTENSIONS = ["pdf", "doc", "docx", "ppt", "pptx", "orc", "db", "sqlite"]

/**
 * Why a file cannot be sent judging by its NAME alone, or `null`. A workbook
 * the API does not read, an archive it does not read (`.tar`, `.7z` ...) or
 * another binary is refused at once, before any byte is sent, in words
 * consistent with the API's. `.xls`, `.xlsx`, `.parquet`, `.json`, `.jsonl`,
 * `.ndjson`, `.avro`, `.gz` and `.zip` pass: the API opens them, and refuses one
 * that does not open, in its own sentence. A name with no
 * extension or an unknown one (`.txt`, `.dat`, an extension-less export) is
 * never refused here: it may be delimited text, and the API judges content.
 * Case does not matter.
 */
export function fileKindProblem(name: string): string | null {
  const dot = name.lastIndexOf(".")
  if (dot < 0) return null
  const extension = name.slice(dot + 1).toLowerCase()
  if (WORKBOOK_EXTENSIONS.includes(extension) || ARCHIVE_EXTENSIONS.includes(extension)) {
    return `${name} looks like a workbook or an archive this upload does not read. Excel workbooks must be .xls or .xlsx, and archives .gz or .zip; or save the sheet as .xlsx or CSV first.`
  }
  if (OTHER_BINARY_EXTENSIONS.includes(extension)) {
    return `${name} is not a file this upload reads. Upload CSV, TSV, JSON, JSON Lines, Avro, Parquet or Excel (.xls, .xlsx) files, a .gz of a text or JSON file, or a .zip of such files.`
  }
  return null
}

/** Why a chosen file cannot be sent (its kind by name, then its size), or `null`. */
export function fileProblem(name: string, sizeBytes: number): string | null {
  return fileKindProblem(name) ?? fileSizeProblem(name, sizeBytes)
}

/**
 * What a pick of files becomes: `candidates` for the upload queue, in order,
 * and how many files of the pick were left out because the list is full.
 *
 * - The list holds at most [`MAX_FILES`]; the first files of a pick are kept
 *   (decision 5) and `dropped` counts the rest.
 * - A file with a problem of its own ([`fileProblem`]) is listed with it.
 * - The 50 MB limit counts all the files of one upload together (decision 2):
 *   a file that would take the sum over it is listed with a sentence saying
 *   so and is not sent, so a person never waits for files the load would
 *   refuse. `existing` is what the list holds already (a refused file does not
 *   count: it is not sent).
 */
export function planPick(
  existing: readonly { readonly size: number; readonly state: string }[],
  picked: readonly File[]
): { candidates: Candidate[]; dropped: number } {
  const room = Math.max(0, MAX_FILES - existing.length)
  const taken = picked.slice(0, room)
  let total = existing.filter((f) => f.state !== "refused").reduce((sum, f) => sum + f.size, 0)
  const candidates = taken.map((file): Candidate => {
    const own = fileProblem(file.name, file.size)
    if (own !== null) return { file, problem: own }
    if (total + file.size > MAX_UPLOAD_BYTES) {
      return {
        file,
        problem: `${file.name} would bring the files to ${formatBytes(total + file.size)}, over the 50 MB limit for one upload (all its files together). Remove a file, then choose this one again.`,
      }
    }
    total += file.size
    return { file, problem: null }
  })
  return { candidates, dropped: picked.length - taken.length }
}

/**
 * The body "Try again" sends: exactly what the failed load was told, which
 * the API records on the upload (`parseOptions`, `bronzeTable`, `loadMode`),
 * or `null` when the upload does not carry all of it (nothing is guessed).
 *
 * The one exception to "the same as before" is a load that failed only at
 * the catalog (`REGISTRATION_FAILED_REASON`): its rows ARE in the table, so a
 * retry with `append` would write them twice. That retry replaces, whatever
 * the first load chose.
 *
 * `alsoUploads` are the other files of a load of several files (`DATA-17` slice 5), which the
 * upload row of the first file does not list: the screen knows them (the files it loaded with).
 */
export function retryInput(upload: Upload, alsoUploads: readonly string[] = []): IngestUploadInput | null {
  const { parseOptions, bronzeTable } = upload
  if (parseOptions === undefined || bronzeTable === undefined) return null
  const mode =
    upload.error === REGISTRATION_FAILED_REASON
      ? "replace"
      : upload.loadMode === "append"
        ? "append"
        : "replace"
  // `DATA-18`: what the load was told about the columns goes back with it, or a retry
  // would load everything as text.
  const withColumns =
    upload.columns !== undefined && upload.onBadValue !== undefined
      ? { columns: upload.columns, onBadValue: upload.onBadValue }
      : {}
  // `DATA-17` slice 5: a load of several files is retried with the same files.
  const withOthers = alsoUploads.length > 0 ? { alsoUploads: [...alsoUploads] } : {}
  return { ...parseOptions, bronzeTable, mode, ...withColumns, ...withOthers }
}

/**
 * The name a raw table has in Data Explorer and in queries: the writer puts
 * it in the `bronze` namespace (`Iceberg bronze.<table>` in
 * `dagster/dispar_orchestrate/file_ingest.py`; `BRONZE_NAMESPACE` in
 * `routes/catalog_source.rs`). Shown on the Table and Review steps so the
 * person sees where the rows will land before pressing Load.
 */
export function rawTableFullName(table: string): string {
  return `bronze.${table}`
}

/** What a chosen file looks like to the eye: an Excel workbook, a Parquet file, a JSON, JSON Lines or Avro file, an archive (gzip, zip), CSV or TSV text, or anything else. */
export type UploadFileKind = "workbook" | "parquet" | "json" | "avro" | "archive" | "delimited" | "other"

/** The extension of a file name, lower-cased, or `""` when there is none (`.csv` alone and `a.` have none). */
function extensionOf(name: string): string {
  const dot = name.lastIndexOf(".")
  return dot <= 0 ? "" : name.slice(dot + 1).toLowerCase()
}

/**
 * The kind of a file by its name only, for the mark beside it (presentation;
 * the API still judges by content). `.txt`, an unknown extension and no
 * extension are `other`. Case does not matter.
 */
export function uploadFileKind(name: string): UploadFileKind {
  const extension = extensionOf(name)
  if (extension === "xls" || extension === "xlsx") return "workbook"
  if (extension === "parquet") return "parquet"
  if (extension === "json" || extension === "jsonl" || extension === "ndjson") return "json"
  if (extension === "avro") return "avro"
  if (extension === "gz" || extension === "zip") return "archive"
  if (extension === "csv" || extension === "tsv") return "delimited"
  return "other"
}

/** The longest extension shown as a label; a longer one would not fit the mark. */
const MAX_LABEL_CHARS = 7

/** The extension in capitals for the mark (`XLS`, `CSV`), or `null` when there is none or it is too long to fit. */
export function uploadFileLabel(name: string): string | null {
  const extension = extensionOf(name)
  if (extension === "" || extension.length > MAX_LABEL_CHARS || !/^[a-z0-9]+$/.test(extension)) return null
  return extension.toUpperCase()
}

// ── JSON, Avro, gzip and zip (`DATA-17` slice 4) ──────────────────────────

/** Whether the file has no encoding, delimiter or header row to choose: its keys or fields are the columns. */
export function hasFixedDialect(layout: UploadLayout | undefined): boolean {
  return layout !== undefined && layout.format !== "delimited"
}

/** What a file is read as, in words (`JSON`, `CSV or TSV in a gzip file`), for the Review step. */
export function layoutLabel(layout: UploadLayout): string {
  const what =
    layout.format === "json"
      ? "JSON"
      : layout.format === "jsonl"
        ? "JSON Lines"
        : layout.format === "avro"
          ? "Avro"
          : "CSV or TSV"
  if (layout.compression === "gzip") return `${what} in a gzip file`
  if (layout.compression === "zip") return `${what} in a zip archive`
  return what
}

/**
 * Why a zip cannot be loaded as it is, or `null`: a file whose columns differ from the first
 * file's. The API refuses the same load (`ZIP_COLUMNS_DIFFER`); this tells the person on the
 * Check step, where Next stays off.
 */
export function archiveProblem(archive: UploadArchive | undefined): string | null {
  if (archive === undefined) return null
  const different = archive.files.filter((file) => !file.matchesFirst)
  if (different.length === 0) return null
  return `${different.length === 1 ? "One file" : `${different.length} files`} of the zip ${
    different.length === 1 ? "does not have" : "do not have"
  } the same columns as the first. A zip loads whole, so make the files agree and upload it again.`
}

// ── Several files into one table (`DATA-17` slice 5) ──────────────────────

/**
 * What differs in a file that cannot load with the first one, as short lines for the list:
 * the columns it lacks, the ones it adds, and each column whose values do not fit the first
 * file's type. The names are those the load gives the columns.
 */
export function mismatchLines(mismatch: UploadMismatch): string[] {
  const lines: string[] = []
  if (mismatch.missing.length > 0) lines.push(`Missing: ${mismatch.missing.join(", ")}`)
  if (mismatch.extra.length > 0) lines.push(`Not in the first file: ${mismatch.extra.join(", ")}`)
  for (const conflict of mismatch.typeConflicts) {
    lines.push(
      `${conflict.column}: the first file reads it as ${columnTypeLabel(conflict.expected).toLowerCase()}, here it looks like ${columnTypeLabel(conflict.found).toLowerCase()}`
    )
  }
  return lines
}

/**
 * Why Load stays off for a load of several files, or `null` (decision 4): a file still being
 * sent is not in the load yet, and a file whose columns differ must be removed.
 */
export function groupProblem(input: {
  readonly pending: number
  readonly mismatched: number
}): string | null {
  if (input.mismatched > 0) {
    return `${input.mismatched === 1 ? "One file does" : `${input.mismatched} files do`} not have the columns of the first. Remove ${
      input.mismatched === 1 ? "it" : "them"
    } to load the rest.`
  }
  if (input.pending > 0) {
    return `${input.pending === 1 ? "One file is" : `${input.pending} files are`} still being sent. Load waits until ${
      input.pending === 1 ? "it arrives" : "they arrive"
    }.`
  }
  return null
}

// ── Column types (`DATA-18`) ──────────────────────────────────────────────

/** The six types a person chooses among (decision 2), in the order the select lists them. */
export const COLUMN_TYPES: readonly { value: UploadColumnType; label: string }[] = [
  { value: "text", label: "Text" },
  { value: "integer", label: "Integer" },
  { value: "decimal", label: "Decimal" },
  { value: "boolean", label: "Boolean" },
  { value: "date", label: "Date" },
  { value: "timestamp", label: "Timestamp" },
]

/** A type on screen. `float` only comes from a file that carries one (decision 10). */
export function columnTypeLabel(type: UploadColumnType): string {
  if (type === "float") return "Floating point (from the file)"
  return COLUMN_TYPES.find((t) => t.value === type)?.label ?? type
}

/** How a date is read, as the select lists it. */
export const DATE_FORMATS: readonly { value: UploadDateFormat; label: string }[] = [
  { value: "iso", label: "Year first (2026-10-16)" },
  { value: "dmy", label: "Day first (16/10/2026)" },
  { value: "mdy", label: "Month first (10/16/2026)" },
]

/** The API's sentence for a column name that breaks the rule (`NAME_RULE` in `upload_types.rs`). */
export const COLUMN_NAME_RULE =
  "Column names start with a lower-case letter and use lower-case letters and digits joined by single underscores, with at most 128 characters, and each name is used once."

const COLUMN_NAME = /^[a-z][a-z0-9]*(_[a-z0-9]+)*$/

/** Why a typed column name is refused, or `null`: the API's rule, which this repeats so the person is told while typing. */
export function columnNameProblem(name: string): string | null {
  return COLUMN_NAME.test(name) && name.length <= 128 && !name.startsWith("_dlt") ? null : COLUMN_NAME_RULE
}

/**
 * What stops a column list from being loaded, as sentences: a name that breaks the
 * rule or is used twice, and a date or timestamp whose reading is still a question
 * ("day first or month first?", decision 4). Empty when the list may go on.
 */
export function columnListProblems(columns: readonly UploadPreviewColumn[]): string[] {
  const problems: string[] = []
  const seen = new Set<string>()
  for (const column of columns) {
    if (columnNameProblem(column.name) !== null) problems.push(COLUMN_NAME_RULE)
    else if (seen.has(column.name)) problems.push(COLUMN_NAME_RULE)
    seen.add(column.name)
    if ((column.type === "date" || column.type === "timestamp") && column.dateFormat === undefined) {
      problems.push(`Say how the dates of ${column.name || column.source} are read: day first or month first.`)
    }
  }
  return [...new Set(problems)]
}

/** What goes to the API for a column: the spec, without what the screen keeps for itself. */
export function columnSpecOf(column: UploadPreviewColumn): UploadColumnSpec {
  const dated = column.type === "date" || column.type === "timestamp"
  return {
    source: column.source,
    name: column.name,
    type: column.type,
    ...(dated && column.dateFormat !== undefined ? { dateFormat: column.dateFormat } : {}),
    ...(column.type === "timestamp" ? { timezone: column.timezone ?? false } : {}),
  }
}

/** The column as it is after a change of type: a type that has no date reading or zone sheds them, one that has takes the guess's or a default. */
export function withColumnType(column: UploadPreviewColumn, type: UploadColumnType): UploadPreviewColumn {
  const dated = type === "date" || type === "timestamp"
  const { dateFormat, timezone, ...rest } = column
  return {
    ...rest,
    type,
    ...(dated ? { dateFormat: dateFormat ?? undefined } : {}),
    ...(type === "timestamp" ? { timezone: timezone ?? false } : {}),
  }
}
