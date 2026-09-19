import { strict as assert } from "node:assert"
import { test } from "node:test"
import { filterDataClientSide } from "./data-table"

type Row = { id: string; at: string; active: boolean }

const rows: Row[] = [
  { id: "a", at: "2026-06-11T23:30:00", active: true },
  { id: "b", at: "2026-06-12T08:00:00", active: false },
  // Format ClickHouse (spasi, tanpa zona) dibaca sebagai UTC; tengah hari
  // UTC tetap jatuh di tanggal 12 untuk zona waktu UTC-11 s.d. UTC+11.
  { id: "c", at: "2026-06-12 12:00:00", active: true },
  { id: "d", at: "2026-06-13T00:05:00", active: true },
]

/** Nilai filter tanggal disimpan date picker sebagai epoch milidetik. */
function day(y: number, m: number, d: number): string {
  return String(new Date(y, m - 1, d).getTime())
}

function ids(filters: unknown[]): string[] {
  return filterDataClientSide(rows, {
    filters: JSON.stringify(filters),
  }).map((r) => r.id)
}

test("filter tanggal 'is' mencocokkan seluruh hari itu", () => {
  const f = { id: "at", variant: "date", operator: "eq", value: day(2026, 6, 12), filterId: "x" }
  assert.deepEqual(ids([f]), ["b", "c"])
})

test("filter tanggal 'is before' dan 'is on or after' membandingkan per hari", () => {
  const before = { id: "at", variant: "date", operator: "lt", value: day(2026, 6, 12), filterId: "x" }
  const onOrAfter = { id: "at", variant: "date", operator: "gte", value: day(2026, 6, 12), filterId: "y" }
  assert.deepEqual(ids([before]), ["a"])
  assert.deepEqual(ids([onOrAfter]), ["b", "c", "d"])
})

test("filter tanggal 'is between' inklusif di kedua ujung", () => {
  const f = {
    id: "at",
    variant: "dateRange",
    operator: "isBetween",
    value: [day(2026, 6, 11), day(2026, 6, 12)],
    filterId: "x",
  }
  assert.deepEqual(ids([f]), ["a", "b", "c"])
})

test("filter boolean membaca nilai string dari URL", () => {
  const f = { id: "active", variant: "boolean", operator: "eq", value: "false", filterId: "x" }
  assert.deepEqual(ids([f]), ["b"])
})
