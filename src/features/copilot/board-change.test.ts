import { describe, expect, it } from "bun:test"

import { notifyDashboardsChanged, useDashboardsChanged } from "@/features/dashboards/dashboard-events"
import { changedABoard, takeNewBoardChanges } from "./board-change"

describe("an assistant change to a board reloads the open page (BI-18 part C review fix (SHOULD-FIX) R23)", () => {
  it("counts a tab, layout, format or theme tool that worked", () => {
    for (const tool of ["move_charts_to_tab", "create_tab", "format_chart", "set_dashboard_theme", "arrange_board", "update_chart"]) {
      expect(changedABoard({ tool, ok: true, result: { updated: true, board: "b1" } })).toBe(true)
    }
  })

  it("does not count a read, a refusal, a cancel or a pending approval", () => {
    expect(changedABoard({ tool: "list_charts", ok: true, result: {} })).toBe(false)
    expect(changedABoard({ tool: "move_charts_to_tab", ok: false, result: { error: "no" } })).toBe(false)
    expect(changedABoard({ tool: "move_charts_to_tab", ok: true, result: { error: "no" } })).toBe(false)
    expect(changedABoard({ tool: "delete_tab", ok: false, result: { cancelled: true } })).toBe(false)
    expect(changedABoard({ tool: "delete_tab", result: { needsApproval: true } })).toBe(false)
  })

  it("the page's listener runs when the event is raised", () => {
    // The page subscribes with useDashboardsChanged (a window event); raising it reloads the board.
    let reloads = 0
    const on = () => { reloads += 1 }
    window.addEventListener("dashboards:changed", on)
    notifyDashboardsChanged()
    window.removeEventListener("dashboards:changed", on)
    expect(reloads).toBe(1)
    expect(typeof useDashboardsChanged).toBe("function")
  })
})

// BI-18 part C review fix (SHOULD-FIX) R29.
describe("one reload per confirmed board-changing result", () => {
  it("a streamed turn with several trace updates and one confirmed change reloads exactly once", () => {
    const seen = new Set<string>()
    const read = { tool: "list_charts", ok: true, result: {} }
    const change = { tool: "move_charts_to_tab", ok: true, result: { updated: true, board: "b1" } }
    let reloads = 0
    // The trace grows as the turn streams; the same message is offered again each time.
    for (const trace of [[read], [read, change], [read, change], [read, change, read], [read, change, read]]) {
      if (takeNewBoardChanges("m1", trace, seen)) reloads += 1
    }
    expect(reloads).toBe(1)
  })

  it("never reloads on reads or a partial trace, and reloads again for a second change", () => {
    const seen = new Set<string>()
    expect(takeNewBoardChanges("m1", [{ tool: "list_charts", ok: true, result: {} }], seen)).toBe(false)
    expect(takeNewBoardChanges("m1", [{ tool: "format_chart", ok: true, result: { error: "no" } }], seen)).toBe(false)
    expect(takeNewBoardChanges("m1", [{ tool: "create_tab", ok: true, result: { updated: true } }], seen)).toBe(true)
    expect(takeNewBoardChanges("m2", [{ tool: "create_tab", ok: true, result: { updated: true } }], seen)).toBe(true)
  })
})
