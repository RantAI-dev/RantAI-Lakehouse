/**
 * The medallion zones a pipeline can read from and write to.
 *
 * These are the product's three layers, NOT Lakekeeper's Iceberg namespaces.
 * Reading the zone list from `/api/lakehouse/namespaces` used to expose the
 * warehouse's `default` namespace and hide ClickHouse's `silver` and
 * `serving` tables: neither is a pipeline zone. Gold's real zone is `serving`
 * (the namespace the catalog and Query Studio use for marts); `bronze` is the
 * Iceberg namespace the upload and connector jobs load into.
 *
 * A zone value is the stored, exact string the API validates. The label is
 * the console's own name for the layer (`lib/status.ts`'s `DATA_LAYER_LABEL`),
 * so a picker reads "Gold" while saving `serving`.
 */

/** A zone offered as a pipeline input, with the label the console uses. */
export const INPUT_ZONES = [
  { value: "bronze", label: "Bronze" },
  { value: "silver", label: "Silver" },
  { value: "serving", label: "Gold" },
] as const

/** A zone offered as a pipeline output. Bronze is never written by a pipeline. */
export const OUTPUT_ZONES = [
  { value: "silver", label: "Silver" },
  { value: "serving", label: "Gold" },
] as const

/** The stored values, one array per picker, kept stable across renders. */
export const INPUT_ZONE_VALUES: readonly string[] = INPUT_ZONES.map((zone) => zone.value)
export const OUTPUT_ZONE_VALUES: readonly string[] = OUTPUT_ZONES.map((zone) => zone.value)

/** Value -> human label, shared by both pickers (including the legacy held value's base). */
export const ZONE_LABELS: Readonly<Record<string, string>> = Object.fromEntries(
  INPUT_ZONES.map((zone) => [zone.value, zone.label])
)

/**
 * The catalog namespace that holds a stored zone's tables and columns, or
 * `null` for Iceberg Bronze (which the lakehouse API, not the catalog, lists).
 *
 * A pipeline saved before Gold moved to `serving` can hold `gold`; it reads
 * from the same place as `serving` without the edit silently rewriting the
 * stored value.
 */
export function catalogNamespaceForZone(zone: string): string | null {
  const trimmed = zone.trim()
  if (trimmed === "silver" || trimmed === "serving") return trimmed
  if (trimmed === "gold") return "serving"
  return null
}

/** The catalog id of a table in a namespace, e.g. `silver.orders`. */
export function catalogAssetId(namespace: string, table: string): string {
  return `${namespace}.${table}`
}
