/**
 * Picking a `set_board_view_filters` result from a Copilot trace and
 * deciding whether it applies to the dashboard the page is showing. The
 * tool is board-scoped: it only changes the view for the board it names,
 * so the same result applied to a different board would be a leak from
 * another conversation. Returns the filters to apply, or `null` when the
 * latest tool call either was not a view-filter call, was for another
 * board, or failed.
 *
 * Pure so the rule is testable without the page or the Copilot brain.
 */

import type { FilterDef } from "@/services/clients/bi-store"
import type { Msg } from "@/features/copilot/use-copilot"

/**
 * The newest `set_board_view_filters` step in the conversation whose
 * `board` arg matches the page's board; `null` when no such step exists
 * (or every match failed).
 */
export function extractViewFiltersFromChat(messages: Msg[], currentBoard: string): FilterDef[] | null {
  if (!currentBoard) return null
  // Walk newest assistant message first; within it, newest tool step first.
  // The Copilot adds new tool steps to the end of `msg.tools`, so the latest
  // assistant answer is `messages[messages.length - 1]`.
  for (let i = messages.length - 1; i >= 0; i--) {
    const msg = messages[i]
    if (!msg || msg.role !== "assistant") continue
    const steps = msg.tools ?? []
    for (let j = steps.length - 1; j >= 0; j--) {
      const step = steps[j]
      if (!step || step.tool !== "set_board_view_filters") continue
      if (!step.ok) return null
      const args = step.args && typeof step.args === "object" ? (step.args as Record<string, unknown>) : {}
      const stepBoard = typeof args.board === "string" ? args.board : ""
      if (stepBoard !== currentBoard) return null
      // The trace's `result.filters` is canonical for the live page; fall back
      // to `args.filters` only when the tool did not produce a structured
      // result (an older schema).
      const result = step.result as { filters?: unknown } | undefined
      const rawFilters = Array.isArray(result?.filters)
        ? (result!.filters as unknown[])
        : Array.isArray(args.filters)
          ? (args.filters as unknown[])
          : null
      // An empty list is an ANSWER, not the absence of one: the tool says an
      // empty list clears the view, and the page turns that into a cleared
      // address. Collapsing it to `null` here would read "clear my filters"
      // as "nothing to do" and leave the old filters in the URL. Only a step
      // that carried no filter list at all has nothing to say.
      if (!rawFilters) return null
      const parsed: FilterDef[] = []
      for (const raw of rawFilters) {
        if (!raw || typeof raw !== "object") continue
        const f = raw as FilterDef
        if (typeof f.column === "string" && f.column) parsed.push(f)
      }
      return parsed
    }
    // The newest assistant answer had tool steps but none were the view
    // filter for `currentBoard`: stop here so an earlier matching one does
    // not re-apply.
    return null
  }
  return null
}
