import { decimalsFor, formatNumber } from "@/lib/chart-axis"
import type { FilterDef } from "@/services/clients/bi-store"

/**
 * Number bins and map grids (`BI-10`), mirroring `lakehouse_bi::bin`. The
 * server cuts the bins and returns each row's **lower edge** as a number; the
 * tile carries the bin's width, so this module only names the shapes, builds
 * the payload, labels a bin ("500 – 1,000") and turns a clicked bin back into
 * the range it stands for. Pure, so each rule is testable without a browser.
 */

export type BinMode = "auto" | "count" | "width"
/** A bin as saved and sent: `count` and `width` may arrive as a number or a string. */
export type BinDef = { mode: BinMode; count?: number | string; width?: number | string }
/** The builder's state for one bin. */
export type BinDraft = { mode: BinMode; count: string; width: string }
/** The counts a bin may ask for. */
export const BIN_COUNTS = ["10", "50", "100"] as const

export const AUTO_BIN: BinDraft = { mode: "auto", count: "10", width: "" }

/** A saved bin as a draft; null when the chart has none. */
export function binDraftFromDef(def: BinDef | undefined | null): BinDraft | null {
  if (!def || !["auto", "count", "width"].includes(def.mode)) return null
  return {
    mode: def.mode,
    count: def.count !== undefined && def.count !== null ? String(def.count) : "10",
    width: def.width !== undefined && def.width !== null ? String(def.width) : "",
  }
}

/** The width as a number, or null when it is not a positive finite number. */
export function parseWidth(width: string): number | null {
  const n = Number(width.trim())
  return width.trim() !== "" && Number.isFinite(n) && n > 0 ? n : null
}

/** What is sent for a bin, or undefined for none; a width that is not a positive number is not sent (see {@link binProblem}). */
export function binPayload(d: BinDraft | null): BinDef | undefined {
  if (!d) return undefined
  if (d.mode === "auto") return { mode: "auto" }
  if (d.mode === "count") return { mode: "count", count: Number(d.count) }
  const width = parseWidth(d.width)
  return width === null ? undefined : { mode: "width", width }
}

/** What is still needed before the bin can be sent, in the builder's words; null when nothing is. */
export function binProblem(d: BinDraft | null, what: string): string | null {
  return d && d.mode === "width" && parseWidth(d.width) === null ? `${what} bin width` : null
}

/** The sentence a bin choice reads as in the picker. */
export const BIN_CHOICES: Record<string, string> = {
  none: "No bins",
  auto: "Automatic (about 10 bins)",
  "count:10": "10 bins",
  "count:50": "50 bins",
  "count:100": "100 bins",
  width: "A width of my own",
}

/** The picker value of a draft. */
export function binChoiceOf(d: BinDraft | null): string {
  if (!d) return "none"
  return d.mode === "count" ? `count:${d.count}` : d.mode
}

/** The draft a picker value stands for, keeping the width already typed. */
export function binDraftFromChoice(choice: string, previous: BinDraft | null): BinDraft | null {
  if (choice === "none") return null
  const width = previous?.width ?? ""
  if (choice === "auto") return { mode: "auto", count: "10", width }
  if (choice === "width") return { mode: "width", count: "10", width }
  const count = choice.startsWith("count:") ? choice.slice("count:".length) : "10"
  return { mode: "count", count, width }
}

/** Automatic and counted bins need a table; a SQL source takes a width only. */
export function binChoicesFor(sqlSource: boolean): string[] {
  return sqlSource ? ["none", "width"] : Object.keys(BIN_CHOICES)
}

// ── map grid ─────────────────────────────────────────────────────────────

/** The grid sizes in degrees, and automatic. */
export const GRID_SIZES = ["auto", "0.01", "0.1", "1", "10"] as const
export const GRID_LABELS: Record<string, string> = {
  none: "One mark per point",
  auto: "Grid, automatic size",
  "0.01": "Grid cells of 0.01°",
  "0.1": "Grid cells of 0.1°",
  "1": "Grid cells of 1°",
  "10": "Grid cells of 10°",
}
export type GridDef = { size: number | string }

export function gridChoiceOf(def: GridDef | undefined | null): string {
  if (!def) return "none"
  const size = String(def.size)
  return size === "auto" || (GRID_SIZES as readonly string[]).includes(size) ? size : "none"
}

export function gridPayload(choice: string): GridDef | undefined {
  if (choice === "none" || !(GRID_SIZES as readonly string[]).includes(choice)) return undefined
  return { size: choice === "auto" ? "auto" : Number(choice) }
}

// ── what comes back ──────────────────────────────────────────────────────

/** What a tile says of one binned column (`bins` on the payload). */
export type BinMeta = { mode?: string; width?: unknown; min?: unknown; max?: unknown }

/** A bin as the tile knows it: the width, and the data's maximum when the server said. */
export type BinInfo = { width: number; max?: number }

const finite = (v: unknown): number | null => {
  const n = typeof v === "string" ? Number(v) : v
  return typeof n === "number" && Number.isFinite(n) ? n : null
}

/**
 * The bins of a tile, by column: from what the server read over the same
 * filtered rows, or (a SQL source preview, where nothing is read) from the
 * width the chart itself fixes.
 */
export function binsOfTile(
  meta: Record<string, BinMeta> | undefined,
  def: { bin?: BinDef; breakdownBin?: BinDef; dimension?: string; breakdown?: string; groupings?: { column: string; bin?: BinDef }[] } | undefined,
): Record<string, BinInfo> {
  const out: Record<string, BinInfo> = {}
  for (const [column, m] of Object.entries(meta ?? {})) {
    const width = finite(m.width)
    if (width !== null && width > 0) {
      const max = finite(m.max)
      out[column] = max === null ? { width } : { width, max }
    }
  }
  const fixed: [string | undefined, BinDef | undefined][] = [
    [def?.dimension, def?.bin],
    [def?.breakdown, def?.breakdownBin],
    ...(def?.groupings ?? []).map((g): [string, BinDef | undefined] => [g.column, g.bin]),
  ]
  for (const [column, bin] of fixed) {
    if (!column || out[column] || bin?.mode !== "width") continue
    const width = finite(bin.width)
    if (width !== null && width > 0) out[column] = { width }
  }
  return out
}

/** "500 – 1,000": the lower edge to the upper, with as many decimals as the edges need. */
export function binLabel(lower: number, width: number): string {
  const upper = Math.round((lower + width) * 1e10) / 1e10
  const decimals = Math.max(decimalsFor([lower, upper]), decimalsOf(lower), decimalsOf(upper))
  return `${formatNumber(lower, decimals)} – ${formatNumber(upper, decimals)}`
}

function decimalsOf(v: number): number {
  const text = String(v)
  const dot = text.indexOf(".")
  return dot < 0 ? 0 : Math.min(6, text.length - dot - 1)
}

/** Rows with every binned column's value replaced by its label; a value that is not a number is left alone. */
export function relabelBins<R extends Record<string, unknown>>(rows: readonly R[], bins: Record<string, BinInfo>): R[] {
  const columns = Object.keys(bins)
  if (!columns.length) return [...rows]
  return rows.map((row) => {
    let next = row
    for (const column of columns) {
      const lower = finite(row[column])
      if (lower !== null && row[column] !== "" && row[column] !== null) {
        next = { ...next, [column]: binLabel(lower, bins[column].width) }
      }
    }
    return next
  })
}

/** Whether the bin starting at `lower` is the last one: it holds the data's maximum (the maximum is in the last bin, not in one of its own). */
export function isLastBin(lower: number, width: number, max: number | undefined): boolean {
  if (max === undefined) return false
  const upper = lower + width
  return max < upper || Math.abs(max - upper) < width * 1e-9
}

/** The clicked bin as a dashboard filter: the lower edge included, the upper excluded, except in the last bin, which includes the maximum. */
export function binRangeFilter(column: string, lower: number, width: number, last: boolean): FilterDef {
  const upper = Math.round((lower + width) * 1e10) / 1e10
  return { column, values: [], op: "between", min: String(lower), max: String(upper), ...(last ? {} : { maxExclusive: true }) }
}

/** The dashboard's filters after a click on a bin: that column holds the bin's range; a second click on the same bin removes it. */
export function toggleBinFilter(filters: FilterDef[], next: FilterDef): FilterDef[] {
  const current = filters.find((f) => f.column === next.column)
  const same =
    current !== undefined &&
    (current.op ?? "in") === "between" &&
    current.min === next.min &&
    current.max === next.max &&
    !!current.maxExclusive === !!next.maxExclusive &&
    !current.minExclusive
  const rest = filters.filter((f) => f.column !== next.column)
  return same ? rest : [...rest, next]
}
