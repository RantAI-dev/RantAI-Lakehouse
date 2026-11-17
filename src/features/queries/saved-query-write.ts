/**
 * Which write a Save sends (`WEB-7` D5).
 *
 * Saving changes the saved query the editor's tab came from when the caller
 * may change it, and makes a copy otherwise. That rule used to live inside
 * the editor; it is a pure function now because two callers need it: the tab
 * (`useQueryStudio`) and the Save query page, which owns the write since
 * saving became a page of its own (`save-query-page.tsx`). One rule, so the
 * two cannot disagree about whether a save is a change or a copy.
 *
 * A change never carries a folder: where a saved query sits is the Move
 * dialog's business (`QS-3` C4).
 */

import type {
  SaveQueryInput,
  UpdateSavedQueryInput,
} from "@/services/contracts/queries"

/** What the editor knows about the tab being saved. */
export type SavedQueryTarget = {
  /** "update" changes `savedId`; "create" makes a new query. */
  mode: "create" | "update"
  /** The saved query the tab came from, when it came from one. */
  savedId: string | null
}

export type SavedQueryRequest =
  | { kind: "update"; id: string; body: UpdateSavedQueryInput }
  | { kind: "create"; input: SaveQueryInput }

/** A change of `target.savedId` when there is one, a copy otherwise. */
export function savedQueryRequest(
  target: SavedQueryTarget,
  input: SaveQueryInput
): SavedQueryRequest {
  if (target.mode === "update" && target.savedId) {
    return {
      kind: "update",
      id: target.savedId,
      body: {
        title: input.title,
        sql: input.sql,
        tags: input.tags,
        // The definitions of the SQL being saved. An empty list clears
        // them; absent leaves the query's as they are.
        ...(input.parameters !== undefined && { parameters: input.parameters }),
      },
    }
  }
  return { kind: "create", input }
}
