/**
 * Building links that arrive at a table page with filters already applied.
 *
 * The advanced tables read their filters from one `filters` URL parameter
 * holding a JSON array, and every entry needs a `filterId` — without it the
 * toolbar cannot parse the filter, so it drops it and no chip appears even
 * though the rows come back filtered. That detail is easy to forget at each
 * call site, so links are built here instead.
 */

import type { FilterItemSchema } from "./parsers"

/** A filter as a caller states it; the id is filled in when missing. */
export type FilterLink = Omit<FilterItemSchema, "filterId"> & {
  filterId?: string
}

/**
 * The `filters` parameter value for these filters.
 *
 * Ids are derived from the position rather than random, so the same link
 * built twice is the same URL — comparable, cacheable and testable.
 */
export function filterParam(filters: readonly FilterLink[]): string {
  return JSON.stringify(
    filters.map((filter, index) => ({
      ...filter,
      filterId: filter.filterId ?? `link${index}`,
    }))
  )
}

/** `/data` with filters, and optionally a search term, already applied. */
export function tableFilterHref(
  path: string,
  filters: readonly FilterLink[],
  search?: string
): string {
  const params = new URLSearchParams()
  if (search) params.set("search", search)
  if (filters.length > 0) params.set("filters", filterParam(filters))
  const query = params.toString()
  return query ? `${path}?${query}` : path
}

/** Data Explorer, showing one namespace's assets. */
export function namespaceAssetsHref(namespace: string): string {
  return tableFilterHref("/data", [
    { id: "namespace", value: namespace, variant: "text", operator: "eq" },
  ])
}

/** Values currently selected for one column, read from a `filters` param. */
export function activeFilterValues(
  filtersParam: string,
  id: string
): string[] {
  if (!filtersParam) return []
  let parsed: unknown
  try {
    parsed = JSON.parse(filtersParam)
  } catch {
    // A hand-edited URL is the user's problem to fix, not a crash: an
    // unreadable filter simply means nothing is selected.
    return []
  }
  if (!Array.isArray(parsed)) return []
  return parsed.flatMap((filter) => {
    if (typeof filter !== "object" || filter === null) return []
    const entry = filter as { id?: unknown; value?: unknown }
    if (entry.id !== id) return []
    if (Array.isArray(entry.value)) return entry.value.map(String)
    return entry.value == null ? [] : [String(entry.value)]
  })
}

/**
 * Add or remove one value from a column's `inArray` filter, leaving every
 * other filter in the parameter untouched. Returns the new `filters` value,
 * or `""` when nothing is left to filter by.
 *
 * This is what the quick filters above a table need: they own one column
 * and must not disturb whatever the toolbar has set on the others.
 */
export function toggleFilterValue(
  filtersParam: string,
  id: string,
  value: string
): string {
  const others = (() => {
    if (!filtersParam) return []
    try {
      const parsed: unknown = JSON.parse(filtersParam)
      return Array.isArray(parsed)
        ? (parsed as FilterLink[]).filter((filter) => filter?.id !== id)
        : []
    } catch {
      return []
    }
  })()

  const current = activeFilterValues(filtersParam, id)
  const next = current.includes(value)
    ? current.filter((v) => v !== value)
    : [...current, value]

  const filters: FilterLink[] =
    next.length > 0
      ? [
          ...others,
          { id, value: next, variant: "multiSelect", operator: "inArray" },
        ]
      : others
  return filters.length > 0 ? filterParam(filters) : ""
}

/** The same `filters` value with one column's entries dropped. */
export function removeFilter(filtersParam: string, id: string): string {
  const values = activeFilterValues(filtersParam, id)
  return values.reduce(
    (acc, value) => toggleFilterValue(acc, id, value),
    filtersParam
  )
}
