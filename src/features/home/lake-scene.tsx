"use client"

/**
 * Home's lake as print (`WEB-17` D1, mockup B): sky, the sun or the moon,
 * the whale's head from the RantAI Lakehouse mark at the waterline, its
 * broken reflection, ripples and the depth rings spreading from it, drawn
 * either as an ordered 4×4 Bayer dither ("Dither", the default) or as text
 * glyphs ("ASCII") in the logo's inks.
 *
 * Day and night follow the theme (owner, 2026-11-14: "kalo buat light mode
 * berarti kondisinya dibuat siang juga, matahari muncul di sebelah kiri;
 * kalo dark berarti matahari turun dan bulan naik di sebelah kanan"). The
 * light theme is a day lake printed on the page white: the sun high on the left, its
 * light sparkling on the water, the whale in dense navy. The dark theme is
 * night on deep water: the moon high on the right, stars, the whale in lake
 * blue. One number, the phase (0 day, 1 night), places the sun and the
 * moon and blends every ink and the base colour, so a theme switch on Home
 * is the sun setting on the left while the moon rises on the right (or the
 * reverse), 1.5 s eased, drawn at up to 30 fps only while it runs. Under
 * reduced motion the end state is drawn at once.
 *
 * Our own code, from the mark's own geometry (`RantAI Lakehouse *.svg`: a
 * circle cut a quarter radius below its centre, the eye at (+0.34 r,
 * −0.14 r)); ported from the owner-approved mockup
 * (`design/chart-builder-mockups`, `home-history/lake-art.tsx`). No
 * third-party code and no traced picture (public AGPL repo).
 *
 * The whole picture is a dim background (WEB-17 T9, owner 2026-11-15): every
 * ink is moved towards the band's base by one factor per phase (`DIM`), so the
 * art never competes with the greeting and nothing marks the text.
 *
 * The whale lives in the water (WEB-17 T11, owner QA 2026-11-15: "entah kurang
 * detail atau kek statis dan ga gerak jadi bikin aneh aja"). Its head is a lit
 * volume, brighter on the side of the sun (day, left) or the moon (night,
 * right) and darker towards the water; it meets the water along a waterline
 * that follows the ripples, with a band of wet shadow above it and a broken
 * glint below it, instead of the mark's straight gap and band. It bobs with
 * the swell, blinks now and then and spouts now and then (`whalePose`). Only
 * the whale's own box is redrawn for that, inside the water's 10 fps loop.
 * The eye stays plain (DESIGN.md §5: no glow, no pupil, no shine).
 *
 * What it costs, and the rules that keep it cheap (spec "Scene cost"):
 * - It is sized to the hero band only, never the page.
 * - The first drawing waits for the browser to be idle after first paint,
 *   so it never delays the composer or the largest paint; until then the
 *   band is its solid base colour (the canvas paints the same base), and
 *   the drawing fades in once.
 * - The sky (sun or moon, stars) only moves during a theme change, so it is
 *   computed once per size and phase. At rest only the water rows and the
 *   whale's box are recomputed, at most 10 times a second (`FRAME_MS`),
 *   and only while the band is on screen and the tab is visible; a theme
 *   change while it is hidden jumps to its end.
 * - The dither is computed one pixel per 3 px block into a small canvas and
 *   scaled up without smoothing; the glyphs are drawn one colour at a time.
 * - Under `prefers-reduced-motion` it is drawn once and nothing moves (the
 *   whale at rest: no bob, eye open, no spout), and the fade-in is skipped.
 */
import * as React from "react"
import { useReducedMotion } from "motion/react"
import { useTheme } from "next-themes"

import { cn } from "@/lib/utils"

export type SceneVariant = "dither" | "ascii"

/** The waterline, 0 (top) to 1 (bottom), and the whale at the centre on it (mockup B). */
export const HZ = 0.4
const WHALE = { x: 0.5, r: 0.2 }
/** The sun on the left, the moon on the right (owner 2026-11-14); radius as a share of the height. */
const SUN = { x: 0.2, r: 0.07 }
const MOON = { x: 0.8, r: 0.065 }
/** Where a body stands when it is up, and where it is set (its top below the waterline). */
const UP = 0.17
const DOWN = HZ + 0.12

const ASCII_RAMP = " .,:-=+*#%@"
const BAYER = [0, 8, 2, 10, 12, 4, 14, 6, 3, 11, 1, 9, 15, 7, 13, 5].map((v) => (v + 0.5) / 16)
/** 100 ms: at rest the water moves at 10 frames a second at most. */
const FRAME_MS = 100
/** A theme change: 1.5 s, at up to 30 frames a second while it runs. */
const TRANSITION_MS = 1500
const TRANSITION_FRAME_MS = 33
/** The still picture's moment, so a reduced-motion scene is the same one every time. */
const STILL_T = 6

/**
 * Inks per phase and region: 0 sky, 1 whale, 2 water, 3 moonlight on the
 * water (and the eye), 4 the moon, 5 stars, 6 the sun, 7 sunlight on the
 * water, 8 the spout's spray (WEB-17 T11: the water's blue by day, the
 * moonlight's cream by night, so it adds no colour of its own).
 */
const INKS = {
  day: ["#4a6fb0", "#050A30", "#2b78cc", "#2b78cc", "#2b78cc", "#4a6fb0", "#e08a1e", "#e08a1e", "#2b78cc"],
  night: ["#3f68ad", "#8fcaff", "#44A6FA", "#f4efe3", "#f4efe3", "#9fb4d6", "#f2b04a", "#f2b04a", "#f4efe3"],
} as const

/**
 * The band's solid base, the colour shown before (and under) the drawing:
 * the page white of the day print and the deep navy of the night, not theme
 * tokens, since the scene is a picture in the brand's inks. The canvas
 * paints exactly these at rest, so the first paint never flashes.
 */
const SCENE_BASE = "bg-[#ffffff] dark:bg-[#061230]"
/**
 * WEB-17 T10 (owner QA 2026-11-15): the day base is the light page background
 * (`--background`, oklch(1 0 0), pinned in the test), so the band has no tint
 * of its own and nothing fades into a yellow. The night base is the deep navy.
 */
export const BASE = { day: "#ffffff", night: "#061230" } as const

/**
 * From the scene's light to ink. At night light is drawn as dots; by day the
 * same light is printed on the page white: a pale sky, the whale in dense navy, the
 * water in lake blue, the sun in warm ink, its light on the water as gaps
 * and a few warm glints.
 */
function toInk(L: number, region: number, night: boolean): number {
  if (night) return region === 5 ? 0.5 : L
  switch (region) {
    case 0:
      return (1 - L) * 0.36
    case 1:
      return 1 - L
    case 2:
      return (1 - L) * 0.85
    case 4:
      return 0.55
    case 6:
      return 0.6 + 0.4 * L
    case 7:
      return L > 0.9 ? 0.55 : 0
    case 8:
      // Denser than the light sky's dots around it, so the spray reads as a plume by day too.
      return Math.min(1, L * 1.3)
    default:
      return 0
  }
}

function hash(i: number, j: number): number {
  const s = Math.sin(i * 127.1 + j * 311.7) * 43758.5453
  return s - Math.floor(s)
}

/** The sun's and the moon's height at a phase (0 day, 1 night). */
export function bodies(phase: number): { sunY: number; moonY: number } {
  return { sunY: UP + (DOWN - UP) * phase, moonY: DOWN - (DOWN - UP) * phase }
}

/** How much of a body's light reaches the water: 1 high up, 0 once it is down. */
function above(y: number): number {
  return Math.max(0, Math.min(1, (HZ - y) / (HZ - UP)))
}

/**
 * What the whale is doing at one moment (WEB-17 T11): `bob` moves the head
 * down (positive) or up, in the scene's height units; `eye` is 1 open and 0
 * shut; `spout` is the spout's progress, 0 to 1, or −1 when there is none.
 */
export type WhalePose = { readonly bob: number; readonly eye: number; readonly spout: number }
/** The whale at rest: the still picture, reduced motion and the previews. */
export const REST: WhalePose = { bob: 0, eye: 1, spout: -1 }
/** A slow bob with the swell: about 3.6 px each way on the 400 px band, every 4.6 s. */
const BOB = { amp: 0.009, period: 4.6 }
/**
 * Occasional events, each at a jittered spot in its own slot (`every` ± `jitter`), so two in a
 * row are `every` ± 2 × `jitter` apart: a blink (eye shut 120 ms) every 6 to 12 s, a spout
 * (1.5 s) every 20 to 40 s. Seeded, so a test and a screenshot can know when they come.
 */
export const BLINK = { every: 9, jitter: 1.5, dur: 0.12, salt: 17 } as const
export const SPOUT = { every: 30, jitter: 5, dur: 1.5, salt: 53 } as const
type Schedule = typeof BLINK | typeof SPOUT

/** The progress (0 to 1) of the schedule's event running at `t`, or −1. Constant time: it looks only at the nearby slots. */
export function eventAt(t: number, e: Schedule, seed = 0): number {
  const n = Math.floor(t / e.every)
  for (let k = n - 1; k <= n + 1; k++) {
    const start = e.every * (k + 0.5) + e.jitter * (2 * hash(k + seed * 7.31, e.salt) - 1)
    if (t >= start && t < start + e.dur) return (t - start) / e.dur
  }
  return -1
}

/** The whale's pose at `t` (seconds). Under reduced motion the scene uses `REST` instead (`poseFor`). */
export function whalePose(t: number, seed = 0): WhalePose {
  return {
    bob: BOB.amp * Math.sin((2 * Math.PI * t) / BOB.period),
    eye: eventAt(t, BLINK, seed) >= 0 ? 0 : 1,
    spout: eventAt(t, SPOUT, seed),
  }
}

/** The pose the scene draws: none of the whale's motion when motion is reduced or the picture is still. */
export function poseFor(t: number, still: boolean, seed = 0): WhalePose {
  return still ? REST : whalePose(t, seed)
}

/**
 * Where the water meets the head at `x`: it rides the swell with the head (most of the bob, so a
 * little more of the head shows at the top of a bob) and follows the ripples, so it is never a
 * straight edge (DESIGN.md §2). About ±5.6 px on the 400 px band.
 */
export function waterlineAt(x: number, t: number, pose: WhalePose): number {
  return (
    HZ +
    0.6 * pose.bob +
    0.01 * Math.sin(x * 30 + Math.sin(x * 4 + t * 0.5) * 0.9 + t * 0.9) +
    0.004 * Math.sin(x * 67 - t * 1.7)
  )
}

/** Hermite step, 0 at `a`, 1 at `b`. */
const smooth = (a: number, b: number, v: number) => {
  const k = Math.max(0, Math.min(1, (v - a) / (b - a)))
  return k * k * (3 - 2 * k)
}

/**
 * The head's light at a point inside it (offsets from its centre, and the height above the
 * waterline): a sphere lit from the sun's side (upper left) by day and the moon's (upper right) by
 * night, the light swinging over the top during a theme change. Diffuse light with a soft core
 * shadow on the far side, a brighter rim on the near side, and a wet shadow that darkens the last
 * few pixels above the water.
 */
function headLight(dx: number, dy: number, aboveWater: number, phase: number): number {
  const R = WHALE.r
  const nx = dx / R
  const ny = dy / R
  const nz = Math.sqrt(Math.max(0, 1 - nx * nx - ny * ny))
  const lx0 = -0.62 + 1.24 * phase
  const ly0 = -0.5
  const lz0 = 0.6
  const ll = Math.hypot(lx0, ly0, lz0)
  const diffuse = Math.max(0, (nx * lx0 + ny * ly0 + nz * lz0) / ll)
  const side = Math.max(0, (nx * lx0 + ny * ly0) / Math.hypot(lx0, ly0))
  const rim = (1 - nz) ** 2 * side
  const wet = 0.3 + 0.7 * smooth(0, 0.035, aboveWater)
  return (0.1 + 0.38 * diffuse + 0.45 * rim) * wet
}

/**
 * The spout's spray at `ox` across and `hy` above the blowhole, at progress `p` of its 1.5 s: a
 * narrow stem that shoots up (ease-out), a crown that opens wider as it falls back and fades. Its
 * droplets sparkle: the density changes as the spray moves.
 */
function spray(ox: number, hy: number, p: number, i: number, j: number): number {
  if (p < 0 || p > 1 || hy < 0) return 0
  const R = WHALE.r
  const H = 0.34 * R
  const up = 1 - (1 - Math.min(1, p / 0.4)) ** 3
  const fall = Math.max(0, (p - 0.4) / 0.6)
  const top = H * up - 0.35 * H * fall * fall
  let d = 0
  if (hy < top) {
    const w = 0.035 * R + 0.12 * R * (hy / H) ** 2
    d = Math.exp(-((ox / w) ** 2)) * 0.75 * (1 - fall)
  }
  const rc = (0.08 + 0.2 * p) * R
  const cy = hy - top
  d = Math.max(d, Math.exp(-((ox / (1.7 * rc)) ** 2) - (cy / (0.75 * rc)) ** 2) * 0.85 * up * (1 - fall ** 1.5))
  return d * (0.5 + 0.5 * hash(i + Math.floor(p * 15) * 31, j))
}

/** The whale's box as shares of the band, with room for the bob and the spout: only it is redrawn while the whale moves. */
export function whaleBox(aspect: number): { u0: number; u1: number; v0: number } {
  const R = WHALE.r
  const half = (1.05 * R) / aspect
  return { u0: WHALE.x - half, u1: WHALE.x + half, v0: HZ - 1.265 * R - BOB.amp - 0.42 * R - 0.01 }
}

/** The scene's light at one point (u, v in 0–1) at a phase, and which ink it belongs to. */
export function field(
  u: number,
  v: number,
  aspect: number,
  t: number,
  phase: number,
  i: number,
  j: number,
  pose: WhalePose = REST,
): [number, number] {
  const x = (u - 0.5) * aspect
  const y = v
  const { sunY, moonY } = bodies(phase)
  const sx0 = (SUN.x - 0.5) * aspect
  const mx0 = (MOON.x - 0.5) * aspect
  const R = WHALE.r
  const wx = (WHALE.x - 0.5) * aspect
  // The mark's circle, cut a quarter radius below its centre by the water; it bobs with the swell.
  const cy = HZ - 0.265 * R + pose.bob
  const near = Math.abs(x - wx) < R
  // The waterline stays within ±0.02 of HZ; farther up or down, HZ answers "above or below it" the same.
  const yw = near && Math.abs(y - HZ) < 0.03 ? waterlineAt(x, t, pose) : HZ

  if (y < yw) {
    const dx = x - wx
    const dy = y - cy
    if (dx * dx + dy * dy < R * R) {
      // The eye: plain and flat. A blink shuts it to a slit at its lower lid.
      const er = 0.199 * R
      const ry = er * Math.max(0.2, pose.eye)
      const ex = (x - (wx + 0.342 * R)) / er
      const ey = (y - (cy - 0.138 * R + (er - ry) * 0.6)) / ry
      // Above the highest Bayer threshold (15.5/16), so the dither draws it solid: a dotted eye
      // would read as a pupil. Not a whole 1: V8 would make this array's elements integers, and
      // every other return here is a float pair.
      if (ex * ex + ey * ey < 1) return [0.99, 3]
      return [headLight(dx, dy, yw - y, phase), 1]
    }
  }

  if (y < HZ && (y < yw || !near)) {
    // The spout, from a blowhole just behind the top of the head.
    if (pose.spout >= 0) {
      const bx = wx - 0.14 * R
      const d = spray(x - bx, cy - 0.99 * R - y, pose.spout, i, j)
      if (d > 0.06) return [d, 8]
    }
    let L = 0.12 + 0.46 * Math.pow(y / HZ, 1.6)
    // The bodies are drawn only above the waterline: one that sets sinks behind it.
    const sd = Math.hypot(x - sx0, y - sunY)
    if (sd < SUN.r) return [0.8 + 0.2 * (1 - sd / SUN.r), 6]
    const md = Math.hypot(x - mx0, y - moonY)
    if (md < MOON.r) return [0.78 + 0.22 * (1 - md / MOON.r), 4]
    L += 0.32 * above(sunY) * Math.exp(-((sd - SUN.r) ** 2) / 0.012)
    L += 0.3 * above(moonY) * Math.exp(-((md - MOON.r) ** 2) / 0.01)
    if (y < HZ * 0.8 && hash(i, j) > 0.992) return [0.5, 5]
    return [Math.min(1, L), 0]
  }

  // Water, also where a ripple's crest climbs the head above the mean waterline.
  const dy = Math.max(0, y - HZ)
  const depth = dy / Math.max(0.0001, 1 - HZ)
  let L = 0.52 - 0.32 * depth
  // Ripples: lines that crowd towards the horizon, broken into dashes along x.
  const wave = Math.sin(5.5 / (dy + 0.045) + Math.sin(x * 4 + t * 0.5) * 0.9 + t * 0.9)
  const broken = 0.5 + 0.5 * Math.sin(x * 9 + dy * 23 - t * 0.7)
  const ripple = wave * broken
  L += 0.12 * ripple * (1 - depth * 0.5)
  let region = 2

  // Rings spreading from the whale, the lake's depth contours.
  const fx = (x - wx) / 1.7
  const fy = dy * 2.4
  const c = Math.sqrt(fx * fx + fy * fy) * 9 - t * 0.12
  if (c - Math.floor(c) < 0.07) L += 0.09
  // Its reflection about the waterline it rides, broken by the ripples, and more so with depth:
  // it wavers wider and thins out instead of ending on a straight edge.
  const mean = HZ + 0.6 * pose.bob
  const deep = Math.max(0, (y - mean) / (1.3 * R))
  const rx = x + Math.sin(y * 70 + t * 1.4) * (0.008 + 0.03 * deep)
  const ry = mean - (y - mean) * 0.9
  const ddx = rx - wx
  const ddy = ry - cy
  if (ddx * ddx + ddy * ddy < R * R && ripple < 0.55 - 1.2 * deep) L *= 0.38 + 0.4 * deep
  // Where the water meets the head: a thin glint under the waterline, broken along it, fading at
  // the head's sides (the mark's white gap, now wet and moving).
  if (near) {
    const m = (y - yw) / 0.014
    const across = Math.abs(x - wx) / (0.96 * R)
    if (m >= 0 && m < 1 && across < 1) {
      L += 0.3 * (1 - m) * (1 - across ** 4) * (0.55 + 0.45 * Math.sin(x * 90 - t * 2.2))
    }
  }

  // A path of light under each body, fading as it sets.
  const width = 0.02 + dy * 0.3
  for (const [bx, by, lit] of [[sx0, sunY, 7], [mx0, moonY, 3]] as const) {
    const k = above(by)
    if (k <= 0) continue
    const bdx = Math.abs(x - bx + Math.sin(y * 55 + t) * 0.01)
    if (bdx < width) {
      const glint = 0.6 * k * (1 - bdx / width) * (0.45 + 0.55 * ripple) * Math.sqrt(1 - depth)
      L += glint
      if (L > 0.8) region = lit
    }
  }
  return [Math.max(0, Math.min(1, L)), region]
}

/**
 * WEB-17 T9/T10, owner 2026-11-15 ("dibuat lebih redup aja, sewajarnya background aja",
 * then "jadi ga keliatan"): every ink moves towards the band's base by one factor
 * per style and phase. Calculated from the colours (WCAG relative luminance, see
 * the test): the most contrasting ink is between 2.5:1 and 3.0:1 against its
 * base at rest, ASCII (thin glyphs) near 3.0 and Dither near 2.5, and the
 * greeting's text keeps at least 4.5:1 against every ink and the base.
 */
export const DIM = {
  dither: { day: 0.62, night: 0.69 },
  ascii: { day: 0.57, night: 0.65 },
} as const

const subscribeNothing = () => () => {}
/** False on the server and the hydration pass, true after: the theme is unknown before. */
function useMounted(): boolean {
  return React.useSyncExternalStore(subscribeNothing, () => true, () => false)
}

const rgbOf = (hex: string) => [1, 3, 5].map((k) => parseInt(hex.slice(k, k + 2), 16))
const hexOf = (c: readonly number[]) => `#${c.map((x) => x.toString(16).padStart(2, "0")).join("")}`
const mix = (a: readonly number[], b: readonly number[], k: number) => a.map((x, n) => Math.round(x + (b[n] - x) * k))
const DAY = INKS.day.map(rgbOf)
const NIGHT = INKS.night.map(rgbOf)
const BASE_DAY = rgbOf(BASE.day)
const BASE_NIGHT = rgbOf(BASE.night)

/** The scene's inks (dimmed) and its base at a phase, 0 day to 1 night. Every phase between blends the two ends. */
export function sceneInks(phase: number, style: SceneVariant = "dither"): { inks: number[][]; base: number[] } {
  const base = mix(BASE_DAY, BASE_NIGHT, phase)
  const k = DIM[style].day + (DIM[style].night - DIM[style].day) * phase
  return { inks: DAY.map((d, r) => mix(mix(d, NIGHT[r], phase), base, k)), base }
}
/** Ease in and out (cubic): a setting sun starts and ends slowly. */
const easeInOut = (k: number) => (k < 0.5 ? 4 * k * k * k : 1 - (-2 * k + 2) ** 3 / 2)

export function LakeScene({
  variant,
  still = false,
  className,
}: {
  variant: SceneVariant
  /** Draw once and never move (the Background menu's previews). */
  still?: boolean
  className?: string
}) {
  const canvasRef = React.useRef<HTMLCanvasElement>(null)
  const reduce = (useReducedMotion() ?? false) || still
  const mounted = useMounted()
  const { resolvedTheme } = useTheme()
  const night = mounted && resolvedTheme === "dark"
  const [shown, setShown] = React.useState(false)
  // The phase the theme asks for, and the running scene's way to go there.
  const target = React.useRef(night ? 1 : 0)
  const goTo = React.useRef<((phase: number) => void) | null>(null)
  React.useEffect(() => {
    target.current = night ? 1 : 0
    goTo.current?.(target.current)
  }, [night])

  React.useEffect(() => {
    const canvas = canvasRef.current
    const ctx = canvas?.getContext("2d")
    if (!canvas || !ctx || !mounted) return
    const dither = variant === "dither"
    const still = reduce
    const cw = dither ? 3 : 6
    const ch = dither ? 3 : 10
    let w = 0
    let h = 0
    let cols = 0
    let rows = 0
    /** The first row at or below the waterline: everything above it is still at rest. */
    let water = 0
    /** The whale's box above the water (rows `wr0` to `water`, columns `wc0` to `wc1`): redrawn while it moves. */
    let wr0 = 0
    let wc0 = 0
    let wc1 = 0
    /** The whale's pose for the rows being computed. */
    let pose = REST
    // Dither: one pixel per block. ASCII: the sky, drawn once per phase.
    const small = document.createElement("canvas")
    const smallCtx = small.getContext("2d")
    let image: ImageData | null = null
    const sky = document.createElement("canvas")
    const skyCtx = sky.getContext("2d")
    let dpr = 1
    /** 0 day, 1 night: where the sun and the moon are, and how every ink is blended. */
    let phase = target.current
    /** Each ink for the current phase, dimmed, and the base colour. */
    let inks: number[][] = []
    let base = BASE_DAY
    const paint = () => {
      ;({ inks, base } = sceneInks(phase, variant))
    }
    paint()

    /** Ink and region at one cell. */
    const inkAt = (i: number, j: number, t: number): [number, number] => {
      const u = (i + 0.5) / cols
      const v = (j + 0.5) / rows
      const [L, region] = field(u, v, w / Math.max(1, h), t, phase, i, j, pose)
      const raw = phase <= 0 ? toInk(L, region, false) : phase >= 1 ? toInk(L, region, true) : toInk(L, region, false) * (1 - phase) + toInk(L, region, true) * phase
      let ink = raw
      // Thin out towards the bottom edge, where the page takes over.
      if (v > 0.8) ink *= Math.max(0, 1 - (v - 0.8) / 0.2)
      return [ink, region]
    }

    /** Dither rows into the small image: a dot in its ink, otherwise the base (the canvas is opaque). */
    const ditherRows = (from: number, to: number, t: number, c0: number, c1: number) => {
      if (!image) return
      const px = image.data
      for (let j = from; j < to; j++) {
        for (let i = c0; i < c1; i++) {
          const [ink, region] = inkAt(i, j, t)
          const o = (j * cols + i) * 4
          const c = ink > BAYER[(j % 4) * 4 + (i % 4)] ? inks[region] : base
          px[o] = c[0]
          px[o + 1] = c[1]
          px[o + 2] = c[2]
          px[o + 3] = 255
        }
      }
    }

    /** Glyph rows; `c0`–`c1` limits the columns, and `skipWhale` leaves the whale's box to `draw`. */
    const glyphRows = (target2d: CanvasRenderingContext2D, from: number, to: number, t: number, c0 = 0, c1 = cols, skipWhale = false) => {
      // One bucket per ink, so each is drawn in one colour.
      const buckets: number[][] = Array.from({ length: DAY.length }, () => [])
      for (let j = from; j < to; j++) {
        for (let i = c0; i < c1; i++) {
          if (skipWhale && j >= wr0 && i >= wc0 && i < wc1) continue
          const [raw, region] = inkAt(i, j, t)
          // The head is filled with the ramp from its lightest step, so its dark side reads as a
          // solid body, not an outline (WEB-17 T11). Here, not in `inkAt`, which must stay small
          // enough for the dither loop to inline it (measured: +3 points of main thread otherwise).
          const ink = region === 1 ? 0.18 + 0.82 * raw : raw
          const level = Math.min(ASCII_RAMP.length - 1, Math.floor(ink * ASCII_RAMP.length))
          if (level > 0) buckets[region].push(i, j, level)
        }
      }
      // A system monospace face: these are glyphs drawn as a picture, not text to read.
      target2d.font = "9.5px ui-monospace, SFMono-Regular, Menlo, monospace"
      target2d.textAlign = "center"
      target2d.textBaseline = "middle"
      buckets.forEach((cells, b) => {
        if (cells.length === 0) return
        target2d.fillStyle = hexOf(inks[b])
        for (let k = 0; k < cells.length; k += 3) {
          target2d.fillText(ASCII_RAMP[cells[k + 2]], cells[k] * cw + cw / 2, cells[k + 1] * ch + ch / 2)
        }
      })
    }

    /** The sky for the current phase: rows above the waterline (dither) or the cached layer (ASCII). */
    const drawSky = () => {
      paint()
      pose = REST
      if (dither) {
        ditherRows(0, water, STILL_T, 0, cols)
      } else if (skyCtx) {
        skyCtx.setTransform(1, 0, 0, 1, 0, 0)
        skyCtx.fillStyle = hexOf(base)
        skyCtx.fillRect(0, 0, sky.width, sky.height)
        skyCtx.setTransform(dpr, 0, 0, dpr, 0, 0)
        glyphRows(skyCtx, 0, water, STILL_T, 0, cols, true)
      }
    }

    /** Size everything and compute the sky. */
    const resize = () => {
      dpr = Math.min(2, window.devicePixelRatio || 1)
      // The layout size, not the drawn one: a preview is drawn large and
      // scaled down with a transform, which `getBoundingClientRect` includes.
      w = canvas.clientWidth
      h = canvas.clientHeight
      canvas.width = Math.max(1, Math.round(w * dpr))
      canvas.height = Math.max(1, Math.round(h * dpr))
      cols = Math.max(1, Math.ceil(w / cw))
      rows = Math.max(1, Math.ceil(h / ch))
      water = Math.min(rows, Math.max(0, Math.floor(HZ * rows - 0.5)))
      const box = whaleBox(w / Math.max(1, h))
      wr0 = Math.min(water, Math.max(0, Math.floor(box.v0 * rows)))
      wc0 = Math.max(0, Math.floor(box.u0 * cols))
      wc1 = Math.min(cols, Math.ceil(box.u1 * cols))
      if (dither && smallCtx) {
        small.width = cols
        small.height = rows
        image = smallCtx.createImageData(cols, rows)
      } else if (skyCtx) {
        sky.width = canvas.width
        sky.height = canvas.height
      }
      drawSky()
    }

    /** Recompute the whale and the water at moment `t` (seconds) and show the whole picture. */
    const draw = (t: number) => {
      pose = poseFor(t, still)
      ctx.setTransform(1, 0, 0, 1, 0, 0)
      if (dither && image && smallCtx) {
        ditherRows(wr0, water, t, wc0, wc1)
        ditherRows(water, rows, t, 0, cols)
        smallCtx.putImageData(image, 0, 0)
        ctx.imageSmoothingEnabled = false
        ctx.drawImage(small, 0, 0, cols * cw * dpr, rows * ch * dpr)
        return
      }
      ctx.drawImage(sky, 0, 0)
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0)
      glyphRows(ctx, wr0, water, t, wc0, wc1)
      glyphRows(ctx, water, rows, t)
    }

    let frame = 0
    let last = 0
    let visible = true
    let started = false
    const now = () => (still ? STILL_T : STILL_T + performance.now() / 1000)
    /** A theme change in progress: from which phase to which, since when. */
    let turn: { from: number; to: number; t0: number } | null = null
    const jump = (to: number) => {
      turn = null
      phase = to
      drawSky()
      draw(now())
    }
    const loop = (time: number) => {
      frame = requestAnimationFrame(loop)
      if (turn) {
        // Hidden or off screen, a theme change is not worth drawing: go to its end.
        if (!visible || document.hidden) return jump(turn.to)
        if (time - last < TRANSITION_FRAME_MS) return
        last = time
        const k = Math.min(1, (time - turn.t0) / TRANSITION_MS)
        phase = turn.from + (turn.to - turn.from) * easeInOut(k)
        drawSky()
        draw(STILL_T + time / 1000)
        if (k >= 1) turn = null
        return
      }
      if (still || !visible || document.hidden || time - last < FRAME_MS) return
      last = time
      draw(STILL_T + time / 1000)
    }
    goTo.current = (to: number) => {
      if (!started || to === phase) return
      if (still || !visible || document.hidden) return jump(to)
      turn = { from: phase, to, t0: performance.now() }
      if (!frame) frame = requestAnimationFrame(loop)
    }
    const start = () => {
      started = true
      phase = target.current
      resize()
      draw(STILL_T)
      setShown(true)
      if (!still) frame = requestAnimationFrame(loop)
    }

    // After first paint, when the browser has a moment: the composer and the
    // widgets come first.
    const idle = window.requestIdleCallback
      ? window.requestIdleCallback(start, { timeout: 600 })
      : window.setTimeout(start, 120)

    const ro = new ResizeObserver(() => {
      // Only a real change of the canvas's box redraws: setting its pixel size
      // in `resize` must not feed back into another resize.
      if (!started || (canvas.clientWidth === w && canvas.clientHeight === h)) return
      resize()
      draw(now())
    })
    ro.observe(canvas)
    const io = new IntersectionObserver(([e]) => {
      visible = e?.isIntersecting ?? true
    })
    io.observe(canvas)

    return () => {
      goTo.current = null
      if (window.cancelIdleCallback) window.cancelIdleCallback(idle)
      else window.clearTimeout(idle)
      cancelAnimationFrame(frame)
      ro.disconnect()
      io.disconnect()
    }
  }, [variant, mounted, reduce])

  return (
    <canvas
      ref={canvasRef}
      aria-hidden
      data-slot="lake-scene"
      data-variant={variant}
      className={cn(
        "pointer-events-none block size-full opacity-0",
        shown && "opacity-100",
        !reduce && "transition-opacity duration-500 ease-out",
        className,
      )}
    />
  )
}

/**
 * The hero band without the drawing: its solid base colour and the soft fade
 * at the bottom into the page, so the band has no edge. Home's loading look
 * draws this alone (`home-skeleton.tsx`); `LakeBand` draws the scene in it.
 * Positioned by the caller (absolute, behind the greeting and composer);
 * its height sets the scene's size.
 */
export function LakeBandBase({
  className,
  children,
  "data-state": state,
}: {
  className?: string
  children?: React.ReactNode
  "data-state"?: "on" | "off"
}) {
  return (
    <div
      aria-hidden
      data-slot="lake-band"
      data-state={state}
      className={cn(
        "pointer-events-none [mask-image:linear-gradient(to_bottom,black_58%,transparent)]",
        SCENE_BASE,
        className,
      )}
    >
      {children}
    </div>
  )
}

/** The scene as Home's hero band: `LakeBandBase` with the drawing on top. */
export function LakeBand({
  variant,
  still,
  className,
  "data-state": state,
}: {
  variant: SceneVariant
  still?: boolean
  className?: string
  "data-state"?: "on" | "off"
}) {
  return (
    <LakeBandBase className={className} data-state={state}>
      <LakeScene variant={variant} still={still} />
    </LakeBandBase>
  )
}
