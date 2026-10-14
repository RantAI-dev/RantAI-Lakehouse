import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, mock } from "bun:test"

// The builder reads the signed-in user's permissions; a plain editor without `dashboard:sql` is enough here.
mock.module("@/features/auth/auth-provider", () => ({ useAuth: () => ({ hasPermission: () => false }) }))

import { ChartBuilder, type ChartDef } from "./chart-builder"

const originalFetch = global.fetch
let bodies: Record<string, unknown>[] = []

/** The routes the builder calls on open: the marts, the fields of `mart_visits`, and the preview (answered with an error, which is enough). */
beforeEach(() => {
  bodies = []
  global.fetch = mock(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input)
    const json = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } })
    if (url.includes("/specs/preview")) {
      if (init?.body) bodies.push(JSON.parse(String(init.body)))
      return json({ error: "not in this test" }, 400)
    }
    if (url.includes("/api/dashboard/fields") && url.includes("mart=")) {
      return json({
        dimensions: ["provinsi", "category"], measures: ["visitors", "stay"], calculated: [], hierarchies: [],
        columns: [{ name: "provinsi", type: "String" }, { name: "category", type: "String" }, { name: "visitors", type: "UInt64" }, { name: "stay", type: "Float64" }],
      })
    }
    if (url.includes("/api/dashboard/fields")) return json({ marts: [{ name: "mart_visits", rows: 10 }] })
    if (url.includes("/api/dashboard/sources")) return json({ sources: [] })
    // Metrics, segments and parameters: the builder carries on without them.
    return json({ error: "not in this test" }, 500)
  }) as unknown as typeof fetch
})
afterEach(() => {
  global.fetch = originalFetch
  cleanup()
})

const barDef: ChartDef = { title: "Visitors", kind: "bar", mart: "mart_visits", dimension: "provinsi", measures: ["visitors"], aggregate: "sum" }

function renderBuilder(initial?: ChartDef) {
  return render(<ChartBuilder open onOpenChange={() => {}} hideTrigger onSaved={() => {}} initial={initial} editId={initial ? "c_1" : undefined} />)
}

describe("ChartBuilder sections (phase 2 QA R3)", () => {
  it("shows four ordered sections and folds the last one", async () => {
    renderBuilder(barDef)
    const headings = await screen.findAllByRole("heading", { level: 3 })
    expect(headings.map((h) => h.textContent)).toEqual(["1Data", "2What to show", "3Sort and filter", "4More options"])
    expect(screen.getByRole("button", { name: /More options/ }).getAttribute("aria-expanded")).toBe("false")
    // Nothing of the folded options is drawn, and no row of chips or default-aggregate select stands in the form.
    expect(screen.queryByText("Reference lines")).toBeNull()
    expect(screen.queryByText("Parameters")).toBeNull()
    expect(screen.queryByText(/Aggregation .*default/)).toBeNull()
    expect(screen.queryByText("New calculated field")).toBeNull()
  })

  it("opens the fold on its own when the edited chart uses something inside it", async () => {
    renderBuilder({ ...barDef, referenceLines: [{ value: 100 }] as never })
    const toggle = await screen.findByRole("button", { name: /More options/ })
    await waitFor(() => expect(toggle.getAttribute("aria-expanded")).toBe("true"))
    expect(screen.getByText("Parameters")).toBeTruthy()
  })

  it("opens the fold for steps, and a user can open it by hand", async () => {
    const { unmount } = renderBuilder({ ...barDef, steps: [{ filter: [{ column: "visitors", op: "gt", value: "5" }] }] })
    await waitFor(() => expect(screen.getByRole("button", { name: /More options/ }).getAttribute("aria-expanded")).toBe("true"))
    unmount()
    cleanup()
    renderBuilder(barDef)
    const toggle = await screen.findByRole("button", { name: /More options/ })
    fireEvent.click(toggle)
    expect(toggle.getAttribute("aria-expanded")).toBe("true")
    expect(screen.getByText("Parameters")).toBeTruthy()
  })

  it("keeps a text note to its title and content", async () => {
    renderBuilder({ title: "Note", kind: "text", text: "hello" })
    await screen.findByText(/Content \(markdown\)/)
    expect(screen.queryAllByRole("heading", { level: 3 })).toHaveLength(0)
  })
})

describe("ChartBuilder aggregation (phase 2 QA R4)", () => {
  it("shows a saved chart-level aggregate on each measure and sends the same definition back", async () => {
    renderBuilder({ ...barDef, measures: ["visitors", "stay"], aggregate: "avg" })
    await waitFor(() => expect(screen.getByLabelText("Aggregation of stay").textContent).toContain("avg"))
    expect(screen.getByLabelText("Aggregation of visitors").textContent).toContain("avg")
    // The preview is requested with the very measures and aggregate that were saved.
    await waitFor(() => expect(bodies.length).toBeGreaterThan(0))
    const sent = bodies[bodies.length - 1]
    expect(sent.measures).toEqual(["visitors", "stay"])
    expect(sent.aggregate).toBe("avg")
  })
})

describe("ChartBuilder kind rules (phase 2 QA R5)", () => {
  it("hides steps and questions while 'Other' is on and says why beside the switch", async () => {
    renderBuilder({ ...barDef, other: true, limit: 5, steps: undefined, referenceLines: [{ value: 1 }] as never })
    const toggle = await screen.findByRole("button", { name: /More options/ })
    await waitFor(() => expect(toggle.getAttribute("aria-expanded")).toBe("true"))
    expect(screen.getByText("While this is on, steps and more questions are not available.")).toBeTruthy()
    expect(screen.queryByText("Steps")).toBeNull()
    expect(screen.queryByText("More questions")).toBeNull()
    expect(screen.queryByText(/Remove .* to combine questions/)).toBeNull()
  })

  it("offers 'Other', steps and questions to a bar, and none of them to a KPI", async () => {
    const { unmount } = renderBuilder({ ...barDef, referenceLines: [{ value: 1 }] as never })
    await screen.findByLabelText("Group the rest as Other")
    expect(screen.getByText("Steps")).toBeTruthy()
    expect(screen.getByText("More questions")).toBeTruthy()
    unmount()
    cleanup()
    renderBuilder({ title: "K", kind: "kpi", mart: "mart_visits", measures: ["visitors"], aggregate: "sum" })
    await screen.findByText("Filters")
    expect(screen.queryByLabelText("Group the rest as Other")).toBeNull()
    expect(screen.queryByText("Sort")).toBeNull()
    expect(screen.queryByText("Steps")).toBeNull()
    fireEvent.click(screen.getByRole("button", { name: /More options/ }))
    expect(screen.queryByText("Steps")).toBeNull()
    expect(screen.queryByText("More questions")).toBeNull()
    expect(screen.queryByText("Reference lines")).toBeNull()
  })
})

describe("ChartBuilder kind switch (phase 2 QA R6)", () => {
  it("keeps the dimension as the first grouping of a data table and back as the dimension of a chart", async () => {
    renderBuilder({ ...barDef, breakdown: "category" })
    await screen.findByLabelText("Aggregation of visitors")
    fireEvent.click(screen.getByRole("button", { name: "Data table" }))
    // The dimension and the breakdown are the first groupings, so the steps editor has something to keep.
    expect(await screen.findByLabelText("Move provinsi down")).toBeTruthy()
    expect(screen.getByLabelText("Remove category")).toBeTruthy()
    fireEvent.click(screen.getByRole("button", { name: "Bar" }))
    await waitFor(() => expect(screen.queryByLabelText("Remove category")).toBeNull())
    expect(screen.getAllByText("provinsi").length).toBeGreaterThan(0)
  })
})
