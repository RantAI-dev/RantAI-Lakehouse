import type { ChartKind } from "./dashboard-specs"

/**
 * Progress, histogram, bullet, image and web cards (`BI-16` part B): the
 * fields they add to a chart definition, what the builder holds while they are
 * edited, and the rules the tiles apply. Mirrors `lakehouse_bi::types_b` (flat,
 * camelCase on the wire); the server checks every limit and the allowed sites
 * at save, this module only names the shapes and keeps the form from offering
 * what would be refused.
 */

/** Ranges a histogram aims at when it is not told a width or a count (`AUTO_HISTOGRAM_BINS`). */
export const AUTO_HISTOGRAM_BINS = 20
export const MAX_HISTOGRAM_COUNT = 100
export const MAX_BANDS = 3
export const MAX_ALT = 200
export const MAX_URL = 2000
/** The most ranges the server returns for a histogram (`HISTOGRAM_LIMIT` in `builder.rs`); a full result may have been cut. */
export const HISTOGRAM_ROW_CAP = 100
/** The bullet graphs a tile draws; the rest are summarised ("N more not shown"). */
export const MAX_BULLETS = 20
/** The most ranges a histogram draws between its first and its last; more would be a stripe of zeros. */
export const MAX_DRAWN_RANGES = 500
/** The bars a Gantt chart draws (`BI-16` part C1, feature decision 1); the statement asks for one more to tell "more than this". */
export const MAX_GANTT_BARS = 1000

/** The sentence the server answers with, word for word (`CONTENT_SITE_REFUSED`). */
export const CONTENT_SITE_REFUSED =
  "Images and web pages may only come from the sites an administrator allows (Admin > Settings)."
/** What a card shows when its site left the list after it was saved. */
export const SITE_NO_LONGER_ALLOWED = "This site is no longer allowed"

/** A fixed number, or the chart's second measure (named by its column). */
export type Goal = number | { measure: string }
export type HistogramDef = { width?: number | string; count?: number | string }

export type TypeDefFields = {
  /** `progress`, `bullet`. */
  goal?: Goal
  /** `bullet`: 1 to 3 ascending upper limits. */
  bands?: number[]
  /** `histogram`: absent is automatic. */
  histogram?: HistogramDef
  /** `image`, `web`. */
  url?: string
  /** `image`. */
  alt?: string
  /**
   * `gantt` (`BI-16` part C1, plan D1): the columns a task starts and ends in
   * (dates or date-times, required) and an optional 0 to 100 progress column.
   * Flat on the chart, beside `lat` and `lon`. The task is the `dimension`, the
   * group the `breakdown`. The server (`lakehouse-bi` T1) does not know these
   * fields yet, so a chart of this kind is refused until it lands.
   */
  start?: string
  end?: string
  progress?: string
}

export function takesGoal(kind: string): boolean {
  return kind === "progress" || kind === "bullet"
}

export function isTypeBKind(kind: string): kind is Extract<ChartKind, "progress" | "histogram" | "bullet" | "image" | "web"> {
  return kind === "progress" || kind === "histogram" || kind === "bullet" || kind === "image" || kind === "web"
}

/** The Gantt chart (`BI-16` part C1): one bar per task from a start column to an end column. */
export function isGanttKind(kind: string): kind is Extract<ChartKind, "gantt"> {
  return kind === "gantt"
}

// ── the address ──────────────────────────────────────────────────────────

/**
 * The origin (`https://host[:port]`, in the server's normal form) of an
 * address, or `null` when it is not an `https` address with a plain host and no
 * user name. Mirrors `lakehouse_bi::embed_access::https_origin_of`: the server
 * decides at save, this decides at render, and both read an address the same
 * way (`https://allowed.example@evil.example/` is no address on `allowed.example`).
 */
export function httpsOrigin(url: string): string | null {
  const trimmed = url.trim()
  if (!trimmed.startsWith("https://")) return null
  const rest = trimmed.slice("https://".length)
  const authority = rest.split(/[/?#]/)[0] ?? ""
  if (authority.includes("@") || authority === "" || !/^[A-Za-z0-9.:-]+$/.test(authority)) return null
  const colon = authority.indexOf(":")
  const host = (colon < 0 ? authority : authority.slice(0, colon)).toLowerCase()
  const port = colon < 0 ? null : authority.slice(colon + 1)
  const labels = host.split(".")
  const hostOk =
    host.length > 0 && host.length <= 253 &&
    labels.every((l) => l !== "" && !l.startsWith("-") && !l.endsWith("-") && /^[a-z0-9-]+$/.test(l))
  if (!hostOk) return null
  if (port === null) return `https://${host}`
  if (!/^[0-9]+$/.test(port)) return null
  const n = Number(port)
  return n >= 1 && n <= 65535 ? `https://${host}:${n}` : null
}

/** Whether `url` is an `https` address on one of the administrator's content sites. */
export function siteAllowed(url: string | undefined, sites: readonly string[] | undefined): boolean {
  if (!url || url.length > MAX_URL) return false
  const origin = httpsOrigin(url)
  return origin !== null && (sites ?? []).includes(origin)
}

/** Why the builder will not save this address, or `null`; blank is "missing", not a problem here. */
export function urlProblem(url: string, sites: readonly string[] | undefined): string | null {
  const trimmed = url.trim()
  if (!trimmed) return null
  if (/\s/.test(trimmed) || /[\u0000-\u001f]/.test(trimmed)) return CONTENT_SITE_REFUSED
  return siteAllowed(trimmed, sites) ? null : CONTENT_SITE_REFUSED
}

// ── the draft ────────────────────────────────────────────────────────────

export type TypeDraft = {
  /** A fixed number, or the chart's second measure. */
  goalMode: "number" | "measure"
  goalNumber: string
  goalMeasure: string
  /** Up to three limits, as typed. */
  bands: string[]
  histMode: "auto" | "count" | "width"
  histCount: string
  histWidth: string
  url: string
  alt: string
  /** `gantt`: the start, end and progress columns (progress may stay empty). */
  ganttStart: string
  ganttEnd: string
  ganttProgress: string
}

export function emptyTypeDraft(): TypeDraft {
  return {
    goalMode: "number", goalNumber: "", goalMeasure: "", bands: [], histMode: "auto", histCount: "10", histWidth: "", url: "", alt: "",
    ganttStart: "", ganttEnd: "", ganttProgress: "",
  }
}

const textOf = (v: unknown): string => (v === undefined || v === null ? "" : String(v))

export function typeDraftFromDef(def: TypeDefFields | undefined): TypeDraft {
  const d = emptyTypeDraft()
  if (!def) return d
  if (typeof def.goal === "number") {
    d.goalNumber = String(def.goal)
  } else if (def.goal && typeof def.goal === "object") {
    d.goalMode = "measure"
    d.goalMeasure = def.goal.measure
  }
  d.bands = (def.bands ?? []).map(String)
  if (def.histogram?.width !== undefined) {
    d.histMode = "width"
    d.histWidth = textOf(def.histogram.width)
  } else if (def.histogram?.count !== undefined) {
    d.histMode = "count"
    d.histCount = textOf(def.histogram.count)
  }
  d.url = def.url ?? ""
  d.alt = def.alt ?? ""
  d.ganttStart = def.start ?? ""
  d.ganttEnd = def.end ?? ""
  d.ganttProgress = def.progress ?? ""
  return d
}

const finiteNumber = (s: string): number | null => {
  if (s.trim() === "") return null
  const n = Number(s)
  return Number.isFinite(n) ? n : null
}

/** The band limits typed so far: the filled ones, in order. */
function filledBands(d: TypeDraft): string[] {
  return d.bands.map((b) => b.trim()).filter((b) => b !== "")
}

/** The column of a goal that is a second value, which the builder appends to the measures. */
export function goalMeasureColumn(kind: string, d: TypeDraft): string | undefined {
  return takesGoal(kind) && d.goalMode === "measure" && d.goalMeasure ? d.goalMeasure : undefined
}

/** The fields to merge into the payload: only what the kind takes. */
export function typePayload(kind: string, d: TypeDraft): Record<string, unknown> {
  const out: Record<string, unknown> = {}
  if (takesGoal(kind)) {
    if (d.goalMode === "measure") {
      if (d.goalMeasure) out.goal = { measure: d.goalMeasure }
    } else {
      const goal = finiteNumber(d.goalNumber)
      if (goal !== null) out.goal = goal
    }
  }
  if (kind === "bullet") {
    const bands = filledBands(d).map(Number)
    if (bands.length) out.bands = bands
  }
  if (kind === "histogram") {
    if (d.histMode === "count") out.histogram = { count: Number(d.histCount) }
    else if (d.histMode === "width") out.histogram = { width: Number(d.histWidth) }
  }
  if (kind === "image") {
    out.url = d.url.trim()
    out.alt = d.alt.trim()
  }
  if (kind === "web") out.url = d.url.trim()
  if (kind === "gantt") {
    if (d.ganttStart) out.start = d.ganttStart
    if (d.ganttEnd) out.end = d.ganttEnd
    if (d.ganttProgress) out.progress = d.ganttProgress
  }
  return out
}

/** What is still needed, in the builder's own words (a field name, as `shapeProblems` does). */
export function typeProblems(kind: string, d: TypeDraft): string[] {
  const out: string[] = []
  if (kind === "progress" || kind === "bullet") {
    const word = kind === "progress" ? "Goal" : "Target"
    if (d.goalMode === "measure") {
      if (!d.goalMeasure) out.push(`${word} column`)
    } else if (d.goalNumber.trim() === "") {
      if (kind === "progress") out.push(`${word} number`)
    } else if (finiteNumber(d.goalNumber) === null) {
      out.push(`${word} number`)
    }
  }
  if (kind === "bullet") {
    const bands = filledBands(d)
    const numbers = bands.map(finiteNumber)
    if (numbers.some((n) => n === null) || numbers.some((n, i) => i > 0 && (n as number) <= (numbers[i - 1] as number))) {
      out.push("Band limits, rising")
    }
  }
  if (kind === "histogram") {
    if (d.histMode === "count") {
      const n = finiteNumber(d.histCount)
      if (n === null || !Number.isInteger(n) || n < 1 || n > MAX_HISTOGRAM_COUNT) out.push("Number of ranges")
    } else if (d.histMode === "width") {
      const n = finiteNumber(d.histWidth)
      if (n === null || n <= 0) out.push("Range width")
    }
  }
  if (kind === "gantt") {
    if (!d.ganttStart) out.push("Start column")
    if (!d.ganttEnd) out.push("End column")
  }
  if (kind === "image" || kind === "web") {
    if (d.url.trim() === "") out.push("Address")
    if (kind === "image") {
      if (d.alt.trim() === "") out.push("Description")
      else if (d.alt.trim().length > MAX_ALT) out.push("Shorter description")
    }
  }
  return out
}

// ── progress ─────────────────────────────────────────────────────────────

export type ProgressView =
  | { kind: "no-goal" }
  | { kind: "ok"; value: number; goal: number; ratio: number; fill: number }

/**
 * A value against its goal. A goal of zero or less (or missing) has nothing to
 * compare with (feature decision 1). Above the goal the bar is full and the
 * label keeps the real percentage (`ratio` 1.12 is "112%").
 */
export function progressView(value: number | null, goal: number | null): ProgressView {
  if (value === null || goal === null || !Number.isFinite(value) || !Number.isFinite(goal) || goal <= 0) return { kind: "no-goal" }
  const ratio = value / goal
  return { kind: "ok", value, goal, ratio, fill: Math.min(1, Math.max(0, ratio)) }
}

// ── bullet ───────────────────────────────────────────────────────────────

/** At most `MAX_BULLETS` rows, and how many more there were. */
export function cutBullets<R>(rows: readonly R[]): { shown: R[]; more: number } {
  return { shown: rows.slice(0, MAX_BULLETS), more: Math.max(0, rows.length - MAX_BULLETS) }
}

// ── histogram ────────────────────────────────────────────────────────────

const round10 = (v: number): number => Math.round(v * 1e10) / 1e10

export type FilledRange = { lower: number; count: number }

/**
 * The counted ranges with the empty ones between the first and the last drawn
 * as zero, so the shape is not distorted (feature decision 2). `null` when the
 * stretch from the first to the last range would need more than
 * `MAX_DRAWN_RANGES` bars: the caller says so instead of drawing a stripe.
 */
export function fillRanges(
  rows: readonly { lower: number; count: number }[],
  width: number,
): FilledRange[] | null {
  if (!(width > 0) || rows.length === 0) return rows.map((r) => ({ ...r }))
  const sorted = [...rows].sort((a, b) => a.lower - b.lower)
  const first = sorted[0].lower
  const last = sorted[sorted.length - 1].lower
  const steps = Math.round((last - first) / width)
  if (steps + 1 > MAX_DRAWN_RANGES) return null
  const byLower = new Map(sorted.map((r) => [round10(r.lower), r.count]))
  const out: FilledRange[] = []
  for (let i = 0; i <= steps; i++) {
    const lower = round10(first + i * width)
    out.push({ lower, count: byLower.get(lower) ?? 0 })
  }
  return out
}
