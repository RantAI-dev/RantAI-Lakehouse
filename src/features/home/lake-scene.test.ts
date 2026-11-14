import { describe, expect, it } from "bun:test"
import { bodies, field, HZ } from "./lake-scene"

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
