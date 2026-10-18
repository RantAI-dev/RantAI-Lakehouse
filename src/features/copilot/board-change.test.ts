import { describe, expect, it } from "bun:test"

import { notifyDashboardsChanged, useDashboardsChanged } from "@/features/dashboards/dashboard-events"
import { changedABoard } from "./board-change"

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
