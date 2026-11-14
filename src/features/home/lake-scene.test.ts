import { describe, expect, it } from "bun:test"
import { readFileSync } from "node:fs"
import { BASE, bodies, field, HZ, sceneInks } from "./lake-scene"

// Regions of `field`: 1 whale, 4 moon, 6 sun.
const WHALE = 1
const MOON = 4
const SUN = 6

describe("the lake's day and night (WEB-17 S2, owner 2026-11-14)", () => {
  it("by day the sun is up and the moon is down; by night the reverse", () => {
    const day = bodies(0)
    const night = bodies(1)
    expect(day.sunY).toBeLessThan(HZ)
    expect(day.moonY - 0.07).toBeGreaterThan(HZ)
    expect(night.moonY).toBeLessThan(HZ)
    expect(night.sunY - 0.07).toBeGreaterThan(HZ)
  })

  it("draws the sun on the left by day and the moon on the right by night", () => {
    const aspect = 2.75
    const at = (u: number, v: number, phase: number) => field(u, v, aspect, 6, phase, 0, 0)[1]
    expect(at(0.2, bodies(0).sunY, 0)).toBe(SUN)
    expect(at(0.8, bodies(0).moonY, 0)).not.toBe(MOON)
    expect(at(0.8, bodies(1).moonY, 1)).toBe(MOON)
    expect(at(0.2, bodies(1).sunY, 1)).not.toBe(SUN)
  })

  it("never puts the whale, the sun or the moon behind the greeting, at any moment of a theme change", () => {
    // Where Home puts the greeting: a laptop (one line) and a phone (up to two lines, full width).
    const boxes = [
      { aspect: 1100 / 400, u0: 0.32, u1: 0.68, v0: 0.425, v1: 0.515 },
      { aspect: 390 / 300, u0: 0.04, u1: 0.96, v0: 0.42, v1: 0.65 },
    ]
    for (const b of boxes) {
      for (let p = 0; p <= 1.0001; p += 0.05) {
        for (let u = b.u0; u <= b.u1; u += 0.01) {
          for (let v = b.v0; v <= b.v1; v += 0.005) {
            const region = field(u, v, b.aspect, 6, p, 0, 0)[1]
            expect([WHALE, MOON, SUN]).not.toContain(region)
          }
        }
      }
    }
  })
})

// WEB-17 T9 (owner 2026-11-15): the scene is a dim background. WCAG relative luminance, from the
// colours themselves; the greeting's colour is the theme's foreground (design-system colors.css,
// oklch 0.148 light and 0.987 dark, written here as sRGB).
const lin = (c: number) => ((c /= 255) <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4)
const lum = (c: readonly number[]) => 0.2126 * lin(c[0]) + 0.7152 * lin(c[1]) + 0.0722 * lin(c[2])
const ratio = (a: readonly number[], b: readonly number[]) => {
  const [hi, lo] = [lum(a), lum(b)].sort((x, y) => y - x)
  return (hi + 0.05) / (lo + 0.05)
}
const rgb = (hex: string) => [1, 3, 5].map((k) => parseInt(hex.slice(k, k + 2), 16))
const TEXT = { day: rgb("#0b0f11"), night: rgb("#f8fbfc") }

describe("the lake is a middle-dim background (WEB-17 T9 and T10, owner 2026-11-15)", () => {
  const styles = ["dither", "ascii"] as const
  const between = [0.1, 0.25, 0.5, 0.75, 0.9]
  const strongest = (p: number, style: (typeof styles)[number]) => {
    const { inks, base } = sceneInks(p, style)
    return inks.map((ink) => ratio(ink, base))
  }

  it("the day base is the light page background, pinned to the --background token", () => {
    const css = readFileSync(new URL("../../../design-system/tokens/colors.css", import.meta.url), "utf8")
    // The first `--background` is the light theme's: oklch(1 0 0) is white.
    expect(css.match(/--background:\s*([^;]+);/)?.[1].trim()).toBe("oklch(1 0 0)")
    expect(BASE.day).toBe("#ffffff")
  })

  it("at rest the most contrasting ink is between 2.5:1 and 3.0:1 against its base, per style", () => {
    for (const style of styles) {
      for (const p of [0, 1]) {
        const top = Math.max(...strongest(p, style))
        expect(top).toBeGreaterThanOrEqual(2.5)
        expect(top).toBeLessThanOrEqual(3.0)
      }
    }
    // ASCII sits near the top of the window, Dither nearer the bottom.
    for (const p of [0, 1]) {
      expect(Math.max(...strongest(p, "ascii"))).toBeGreaterThan(Math.max(...strongest(p, "dither")))
    }
  })

  it("between day and night no ink goes past 3.0:1", () => {
    for (const style of styles) {
      for (const p of between) expect(Math.max(...strongest(p, style))).toBeLessThanOrEqual(3.0)
    }
  })

  it("the greeting's text is at least 4.5:1 against every ink and the base, day and night, both styles", () => {
    // At rest. During the 1.5 s theme change the text already has the new theme's colour while
    // the scene is still blending, so the midpoints are not a state anyone reads in.
    for (const style of styles) {
      for (const [p, text] of [[0, TEXT.day], [1, TEXT.night]] as const) {
        const { inks, base } = sceneInks(p, style)
        for (const c of [...inks, base]) expect(ratio(text, c)).toBeGreaterThanOrEqual(4.5)
      }
    }
  })

  it("keeps each ink's hue: the day whale is still blue, not grey", () => {
    const [r, , b] = sceneInks(0, "dither").inks[1]
    expect(b - r).toBeGreaterThan(10)
  })
})
