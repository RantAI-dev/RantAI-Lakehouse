/**
 * What `BI-17` part A adds to a chart definition: number and date formats,
 * colours, axes, data labels, notes, tooltip fields and conditional colours.
 * Mirrors `lakehouse_bi::style::StyleFields` (flat, camelCase on the wire,
 * every field absent on a chart saved before it existed). The server checks
 * every limit at save with a plain sentence; this module names the shapes
 * and the lists the builder offers.
 */

export const FORMAT_STYLES = ["auto", "number", "percent", "currency", "scientific", "compact"] as const
export type FormatStyle = (typeof FORMAT_STYLES)[number]

export type ValueFormat = {
  style?: FormatStyle
  /** Digits after the point, 0 to 10. Absent: the style's own. */
  decimals?: number
  /** Group thousands. Absent means yes. */
  thousands?: boolean
  /** An ISO 4217 code, upper case; with the `currency` or `compact` style. */
  currency?: string
  currencyDisplay?: "symbol" | "code"
  /** At most 16 characters. */
  prefix?: string
  suffix?: string
}

export const DATE_STYLES = ["auto", "short", "medium", "long", "iso", "monthYear"] as const
export type DateStyle = (typeof DATE_STYLES)[number]
export type DateFormat = { style?: DateStyle; hour12?: boolean; abbreviate?: boolean }

/** The palettes the console ships; `company` is the deployment's own (Admin > Settings). */
export const BUILTIN_PALETTES = ["default", "bright", "muted", "sequential-blue", "diverging-red-blue"] as const
export type ChartColors = {
  /** A built-in id or `company`. */
  palette?: string
  /** Series name to `#rrggbb`. */
  series?: Record<string, string>
  /** Two or three `#rrggbb` stops over the shown minimum and maximum. */
  byValue?: string[]
}

export type AxisStyle = {
  title?: string
  /** Absent means shown. */
  show?: boolean
  min?: number
  max?: number
  log?: boolean
  /** Label rotation in degrees. */
  rotate?: 0 | 45 | 90
}
export type Axes = { x?: AxisStyle; y?: AxisStyle; y2?: AxisStyle; rightValues?: string[] }

export type Annotation = { x: string; series?: string; text: string }

export type CompareOp = "lt" | "le" | "gt" | "ge" | "eq" | "ne"
export type CondRuleTest =
  | { type: "compare"; op: CompareOp; value: number }
  | { type: "between"; from: number; to: number }
  | { type: "contains"; text: string }
  | { type: "range"; colors: string[] }
export type CondRule = {
  column: string
  rule: CondRuleTest
  /** `#rrggbb`; a range carries its own colours. */
  color?: string
  /** Absent means the cell's background. */
  target?: "cell" | "text"
}

/**
 * `BI-17` part B (plan D2): colour the points of a scatter or bubble chart by
 * cluster. `k` is the number of groups, 2 to 10, or "auto" (the console picks
 * it). Mirrors `StyleFields.clusters` of `lakehouse_bi::style`, which the
 * server validates once T1 lands; until then only the console checks it.
 */
export type ClusterSetting = { k: "auto" | number }

export const MIN_CLUSTERS = 2
export const MAX_CLUSTERS = 10

export type StyleFields = {
  format?: ValueFormat
  valueFormats?: Record<string, ValueFormat>
  dateFormat?: DateFormat
  colors?: ChartColors
  axes?: Axes
  labels?: { show: boolean }
  annotations?: Annotation[]
  tooltip?: { fields: string[] }
  conditional?: CondRule[]
  clusters?: ClusterSetting
  /** `gantt` (BI-16 part C1, plan section 7): `false` hides the line at today; absent means shown. T1 validates it. */
  todayLine?: boolean
}

/** Limits the server enforces; the builder uses them to stop early. */
export const MAX_DECIMALS = 10
export const MAX_AFFIX_LEN = 16
export const MAX_AXIS_TITLE_LEN = 60
export const MAX_NOTE_LEN = 140
export const MAX_ANNOTATIONS = 20
export const MAX_CONDITIONAL_RULES = 20

/** What a reference line marks; mirrors `lakehouse_bi::params::LineKind`. */
export type ReferenceLineKind = "value" | "average" | "median" | "min" | "max" | "trend" | "band" | "forecast"

/**
 * A reference line as the server sends it with a tile's rows, parameters
 * already resolved (`referenceLines` of the dashboard payload). `kind` is left
 * out for a plain line; `value` is absent on a computed one, which the console
 * works out from the rows on screen (`withReferenceLines`).
 */
export type ResolvedReferenceLine = {
  kind?: ReferenceLineKind
  value?: number
  /** A band's upper edge. */
  to?: number
  label?: string
  color?: string
  /** The value a computed line describes, by result name; absent is the first. */
  measure?: string
  /** Periods ahead of a `forecast` line (`BI-17` part B, D1); the server validates it with T1. */
  horizon?: number
}
