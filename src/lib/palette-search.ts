/**
 * Command-palette catalog search bounds (WS2 §13). The palette calls
 * `assetService.listAssets` (through `@/services`,
 * never a raw URL) after a debounce and abort wiring that lives in
 * `command-palette.tsx` itself — plain `setTimeout`/`AbortController`
 * plumbing, not logic worth hiding behind an interface. The one rule that
 * IS worth pinning with a test is the result cap: the palette must never
 * show more than a handful of matches regardless of how many the server
 * returns. The other rule is the wording of why a result matched
 * (`matchedOnLabel`), shared with the Data Explorer so both say it alike.
 */

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
    case "columnDescription":
      what = "column description"
      break
    default:
      // id, namespace, owner, description; a field a newer server adds is
      // shown as it came rather than hidden.
      what = field
  }
  return approximate ? `${what} · approximate match` : what
}
