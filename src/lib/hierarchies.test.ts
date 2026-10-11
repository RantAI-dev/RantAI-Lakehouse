import assert from "node:assert/strict"
import test from "node:test"

import { canDrillFurther, climbTo, drillRequest, hierarchyProblems, levelAfter, nextLevel, trailOf } from "./hierarchies"
import type { Hierarchy } from "@/services/contracts/hierarchies"

const geo: Hierarchy = { id: "h_1", sourceKind: "mart", sourceId: "m", name: "Geo", levels: ["provinsi", "kab_kota", "place"], createdBy: "" }
const other: Hierarchy = { ...geo, id: "h_2", name: "Short", levels: ["kab_kota", "category"] }

test("a hierarchy needs a name and two to six distinct columns", () => {
  assert.deepEqual(hierarchyProblems("", ["", ""]), ["a name", "2 to 6 columns"])
  assert.deepEqual(hierarchyProblems("Geo", ["a", ""]), ["2 to 6 columns"])
  assert.deepEqual(hierarchyProblems("Geo", ["a", "a"]), ["each column once"])
  assert.deepEqual(hierarchyProblems("Geo", ["a", "b"]), [])
})

test("the menu offers the next level, and says why when there is none", () => {
  assert.deepEqual(nextLevel([geo], "provinsi"), { hierarchy: geo, from: "provinsi", to: "kab_kota" })
  assert.deepEqual(nextLevel([geo], "place"), { reason: "this is the last level of its hierarchy" })
  assert.deepEqual(nextLevel([geo], "category"), { reason: "this column is not a level of a hierarchy" })
  // A column in two hierarchies takes the first one that has a level below it.
  assert.deepEqual(nextLevel([geo, other], "kab_kota"), { hierarchy: geo, from: "kab_kota", to: "place" })
  assert.deepEqual(nextLevel([{ ...geo, levels: ["a", "kab_kota"] }, other], "kab_kota"), { hierarchy: other, from: "kab_kota", to: "category" })
})

test("the trail climbs back to any entry and the chart goes down level by level", () => {
  const steps = [{ value: "Jawa Barat" }, { value: "k1", label: "Kota Bandung" }]
  assert.deepEqual(trailOf(steps), ["All", "Jawa Barat", "Kota Bandung"])
  assert.deepEqual(climbTo(steps, 0), [])
  assert.deepEqual(climbTo(steps, 1), [steps[0]])
  assert.deepEqual(climbTo(steps, 9), steps)
  assert.equal(levelAfter(geo, "provinsi", 0), "provinsi")
  assert.equal(levelAfter(geo, "provinsi", 2), "place")
  assert.equal(levelAfter(geo, "provinsi", 3), null)
  assert.equal(canDrillFurther(geo, "provinsi", 0), true)
  assert.equal(canDrillFurther(geo, "provinsi", 1), true)
  assert.equal(canDrillFurther(geo, "provinsi", 2), false)
})

test("a drill request names the saved chart, the level and every value clicked so far, each with its column", () => {
  assert.deepEqual(drillRequest("c_1", geo, "provinsi", [{ value: "Jawa Barat" }], [{ column: "category" }]), {
    drillOf: "c_1", to: "kab_kota", fix: [{ column: "provinsi", value: "Jawa Barat" }], filters: [{ column: "category" }],
  })
  const two = drillRequest("c_1", geo, "provinsi", [{ value: "Jawa Barat" }, { value: "Kota Bandung" }], [])
  assert.deepEqual(two?.fix, [{ column: "provinsi", value: "Jawa Barat" }, { column: "kab_kota", value: "Kota Bandung" }])
  assert.equal(two?.to, "place")
  // Nothing to ask before the first click, or past the last level.
  assert.equal(drillRequest("c_1", geo, "provinsi", [], []), null)
  assert.equal(drillRequest("c_1", geo, "provinsi", [{ value: "a" }, { value: "b" }, { value: "c" }], []), null)
  assert.equal(drillRequest("c_1", geo, "category", [{ value: "a" }], []), null)
})

test("a chart that cannot be drilled says why in the menu's words", async () => {
  const { drillBlockReason, fixedFilters } = await import("./hierarchies")
  assert.equal(drillBlockReason({ kind: "bar", dimension: "provinsi" }), null)
  assert.match(String(drillBlockReason({ kind: "kpi", dimension: "x" })), /kind of chart/)
  assert.match(String(drillBlockReason({ kind: "bar", dimension: "x", grain: "month" })), /date/)
  assert.match(String(drillBlockReason({ kind: "bar", dimension: "x", bin: { mode: "auto" } })), /bins/)
  assert.match(String(drillBlockReason({ kind: "bar", dimension: "x", steps: [{}] })), /steps/)
  assert.match(String(drillBlockReason({ kind: "bar", dimension: "x", other: true })), /Other/)
  assert.match(String(drillBlockReason({ kind: "bar", dimension: "" })), /no dimension/)
  assert.deepEqual(fixedFilters(geo, "provinsi", [{ value: "Jawa Barat" }, { value: "Kota Bandung" }]), [
    { column: "provinsi", values: ["Jawa Barat"] },
    { column: "kab_kota", values: ["Kota Bandung"] },
  ])
})
