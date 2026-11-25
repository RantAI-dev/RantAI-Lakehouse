// BI-61 T1 (spec T1, AC1): in reading mode the header of a dashboard shows
// Edit (a pencil, for writers), Share (for those who manage it) and ⋯, and
// nothing else; the facts line names the owner, the update and the comments,
// and the breadcrumb names the folder. Renders the real page, like
// `dashboard-stop.test.tsx`.
mock.module("next/navigation", () => ({
  usePathname: () => "/dashboards/b_1",
  useSearchParams: () => new URLSearchParams(),
  useRouter: () => ({
    push: () => {},
    replace: () => {},
    refresh: () => {},
    back: () => {},
    forward: () => {},
    prefetch: () => {},
  }),
}))

import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react"
import { afterAll, afterEach, describe, expect, it, mock, spyOn } from "bun:test"
import { TableProviders } from "@/components/app-shell/table-providers"
import { AuthProvider } from "@/features/auth/auth-provider"
import * as realEchart from "./echart"

// A chart needs a canvas, which the test DOM has not; the marks beside the title do not.
// The real module is put back for the files after this one, as dashboard-page-role does.
mock.module("./echart", () => ({ EChart: () => <div data-testid="chart" /> }))
afterAll(() => {
  mock.module("./echart", () => realEchart)
})

import { DashboardPage } from "./dashboard-page"

afterEach(() => {
  cleanup()
  mock.restore()
  window.localStorage.clear()
})

const json = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } })

function mount({ permissions, level, charts = [], results = {}, extra = {} }: { permissions: string[]; level: string; charts?: unknown[]; results?: Record<string, unknown>; extra?: Record<string, unknown> }) {
  spyOn(globalThis, "fetch").mockImplementation(((input: RequestInfo | URL) => {
    const path = String(input)
    if (path.includes("/api/auth/me")) {
      return Promise.resolve(json({ id: "u1", name: "Rina", email: null, roles: [], permissions, tenants: [] }))
    }
    if (path.startsWith("/api/dashboard?")) {
      return Promise.resolve(json({
        board: "b_1", boards: [{ id: "default", name: "Main" }, { id: "b_1", name: "Quarterly Sales", folderId: "f_2" }],
        charts, kpis: [], filterColumns: [], results, layout: {}, canDownload: true, access: { level }, ...extra,
      }))
    }
    if (path.includes("/api/dashboard/boards")) {
      return Promise.resolve(json({ boards: [{ id: "b_1", name: "Quarterly Sales", createdBy: "Rina Putri", updatedAt: new Date(Date.now() - 2 * 3_600_000).toISOString() }] }))
    }
    if (path.includes("/api/dashboard/folders")) {
      return Promise.resolve(json({ folders: [{ id: "f_1", name: "Finance", parentId: "", createdBy: "u1" }, { id: "f_2", name: "Sales", parentId: "f_1", createdBy: "u1" }] }))
    }
    if (path.includes("/api/dashboard/comments")) {
      return Promise.resolve(json({ comments: [{ id: "c1", body: "hi", createdAt: "2026-11-14T00:00:00Z" }, { id: "c2", body: "yo", createdAt: "2026-11-14T00:00:00Z" }], truncated: false }))
    }
    if (path.includes("/api/dashboard/shortcuts")) return Promise.resolve(json({ favourites: [], recent: [] }))
    return Promise.resolve(json({}))
  }) as unknown as typeof fetch)
  render(
    <AuthProvider>
      <TableProviders>
        <DashboardPage boardId="b_1" />
      </TableProviders>
    </AuthProvider>,
  )
}

const actions = () => document.querySelector("[data-slot='dashboard-header-actions']") as HTMLElement

describe("the dashboard header (BI-61 T1)", () => {
  it("shows Edit, the settings menu and Share, in that order, to someone who manages the dashboard (review round 1, SHOULD-FIX 10)", async () => {
    mount({ permissions: ["dashboard:read", "dashboard:write"], level: "manage" })
    await screen.findByText("Quarterly Sales")
    await waitFor(() => expect(within(actions()).getAllByRole("button").map((b) => b.getAttribute("aria-label") ?? b.textContent?.trim()))
      .toEqual(["Edit layout", "Dashboard settings", "Share"]))
    // BI-61 review round 1, SHOULD-FIX 9: the console's default 32 px, like every other page header.
    for (const b of within(actions()).getAllByRole("button")) {
      expect(b.className).toMatch(/(^|\s)(h-8|size-8)(\s|$)/)
      expect(b.className).not.toMatch(/(^|\s)(h-9|size-9)(\s|$)/)
    }
  })

  it("shows only the settings menu to a reader who may neither edit nor share", async () => {
    mount({ permissions: ["dashboard:read"], level: "view" })
    await screen.findByText("Quarterly Sales")
    await waitFor(() => expect(within(actions()).getAllByRole("button").map((b) => b.getAttribute("aria-label")))
      .toEqual(["Dashboard settings"]))
  })

  it("names the folder in the breadcrumb, and the owner and the comments in the facts line, with no second time", async () => {
    mount({ permissions: ["dashboard:read"], level: "view" })
    const crumbs = await screen.findByRole("navigation", { name: "Breadcrumb" })
    await waitFor(() => expect(crumbs.textContent).toBe("DashboardsFinanceSales"))
    expect(await screen.findByText("Rina Putri")).toBeTruthy()
    // BI-61 review round 1, SHOULD-FIX 11: one time in the header, the data's; the last edit is in Details.
    expect(screen.queryByText(/Updated/)).toBeNull()
    expect(await screen.findByRole("button", { name: "2 comments" })).toBeTruthy()
  })
})

// BI-61 T5: the pencil turns the page into edit mode: Add chart (primary) and
// Done in the header, the edit hint, "+ Filter", Parameters and "+ tab" in the
// row; Done returns to reading.
describe("edit mode (BI-61 T5)", () => {
  it("swaps the reading controls for Add chart and Done, and Done comes back", async () => {
    mount({ permissions: ["dashboard:read", "dashboard:write"], level: "manage" })
    await screen.findByText("Quarterly Sales")
    fireEvent.click(await screen.findByRole("button", { name: "Edit layout" }))
    await waitFor(() => expect(within(actions()).getAllByRole("button").map((b) => b.textContent?.trim())).toEqual(["Add chart", "Done"]))
    expect(screen.getByText("Drag tiles to move · drag a corner to resize · saves automatically")).toBeTruthy()
    expect(screen.getByRole("button", { name: "Parameters" })).toBeTruthy()
    expect(screen.getByText("Add tabs")).toBeTruthy()
    expect(screen.getByRole("button", { name: "Edit phone layout" })).toBeTruthy()
    fireEvent.click(within(actions()).getByRole("button", { name: "Done" }))
    await waitFor(() => expect(within(actions()).getAllByRole("button").map((b) => b.getAttribute("aria-label") ?? b.textContent?.trim()))
      .toEqual(["Edit layout", "Dashboard settings", "Share"]))
    expect(screen.queryByText(/^Drag tiles to move/)).toBeNull()
    expect(screen.queryByRole("button", { name: "Parameters" })).toBeNull()
  })
})

// BI-61 T7: the page is a frame that ends above the assistant's bar; without the
// assistant there is no bar, so it keeps only the shell's own gap.
describe("the page frame (BI-61 T7)", () => {
  it("keeps the shell's gap, not the bar's room, for someone without the assistant, and scrolls only the tiles on a laptop", async () => {
    mount({ permissions: ["dashboard:read"], level: "view" })
    await screen.findByText("Quarterly Sales")
    const frame = document.querySelector("[data-slot='dashboard-frame']") as HTMLElement
    expect(frame.dataset.frame).toBe("plain")
    expect(frame.className).toContain("var(--content-bottom,1.5rem)")
    expect(frame.className).not.toContain("var(--content-bottom,5rem)")
    expect((document.querySelector("[data-slot='dashboard-scroll']") as HTMLElement).className).toContain("lg:overflow-y-auto")
  })
})

// BI-61 review round 1, SHOULD-FIX 2: no sparkles icon beside a tile's title. The mark
// there says the chart can be clicked (a pointing hand); who made the chart is said in
// words in its ⋯, never as a sparkle.
describe("the marks beside a tile's title (BI-61 review round 1)", () => {
  it("draws no sparkles icon, for a chart made with the assistant either", async () => {
    const chart = {
      id: "c_ai", title: "jumlah by tahun", kind: "bar", mart: "mart_x", x: "tahun", y: "jumlah", source: "ai", span: 1,
      def: { title: "jumlah by tahun", kind: "bar", mart: "mart_x", dimension: "tahun", measures: ["jumlah"] },
    }
    mount({ permissions: ["dashboard:read"], level: "view", charts: [chart], results: { c_ai: { columns: ["tahun", "jumlah"], rows: [{ tahun: "2024", jumlah: 1 }] } } })
    expect(await screen.findByText("jumlah by tahun")).toBeTruthy()
    const tile = document.querySelector("[data-slot='tile']") as HTMLElement
    expect(within(tile).getByLabelText("Clickable chart").getAttribute("class")).toContain("lucide-pointer")
    expect(document.querySelector("[data-slot='tile'] .lucide-sparkles")).toBeNull()
    expect(document.querySelector(".lucide-sparkles")).toBeNull()
    // The quiet mark that says who made it, with its words.
    expect(within(tile).getByLabelText("Made with the assistant").getAttribute("class")).toContain("lucide-bot")
  })

  // BI-61 review round 1, SHOULD-FIX 5: a dashboard with no tabs starts its chips at the left.
  it("starts the filter chips at the left on a dashboard with no tabs", async () => {
    const param = { param: { id: "p_top", name: "top", label: "Top berapa", type: "number", default: 5, min: 1, max: 12 }, value: 5 }
    mount({ permissions: ["dashboard:read"], level: "view", extra: { params: [param], defaultParams: { top: 5 }, appliedParams: { top: 5 } } })
    await screen.findByRole("button", { name: /^Top berapa/ })
    expect((document.querySelector("[data-align]") as HTMLElement).dataset.align).toBe("start")
  })
})
