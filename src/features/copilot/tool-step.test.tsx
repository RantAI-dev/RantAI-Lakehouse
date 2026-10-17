import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"
import { ToolStepCard, boardFromPath, toolLabel } from "./tool-step"
import type { ToolStep } from "./tool-step"

afterEach(cleanup)

// `usePathname` lives in `next/navigation`; the component reads it to tell a
// `set_board_view_filters` step which board is on screen. The happy-dom test
// environment does not provide it, so we stub the module before importing
// the component.
let mockPathname = "/dashboards/main-board"
mock.module("next/navigation", () => ({
  usePathname: () => mockPathname,
}))

describe("boardFromPath reads the board the page is showing", () => {
  it("takes the id out of a dashboard path and refuses every other path", () => {
    expect(boardFromPath("/dashboards/sales-board")).toBe("sales-board")
    expect(boardFromPath("/dashboards/sales-board/edit")).toBe("sales-board")
    expect(boardFromPath("/dashboards/other-board")).toBe("other-board")
    // The whole "for another board it does not" rule rests on this returning
    // something other than the step's board, so a similar path must not match.
    // An empty segment is not a board id, and `null` is the answer that keeps
    // it from ever comparing equal to a step's board.
    expect(boardFromPath("/dashboards/")).toBeNull()
    expect(boardFromPath("/dash/boards/sales-board")).toBeNull()
    expect(boardFromPath("/copilot")).toBeNull()
    expect(boardFromPath(null)).toBeNull()
  })

  it("decodes a percent-encoded board id", () => {
    expect(boardFromPath("/dashboards/board%20one")).toBe("board one")
  })
})

describe("toolLabel covers the six dashboard tools", () => {
  it("names each new dashboard tool so the Copilot does not fall back to its capitalized rule", () => {
    expect(toolLabel("rename_board")).toBe("Rename board")
    expect(toolLabel("delete_board")).toBe("Delete board")
    expect(toolLabel("arrange_board")).toBe("Arrange board")
    expect(toolLabel("move_chart")).toBe("Move chart")
    expect(toolLabel("set_board_view_filters")).toBe("Set view filters")
    expect(toolLabel("save_board_filters")).toBe("Save default filters")
  })
})

describe("ToolStepCard set_board_view_filters rendering", () => {
  const baseStep: ToolStep = {
    tool: "set_board_view_filters",
    args: { board: "main-board", filters: [{ column: "province", values: ["Bali"] }] },
    ok: true,
    result: { applyViewFilters: true, board: "main-board", filters: [{ column: "province", values: ["Bali"] }] },
  }

  it("shows 'Filters applied' and no link when the step's board is the one on screen", () => {
    mockPathname = "/dashboards/main-board"
    render(<ToolStepCard step={baseStep} />)
    fireEvent.click(screen.getByRole("button"))
    // The exact text appears; no link to another board is rendered.
    expect(screen.queryByText("✓ Filters applied")).not.toBeNull()
    expect(screen.queryByRole("link")).toBeNull()
  })

  it("links to the other board with the filters in the address when the step targets a different board", () => {
    mockPathname = "/dashboards/main-board"
    const otherStep: ToolStep = {
      ...baseStep,
      args: { board: "sales-board", filters: baseStep.result && typeof baseStep.result === "object" ? (baseStep.result as { filters?: unknown[] }).filters ?? [] : [] },
    }
    render(<ToolStepCard step={otherStep} />)
    fireEvent.click(screen.getByRole("button"))
    // The 'Filters applied' confirmation would be wrong here.
    expect(screen.queryByText("✓ Filters applied")).toBeNull()
    const link = screen.getByRole("link") as HTMLAnchorElement
    expect(link.getAttribute("href")?.startsWith("/dashboards/sales-board")).toBe(true)
    expect(link.getAttribute("href")).toContain("f=")
  })
})

describe("ToolStepCard export links (BI-1, AI-10)", () => {
  const step = (tool: string, result: unknown): ToolStep => ({ tool, args: {}, ok: true, result })

  it("labels the two export tools", () => {
    expect(toolLabel("export_dashboard")).toBe("Export dashboard")
    expect(toolLabel("export_chart")).toBe("Export chart data")
  })

  it("renders a download link to the console's own PDF route", () => {
    render(<ToolStepCard step={step("export_dashboard", { url: "/api/dashboard/export/pdf?board=b1&paper=a4&orientation=portrait", fileName: "Sales-2026-10-17.pdf" })} />)
    fireEvent.click(screen.getByRole("button"))
    const link = screen.getByRole("link", { name: /Download Sales-2026-10-17\.pdf/ }) as HTMLAnchorElement
    expect(link.getAttribute("href")).toBe("/api/dashboard/export/pdf?board=b1&paper=a4&orientation=portrait")
    expect(link.hasAttribute("download")).toBe(true)
  })

  it("renders a download link for a chart's data", () => {
    render(<ToolStepCard step={step("export_chart", { url: "/api/dashboard/chart-export?chart=c1&format=xlsx&values=raw", fileName: "Visits.xlsx" })} />)
    fireEvent.click(screen.getByRole("button"))
    expect((screen.getByRole("link") as HTMLAnchorElement).getAttribute("href")).toContain("/api/dashboard/chart-export?chart=c1")
  })

  it("never links to an address that is not one of the two download routes", () => {
    render(<ToolStepCard step={step("export_dashboard", { url: "https://evil.example/x?y=1", fileName: "x.pdf" })} />)
    fireEvent.click(screen.getByRole("button"))
    expect(screen.queryByRole("link")).toBeNull()
  })

  it("shows the refusal sentence instead of a link", () => {
    render(<ToolStepCard step={{ tool: "export_dashboard", args: {}, ok: false, result: { error: "PDF export is not set up on this deployment" } }} />)
    fireEvent.click(screen.getByRole("button"))
    expect(screen.getByText(/PDF export is not set up on this deployment/)).toBeTruthy()
    expect(screen.queryByRole("link")).toBeNull()
  })
})
