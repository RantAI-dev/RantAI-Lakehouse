// `WEB-7` D5: the rule for "change the saved query the tab came from, or
// make a copy" is one pure function now that two surfaces save: the editor
// tab and the Save query page. A change never carries a folder (`QS-3` C4).

import { describe, expect, it } from "bun:test"
import type { SaveQueryInput } from "@/services/contracts/queries"
import { savedQueryRequest } from "./saved-query-write"

const INPUT: SaveQueryInput = {
  title: "Orders",
  sql: "SELECT 2",
  tags: ["finance"],
  folderId: "f1",
}

describe("savedQueryRequest", () => {
  it("changes the saved query the tab came from, and writes no folder", () => {
    expect(savedQueryRequest({ mode: "update", savedId: "q1" }, INPUT)).toEqual({
      kind: "update",
      id: "q1",
      body: { title: "Orders", sql: "SELECT 2", tags: ["finance"] },
    })
  })

  it("carries the definitions of the SQL being saved, empty list included", () => {
    const request = savedQueryRequest(
      { mode: "update", savedId: "q1" },
      { ...INPUT, parameters: [] }
    )
    expect(request.kind === "update" && request.body.parameters).toEqual([])
    // Absent leaves the saved query's definitions as they are.
    const kept = savedQueryRequest({ mode: "update", savedId: "q1" }, INPUT)
    expect(kept.kind === "update" && "parameters" in kept.body).toBe(false)
  })

  it("makes a copy when the tab came from nothing, or from a query it may not change", () => {
    const fresh = savedQueryRequest({ mode: "create", savedId: null }, INPUT)
    expect(fresh).toEqual({ kind: "create", input: INPUT })
    // A copy keeps the statement and the tags; the folder is the caller's to pick.
    expect(savedQueryRequest({ mode: "create", savedId: "q1" }, INPUT)).toEqual({
      kind: "create",
      input: INPUT,
    })
  })

  it("makes a copy rather than a change when no id is there to change", () => {
    expect(savedQueryRequest({ mode: "update", savedId: null }, INPUT)).toEqual({
      kind: "create",
      input: INPUT,
    })
  })
})
