import { describe, expect, test } from "bun:test"
import { DEFAULT_PALETTE, PALETTES, contrast, interpolate, resolvePalette } from "./chart-palettes"
import { BUILTIN_PALETTES } from "./chart-style"

const LIGHT_CARD = "#ffffff"
const DARK_CARD = "#18181b"

describe("palettes", () => {
  test("every built-in id has a palette and no other id does", () => {
    expect(Object.keys(PALETTES).sort()).toEqual([...BUILTIN_PALETTES].sort())
  })

  // Plan mismatch, reported in the handoff: feature decision 6 says all five palettes pass, but
  // the current default (kept as it is so no saved chart changes colour) does not: its sky,
  // amber, lime and orange are below 3:1 on the light card. Only the four new palettes are held to it.
  test("every colour of the four new palettes reaches 3:1 against the light and the dark card", () => {
    for (const [id, colours] of Object.entries(PALETTES)) {
      if (id === "default") continue
      for (const c of colours) {
        expect(contrast(c, LIGHT_CARD), `${id} ${c} on light`).toBeGreaterThanOrEqual(3)
        expect(contrast(c, DARK_CARD), `${id} ${c} on dark`).toBeGreaterThanOrEqual(3)
      }
    }
  })

  test("the default palette is the one the console always used", () => {
    expect(DEFAULT_PALETTE.slice(0, 4)).toEqual(["#6366f1", "#0ea5e9", "#10b981", "#f59e0b"])
    expect(DEFAULT_PALETTE).toHaveLength(10)
  })

  test("company needs two colours, else the default; an unknown id is the default", () => {
    expect(resolvePalette({ palette: "company" }, ["#112233", "#445566"])).toEqual(["#112233", "#445566"])
    expect(resolvePalette({ palette: "company" }, ["#112233"])).toBe(DEFAULT_PALETTE)
    expect(resolvePalette({ palette: "company" }, undefined)).toBe(DEFAULT_PALETTE)
    expect(resolvePalette({ palette: "nope" }, undefined)).toBe(DEFAULT_PALETTE)
    expect(resolvePalette(undefined, ["#112233", "#445566"])).toBe(DEFAULT_PALETTE)
    expect(resolvePalette({ palette: "muted" }, undefined)).toBe(PALETTES.muted!)
  })
})

describe("interpolate", () => {
  test("two and three stops", () => {
    expect(interpolate(["#000000", "#ffffff"], 0.5)).toBe("#808080")
    expect(interpolate(["#000000", "#ff0000", "#ffffff"], 0.5)).toBe("#ff0000")
    expect(interpolate(["#000000", "#ffffff"], 2)).toBe("#ffffff")
    expect(interpolate(["#000000", "#ffffff"], Number.NaN)).toBe("#000000")
  })
})
