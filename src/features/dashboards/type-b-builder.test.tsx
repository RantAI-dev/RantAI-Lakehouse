import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterAll, afterEach, beforeEach, describe, expect, it, mock } from "bun:test"

import * as realAuth from "@/features/auth/auth-provider"

// As in chart-builder.test.tsx: a plain editor, and the real module put back afterwards.
const real = { ...realAuth }
mock.module("@/features/auth/auth-provider", () => ({ ...real, useAuth: () => ({ hasPermission: () => false }) }))
afterAll(() => {
  mock.module("@/features/auth/auth-provider", () => real)
})

import { ChartBuilder, type ChartDef } from "./chart-builder"
import { ReportingProvider } from "./reporting-context"

const originalFetch = global.fetch
// happy-dom would fetch the page a preview frame points at.
const happyDOM = (window as unknown as { happyDOM?: { settings: { disableIframePageLoading: boolean } } }).happyDOM
if (happyDOM) happyDOM.settings.disableIframePageLoading = true

let previews: Record<string, unknown>[] = []
let saves: { method: string; body: Record<string, unknown> }[] = []

beforeEach(() => {
  previews = []
  saves = []
  window.localStorage.clear()
  global.fetch = mock(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input)
    const json = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } })
    if (url.includes("/specs/preview")) {
      if (init?.body) previews.push(JSON.parse(String(init.body)))
      return json({ error: "not in this test" }, 400)
    }
    if (url.includes("/api/dashboard/specs")) {
      saves.push({ method: init?.method ?? "GET", body: JSON.parse(String(init?.body ?? "{}")) })
      return json({ ok: true })
    }
    if (url.includes("/api/dashboard/fields") && url.includes("mart=")) {
      return json({
        dimensions: ["provinsi", "category"], measures: ["visitors", "stay", "target"], calculated: [], hierarchies: [],
        columns: [
          { name: "provinsi", type: "String" }, { name: "category", type: "String" }, { name: "visitors", type: "UInt64" },
          { name: "stay", type: "Float64" }, { name: "target", type: "Float64" },
        ],
      })
    }
    if (url.includes("/api/dashboard/fields")) return json({ marts: [{ name: "mart_visits", rows: 10 }] })
    if (url.includes("/api/dashboard/sources")) return json({ sources: [] })
    if (url.includes("/api/dashboard/metrics") && !url.includes("usage")) return json({ metrics: [] })
    return json({ error: "not in this test" }, 500)
  }) as unknown as typeof fetch
})
afterEach(() => {
  global.fetch = originalFetch
  cleanup()
})

function renderBuilder(initial: ChartDef, sites: string[] = ["https://img.example"]) {
  return render(
    <ReportingProvider reporting={{ timeZone: "UTC", weekStart: "monday", contentSites: sites }}>
      <ChartBuilder open onOpenChange={() => {}} hideTrigger onSaved={() => {}} initial={initial} editId="c_1" />
    </ReportingProvider>,
  )
}

const lastPreview = async () => {
  await waitFor(() => expect(previews.length).toBeGreaterThan(0))
  return previews[previews.length - 1]
}

describe("ChartBuilder progress bar (BI-16 part B)", () => {
  it("reopens a progress bar with a fixed goal and sends the same definition back", async () => {
    renderBuilder({ title: "Visitors", kind: "progress", mart: "mart_visits", measures: ["visitors"], aggregate: "sum", goal: 1000000 })
    const body = await lastPreview()
    expect(body).toMatchObject({ kind: "progress", measures: ["visitors"], aggregate: "sum", goal: 1000000 })
    expect(body.dimension).toBeUndefined()
    expect((screen.getByLabelText("Goal number") as HTMLInputElement).value).toBe("1000000")
  })

  it("keeps a goal that is a second value as the goal, not as a second measure row, and sends both measures", async () => {
    renderBuilder({ title: "Visitors", kind: "progress", mart: "mart_visits", measures: ["visitors", "target"], aggregate: "sum", goal: { measure: "target" } })
    const body = await lastPreview()
    expect(body).toMatchObject({ kind: "progress", measures: ["visitors", "target"], goal: { measure: "target" } })
    await waitFor(() => expect(screen.getByLabelText("Goal column").textContent).toContain("target"))
    // The form has one measure row (the value); the goal is its own control.
    expect(screen.queryByLabelText("Aggregation of target")).toBeNull()
  })

  it("asks for the goal before it can be previewed", async () => {
    renderBuilder({ title: "Visitors", kind: "progress", mart: "mart_visits", measures: ["visitors"], aggregate: "sum" })
    await screen.findByLabelText("Goal number")
    await screen.findByText(/Fill in Goal number to see real data here/)
    expect(previews).toHaveLength(0)
  })
})

describe("ChartBuilder histogram (BI-16 part B)", () => {
  it("reopens a histogram with its column and ranges, and sends no measure", async () => {
    renderBuilder({ title: "Spread", kind: "histogram", mart: "mart_visits", dimension: "visitors", histogram: { count: 10 } })
    const body = await lastPreview()
    expect(body).toMatchObject({ kind: "histogram", dimension: "visitors", histogram: { count: 10 } })
    expect(body.measures).toBeUndefined()
    expect(body.aggregate).toBeUndefined()
    expect(body.breakdown).toBeUndefined()
    expect(body.order).toBeUndefined()
    expect(body.limit).toBeUndefined()
    // One number column, no breakdown, no measure rows.
    expect(screen.getByText("Number column")).toBeTruthy()
    expect(document.querySelector('[data-slot="breakdown-field"]')).toBeNull()
    expect(screen.queryByText("Measure (Y)")).toBeNull()
    expect(screen.getByLabelText("Ranges").textContent).toContain("A number of ranges")
    expect((screen.getByLabelText("Number of ranges") as HTMLInputElement).value).toBe("10")
  })

  it("sends no histogram field when the ranges are automatic, and a width when one is chosen", async () => {
    const { unmount } = renderBuilder({ title: "Spread", kind: "histogram", mart: "mart_visits", dimension: "stay" })
    const auto = await lastPreview()
    expect(auto.histogram).toBeUndefined()
    unmount()
    cleanup()
    previews = []
    renderBuilder({ title: "Spread", kind: "histogram", mart: "mart_visits", dimension: "stay", histogram: { width: 0.5 } })
    expect(await lastPreview()).toMatchObject({ histogram: { width: 0.5 } })
  })
})

describe("ChartBuilder bullet graph (BI-16 part B)", () => {
  it("reopens a bullet per category with its target value, bands and limit", async () => {
    renderBuilder({
      title: "Bullets", kind: "bullet", mart: "mart_visits", dimension: "provinsi", measures: ["visitors", "target"], aggregate: "sum",
      goal: { measure: "target" }, bands: [10, 20, 30], limit: 20,
    })
    const body = await lastPreview()
    expect(body).toMatchObject({
      kind: "bullet", dimension: "provinsi", measures: ["visitors", "target"], goal: { measure: "target" }, bands: [10, 20, 30], limit: 20,
    })
    expect((screen.getByLabelText("Band 2 limit") as HTMLInputElement).value).toBe("20")
  })

  it("sends one bullet with a fixed target and no limit when there is no category", async () => {
    renderBuilder({ title: "One", kind: "bullet", mart: "mart_visits", measures: ["visitors"], aggregate: "sum", goal: 50, bands: [20, 40] })
    const body = await lastPreview()
    expect(body).toMatchObject({ kind: "bullet", measures: ["visitors"], goal: 50, bands: [20, 40] })
    expect(body.dimension).toBe("")
    expect(body.limit).toBeUndefined()
  })

  it("does not preview bands that do not rise", async () => {
    renderBuilder({ title: "One", kind: "bullet", mart: "mart_visits", measures: ["visitors"], aggregate: "sum", bands: [40, 20] })
    await screen.findByText(/Fill in Band limits, rising to see real data here/)
    expect(previews).toHaveLength(0)
  })
})

describe("ChartBuilder image and web cards (BI-16 part B)", () => {
  const image: ChartDef = { title: "Logo", kind: "image", url: "https://img.example/logo.png", alt: "The logo" }

  it("shows the allowed sites under the address, previews the picture, and saves only title, address and description", async () => {
    renderBuilder(image)
    await screen.findByRole("list", { name: "Allowed content sites" })
    const preview = await screen.findByRole("img", { name: "The logo" })
    expect(preview.getAttribute("src")).toBe("https://img.example/logo.png")
    fireEvent.click(screen.getByRole("button", { name: "Save changes" }))
    await waitFor(() => expect(saves).toHaveLength(1))
    expect(saves[0].method).toBe("PUT")
    expect(saves[0].body).toEqual({ title: "Logo", kind: "image", span: 1, board: "default", id: "c_1", url: "https://img.example/logo.png", alt: "The logo" })
    // No data fields, and the server is never asked to preview.
    expect(previews).toHaveLength(0)
  })

  it("refuses an address on a site that is not listed, with the server's sentence, and sends nothing", async () => {
    renderBuilder({ ...image, url: "https://other.example/logo.png" })
    const refused = "Images and web pages may only come from the sites an administrator allows (Admin > Settings)."
    expect((await screen.findAllByText(refused)).length).toBeGreaterThan(0)
    expect(screen.queryByRole("img", { name: "The logo" })).toBeNull()
    fireEvent.click(screen.getByRole("button", { name: "Save changes" }))
    await waitFor(() => expect(screen.getAllByRole("alert").length).toBeGreaterThan(0))
    expect(saves).toHaveLength(0)
  })

  it("says no site is allowed yet when the list is empty", async () => {
    renderBuilder(image, [])
    expect(await screen.findByText(/No site is allowed yet/)).toBeTruthy()
  })

  it("needs a description for an image and an address for a web page", async () => {
    renderBuilder({ title: "Logo", kind: "image", url: "https://img.example/logo.png" })
    fireEvent.click(await screen.findByRole("button", { name: "Save changes" }))
    await waitFor(() => expect(screen.getByRole("alert").textContent).toContain("Still needed: Description"))
    expect(saves).toHaveLength(0)
  })

  it("previews a web page in a sandboxed frame and saves its address", async () => {
    const { container } = renderBuilder({ title: "Status", kind: "web", url: "https://img.example/status" })
    await waitFor(() => expect(document.querySelector("iframe")).not.toBeNull())
    expect(document.querySelector("iframe")?.getAttribute("sandbox")).toBe("allow-scripts allow-popups")
    expect(container).toBeTruthy()
    fireEvent.click(screen.getByRole("button", { name: "Save changes" }))
    await waitFor(() => expect(saves).toHaveLength(1))
    expect(saves[0].body).toEqual({ title: "Status", kind: "web", span: 1, board: "default", id: "c_1", url: "https://img.example/status" })
  })
})
