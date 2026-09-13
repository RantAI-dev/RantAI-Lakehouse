/**
 * Command-palette catalog search bounds (WS2 §13, Task E2 pre-dispatch fix
 * E2-3). The palette calls `assetService.listAssets` (through `@/services`,
 * never a raw URL) after a debounce and abort wiring that lives in
 * `command-palette.tsx` itself — plain `setTimeout`/`AbortController`
 * plumbing, not logic worth hiding behind an interface. The one rule that
 * IS worth pinning with a test is the result cap: the palette must never
 * show more than a handful of matches regardless of how many the server
 * returns.
 */

/** The palette shows at most this many catalog-asset results per search. */
export const PALETTE_ASSET_RESULT_LIMIT = 8

/**
 * Truncates `results` to at most `PALETTE_ASSET_RESULT_LIMIT` entries,
 * preserving order (the server's own relevance/registry order).
 */
export function capPaletteAssetResults<T>(results: T[]): T[] {
  return results.slice(0, PALETTE_ASSET_RESULT_LIMIT)
}
