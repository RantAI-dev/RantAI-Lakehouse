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
 * light theme is a day lake printed on cream: the sun high on the left, its
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
 * What it costs, and the rules that keep it cheap (spec "Scene cost"):
 * - It is sized to the hero band only, never the page.
 * - The first drawing waits for the browser to be idle after first paint,
 *   so it never delays the composer or the largest paint; until then the
 *   band is its solid base colour (the canvas paints the same base), and
 *   the drawing fades in once.
 * - The sky (sun or moon, stars, the whale above water) only moves during
 *   a theme change, so it is computed once per size and phase. At rest only
 *   the water rows are recomputed, at most 10 times a second (`FRAME_MS`),
 *   and only while the band is on screen and the tab is visible; a theme
 *   change while it is hidden jumps to its end.
 * - The dither is computed one pixel per 3 px block into a small canvas and
 *   scaled up without smoothing; the glyphs are drawn one colour at a time.
 * - Under `prefers-reduced-motion` it is drawn once and nothing moves, and
 *   the fade-in is skipped.
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
 * water (and the eye, and the mark's gap), 4 the moon, 5 stars, 6 the sun,
 * 7 sunlight on the water.
 */
const INKS = {
  day: ["#4a6fb0", "#050A30", "#2b78cc", "#2b78cc", "#2b78cc", "#4a6fb0", "#e08a1e", "#e08a1e"],
  night: ["#3f68ad", "#8fcaff", "#44A6FA", "#f4efe3", "#f4efe3", "#9fb4d6", "#f2b04a", "#f2b04a"],
} as const

/**
 * The band's solid base, the colour shown before (and under) the drawing:
 * the cream of the day print and the deep navy of the night, not theme
 * tokens, since the scene is a picture in the brand's inks. The canvas
 * paints exactly these at rest, so the first paint never flashes.
 */
const SCENE_BASE = "bg-[#f3eee2] dark:bg-[#061230]"
const BASE = { day: "#f3eee2", night: "#061230" } as const

/**
 * From the scene's light to ink. At night light is drawn as dots; by day the
 * same light is printed on cream: a pale sky, the whale in dense navy, the
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

/** The scene's light at one point (u, v in 0–1) at a phase, and which ink it belongs to. */
export function field(u: number, v: number, aspect: number, t: number, phase: number, i: number, j: number): [number, number] {
  const x = (u - 0.5) * aspect
  const y = v
  const { sunY, moonY } = bodies(phase)
  const sx0 = (SUN.x - 0.5) * aspect
  const mx0 = (MOON.x - 0.5) * aspect
  const R = WHALE.r
  const wx = (WHALE.x - 0.5) * aspect
  const cy = HZ - 0.265 * R

  if (y < HZ) {
    const dx = x - wx
    const dy = y - cy
    const d = Math.sqrt(dx * dx + dy * dy)
    if (d < R) {
      const ex = x - (wx + 0.342 * R)
      const ey = y - (cy - 0.138 * R)
      if (ex * ex + ey * ey < (0.199 * R) ** 2) return [0.92, 3]
      if (R - d < 0.022) return [0.5, 1]
      const shx = x - (wx - 0.38 * R)
      const shy = y - (cy - 0.5 * R)
      const sheen = Math.exp(-(shx * shx + shy * shy) / (0.16 * R * R))
      return [0.04 + 0.2 * sheen, 1]
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

  // The mark's white gap between the dome and its blue band.
  if (y < HZ + 0.03 && Math.abs(x - wx) < R * 0.96) return [0.92, 3]

  const dy = y - HZ
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
  // Its reflection, broken by the ripples.
  const rx = x + Math.sin(y * 70 + t * 1.4) * 0.012
  const ry = HZ - dy * 0.9
  const ddx = rx - wx
  const ddy = ry - cy
  if (ddx * ddx + ddy * ddy < R * R && dy < 1.1 * R && ripple < 0.55) L *= 0.38

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
 * WEB-17 T9, owner 2026-11-15 ("dibuat lebih redup aja, sewajarnya background aja"):
 * every ink moves towards the band's base by one factor per phase, so the whole
 * scene is quiet. Calculated from the colours (WCAG relative luminance, see the
 * test): at these factors no ink is more than 2.0:1 against its base, and the
 * greeting's text keeps at least 4.5:1 against every ink and the base.
 */
export const DIM = { day: 0.75, night: 0.8 } as const

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
export function sceneInks(phase: number): { inks: number[][]; base: number[] } {
  const base = mix(BASE_DAY, BASE_NIGHT, phase)
  const k = DIM.day + (DIM.night - DIM.day) * phase
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
    const cw = dither ? 3 : 6
    const ch = dither ? 3 : 10
    let w = 0
    let h = 0
    let cols = 0
    let rows = 0
    /** The first row at or below the waterline: everything above it is still at rest. */
    let water = 0
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
      ;({ inks, base } = sceneInks(phase))
    }
    paint()

    /** Ink and region at one cell. */
    const inkAt = (i: number, j: number, t: number): [number, number] => {
      const u = (i + 0.5) / cols
      const v = (j + 0.5) / rows
      const [L, region] = field(u, v, w / Math.max(1, h), t, phase, i, j)
      const raw = phase <= 0 ? toInk(L, region, false) : phase >= 1 ? toInk(L, region, true) : toInk(L, region, false) * (1 - phase) + toInk(L, region, true) * phase
      let ink = raw
      // Thin out towards the bottom edge, where the page takes over.
      if (v > 0.8) ink *= Math.max(0, 1 - (v - 0.8) / 0.2)
      return [ink, region]
    }

    /** Dither rows into the small image: a dot in its ink, otherwise the base (the canvas is opaque). */
    const ditherRows = (from: number, to: number, t: number) => {
      if (!image) return
      const px = image.data
      for (let j = from; j < to; j++) {
        for (let i = 0; i < cols; i++) {
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

    const glyphRows = (target2d: CanvasRenderingContext2D, from: number, to: number, t: number) => {
      // One bucket per ink, so each is drawn in one colour.
      const buckets: number[][] = Array.from({ length: DAY.length }, () => [])
      for (let j = from; j < to; j++) {
        for (let i = 0; i < cols; i++) {
          const [ink, region] = inkAt(i, j, t)
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
      if (dither) {
        ditherRows(0, water, STILL_T)
      } else if (skyCtx) {
        skyCtx.setTransform(1, 0, 0, 1, 0, 0)
        skyCtx.fillStyle = hexOf(base)
        skyCtx.fillRect(0, 0, sky.width, sky.height)
        skyCtx.setTransform(dpr, 0, 0, dpr, 0, 0)
        glyphRows(skyCtx, 0, water, STILL_T)
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

    /** Recompute the water at moment `t` (seconds) and show the whole picture. */
    const draw = (t: number) => {
      ctx.setTransform(1, 0, 0, 1, 0, 0)
      if (dither && image && smallCtx) {
        ditherRows(water, rows, t)
        smallCtx.putImageData(image, 0, 0)
        ctx.imageSmoothingEnabled = false
        ctx.drawImage(small, 0, 0, cols * cw * dpr, rows * ch * dpr)
        return
      }
      ctx.drawImage(sky, 0, 0)
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0)
      glyphRows(ctx, water, rows, t)
    }

    let frame = 0
    let last = 0
    let visible = true
    let started = false
    const still = reduce
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
