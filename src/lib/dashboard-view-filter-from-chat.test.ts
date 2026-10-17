import { strict as assert } from "node:assert"
import { test } from "node:test"
import { extractViewFiltersFromChat } from "./dashboard-view-filter-from-chat"
import type { Msg } from "@/features/copilot/use-copilot"

const makeMsg = (tools: { tool: string; args: unknown; ok: boolean; result: unknown }[]): Msg[] => [
  { id: "u", role: "user", content: "go", at: "2026-10-01T00:00:00Z" },
  { id: "a", role: "assistant", content: "done", at: "2026-10-01T00:00:05Z", tools },
]

test("returns the filters when a step's board matches the page's board", () => {
  const messages = makeMsg([
    {
      tool: "set_board_view_filters",
      args: { board: "sales-board", filters: [{ column: "province", values: ["Bali"] }] },
      ok: true,
      result: { applyViewFilters: true, board: "sales-board", filters: [{ column: "province", values: ["Bali"] }] },
    },
  ])
  const out = extractViewFiltersFromChat(messages, "sales-board")
  assert.deepEqual(out, [{ column: "province", values: ["Bali"] }])
})

test("returns null when the step's board is not the one on screen (the negative half)", () => {
  const messages = makeMsg([
    {
      tool: "set_board_view_filters",
      args: { board: "other-board", filters: [{ column: "province", values: ["Bali"] }] },
      ok: true,
      result: { applyViewFilters: true, board: "other-board", filters: [{ column: "province", values: ["Bali"] }] },
    },
  ])
  const out = extractViewFiltersFromChat(messages, "sales-board")
  assert.equal(out, null)
})

test("returns an EMPTY list when the tool cleared the view, so the page can clear it", () => {
  // The tool's schema says an empty list clears the view. `null` here would
  // mean "nothing to apply" and the page returns early, leaving the old
  // filters in the address -- so this must be `[]`, not `null`.
  const messages = makeMsg([
    {
      tool: "set_board_view_filters",
      args: { board: "sales-board", filters: [] },
      ok: true,
      result: { applyViewFilters: true, board: "sales-board", filters: [] },
    },
  ])
  const out = extractViewFiltersFromChat(messages, "sales-board")
  assert.deepEqual(out, [])
})

test("returns null when the step carried no filter list at all", () => {
  // The other half: a result with no `filters` key has nothing to apply,
  // which must stay distinct from an empty list.
  const messages = makeMsg([
    {
      tool: "set_board_view_filters",
      args: { board: "sales-board" },
      ok: true,
      result: { applyViewFilters: true, board: "sales-board" },
    },
  ])
  assert.equal(extractViewFiltersFromChat(messages, "sales-board"), null)
})

test("returns null when the step is failed", () => {
  const messages = makeMsg([
    {
      tool: "set_board_view_filters",
      args: { board: "sales-board", filters: [{ column: "province", values: ["Bali"] }] },
      ok: false,
      result: { error: "refused" },
    },
  ])
  const out = extractViewFiltersFromChat(messages, "sales-board")
  assert.equal(out, null)
})

test("returns null when no set_board_view_filters step is in the trace", () => {
  const messages = makeMsg([
    { tool: "list_charts", args: {}, ok: true, result: { charts: [] } },
    { tool: "create_chart", args: { sql: "SELECT 1" }, ok: true, result: { created: true } },
  ])
  const out = extractViewFiltersFromChat(messages, "sales-board")
  assert.equal(out, null)
})

test("uses the most recent assistant message's last matching step", () => {
  // Two assistant answers each with a step: the newer one wins.
  const messages: Msg[] = [
    { id: "a1", role: "assistant", content: "first", tools: [
      {
        tool: "set_board_view_filters",
        args: { board: "sales-board", filters: [{ column: "province", values: ["Bali"] }] },
        ok: true,
        result: { applyViewFilters: true, board: "sales-board", filters: [{ column: "province", values: ["Bali"] }] },
      },
    ] },
    { id: "u2", role: "user", content: "and Aceh too", at: "2026-10-02T00:00:00Z" },
    { id: "a2", role: "assistant", content: "second", tools: [
      {
        tool: "set_board_view_filters",
        args: { board: "sales-board", filters: [{ column: "province", values: ["Aceh"] }] },
        ok: true,
        result: { applyViewFilters: true, board: "sales-board", filters: [{ column: "province", values: ["Aceh"] }] },
      },
    ] },
  ]
  const out = extractViewFiltersFromChat(messages, "sales-board")
  assert.deepEqual(out, [{ column: "province", values: ["Aceh"] }])
})
