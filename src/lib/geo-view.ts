/**
 * Pan/zoom state of a map tile, kept apart from ECharts so the arithmetic is
 * tested. It is view state only: never saved in the chart spec, and lost
 * when the tile unmounts. A data refresh keeps it by writing it back into
 * the rebuilt option (`withView`) instead of letting `setOption` reset the
 * map to its initial fit.
 */

export const ZOOM_MIN = 1
export const ZOOM_MAX = 20
/** One press of a zoom button. */
export const ZOOM_STEP = 1.5

export type MapView = { zoom: number; center?: [number, number] }

export function clampZoom(zoom: number): number {
  if (!Number.isFinite(zoom)) return ZOOM_MIN
  return Math.min(ZOOM_MAX, Math.max(ZOOM_MIN, zoom))
}

/** The zoom after pressing zoom in (`1`) or zoom out (`-1`), kept in 1x-20x. */
export function stepZoom(zoom: number, direction: 1 | -1): number {
  return clampZoom(direction === 1 ? zoom * ZOOM_STEP : zoom / ZOOM_STEP)
}

type Loose = Record<string, unknown>
const isRecord = (v: unknown): v is Loose => typeof v === "object" && v !== null && !Array.isArray(v)

/** The component or series that carries the roam state: `geo`, else the first series. */
function roamHost(option: Loose): Loose | undefined {
  const geo = Array.isArray(option.geo) ? option.geo[0] : option.geo
  if (isRecord(geo)) return geo
  const series = Array.isArray(option.series) ? option.series[0] : option.series
  return isRecord(series) ? series : undefined
}

/**
 * The view an ECharts instance reports through `getOption()` (it writes the
 * pan and zoom back into the model), or `null` when it carries none.
 */
export function readView(option: unknown): MapView | null {
  if (!isRecord(option)) return null
  const host = roamHost(option)
  if (!host) return null
  const zoom = typeof host.zoom === "number" ? clampZoom(host.zoom) : ZOOM_MIN
  const c = host.center
  const center =
    Array.isArray(c) && c.length === 2 && typeof c[0] === "number" && typeof c[1] === "number"
      ? ([c[0], c[1]] as [number, number])
      : undefined
  return { zoom, center }
}

/**
 * A copy of `option` whose map starts at `view`. With no view, or the
 * untouched one (zoom 1, no centre), the option is returned as it is so the
 * map keeps its initial fit.
 */
export function withView<T extends object>(option: T, view: MapView | null): T {
  if (!view || (view.zoom === ZOOM_MIN && !view.center)) return option
  const patch = { zoom: view.zoom, ...(view.center ? { center: view.center } : {}) }
  const o = option as Loose
  if (isRecord(o.geo)) return { ...o, geo: { ...o.geo, ...patch } } as T
  if (Array.isArray(o.series) && isRecord(o.series[0])) {
    return { ...o, series: [{ ...o.series[0], ...patch }, ...o.series.slice(1)] } as T
  }
  return option
}
