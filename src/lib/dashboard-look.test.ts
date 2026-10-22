import { describe, expect, test } from "bun:test"
import { FONT_FAMILIES, dashboardLook } from "./dashboard-look"
import { SHIPPED_FONT_IDS } from "./formatting-settings"

describe("dashboardLook", () => {
  test("auto follows the console", () => {
    expect(dashboardLook(undefined, true)).toEqual({ dark: true, className: "", style: undefined })
    expect(dashboardLook({ theme: "auto" }, false)).toEqual({ dark: false, className: "", style: undefined })
  })

  test("a dark dashboard is dark inside a light console, a light one is light inside a dark console", () => {
    expect(dashboardLook({ theme: "dark" }, false)).toMatchObject({ dark: true, className: "dark bg-background text-foreground" })
    expect(dashboardLook({ theme: "light" }, true)).toMatchObject({ dark: false, className: "theme-light bg-background text-foreground" })
  })

  test("a font the console ships and the administrator allows is applied", () => {
    expect(dashboardLook({ font: "source-serif" }, false).style?.fontFamily).toContain("var(--font-source-serif)")
    expect(dashboardLook({ font: "inter" }, false, ["inter", "system"]).style).toBeDefined()
  })

  test("a font removed from the allowed list, or unknown, falls back to the console's", () => {
    expect(dashboardLook({ font: "inter" }, false, ["system"]).style).toBeUndefined()
    expect(dashboardLook({ font: "comic-sans" }, false).style).toBeUndefined()
  })

  test("a theme the server does not know is read as auto", () => {
    expect(dashboardLook({ theme: "sepia" }, true)).toMatchObject({ dark: true, className: "" })
  })

  test("every shipped font id has a family and no other does", () => {
    expect(Object.keys(FONT_FAMILIES).sort()).toEqual([...SHIPPED_FONT_IDS].sort())
  })
})
