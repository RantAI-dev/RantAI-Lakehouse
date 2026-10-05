import { expect, test } from "bun:test"
import { chartDefFromArgs } from "./chart-draft-card"

test("a create_chart call on a SQL source keeps its sqlSource for the preview and the builder", () => {
  const def = chartDefFromArgs({
    title: "Material per kelompok (pie)",
    kind: "pie",
    sqlSource: "s_7c5e2e11",
    dimension: "kelompok",
    measures: ["materials"],
  })
  expect(def.sqlSource).toBe("s_7c5e2e11")
  expect(def.mart).toBeUndefined()
})

test("a mart chart is unchanged", () => {
  const def = chartDefFromArgs({ title: "T", kind: "bar", mart: "mart_x", dimension: "d", measures: ["m"] })
  expect(def.mart).toBe("mart_x")
  expect(def.sqlSource).toBeUndefined()
})

test("a map chart keeps its map and coordinate columns for the preview and the builder", () => {
  const def = chartDefFromArgs({
    title: "Visitors by place", kind: "pointmap", mart: "mart_x", measures: ["visitors"],
    map: "id-provinces", lat: "lat", lon: "lon",
  })
  expect([def.map, def.lat, def.lon]).toEqual(["id-provinces", "lat", "lon"])
  const plain = chartDefFromArgs({ title: "T", kind: "bar", mart: "mart_x", dimension: "d", measures: ["m"], lat: "  " })
  expect([plain.map, plain.lat, plain.lon]).toEqual([undefined, undefined, undefined])
})
