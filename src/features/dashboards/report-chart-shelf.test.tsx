import * as realNavigation from "next/navigation"
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react"
import { afterAll, afterEach, beforeEach, describe, expect, it, mock, spyOn } from "bun:test"
import * as realTileBody from "./tile-body"

// BI-62 (T6): the Charts tab of the one side panel on the report editor.
// `mock.module` outlives this file in `bun test src`, so both real modules
// are put back after it; every global changed here is put back after each test.
const navigation = { ...realNavigation }
mock.module("next/navigation", () => ({
  ...navigation,
  usePathname: () => "/dashboards/reports/new",
  useSearchParams: () => new URLSearchParams(),
  useRouter: () => ({ push: () => {}, replace: () => {}, refresh: () => {}, back: () => {}, forward: () => {}, prefetch: () => {} }),
}))
const tileBody = { ...realTileBody }
mock.module("./tile-body", () => ({
  ...tileBody,
  TileBody: ({ spec }: { spec: { id: string } }) => <div data-testid={`tile-${spec.id}`} />,
}))
afterAll(() => {
  mock.module("next/navigation", () => navigation)
  mock.module("./tile-body", () => tileBody)
})

const { AuthProvider } = await import("@/features/auth/auth-provider")
const { SidePanelProvider } = await import("@/components/app-shell/side-panel")
const { CopilotSidebar } = await import("@/features/copilot/copilot-sidebar")
const { ReportEditorPage } = await import("./report-editor-page")
const { CHARTS_TAB_KEY } = await import("./report-chart-shelf")

const json = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } })

function serve() {
  spyOn(globalThis, "fetch").mockImplementation(((input: RequestInfo | URL) => {
    const url = String(input)
    if (url.includes("/api/auth/me")) {
      return Promise.resolve(json({ id: "u1", name: "Rina", email: null, roles: ["R"], permissions: ["dashboard:read", "dashboard:write"], tenants: [] }))
    }
    if (url.startsWith("/api/dashboard/boards")) return Promise.resolve(json({ boards: [{ id: "b_1", name: "Sales" }, { id: "b_2", name: "Ops" }] }))
    if (url.startsWith("/api/dashboard/specs")) {
      return Promise.resolve(
        json({ charts: [{ id: "c_one", title: "Visits by month", board: "b_1", kind: "bar" }, { id: "c_ops", title: "Tickets", board: "b_2", kind: "kpi" }] }),
      )
    }
    if (url.startsWith("/api/dashboard?")) {
      const ops = url.includes("b_2")
      return Promise.resolve(
        json({ charts: [ops ? { id: "c_ops", title: "Tickets", kind: "kpi", source: "ui" } : { id: "c_one", title: "Visits by month", kind: "bar", source: "ui" }], results: {} }),
      )
    }
    if (url.includes("/api/dashboard/folders")) return Promise.resolve(json({ folders: [] }))
    return Promise.resolve(json({}))
  }) as unknown as typeof fetch)
}

const realMatchMedia = window.matchMedia
const realObserver = globalThis.IntersectionObserver

function screenIs(wide: boolean) {
  window.matchMedia = ((query: string) =>
    ({ matches: wide && query.includes("min-width: 1024px"), media: query, addEventListener: () => {}, removeEventListener: () => {} }) as unknown as MediaQueryList) as typeof window.matchMedia
}

beforeEach(() => {
  // The pictures load when on screen; this observer says each one is.
  globalThis.IntersectionObserver = class {
    constructor(private readonly cb: IntersectionObserverCallback) {}
    observe(el: Element) {
      this.cb([{ isIntersecting: true, target: el } as IntersectionObserverEntry], this as unknown as IntersectionObserver)
    }
    disconnect() {}
    unobserve() {}
    takeRecords() {
      return []
    }
  } as unknown as typeof IntersectionObserver
})

afterEach(() => {
  cleanup()
  mock.restore()
  window.matchMedia = realMatchMedia
  globalThis.IntersectionObserver = realObserver
  window.localStorage.removeItem(CHARTS_TAB_KEY)
})

function mount() {
  serve()
  return render(
    <AuthProvider>
      <SidePanelProvider>
        <ReportEditorPage />
        <CopilotSidebar hasAssistant={false} />
      </SidePanelProvider>
    </AuthProvider>,
  )
}

async function pageEditor(container: HTMLElement) {
  return waitFor(() => {
    const el = container.querySelector(".ProseMirror") as (HTMLElement & { editor?: import("@tiptap/core").Editor }) | null
    if (!el?.editor) throw new Error("no editor yet")
    return el.editor
  })
}

const panel = () => document.querySelector("[data-slot='copilot-sidebar']") as HTMLElement | null

describe("the Charts tab (BI-62, D6)", () => {
  it("is a tab of the one side panel and opens on Charts for an empty report on a wide screen", async () => {
    screenIs(true)
    const { container } = mount()
    await pageEditor(container)
    await waitFor(() => expect(panel()).not.toBeNull())
    expect(within(panel()!).getByRole("tab", { name: "Charts" }).getAttribute("aria-selected")).toBe("true")
    expect(await within(panel()!).findByRole("region", { name: "Sales" })).toBeTruthy()
    expect(document.querySelectorAll("[data-slot='copilot-sidebar']")).toHaveLength(1)
    expect(screen.getByRole("button", { name: "Charts" }).getAttribute("aria-pressed")).toBe("true")
  })

  it("stays closed on a narrow screen until the Charts button opens it", async () => {
    screenIs(false)
    const { container } = mount()
    await pageEditor(container)
    expect(panel()).toBeNull()
    fireEvent.click(screen.getByRole("button", { name: "Charts" }))
    await waitFor(() => expect(panel()).not.toBeNull())
    fireEvent.click(screen.getByRole("button", { name: "Charts" }))
    await waitFor(() => expect(panel()).toBeNull())
  })

  it("a click on a picture adds that chart at the cursor, drawn at once", async () => {
    screenIs(true)
    const { container } = mount()
    const editor = await pageEditor(container)
    const add = await within((await waitFor(() => panel()!))).findByRole("button", { name: "Add Visits by month to the report" })
    fireEvent.click(add)
    await waitFor(() => expect(editor.getJSON().content?.some((n) => n.type === "chart" && n.attrs?.chart === "c_one")).toBe(true))
    // Drawn from the data its picture read, in the document as well as in the panel.
    await waitFor(() => expect(container.querySelector(".ProseMirror [data-testid='tile-c_one']")).not.toBeNull())
    expect(await within(panel()!).findByLabelText("In the report")).toBeTruthy()
  })

  it("searches the charts by title or dashboard", async () => {
    screenIs(true)
    const { container } = mount()
    await pageEditor(container)
    const side = await waitFor(() => panel()!)
    await within(side).findByRole("region", { name: "Sales" })
    fireEvent.change(within(side).getByPlaceholderText("Search charts"), { target: { value: "ops" } })
    expect(within(side).queryByRole("region", { name: "Sales" })).toBeNull()
    expect(within(side).getByRole("region", { name: "Ops" })).toBeTruthy()
    fireEvent.change(within(side).getByPlaceholderText("Search charts"), { target: { value: "nothing like it" } })
    expect(within(side).getByText("No charts match “nothing like it”")).toBeTruthy()
  })
})
