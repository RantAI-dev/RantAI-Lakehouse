import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"
import { ToolStepCard, boardFromPath, exportLink, toolLabel } from "./tool-step"
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

describe("an export step links only to the console's own downloads", () => {
  // BI-37 D7: the dashboard file is a link like the PDF and the chart data, and
  // the card must never turn any other string into one.
  it("admits the PDF, the chart data and the dashboard file, and nothing else", () => {
    const step = (url: string) => exportLink("export_dashboard", { url, fileName: "f" })
    expect(step("/api/dashboard/export/pdf?board=b1&paper=a4")).not.toBeNull()
    expect(step("/api/dashboard/export?format=file&board=b1")).not.toBeNull()
    expect(exportLink("export_chart", { url: "/api/dashboard/chart-export?chart=c1&format=png" })).not.toBeNull()
    for (const url of [
      "/api/dashboard/export?format=yaml",
      "/api/dashboard/export?board=b1",
      "https://evil.example/api/dashboard/export/pdf?board=b1",
      "/api/alerts?id=1",
    ]) {
      expect(step(url)).toBeNull()
    }
  })

  it("is not the card of any other tool", () => {
    expect(exportLink("list_dashboard_comments", { url: "/api/dashboard/export/pdf?board=b1" })).toBeNull()
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
    expect(toolLabel("create_report_subscription")).toBe("Schedule report")
    expect(toolLabel("delete_report_subscription")).toBe("Delete scheduled report")
  })

  // BI-1 QA fix A1: the Download button is on the card itself, collapsed or not,
  // and its address is the tool result's own `url`.
  it("shows a Download button on the collapsed card, built from the tool result's url", () => {
    render(<ToolStepCard step={{ tool: "export_dashboard", args: { board: "b1", url: "/api/dashboard/export/pdf?evil=1" }, ok: true, result: { url: "/api/dashboard/export/pdf?board=b1&paper=a4&orientation=portrait", fileName: "Sales-2026-10-17.pdf" } }} />)
    const link = screen.getByRole("link", { name: /Download Sales-2026-10-17\.pdf/ }) as HTMLAnchorElement
    expect(link.getAttribute("href")).toBe("/api/dashboard/export/pdf?board=b1&paper=a4&orientation=portrait")
    expect(link.hasAttribute("download")).toBe(true)
    // The details stay collapsed.
    expect(screen.getByRole("button").getAttribute("aria-expanded")).toBe("false")
  })

  it("renders a download button for a chart's data", () => {
    render(<ToolStepCard step={step("export_chart", { url: "/api/dashboard/chart-export?chart=c1&format=xlsx&values=raw", fileName: "Visits.xlsx" })} />)
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

describe("a failed step's reference (AI-33)", () => {
  // Since #533 the server's fixed sentence carries the reference it logged
  // the real error under at its end, and no step carries `errorId`, so the
  // reference rode inside the sentence and could not be copied the way
  // every other error allows.
  it("shows the sentence and the reference in its own small line", () => {
    const { container } = render(
      <ToolStepCard
        step={{
          tool: "run_sql",
          args: { sql: "SELECT nope FROM serving.mart_x" },
          ok: false,
          result: { error: "This query names a column that does not exist. Reference: 3fc37a0664" },
        }}
      />,
    )
    fireEvent.click(screen.getByRole("button"))
    // The sentence, and no reference glued to it.
    expect(container.textContent).toContain("This query names a column that does not exist.")
    expect(container.textContent).not.toContain("does not exist. Reference:")
    // The reference, in the small selectable line the other errors use.
    expect(container.textContent).toContain("Reference: 3fc37a0664")
    const referenceLine = container.querySelector(".select-text")
    expect(referenceLine).not.toBeNull()
    expect(referenceLine?.textContent).toBe("Reference: 3fc37a0664")
  })

  it("shows a sentence without a reference as one block, with no reference line", () => {
    const { container } = render(
      <ToolStepCard
        step={{
          tool: "run_sql",
          args: { sql: "SELECT nope FROM serving.mart_x" },
          ok: false,
          result: { error: "This query names a column that does not exist." },
        }}
      />,
    )
    fireEvent.click(screen.getByRole("button"))
    expect(container.textContent).toContain("This query names a column that does not exist.")
    expect(container.textContent).not.toContain("Reference")
    expect(container.querySelector(".select-text")).toBeNull()
  })
})

describe("a step whose result was dropped from a long conversation (AI-24)", () => {
  const dropped: ToolStep = {
    tool: "run_sql",
    args: { sql: "SELECT count() FROM serving.mart_x" },
    ok: true,
    resultDropped: true,
  }

  it("keeps the step's label and its outcome", () => {
    render(<ToolStepCard step={dropped} />)
    // The label and the outcome come from the fields the server kept.
    expect(screen.getByText("SQL")).toBeTruthy()
    expect(screen.getByRole("button").className).not.toContain("text-destructive")
  })

  it("opens to the note instead of a result, with no raw view", () => {
    render(<ToolStepCard step={dropped} />)
    fireEvent.click(screen.getByRole("button"))
    expect(screen.getByText(/Details not kept for long conversations/)).toBeTruthy()
    expect(screen.queryByText("View raw")).toBeNull()
  })

  it("marks a dropped step that failed as failed", () => {
    render(<ToolStepCard step={{ ...dropped, ok: false }} />)
    expect(screen.getByText("SQL")).toBeTruthy()
    expect(screen.getByRole("button").className).toContain("text-destructive")
  })
})

describe("a reopened approval step shows what the run did (AI-28, LH-AI-18-09)", () => {
  const step: ToolStep = {
    tool: "delete_chart",
    args: { id: "c1" },
    ok: true,
    result: {
      needs_approval: true,
      approval_id: "appr-1",
      run_id: "r1",
      summary: "Moves chart c1 from its dashboard to the trash.",
    },
    approvalState: "ran",
  }

  it("shows the decided badge and line, never pending approval", () => {
    const { container } = render(<ToolStepCard step={step} />)
    expect(container.textContent).toContain("approved")
    expect(container.textContent).not.toContain("pending approval")
    fireEvent.click(container.querySelector("button")!)
    expect(container.textContent).toContain("Approved and run.")
    expect(container.textContent).not.toContain(
      "Someone else with approval rights must approve it."
    )
  })

  it("reads 'the action ran and failed' for an executed failure (AI-28 review R1)", () => {
    const { container } = render(
      <ToolStepCard step={{ ...step, approvalState: "failed" }} />
    )
    expect(container.textContent).toContain("approved, action failed")
    fireEvent.click(container.querySelector("button")!)
    expect(container.textContent).toContain("Approved; the action ran and failed.")
    expect(container.textContent).not.toContain("Approved, but it did not run.")
  })

  it("keeps the pending wording for a step that is still waiting", () => {
    const { container } = render(<ToolStepCard step={{ ...step, approvalState: "pending" }} />)
    // A pending step starts open, so the line is already on screen.
    expect(container.textContent).toContain("pending approval")
    expect(container.textContent).toContain(
      "Someone else with approval rights must approve it."
    )
  })
})
