/**
 * Row → point shapes for the two point map kinds (`pointmap`, `geoheat`),
 * plus the helpers the chart builder and the tile share. Pure, so they are
 * tested here and `features/dashboards/geo-option.ts` only styles them.
 *
 * Rows come from the builder's SQL (one row per distinct coordinate pair,
 * largest value first, capped at `POINT_LIMIT`) or from a SQL source the
 * same way. Nothing here invents a position or a value: a row that cannot
 * be placed is dropped AND counted so the tile can say how many.
 */

type Row = Record<string, unknown>

/** Mirrors `lakehouse_bi::builder::POINT_LIMIT`. */
export const POINT_LIMIT = 5000
/**
 * Mirrors `lakehouse_bi::builder::point_limit` for a SQL source, which
 * ClickHouse caps at 2000 rows (`SQL_SOURCE_SETTINGS`).
 */
export const SQL_SOURCE_POINT_LIMIT = 2000

/** The most rows a point map's query can return, for a mart or a SQL source. */
export function pointLimit(sqlSource: string | undefined): number {
  return sqlSource ? SQL_SOURCE_POINT_LIMIT : POINT_LIMIT
}

export type MapPoint = { lat: number; lon: number; value: number; label: string }

export type PointSet = {
  points: MapPoint[]
  /** Rows without a usable coordinate pair (missing, not a number, out of range) or value. */
  dropped: number
  /** The query returned exactly its limit, so lower-valued rows may exist. */
  capped: boolean
}

/** A cell as a number; `null`, `""` and anything non-numeric are NaN (never 0). */
function toNumber(v: unknown): number {
  if (v === null || v === undefined || v === "") return Number.NaN
  return Number(v)
}

export function toPoints(
  rows: Row[],
  cols: { lat: string; lon: string; value: string; label?: string },
  limit: number
): PointSet {
  const points: MapPoint[] = []
  let dropped = 0
  for (const r of rows) {
    const lat = toNumber(r[cols.lat])
    const lon = toNumber(r[cols.lon])
    const value = toNumber(r[cols.value])
    const placeable = Number.isFinite(lat) && Math.abs(lat) <= 90 && Number.isFinite(lon) && Math.abs(lon) <= 180
    if (!placeable || !Number.isFinite(value)) {
      dropped += 1
      continue
    }
    points.push({ lat, lon, value, label: cols.label ? String(r[cols.label] ?? "") : "" })
  }
  return { points, dropped, capped: rows.length === limit }
}

/** Symbol diameter limits, in px. */
export const POINT_SIZE_MIN = 4
export const POINT_SIZE_MAX = 28

/**
 * Symbol diameter for `value`: area follows the value (square-root scale),
 * clamped to [POINT_SIZE_MIN, POINT_SIZE_MAX]. A value of zero or below, or
 * a map whose largest value is not positive, gets the smallest symbol.
 */
export function pointSize(value: number, max: number): number {
  if (!(max > 0) || !(value > 0)) return POINT_SIZE_MIN
  const share = Math.min(1, value / max)
  return POINT_SIZE_MIN + (POINT_SIZE_MAX - POINT_SIZE_MIN) * Math.sqrt(share)
}

const LAT_WORDS = ["lat", "latitude", "lintang"]
const LON_WORDS = ["lon", "lng", "long", "longitude", "bujur"]

function guess(columns: string[], words: string[]): string | undefined {
  const lower = columns.map((c) => c.toLowerCase())
  // A column named exactly so wins over one that only contains the word
  // (`lat` over `place_lat`), in the order the columns come.
  const exact = lower.findIndex((c) => words.includes(c))
  if (exact >= 0) return columns[exact]
  const partial = lower.findIndex((c) => c.split(/[^a-z0-9]+/).some((token) => words.includes(token)))
  return partial >= 0 ? columns[partial] : undefined
}

/**
 * The columns most likely to be latitude and longitude, from their names
 * (`lat|latitude|lintang`, `lon|lng|long|longitude|bujur`, alone or as a
 * `_`/`-` separated word of a longer name). Only a suggestion: the builder
 * pre-fills the selects with it and the user can change both.
 */
export function guessCoordinateColumns(columns: string[]): { lat?: string; lon?: string } {
  return { lat: guess(columns, LAT_WORDS), lon: guess(columns, LON_WORDS) }
}

/** For text interpolated into the HTML that an ECharts tooltip formatter returns. */
export function escapeHtml(text: string): string {
  return text
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;")
}
