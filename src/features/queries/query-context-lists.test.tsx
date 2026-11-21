// The rail lists beside the editor. A saved query loaded from the list brings
// its parameter definitions with its SQL (`QS-5`); a History entry brings
// only the statement that ran.

import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"
import type { ServiceState } from "@/hooks/use-service"
import type { QueryHistoryItem, SavedQuery } from "@/services/contracts/queries"
import { HistoryQuickList, SavedQuickList } from "./query-context-lists"

afterEach(cleanup)

const SAVED: SavedQuery = {
  id: "q1",
  title: "By region",
  sql: "SELECT {{region}}",
  owner: "Rina Wijaya",
  updatedAt: "2026-11-04T10:00:00.000Z",
  tags: [],
  folderId: null,
  ownerId: "u1",
  level: "edit",
  parameters: [{ name: "region", type: "text", default: "EU" }],
}

describe("SavedQuickList", () => {
  it("loads a saved query's SQL together with its definitions", () => {
    const onLoadSql = mock<(sql: string, definitions?: SavedQuery["parameters"]) => void>(() => {})
    const state = { status: "success", data: [SAVED], error: null, reload: () => {} } as ServiceState<
      SavedQuery[]
    > & { reload: () => void }
    render(<SavedQuickList state={state} onLoadSql={onLoadSql} />)

    fireEvent.click(screen.getByRole("button", { name: /By region/ }))

    expect(onLoadSql).toHaveBeenCalledWith("SELECT {{region}}", SAVED.parameters)
  })
})

describe("HistoryQuickList", () => {
  // QS-11 (LH-QS-4-02), D13: the side panel calls a blocked run what History
  // calls it, so the two lists never disagree about a refusal.
  it("reads a blocked run as Refused, not Blocked", () => {
    const onLoadSql = mock<(sql: string) => void>(() => {})
    const item = {
      id: "h2",
      sql: "SELECT 1",
      status: "blocked",
      at: "2026-11-04T10:00:00.000Z",
      durationMs: 5,
    } as unknown as QueryHistoryItem
    const state = { status: "success", data: [item], error: null, reload: () => {} } as ServiceState<
      QueryHistoryItem[]
    > & { reload: () => void }
    render(<HistoryQuickList state={state} onLoadSql={onLoadSql} />)

    expect(screen.getByText("Refused")).toBeTruthy()
    expect(screen.queryByText("Blocked")).toBeNull()
  })

  it("loads only the statement that ran", () => {
    const onLoadSql = mock<(sql: string, definitions?: SavedQuery["parameters"]) => void>(() => {})
    const item = {
      id: "h1",
      sql: "SELECT 'EU'",
      status: "success",
      startedAt: "2026-11-04T10:00:00.000Z",
      durationMs: 5,
    } as unknown as QueryHistoryItem
    const state = { status: "success", data: [item], error: null, reload: () => {} } as ServiceState<
      QueryHistoryItem[]
    > & { reload: () => void }
    render(<HistoryQuickList state={state} onLoadSql={onLoadSql} />)

    fireEvent.click(screen.getByTitle("SELECT 'EU'"))

    expect(onLoadSql).toHaveBeenCalledTimes(1)
    expect(onLoadSql.mock.calls[0]).toEqual(["SELECT 'EU'"])
  })
})
