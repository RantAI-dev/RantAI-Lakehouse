import type { QueryLimitValues, TileLimit } from "@/services/contracts/settings"

/**
 * The ranges and defaults of the query limits (`BI-5`, feature decisions 1
 * and 2), and the sentences a tile says about them (decision 5). The server
 * checks every range again and has the last word (`query_limits` CHECKs); this
 * only keeps the form from offering what would be refused.
 */
export const LIMIT_RANGES = {
  timeoutSeconds: { min: 10, max: 1800, label: "Time limit", unit: "seconds" },
  chartRows: { min: 100, max: 100_000, label: "Rows a chart reads", unit: "rows" },
  mapPoints: { min: 100, max: 50_000, label: "Points a map shows", unit: "points" },
  studioRows: { min: 100, max: 100_000, label: "Rows Query Studio returns", unit: "rows" },
} as const

export type LimitField = keyof typeof LIMIT_RANGES

/** What an install has before an administrator saves (feature decisions 1 and 2). */
export const LIMIT_DEFAULTS: QueryLimitValues = { timeoutSeconds: 60, chartRows: 2000, mapPoints: 5000, studioRows: 64000 }

/** A whole number inside the field's range, else a sentence that says the range; `null` when it is fine. */
export function limitProblem(field: LimitField, text: string): string | null {
  const r = LIMIT_RANGES[field]
  const n = Number(text)
  if (text.trim() === "" || !Number.isInteger(n) || n < r.min || n > r.max) {
    return `${r.label}: a whole number from ${r.min.toLocaleString("en-US")} to ${r.max.toLocaleString("en-US")} ${r.unit}.`
  }
  return null
}

/** 90 seconds is "90 seconds"; the limit is shown in the unit it is set in. */
function seconds(n: number): string {
  return `${n.toLocaleString("en-US")} ${n === 1 ? "second" : "seconds"}`
}

/** Where a limit came from, in the words of a tile: "for your role" or "for this install". */
function whose(limit: TileLimit): string {
  return limit.source.kind === "role" ? "the limit for your role" : "the limit for this install"
}

/**
 * The sentence a tile says when a limit ended it (feature decision 5):
 * "This chart stopped after 30 seconds, the limit for your role." for time,
 * "Showing the first 2,000 rows, the limit set by your administrator." for rows.
 */
export function limitSentence(limit: TileLimit): string {
  if (limit.kind === "time") return `This chart stopped after ${seconds(limit.value)}, ${whose(limit)}.`
  return `Showing the first ${limit.value.toLocaleString("en-US")} rows, the limit set by your administrator.`
}

/** What a tile says after the person pressed Stop (feature decision 5). */
export const STOPPED_SENTENCE = "Stopped"
