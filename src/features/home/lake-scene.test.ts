import { describe, expect, it } from "bun:test"
import { readFileSync } from "node:fs"
import { BASE, BLINK, bodies, eventAt, field, HZ, poseFor, REST, sceneInks, SPOUT, waterlineAt, whalePose } from "./lake-scene"

// Regions of `field`: 1 whale, 3 the eye (and moonlight), 4 moon, 6 sun, 8 the spout.
const WHALE = 1
const EYE = 3
const MOON = 4
const SUN = 6
const SPRAY = 8

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

// WEB-17 T11 (owner QA 2026-11-15): "entah kurang detail atau kek statis dan ga gerak".
describe("the whale lives in the water (WEB-17 T11)", () => {
  const aspect = 1110 / 400
  const wx = 0
  const R = 0.2

  /** Event starts and lengths over `span` seconds, sampled every 5 ms. */
  const events = (e: typeof BLINK | typeof SPOUT, seed: number, span: number) => {
    const starts: number[] = []
    const lengths: number[] = []
    let since = -1
    for (let t = 0; t <= span; t += 0.005) {
      const on = eventAt(t, e, seed) >= 0
      if (on && since < 0) {
        since = t
        starts.push(t)
      }
      if (!on && since >= 0) {
        lengths.push(t - since)
        since = -1
      }
    }
    return { starts, lengths }
  }

  it("meets the water along a line that follows the ripples, never a straight edge", () => {
    // Where the head ends going down, across its width, at a 400 px band (one cell is 3 px).
    for (const t of [6, 9.3, 14.1]) {
      const pose = whalePose(t)
      const bottoms: number[] = []
      for (let u = 0.5 - 0.8 * R / aspect; u <= 0.5 + 0.8 * R / aspect; u += 0.004) {
        let v = HZ - 0.1
        while (field(u, v, aspect, t, 1, 0, 0, pose)[1] === WHALE || field(u, v, aspect, t, 1, 0, 0, pose)[1] === EYE) v += 0.0025
        bottoms.push(v)
      }
      // At least 2 cells (6 px of 400) between its highest and lowest point.
      expect(Math.max(...bottoms) - Math.min(...bottoms)).toBeGreaterThanOrEqual(6 / 400)
    }
    // And the line moves: at one point it is not where it was a moment ago.
    const at = (t: number) => waterlineAt(wx + 0.05, t, whalePose(t))
    expect(Math.abs(at(6) - at(6.5))).toBeGreaterThan(0.002)
  })

  it("leaves no straight bar under the head: just under the waterline is water, not the mark's gap", () => {
    for (let u = 0.5 - 0.9 * R / aspect; u <= 0.5 + 0.9 * R / aspect; u += 0.004) {
      const x = (u - 0.5) * aspect
      const v = waterlineAt(x, 6, REST) + 0.01
      expect(field(u, v, aspect, 6, 1, 0, 0, REST)[1]).not.toBe(EYE)
    }
  })

  it("is a lit volume: brighter on the sun's side by day and the moon's side by night", () => {
    const light = (u: number, phase: number) => field(u, HZ - 0.12, aspect, 6, phase, 0, 0)[0]
    const left = 0.5 - 0.7 * R / aspect
    const right = 0.5 + 0.7 * R / aspect
    expect(light(left, 0)).toBeGreaterThan(light(right, 0))
    expect(light(right, 1)).toBeGreaterThan(light(left, 1))
    // Darker just above the water than higher up on the same side (the wet shadow).
    const lowU = 0.5 - 0.3 * R / aspect
    const xLow = (lowU - 0.5) * aspect
    const wet = field(lowU, waterlineAt(xLow, 6, REST) - 0.004, aspect, 6, 0, 0, 0)
    const dry = field(lowU, waterlineAt(xLow, 6, REST) - 0.06, aspect, 6, 0, 0, 0)
    expect(wet[1]).toBe(WHALE)
    expect(wet[0]).toBeLessThan(dry[0])
  })

  it("blinks for 120 ms every 6 to 12 s, the same way for the same seed", () => {
    for (const seed of [0, 1, 7]) {
      const { starts, lengths } = events(BLINK, seed, 600)
      expect(starts.length).toBeGreaterThan(45)
      for (let k = 1; k < starts.length; k++) {
        const gap = starts[k] - starts[k - 1]
        expect(gap).toBeGreaterThanOrEqual(6 - 0.01)
        expect(gap).toBeLessThanOrEqual(12 + 0.01)
      }
      for (const l of lengths) expect(l).toBeCloseTo(0.12, 1)
    }
    expect(events(BLINK, 0, 120).starts).toEqual(events(BLINK, 0, 120).starts)
    expect(events(BLINK, 0, 120).starts).not.toEqual(events(BLINK, 1, 120).starts)
    // Shut means the eye is a slit, not gone.
    const shut = { ...REST, eye: 0 }
    let open = 0
    let slit = 0
    for (let v = HZ - 0.12; v < HZ - 0.03; v += 0.002) {
      if (field(0.5 + (0.342 * R) / aspect, v, aspect, 6, 1, 0, 0, REST)[1] === EYE) open++
      if (field(0.5 + (0.342 * R) / aspect, v, aspect, 6, 1, 0, 0, shut)[1] === EYE) slit++
    }
    expect(slit).toBeGreaterThan(0)
    expect(slit).toBeLessThan(open / 3)
  })

  it("spouts for 1.5 s every 20 to 40 s, a plume above the head that rises and falls", () => {
    for (const seed of [0, 1, 7]) {
      const { starts, lengths } = events(SPOUT, seed, 1200)
      expect(starts.length).toBeGreaterThan(28)
      for (let k = 1; k < starts.length; k++) {
        const gap = starts[k] - starts[k - 1]
        expect(gap).toBeGreaterThanOrEqual(20 - 0.01)
        expect(gap).toBeLessThanOrEqual(40 + 0.01)
      }
      for (const l of lengths) expect(l).toBeCloseTo(1.5, 1)
    }
    // How high the spray reaches above the head at each moment: up, then back down.
    const reach = (p: number) => {
      let top = 1
      for (let v = 0; v < HZ; v += 0.0025) {
        for (let u = 0.44; u < 0.52; u += 0.002) {
          if (field(u, v, aspect, 6, 1, Math.round(u * 400), Math.round(v * 400), { ...REST, spout: p })[1] === SPRAY) top = Math.min(top, v)
        }
      }
      return top
    }
    expect(reach(0.4)).toBeLessThan(reach(0.1))
    expect(reach(0.95)).toBeGreaterThan(reach(0.4))
    // It stays well inside the band (14 px below its top at 400 px).
    expect(reach(0.4)).toBeGreaterThan(0.035)
  })

  it("does not move at all under reduced motion: no bob, eye open, no spout", () => {
    for (let t = 0; t < 300; t += 0.05) expect(poseFor(t, true)).toEqual(REST)
    // And it does move otherwise.
    const poses = Array.from({ length: 600 }, (_, k) => poseFor(k * 0.1, false))
    expect(new Set(poses.map((p) => p.bob.toFixed(4))).size).toBeGreaterThan(10)
    expect(poses.some((p) => p.eye === 0)).toBe(true)
    expect(poses.some((p) => p.spout >= 0)).toBe(true)
  })

  it("never moves the whale or its spout behind the greeting", () => {
    const boxes = [
      { aspect: 1100 / 400, u0: 0.32, u1: 0.68, v0: 0.425, v1: 0.515 },
      { aspect: 390 / 300, u0: 0.04, u1: 0.96, v0: 0.42, v1: 0.65 },
    ]
    for (const b of boxes) {
      for (let t = 0; t < 60; t += 0.37) {
        const pose = { ...whalePose(t), spout: (t / 60) % 1 }
        for (let u = b.u0; u <= b.u1; u += 0.02) {
          for (let v = b.v0; v <= b.v1; v += 0.01) {
            expect([WHALE, SPRAY]).not.toContain(field(u, v, b.aspect, t, 1, 0, 0, pose)[1])
          }
        }
      }
    }
  })

  it("the spout adds no colour of its own", () => {
    for (const p of [0, 1]) {
      const { inks } = sceneInks(p, "dither")
      expect(inks.slice(0, SPRAY).map(String)).toContain(String(inks[SPRAY]))
    }
  })
})
