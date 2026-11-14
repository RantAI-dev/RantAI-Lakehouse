/**
 * Owner QA 2026-11-14, gallery pictures centred: the chart-type gallery draws
 * each type small, without axes. The chart's own option keeps margins sized
 * for axis labels (a wide left margin for the Y labels, `containLabel`) and
 * none for big symbols, so with the axes hidden a scatter sat a third in from
 * the left, a bubble chart clipped its last bubble, and a heatmap's grid and
 * colour bar sat off centre. This turns a finished ECharts option into the
 * picture's: the same marks, centred, with equal margins on every side that
 * are at least the largest symbol's radius, and no axes, legend, colour bar or
 * calendar labels.
 */

type Opt = Record<string, unknown>

const list = (v: unknown): Opt[] => (Array.isArray(v) ? (v as Opt[]) : v && typeof v === "object" ? [v as Opt] : [])
/** Keeps an option's shape: one object stays one object, a list stays a list. */
const mapSame = (v: unknown, f: (o: Opt) => Opt): unknown => (Array.isArray(v) ? (v as Opt[]).map(f) : v && typeof v === "object" ? f(v as Opt) : v)

/** The largest symbol radius any series draws, in px (a size given as a function is asked for each point). */
export function maxSymbolRadius(option: Opt): number {
  let max = 0
  for (const s of list(option.series)) {
    const size = s.symbolSize
    if (typeof size === "number") max = Math.max(max, size / 2)
    else if (typeof size === "function") {
      for (const d of (s.data as unknown[] | undefined) ?? []) {
        const value = d && typeof d === "object" && !Array.isArray(d) && "value" in d ? (d as { value: unknown }).value : d
        const r = (size as (v: unknown) => unknown)(value)
        if (typeof r === "number" && Number.isFinite(r)) max = Math.max(max, r / 2)
      }
    } else if (s.type === "line" || s.type === "scatter") max = Math.max(max, 3)
  }
  return max
}

/** The picture's option: equal margins of at least the largest symbol's radius, and nothing around the marks. */
export function thumbnailOption<T>(option: T): T {
  const o = option as unknown as Opt
  const pad = Math.ceil(Math.max(8, maxSymbolRadius(o) + 4))
  const box = { left: pad, right: pad, top: pad, bottom: pad }
  const hideAxis = (a: Opt): Opt => ({ ...a, show: false, name: "" })
  const out: Opt = { ...o, legend: { show: false } }
  const cartesian = o.xAxis !== undefined || o.yAxis !== undefined
  if (o.grid !== undefined || cartesian) out.grid = mapSame(o.grid ?? {}, (g) => ({ ...g, ...box, containLabel: false }))
  if (o.xAxis !== undefined) out.xAxis = mapSame(o.xAxis, hideAxis)
  if (o.yAxis !== undefined) out.yAxis = mapSame(o.yAxis, hideAxis)
  if (o.visualMap !== undefined) out.visualMap = mapSame(o.visualMap, (v) => ({ ...v, show: false }))
  if (o.dataZoom !== undefined) out.dataZoom = mapSame(o.dataZoom, (z) => ({ ...z, show: false }))
  // A radar's axis names circle it like axis labels; the picture keeps the shape, centred.
  if (o.radar !== undefined) out.radar = mapSame(o.radar, (r) => ({ ...r, center: ["50%", "50%"], radius: "78%", axisName: { show: false } }))
  if (o.calendar !== undefined) {
    out.calendar = mapSame(o.calendar, (c) => ({ ...c, ...box, dayLabel: { show: false }, monthLabel: { show: false }, yearLabel: { show: false } }))
  }
  return out as unknown as T
}
