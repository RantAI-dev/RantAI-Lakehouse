import type { MetricDraft, MetricFilter, MetricUsage, StoredFilter } from "@/services/contracts/metrics"

/**
 * The metric editor's form (BI-2): what the dialog holds while a metric is
 * written, and how it becomes the body the server takes. Pure, so the rules
 * (what blocks Save, what a plain filter is against a segment) are tested
 * without a screen.
 */
export type MetricForm = {
  label: string
  description: string
  formula: string
  /** Plain filters of the metric (a `StoredFilter` with a column). */
  filters: StoredFilter[]
  /** Names of the segments the metric is also narrowed by. */
  segments: string[]
  timeColumn: string
  grain: string
  format: string
}

export const MAX_DESCRIPTION = 500
export const MAX_LABEL = 100

export function emptyMetricForm(): MetricForm {
  return { label: "", description: "", formula: "", filters: [], segments: [], timeColumn: "", grain: "", format: "" }
}

/** A saved metric's filters split into plain filters and segment names. */
export function formFromMetric(m: {
  label: string; description: string; formula: string; filters: readonly MetricFilter[]; timeColumn?: string; grain?: string; format?: string
}): MetricForm {
  return {
    label: m.label, description: m.description, formula: m.formula,
    filters: m.filters.filter((f) => typeof f.segment !== "string"),
    segments: m.filters.flatMap((f) => (typeof f.segment === "string" ? [f.segment] : [])),
    timeColumn: m.timeColumn ?? "", grain: m.grain ?? "", format: m.format ?? "",
  }
}

/** The body the server takes: segments first as references, then the metric's own filters. */
export function draftFromForm(f: MetricForm): MetricDraft {
  return {
    label: f.label.trim(),
    description: f.description.trim(),
    formula: f.formula,
    filters: [...f.segments.map((segment) => ({ segment })), ...f.filters],
    timeColumn: f.timeColumn || undefined,
    // A grain without a date column means nothing; the server refuses it.
    grain: f.timeColumn && f.grain ? f.grain : undefined,
    format: f.format || undefined,
  }
}

/**
 * The label the live check sends while the field is still empty. `POST
 * /api/dashboard/metrics/validate` refuses an empty label ("a label is 1 to
 * 100 characters."), but the value only depends on the source and the formula,
 * so "Value now" must not wait for a label (phase 2 QA R22). It is never
 * saved and never shown: Create still blocks on the empty label through
 * `formProblems`, with its own message.
 */
export const PROVISIONAL_LABEL = "Preview"

/** The draft the live check sends: the real one, with a provisional label if none is typed yet. */
export function draftForCheck(f: MetricForm): MetricDraft {
  const draft = draftFromForm(f)
  return draft.label ? draft : { ...draft, label: PROVISIONAL_LABEL }
}

/** What is still missing, in the editor's own words; empty when the form can be checked and saved. */
export function formProblems(f: MetricForm, isNew: boolean, name: string): string[] {
  const out: string[] = []
  if (isNew && !/^[A-Za-z_][A-Za-z0-9_]{0,63}$/.test(name)) out.push("Name")
  if (!f.label.trim()) out.push("Label")
  if (!f.formula.trim()) out.push("Formula")
  if (f.description.length > MAX_DESCRIPTION) out.push("Description (at most 500 characters)")
  return out
}

/** The sentence that asks before a change reaches charts: what uses the metric, by title. */
export function usageSentence(u: Pick<MetricUsage, "charts" | "alerts" | "dashboards">): string {
  const charts = u.charts.length
  const boards = u.dashboards.length
  const titles = u.charts.slice(0, 5).map((c) => `"${c.title}"${c.boardName ? ` on ${c.boardName}` : ""}`)
  const more = charts > 5 ? ` and ${charts - 5} more` : ""
  const alerts = u.alerts.length ? ` and ${u.alerts.length} alert${u.alerts.length === 1 ? "" : "s"}` : ""
  if (charts === 0 && u.alerts.length === 0) return ""
  return `This changes ${charts} chart${charts === 1 ? "" : "s"}${alerts} on ${boards} dashboard${boards === 1 ? "" : "s"} at once: ${titles.join(", ")}${more}.`
}
