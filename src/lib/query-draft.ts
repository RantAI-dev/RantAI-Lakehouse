/**
 * The Query Studio draft: what someone had typed when they last left.
 *
 * Kept in `localStorage` rather than the URL — a half-written query is a
 * private scratchpad, not something to put in a shareable address — and
 * read through a parser that never trusts what it finds there, since a
 * stale or hand-edited entry must not break the page.
 *
 * The draft is a set of query tabs now: each keeps its own name, SQL and
 * engine, and the browser keeps the active one. The single draft that came
 * before is read as one tab, so the first read after this change finds the
 * query that was there and nothing else.
 */

import type { QueryEngine } from "@/services/contracts/queries"

export const QUERY_DRAFT_KEY = "rantai-lakehouse:query-studio-draft"

/**
 * How many query tabs one browser keeps. A browser that could hold more
 * would lose them to a quota nobody asked for; the limit is the number the
 * feature page tells a customer about.
 */
export const MAX_QUERY_TABS = 20

export type QueryDraft = {
  sql: string
  question: string
  tab: "nl" | "sql"
}

/** One query tab: what it is called, what it holds and what it runs on. */
export type QueryTab = {
  id: string
  name: string
  sql: string
  engine: QueryEngine
}

/** The tabs as they are kept in the browser. */
export type QueryTabs = {
  tabs: QueryTab[]
  activeId: string
  /** The natural-language question and which of the two editors is open, as today. */
  question: string
  tab: "nl" | "sql"
}

/** A stored draft, or `null` when there is nothing usable to restore. */
export function parseDraft(raw: string | null): QueryDraft | null {
  if (!raw) return null
  let parsed: unknown
  try {
    parsed = JSON.parse(raw)
  } catch {
    return null
  }
  if (typeof parsed !== "object" || parsed === null) return null
  const draft = parsed as Partial<QueryDraft>
  const sql = typeof draft.sql === "string" ? draft.sql : ""
  const question = typeof draft.question === "string" ? draft.question : ""
  if (!sql.trim() && !question.trim()) return null
  return {
    sql,
    question,
    tab: draft.tab === "sql" ? "sql" : "nl",
  }
}

function isEngine(value: unknown): value is QueryEngine {
  return value === "clickhouse" || value === "trino"
}

/** A tab as it is stored, or `null` when a field is missing or wrong. */
function parseTab(value: unknown): QueryTab | null {
  if (typeof value !== "object" || value === null) return null
  const tab = value as Partial<QueryTab>
  if (typeof tab.id !== "string" || !tab.id.trim()) return null
  if (typeof tab.name !== "string") return null
  if (typeof tab.sql !== "string") return null
  if (!isEngine(tab.engine)) return null
  return { id: tab.id, name: tab.name, sql: tab.sql, engine: tab.engine }
}

/**
 * The tabs in `raw`, or `null` when there is nothing usable to restore.
 *
 * Two shapes are read. The current one holds a list of tabs and the id of
 * the active one; a tab that is missing a field, that repeats an id or
 * that is past the limit of 20 is dropped rather than repaired, because a
 * hand-edited entry must not decide what the editor shows. The shape that
 * came before held one draft and no tabs: it is read as one tab, so the
 * query that was open is still there after this change.
 */
export function parseTabs(raw: string | null): QueryTabs | null {
  if (!raw) return null
  let parsed: unknown
  try {
    parsed = JSON.parse(raw)
  } catch {
    return null
  }
  if (typeof parsed !== "object" || parsed === null) return null
  const stored = parsed as {
    tabs?: unknown
    activeId?: unknown
    question?: unknown
    tab?: unknown
    sql?: unknown
  }
  const question = typeof stored.question === "string" ? stored.question : ""
  const editor = stored.tab === "sql" ? "sql" : "nl"

  if (Array.isArray(stored.tabs)) {
    const kept: QueryTab[] = []
    const ids = new Set<string>()
    for (const entry of stored.tabs) {
      if (kept.length >= MAX_QUERY_TABS) break
      const tab = parseTab(entry)
      if (!tab || ids.has(tab.id)) continue
      ids.add(tab.id)
      kept.push(tab)
    }
    if (kept.length === 0) return null
    const activeId =
      typeof stored.activeId === "string" && ids.has(stored.activeId)
        ? stored.activeId
        : kept[0].id
    return { tabs: kept, activeId, question, tab: editor }
  }

  // The shape that came before: one draft, no tabs. It becomes one tab.
  const sql = typeof stored.sql === "string" ? stored.sql : ""
  if (!sql.trim() && !question.trim()) return null
  return {
    tabs: [{ id: "tab-1", name: "Query 1", sql, engine: "clickhouse" }],
    activeId: "tab-1",
    question,
    tab: editor,
  }
}

/**
 * Read the draft. Every access is guarded: storage throws in a private
 * window and is simply absent during server rendering.
 */
export function readDraft(): QueryDraft | null {
  try {
    return parseDraft(window.localStorage.getItem(QUERY_DRAFT_KEY))
  } catch {
    return null
  }
}

/** Store the draft, or clear it when there is nothing left to keep. */
export function writeDraft(draft: QueryDraft): void {
  try {
    if (!draft.sql.trim() && !draft.question.trim()) {
      window.localStorage.removeItem(QUERY_DRAFT_KEY)
      return
    }
    window.localStorage.setItem(QUERY_DRAFT_KEY, JSON.stringify(draft))
  } catch {
    // A draft that cannot be saved is a lost convenience, not an error
    // worth interrupting anyone over.
  }
}

/**
 * Read the tabs. Every access is guarded, like `readDraft`: storage throws
 * in a private window and is simply absent during server rendering.
 */
export function readTabs(): QueryTabs | null {
  try {
    return parseTabs(window.localStorage.getItem(QUERY_DRAFT_KEY))
  } catch {
    return null
  }
}

/**
 * Store the tabs under the same key the old draft used, so the shape in
 * the browser tells the two apart on the next read.
 */
export function writeTabs(tabs: QueryTabs): void {
  try {
    window.localStorage.setItem(QUERY_DRAFT_KEY, JSON.stringify(tabs))
  } catch {
    // A draft that cannot be saved is a lost convenience, not an error
    // worth interrupting anyone over.
  }
}
