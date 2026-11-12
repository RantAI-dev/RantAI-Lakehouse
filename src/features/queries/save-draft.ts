/**
 * The handoff from Query Studio to the two save pages (`WEB-7` D5).
 *
 * Save query and Save as SQL source are pages of their own now, so the page
 * they open needs what the editor holds: the statement, its parameter
 * definitions, and whether saving changes the query the tab came from or
 * makes a copy. That travels in `sessionStorage` under one key per page, and
 * never in the URL: SQL is not a secret, but it is long, and an address
 * someone copies must not carry a half-written query (`WEB-7` D5).
 *
 * A draft is deleted after a successful save. Cancel leaves it in place, so
 * going back to the page finds the same statement again. Neither key ever
 * holds a credential — the shown-once invite password has its own in-memory
 * store (`features/admin/invite-password.ts`) for exactly that reason.
 *
 * Every read is guarded and parsed: a stale or hand-edited entry is `null`
 * rather than an exception, and the page then says to open it from Query
 * Studio. Storage throws in a private window and is absent during server
 * rendering.
 */

import type { QueryParamDef } from "@/services/contracts/queries"

export const SAVE_QUERY_DRAFT_KEY = "lakehouse.saveQueryDraft"
export const SAVE_SOURCE_DRAFT_KEY = "lakehouse.saveSourceDraft"

/** What the Save query page needs from the editor. */
export type SaveQueryDraft = {
  /** The statement the editor would save, as `targetStatement` gives it. */
  sql: string
  /** The definitions of the `{{name}}` names that statement uses, defaults included. */
  parameters: QueryParamDef[]
  /** "update" changes `savedId`; "create" makes a new query. */
  mode: "create" | "update"
  /** The saved query the tab came from, when it came from one. */
  savedId: string | null
  /**
   * The query tab the save started from. The page hands the saved id back to
   * that tab (`rememberSavedTab`), so the editor still says "Save" and
   * changes the copy it just made instead of making another one.
   */
  tabId: string
}

/** What the Save as SQL source page needs from the editor. */
export type SaveSourceDraft = { sql: string }

function readJson(key: string): unknown {
  try {
    const raw = window.sessionStorage.getItem(key)
    return raw === null ? null : (JSON.parse(raw) as unknown)
  } catch {
    return null
  }
}

function writeJson(key: string, value: unknown): void {
  try {
    window.sessionStorage.setItem(key, JSON.stringify(value))
  } catch {
    // A draft that cannot be stored is a lost convenience: the page then
    // asks the person to open it from Query Studio, which is honest.
  }
}

function clear(key: string): void {
  try {
    window.sessionStorage.removeItem(key)
  } catch {
    // Nothing to clear if storage is refused.
  }
}

function isMode(value: unknown): value is "create" | "update" {
  return value === "create" || value === "update"
}

function isDefinition(value: unknown): value is QueryParamDef {
  if (typeof value !== "object" || value === null) return false
  const def = value as Partial<QueryParamDef>
  return typeof def.name === "string" && def.name.trim().length > 0 && typeof def.type === "string"
}

/** The stored draft, or `null` when there is nothing usable. */
export function readSaveQueryDraft(): SaveQueryDraft | null {
  const raw = readJson(SAVE_QUERY_DRAFT_KEY)
  if (typeof raw !== "object" || raw === null) return null
  const draft = raw as Partial<SaveQueryDraft>
  if (typeof draft.sql !== "string" || !draft.sql.trim()) return null
  if (!isMode(draft.mode)) return null
  if (typeof draft.tabId !== "string" || !draft.tabId) return null
  const savedId = typeof draft.savedId === "string" && draft.savedId ? draft.savedId : null
  // A change names the query it changes: a draft that lost the id is a copy.
  if (draft.mode === "update" && savedId === null) return null
  const parameters = Array.isArray(draft.parameters)
    ? draft.parameters.filter(isDefinition).map((def) => ({ ...def }))
    : []
  return { sql: draft.sql, parameters, mode: draft.mode, savedId, tabId: draft.tabId }
}

export function writeSaveQueryDraft(draft: SaveQueryDraft): void {
  writeJson(SAVE_QUERY_DRAFT_KEY, draft)
}

export function clearSaveQueryDraft(): void {
  clear(SAVE_QUERY_DRAFT_KEY)
}

/** The stored SQL source draft, or `null` when there is nothing usable. */
export function readSaveSourceDraft(): SaveSourceDraft | null {
  const raw = readJson(SAVE_SOURCE_DRAFT_KEY)
  if (typeof raw !== "object" || raw === null) return null
  const draft = raw as Partial<SaveSourceDraft>
  if (typeof draft.sql !== "string" || !draft.sql.trim()) return null
  return { sql: draft.sql }
}

export function writeSaveSourceDraft(draft: SaveSourceDraft): void {
  writeJson(SAVE_SOURCE_DRAFT_KEY, draft)
}

export function clearSaveSourceDraft(): void {
  clear(SAVE_SOURCE_DRAFT_KEY)
}
