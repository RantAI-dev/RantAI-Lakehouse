/**
 * Joining a chart's region names to the features of a bundled map.
 *
 * A choropleth colours the map feature whose `name` equals the row's region
 * name, so the join key decides what is drawn. Data comes from many sources
 * ("Kab. Bandung", "KOTA ADMINISTRASI JAKARTA PUSAT", "D.I. Yogyakarta"), the
 * GeoJSON files carry one spelling each, and this module is the single place
 * that reconciles them. Pure and tested; the ECharts side only draws what
 * it returns.
 *
 * The rules are deliberately few and spelled out. A name that matches no
 * feature is reported back, never guessed at and never dropped silently
 * (`matchRegionRows` returns it in `unmatchedNames`).
 */

/** What a bundled map's features are: Jakarta cities, provinces or kabupaten/kota. */
export type RegionLevel = "city" | "province" | "regency"

export type RegionValue = { name: string; value: number }

export type RegionMatch = {
  /** One entry per matched feature, values of rows that map to it summed. */
  data: RegionValue[]
  /** Rows whose region matched no feature (rows, not distinct names). */
  unmatchedRows: number
  /** Distinct unmatched names as they appeared in the data, first seen first. */
  unmatchedNames: string[]
}

/**
 * Comparison form of a name: lower case, dots read as spaces (so "D.I." and
 * "Kab." need no special case), runs of whitespace collapsed.
 */
function plain(raw: unknown): string {
  return String(raw ?? "")
    .toLowerCase()
    .replace(/\./g, " ")
    .replace(/\s+/g, " ")
    .trim()
}

/**
 * Province spellings that are not the bundled name, keyed by `plain()` form.
 * The bundled file uses "Dki Jakarta", "Daerah Istimewa Yogyakarta" and
 * "Kepulauan Bangka Belitung" (geoBoundaries' spelling). Only provinces
 * have aliases: on the regency map "Yogyakarta" is a kota, not the province.
 */
const PROVINCE_ALIASES: Record<string, string> = {
  "dki jakarta": "dki jakarta",
  jakarta: "dki jakarta",
  "di yogyakarta": "daerah istimewa yogyakarta",
  "d i yogyakarta": "daerah istimewa yogyakarta",
  yogyakarta: "daerah istimewa yogyakarta",
  "bangka belitung": "kepulauan bangka belitung",
  "kep bangka belitung": "kepulauan bangka belitung",
  "kep riau": "kepulauan riau",
  "nanggroe aceh darussalam": "aceh",
  nad: "aceh",
  ntb: "nusa tenggara barat",
  ntt: "nusa tenggara timur",
}

/** `Kabupaten X`, `Kab. X`, `Kab X`, `Kabupaten Administrasi X` → `X`. */
const KABUPATEN_PREFIX = /^(?:kabupaten|kab) (?:(?:administrasi|adm) )?/
/** `Kota Administrasi X`, `Kota Adm. X` → `Kota X` (plain `Kota X` is untouched). */
const KOTA_ADMIN_PREFIX = /^kota (?:administrasi|adm) /
const PROVINCE_PREFIX = /^(?:provinsi|propinsi|prov) /

/**
 * Normalizes a Jakarta area name so it matches the feature names in the
 * Jakarta GeoJSON (e.g. "KOTA JAKARTA PUSAT"/"Jakarta Pusat" → "Jakarta
 * Pusat"). Kept as it was when Jakarta was the only map, so existing
 * dashboards draw exactly as before.
 */
export function normalizeJakartaArea(raw: string): string {
  let s = String(raw ?? "").trim().replace(/\s+/g, " ")
  s = s.replace(/^(kota\s+(administrasi\s+)?|kabupaten\s+(administrasi\s+)?|kab\.?\s+)/i, "")
  const t = s.toLowerCase()
  if (t.includes("seribu")) return "Kepulauan Seribu"
  if (t.includes("pusat")) return "Jakarta Pusat"
  if (t.includes("utara")) return "Jakarta Utara"
  if (t.includes("barat")) return "Jakarta Barat"
  if (t.includes("selatan")) return "Jakarta Selatan"
  if (t.includes("timur")) return "Jakarta Timur"
  // Title-case fallback.
  return s.replace(/\b\w/g, (c) => c.toUpperCase())
}

/** The `plain()` name a row's region should have to match a feature of `level`. */
function candidate(raw: string, level: RegionLevel): string {
  if (level === "city") return plain(normalizeJakartaArea(raw))
  const s = plain(raw)
  if (level === "province") {
    const bare = s.replace(PROVINCE_PREFIX, "")
    return PROVINCE_ALIASES[bare] ?? bare
  }
  if (KOTA_ADMIN_PREFIX.test(s)) return s.replace(KOTA_ADMIN_PREFIX, "kota ")
  return s.replace(KABUPATEN_PREFIX, "")
}

/** Lookup from the comparison form of each feature name to the name as written. */
export type RegionIndex = Map<string, string>

export function buildRegionIndex(featureNames: Iterable<string>): RegionIndex {
  const index: RegionIndex = new Map()
  for (const name of featureNames) {
    const key = plain(name)
    // The first feature wins; a map whose names collide after normalizing
    // would be a data problem in the bundled file, not something to merge.
    if (key && !index.has(key)) index.set(key, name)
  }
  return index
}

/** The feature name a row's region refers to, or `null` when it matches none. */
export function matchRegion(raw: string, level: RegionLevel, index: RegionIndex): string | null {
  return index.get(candidate(raw, level)) ?? null
}

/**
 * Rows → one value per matched feature. Several rows for the same feature
 * (two spellings of one kabupaten, or a row per month) are summed. A row
 * whose value is not a number adds nothing, and a row whose region matches
 * no feature is counted in `unmatchedRows` so the tile can say so.
 */
export function matchRegionRows(
  rows: { name: string; value: number }[],
  level: RegionLevel,
  featureNames: Iterable<string>
): RegionMatch {
  const index = buildRegionIndex(featureNames)
  const sums = new Map<string, number>()
  const unmatched = new Set<string>()
  let unmatchedRows = 0
  for (const row of rows) {
    const feature = matchRegion(row.name, level, index)
    if (feature === null) {
      unmatchedRows += 1
      unmatched.add(String(row.name ?? "").trim() || "(blank)")
      continue
    }
    if (Number.isFinite(row.value)) sums.set(feature, (sums.get(feature) ?? 0) + row.value)
  }
  return {
    data: [...sums].map(([name, value]) => ({ name, value })),
    unmatchedRows,
    unmatchedNames: [...unmatched],
  }
}
