"use client"

/**
 * Home's lake at night, as print (`WEB-17` D1, mockup B): sky, moon, the
 * whale's head from the RantAI Lakehouse mark at the waterline, its broken
 * reflection, ripples and the depth rings spreading from it, drawn either as
 * an ordered 4×4 Bayer dither ("Dither", the default) or as text glyphs
 * ("ASCII") in the logo's two inks: navy and lake blue on cream in the light
 * theme, lake blue and cream on deep water in the dark one.
 *
 * Our own code, from the mark's own geometry (`RantAI Lakehouse *.svg`: a
 * circle cut a quarter radius below its centre, the eye at (+0.34 r,
 * −0.14 r)); ported from the owner-approved mockup
 * (`design/chart-builder-mockups`, `home-history/lake-art.tsx`). No
 * third-party code and no traced picture (public AGPL repo).
 *
 * One light field serves both themes: the dark theme draws `light` as ink,
 * the light theme prints `1 − light`, so day and night are the same picture.
 *
 * What it costs, and the rules that keep it cheap (spec "Scene cost"):
 * - It is sized to the hero band only, never the page.
 * - The first drawing waits for the browser to be idle after first paint,
 *   so it never delays the composer or the largest paint; until then the
 *   band is its solid base colour, and the drawing fades in once.
 * - The sky (moon, stars, the whale above water) never moves, so it is
 *   computed once per size and theme. Only the water rows are recomputed,
 *   at most 10 times a second (`FRAME_MS`), and only while the band is on
 *   screen and the tab is visible.
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

/** The whale at the centre on the waterline, the moon high on the left (mockup B). */
const SCENE = {
  horizon: 0.4,
  whale: { x: 0.5, r: 0.2 },
  moon: { x: 0.2, y: 0.17, r: 0.065 },
} as const

const ASCII_RAMP = " .,:-=+*#%@"
const BAYER = [0, 8, 2, 10, 12, 4, 14, 6, 3, 11, 1, 9, 15, 7, 13, 5].map((v) => (v + 0.5) / 16)
/** 100 ms: the water moves at 10 frames a second at most. */
const FRAME_MS = 100
/** The still picture's moment, so a reduced-motion scene is the same one every time. */
const STILL_T = 6

/**
 * Inks per theme and region: 0 sky, 1 whale, 2 water, 3 light on the water
 * (and the eye, and the mark's gap), 4 the moon, 5 stars.
 */
const INKS = {
  light: ["#4a6fb0", "#050A30", "#2b78cc", "#2b78cc", "#2b78cc", "#4a6fb0"],
  dark: ["#3f68ad", "#8fcaff", "#44A6FA", "#f4efe3", "#f4efe3", "#9fb4d6"],
} as const

/**
 * The band's solid base, the colour shown before (and under) the drawing:
 * the cream and the deep navy of the brand's print, not theme tokens, since
 * the scene is a picture in the brand's inks.
 */
const SCENE_BASE = "bg-[#f3eee2] dark:bg-[#061230]"

/**
 * From the scene's light to ink. At night (dark theme) light is drawn; by
 * day (light theme) the same scene is printed: a pale sky, the whale in
 * dense navy, the water in lake blue, and the moon a blue sun.
 */
function toInk(L: number, region: number, dark: boolean): number {
  if (dark) return region === 5 ? 0.5 : L
  switch (region) {
    case 0:
      return (1 - L) * 0.36
    case 1:
      return 1 - L
    case 2:
      return (1 - L) * 0.85
    case 4:
      return 0.55
    default:
      return 0
  }
}

function hash(i: number, j: number): number {
  const s = Math.sin(i * 127.1 + j * 311.7) * 43758.5453
  return s - Math.floor(s)
}

/** The scene's light at one point (u, v in 0–1), and which ink it belongs to. */
function field(u: number, v: number, aspect: number, t: number, i: number, j: number): [number, number] {
  const hz = SCENE.horizon
  const x = (u - 0.5) * aspect
  const y = v
  const moon = SCENE.moon
  const mx = (moon.x - 0.5) * aspect
  const R = SCENE.whale.r
  const wx = (SCENE.whale.x - 0.5) * aspect
  const cy = hz - 0.265 * R

  if (y < hz) {
    const dx = x - wx
    const dy = y - cy
    const d = Math.sqrt(dx * dx + dy * dy)
    if (d < R) {
      const ex = x - (wx + 0.342 * R)
      const ey = y - (cy - 0.138 * R)
      if (ex * ex + ey * ey < (0.199 * R) ** 2) return [0.92, 3]
      if (R - d < 0.022) return [0.5, 1]
      const sx = x - (wx - 0.38 * R)
      const sy = y - (cy - 0.5 * R)
      const sheen = Math.exp(-(sx * sx + sy * sy) / (0.16 * R * R))
      return [0.04 + 0.2 * sheen, 1]
    }
    let L = 0.12 + 0.46 * Math.pow(y / hz, 1.6)
    const mdx = x - mx
    const mdy = y - moon.y
    const md = Math.sqrt(mdx * mdx + mdy * mdy)
    if (md < moon.r) return [0.78 + 0.22 * (1 - md / moon.r), 4]
    L += 0.3 * Math.exp(-((md - moon.r) ** 2) / 0.01)
    if (y < hz * 0.8 && hash(i, j) > 0.992) return [0.5, 5]
    return [Math.min(1, L), 0]
  }

  // The mark's white gap between the dome and its blue band.
  if (y < hz + 0.03 && Math.abs(x - wx) < R * 0.96) return [0.92, 3]

  const dy = y - hz
  const depth = dy / Math.max(0.0001, 1 - hz)
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
  const ry = hz - dy * 0.9
  const ddx = rx - wx
  const ddy = ry - cy
  if (ddx * ddx + ddy * ddy < R * R && dy < 1.1 * R && ripple < 0.55) L *= 0.38

  const width = 0.02 + dy * 0.3
  const mdx = Math.abs(x - mx + Math.sin(y * 55 + t) * 0.01)
  if (mdx < width) {
    const glint = 0.6 * (1 - mdx / width) * (0.45 + 0.55 * ripple) * Math.sqrt(1 - depth)
    L += glint
    if (L > 0.8) region = 3
  }
  return [Math.max(0, Math.min(1, L)), region]
}

/**
 * A calm patch under the greeting, so the words never sit on busy ink:
 * centred where Home puts the greeting (its top padding is 43 % of the
 * band), wide enough for a long name. Below 560 px a long greeting takes two
 * lines, so the patch is taller and a little lower there.
 */
function clearing(u: number, v: number, w: number): number {
  const narrow = w < 560
  const rx = Math.min(0.48, Math.max(0.17, 270 / Math.max(1, w)))
  const dx = (u - 0.5) / rx
  const dy = (v - (narrow ? 0.53 : 0.48)) / (narrow ? 0.11 : 0.065)
  const d2 = dx * dx + dy * dy
  return d2 < 2 ? Math.min(1, Math.max(0, (d2 - 0.4) / 1.6)) : 1
}

const subscribeNothing = () => () => {}
/** False on the server and the hydration pass, true after: the theme is unknown before. */
function useMounted(): boolean {
  return React.useSyncExternalStore(subscribeNothing, () => true, () => false)
}

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
  const dark = mounted && resolvedTheme === "dark"
  const [shown, setShown] = React.useState(false)

  React.useEffect(() => {
    const canvas = canvasRef.current
    const ctx = canvas?.getContext("2d")
    if (!canvas || !ctx || !mounted) return
    const inks = dark ? INKS.dark : INKS.light
    const rgb = inks.map((hex) => [1, 3, 5].map((k) => parseInt(hex.slice(k, k + 2), 16)))
    const dither = variant === "dither"
    const cw = dither ? 3 : 6
    const ch = dither ? 3 : 10
    let w = 0
    let h = 0
    let cols = 0
    let rows = 0
    /** The first row at or below the waterline: everything above it is still. */
    let water = 0
    // Dither: one pixel per block. ASCII: the still sky, drawn once.
    const small = document.createElement("canvas")
    const smallCtx = small.getContext("2d")
    let image: ImageData | null = null
    const sky = document.createElement("canvas")
    const skyCtx = sky.getContext("2d")
    let dpr = 1

    const inkAt = (i: number, j: number, t: number): [number, number] => {
      const u = (i + 0.5) / cols
      const v = (j + 0.5) / rows
      const [L, region] = field(u, v, w / Math.max(1, h), t, i, j)
      let ink = toInk(L, region, dark) * clearing(u, v, w)
      // Thin out towards the bottom edge, where the page takes over.
      if (v > 0.8) ink *= Math.max(0, 1 - (v - 0.8) / 0.2)
      return [ink, region]
    }

    const ditherRows = (from: number, to: number, t: number) => {
      if (!image) return
      const px = image.data
      for (let j = from; j < to; j++) {
        for (let i = 0; i < cols; i++) {
          const [ink, region] = inkAt(i, j, t)
          const o = (j * cols + i) * 4
          if (ink > BAYER[(j % 4) * 4 + (i % 4)]) {
            const c = rgb[region]
            px[o] = c[0]
            px[o + 1] = c[1]
            px[o + 2] = c[2]
            px[o + 3] = 255
          } else {
            px[o + 3] = 0
          }
        }
      }
    }

    const glyphRows = (target: CanvasRenderingContext2D, from: number, to: number, t: number) => {
      const buckets: number[][] = inks.map(() => [])
      for (let j = from; j < to; j++) {
        for (let i = 0; i < cols; i++) {
          const [ink, region] = inkAt(i, j, t)
          const level = Math.min(ASCII_RAMP.length - 1, Math.floor(ink * ASCII_RAMP.length))
          if (level > 0) buckets[region].push(i, j, level)
        }
      }
      // A system monospace face: these are glyphs drawn as a picture, not text to read.
      target.font = "9.5px ui-monospace, SFMono-Regular, Menlo, monospace"
      target.textAlign = "center"
      target.textBaseline = "middle"
      buckets.forEach((cells, region) => {
        target.fillStyle = inks[region]
        for (let k = 0; k < cells.length; k += 3) {
          target.fillText(ASCII_RAMP[cells[k + 2]], cells[k] * cw + cw / 2, cells[k + 1] * ch + ch / 2)
        }
      })
    }

    /** Size everything and compute the still sky once. */
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
      water = Math.min(rows, Math.max(0, Math.floor(SCENE.horizon * rows - 0.5)))
      if (dither && smallCtx) {
        small.width = cols
        small.height = rows
        image = smallCtx.createImageData(cols, rows)
        ditherRows(0, water, 0)
      } else if (skyCtx) {
        sky.width = canvas.width
        sky.height = canvas.height
        skyCtx.setTransform(dpr, 0, 0, dpr, 0, 0)
        skyCtx.clearRect(0, 0, w, h)
        glyphRows(skyCtx, 0, water, 0)
      }
    }

    /** Recompute the water at moment `t` (seconds) and show the whole picture. */
    const draw = (t: number) => {
      ctx.setTransform(1, 0, 0, 1, 0, 0)
      ctx.clearRect(0, 0, canvas.width, canvas.height)
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
    const loop = (now: number) => {
      frame = requestAnimationFrame(loop)
      if (!visible || document.hidden || now - last < FRAME_MS) return
      last = now
      draw(STILL_T + now / 1000)
    }
    const start = () => {
      started = true
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
      // Only a real change of the box redraws: setting the canvas's pixel
      // size in `resize` must not feed back into another resize.
      if (!started || (canvas.clientWidth === w && canvas.clientHeight === h)) return
      resize()
      draw(still ? STILL_T : STILL_T + performance.now() / 1000)
    })
    ro.observe(canvas)
    const io = new IntersectionObserver(([e]) => {
      visible = e?.isIntersecting ?? true
    })
    io.observe(canvas)

    return () => {
      if (window.cancelIdleCallback) window.cancelIdleCallback(idle)
      else window.clearTimeout(idle)
      cancelAnimationFrame(frame)
      ro.disconnect()
      io.disconnect()
    }
  }, [variant, dark, mounted, reduce])

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
export function LakeBandBase({ className, children }: { className?: string; children?: React.ReactNode }) {
  return (
    <div
      aria-hidden
      data-slot="lake-band"
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
}: {
  variant: SceneVariant
  still?: boolean
  className?: string
}) {
  return (
    <LakeBandBase className={className}>
      <LakeScene variant={variant} still={still} />
    </LakeBandBase>
  )
}
