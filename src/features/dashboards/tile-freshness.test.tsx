import { cleanup, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"
import type { ChartRenderSpec } from "@/lib/dashboard-specs"
import type { Rows } from "./tile-dialogs"
import { ReportingProvider } from "./reporting-context"

// BI-4 part B (feature decision 5): the tile's footer says when its data was computed, and "remembered" when it came from memory.
mock.module("./echart", () => ({ EChart: () => <div data-testid="chart" /> }))
const { TileBody } = await import("./tile-body")
const { TileCard } = await import("./tile-card")
const { tileFreshness } = await import("./tile-freshness")
afterEach(cleanup)

const spec = { id: "c_1", title: "Visits", kind: "bar", mart: "m", x: "p", y: "v", source: "ui" } as unknown as ChartRenderSpec
const show = (cell: Rows) =>
  render(
    <ReportingProvider reporting={{ timeZone: "Asia/Jakarta", weekStart: "monday" }}>
      <TileBody spec={spec} cell={cell as never} dark={false} loading={false} />
    </ReportingProvider>,
  )
const rows = { columns: ["p", "v"], rows: [{ p: "a", v: 1 }] }

describe("a tile's footer says when its data is from", () => {
  it("writes the time in the report time zone for a fresh result", () => {
    show({ ...rows, computedAt: "2026-10-24T03:42:10Z", remembered: false })
    expect(screen.getByText("Data from 10:42")).toBeTruthy()
    expect(screen.queryByText(/remembered/)).toBeNull()
  })

  it("adds 'remembered' when the result came from memory", () => {
    show({ ...rows, computedAt: "2026-10-24T03:42:10Z", remembered: true })
    expect(screen.getByText("Data from 10:42 · remembered")).toBeTruthy()
  })

  it("says nothing when the server sent no time, and still draws the tile", () => {
    show(rows)
    expect(screen.getByTestId("chart")).toBeTruthy()
    expect(screen.queryByText(/Data from/)).toBeNull()
  })
})

// BI-4-AC1, owner acceptance 2026-11-16: the dashboard page's tile (hover chip and ⋯ note) says "remembered" too.
describe("the dashboard page's tile chip and ⋯ note (BI-4-AC1)", () => {
  const card = (remembered: boolean) => {
    const f = tileFreshness("2026-10-24T03:42:10Z", "Asia/Jakarta", remembered, "mart_wisman", "Reads mart_wisman.")
    render(<TileCard item={{ id: "c_1", title: "Visits", body: <div />, menu: [], ...f }} editable={false} look="soft" />)
    return f
  }

  it("says remembered in the chip, the ⋯ line and its tooltip", () => {
    const f = card(true)
    expect(document.querySelector("[data-slot='tile-data-from']")?.textContent).toBe("Data from 10:42 · remembered")
    expect(f.menuNote).toEqual({ label: "10:42 · remembered · mart_wisman", detail: "Data from 10:42, remembered. Reads mart_wisman." })
  })

  it("says nothing of it for a fresh result", () => {
    const f = card(false)
    expect(document.querySelector("[data-slot='tile-data-from']")?.textContent).toBe("Data from 10:42")
    expect(f.menuNote).toEqual({ label: "10:42 · mart_wisman", detail: "Data from 10:42. Reads mart_wisman." })
  })
})
