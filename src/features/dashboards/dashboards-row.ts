import * as React from "react"

/**
 * The memory of the sidebar's "Dashboards" row: the dashboard or report
 * opened last, so the row is named after it and goes straight to it.
 *
 * Kept apart from `last-board.ts`, which feeds the `/dashboards` resolver and
 * Home: opening Browse empties this row, but must not make Home lose its last
 * board.
 *
 * In `localStorage`, per browser. A change in this tab is announced with an
 * event, one in another tab through `storage`.
 *
 * It belongs to one signed-in person (PR #541 review SHOULD-FIX 1, security):
 * it holds a dashboard's or report's title, and a browser is shared. The
 * entry records whose it is (`user`, the account id), and is read and written
 * only for the account `AuthProvider` names with `setRowOwner`, once the
 * session has answered: another person's entry is removed when they sign in,
 * an entry with no owner is never shown, and sign-out removes it. So the next
 * person on the browser never sees the last title the previous one opened.
 * (`last-board.ts` stores an id only, never a name.)
 */
export type RowKind = "dashboard" | "report"
export type RowMemory = { kind: RowKind; id: string; name: string }

const ROW_KEY = "dashboards:menu-row"
const ROW_EVENT = "dashboards:menu-row-change"

/** The signed-in account the row is for; `null` before the session answers and after sign-out. */
let owner: string | null = null

function announce() {
  try {
    window.dispatchEvent(new Event(ROW_EVENT))
  } catch {
    /* ignore */
  }
}

function readStored(): string | null {
  if (typeof window === "undefined") return null
  try {
    return window.localStorage.getItem(ROW_KEY)
  } catch {
    return null
  }
}

/** The stored entry, whatever its owner, or `null` for none or a malformed one. */
function parseStored(raw: string | null): (RowMemory & { user: string }) | null {
  if (!raw) return null
  try {
    const v = JSON.parse(raw) as Partial<RowMemory & { user: string }>
    if ((v.kind !== "dashboard" && v.kind !== "report") || typeof v.id !== "string" || !v.id) return null
    if (typeof v.name !== "string" || !v.name.trim()) return null
    if (typeof v.user !== "string" || !v.user) return null
    return { kind: v.kind, id: v.id, name: v.name, user: v.user }
  } catch {
    return null
  }
}

/** The stored text when it is the signed-in person's own entry, else `null`. */
function readRaw(): string | null {
  const raw = readStored()
  return owner !== null && parseStored(raw)?.user === owner ? raw : null
}

function parse(raw: string | null): RowMemory | null {
  const v = parseStored(raw)
  return v ? { kind: v.kind, id: v.id, name: v.name } : null
}

export function readRow(): RowMemory | null {
  return parse(readRaw())
}

/**
 * Names the account the row belongs to: `AuthProvider` calls it with the
 * signed-in person's id once the session has answered, and with `null` on
 * sign-out or when there is no session. An entry stored for anybody else is
 * removed at once (PR #541 review SHOULD-FIX 1).
 */
export function setRowOwner(userId: string | null) {
  if (typeof window === "undefined") return
  const next = userId || null
  const raw = readStored()
  // Somebody else's entry, or one with no owner, is removed as soon as a person is known.
  const stale = next !== null && raw !== null && parseStored(raw)?.user !== next
  if (stale) {
    try {
      window.localStorage.removeItem(ROW_KEY)
    } catch {
      /* ignore */
    }
  }
  if (owner === next && !stale) return
  owner = next
  announce()
}

/** Called when a dashboard or report page opens, and again when its name changes. Needs a signed-in owner. */
export function rememberRow(kind: RowKind, id: string, name: string) {
  if (typeof window === "undefined" || owner === null || !id || !name.trim()) return
  try {
    const next = JSON.stringify({ kind, id, name, user: owner })
    if (window.localStorage.getItem(ROW_KEY) === next) return
    window.localStorage.setItem(ROW_KEY, next)
  } catch {
    return
  }
  announce()
}

/**
 * With no argument: empty the row (Browse was opened, or the person signs
 * out). With an item: only when it is the one remembered (it was deleted).
 */
export function forgetRow(item?: { kind: RowKind; id: string }) {
  if (typeof window === "undefined") return
  try {
    if (item) {
      const cur = readRow()
      if (!cur || cur.kind !== item.kind || cur.id !== item.id) return
    }
    if (window.localStorage.getItem(ROW_KEY) === null) return
    window.localStorage.removeItem(ROW_KEY)
  } catch {
    return
  }
  announce()
}

function subscribe(onChange: () => void) {
  window.addEventListener(ROW_EVENT, onChange)
  window.addEventListener("storage", onChange)
  return () => {
    window.removeEventListener(ROW_EVENT, onChange)
    window.removeEventListener("storage", onChange)
  }
}

/** The server and the first render give `null`, so the row says "Dashboards" with no hydration mismatch. */
export function useDashboardsRow(): RowMemory | null {
  const raw = React.useSyncExternalStore(subscribe, readRaw, () => null)
  return React.useMemo(() => parse(raw), [raw])
}
