import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterAll, afterEach, beforeEach, describe, expect, it, mock, spyOn } from "bun:test"

import * as realAuth from "@/features/auth/auth-provider"

// The builder reads the signed-in user's permissions; a plain editor without `dashboard:sql` is enough here.
// PR #160 CI fix: `mock.module` outlives this file in `bun test src`, so the real exports are kept and only
// `useAuth` is replaced, and the real module is put back in `afterAll`; a bare `{ useAuth }` left
// `tenant-switcher.test.tsx`, which runs later, without `AuthProvider`.
const real = { ...realAuth }
mock.module("@/features/auth/auth-provider", () => ({ ...real, useAuth: () => ({ hasPermission: () => false }) }))
afterAll(() => {
  mock.module("@/features/auth/auth-provider", () => real)
})

import { ChartBuilder, type ChartDef } from "./chart-builder"

const originalFetch = global.fetch
let bodies: Record<string, unknown>[] = []

/** The routes the builder calls on open: the marts, the fields of `mart_visits`, and the preview (answered with an error, which is enough). */
beforeEach(() => {
  bodies = []
  window.localStorage.clear()
  global.fetch = mock(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input)
    const json = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } })
    if (url.includes("/specs/preview")) {
      if (init?.body) bodies.push(JSON.parse(String(init.body)))
      return json({ error: "not in this test" }, 400)
    }
    if (url.includes("/api/dashboard/fields") && url.includes("mart=")) {
      return json({
        dimensions: ["provinsi", "category"], measures: ["visitors", "stay", "mart_demo_province_capacity__rooms"], calculated: [], hierarchies: [],
        columns: [{ name: "provinsi", type: "String" }, { name: "category", type: "String" }, { name: "visitors", type: "UInt64" }, { name: "stay", type: "Float64" }, { name: "mart_demo_province_capacity__rooms", type: "Float64" }],
      })
    }
    if (url.includes("/api/dashboard/fields")) return json({ marts: [{ name: "mart_visits", rows: 10 }] })
    if (url.includes("/api/dashboard/sources")) return json({ sources: [] })
    if (url.includes("/api/dashboard/metrics") && !url.includes("usage")) {
      return json({ metrics: [{ id: "m1", name: "coba_metrik", label: "Coba metrik", description: "", sourceKind: "mart", sourceId: "mart_visits", formula: "sum(visitors)" }] })
    }
    // Segments and parameters: the builder carries on without them.
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
    // A folded section also shows its summary after the title (phase 2 QA R10).
    const titles = ["Data", "What to show", "Sort and filter", "More options"]
    expect(headings.map((h) => h.textContent ?? "")).toHaveLength(4)
    headings.forEach((h, i) => expect(h.textContent).toContain(titles[i]))
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
    // The type list is folded for a chart being edited; the menu in the header changes the type.
    fireEvent.click(screen.getByRole("button", { name: /Change chart type/ }))
    fireEvent.click(screen.getByRole("button", { name: "Data table" }))
    // The dimension and the breakdown are the first groupings, so the steps editor has something to keep.
    expect(await screen.findByLabelText("Move provinsi down")).toBeTruthy()
    expect(screen.getByLabelText("Remove category")).toBeTruthy()
    fireEvent.click(screen.getByRole("button", { name: /Change chart type/ }))
    fireEvent.click(screen.getByRole("button", { name: "Bar" }))
    await waitFor(() => expect(screen.queryByLabelText("Remove category")).toBeNull())
    expect(screen.getAllByText("provinsi").length).toBeGreaterThan(0)
  })
})

const expanded = (name: RegExp | string) => screen.getByRole("button", { name }).getAttribute("aria-expanded")

describe("ChartBuilder type list (phase 2 QA R9)", () => {
  it("shows the list for a new chart, folds it once a type is chosen, and the choice of folding is remembered", async () => {
    const { unmount } = renderBuilder()
    expect(screen.getByRole("navigation", { name: "Chart type" })).toBeTruthy()
    fireEvent.click(screen.getByRole("button", { name: "Line" }))
    // No remembered choice yet: choosing a type folds the list away, and nothing is stored by that.
    expect(screen.queryByRole("navigation", { name: "Chart type" })).toBeNull()
    expect(window.localStorage.getItem("lh.chartBuilder.typeList")).toBeNull()
    // Opening it by hand is remembered, and a builder opened later starts open.
    fireEvent.click(screen.getByRole("button", { name: "Show the chart type list" }))
    expect(screen.getByRole("navigation", { name: "Chart type" })).toBeTruthy()
    expect(window.localStorage.getItem("lh.chartBuilder.typeList")).toBe("expanded")
    unmount()
    cleanup()
    renderBuilder({ ...barDef })
    expect(screen.getByRole("navigation", { name: "Chart type" })).toBeTruthy()
    // Folding by hand is remembered too, and a new chart then opens folded.
    fireEvent.click(screen.getByRole("button", { name: "Fold the chart type list" }))
    expect(window.localStorage.getItem("lh.chartBuilder.typeList")).toBe("collapsed")
    cleanup()
    renderBuilder()
    expect(screen.queryByRole("navigation", { name: "Chart type" })).toBeNull()
  })

  it("starts folded for an edited chart, and the folded control still searches the types", async () => {
    renderBuilder(barDef)
    expect(screen.queryByRole("navigation", { name: "Chart type" })).toBeNull()
    fireEvent.click(screen.getByRole("button", { name: /Change chart type, now Bar/ }))
    const search = screen.getByLabelText("Search chart types")
    fireEvent.change(search, { target: { value: "sankey" } })
    expect(screen.queryByRole("button", { name: "Line" })).toBeNull()
    fireEvent.keyDown(search, { key: "Enter" })
    // The first match was picked and the menu closed.
    await waitFor(() => expect(screen.queryByLabelText("Search chart types") === null).toBe(true))
    expect(screen.getByRole("button", { name: /Change chart type, now Sankey/ })).toBeTruthy()
  })

  it("closes the folded type menu on Escape and on an outside press, and focus returns to its trigger (phase 2 QA R17)", async () => {
    renderBuilder(barDef)
    const trigger = () => screen.getByRole("button", { name: /Change chart type, now Bar/ })
    fireEvent.click(trigger())
    const search = await screen.findByLabelText("Search chart types")
    fireEvent.keyDown(search, { key: "Escape" })
    await waitFor(() => expect(screen.queryByLabelText("Search chart types") === null).toBe(true))
    // A boolean, not the element: a failing match would print the whole dialog on every retry.
    await waitFor(() => expect(document.activeElement === trigger()).toBe(true))
    // The dialog itself stays open after the Escape.
    expect(screen.getByText("Edit chart")).toBeTruthy()
    fireEvent.click(trigger())
    await screen.findByLabelText("Search chart types")
    const heading = screen.getByText("Edit chart")
    fireEvent.pointerDown(heading)
    fireEvent.mouseDown(heading)
    fireEvent.click(heading)
    await waitFor(() => expect(screen.queryByLabelText("Search chart types") === null).toBe(true))
    expect(screen.getByText("Edit chart")).toBeTruthy()
  })

  it("works when the browser refuses storage", async () => {
    const read = spyOn(Storage.prototype, "getItem").mockImplementation(() => { throw new Error("blocked") })
    const write = spyOn(Storage.prototype, "setItem").mockImplementation(() => { throw new Error("blocked") })
    try {
      renderBuilder()
      expect(screen.getByRole("navigation", { name: "Chart type" })).toBeTruthy()
      fireEvent.click(screen.getByRole("button", { name: "Fold the chart type list" }))
      expect(screen.queryByRole("navigation", { name: "Chart type" })).toBeNull()
    } finally {
      read.mockRestore()
      write.mockRestore()
    }
  })
})

describe("ChartBuilder accordion (phase 2 QA R10)", () => {
  it("opens only section 1 for a new chart, with no way on until its required field is filled", async () => {
    renderBuilder()
    await screen.findByText("Data source")
    expect(expanded(/^Data(?! table)/)).toBe("true")
    expect(expanded(/What to show/)).toBe("false")
    expect(expanded(/Sort and filter/)).toBe("false")
    // Opening another section folds the open one.
    fireEvent.click(screen.getByRole("button", { name: /What to show/ }))
    expect(expanded(/What to show/)).toBe("true")
    expect(expanded(/^Data(?! table)/)).toBe("false")
  })

  it("has no Next button, even once the source is picked (phase 2 QA R19)", async () => {
    renderBuilder()
    fireEvent.click(await screen.findByRole("combobox", { name: /pick a mart/i }).catch(() => screen.getByText(/pick a mart/)))
    fireEvent.click(await screen.findByRole("option", { name: /mart_visits/ }))
    await screen.findByText("Data source")
    expect(screen.queryByRole("button", { name: /^Next/ })).toBeNull()
    // Opening a section by its heading still folds the open one.
    fireEvent.click(screen.getByRole("button", { name: /What to show/ }))
    expect(expanded(/What to show/)).toBe("true")
    expect(expanded(/^Data(?! table)/)).toBe("false")
  })

  it("opens an edited chart with sections 1 to 3 folded to one-line summaries", async () => {
    renderBuilder({ ...barDef, measures: ["visitors", { column: "stay", aggregate: "avg" }] as never, filters: [{ column: "category", op: "eq", value: "a" }] as never, limit: 5 })
    await waitFor(() => expect(screen.getByRole("button", { name: /^Data(?! table)/ }).textContent).toContain("mart_visits"))
    expect(expanded(/^Data(?! table)/)).toBe("false")
    expect(expanded(/What to show/)).toBe("false")
    expect(expanded(/Sort and filter/)).toBe("false")
    expect(screen.getByRole("button", { name: /What to show/ }).textContent).toContain("provinsi · visitors, avg of stay")
    expect(screen.getByRole("button", { name: /Sort and filter/ }).textContent).toContain("highest first · top 5 · 1 filter")
    // A folded section is opened by its heading, which is a button reached by Tab.
    fireEvent.click(screen.getByRole("button", { name: /What to show/ }))
    expect(expanded(/What to show/)).toBe("true")
  })

  it("opens the section that holds a missing field and focuses the field when saving", async () => {
    renderBuilder({ title: "T", kind: "bar", mart: "mart_visits", measures: ["visitors"], aggregate: "sum" })
    await screen.findByLabelText("Aggregation of visitors")
    expect(expanded(/What to show/)).toBe("false")
    fireEvent.click(screen.getByRole("button", { name: "Save changes" }))
    await waitFor(() => expect(expanded(/What to show/)).toBe("true"))
    expect(screen.getByRole("alert").textContent).toContain("Dimension (X)")
    const focused = document.activeElement as HTMLElement
    expect(focused.closest("section")?.querySelector("h3")?.textContent).toContain("What to show")
  })

  it("opens section 1 and focuses the source select when a new chart has no source", async () => {
    renderBuilder()
    await screen.findByText("Data source")
    fireEvent.click(screen.getByRole("button", { name: /What to show/ }))
    fireEvent.click(screen.getByRole("button", { name: "Create chart" }))
    await waitFor(() => expect(expanded(/^Data(?! table)/)).toBe("true"))
    const focused = document.activeElement as HTMLElement
    expect(focused.closest("section")?.querySelector("h3")?.textContent).toContain("Data")
    expect(focused.hasAttribute("data-placeholder")).toBe(true)
  })
})

describe("ChartBuilder header (phase 2 QA R11)", () => {
  it("keeps title, dashboard and width in one row and has no helper line under the title", async () => {
    renderBuilder()
    const title = await screen.findByLabelText(/^Title/)
    const row = title.closest("div.grid")?.parentElement as HTMLElement
    expect(row.textContent).toContain("Dashboard")
    expect(row.textContent).toContain("Width")
    expect(screen.queryByText(/Left empty, the chart is saved as/)).toBeNull()
  })
})

describe("ChartBuilder long column names (phase 2 QA R14)", () => {
  it("shows a picked joined measure by its own name with the table as a tag, and the full reference on hover", async () => {
    renderBuilder({ ...barDef, measures: ["mart_demo_province_capacity__rooms"] })
    fireEvent.click(await screen.findByRole("button", { name: /What to show/ }))
    const trigger = await screen.findByRole("combobox", { name: /^Measure/ })
    await waitFor(() => expect(trigger.textContent).toContain("rooms"))
    expect(trigger.textContent).toContain("capacity")
    expect(trigger.textContent).not.toContain("__")
    expect(trigger.getAttribute("title")).toBe("mart_demo_province_capacity.rooms")
    // The label folds inside the trigger instead of widening it.
    expect(trigger.querySelector(".truncate")?.textContent).toBe("rooms")
  })

  it("gives every grid of the form a column the container sizes, so no long name can scroll the form sideways", async () => {
    renderBuilder({ ...barDef, measures: ["mart_demo_province_capacity__rooms"] })
    const section = (await screen.findByRole("button", { name: /What to show/ })).closest("section") as HTMLElement
    expect(section.className).toContain("grid-cols-[minmax(0,1fr)]")
    expect(section.querySelector("[id$=-panel]")?.className).toContain("grid-cols-[minmax(0,1fr)]")
    const form = section.parentElement as HTMLElement
    expect(form.className).toContain("grid-cols-[minmax(0,1fr)]")
    expect(form.className).toContain("min-w-0")
    expect(form.className).toContain("[&_.grid:not([class*=grid-cols])]:grid-cols-[minmax(0,1fr)]")
  })
})

describe("ChartBuilder conditional controls (phase 2 QA R15)", () => {
  it("puts the bin of a number dimension in a row of its own, so Dimension and Breakdown stay in one row", async () => {
    renderBuilder({ ...barDef, dimension: "visitors", measures: ["stay"] })
    fireEvent.click(await screen.findByRole("button", { name: /What to show/ }))
    const bin = await screen.findByLabelText("Group in bins")
    const dimension = document.querySelector("[data-slot=dimension-field]") as HTMLElement
    const breakdown = document.querySelector("[data-slot=breakdown-field]") as HTMLElement
    const row = document.querySelector("[data-slot=conditional-row]") as HTMLElement
    // Both fields hold only their label and select, are siblings in one grid, and do not stretch.
    expect(dimension.contains(bin)).toBe(false)
    expect(breakdown.contains(bin)).toBe(false)
    expect(row.contains(bin)).toBe(true)
    expect(dimension.parentElement).toBe(breakdown.parentElement)
    expect(row.parentElement).toBe(dimension.parentElement)
    expect(dimension.className).toContain("content-start")
    expect(breakdown.className).toContain("content-start")
    expect(dimension.parentElement?.className).toContain("items-start")
    expect(row.className).toContain("sm:col-span-2")
  })

  it("draws no extra row when the dimension needs no bin or grain", async () => {
    renderBuilder(barDef)
    fireEvent.click(await screen.findByRole("button", { name: /What to show/ }))
    await screen.findByText(/Dimension \(X\)/)
    expect(document.querySelector("[data-slot=conditional-row]")).toBeNull()
  })
})

describe("ChartBuilder labels (phase 2 QA R20)", () => {
  it("suggests a title from the metric's label, not its stored reference", async () => {
    renderBuilder({ title: "", kind: "bar", mart: "mart_visits", dimension: "category", measures: [{ metric: "coba_metrik" }] as never })
    const field = await screen.findByLabelText("Title")
    await waitFor(() => expect(field.getAttribute("placeholder")).toBe("Coba metrik by category"))
  })

  it("shows the kind after the chevron of the breadcrumb, and no chevron when the list is folded", async () => {
    const { unmount } = renderBuilder()
    const title = screen.getByText("New chart").parentElement as HTMLElement
    expect(title.textContent).toContain("Horizontal bar")
    expect(title.querySelector("svg")).not.toBeNull()
    unmount()
    cleanup()
    renderBuilder(barDef)
    const folded = screen.getByText("Edit chart").parentElement as HTMLElement
    expect(folded.querySelector("svg")).toBeNull()
  })
})

describe("ChartBuilder resizable split (phase 2 QA R18)", () => {
  it("draws a focusable separator between the form panel and the preview panel at a wide screen", async () => {
    const original = window.matchMedia
    window.matchMedia = ((query: string) => ({
      matches: true, media: query, onchange: null,
      addEventListener: () => {}, removeEventListener: () => {}, addListener: () => {}, removeListener: () => {}, dispatchEvent: () => false,
    })) as unknown as typeof window.matchMedia
    try {
      renderBuilder(barDef)
      await screen.findByLabelText("Aggregation of visitors")
      const handle = document.querySelector('[data-slot="resizable-handle"]') as HTMLElement
      expect(handle).not.toBeNull()
      expect(handle.getAttribute("role")).toBe("separator")
      expect(handle.getAttribute("aria-orientation")).toBe("vertical")
      expect(handle.getAttribute("tabindex")).toBe("0")
      const panels = Array.from(document.querySelectorAll('[data-slot="resizable-panel"]'))
      expect(panels.length).toBe(2)
      // The form panel holds the form (its own scroller) and the preview panel the preview.
      expect(panels[0].querySelector('[aria-label="Aggregation of visitors"]')).not.toBeNull()
      expect(panels[1].querySelector('section[aria-label="Preview"]')).not.toBeNull()
    } finally {
      window.matchMedia = original
    }
  })
})
