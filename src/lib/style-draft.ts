import type { Annotation, Axes, AxisStyle, ChartColors, CondRule, DateFormat, StyleFields, ValueFormat } from "./chart-style"
import { isStaticKind } from "./dashboard-tabs"

/**
 * What the chart builder does with the style of a chart (`BI-17` part A, T7):
 * which groups of controls a kind has, what is read back from a saved
 * definition, and what is sent. Mirrors `lakehouse_bi::style` (which refuses a
 * field its kind does not take); the builder hides what the server would
 * refuse, so the form never offers a combination that cannot be saved.
 */

export type FormatRules = {
  /** Number formats (all values, or one) and date formats. */
  values: boolean
  dates: boolean
  /** A palette, a colour per series. */
  colors: boolean
  /** Colour by value: one value, no breakdown, a kind that draws one mark per value. */
  colorByValue: boolean
  axes: boolean
  /** A second Y axis, for the right-hand values. */
  secondAxis: boolean
  labels: boolean
  notes: boolean
  tooltip: boolean
  /** Table, pivot and KPI colours by rule. */
  conditional: boolean
  referenceLines: boolean
}

export type FormatState = { kind: string; breakdown: boolean; measures: number }

const AXIS_KINDS = ["bar", "hbar", "line", "area", "stacked", "combo", "scatter", "bubble", "waterfall", "boxplot"]
const SECOND_AXIS_KINDS = ["bar", "line", "area", "combo"]
export const NOTE_KINDS = ["bar", "hbar", "line", "area", "stacked", "combo", "waterfall"]
const TOOLTIP_KINDS = ["bar", "hbar", "line", "area", "stacked", "combo"]
const BY_VALUE_KINDS = ["bar", "hbar", "pie", "treemap", "geomap"]
const NOT_CHARTS = ["table", "pivot", "kpi", "gauge", "progress"]
export const LINE_KINDS = ["bar", "hbar", "line", "area", "stacked", "combo"]

export function formatRules(s: FormatState): FormatRules {
  const text = isStaticKind(s.kind)
  const chart = !text && !NOT_CHARTS.includes(s.kind)
  return {
    values: !text,
    dates: !text,
    colors: chart,
    colorByValue: BY_VALUE_KINDS.includes(s.kind) && !s.breakdown && s.measures <= 1,
    axes: AXIS_KINDS.includes(s.kind),
    secondAxis: SECOND_AXIS_KINDS.includes(s.kind) && s.measures >= 2,
    labels: chart,
    notes: NOTE_KINDS.includes(s.kind),
    tooltip: TOOLTIP_KINDS.includes(s.kind),
    conditional: ["table", "pivot", "kpi"].includes(s.kind),
    referenceLines: LINE_KINDS.includes(s.kind),
  }
}

/** The style fields of a saved definition, copied; nothing else of the definition. */
export function styleFromDef(def: StyleFields | undefined): StyleFields {
  if (!def) return {}
  const out: StyleFields = {}
  if (def.format) out.format = { ...def.format }
  if (def.valueFormats) out.valueFormats = Object.fromEntries(Object.entries(def.valueFormats).map(([k, v]) => [k, { ...v }]))
  if (def.dateFormat) out.dateFormat = { ...def.dateFormat }
  if (def.colors) out.colors = { ...def.colors, ...(def.colors.series ? { series: { ...def.colors.series } } : {}), ...(def.colors.byValue ? { byValue: [...def.colors.byValue] } : {}) }
  if (def.axes) out.axes = { ...def.axes, ...(def.axes.x ? { x: { ...def.axes.x } } : {}), ...(def.axes.y ? { y: { ...def.axes.y } } : {}), ...(def.axes.y2 ? { y2: { ...def.axes.y2 } } : {}), ...(def.axes.rightValues ? { rightValues: [...def.axes.rightValues] } : {}) }
  if (def.labels) out.labels = { ...def.labels }
  if (def.annotations) out.annotations = def.annotations.map((a) => ({ ...a }))
  if (def.tooltip) out.tooltip = { fields: [...def.tooltip.fields] }
  if (def.conditional) out.conditional = def.conditional.map((r) => ({ ...r, rule: { ...r.rule } }))
  return out
}

function cleanValueFormat(f: ValueFormat): ValueFormat | undefined {
  const out: ValueFormat = {}
  if (f.style && f.style !== "auto") out.style = f.style
  if (f.decimals !== undefined) out.decimals = f.decimals
  if (f.thousands === false) out.thousands = false
  const currencyStyle = f.style === "currency" || f.style === "compact"
  if (f.currency && currencyStyle) {
    out.currency = f.currency
    if (f.currencyDisplay === "code") out.currencyDisplay = "code"
  }
  if (f.prefix) out.prefix = f.prefix
  if (f.suffix) out.suffix = f.suffix
  return Object.keys(out).length ? out : undefined
}

function cleanDate(f: DateFormat): DateFormat | undefined {
  const out: DateFormat = {}
  if (f.style && f.style !== "auto") out.style = f.style
  if (f.hour12 !== undefined) out.hour12 = f.hour12
  if (f.abbreviate) out.abbreviate = true
  return Object.keys(out).length ? out : undefined
}

function cleanAxis(a: AxisStyle | undefined): AxisStyle | undefined {
  if (!a) return undefined
  const out: AxisStyle = {}
  if (a.title) out.title = a.title
  if (a.show === false) out.show = false
  if (a.min !== undefined && Number.isFinite(a.min)) out.min = a.min
  if (a.max !== undefined && Number.isFinite(a.max)) out.max = a.max
  if (a.log) out.log = true
  if (a.rotate !== undefined) out.rotate = a.rotate
  return Object.keys(out).length ? out : undefined
}

function cleanColors(c: ChartColors, byValueOk: boolean): ChartColors | undefined {
  const out: ChartColors = {}
  if (c.palette) out.palette = c.palette
  const series = Object.fromEntries(Object.entries(c.series ?? {}).filter(([name, colour]) => name && colour))
  if (Object.keys(series).length) out.series = series
  if (byValueOk && c.byValue && c.byValue.length >= 2) out.byValue = c.byValue
  return Object.keys(out).length ? out : undefined
}

function cleanRule(r: CondRule): CondRule | undefined {
  if (!r.column) return undefined
  const t = r.rule
  if (t.type === "range") return t.colors.length >= 2 ? { column: r.column, rule: t } : undefined
  if (!r.color) return undefined
  if (t.type === "compare" && !Number.isFinite(t.value)) return undefined
  if (t.type === "between" && !(Number.isFinite(t.from) && Number.isFinite(t.to))) return undefined
  if (t.type === "contains" && !t.text) return undefined
  return { column: r.column, rule: t, color: r.color, ...(r.target === "text" ? { target: "text" as const } : {}) }
}

/**
 * The style fields to send: empty containers are dropped, and so is every
 * group the kind does not take. Reading a definition with `styleFromDef` and
 * sending it back through this gives the same JSON (tested).
 */
export function stylePayload(style: StyleFields, rules: FormatRules): StyleFields {
  const out: StyleFields = {}
  if (rules.values) {
    const format = style.format && cleanValueFormat(style.format)
    if (format) out.format = format
    const per = Object.fromEntries(Object.entries(style.valueFormats ?? {}).flatMap(([k, v]) => { const c = cleanValueFormat(v); return c ? [[k, c]] : [] }))
    if (Object.keys(per).length) out.valueFormats = per
  }
  if (rules.dates) {
    const date = style.dateFormat && cleanDate(style.dateFormat)
    if (date) out.dateFormat = date
  }
  if (rules.colors && style.colors) {
    const colors = cleanColors(style.colors, rules.colorByValue)
    if (colors) out.colors = colors
  }
  if (rules.axes && style.axes) {
    const x = cleanAxis(style.axes.x)
    const y = cleanAxis(style.axes.y)
    const y2 = rules.secondAxis ? cleanAxis(style.axes.y2) : undefined
    const rightValues = rules.secondAxis ? style.axes.rightValues?.filter(Boolean) : undefined
    const axes: Axes = { ...(x ? { x } : {}), ...(y ? { y } : {}), ...(y2 ? { y2 } : {}), ...(rightValues?.length ? { rightValues } : {}) }
    if (Object.keys(axes).length) out.axes = axes
  }
  if (rules.labels && style.labels?.show) out.labels = { show: true }
  if (rules.notes) {
    const notes: Annotation[] = (style.annotations ?? []).filter((a) => a.x.trim() && a.text.trim()).map((a) => ({ x: a.x, ...(a.series ? { series: a.series } : {}), text: a.text }))
    if (notes.length) out.annotations = notes
  }
  if (rules.tooltip && style.tooltip?.fields.length) out.tooltip = { fields: style.tooltip.fields }
  if (rules.conditional) {
    const rows = (style.conditional ?? []).flatMap((r) => { const c = cleanRule(r); return c ? [c] : [] })
    if (rows.length) out.conditional = rows
  }
  return out
}

/**
 * The palette a chart draws with when the person chose none: the company
 * palette in a deployment that has one, the default otherwise
 * (`BI-17` part A, decision 6). A chart being edited keeps what it has.
 */
function paletteOfChoice(companyPalette: readonly string[], isEdit: boolean): string {
  return !isEdit && companyPalette.length >= 2 ? "company" : "default"
}

/**
 * The colours of a new chart in a deployment with a company palette
 * (`BI-17` part A, decision 6): the palette the company chose, unless the
 * person chose a palette of their own. A chart being edited keeps what it
 * has, and a chart saved before the palette was set keeps the default it was
 * drawn with (`PR #199 review M SHOULD-FIX BI-17 4`).
 */
export function withCompanyPalette(
  style: StyleFields,
  companyPalette: readonly string[],
  isEdit: boolean
): StyleFields {
  if (style.colors?.palette) return style
  const palette = paletteOfChoice(companyPalette, isEdit)
  if (palette === "default") return style
  return { ...style, colors: { ...style.colors, palette } }
}

/**
 * The palette the builder shows and the chart draws with: the one the person
 * chose, or the one a chart with no choice of its own draws with
 * (`PR #199 review O SHOULD-FIX 4`). A palette the person chose is kept as
 * it is — including "default", which is stored as a choice of its own, so a
 * company palette does not take it.
 */
export function effectivePalette(
  style: StyleFields,
  companyPalette: readonly string[],
  isEdit: boolean
): string {
  return style.colors?.palette ?? paletteOfChoice(companyPalette, isEdit)
}

/** Whether the chart being edited uses anything in the Format section, so the section opens by itself. */
export function formatUsed(style: StyleFields, referenceLines: number): boolean {
  return referenceLines > 0 || Object.keys(style).length > 0
}

/**
 * The definition of a chart with one more note pinned (`Add note` of the click menu): the stored
 * definition as it is, plus the note. The caller sends it to the chart's write route. `point` is
 * what the viewer clicked, as the chart shows it.
 */
export function withNote<T extends { annotations?: Annotation[] }>(def: T, point: string, text: string): T {
  return { ...def, annotations: [...(def.annotations ?? []), { x: point, text: text.trim() }] }
}
