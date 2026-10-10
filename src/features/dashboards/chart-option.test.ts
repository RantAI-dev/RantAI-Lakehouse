import assert from "node:assert/strict"
import test from "node:test"

import { MARKER_LIMIT, buildOption, showsPointMarkers } from "./chart-option"

const rowsOf = (n: number) => Array.from({ length: n }, (_, i) => ({ x: `p${i}`, v: i }))
const lineSpec = { kind: "line" as const, x: "x", y: "v" }

function showSymbol(kind: "line" | "area", n: number): unknown {
  const option = buildOption({ ...lineSpec, kind }, rowsOf(n), false)
  const series = option.series as { showSymbol?: boolean }[]
  return series[0].showSymbol
}

test("a line draws point markers up to the threshold and none above it", () => {
  assert.equal(showSymbol("line", MARKER_LIMIT), true)
  assert.equal(showSymbol("line", MARKER_LIMIT + 1), false)
  assert.equal(showSymbol("line", 1000), false)
  assert.equal(MARKER_LIMIT, 60)
})

test("an area never drew point markers", () => {
  assert.equal(showSymbol("area", 5), false)
  assert.equal(showsPointMarkers("area", 5), false)
  assert.equal(showsPointMarkers("bar", 5), false)
})

test("a line with a breakdown counts its categories, not its series", () => {
  const rows = rowsOf(61).flatMap((r) => [{ ...r, g: "a" }, { ...r, g: "b" }])
  const option = buildOption({ ...lineSpec, series: "g" }, rows, false)
  const series = option.series as { showSymbol?: boolean }[]
  assert.equal(series.length, 2)
  assert.equal(series[0].showSymbol, false)
})
