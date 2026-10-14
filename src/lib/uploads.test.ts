import assert from "node:assert/strict"
import { readFileSync } from "node:fs"
import { join } from "node:path"
import { test } from "node:test"
import type { Upload } from "@/services/contracts/uploads"
import {
  MAX_TABLE_NAME_CHARS,
  MAX_UPLOAD_BYTES,
  REGISTRATION_FAILED_REASON,
  TABLE_NAME_RULE,
  UPLOAD_DELIMITERS,
  UPLOAD_ENCODINGS,
  delimiterLabel,
  encodingLabel,
  fileKindProblem,
  fileProblem,
  fileSizeProblem,
  headerRowDisplay,
  headerRowFromDisplay,
  isUploadedTable,
  rawTableFullName,
  retryInput,
  statusLabel,
  suggestTableName,
  tableNameFault,
  tableNameProblem,
  uploadFileKind,
  uploadFileLabel,
} from "./uploads"

test("MAX_UPLOAD_BYTES is the API's cap, 50 MiB", () => {
  assert.equal(MAX_UPLOAD_BYTES, 50 * 1024 * 1024)
  assert.equal(MAX_UPLOAD_BYTES, 52_428_800)
})

test("tableNameProblem says the API's sentence, word for word", () => {
  // `TABLE_NAME_RULE` in `rust/crates/lakehouse-api/src/routes/uploads.rs`;
  // a test there pins the same text, so the two cannot drift unseen.
  assert.equal(
    TABLE_NAME_RULE,
    "Table names start with a lower-case letter and use lower-case letters and digits joined by single underscores, with at most 128 characters."
  )
  assert.equal(tableNameProblem("Orders"), TABLE_NAME_RULE)
})

test("tableNameProblem accepts what the API accepts", () => {
  for (const name of [
    "a",
    "a1",
    "a_1",
    "sap_material_master",
    "qa_upload_a",
    "t_2025_stock",
    "a".repeat(MAX_TABLE_NAME_CHARS),
    `a${"_b".repeat(63)}`,
  ]) {
    assert.equal(tableNameProblem(name), null, name)
  }
})

test("tableNameProblem refuses what the API refuses", () => {
  const refused = [
    "x_", // trailing underscore: the writer would call it `xx`
    "_x", // leading underscore
    "__x",
    "a__b", // doubled underscore
    "1a", // starts with a digit
    "Orders", // upper case is refused, never folded
    "orders 2025",
    "Orders 2025",
    "orders-2025",
    "orders.2025",
    "größe", // non-ASCII letters
    "名前",
    "",
    " ",
    "orders\n", // a line break at the end must not slip past `$`
    "a".repeat(MAX_TABLE_NAME_CHARS + 1),
    `${"a_".repeat(50_000)}a`, // a long paste: refused by its length, not walked
  ]
  for (const name of refused) {
    assert.equal(tableNameProblem(name), TABLE_NAME_RULE, JSON.stringify(name.slice(0, 40)))
  }
})

test("suggestTableName drops the extension, lower-cases and joins words with one underscore", () => {
  assert.equal(suggestTableName("stock.csv"), "stock")
  assert.equal(suggestTableName("Q1 Sales Report (final).CSV"), "q1_sales_report_final")
  assert.equal(suggestTableName("material-master_2025.tsv"), "material_master_2025")
  assert.equal(suggestTableName("no extension"), "no_extension")
  assert.equal(suggestTableName("archive.tar.gz"), "archive_tar")
})

test("suggestTableName puts a letter group in front of a name that starts with a digit", () => {
  assert.equal(suggestTableName("2025 Stock.xls"), "t_2025_stock")
  assert.equal(suggestTableName("123.csv"), "t_123")
})

test("suggestTableName trims underscores and falls back when nothing is left", () => {
  assert.equal(suggestTableName("__x__.csv"), "x")
  assert.equal(suggestTableName("  --report--  .csv"), "report")
  for (const name of ["!!!.csv", "", ".csv", "   ", "データ.csv", "---", "_"]) {
    assert.equal(suggestTableName(name), "uploaded_file", JSON.stringify(name))
  }
})

test("suggestTableName turns non-ASCII letters into separators, as the writer does", () => {
  assert.equal(suggestTableName("Größe.csv"), "gr_e")
  assert.equal(suggestTableName("café menu.csv"), "caf_menu")
})

test("suggestTableName cuts a long name to 128 characters and never ends on an underscore", () => {
  const long = suggestTableName(`${"a".repeat(300)}.csv`)
  assert.equal(long.length, MAX_TABLE_NAME_CHARS)
  assert.equal(tableNameProblem(long), null)

  // A cut that lands just after a separator must not leave it behind.
  const cutOnSeparator = suggestTableName(`${"a".repeat(127)} ${"b".repeat(50)}.csv`)
  assert.equal(cutOnSeparator, "a".repeat(127))
  assert.equal(tableNameProblem(cutOnSeparator), null)

  // The `t_` in front counts towards the 128.
  const digits = suggestTableName(`${"7".repeat(300)}.csv`)
  assert.equal(digits.length, MAX_TABLE_NAME_CHARS)
  assert.ok(digits.startsWith("t_7"))
  assert.equal(tableNameProblem(digits), null)
})

test("suggestTableName always suggests a name tableNameProblem accepts", () => {
  const awkward = [
    "2025 Stock.xls",
    "__x__.csv",
    "x_.csv",
    "_.csv",
    "!!!.csv",
    "データ.csv",
    "Größe & Maße.csv",
    "a".repeat(300),
    `${"ab ".repeat(100)}.csv`,
    "line\nbreak.csv",
    "tab\tname.tsv",
    "UPPER lower.CSV",
    "0.csv",
    "1.5 GB export.csv",
    "C:\\Users\\x\\stock.csv",
    "../../etc/passwd",
    "a..b...c.csv",
    "İstanbul.csv",
    "\u0000.csv",
    "😀😀.csv",
    "x".repeat(128) + "_" + "y".repeat(10),
  ]
  // And a deterministic spread of strings over an alphabet that holds every
  // trap: separators, digits, upper case, non-ASCII, a dot, a line break.
  const alphabet = ["a", "Z", "0", "9", "_", "-", " ", ".", "é", "名", "\n", "!"]
  let seed = 20_261_002
  // A 32-bit linear congruential generator, kept exact with `Math.imul`; the
  // high bits are used because the low ones of such a generator repeat.
  const next = (bound: number) => {
    seed = (Math.imul(seed, 1_664_525) + 1_013_904_223) >>> 0
    return (seed >>> 16) % bound
  }
  for (let i = 0; i < 4000; i += 1) {
    let name = ""
    const length = next(40)
    for (let j = 0; j < length; j += 1) name += alphabet[next(alphabet.length)]
    awkward.push(name)
  }
  for (const name of awkward) {
    const suggestion = suggestTableName(name)
    assert.equal(tableNameProblem(suggestion), null, `${JSON.stringify(name)} gave ${suggestion}`)
  }
})

test("statusLabel names the four statuses and shows an unknown one as received", () => {
  assert.equal(statusLabel("uploaded"), "Uploaded")
  assert.equal(statusLabel("ingesting"), "Loading")
  assert.equal(statusLabel("ingested"), "Loaded")
  assert.equal(statusLabel("failed"), "Failed")
  assert.equal(statusLabel("quarantined"), "quarantined")
  // Not looked up in an object, so a name from `Object.prototype` is not a label.
  assert.equal(statusLabel("constructor"), "constructor")
})

test("delimiterLabel names the four delimiters and shows another character as it is", () => {
  assert.equal(delimiterLabel(","), "Comma")
  assert.equal(delimiterLabel(";"), "Semicolon")
  assert.equal(delimiterLabel("\t"), "Tab")
  assert.equal(delimiterLabel("|"), "Pipe")
  assert.equal(delimiterLabel("~"), "~")
})

test("encodingLabel names the two encodings", () => {
  assert.equal(encodingLabel("utf-8"), "UTF-8")
  assert.equal(encodingLabel("utf-16"), "UTF-16")
  assert.equal(encodingLabel("latin-1"), "latin-1")
})

test("the menus list exactly what the API reads, with a tab as the tab character", () => {
  assert.deepEqual(UPLOAD_ENCODINGS, [
    { value: "utf-8", label: "UTF-8" },
    { value: "utf-16", label: "UTF-16" },
  ])
  assert.deepEqual(UPLOAD_DELIMITERS, [
    { value: ",", label: "Comma" },
    { value: ";", label: "Semicolon" },
    { value: "\t", label: "Tab" },
    { value: "|", label: "Pipe" },
  ])
})

test("headerRowDisplay counts from 1 where the API counts from 0", () => {
  assert.equal(headerRowDisplay(0), 1)
  assert.equal(headerRowDisplay(5), 6)
})

test("headerRowFromDisplay turns what a person typed into the API's index, or null", () => {
  assert.equal(headerRowFromDisplay("1"), 0)
  assert.equal(headerRowFromDisplay("7"), 6)
  assert.equal(headerRowFromDisplay(" 3 "), 2)
  for (const text of ["", " ", "0", "-1", "1.5", "2e3", "abc", "1 2", "+1", "9007199254740993"]) {
    assert.equal(headerRowFromDisplay(text), null, JSON.stringify(text))
  }
  // The two directions agree.
  for (const row of [0, 1, 6, 1000]) {
    assert.equal(headerRowFromDisplay(String(headerRowDisplay(row))), row)
  }
})

function upload(over: Partial<Upload>): Upload {
  return {
    id: "up-1",
    originalFilename: "stock.csv",
    sizeBytes: 10,
    uploadedBy: "Ana",
    status: "uploaded",
    createdAt: "2026-10-02T10:00:00Z",
    updatedAt: "2026-10-02T10:00:00Z",
    ...over,
  }
}

test("isUploadedTable is true only for a table a live upload has loaded", () => {
  const uploads = [
    upload({ id: "up-1", status: "ingested", bronzeTable: "stock" }),
    upload({ id: "up-2", status: "failed", bronzeTable: "tried" }),
    upload({ id: "up-3", status: "ingesting", bronzeTable: "busy" }),
    upload({ id: "up-4", status: "uploaded" }),
  ]
  assert.equal(isUploadedTable("stock", uploads), true)
  assert.equal(isUploadedTable("tried", uploads), false)
  assert.equal(isUploadedTable("busy", uploads), false)
  assert.equal(isUploadedTable("other", uploads), false)
  assert.equal(isUploadedTable("stock", []), false)
})

test("REGISTRATION_FAILED_REASON is one of the reasons the load job records", () => {
  // `ops/fixtures/upload_load_failure_reasons.json` is the file the API's
  // constants and the job's are asserted against; reading it here keeps the
  // console's copy from drifting away from them.
  const file = join(import.meta.dir, "..", "..", "ops", "fixtures", "upload_load_failure_reasons.json")
  const reasons: unknown = JSON.parse(readFileSync(file, "utf8"))
  assert.ok(Array.isArray(reasons))
  assert.ok(reasons.includes(REGISTRATION_FAILED_REASON))
  assert.equal(REGISTRATION_FAILED_REASON, "The table was loaded but could not be registered in the catalog.")
})

test("fileSizeProblem refuses only a file over 50 MB, names the limit and says what to do", () => {
  assert.equal(fileSizeProblem("a.csv", 0), null)
  assert.equal(fileSizeProblem("a.csv", MAX_UPLOAD_BYTES), null)
  const problem = fileSizeProblem("big.csv", MAX_UPLOAD_BYTES + 1)
  assert.ok(problem !== null)
  assert.match(problem, /^big\.csv is /)
  assert.match(problem, /50 MB limit/)
  assert.match(problem, /Split it/)
})

test("retryInput repeats what the load was told, and replaces after a registration failure", () => {
  const options = { encoding: "utf-8" as const, delimiter: ";", headerRow: 2 }
  const failed = upload({
    status: "failed",
    parseOptions: options,
    bronzeTable: "stock",
    loadMode: "append",
    error: "The load into the table failed.",
  })
  assert.deepEqual(retryInput(failed), { ...options, bronzeTable: "stock", mode: "append" })
  assert.deepEqual(retryInput({ ...failed, loadMode: "replace" }), { ...options, bronzeTable: "stock", mode: "replace" })
  // Rows are already in the table: adding them again would double them.
  assert.equal(retryInput({ ...failed, error: REGISTRATION_FAILED_REASON })?.mode, "replace")
  // A load nobody recorded the settings of is not guessed at.
  assert.equal(retryInput(upload({ status: "failed" })), null)
  assert.equal(retryInput(upload({ status: "failed", parseOptions: options })), null)
})

test("fileKindProblem refuses a workbook the API does not read, or an archive, by its name, in the API's words", () => {
  for (const name of ["a.xlsm", "a.xlsb", "a.ods", "a.zip", "a.gz", "a.7z", "report.final.xlsm"]) {
    const problem = fileKindProblem(name)
    assert.ok(problem?.startsWith(`${name} looks like a workbook or a zip archive that is not an .xls or .xlsx file.`), name)
    assert.ok(problem?.includes("Only .xls and .xlsx workbooks, .parquet files and delimited text files (CSV, TSV) can be uploaded; save the sheet as .xlsx or CSV first."), name)
  }
})

test("fileKindProblem lets an .xls or .xlsx through: the API opens it, and refuses one that does not open", () => {
  for (const name of ["a.xls", "a.xlsx", "DATA.XLS", "Data.XlsX", "report.final.xlsx", "a.csv.xlsx"]) {
    assert.equal(fileKindProblem(name), null, name)
  }
})

test("fileKindProblem lets a .parquet through: the API opens it, and refuses one that does not open", () => {
  for (const name of ["a.parquet", "DATA.PARQUET", "report.final.parquet", "a.csv.parquet"]) {
    assert.equal(fileKindProblem(name), null, name)
  }
})

test("fileKindProblem refuses other binaries in the API's sentence, which now names Parquet too", () => {
  for (const name of ["a.pdf", "a.docx", "a.avro", "a.orc", "a.sqlite"]) {
    assert.equal(
      fileKindProblem(name),
      `${name} is not a delimited text file, an Excel workbook or a Parquet file. Only .xls and .xlsx workbooks, .parquet files and delimited text files (CSV, TSV) can be uploaded.`
    )
  }
})

test("fileKindProblem ignores the case of the extension", () => {
  assert.ok(fileKindProblem("DATA.XLSM")?.includes("not an .xls or .xlsx file"))
  assert.ok(fileKindProblem("Data.OdS")?.includes("not an .xls or .xlsx file"))
  assert.equal(fileKindProblem("DATA.PARQUET"), null)
})

test("fileKindProblem lets through delimited text, unknown extensions and names with no extension", () => {
  for (const name of ["a.csv", "A.CSV", "a.tsv", "a.txt", "a.dat", "export", "xls", "a.xls.csv", ".csv", "a."]) {
    assert.equal(fileKindProblem(name), null, name)
  }
})

test("fileProblem gives the kind first, then the size, and null for an acceptable file", () => {
  assert.ok(fileProblem("a.xlsm", MAX_UPLOAD_BYTES + 1)?.includes("not an .xls or .xlsx file"))
  assert.ok(fileProblem("a.csv", MAX_UPLOAD_BYTES + 1)?.includes("50 MB limit"))
  assert.ok(fileProblem("a.xlsx", MAX_UPLOAD_BYTES + 1)?.includes("50 MB limit"))
  assert.equal(fileProblem("a.csv", 1024), null)
  assert.equal(fileProblem("a.xlsx", 1024), null)
})

test("rawTableFullName puts the table in the bronze namespace, as Data Explorer names it", () => {
  assert.equal(rawTableFullName("stock"), "bronze.stock")
})

test("uploadFileKind tells a workbook from delimited text by the name alone, whatever the case", () => {
  assert.equal(uploadFileKind("a.xlsx"), "workbook")
  assert.equal(uploadFileKind("A.XLS"), "workbook")
  assert.equal(uploadFileKind("a.csv"), "delimited")
  assert.equal(uploadFileKind("a.TSV"), "delimited")
  assert.equal(uploadFileKind("a.parquet"), "parquet")
  assert.equal(uploadFileKind("A.PARQUET"), "parquet")
  for (const name of ["a.txt", "a.dat", "export", ".csv", "a.", "a.xlsm"]) assert.equal(uploadFileKind(name), "other", name)
})

test("uploadFileLabel is the extension in capitals, or null when there is none or it is too long", () => {
  assert.equal(uploadFileLabel("a.xls"), "XLS")
  assert.equal(uploadFileLabel("a.XLSX"), "XLSX")
  assert.equal(uploadFileLabel("a.csv"), "CSV")
  assert.equal(uploadFileLabel("a.Tsv"), "TSV")
  assert.equal(uploadFileLabel("a.parquet"), "PARQUET")
  assert.equal(uploadFileLabel("a.txt"), "TXT")
  assert.equal(uploadFileLabel("export"), null)
  assert.equal(uploadFileLabel(".csv"), null)
  assert.equal(uploadFileLabel("a."), null)
  assert.equal(uploadFileLabel("a.verylongext"), null)
  assert.equal(uploadFileLabel("a.b c"), null)
})

test("tableNameFault names the specific break and is null exactly when tableNameProblem is", () => {
  const cases: [string, string][] = [
    ["Orders", "Upper-case"],
    ["orders 2025", "Spaces"],
    ["2025_orders", "start with a digit"],
    ["_orders", "start with an underscore"],
    ["orders_", "end with an underscore"],
    ["orders__raw", "two in a row"],
    ["orders-raw", '"-" is not allowed'],
    ["a".repeat(MAX_TABLE_NAME_CHARS + 1), "129 characters"],
    ["orders\n", "Spaces"],
  ]
  for (const [name, part] of cases) {
    assert.ok(tableNameFault(name)?.includes(part), `${JSON.stringify(name)} -> ${tableNameFault(name)}`)
  }
  for (const name of ["stock", "stock_2025", "a", "a".repeat(MAX_TABLE_NAME_CHARS), "x_1_y"]) {
    assert.equal(tableNameProblem(name), null)
    assert.equal(tableNameFault(name), null)
  }
  for (const name of ["", "A", "_", "a_", "é", "a b", "a\n"]) {
    assert.notEqual(tableNameProblem(name), null, name)
    assert.notEqual(tableNameFault(name), null, name)
  }
})
