import { expect, test } from "bun:test"
import { chartDefFromArgs, draftMeasureLabel } from "./chart-draft-card"

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

test("a create_chart call with layers keeps them, and how its own series is drawn, for the preview and the builder (BI-13)", () => {
  const layers = [{ mart: "mart_rooms", dimension: "month", measures: ["rooms"], draw: "line", axis: "right" }]
  const def = chartDefFromArgs({ title: "T", kind: "bar", mart: "mart_x", dimension: "month", measures: ["visitors"], layers, draw: "bar", axis: "left" })
  expect(def.layers).toEqual(layers as never)
  expect([def.draw, def.axis]).toEqual(["bar", "left"])
  const plain = chartDefFromArgs({ title: "T", kind: "bar", mart: "mart_x", dimension: "d", measures: ["m"], draw: "dots", axis: "up" })
  expect([plain.layers, plain.draw, plain.axis]).toEqual([undefined, undefined, undefined])
})

test("a create_chart call keeps every field Confirm sends, so the preview and the builder show the chart that is saved", () => {
  const sent = {
    grain: "month",
    tableMode: "rows",
    columns: ["year", "month"],
    rows: ["country"],
    values: [{ column: "visitors", aggregate: "sum" }],
    totals: "all",
    sortColumn: "visitors",
    sortDir: "desc",
    columnSettings: { visitors: { label: "Visitors" } },
    compare: { kind: "last_year", dateColumn: "day", period: "month" },
    goodDirection: "down",
    order: "asc",
    limit: { param: "top_n" },
    referenceLines: [{ value: 100, label: "Target" }, { value: { param: "cap" } }],
    click: { kind: "url", url: "https://example.org/{value}" },
  }
  const def = chartDefFromArgs({ title: "T", kind: "pivot", mart: "mart_x", ...sent })
  expect(def).toMatchObject(sent as never)
  expect(chartDefFromArgs({ title: "T", mart: "mart_x", limit: 5 }).limit).toBe(5)
  expect(chartDefFromArgs({ title: "T", mart: "mart_x", limit: "5" }).limit).toBeUndefined()
})

test("a create_chart call with only a text field is titled by that text", () => {
  expect(chartDefFromArgs({ kind: "text", text: "Notes for the week" }).title).toBe("Notes for the week")
  expect(chartDefFromArgs({ title: "T", caption: "C", text: "X" }).title).toBe("T")
})

test("a saved metric reads by its label, with no aggregate around it", () => {
  const def = chartDefFromArgs({ title: "T", mart: "mart_x", measures: [{ metric: "visitors_total" }] })
  const label = draftMeasureLabel(def, { visitors_total: "Total visitors" })
  expect(label).toBe("Total visitors")
  expect(label).not.toContain("sum(")
})

test("a saved metric reads by its name until the preview has sent its label", () => {
  const def = chartDefFromArgs({ title: "T", mart: "mart_x", measures: [{ metric: "visitors_total" }] })
  const label = draftMeasureLabel(def, undefined)
  expect(label).toBe("visitors_total")
  expect(label).not.toContain("sum(")
})

test("a measure with its own aggregate reads with that aggregate, not the chart's", () => {
  const def = chartDefFromArgs({ title: "T", mart: "mart_x", measures: [{ column: "x", aggregate: "avg" }] })
  expect(draftMeasureLabel(def, undefined)).toBe("avg(x)")
})

test("a plain column measure reads as a sum when the chart sets no aggregate, and as the chart's aggregate otherwise", () => {
  expect(draftMeasureLabel(chartDefFromArgs({ title: "T", mart: "mart_x", measures: ["x"] }), undefined)).toBe("sum(x)")
  expect(draftMeasureLabel(chartDefFromArgs({ title: "T", mart: "mart_x", measures: ["x"], aggregate: "max" }), undefined)).toBe("max(x)")
})

test("a chart grouped by a date grain says so after its measures", () => {
  const def = chartDefFromArgs({ title: "T", mart: "mart_x", measures: ["x"], grain: "month" })
  expect(draftMeasureLabel(def, undefined)).toBe("sum(x) per month")
})

test("a chart with no measures has no measure label", () => {
  expect(draftMeasureLabel(chartDefFromArgs({ title: "T", mart: "mart_x" }), undefined)).toBe("")
})

// BI-18 part C review fix (SHOULD-FIX) R25, R26: the draft previews what Confirm saves.
test("a histogram's width and a progress goal reach the preview as the model sent them", () => {
  const histogram = chartDefFromArgs({ title: "H", kind: "histogram", mart: "m", dimension: "visitors", histogram: { width: 500 } })
  expect(histogram.histogram).toEqual({ width: 500 })
  const progress = chartDefFromArgs({ title: "P", kind: "progress", mart: "m", measures: ["visitors"], goal: 300000 })
  expect(progress.goal).toBe(300000)
  const second = chartDefFromArgs({ title: "B", kind: "bullet", mart: "m", dimension: "d", measures: ["a", "b"], goal: { measure: "b" }, bands: [1, 2, 3] })
  expect(second.goal).toEqual({ measure: "b" })
  expect(second.bands).toEqual([1, 2, 3])
  const image = chartDefFromArgs({ title: "I", kind: "image", url: "https://example.com/a.png", alt: "A" })
  expect([image.url, image.alt]).toEqual(["https://example.com/a.png", "A"])
})

test("every field Confirm sends is in the draft, so the previewed and saved charts are one", () => {
  const args = {
    title: "T", kind: "bar", mart: "m", dimension: "d", measures: ["v"],
    format: { style: "currency", currency: "IDR" }, colors: { series: { v: "#16a34a" } },
    histogram: { count: 10 }, goal: 5, bands: [1], url: "https://example.com", alt: "x",
  }
  const def = chartDefFromArgs(args) as Record<string, unknown>
  for (const key of ["format", "colors", "histogram", "goal", "bands", "url", "alt"]) {
    expect(def[key]).toEqual((args as Record<string, unknown>)[key])
  }
  // Not sending a width or a count stays automatic: no histogram key appears.
  expect(chartDefFromArgs({ title: "H", kind: "histogram", mart: "m", dimension: "v" }).histogram).toBeUndefined()
})
