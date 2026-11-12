import { strict as assert } from "node:assert"
import { test } from "node:test"
import {
  inCommentOrString,
  inPlaceholder,
  sqlCompletionOptions,
  tablesInStatement,
  type ColumnState,
  type CompletionTable,
} from "./sql-completion"

/** The tables the catalog shows: two named ones and one Bronze registry slug. */
const TABLES: CompletionTable[] = [
  {
    id: "silver.orders",
    label: "Orders",
    reference: "silver.orders",
    layer: "silver",
    names: ["silver.orders", "silver.Orders"],
  },
  {
    id: "serving.customers",
    label: "Customers",
    reference: "serving.customers",
    layer: "gold",
    names: ["serving.customers", "serving.Customers"],
  },
  {
    id: "demo_orders",
    label: "Demo Orders",
    reference: "",
    layer: "raw",
    names: ["demo_orders", "bronze.Demo Orders"],
  },
]

/** The columns of the two tables that have a SQL name. */
const COLUMNS: Record<string, ColumnState> = {
  "silver.orders": {
    status: "ready",
    columns: [
      { name: "amount", dataType: "Int64" },
      { name: "status", dataType: "String" },
    ],
  },
  "serving.customers": {
    status: "ready",
    columns: [
      { name: "amount", dataType: "Decimal(10, 2)" },
      { name: "email", dataType: "String" },
    ],
  },
}

/** The cursor right after the first occurrence of `needle` in `sql`. */
function after(sql: string, needle: string): number {
  return sql.indexOf(needle) + needle.length
}

test("tablesInStatement menemukan tabel yang ditulis dengan nama SQL-nya", () => {
  const named = tablesInStatement("SELECT * FROM silver.orders", TABLES)
  assert.deepEqual(named, [{ id: "silver.orders", reference: "silver.orders" }])
})

test("tablesInStatement menemukan tabel Bronze lewat bagian terakhir dari nama yang ditulis", () => {
  const named = tablesInStatement("SELECT * FROM iceberg.bronze.demo_orders", TABLES)
  assert.deepEqual(named, [{ id: "demo_orders", reference: "iceberg.bronze.demo_orders" }])
})

test("tablesInStatement tidak menemukan tabel yang tidak ada di katalog", () => {
  const named = tablesInStatement("SELECT * FROM unknown_table", TABLES)
  assert.deepEqual(named, [])
})

test("sqlCompletionOptions memberi kolom tabel yang diberi alias dengan AS", () => {
  const sql = "SELECT o. FROM silver.orders AS o"
  const { options } = sqlCompletionOptions({
    sql,
    cursor: after(sql, "o."),
    tables: TABLES,
    columns: COLUMNS,
  })
  assert.deepEqual(options.map((option) => option.label), ["amount", "status"])
  assert.deepEqual(options.map((option) => option.detail), ["Int64", "String"])
  assert.deepEqual(options.map((option) => option.kind), ["column", "column"])
})

test("sqlCompletionOptions memberi kolom tabel yang diberi alias tanpa AS", () => {
  const sql = "SELECT o. FROM silver.orders o"
  const { options } = sqlCompletionOptions({
    sql,
    cursor: after(sql, "o."),
    tables: TABLES,
    columns: COLUMNS,
  })
  assert.deepEqual(options.map((option) => option.label), ["amount", "status"])
})

test("sqlCompletionOptions memberi kolom tabel Bronze yang ditulis dengan namespace.name", () => {
  const bronze: CompletionTable = {
    id: "bronze_orders",
    label: "Orders",
    reference: "",
    layer: "raw",
    names: ["bronze_orders", "bronze.orders"],
  }
  const sql = "SELECT o. FROM bronze.orders o"
  const { options } = sqlCompletionOptions({
    sql,
    cursor: after(sql, "o."),
    tables: [bronze],
    columns: {
      bronze_orders: {
        status: "ready",
        columns: [{ name: "oid", dataType: "String" }],
      },
    },
  })
  assert.deepEqual(options.map((option) => option.label), ["oid"])
})

/**
 * A Bronze table as the studio hands it over on a Trino tab: the registry slug
 * is not the table's name in Trino, so the Trino name is one of its names.
 */
const BRONZE_ON_TRINO: CompletionTable = {
  id: "demo_orders",
  label: "Demo Orders",
  reference: "iceberg.bronze.orders",
  layer: "raw",
  names: ["demo_orders", "bronze.Demo Orders", "iceberg.bronze.orders", "bronze.orders"],
  trinoOnly: true,
}
const BRONZE_COLUMNS: Record<string, ColumnState> = {
  demo_orders: { status: "ready", columns: [{ name: "oid", dataType: "String" }] },
}

test("sqlCompletionOptions offers the columns after an alias on a Bronze table's Trino name, whatever its id", () => {
  const sql = "SELECT b. FROM iceberg.bronze.orders b"
  const { options } = sqlCompletionOptions({
    sql,
    cursor: after(sql, "b."),
    tables: [BRONZE_ON_TRINO],
    columns: BRONZE_COLUMNS,
  })
  assert.deepEqual(options.map((option) => option.label), ["oid"])
})

test("sqlCompletionOptions finds a Bronze table by its Trino name even when it has no reference of its own", () => {
  const sql = "SELECT b. FROM iceberg.bronze.orders b"
  const { options } = sqlCompletionOptions({
    sql,
    cursor: after(sql, "b."),
    tables: [{ ...BRONZE_ON_TRINO, reference: "" }],
    columns: BRONZE_COLUMNS,
  })
  assert.deepEqual(options.map((option) => option.label), ["oid"])
})

test("sqlCompletionOptions offers the columns after an alias on bronze.<table>", () => {
  const sql = "SELECT b. FROM bronze.orders AS b"
  const { options } = sqlCompletionOptions({
    sql,
    cursor: after(sql, "b."),
    tables: [BRONZE_ON_TRINO],
    columns: BRONZE_COLUMNS,
  })
  assert.deepEqual(options.map((option) => option.label), ["oid"])
})

test("sqlCompletionOptions offers a Bronze table by its Trino name after FROM on a Trino tab", () => {
  const sql = "SELECT * FROM "
  const { options } = sqlCompletionOptions({
    sql,
    cursor: sql.length,
    tables: [BRONZE_ON_TRINO],
    columns: {},
  })
  assert.deepEqual(options, [
    {
      label: "iceberg.bronze.orders",
      kind: "table",
      detail: "Demo Orders",
      apply: "iceberg.bronze.orders",
    },
  ])
})

test("sqlCompletionOptions offers no Bronze table after FROM on a ClickHouse tab", () => {
  const sql = "SELECT * FROM "
  const { options } = sqlCompletionOptions({
    sql,
    cursor: sql.length,
    tables: [{ ...BRONZE_ON_TRINO, reference: "" }, TABLES[0] as CompletionTable],
    columns: {},
  })
  assert.deepEqual(options.map((option) => option.label), ["silver.orders"])
})

test("sqlCompletionOptions tidak mencampur kolom dua tabel yang nama kolomnya sama", () => {
  const sql = "SELECT a., b. FROM silver.orders a JOIN serving.customers b"
  const first = sqlCompletionOptions({
    sql,
    cursor: after(sql, "a."),
    tables: TABLES,
    columns: COLUMNS,
  })
  const second = sqlCompletionOptions({
    sql,
    cursor: after(sql, "b."),
    tables: TABLES,
    columns: COLUMNS,
  })
  assert.deepEqual(first.options.map((option) => option.label), ["amount", "status"])
  assert.deepEqual(second.options.map((option) => option.label), ["amount", "email"])
})

test("sqlCompletionOptions memberi satu pesan saat kolom tabel masih dimuat", () => {
  const sql = "SELECT o. FROM silver.orders o"
  const { options } = sqlCompletionOptions({
    sql,
    cursor: after(sql, "o."),
    tables: TABLES,
    columns: { "silver.orders": { status: "loading" } },
  })
  assert.equal(options.length, 1)
  assert.equal(options[0]?.label, "Loading columns…")
  assert.equal(options[0]?.kind, "message")
  assert.equal(options[0]?.apply, undefined)
})

test("sqlCompletionOptions memberi satu pesan saat kolom tabel gagal dimuat", () => {
  const sql = "SELECT o. FROM silver.orders o"
  const { options } = sqlCompletionOptions({
    sql,
    cursor: after(sql, "o."),
    tables: TABLES,
    columns: { "silver.orders": { status: "failed" } },
  })
  assert.equal(options.length, 1)
  assert.equal(options[0]?.label, "Columns could not be loaded")
  assert.equal(options[0]?.kind, "message")
  assert.equal(options[0]?.apply, undefined)
})

test("sqlCompletionOptions menyebut kegagalan dengan kalimat yang sama walau ada alasan penolakan", () => {
  const sql = "SELECT o. FROM silver.orders o"
  const { options } = sqlCompletionOptions({
    sql,
    cursor: after(sql, "o."),
    tables: TABLES,
    columns: { "silver.orders": { status: "failed", reason: "Tenant belum dikonfigurasi" } },
  })
  // The reason belongs to the schema browser; the editor keeps its one line.
  assert.equal(options.length, 1)
  assert.equal(options[0]?.label, "Columns could not be loaded")
})

test("sqlCompletionOptions tidak peduli pada layer tabel", () => {
  const sql = "SELECT * FROM "
  const { options } = sqlCompletionOptions({
    sql,
    cursor: sql.length,
    tables: TABLES,
    columns: {},
  })
  // Silver, gold and raw tables alike: only those with a SQL name are offered.
  assert.deepEqual(
    options.map((option) => option.label),
    ["silver.orders", "serving.customers"]
  )
})

test("sqlCompletionOptions tidak memberi kolom untuk tabel yang tidak ada di katalog", () => {
  const sql = "SELECT u. FROM unknown_table u"
  const { options } = sqlCompletionOptions({
    sql,
    cursor: after(sql, "u."),
    tables: TABLES,
    columns: COLUMNS,
  })
  assert.deepEqual(options.filter((option) => option.kind === "column"), [])
})

test("sqlCompletionOptions memberi kolom tabel yang disebut, lalu tabelnya", () => {
  const sql = "SELECT amount FROM silver.orders WHERE "
  const { options } = sqlCompletionOptions({
    sql,
    cursor: sql.length,
    tables: TABLES,
    columns: COLUMNS,
  })
  assert.deepEqual(options.map((option) => option.label), [
    "amount",
    "status",
    "silver.orders",
    "serving.customers",
  ])
})

test("sqlCompletionOptions memberi tabel setelah FROM", () => {
  const sql = "SELECT * FROM "
  const { options } = sqlCompletionOptions({
    sql,
    cursor: sql.length,
    tables: TABLES,
    columns: COLUMNS,
  })
  assert.deepEqual(options.map((option) => option.label), [
    "silver.orders",
    "serving.customers",
  ])
  assert.deepEqual(options.map((option) => option.detail), ["Orders", "Customers"])
})

test("sqlCompletionOptions menghitung dari awal kata yang sedang diselesaikan", () => {
  const sql = "SELECT o.am FROM silver.orders o"
  const { from } = sqlCompletionOptions({
    sql,
    cursor: after(sql, "am"),
    tables: TABLES,
    columns: COLUMNS,
  })
  assert.equal(from, sql.indexOf("am"))
})

test("sqlCompletionOptions offers the defined parameter names after {{ and nothing else", () => {
  const sql = "SELECT amount FROM silver.orders WHERE status = {{"
  const { from, options } = sqlCompletionOptions({
    sql,
    cursor: sql.length,
    tables: TABLES,
    columns: COLUMNS,
    parameters: ["region", "day"],
  })
  assert.equal(from, sql.length)
  assert.deepEqual(
    options.map((option) => [option.label, option.kind, option.apply]),
    [
      ["region", "parameter", "region"],
      ["day", "parameter", "day"],
    ]
  )
})

test("sqlCompletionOptions completes a parameter name that is half typed, past blanks inside the braces", () => {
  const sql = "SELECT amount FROM silver.orders WHERE status = {{  re"
  const { from, options } = sqlCompletionOptions({
    sql,
    cursor: sql.length,
    tables: TABLES,
    columns: COLUMNS,
    parameters: ["region"],
  })
  assert.equal(from, sql.indexOf("re"))
  assert.deepEqual(options.map((option) => option.label), ["region"])
})

test("sqlCompletionOptions offers no parameter name outside the braces", () => {
  const sql = "SELECT amount FROM silver.orders WHERE "
  const { options } = sqlCompletionOptions({
    sql,
    cursor: sql.length,
    tables: TABLES,
    columns: COLUMNS,
    parameters: ["region"],
  })
  assert.equal(options.some((option) => option.kind === "parameter"), false)
})

test("sqlCompletionOptions offers nothing after {{ when the tab defines no parameter", () => {
  const sql = "SELECT amount FROM silver.orders WHERE status = {{"
  const { options } = sqlCompletionOptions({
    sql,
    cursor: sql.length,
    tables: TABLES,
    columns: COLUMNS,
  })
  assert.deepEqual(options, [])
})

test("sqlCompletionOptions offers the defined parameter names after a {{ that is half typed", () => {
  const sql = "SELECT amount FROM silver.orders WHERE status = {{reg"
  const { from, options } = sqlCompletionOptions({
    sql,
    cursor: sql.length,
    tables: TABLES,
    columns: COLUMNS,
    parameters: ["region", "day"],
  })
  // The name is half typed, so the list starts where the name starts; the
  // list itself holds every defined name, and the editor narrows it down.
  assert.equal(from, sql.indexOf("reg"))
  assert.deepEqual(options.map((option) => option.label), ["region", "day"])
})

test("sqlCompletionOptions offers no parameter name after a {{ inside a line comment", () => {
  const sql = "-- a note with a {{"
  const { options } = sqlCompletionOptions({
    sql,
    cursor: sql.length,
    tables: TABLES,
    columns: COLUMNS,
    parameters: ["region", "day"],
  })
  // The braces are text in a comment, so no name belongs there.
  assert.deepEqual(options.filter((option) => option.kind === "parameter"), [])
})

test("sqlCompletionOptions offers no parameter name after a {{ inside a block comment", () => {
  const sql = "/* a note with a {{ */"
  const { options } = sqlCompletionOptions({
    sql,
    cursor: sql.indexOf("{{") + 2,
    tables: TABLES,
    columns: COLUMNS,
    parameters: ["region", "day"],
  })
  assert.deepEqual(options.filter((option) => option.kind === "parameter"), [])
})

test("sqlCompletionOptions offers no parameter name after a {{ inside a quoted run", () => {
  const sql = "SELECT 'a {{ b'"
  const { options } = sqlCompletionOptions({
    sql,
    cursor: sql.indexOf("{{") + 2,
    tables: TABLES,
    columns: COLUMNS,
    parameters: ["region", "day"],
  })
  assert.deepEqual(options.filter((option) => option.kind === "parameter"), [])
})

test("inPlaceholder reads the braces before the name, and inCommentOrString the text around the cursor", () => {
  // The braces open a placeholder, with or without a half typed name.
  assert.equal(inPlaceholder("SELECT {{", 9), true)
  assert.equal(inPlaceholder("SELECT {{a", 10), true)
  assert.equal(inPlaceholder("SELECT a", 8), false)
  // A comment and a quoted run hold text, not code.
  assert.equal(inCommentOrString("-- a {{", 6), true)
  assert.equal(inCommentOrString("/* a {{ */", 6), true)
  assert.equal(inCommentOrString("SELECT 'a {{'", 12), true)
  // The cursor right after a closed quoted run writes code again.
  assert.equal(inCommentOrString("SELECT 'abc'", 12), false)
  assert.equal(inCommentOrString("SELECT 1", 8), false)
})

// Plan 73 D5 (QS-4-AC8): after `q.`, an alias the statement declares for a
// table the list does not hold has no columns to offer, and the table list
// is no answer to it either.
test("sqlCompletionOptions offers nothing after an alias declared for a table that is not in the list", () => {
  const sql = "SELECT q. FROM x q"
  const { options } = sqlCompletionOptions({
    sql,
    cursor: after(sql, "q."),
    tables: TABLES,
    columns: COLUMNS,
  })
  assert.deepEqual(options, [])
})

test("sqlCompletionOptions offers nothing after an AS alias of an unknown table in a JOIN", () => {
  const sql = "SELECT q. FROM silver.orders o JOIN nowhere.gone AS q ON q.id = o.id"
  const { options } = sqlCompletionOptions({
    sql,
    cursor: after(sql, "q."),
    tables: TABLES,
    columns: COLUMNS,
  })
  assert.deepEqual(options, [])
})

test("sqlCompletionOptions still offers a listed table's columns after its alias, next to an unknown table's alias", () => {
  const sql = "SELECT o. FROM silver.orders o JOIN nowhere.gone AS q ON q.id = o.id"
  const { options } = sqlCompletionOptions({
    sql,
    cursor: after(sql, "o."),
    tables: TABLES,
    columns: COLUMNS,
  })
  assert.deepEqual(options.map((option) => option.label), ["amount", "status"])
})

test("sqlCompletionOptions still offers the tables after a schema name, whatever aliases the statement declares", () => {
  const sql = "SELECT * FROM x q JOIN serving."
  const { options } = sqlCompletionOptions({
    sql,
    cursor: sql.length,
    tables: TABLES,
    columns: COLUMNS,
  })
  assert.ok(options.some((option) => option.label === "serving.customers"))
})

test("sqlCompletionOptions keeps the general answer for a qualifier that is neither an alias nor a table", () => {
  const sql = "SELECT zz. FROM silver.orders"
  const { options } = sqlCompletionOptions({
    sql,
    cursor: after(sql, "zz."),
    tables: TABLES,
    columns: COLUMNS,
  })
  assert.ok(options.some((option) => option.label === "amount"))
})

test("sqlCompletionOptions answers with the listed table when an alias name is declared for a listed and an unknown table", () => {
  const sql = "SELECT q. FROM silver.orders q JOIN nowhere AS q ON 1 = 1"
  const { options } = sqlCompletionOptions({
    sql,
    cursor: after(sql, "q."),
    tables: TABLES,
    columns: COLUMNS,
  })
  assert.deepEqual(options.map((option) => option.label), ["amount", "status"])
})
