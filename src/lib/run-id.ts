/**
 * The id of one run of queries (`BI-5` plan D5): the console sends it as
 * `X-Run-Id` with a dashboard load or a Query Studio run, and Stop names it in
 * `POST /api/query/cancel { runId }`, so the server can end the person's own
 * queries of that run and no one else's. A UUID v4, because the server checks
 * the id is one before it uses it in a statement.
 */
export function newRunId(): string {
  const c = typeof globalThis !== "undefined" ? globalThis.crypto : undefined
  if (c && typeof c.randomUUID === "function") return c.randomUUID()
  // An insecure context (plain http on a LAN) has no `randomUUID`; `getRandomValues` is there.
  const bytes = new Uint8Array(16)
  if (c && typeof c.getRandomValues === "function") c.getRandomValues(bytes)
  else for (let i = 0; i < bytes.length; i++) bytes[i] = Math.floor(Math.random() * 256)
  bytes[6] = (bytes[6] & 0x0f) | 0x40
  bytes[8] = (bytes[8] & 0x3f) | 0x80
  const hex = [...bytes].map((b) => b.toString(16).padStart(2, "0")).join("")
  return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`
}

/** The header the run id travels in (`BI-5` plan D5). */
export const RUN_ID_HEADER = "X-Run-Id"
