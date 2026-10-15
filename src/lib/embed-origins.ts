/**
 * Validation of an "allowed site" before it becomes a chip in the Share
 * dialog (SEC-12). Mirrors `lakehouse_bi::embed_access::validate_origins`
 * (`rust/crates/lakehouse-bi/src/embed_access.rs`), which stays the
 * authority: the server validates again on save and its message is shown if
 * it refuses.
 */

/** At most this many sites per dashboard, as the server allows. */
export const MAX_ORIGINS = 20

export type OriginCheck = { ok: true; origin: string } | { ok: false; message: string }

/** Normalise one entry, or say why it is not acceptable. */
export function checkOrigin(raw: string): OriginCheck {
  const entry = raw.trim()
  const bad = (why: string): OriginCheck => ({ ok: false, message: why })
  if (entry.includes("*")) return bad("Wildcards are not accepted; list each site.")
  let scheme: "https" | "http"
  let rest: string
  if (entry.startsWith("https://")) {
    scheme = "https"
    rest = entry.slice("https://".length)
  } else if (entry.startsWith("http://")) {
    scheme = "http"
    rest = entry.slice("http://".length)
  } else {
    return bad("Write it as https://host or https://host:port.")
  }
  if (rest === "" || /[^A-Za-z0-9.:-]/.test(rest)) {
    return bad("Use only the scheme, host and optional port: no path, query or user name.")
  }
  const colon = rest.indexOf(":")
  const host = (colon === -1 ? rest : rest.slice(0, colon)).toLowerCase()
  const port = colon === -1 ? null : rest.slice(colon + 1)
  const hostOk =
    host.length > 0 &&
    host.length <= 253 &&
    host.split(".").every((l) => /^[a-z0-9]([a-z0-9-]*[a-z0-9])?$/.test(l))
  if (!hostOk) return bad("The host name is not valid.")
  if (scheme === "http" && host !== "localhost") return bad("http:// is only accepted for localhost; use https://.")
  if (port === null) return { ok: true, origin: `${scheme}://${host}` }
  const n = Number(port)
  if (!/^[0-9]+$/.test(port) || !Number.isInteger(n) || n < 1 || n > 65535) {
    return bad("The port must be a number from 1 to 65535.")
  }
  return { ok: true, origin: `${scheme}://${host}:${n}` }
}

/** Add `raw` to `current`: the new list, or the reason it was not added. */
export function addOrigin(current: string[], raw: string): { ok: true; origins: string[] } | { ok: false; message: string } {
  const checked = checkOrigin(raw)
  if (!checked.ok) return checked
  if (current.includes(checked.origin)) return { ok: false, message: "That site is already in the list." }
  if (current.length >= MAX_ORIGINS) return { ok: false, message: `At most ${MAX_ORIGINS} sites can be allowed.` }
  return { ok: true, origins: [...current, checked.origin] }
}
