/**
 * Whether saving a connector's connection settings would point it somewhere
 * else (`SEC-14`): the edit form uses it to decide whether to ask for the
 * credentials again.
 *
 * The Rust function `Dial::target_identity` / `is_repoint`
 * (`rust/crates/lakehouse-store/src/ingest_spec.rs`) is the AUTHORITY. This
 * one only decides what the form asks for; a difference between the two can
 * make the form ask for too little (the server then answers 409 and the
 * user sees why) or too much, never let a credential go anywhere. Keep the
 * field list and the normalisation in step with that function:
 *
 * | Adapter | Compared |
 * | --- | --- |
 * | `sql`, `cdc` | `driver`, `host`, `port`, `database` |
 * | `files` | `protocol`, `endpoint`, `bucket` |
 * | `rest` | scheme, host and port of `baseUrl` |
 * | `mongodb` | `hosts` (as a set), `database` |
 * | `kafka` | `bootstrapServers` (as a set) |
 * | `sftp` | `host`, `port`, `hostKeyFingerprint` |
 * | `sheets` | nothing |
 *
 * Host names compare case-insensitively. In a URL an absent port is the
 * scheme's default (`https://h` is `https://h:443`); the other shapes have no
 * absent port. The adapter is part of the identity.
 */

type Dial = Record<string, unknown> | null | undefined

const text = (value: unknown): string | null => (typeof value === "string" ? value : null)
const port = (value: unknown): string | null =>
  typeof value === "number" && Number.isInteger(value) ? String(value) : null
const host = (value: unknown): string | null => {
  const raw = text(value)
  return raw === null ? null : raw.trim().toLowerCase()
}

/** The port at the end of `host:port`, when it is a number in range. */
function splitAuthority(raw: string): { host: string; port: number | null } {
  const authority = raw.trim().toLowerCase()
  let hostPart = authority
  let portPart: string | null = null
  if (authority.startsWith("[")) {
    const at = authority.lastIndexOf("]:")
    if (at >= 0) {
      hostPart = authority.slice(0, at + 1)
      portPart = authority.slice(at + 2)
    }
  } else {
    const at = authority.lastIndexOf(":")
    if (at >= 0) {
      hostPart = authority.slice(0, at)
      portPart = authority.slice(at + 1)
    }
  }
  if (portPart !== null && /^\d{1,5}$/.test(portPart) && Number(portPart) <= 65535) {
    return { host: hostPart, port: Number(portPart) }
  }
  return { host: authority, port: null }
}

/** `scheme://host:port` of a URL, default port filled in; anything else as it is. */
function origin(raw: string): string {
  const url = raw.trim()
  const at = url.indexOf("://")
  if (at < 0) return `raw:${url.toLowerCase()}`
  const scheme = url.slice(0, at).toLowerCase()
  const authority = url.slice(at + 3).split(/[/?#]/)[0] ?? ""
  const hostPort = authority.slice(authority.lastIndexOf("@") + 1)
  const split = splitAuthority(hostPort)
  const defaultPort = scheme === "http" ? 80 : scheme === "https" ? 443 : null
  const effective = split.port ?? defaultPort
  return effective === null ? `${scheme}://${split.host}` : `${scheme}://${split.host}:${effective}`
}

/** `host:port` entries as one string: order, repeats and case do not matter. */
function hostPortSet(value: unknown): string | null {
  if (!Array.isArray(value) || !value.every((entry) => typeof entry === "string")) return null
  const keys = (value as string[]).map((entry) => {
    const split = splitAuthority(entry)
    return split.port === null ? split.host : `${split.host}:${split.port}`
  })
  return [...new Set(keys)].sort().join(",")
}

/**
 * The comparable identity of `dial` for `adapter`, or `null` when it cannot
 * be computed (an unknown adapter, or a compared field missing or of the
 * wrong type).
 */
export function targetIdentity(adapter: string | null, dial: Dial): string | null {
  if (!adapter || !dial) return null
  let parts: (string | null)[]
  switch (adapter) {
    case "sql":
    case "cdc":
      parts = [text(dial.driver), host(dial.host), port(dial.port), text(dial.database)]
      break
    case "files": {
      const endpoint = dial.endpoint
      const named = typeof endpoint === "string" && endpoint.trim() !== "" ? origin(endpoint) : ""
      parts = [text(dial.protocol), endpoint == null || typeof endpoint === "string" ? named : null, text(dial.bucket)]
      break
    }
    case "rest": {
      const baseUrl = text(dial.baseUrl)
      parts = [baseUrl === null ? null : origin(baseUrl)]
      break
    }
    case "mongodb":
      parts = [hostPortSet(dial.hosts), text(dial.database)]
      break
    case "kafka":
      parts = [hostPortSet(dial.bootstrapServers)]
      break
    case "sftp": {
      const fingerprint = text(dial.hostKeyFingerprint)
      parts = [host(dial.host), port(dial.port), fingerprint === null ? null : fingerprint.trim()]
      break
    }
    case "sheets":
      parts = []
      break
    default:
      return null
  }
  if (parts.some((part) => part === null)) return null
  return JSON.stringify([adapter, ...parts])
}

/**
 * Whether saving `dial` over the stored `(storedAdapter, storedDial)` points
 * the connector somewhere else, so its credentials must be entered again.
 *
 * - A connector with no stored adapter and an empty dial has never been
 *   configured: its first dial is not a change (the server agrees).
 * - Anything that cannot be compared counts as a change: asking for the
 *   credential too often is the safe direction.
 */
export function targetChanged({
  storedAdapter,
  storedDial,
  adapter,
  dial,
}: {
  storedAdapter: string | null
  storedDial: Dial
  adapter: string | null
  dial: Dial
}): boolean {
  if (storedAdapter === null) {
    return !(storedDial == null || Object.keys(storedDial).length === 0)
  }
  const before = targetIdentity(storedAdapter, storedDial)
  const after = targetIdentity(adapter, dial)
  return before === null || after === null || before !== after
}
