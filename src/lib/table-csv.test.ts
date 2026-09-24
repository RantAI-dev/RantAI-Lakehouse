import { strict as assert } from "node:assert"
import { test } from "node:test"
import { csvCell, csvFileName, csvHeaders, tableCsv } from "./table-csv"

test("csvHeaders memakai label kolom, dan id bila labelnya berulang", () => {
  assert.deepEqual(
    csvHeaders([
      { id: "name", label: "Name" },
      { id: "tier", label: "Tier" },
    ]),
    ["Name", "Tier"]
  )
  assert.deepEqual(
    csvHeaders([
      { id: "createdAt", label: "Date" },
      { id: "updatedAt", label: "Date" },
    ]),
    ["Date", "updatedAt"]
  )
})

test("csvCell mengosongkan nilai yang tidak ada, tapi mempertahankan 0 dan false", () => {
  assert.equal(csvCell(null), "")
  assert.equal(csvCell(undefined), "")
  assert.equal(csvCell(0), "0")
  assert.equal(csvCell(false), "false")
})

test("csvCell menulis tanggal sebagai ISO dan objek sebagai JSON", () => {
  assert.equal(
    csvCell(new Date("2026-09-20T10:00:00Z")),
    "2026-09-20T10:00:00.000Z"
  )
  assert.equal(csvCell(["a", "b"]), '["a","b"]')
})

test("tableCsv hanya mengambil kolom yang diminta dan meng-escape isinya", () => {
  const csv = tableCsv(
    [
      { id: "name", label: "Name" },
      { id: "sizeBytes", label: "Size" },
    ],
    [
      { name: "Kunjungan, Wisata", sizeBytes: 1024, extra: "diabaikan" },
      { name: "Mart Atlas", sizeBytes: null },
    ]
  )
  assert.equal(csv, 'Name,Size\r\n"Kunjungan, Wisata",1024\r\nMart Atlas,')
})

test("csvFileName memakai nama tabel dan tanggal unduhan", () => {
  assert.equal(
    csvFileName("Data Explorer", new Date(2026, 8, 20)),
    "data-explorer-2026-09-20.csv"
  )
  assert.equal(csvFileName("—", new Date(2026, 8, 5)), "table-2026-09-05.csv")
})
