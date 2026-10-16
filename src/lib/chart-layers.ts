import type { FilterDef } from "@/services/clients/bi-store"
import { type MeasureEntry, type MeasureRow, entriesFromRows, rowsFromEntries } from "./chart-shape"
import type { JoinDef } from "./chart-joins"
import { decodeSourceChoice, sourcePayload, sourceValueFromDef } from "./chart-source"
import type { ColumnKind } from "./time-grain"

/**
 * More questions on one chart (`BI-13`): the builder's draft of a layer, the
 * `layers` field it sends, and the rules the console applies before the server
 * does (which dimensions can line up, what is still missing). They mirror
 * `lakehouse_bi::layers`, which checks every rule at save. Pure, so a test can
 * pin each conversion without rendering the dialog.
 */

/** A layer as stored and sent (`lakehouse_bi::layers::LayerDef`, camelCase). */
export type LayerDef = {
  mart?: string
  sqlSource?: string
  dimension: string
  grain?: string
  measures: MeasureEntry[]
  aggregate?: string
  filters?: FilterDef[]
  joins?: JoinDef[]
  label?: string
  draw: LayerDraw
  axis: LayerAxis
}

export type LayerDraw = "bar" | "line"
export type LayerAxis = "left" | "right"

/** Most questions a chart takes besides its own. */
export const MAX_LAYERS = 4
/** Kinds that take layers (`lakehouse_bi::layers::takes_layers`). */
export const LAYER_KINDS: readonly string[] = ["bar", "line", "area", "combo"]
/** Longest legend name of a layer, in characters. */
export const MAX_LABEL = 60

export function takesLayers(kind: string): boolean {
  return LAYER_KINDS.includes(kind)
}

/** What the builder holds for one more question while it is edited. */
export type LayerDraft = {
  /** The encoded source choice (`mart:<name>` or `sql:<id>`), as the data-source picker holds it. */
  source: string
  dimension: string
  measures: MeasureRow[]
  /** The aggregate of the measures that have none of their own. */
  aggregate: string
  filters: FilterDef[]
  /** Kept as saved: the builder has no editor for a layer's joins, and must not drop them. */
  joins: JoinDef[]
  label: string
  draw: LayerDraw
  axis: LayerAxis
}

/** A new question: a line on the right axis, the usual reason to add one (numbers on another scale). */
export function emptyLayer(): LayerDraft {
  return { source: "", dimension: "", measures: [{ column: "", aggregate: "" }], aggregate: "sum", filters: [], joins: [], label: "", draw: "line", axis: "right" }
}

/** A saved layer as a draft. */
export function layerDraftFromDef(def: LayerDef): LayerDraft {
  return {
    source: sourceValueFromDef(def),
    dimension: def.dimension ?? "",
    measures: rowsFromEntries(def.measures),
    aggregate: def.aggregate ?? "sum",
    filters: (def.filters ?? []).map((f) => ({ ...f, values: [...(f.values ?? [])] })),
    joins: structuredClone(def.joins ?? []),
    label: def.label ?? "",
    draw: def.draw === "bar" ? "bar" : "line",
    axis: def.axis === "left" ? "left" : "right",
  }
}

/** The layers of a saved definition as drafts. */
export function layerDraftsFromDef(layers: readonly LayerDef[] | undefined): LayerDraft[] {
  return (layers ?? []).map(layerDraftFromDef)
}

/** The `layers` field to send: one entry per draft, in order. */
export function layersPayload(drafts: readonly LayerDraft[]): LayerDef[] {
  return drafts.map((d) => {
    const label = d.label.trim()
    return {
      ...sourcePayload(decodeSourceChoice(d.source)),
      dimension: d.dimension,
      measures: entriesFromRows(d.measures, true),
      aggregate: d.aggregate,
      ...(d.filters.length ? { filters: d.filters } : {}),
      ...(d.joins.length ? { joins: d.joins } : {}),
      ...(label ? { label } : {}),
      draw: d.draw,
      axis: d.axis,
    } as LayerDef
  })
}

/** What is still needed before the layers can be sent, in the builder's own words. */
export function layerProblems(drafts: readonly LayerDraft[]): string[] {
  const out: string[] = []
  drafts.forEach((d, i) => {
    const n = i + 1
    if (!decodeSourceChoice(d.source)) out.push(`Question ${n} source`)
    if (!d.dimension) out.push(`Question ${n} dimension`)
    if (!d.measures.some((m) => m.column)) out.push(`Question ${n} measure`)
    if (d.label.trim().length > MAX_LABEL) out.push(`Question ${n} label (at most ${MAX_LABEL} characters)`)
  })
  return out
}

const isDate = (kind: string | undefined) => kind === "date" || kind === "datetime"

/**
 * Whether a layer's dimension can line up with the chart's: both dates, both
 * text or both numbers. A date and a timestamp meet only through a time grain,
 * which cuts both to the same bucket text (`lakehouse_bi::layers::same_dimension_kind`).
 * An unknown kind is not refused here; the server knows.
 */
export function dimensionsLineUp(base: string | undefined, other: string | undefined, grained: boolean): boolean {
  if (!base || !other) return true
  if (base === other) return true
  return isDate(base) && isDate(other) && grained
}

/** The columns of a layer's source that can line up with the chart's dimension. */
export function matchingDimensions<C extends { name: string; kind: ColumnKind | string }>(base: string | undefined, grained: boolean, columns: readonly C[]): C[] {
  return columns.filter((c) => dimensionsLineUp(base, c.kind, grained))
}

/** The note under a layer whose source has no column that lines up. */
export function noMatchNote(base: string | undefined): string {
  return base ? `This source has no ${base === "datetime" ? "date or timestamp" : base} column to line up with the chart's dimension.` : "Pick the chart's dimension first."
}
