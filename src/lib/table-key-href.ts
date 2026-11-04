/**
 * The asset page a table key (`silver.orders`) opens, or `null` when the key
 * is not an asset id. A Silver or serving table's key is its asset id; a
 * Bronze table's asset is addressed by a registry slug its key does not
 * carry (`WEB-3` section 6.1, the Bronze slug gap), so a Bronze key gets
 * no link rather than a guessed one. `tab` opens one of the page's tabs.
 */
export function tableKeyAssetHref(key: string, tab?: string): string | null {
  const k = key.trim()
  if (!k.startsWith("silver.") && !k.startsWith("serving.")) return null
  const href = `/data/assets/${encodeURIComponent(k)}`
  return tab ? `${href}?tab=${encodeURIComponent(tab)}` : href
}
