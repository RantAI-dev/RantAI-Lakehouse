import assert from "node:assert/strict"
import test from "node:test"

import {
  AUTO_BIN, binChoiceOf, binChoicesFor, binDraftFromChoice, binDraftFromDef, binLabel, binPayload, binProblem,
  binRangeFilter, binsOfTile, gridChoiceOf, gridPayload, isLastBin, relabelBins, toggleBinFilter,
} from "./chart-bins"

test("a bin is sent as its mode, and a width only when it is a positive number", () => {
  assert.deepEqual(binPayload(null), undefined)
  assert.deepEqual(binPayload(AUTO_BIN), { mode: "auto" })
  assert.deepEqual(binPayload({ mode: "count", count: "50", width: "" }), { mode: "count", count: 50 })
  assert.deepEqual(binPayload({ mode: "width", count: "10", width: " 500 " }), { mode: "width", width: 500 })
  for (const bad of ["", "0", "-5", "abc"]) {
    assert.equal(binPayload({ mode: "width", count: "10", width: bad }), undefined, bad)
    assert.equal(binProblem({ mode: "width", count: "10", width: bad }, "visitors"), "visitors bin width")
  }
  assert.equal(binProblem(AUTO_BIN, "visitors"), null)
})

test("a saved bin comes back as a draft and the picker values round trip", () => {
  assert.deepEqual(binDraftFromDef({ mode: "width", width: 500 }), { mode: "width", count: "10", width: "500" })
  assert.deepEqual(binDraftFromDef({ mode: "count", count: 100 }), { mode: "count", count: "100", width: "" })
  assert.equal(binDraftFromDef(undefined), null)
  assert.equal(binDraftFromDef({ mode: "nope" as "auto" }), null)
  for (const choice of ["none", "auto", "count:10", "count:50", "count:100", "width"]) {
    assert.equal(binChoiceOf(binDraftFromChoice(choice, null)), choice)
  }
  assert.equal(binDraftFromChoice("width", { mode: "width", count: "10", width: "250" })?.width, "250")
  assert.deepEqual(binChoicesFor(true), ["none", "width"])
  assert.equal(binChoicesFor(false).length, 6)
})

test("a bin reads 500 – 1,000 and keeps the decimals its edges need", () => {
  assert.equal(binLabel(500, 500), "500 – 1,000")
  assert.equal(binLabel(0, 50), "0 – 50")
  assert.equal(binLabel(0.35, 0.05), "0.35 – 0.4")
  assert.equal(binLabel(-1500, 1000), "-1,500 – -500")
  assert.equal(binLabel(1000, 1000), "1,000 – 2,000")
})

test("rows are relabelled column by column and a value that is not a number is left alone", () => {
  const rows = [{ v: 0, n: 3 }, { v: 500, n: 4 }, { v: null, n: 1 }, { v: "x", n: 2 }]
  const out = relabelBins(rows, { v: { width: 500 } })
  assert.deepEqual(out.map((r) => r.v), ["0 – 500", "500 – 1,000", null, "x"])
  assert.deepEqual(out.map((r) => r.n), [3, 4, 1, 2])
  assert.deepEqual(relabelBins(rows, {}), rows)
})

test("the tile's bins come from what the server read, else from a width the chart fixes", () => {
  const meta = { visitors: { mode: "auto", width: 500, min: 44, max: 2377 } }
  assert.deepEqual(binsOfTile(meta, undefined), { visitors: { width: 500, max: 2377 } })
  assert.deepEqual(binsOfTile({ visitors: { width: "250" } }, undefined), { visitors: { width: 250 } })
  const fixed = binsOfTile(undefined, { dimension: "visitors", bin: { mode: "width", width: 100 }, groupings: [{ column: "g", bin: { mode: "auto" } }] })
  assert.deepEqual(fixed, { visitors: { width: 100 } })
  assert.deepEqual(binsOfTile({ visitors: { width: 0 } }, { dimension: "visitors", bin: { mode: "auto" } }), {})
})

test("a clicked bin filters with the lower edge in and the upper out, except the last bin", () => {
  assert.deepEqual(binRangeFilter("v", 500, 500, false), { column: "v", values: [], op: "between", min: "500", max: "1000", maxExclusive: true })
  assert.deepEqual(binRangeFilter("v", 2000, 500, true), { column: "v", values: [], op: "between", min: "2000", max: "2500" })
  assert.equal(isLastBin(2000, 500, 2377), true)
  assert.equal(isLastBin(1500, 500, 2377), false)
  // The maximum sits on the upper edge: the bin below it is the last one.
  assert.equal(isLastBin(2000, 500, 2500), true)
  assert.equal(isLastBin(2500, 500, 2500), true)
  assert.equal(isLastBin(0, 500, undefined), false)
})

test("a second click on the same bin removes its filter, another bin replaces it", () => {
  const a = binRangeFilter("v", 0, 500, false)
  const b = binRangeFilter("v", 500, 500, false)
  assert.deepEqual(toggleBinFilter([], a), [a])
  assert.deepEqual(toggleBinFilter([a], a), [])
  assert.deepEqual(toggleBinFilter([a], b), [b])
  const other = { column: "c", values: ["x"] }
  assert.deepEqual(toggleBinFilter([other, a], b), [other, b])
})

test("a grid is one of the listed sizes or automatic", () => {
  assert.equal(gridChoiceOf(undefined), "none")
  assert.equal(gridChoiceOf({ size: "auto" }), "auto")
  assert.equal(gridChoiceOf({ size: 0.1 }), "0.1")
  assert.equal(gridChoiceOf({ size: 0.3 }), "none")
  assert.deepEqual(gridPayload("auto"), { size: "auto" })
  assert.deepEqual(gridPayload("0.01"), { size: 0.01 })
  assert.equal(gridPayload("none"), undefined)
  assert.equal(gridPayload("7"), undefined)
})
