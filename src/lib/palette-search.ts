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
import { tableKeyOf } from "@/lib/table-key"

/** The palette shows at most this many catalog-asset results per search. */
export const PALETTE_ASSET_RESULT_LIMIT = 8

/**
 * The longest text the box takes: the dashboard search's own limit
 * (`lakehouse_bi::search::MAX_QUERY_CHARS`). The server refuses a longer text
 * with 400, so the box stops the typing at the same number and says what to do
 * when an older answer is still refused (`QA LH-DATA-11-15` F2, `DATA-54`).
 * `palette-search.test.ts` fails when the two numbers differ.
 */
export const PALETTE_SEARCH_MAX_CHARS = 100

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
    case "description":
      // QA `LH-DATA-11-04` (`DATA-54`): the server sends a piece of the
      // description around the matched word; it is shown as text, never as
      // HTML.
      what = value ? `description: "${value}"` : "description"
      break
    case "columnDescription":
      what = "column description"
      break
    default:
      // namespace, owner; a field a newer server adds is shown as it came
      // rather than hidden.
      what = field
  }
  return approximate ? `${what} · approximate match` : what
}

/**
 * The two lines of a catalog-asset result (`LH-DATA-11-01` F1, `DATA-54`):
 * the table's physical name (`bronze.orders`, `serving.mart_sales`), and why
 * it matched. When the reason is the table key itself the key is not printed
 * twice — the reason is then only "approximate match" when a typo was
 * forgiven, and nothing otherwise.
 */
export function paletteAssetLines(
  asset: Pick<Asset, "id" | "tableKey" | "matchedOn">,
): { path: string | null; reason: string | null } {
  const path = tableKeyOf(asset)
  if (path !== null && asset.matchedOn?.field === "table") {
    return { path, reason: asset.matchedOn.approximate ? "approximate match" : null }
  }
  return { path, reason: matchedOnLabel(asset.matchedOn) }
}

/** The `value` prefix of a catalog-asset row in the palette (`command-palette.tsx`). */
export const ASSET_VALUE_PREFIX = "asset "

/**
 * Whether every word of `search` is a substring, in any case, of `value` and
 * the `keywords` (QA `LH-DATA-11-14`, `DATA-54`).
 */
function everyWordIn(value: string, search: string, keywords?: string[]): boolean {
  const haystack = `${value} ${(keywords ?? []).join(" ")}`.toLowerCase()
  return search
    .toLowerCase()
    .split(/\s+/)
    .filter((word) => word !== "")
    .every((word) => haystack.includes(word))
}

/**
 * cmdk's `filter` for the palette. QA LH-DATA-11-06: cmdk scores every item
 * by fuzzy match and re-sorts, so the server's own order of catalog assets
 * (its relevance order) was lost. Every asset row gets one score, so the sort
 * (stable) leaves them in the order they were rendered. The score is the best
 * one cmdk would have given any asset row, so the asset group keeps the
 * position among the other groups that it has today (cmdk orders groups by
 * their best item). Every other item keeps cmdk's own score.
 *
 * QA `LH-DATA-11-14` (`DATA-54`): `cmdk` accepts the letters in order
 * anywhere, so `gic` was found in "Governance Access rules
 * /governance/policies". A page or an action is listed only when every typed
 * word is in its text; `cmdk`'s score still decides the order and nothing
 * new is listed. Asset rows are not affected: the server decided them.
 */
export function makePaletteFilter(assetValues: string[]) {
  return (value: string, search: string, keywords?: string[]): number => {
    const score = defaultFilter(value, search, keywords)
    if (value.startsWith(ASSET_VALUE_PREFIX)) {
      let best = 0
      for (const v of assetValues) best = Math.max(best, defaultFilter(v, search, keywords))
      return best
    }
    return score > 0 && everyWordIn(value, search, keywords) ? score : 0
  }
}
