import { strict as assert } from "node:assert"
import { test } from "node:test"
import { filterKindGroups, type KindGroup } from "./chart-kind-search"

type K = "hbar" | "sankey" | "boxplot" | "pie" | "pointmap"
const GROUPS: KindGroup<K>[] = [
  { group: "Comparison", items: [{ value: "hbar", label: "Horizontal bar" }] },
  { group: "Composition", items: [{ value: "pie", label: "Donut / pie" }] },
  { group: "Relationship / distribution", items: [{ value: "boxplot", label: "Box plot (distribution)" }] },
  { group: "Flow", items: [{ value: "sankey", label: "Sankey (flow between 2 dimensions)" }] },
  { group: "Geographic", items: [{ value: "pointmap", label: "Map — points / bubbles" }] },
]
const DESC: Partial<Record<K, string>> = { pie: "Share of a whole", hbar: "Rank categories" }
const kinds = (q: string) => filterKindGroups(GROUPS, DESC, q).flatMap((g) => g.items.map((i) => i.value))

test("an empty or blank query returns every group unchanged", () => {
  assert.equal(filterKindGroups(GROUPS, DESC, ""), GROUPS)
  assert.equal(filterKindGroups(GROUPS, DESC, "   "), GROUPS)
})

test("labels match case-insensitively and groups left empty are dropped", () => {
  assert.deepEqual(filterKindGroups(GROUPS, DESC, "SANKEY"), [
    { group: "Flow", items: [{ value: "sankey", label: "Sankey (flow between 2 dimensions)" }] },
  ])
})

test("description, group name and kind id also match", () => {
  assert.deepEqual(kinds("share"), ["pie"])
  assert.deepEqual(kinds("composition"), ["pie"])
  assert.deepEqual(kinds("boxplot"), ["boxplot"])
})

test("every word of the query has to match", () => {
  assert.deepEqual(kinds("bar rank"), ["hbar"])
  assert.deepEqual(kinds("bar flow"), [])
})

test("extra keywords match too, so a search in Indonesian finds the map", () => {
  const keywords: Partial<Record<K, string>> = { pointmap: "map peta points titik lokasi bubble" }
  const found = (q: string) => filterKindGroups(GROUPS, DESC, q, keywords).flatMap((g) => g.items.map((i) => i.value))
  assert.deepEqual(found("peta"), ["pointmap"])
  assert.deepEqual(found("titik lokasi"), ["pointmap"])
  assert.deepEqual(found("lokasi bar"), [])
  // Without the keywords the same word finds nothing.
  assert.deepEqual(kinds("lokasi"), [])
})
