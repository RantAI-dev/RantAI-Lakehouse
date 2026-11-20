import { describe, expect, it } from "bun:test"

import { drawsFrom, kindNeed, MAIN_KINDS, previewShape } from "./kind-gallery"
import { KIND_GROUPS } from "@/features/dashboards/chart-kinds"

const rich = { dates: 1, groups: 3, numbers: 3 }

describe("kind-gallery (BI-58 T3)", () => {
  it("keeps the eight toolbar types among the offered ones", () => {
    const offered = new Set(KIND_GROUPS.flatMap((g) => g.items.map((i) => i.value)))
    expect(MAIN_KINDS).toHaveLength(8)
    for (const kind of MAIN_KINDS) expect(offered.has(kind), kind).toBe(true)
  })

  it("greys out nothing before a source is chosen, and nothing a rich source can make", () => {
    for (const g of KIND_GROUPS) for (const i of g.items) {
      expect(kindNeed(i.value, null), i.value).toBeNull()
      expect(kindNeed(i.value, rich), i.value).toBeNull()
    }
  })

  it("names what the source lacks, from the builder's own requirements", () => {
    expect(kindNeed("calendar", { ...rich, dates: 0 })).toBe("Needs a date column")
    expect(kindNeed("gantt", { ...rich, dates: 0 })).toBe("Needs a date column")
    expect(kindNeed("heatmap", { ...rich, groups: 1 })).toBe("Needs two text or date columns")
    expect(kindNeed("scatter", { ...rich, numbers: 1 })).toBe("Needs 2 number columns")
    expect(kindNeed("bubble", { ...rich, numbers: 2 })).toBe("Needs 3 number columns")
    expect(kindNeed("bar", { ...rich, numbers: 0 })).toBe("Needs a number column")
    expect(kindNeed("histogram", { ...rich, numbers: 0 })).toBe("Needs a number column")
    expect(kindNeed("geomap", { ...rich, groups: 0 })).toBe("Needs a column of region names")
    // A table, a pivot and a note need no value.
    for (const kind of ["table", "pivot", "text", "image", "web"] as const) expect(kindNeed(kind, { dates: 0, groups: 0, numbers: 0 }), kind).toBeNull()
  })

  it("reads the preview's shape from its spec", () => {
    expect(previewShape({ kind: "bar", y: "v" }, false)).toEqual({ shape: "category", values: 1 })
    expect(previewShape({ kind: "bar", y: ["a", "b"] }, false)).toEqual({ shape: "category", values: 2 })
    expect(previewShape({ kind: "line", y: "v", series: "s" }, false)).toEqual({ shape: "series", values: 1 })
    expect(previewShape({ kind: "bar", y: "v" }, true)).toBeNull()
    expect(previewShape({ kind: "kpi", y: "v" }, false)).toBeNull()
    expect(previewShape({ kind: "geomap", y: "v" }, false)).toBeNull()
  })

  it("draws a type from rows of the same shape only", () => {
    const one = previewShape({ kind: "bar", y: "v" }, false)
    expect(drawsFrom("pie", one)).toBe(true)
    expect(drawsFrom("line", one)).toBe(true)
    expect(drawsFrom("stacked", one)).toBe(false)
    expect(drawsFrom("kpi", one)).toBe(false)
    expect(drawsFrom("heatmap", one)).toBe(false)
    expect(drawsFrom("stacked", previewShape({ kind: "bar", y: ["a", "b"] }, false))).toBe(true)
    const split = previewShape({ kind: "bar", y: "v", series: "s" }, false)
    expect(drawsFrom("heatmap", split)).toBe(true)
    expect(drawsFrom("pie", split)).toBe(false)
    expect(drawsFrom("bar", null)).toBe(false)
  })
})
