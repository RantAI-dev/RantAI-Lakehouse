import { strict as assert } from "node:assert"
import { test } from "node:test"
import { boxplotNeedsLogAxis, toBoxplot, toCalendar, toSankey, toSunburst } from "./chart-transforms"

test("toSankey keeps a value that appears on both sides as two nodes and sums repeated links", () => {
  const rows = [
    { region: "Other", channel: "Web", v: 3 },
    { region: "North", channel: "Other", v: 2 },
    { region: "North", channel: "Other", v: 5 },
    { region: "South", channel: "Web", v: 0 },
  ]
  const { nodes, links } = toSankey(rows, "region", "channel", "v")
  assert.deepEqual(nodes.map((n) => n.name).sort(), ["a:North", "a:Other", "b:Other", "b:Web"])
  assert.deepEqual(links.find((l) => l.source === "a:North"), { source: "a:North", target: "b:Other", value: 7 })
  assert.equal(links.length, 2, "a zero-width link is dropped")
})

test("toSunburst nests the breakdown under each category", () => {
  const tree = toSunburst(
    [
      { k: "Tipe", n: "1010", v: 10 },
      { k: "Grup", n: "9990", v: 8 },
      { k: "Grup", n: "1513", v: 2 },
    ],
    "k",
    "n",
    "v"
  )
  assert.deepEqual(tree, [
    { name: "Tipe", children: [{ name: "1010", value: 10 }] },
    { name: "Grup", children: [{ name: "9990", value: 8 }, { name: "1513", value: 2 }] },
  ])
})

test("toBoxplot takes five-number arrays and drops anything else", () => {
  const out = toBoxplot(
    [
      { g: "A", m: [1, 9, 17, 264, 34421] },
      { g: "B", m: [1, 2] },
      { g: "C", m: "oops" },
    ],
    "g",
    "m"
  )
  assert.deepEqual(out, { categories: ["A"], data: [[1, 9, 17, 264, 34421]], counts: [null] })
})

test("toBoxplot reads the row count, which ClickHouse sends as a string", () => {
  const out = toBoxplot(
    [
      { g: "Grup", m: [1, 9, 17, 264, 34421], __n: "16" },
      { g: "Tipe", m: [35808, 35808, 35808, 35808, 35808], __n: 1 },
    ],
    "g",
    "m"
  )
  assert.deepEqual(out.counts, [16, 1])
})

test("a log axis only for all-positive values spanning three orders of magnitude", () => {
  assert.equal(boxplotNeedsLogAxis([[1, 9, 17, 264, 34421], [35808, 35808, 35808, 35808, 35808]]), true)
  assert.equal(boxplotNeedsLogAxis([[10, 20, 30, 40, 900]]), false)
  assert.equal(boxplotNeedsLogAxis([[0, 9, 17, 264, 34421]]), false, "log cannot draw zero")
  assert.equal(boxplotNeedsLogAxis([[-5, 1, 2, 3, 5000]]), false)
  assert.equal(boxplotNeedsLogAxis([]), false)
})

test("toCalendar keeps dated rows and reports the range they span", () => {
  const out = toCalendar(
    [
      { d: "2021-03-05", v: 4 },
      { d: "2020-12-31 00:00:00", v: 1 },
      { d: "not a date", v: 9 },
    ],
    "d",
    "v"
  )
  assert.deepEqual(out.data, [["2021-03-05", 4], ["2020-12-31", 1]])
  assert.deepEqual(out.range, ["2020-12-31", "2021-03-05"])
  assert.equal(toCalendar([], "d", "v").range, null)
})

test("toCalendar draws at most the last year of a long series", () => {
  const out = toCalendar([{ d: "2019-01-01", v: 1 }, { d: "2021-06-30", v: 2 }], "d", "v")
  assert.deepEqual(out.range, ["2020-07-01", "2021-06-30"])
})
