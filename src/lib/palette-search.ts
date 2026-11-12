/**
 * Command-palette catalog search bounds (WS2 §13). The palette calls
 * `assetService.searchAssets` (through `@/services`,
 * never a raw URL) after a debounce and abort wiring that lives in
 * `command-palette.tsx` itself — plain `setTimeout`/`AbortController`
 * plumbing, not logic worth hiding behind an interface. The one rule that
 * IS worth pinning with a test is the result cap: the palette must never
 * show more than a handful of matches regardless of how many the server
 * returns. The other rule is the wording of why a result matched
 * (`matchedOnLabel`), shared with the Data Explorer so both say it alike.
 */

import { defaultFilter } from "cmdk"

import type { Asset } from "@/services/contracts/assets"

/** The palette shows at most this many catalog-asset results per search. */
export const PALETTE_ASSET_RESULT_LIMIT = 8

/**
 * Truncates `results` to at most `PALETTE_ASSET_RESULT_LIMIT` entries,
 * preserving order (the server's own relevance/registry order).
 */
export function capPaletteAssetResults<T>(results: T[]): T[] {
  return results.slice(0, PALETTE_ASSET_RESULT_LIMIT)
}

/**
 * The line under a search result that says why it matched (`DATA-11`):
 * "column revenue_amount", "tag finance", "description", with
 * "approximate match" added when a typo was forgiven. `null` when the
 * server sent no reason — a match on the name needs none.
 *
 * `DATA-32` D1: the server reports the name itself when the name forgave a
 * typo and nothing else explains the hit. The name is on screen already, so
 * that reason reads "approximate match" and nothing more.
 */
export function matchedOnLabel(matchedOn: Asset["matchedOn"]): string | null {
  if (!matchedOn) return null
  const { field, value, approximate } = matchedOn
  let what: string
  switch (field) {
    case "name":
      // `DATA-32` D1: the name forgave a typo; saying the name again would
      // say nothing the reader does not already see.
      return approximate ? "approximate match" : "name"
    case "column":
    case "tag":
      what = value ? `${field} ${value}` : field
      break
    case "table":
      // QA `LH-DATA-11-05` (`DATA-54`): the server reports the physical name
      // the search read; the reader sees which table matched.
      what = value ? `table ${value}` : "table"
      break
    case "id":
      // An API build older than `DATA-54` still sends `"id"`; it read as a
      // column named id, which is not what it is.
      what = "table id"
      break
    case "columnDescription":
      what = "column description"
      break
    default:
      // namespace, owner, description; a field a newer server adds is shown
      // as it came rather than hidden.
      what = field
  }
  return approximate ? `${what} · approximate match` : what
}

/** The `value` prefix of a catalog-asset row in the palette (`command-palette.tsx`). */
export const ASSET_VALUE_PREFIX = "asset "

/**
 * cmdk's `filter` for the palette. QA LH-DATA-11-06: cmdk scores every item
 * by fuzzy match and re-sorts, so the server's own order of catalog assets
 * (its relevance order) was lost. Every asset row gets one score, so the sort
 * (stable) leaves them in the order they were rendered. The score is the best
 * one cmdk would have given any asset row, so the asset group keeps the
 * position among the other groups that it has today (cmdk orders groups by
 * their best item). Every other item keeps cmdk's own score.
 */
export function makePaletteFilter(assetValues: string[]) {
  return (value: string, search: string, keywords?: string[]): number => {
    if (!value.startsWith(ASSET_VALUE_PREFIX)) return defaultFilter(value, search, keywords)
    let best = 0
    for (const v of assetValues) best = Math.max(best, defaultFilter(v, search, keywords))
    return best
  }
}
