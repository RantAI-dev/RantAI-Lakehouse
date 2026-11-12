/**
 * The `<schema>.<table>` key a catalog row names — the name a person types,
 * a query uses, and every governance form sends (`GOV-7` D9, `DATA-54`).
 *
 * A row's `id` is two different things: for Silver and Gold it IS the table
 * (`silver.orders`, `serving.mart_sales`), while for Bronze it is the
 * registry's slug, which names no table. A Bronze row names its table only
 * in `tableKey` (`bronze.<table_name>`, built by the catalog's list row), and
 * a row whose registry records no table name has none.
 */

/** The catalog fields a key is read from, as a list row carries them. */
type KeyedAsset = { id: string; tableKey?: string | null }

/**
 * The `<schema>.<table>` key a list row names, or `null` when it names none.
 * `GOV-7` D9: a Bronze list row carries `tableKey` (`bronze.<table_name>`);
 * a Silver or Gold list row carries none, because its `id` already is its
 * key (`silver_catalog_row`, `gold_catalog_row`). The `id` is therefore the
 * second candidate, which is also what keeps a row from an older API build —
 * one that sends no `tableKey` at all — usable.
 * `GOV-7 S2 review 2 R2-S1`.
 */
export function tableKeyOf(asset: KeyedAsset): string | null {
  for (const candidate of [asset.tableKey, asset.id]) {
    const key = candidate?.trim() ?? ""
    const dot = key.indexOf(".")
    if (dot > 0 && dot < key.length - 1) return key
  }
  return null
}
