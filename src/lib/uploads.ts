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
import type {
  IngestUploadInput,
  Upload,
  UploadEncoding,
  UploadStatus,
} from "@/services/contracts/uploads"

/**
 * The largest file the API accepts (its `MAX_UPLOAD_BYTES`): 50 MiB, which
 * its sentence calls "50 MB". A bigger file is refused in the browser, before
 * a byte of it is sent.
 */
export const MAX_UPLOAD_BYTES = 50 * 1024 * 1024

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
export const UPLOAD_ENCODINGS: readonly { value: UploadEncoding; label: string }[] = [
  { value: "utf-8", label: encodingLabel("utf-8") },
  { value: "utf-16", label: encodingLabel("utf-16") },
]

/**
 * The delimiters the API reads (its `DELIMITERS`), in the order a menu lists
 * them. The tab is the tab character itself, which is what the API sends and
 * takes.
 */
export const UPLOAD_DELIMITERS: readonly { value: string; label: string }[] = [",", ";", "\t", "|"].map(
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
 * workbook or a file that is not delimited text is the API's to refuse, in
 * its own sentence, so a refusal the checklist expects (workbook: "save it as
 * CSV") comes from one place. The sentence names the limit and says what to
 * do next.
 */
export function fileSizeProblem(name: string, sizeBytes: number): string | null {
  if (sizeBytes <= MAX_UPLOAD_BYTES) return null
  return `${name} is ${formatBytes(sizeBytes)}, over the 50 MB limit. Split it into smaller files or leave out columns you do not need, then choose a file again.`
}

/**
 * The sentence for a file a person must not send, and the extensions it is
 * for. The words follow the API's `WORKBOOK`, `PARQUET` and `OTHER_BINARY`
 * (`routes/uploads.rs`); the API still decides by content, so a file that
 * passes here can be refused there, and only a name that is clearly not
 * delimited text is refused here.
 */
const WORKBOOK_EXTENSIONS = ["xls", "xlsx", "xlsm", "xlsb", "ods"]
const ARCHIVE_EXTENSIONS = ["zip", "gz", "tgz", "bz2", "xz", "7z", "rar", "tar"]
const OTHER_BINARY_EXTENSIONS = ["pdf", "doc", "docx", "ppt", "pptx", "avro", "orc", "db", "sqlite"]

/**
 * Why a file cannot be sent judging by its NAME alone, or `null`. An Excel
 * workbook, archive, Parquet file or other binary is refused at once, before
 * any byte is sent, in words consistent with the API's. A name with no
 * extension or an unknown one (`.txt`, `.dat`, an extension-less export) is
 * never refused here: it may be delimited text, and the API judges content.
 * Case does not matter.
 */
export function fileKindProblem(name: string): string | null {
  const dot = name.lastIndexOf(".")
  if (dot < 0) return null
  const extension = name.slice(dot + 1).toLowerCase()
  if (WORKBOOK_EXTENSIONS.includes(extension) || ARCHIVE_EXTENSIONS.includes(extension)) {
    return `${name} looks like an Excel workbook or a zip archive. Only delimited text files (CSV, TSV) can be uploaded; save the sheet as CSV first.`
  }
  if (extension === "parquet") {
    return `${name} looks like a Parquet file. Only delimited text files (CSV, TSV) can be uploaded.`
  }
  if (OTHER_BINARY_EXTENSIONS.includes(extension)) {
    return `${name} is not a delimited text file. Only delimited text files (CSV, TSV) can be uploaded.`
  }
  return null
}

/** Why a chosen file cannot be sent (its kind by name, then its size), or `null`. */
export function fileProblem(name: string, sizeBytes: number): string | null {
  return fileKindProblem(name) ?? fileSizeProblem(name, sizeBytes)
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
 */
export function retryInput(upload: Upload): IngestUploadInput | null {
  const { parseOptions, bronzeTable } = upload
  if (parseOptions === undefined || bronzeTable === undefined) return null
  const mode =
    upload.error === REGISTRATION_FAILED_REASON
      ? "replace"
      : upload.loadMode === "append"
        ? "append"
        : "replace"
  return { ...parseOptions, bronzeTable, mode }
}
