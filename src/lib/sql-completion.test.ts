import { strict as assert } from "node:assert"
import { test } from "node:test"
import {
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
    names: ["silver.orders", "silver.Orders"],
  },
  {
    id: "serving.customers",
    label: "Customers",
    reference: "serving.customers",
    names: ["serving.customers", "serving.Customers"],
  },
  {
    id: "demo_orders",
    label: "Demo Orders",
    reference: "",
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
