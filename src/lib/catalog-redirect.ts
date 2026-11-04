/**
 * Where an old `/data` link lands (`WEB-3` S2-D1).
 *
 * Data Explorer and Catalog were one asset list twice; the list is
 * `/catalog` now and `/data` only redirects. A link to `/data` carried the
 * table's own state in its query string, and some of it still means the
 * same thing on `/catalog`: the search term, the filters, the sort, how
 * filters join, the grouping, and the legacy `?q/layer/tier/type` names the
 * Catalog still translates on arrival (other pages and saved bookmarks still
 * build them). Those are kept. Everything else (page number, page size,
 * column layout, the list/table view) described the old page and is
 * dropped, so the Catalog opens in its own layout.
 */

/** Query parameters that mean the same thing on `/catalog` as on `/data`. */
const KEPT_PARAMS = [
  "search",
  "filters",
  "sort",
  "joinOperator",
  "groupBy",
  "q",
  "layer",
  "tier",
  "type",
] as const

/** The `/catalog` address an old `/data?…` link should open. */
export function catalogRedirectHref(
  query: Record<string, string | string[] | undefined>
): string {
  const next = new URLSearchParams()
  for (const key of KEPT_PARAMS) {
    const value = query[key]
    // A repeated parameter is not something the table ever wrote; the first
    // value is the one Next would have read too.
    const first = Array.isArray(value) ? value[0] : value
    if (first) next.set(key, first)
  }
  const search = next.toString()
  return search ? `/catalog?${search}` : "/catalog"
}
