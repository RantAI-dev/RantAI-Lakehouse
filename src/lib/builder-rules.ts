import type { Grouping } from "./chart-shape"
import { takesBin, takesOther, takesSteps } from "./chart-shape"
import { takesLayers } from "./chart-layers"
import { kindHasClickValue } from "./chart-click"
import type { ChartKind } from "./dashboard-specs"

/**
 * Which sections and controls of the chart builder apply to the chosen kind
 * and to what is already set (phase 2 QA R5). One pure function, so the form
 * has no scattered conditions and a test can pin every kind. It agrees with
 * what `lakehouse_bi` refuses (`shape.rs` `check_steps_kind`, `is_grouped_kind`,
 * `takes_other`, `check_against_relation`; `layers.rs` `check_definition`) and
 * prevents the combination in the form: a control the kind cannot use is
 * hidden, and one that a choice already made rules out is hidden with a
 * one-line reason where it would have been.
 */
export type BuilderState = {
  kind: string
  /** A raw-rows table. */
  rowsMode: boolean
  /** A time grain is set on the chart's dimension. */
  grained: boolean
  /** A breakdown is chosen. */
  breakdown: boolean
  /** "Other" is on. */
  other: boolean
  /** How many steps and more questions the chart has now. */
  steps: number
  layers: number
  /** A grouped table with several groupings. */
  multiGrouped: boolean
  /** Whether the limit does anything (`limitHasEffect`): not beside a time grain with a natural order. */
  limitHasEffect: boolean
  /** A data source is chosen (or a new SQL source is being written). */
  hasSource: boolean
}

export type SortControl = "choice" | "by-date" | "by-category" | "none"

export type BuilderRules = {
  /** The four sections; a text note has none of them (title and content only). */
  sections: { data: boolean; show: boolean; sortAndFilter: boolean; moreOptions: boolean }
  sort: SortControl
  limit: boolean
  filters: boolean
  /** The "Other" switch: shown when the kind and the chart can use it; `disabled` with its reason when a step or a question is there. */
  other: { shown: boolean; disabled: boolean; hint?: string }
  steps: boolean
  layers: boolean
  referenceLines: boolean
  parameters: boolean
  click: boolean
  /** Why the breakdown (and bins) cannot be chosen now, shown where the control would be. */
  breakdownLocked?: string
  /** Bins on a number dimension or breakdown are offered (not beside more questions). */
  bins: boolean
}

const LINE_KINDS = ["bar", "hbar", "line", "area", "stacked", "combo"]
const SINGLE_KINDS = ["kpi", "gauge"]
const POINT_KINDS = ["pointmap", "geoheat"]

export function builderRules(s: BuilderState): BuilderRules {
  const text = s.kind === "text"
  const ownColumns = s.kind === "pivot" || (s.kind === "table" && s.rowsMode)
  const noSortOrLimit = text || SINGLE_KINDS.includes(s.kind) || POINT_KINDS.includes(s.kind) || ownColumns
  const sort: SortControl = noSortOrLimit ? "none" : s.kind === "boxplot" ? "by-category" : s.kind === "calendar" ? "by-date" : "choice"
  const otherApplies = takesOther(s.kind, s.rowsMode) && !s.grained && !s.multiGrouped && s.limitHasEffect
  const stepsApply = takesSteps(s.kind, s.rowsMode)
  const layersApply = takesLayers(s.kind) && !s.rowsMode
  const otherBlocked = !s.other && (s.steps > 0 || s.layers > 0)
  const hint = otherBlocked
    ? s.steps > 0 ? "Not available with steps." : "Not available with more questions."
    : s.other && (stepsApply || layersApply)
      ? "While this is on, steps and more questions are not available."
      : undefined
  return {
    sections: { data: !text, show: !text, sortAndFilter: !text, moreOptions: !text },
    sort,
    limit: !noSortOrLimit && s.limitHasEffect,
    filters: !text,
    other: { shown: !text && otherApplies, disabled: otherBlocked, hint },
    steps: stepsApply && !s.grained && !s.breakdown && !s.other && s.layers === 0,
    // A chart that has questions keeps the section so they can be removed; one that has none is offered it only when nothing rules it out.
    layers: layersApply && s.hasSource && (s.layers > 0 || (!s.breakdown && !s.other && s.steps === 0)),
    referenceLines: LINE_KINDS.includes(s.kind),
    parameters: !text,
    click: !text && kindHasClickValue(s.kind as ChartKind),
    breakdownLocked: s.layers > 0 ? "Not available with more questions." : undefined,
    bins: takesBin(s.kind, s.rowsMode) && s.layers === 0,
  }
}

/** What a chart being edited uses that sits inside "More options", so the fold opens by itself. */
export type MoreOptionsUse = {
  steps: number
  layers: number
  referenceLines: number
  /** A click that replaces the drill menu. */
  click: boolean
  /** Column formats of a grouped table. */
  columnSettings: number
  /** A KPI compared with an earlier period or a goal. */
  compare: boolean
}

export function moreOptionsUsed(u: MoreOptionsUse): boolean {
  return u.steps > 0 || u.layers > 0 || u.referenceLines > 0 || u.click || u.columnSettings > 0 || u.compare
}

/**
 * The picks of a chart kind that carry over to another (phase 2 QA R6). A chart
 * becoming a grouped table: its dimension (and breakdown) are the first
 * groupings, with the date grain or bins they had. A grouped table becoming a
 * chart: the first grouping is the dimension. Nothing is returned when nothing
 * changes, so the caller leaves the picks as they are.
 */
export type KindSwitch = {
  from: string
  to: string
  rowsMode: boolean
  dimension: string
  breakdown: string
  /** The dimension's grain and the bins of the dimension and the breakdown, as the chart sends them. */
  grain: string
  dimensionBin: Grouping["bin"]
  breakdownBin: Grouping["bin"]
  groupings: readonly Grouping[]
}
export type KindSwitchPatch = {
  groupings?: Grouping[]
  dimension?: string
  grain?: string
  bin?: Grouping["bin"] | null
}

export function carryOverOnKindSwitch(s: KindSwitch): KindSwitchPatch {
  if (s.from === s.to) return {}
  const groupedFrom = s.from === "table" && !s.rowsMode
  if (s.to === "table" && s.from !== "table") {
    // Only a chart that has a dimension, and a table that has no groupings of its own yet.
    if (!s.dimension || s.groupings.length) return {}
    const groupings: Grouping[] = [{ column: s.dimension, ...(s.grain ? { grain: s.grain } : {}), ...(s.dimensionBin ? { bin: s.dimensionBin } : {}) }]
    if (s.breakdown && s.breakdown !== s.dimension) groupings.push({ column: s.breakdown, ...(s.breakdownBin ? { bin: s.breakdownBin } : {}) })
    return { groupings, dimension: s.dimension }
  }
  if (groupedFrom && s.to !== "table" && s.groupings.length) {
    const first = s.groupings[0]
    return {
      dimension: first.column,
      grain: first.grain ?? "",
      bin: first.bin ?? null,
    }
  }
  return {}
}
