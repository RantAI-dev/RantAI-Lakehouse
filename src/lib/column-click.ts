import type { ColumnKind } from "./time-grain"

/**
 * `BI-58` T2: what one click on a column of the chart builder's data panel
 * does. A column that is not in use goes where it fits: a date or text column
 * becomes the X axis while the chart has none, a number column becomes the
 * value while the chart has none. Anything else opens a small menu of the
 * uses the chart can take now ("Use as X axis / value / split / filter").
 * Pure, so the rule is pinned by a test and the panel carries no conditions.
 */

/** How the panel groups a column: Dates, Text or Numbers. */
export type ColumnGroup = "date" | "text" | "number"

export type ColumnUse = "x" | "value" | "split" | "filter"

/** The order and the words of the menu. */
export const COLUMN_USES: readonly { use: ColumnUse; label: string }[] = [
  { use: "x", label: "Use as X axis" },
  { use: "value", label: "Use as value" },
  { use: "split", label: "Use as split" },
  { use: "filter", label: "Use as filter" },
]

export type ColumnClick = {
  group: ColumnGroup
  /** The uses the chart's kind and its picks offer this column now (the builder's own pickers decide). */
  offers: Readonly<Record<ColumnUse, boolean>>
  /** The chart has no X axis (dimension) yet. */
  xEmpty: boolean
  /** The chart's first value (measure) is not picked yet. */
  valueEmpty: boolean
  /** The column is the X axis, a value or the split already. */
  inUse: boolean
}

/** The group a column is listed under, from its kind (a column without one reads as text). */
export function columnGroup(kind: ColumnKind | undefined): ColumnGroup {
  if (kind === "date" || kind === "datetime") return "date"
  if (kind === "number") return "number"
  return "text"
}

/** The use one click applies at once, or `null` when the click opens the menu instead. */
export function autoUse(c: ColumnClick): ColumnUse | null {
  if (c.inUse) return null
  if (c.group !== "number" && c.offers.x && c.xEmpty) return "x"
  if (c.group === "number" && c.offers.value && c.valueEmpty) return "value"
  return null
}

/** The menu's items: the uses offered now, in the menu's order. */
export function menuUses(c: ColumnClick): { use: ColumnUse; label: string }[] {
  return COLUMN_USES.filter((u) => c.offers[u.use])
}
