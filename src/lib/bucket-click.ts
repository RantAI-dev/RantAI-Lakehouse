/**
 * What a click on a bucket of a grouped chart does (`BI-9`, feature page
 * decision 3): a day, week, month, quarter or year becomes a date-range filter
 * on the dashboard (first to last day of the bucket) and a list of the rows in
 * it; a minute or hour lists rows only; a part of the date (a weekday) does
 * neither, and the tile says why. The server cuts the buckets and lists the
 * rows in the report time zone; this module only turns a bucket into the range
 * a filter carries.
 */

import { encodeFilters, FILTER_PARAM } from "@/lib/dashboard-filter-state"
import { bucketFiltersDashboard, bucketListsRecords, bucketRange, type Grain } from "@/lib/time-grain"
import type { FilterDef } from "@/services/clients/bi-store"

/** What the drill menu offers for a bucket. */
export type BucketActions = { filter: boolean; records: boolean }

export function bucketActions(grain: Grain): BucketActions {
  return { filter: bucketFiltersDashboard(grain), records: bucketListsRecords(grain) }
}

/** One `between` filter on `column`, from the bucket's first to its last day; null for a bucket with no range. */
export function bucketRangeFilter(column: string, grain: Grain, value: string): FilterDef | null {
  const range = bucketRange(grain, value)
  return range ? { column, values: [], op: "between", min: range.from, max: range.to } : null
}

/**
 * The dashboard's filters after a click on a bucket: that column holds the
 * bucket's range, replacing any earlier filter on it. Clicking the bucket
 * that is already the filter removes it, as a value click does.
 */
export function toggleBucketFilter(filters: FilterDef[], next: FilterDef): FilterDef[] {
  const current = filters.find((f) => f.column === next.column)
  const same =
    current !== undefined &&
    (current.op ?? "in") === "between" &&
    current.min === next.min &&
    current.max === next.max &&
    !current.minExclusive &&
    !current.maxExclusive
  const rest = filters.filter((f) => f.column !== next.column)
  return same ? rest : [...rest, next]
}

/** Another dashboard, opened with the bucket's range as one `between` filter on `column`. */
export function dashboardRangeDestination(board: string, filter: FilterDef): string {
  const q = new URLSearchParams({ [FILTER_PARAM]: encodeFilters([filter]) })
  return `/dashboards/${encodeURIComponent(board)}?${q.toString()}`
}
