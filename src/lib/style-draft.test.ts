import { describe, expect, test } from "bun:test"
import type { StyleFields } from "./chart-style"
import { NOTE_KINDS, effectivePalette, formatRules, formatUsed, styleFromDef, stylePayload, withCompanyPalette, withNote } from "./style-draft"

const FULL: StyleFields = {
  format: { style: "compact", currency: "IDR", decimals: 2, thousands: false, currencyDisplay: "code", prefix: "~", suffix: " x" },
  valueFormats: { visitors: { style: "percent", decimals: 1 } },
  dateFormat: { style: "monthYear", hour12: true, abbreviate: true },
  colors: { palette: "muted", series: { A: "#112233" }, byValue: ["#000000", "#ffffff"] },
  axes: { x: { title: "Province", rotate: 45 }, y: { title: "Visitors", min: 0, max: 10, log: true, show: false }, y2: { title: "Nights" }, rightValues: ["nights"] },
  labels: { show: true },
  annotations: [{ x: "Bali", series: "A", text: "Peak" }],
  tooltip: { fields: ["visitors"] },
}

describe("formatRules", () => {
  test("a bar chart has colours, axes, labels, reference lines and notes but no conditional colours", () => {
    const r = formatRules({ kind: "bar", breakdown: false, measures: 1 })
    expect(r).toMatchObject({ values: true, dates: true, colors: true, colorByValue: true, axes: true, labels: true, notes: true, tooltip: true, referenceLines: true, conditional: false, secondAxis: false })
  })

  test("a table has conditional colours and none of the chart groups", () => {
    const r = formatRules({ kind: "table", breakdown: false, measures: 2 })
    expect(r).toMatchObject({ values: true, conditional: true, colors: false, axes: false, labels: false, notes: false, referenceLines: false })
  })

  test("colour by value needs one value and no breakdown; a second axis needs two values", () => {
    expect(formatRules({ kind: "bar", breakdown: true, measures: 1 }).colorByValue).toBe(false)
    expect(formatRules({ kind: "bar", breakdown: false, measures: 2 }).colorByValue).toBe(false)
    expect(formatRules({ kind: "line", breakdown: false, measures: 2 }).secondAxis).toBe(true)
    expect(formatRules({ kind: "hbar", breakdown: false, measures: 2 }).secondAxis).toBe(false)
  })

  test("a text tile has nothing to format and a KPI has formats and colours by rule only", () => {
    expect(Object.values(formatRules({ kind: "text", breakdown: false, measures: 0 })).some(Boolean)).toBe(false)
    expect(formatRules({ kind: "kpi", breakdown: false, measures: 1 })).toMatchObject({ values: true, conditional: true, axes: false, colors: false })
  })
})

describe("stylePayload", () => {
  test("a saved style with every field reads back and sends the same JSON", () => {
    const rules = formatRules({ kind: "bar", breakdown: false, measures: 2 })
    const draft = styleFromDef(FULL)
    // Colour by value needs one value: with two, it is dropped, as the server would refuse it.
    const sent = stylePayload(draft, { ...rules, colorByValue: true })
    expect(sent).toEqual(FULL)
    expect(JSON.stringify(stylePayload(styleFromDef(sent), { ...rules, colorByValue: true }))).toBe(JSON.stringify(sent))
  })

  test("a group the kind does not take is dropped", () => {
    const bar = stylePayload(styleFromDef({ ...FULL, conditional: [{ column: "v", rule: { type: "contains", text: "x" }, color: "#000000" }] }), formatRules({ kind: "bar", breakdown: false, measures: 2 }))
    expect(bar.conditional).toBeUndefined()
    expect(bar.colors?.byValue).toBeUndefined()
    const table = stylePayload(styleFromDef(FULL), formatRules({ kind: "table", breakdown: false, measures: 2 }))
    expect(Object.keys(table).sort()).toEqual(["dateFormat", "format", "valueFormats"])
  })

  test("empty and default choices are not sent", () => {
    const rules = formatRules({ kind: "bar", breakdown: false, measures: 1 })
    const draft: StyleFields = {
      format: { style: "auto" }, valueFormats: { v: {} }, dateFormat: { style: "auto" }, colors: { series: {} },
      axes: { x: { show: true }, y: { title: "" } }, labels: { show: false }, annotations: [{ x: "Bali", text: " " }], tooltip: { fields: [] },
    }
    expect(stylePayload(draft, rules)).toEqual({})
    expect(formatUsed({}, 0)).toBe(false)
  })

  test("a currency code is sent only with the currency or compact style", () => {
    const rules = formatRules({ kind: "bar", breakdown: false, measures: 1 })
    expect(stylePayload({ format: { style: "number", currency: "IDR", decimals: 0 } }, rules).format).toEqual({ style: "number", decimals: 0 })
  })

  test("a rule that is not complete is not sent, and a range carries no colour of its own", () => {
    const rules = formatRules({ kind: "table", breakdown: false, measures: 1 })
    const sent = stylePayload({ conditional: [
      { column: "v", rule: { type: "compare", op: "lt", value: 5 } },
      { column: "", rule: { type: "contains", text: "x" }, color: "#000000" },
      { column: "v", rule: { type: "range", colors: ["#000000", "#ffffff"] }, color: "#ff0000" },
      { column: "v", rule: { type: "compare", op: "gt", value: 1 }, color: "#00ff00", target: "text" },
    ] }, rules)
    expect(sent.conditional).toEqual([
      { column: "v", rule: { type: "range", colors: ["#000000", "#ffffff"] } },
      { column: "v", rule: { type: "compare", op: "gt", value: 1 }, color: "#00ff00", target: "text" },
    ])
  })

  test("the Format section opens by itself for a chart that uses it", () => {
    expect(formatUsed({ labels: { show: true } }, 0)).toBe(true)
    expect(formatUsed({}, 2)).toBe(true)
  })
})

describe("withNote", () => {
  test("adds the note after the ones there and leaves the rest of the definition alone", () => {
    const def = { title: "T", annotations: [{ x: "Bali", text: "Peak" }] }
    const next = withNote(def, "Java", "  Dip  ")
    expect(next).toEqual({ title: "T", annotations: [{ x: "Bali", text: "Peak" }, { x: "Java", text: "Dip" }] })
    expect(def.annotations).toHaveLength(1)
    expect(withNote({ title: "T" } as { annotations?: { x: string; text: string }[] }, "Bali", "x").annotations).toEqual([{ x: "Bali", text: "x" }])
  })

  test("the kinds that take a note are the kinds with a category or date axis", () => {
    expect(NOTE_KINDS).toContain("bar")
    expect(NOTE_KINDS).not.toContain("pie")
  })
})

describe("the company palette of a new chart (BI-17 part A, decision 6)", () => {
  const company = ["#112233", "#445566"]

  test("a new chart with no palette of its own draws with the company palette", () => {
    expect(withCompanyPalette({}, company, false).colors).toEqual({ palette: "company" })
  })

  test("a palette the person chose is kept", () => {
    const style: StyleFields = { colors: { palette: "muted" } }
    expect(withCompanyPalette(style, company, false).colors).toEqual({ palette: "muted" })
  })

  test("a chart being edited keeps what it has", () => {
    expect(withCompanyPalette({}, company, true).colors).toBeUndefined()
  })

  test("without a company palette nothing changes", () => {
    expect(withCompanyPalette({}, [], false).colors).toBeUndefined()
    expect(withCompanyPalette({}, ["#112233"], false).colors).toBeUndefined()
  })
})

// PR #199 review O SHOULD-FIX 4: the builder must be able to keep "Default"
// on a new chart once a company palette exists.
describe("the palette the builder shows", () => {
  const company = ["#112233", "#445566"]

  test("a new chart with no choice of its own shows the company palette", () => {
    expect(effectivePalette({}, company, false)).toBe("company")
  })

  test("a palette the person chose is shown as it is", () => {
    const style: StyleFields = { colors: { palette: "muted" } }
    expect(effectivePalette(style, company, false)).toBe("muted")
  })

  test("the default the person chose is shown as the default, not as the company palette", () => {
    const style: StyleFields = { colors: { palette: "default" } }
    expect(effectivePalette(style, company, false)).toBe("default")
    // And it is kept when the chart is saved.
    expect(withCompanyPalette(style, company, false).colors).toEqual({ palette: "default" })
  })

  test("a chart being edited and a deployment without a company palette show the default", () => {
    expect(effectivePalette({}, company, true)).toBe("default")
    expect(effectivePalette({}, [], false)).toBe("default")
  })
})
