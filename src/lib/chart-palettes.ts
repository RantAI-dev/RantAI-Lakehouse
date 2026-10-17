import type { ChartColors } from "./chart-style"

/**
 * The palettes the console ships (`BI-17` part A, feature decision 6). `default` is the
 * indigo-led categorical palette every chart used before; the four others are new.
 * Every colour has a relative luminance between 0.13 and 0.30, which keeps at least 3:1
 * contrast (the graphics threshold) against both the light card (#ffffff) and the dark
 * card (#18181b); `chart-palettes.test.ts` checks it.
 */
export const DEFAULT_PALETTE = [
  "#6366f1", "#0ea5e9", "#10b981", "#f59e0b",
  "#ef4444", "#8b5cf6", "#14b8a6", "#ec4899",
  "#84cc16", "#f97316",
]

export const PALETTES: Record<string, readonly string[]> = {
  default: DEFAULT_PALETTE,
  bright: ["#4f75e6", "#1a83b8", "#148e5b", "#a17317", "#e33c3c", "#965ce8", "#148c78", "#e23189"],
  muted: ["#5a7daf", "#a2714e", "#428965", "#8d7b44", "#b46371", "#926bb8", "#43858c", "#b26097"],
  "sequential-blue": ["#5792ea", "#488ceb", "#3580e9", "#1e71e6", "#1867d7", "#1661ca"],
  "diverging-red-blue": ["#cb2424", "#d84949", "#d77474", "#747c8d", "#6592d2", "#357bdb", "#1765d1"],
}

/**
 * The colours a chart draws with: the chosen built-in palette, or the company palette when
 * `company` is chosen and one is set (2 or more colours); anything else is the default.
 * With no choice, a company palette that is set is the default of new charts too (decision 6),
 * which the builder applies when it creates a chart, so a saved chart with no choice keeps `default`.
 */
export function resolvePalette(colors: ChartColors | undefined, company: readonly string[] | undefined): readonly string[] {
  const id = colors?.palette
  if (id === "company") return company && company.length >= 2 ? company : DEFAULT_PALETTE
  return (id && PALETTES[id]) || DEFAULT_PALETTE
}

function channels(hex: string): [number, number, number] {
  return [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16)) as [number, number, number]
}

/** WCAG relative luminance of `#rrggbb`. */
export function luminance(hex: string): number {
  const [r, g, b] = channels(hex).map((c) => {
    const s = c / 255
    return s <= 0.03928 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4
  }) as [number, number, number]
  return 0.2126 * r + 0.7152 * g + 0.0722 * b
}

/** WCAG contrast ratio of two `#rrggbb` colours. */
export function contrast(a: string, b: string): number {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x) as [number, number]
  return (hi + 0.05) / (lo + 0.05)
}

/** `#rrggbb` stops sampled at `t` (0 to 1), linear in RGB. */
export function interpolate(stops: readonly string[], t: number): string {
  if (stops.length === 0) return "#000000"
  if (stops.length === 1) return stops[0]!
  const x = Math.min(1, Math.max(0, Number.isFinite(t) ? t : 0)) * (stops.length - 1)
  const i = Math.min(stops.length - 2, Math.floor(x))
  const f = x - i
  const a = channels(stops[i]!)
  const b = channels(stops[i + 1]!)
  const mix = a.map((c, k) => Math.round(c + (b[k]! - c) * f))
  return `#${mix.map((c) => c.toString(16).padStart(2, "0")).join("")}`
}
