/**
 * The Query Studio draft: what someone had typed when they last left.
 *
 * Kept in `localStorage` rather than the URL — a half-written query is a
 * private scratchpad, not something to put in a shareable address — and
 * read through a parser that never trusts what it finds there, since a
 * stale or hand-edited entry must not break the page.
 */

export const QUERY_DRAFT_KEY = "rantai-lakehouse:query-studio-draft"

export type QueryDraft = {
  sql: string
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
