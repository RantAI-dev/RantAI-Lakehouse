/**
 * The freshness rule of remembered dashboard results (`BI-4` part B, feature
 * decision 2) and the words the tiles use for it. The server owns the range
 * (`lakehouse_bi::cache_rule`, 1 to 1440 minutes) and has the last word; this
 * mirrors it so a form can say the range before a save.
 */
export type CacheMode = "default" | "fresh" | "minutes"

/** `{mode: "default"}` follows the level above; `minutes` carries `minutes`. */
export type CacheRule = { mode: CacheMode | string; minutes?: number }

export const CACHE_MINUTES = { min: 1, max: 1440 } as const

/** The sentence the server answers for a rule out of range (kept word for word). */
export const MINUTES_SENTENCE = "Remember for between 1 minute and 24 hours (1 to 1440 minutes)."

export const MODE_LABELS: Record<CacheMode, string> = {
  default: "Follow the default",
  fresh: "Always fresh",
  minutes: "Remember for…",
}

/** A form's draft of a rule: the minutes stay text while the person types. */
export type RuleDraft = { mode: CacheMode; minutes: string }

export function isMode(v: unknown): v is CacheMode {
  return v === "default" || v === "fresh" || v === "minutes"
}

export function draftOf(rule: CacheRule | null | undefined): RuleDraft {
  if (rule && isMode(rule.mode)) return { mode: rule.mode, minutes: rule.minutes != null ? String(rule.minutes) : "" }
  return { mode: "default", minutes: "" }
}

/** The sentence for a draft that cannot be saved, or null. */
export function draftProblem(d: RuleDraft): string | null {
  if (d.mode !== "minutes") return null
  const n = Number(d.minutes)
  return d.minutes.trim() !== "" && Number.isInteger(n) && n >= CACHE_MINUTES.min && n <= CACHE_MINUTES.max ? null : MINUTES_SENTENCE
}

/** The rule a draft stands for (call after `draftProblem` found none). */
export function ruleOf(d: RuleDraft): CacheRule {
  return d.mode === "minutes" ? { mode: "minutes", minutes: Number(d.minutes) } : { mode: d.mode }
}

/** A rule as a short phrase ("Always fresh", "Remember for 30 minutes"). */
export function ruleLabel(rule: CacheRule | null | undefined): string {
  if (!rule || rule.mode === "default") return "Follow the default"
  if (rule.mode === "fresh") return "Always fresh"
  if (rule.mode === "minutes" && rule.minutes != null) return rule.minutes === 1 ? "Remember for 1 minute" : `Remember for ${rule.minutes} minutes`
  return "Follow the default"
}

/**
 * "Data from 10:42" in the report time zone, with "· remembered" when the
 * result came from memory (feature decision 5). Null when the result carries
 * no time (an older server, a text tile). An unknown zone falls back to UTC
 * and says so, never to the browser's zone.
 */
export function dataFromLabel(computedAt: string | undefined, timeZone: string | undefined, remembered: boolean | undefined): string | null {
  if (!computedAt) return null
  const at = new Date(computedAt)
  if (Number.isNaN(at.getTime())) return null
  const format = (zone: string) => new Intl.DateTimeFormat("en-GB", { hour: "2-digit", minute: "2-digit", hour12: false, timeZone: zone }).format(at)
  let clock: string
  try {
    clock = format(timeZone || "UTC") + (timeZone ? "" : " UTC")
  } catch {
    clock = `${format("UTC")} UTC`
  }
  return `Data from ${clock}${remembered ? " · remembered" : ""}`
}
