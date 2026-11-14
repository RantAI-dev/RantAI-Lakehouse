import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react"
import { afterAll, afterEach, beforeEach, describe, expect, it, mock } from "bun:test"

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

import { ChartBuilder, KIND_GROUPS, type ChartDef } from "./chart-builder"

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
  return render(<ChartBuilder onSaved={() => {}} onCancel={() => {}} initial={initial} editId={initial ? "c_1" : undefined} />)
}

/** The inspector's tabs (`BI-58` T4): open one, and read which is open. */
const openTab = (name: string) => fireEvent.click(screen.getByRole("tab", { name }))
const selectedTab = () => screen.getAllByRole("tab").find((t) => t.getAttribute("aria-selected") === "true")?.textContent

// `BI-58` T4: the five sections of phase 2 QA R3 are three tabs; nothing was dropped.
describe("ChartBuilder inspector tabs (phase 2 QA R3, BI-58 T4)", () => {
  it("has three tabs and opens on Data, with the source in the data panel and nothing of Style drawn", async () => {
    renderBuilder(barDef)
    await screen.findByLabelText("Aggregation of visitors")
    expect(screen.getAllByRole("tab").map((t) => t.textContent)).toEqual(["Data", "Sort & filter", "Style"])
    expect(selectedTab()).toBe("Data")
    expect(screen.queryAllByRole("heading", { level: 3 })).toHaveLength(0)
    expect(screen.getByRole("complementary", { name: "Data" }).textContent).toContain("Data source")
    // Style and Sort & filter are not drawn until opened, and no row of chips or default-aggregate select stands in the form.
    expect(screen.queryByText("Reference lines")).toBeNull()
    expect(screen.queryByText("Filters")).toBeNull()
    expect(screen.queryByText(/Aggregation .*default/)).toBeNull()
    expect(screen.queryByText("New calculated field")).toBeNull()
  })

  it("keeps what More options held in the Data tab: parameters, the click and the freshness", async () => {
    renderBuilder({ ...barDef, click: { kind: "url", url: "https://example.com/x" } })
    await screen.findByLabelText("Aggregation of visitors")
    const data = screen.getByRole("tabpanel")
    expect(within(data).getByText("Parameters")).toBeTruthy()
    expect(within(data).getByLabelText("On click")).toBeTruthy()
    expect(within(data).getByLabelText("Freshness")).toBeTruthy()
  })

  // BI-17: the reference lines editor moved from More options to the Format section, now the Style tab.
  it("lists a chart's reference line under Style", async () => {
    renderBuilder({ ...barDef, referenceLines: [{ value: 100 }] as never })
    await screen.findByLabelText("Aggregation of visitors")
    expect(screen.queryByLabelText("Line 1 value")).toBeNull()
    openTab("Style")
    expect((await screen.findByLabelText("Line 1 value") as HTMLInputElement).value).toBe("100")
  })

  it("lists a chart's steps under Sort & filter", async () => {
    renderBuilder({ ...barDef, steps: [{ filter: [{ column: "visitors", op: "gt", value: "5" }] }] })
    await screen.findByLabelText("Aggregation of visitors")
    expect(screen.queryByText("Steps")).toBeNull()
    openTab("Sort & filter")
    expect(await screen.findByText("Steps")).toBeTruthy()
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
    renderBuilder({ ...barDef, other: true, limit: 5, steps: undefined })
    await screen.findByLabelText("Aggregation of visitors")
    // The questions would be in the Data tab, open now.
    expect(screen.queryByText("More questions")).toBeNull()
    expect(screen.queryByText(/Remove .* to combine questions/)).toBeNull()
    openTab("Sort & filter")
    expect(await screen.findByText("While this is on, steps and more questions are not available.")).toBeTruthy()
    expect(screen.queryByText("Steps")).toBeNull()
  })

  it("offers 'Other', steps and questions to a bar, and none of them to a KPI", async () => {
    const { unmount } = renderBuilder({ ...barDef, click: { kind: "url", url: "https://example.com/x" } })
    expect(await screen.findByText("More questions")).toBeTruthy()
    openTab("Sort & filter")
    await screen.findByLabelText("Group the rest as Other")
    expect(screen.getByText("Steps")).toBeTruthy()
    unmount()
    cleanup()
    renderBuilder({ title: "K", kind: "kpi", mart: "mart_visits", measures: ["visitors"], aggregate: "sum" })
    await screen.findByLabelText("Aggregation of visitors").catch(() => screen.findByText("Compare with"))
    expect(screen.queryByText("More questions")).toBeNull()
    openTab("Sort & filter")
    await screen.findByText("Filters")
    expect(screen.queryByLabelText("Group the rest as Other")).toBeNull()
    expect(screen.queryByText("Sort")).toBeNull()
    expect(screen.queryByText("Steps")).toBeNull()
    openTab("Style")
    expect(screen.queryByText("Reference lines")).toBeNull()
  })
})

describe("ChartBuilder single-number payload (QA LH-BI-2-01)", () => {
  it("sends a reusable metric of a KPI as { metric }, not as the raw row column", async () => {
    renderBuilder({ title: "K", kind: "kpi", mart: "mart_visits", measures: [{ metric: "coba_metrik" }] as never, aggregate: "sum" })
    await waitFor(() => expect(bodies.length).toBeGreaterThan(0))
    expect(bodies[bodies.length - 1]!.measures).toEqual([{ metric: "coba_metrik" }])
  })

  it("keeps a plain column of a KPI as the column name, as before", async () => {
    renderBuilder({ title: "K", kind: "kpi", mart: "mart_visits", measures: ["visitors"], aggregate: "sum" })
    await waitFor(() => expect(bodies.length).toBeGreaterThan(0))
    expect(bodies[bodies.length - 1]!.measures).toEqual(["visitors"])
  })
})

describe("ChartBuilder dimension beside more questions (QA LH-BI-13-01)", () => {
  /** The fields answer names `visitors` a number column, as a year column would be. */
  function withNumberKinds() {
    const inner = global.fetch
    global.fetch = mock(async (input: RequestInfo | URL, init?: RequestInit) => {
      const res = await inner(input, init)
      const url = String(input)
      if (url.includes("/api/dashboard/fields") && url.includes("mart=")) {
        const body = (await res.json()) as Record<string, unknown>
        return new Response(JSON.stringify({ ...body, kinds: { visitors: "number", stay: "number" } }), { status: 200, headers: { "Content-Type": "application/json" } })
      }
      return res
    }) as unknown as typeof fetch
  }
  const layered = {
    ...barDef,
    dimension: "provinsi",
    // The question's own dimension must line up with the chart's (both numbers), as it does
    // for a year column, or the builder drops it and the chart cannot be sent at all.
    layers: [{ mart: "mart_visits", dimension: "stay", measures: ["stay"], aggregate: "sum", draw: "line", axis: "right" }],
  } as unknown as ChartDef

  it("still offers a number column as the dimension when the chart has more questions", async () => {
    withNumberKinds()
    renderBuilder({ ...layered, dimension: "visitors" })
    await screen.findByLabelText("Aggregation of visitors")
    // The trigger of the Dimension (X) select is the one in the same field as its label.
    const field = screen.getByText(/^Dimension \(X\)/).closest("div") as HTMLElement
    fireEvent.click(field.querySelector('[role="combobox"]') as HTMLElement)
    // `stay` is a number column, in no way a dimension of the source: only the rule that keeps numbers offers it.
    expect(await screen.findByRole("option", { name: "stay" })).toBeTruthy()
    expect(screen.getByRole("option", { name: "provinsi" })).toBeTruthy()
  })

  it("saves that number dimension without a bin, because bins are off beside more questions", async () => {
    withNumberKinds()
    renderBuilder({ ...layered, dimension: "visitors" })
    await waitFor(() => expect(bodies.length).toBeGreaterThan(0))
    expect(bodies[bodies.length - 1]!.dimension).toBe("visitors")
    expect(bodies[bodies.length - 1]!.dimensionBin).toBeUndefined()
  })
})

describe("ChartBuilder kind switch (phase 2 QA R6)", () => {
  it("keeps the dimension as the first grouping of a data table and back as the dimension of a chart", async () => {
    renderBuilder({ ...barDef, breakdown: "category" })
    await screen.findByLabelText("Aggregation of visitors")
    // `BI-58` T3: both types are icons in the canvas toolbar.
    fireEvent.click(screen.getByRole("button", { name: "Data table" }))
    // The dimension and the breakdown are the first groupings, so the steps editor has something to keep.
    expect(await screen.findByLabelText("Move provinsi down")).toBeTruthy()
    expect(screen.getByLabelText("Remove category")).toBeTruthy()
    fireEvent.click(screen.getByRole("button", { name: "Bar" }))
    await waitFor(() => expect(screen.queryByLabelText("Remove category")).toBeNull())
    expect(screen.getAllByText("provinsi").length).toBeGreaterThan(0)
  })
})

/** The pressed type in the canvas toolbar (`BI-58` T3). */
const pressedKind = () => within(screen.getByRole("group", { name: "Chart type" })).getAllByRole("button").find((b) => b.getAttribute("aria-pressed") === "true")?.getAttribute("aria-label")
const gallery = () => screen.getByRole("dialog", { name: "Chart types" })

describe("ChartBuilder type list (phase 2 QA R9, BI-58 T3)", () => {
  // `BI-58` T1, T3: the long chart-type column is gone; the type is an icon above the canvas, for a new chart and an edited one.
  it("has no chart-type column, and a new chart changes its type from the canvas toolbar", async () => {
    renderBuilder()
    expect(screen.queryByRole("navigation", { name: "Chart type" })).toBeNull()
    expect(pressedKind()).toBe("Horizontal bar (ranking)")
    fireEvent.click(screen.getByRole("button", { name: "Line" }))
    await waitFor(() => expect(pressedKind()).toBe("Line"))
    expect(window.localStorage.length).toBe(0)
  })

  it("searches every type in the gallery behind More, and Enter picks the first match", async () => {
    renderBuilder(barDef)
    fireEvent.click(screen.getByRole("button", { name: "More chart types" }))
    const search = await screen.findByLabelText("Search chart types")
    fireEvent.change(search, { target: { value: "sankey" } })
    expect(within(gallery()).queryByRole("button", { name: "Line" })).toBeNull()
    fireEvent.keyDown(search, { key: "Enter" })
    // The first match was picked and the gallery closed.
    await waitFor(() => expect(screen.queryByLabelText("Search chart types") === null).toBe(true))
    expect(screen.getByRole("button", { name: /More chart types, now Sankey/ })).toBeTruthy()
  })

  it("reaches every kind of KIND_GROUPS from the toolbar or the gallery", async () => {
    renderBuilder(barDef)
    fireEvent.click(screen.getByRole("button", { name: "More chart types" }))
    await screen.findByLabelText("Search chart types")
    for (const g of KIND_GROUPS) for (const item of g.items) {
      expect(within(gallery()).getByRole("button", { name: item.label }), item.value).toBeTruthy()
    }
  })

  it("closes the gallery on Escape and on an outside press, and focus returns to its trigger (phase 2 QA R17)", async () => {
    renderBuilder(barDef)
    const trigger = () => screen.getByRole("button", { name: "More chart types" })
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
})

// `BI-58` T4: the accordion of phase 2 QA R10 became the inspector's tabs; the folded summaries are the tabs' hover titles.
describe("ChartBuilder inspector placement (phase 2 QA R10, BI-58 T4)", () => {
  it("opens a new chart on the Data tab with the source in the data panel, and a tab opens by a click", async () => {
    renderBuilder()
    await screen.findByText("Data source")
    expect(screen.getByRole("complementary", { name: "Data" }).textContent).toContain("Data source")
    expect(selectedTab()).toBe("Data")
    openTab("Sort & filter")
    await waitFor(() => expect(selectedTab()).toBe("Sort & filter"))
    expect(screen.queryByLabelText("Measure (Y)")).toBeNull()
  })

  it("has no Next button, even once the source is picked (phase 2 QA R19)", async () => {
    renderBuilder()
    fireEvent.click(await screen.findByRole("combobox", { name: /pick a mart/i }).catch(() => screen.getByText(/pick a mart/)))
    fireEvent.click(await screen.findByRole("option", { name: /mart_visits/ }))
    await screen.findByText("Data source")
    expect(screen.queryByRole("button", { name: /^Next/ })).toBeNull()
  })

  it("names what an edited chart shows and how it is sorted and filtered on the tabs, with its source in the data panel", async () => {
    renderBuilder({ ...barDef, measures: ["visitors", { column: "stay", aggregate: "avg" }] as never, filters: [{ column: "category", op: "eq", value: "a" }] as never, limit: 5 })
    await waitFor(() => expect(screen.getByRole("complementary", { name: "Data" }).textContent).toContain("mart_visits"))
    await waitFor(() => expect(screen.getByRole("tab", { name: "Data" }).getAttribute("title")).toContain("provinsi · visitors, avg of stay"))
    expect(screen.getByRole("tab", { name: "Sort & filter" }).getAttribute("title")).toContain("highest first · top 5 · 1 filter")
  })

  it("opens the tab that holds a missing field and focuses the field when saving", async () => {
    renderBuilder({ title: "T", kind: "bar", mart: "mart_visits", measures: ["visitors"], aggregate: "sum" })
    await screen.findByLabelText("Aggregation of visitors")
    openTab("Style")
    fireEvent.click(screen.getByRole("button", { name: "Save changes" }))
    await waitFor(() => expect(selectedTab()).toBe("Data"))
    expect(screen.getByRole("alert").textContent).toContain("Dimension (X)")
    await waitFor(() => expect((document.activeElement as HTMLElement).closest("[data-slot=dimension-field]")).not.toBeNull())
    expect(document.activeElement?.hasAttribute("data-placeholder")).toBe(true)
  })

  it("focuses the source select in the data panel when a new chart has no source", async () => {
    renderBuilder()
    await screen.findByText("Data source")
    fireEvent.click(screen.getByRole("button", { name: "Create chart" }))
    await waitFor(() => expect((document.activeElement as HTMLElement).closest("aside")?.getAttribute("aria-label")).toBe("Data"))
    const focused = document.activeElement as HTMLElement
    expect(focused.hasAttribute("data-placeholder")).toBe(true)
  })
})

describe("ChartBuilder header (phase 2 QA R11, BI-58 T1)", () => {
  it("keeps the dashboard, the width, Cancel and Save in one header row, the title on the canvas, and no helper line under the title", async () => {
    renderBuilder()
    const header = (await screen.findByRole("heading", { level: 1 })).closest("header") as HTMLElement
    expect(header.querySelector('[aria-label="Add to dashboard"]')).not.toBeNull()
    expect(header.querySelector('[role="group"][aria-label="Width"]')?.textContent).toBe("Half widthFull width")
    expect([...header.querySelectorAll("button")].map((b) => b.textContent)).toEqual(expect.arrayContaining(["Cancel", "Create chart"]))
    const title = screen.getByLabelText(/^Title/)
    expect(header.contains(title)).toBe(false)
    expect(title.closest('section[aria-label="Chart"]')).not.toBeNull()
    expect(screen.queryByText(/Left empty, the chart is saved as/)).toBeNull()
  })

  it("sets the width from the header's segmented control", async () => {
    renderBuilder(barDef)
    await waitFor(() => expect(bodies.length).toBeGreaterThan(0))
    const full = screen.getByRole("button", { name: "Full width" })
    expect(full.getAttribute("aria-pressed")).toBe("false")
    fireEvent.click(full)
    expect(full.getAttribute("aria-pressed")).toBe("true")
    await waitFor(() => expect(bodies[bodies.length - 1]!.span).toBe(2))
  })
})

describe("ChartBuilder long column names (phase 2 QA R14)", () => {
  it("shows a picked joined measure by its own name with the table as a tag, and the full reference on hover", async () => {
    renderBuilder({ ...barDef, measures: ["mart_demo_province_capacity__rooms"] })
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
    await screen.findByLabelText("Aggregation of rooms").catch(() => screen.findByRole("tabpanel"))
    // `BI-58` T4: each tab's form is the grid; the rule reaches every grid inside it, and the tab's
    // panel scrolls inside the inspector, never sideways.
    const form = screen.getByRole("tabpanel").firstElementChild as HTMLElement
    expect(form.className).toContain("grid-cols-[minmax(0,1fr)]")
    expect(form.className).toContain("min-w-0")
    expect(form.className).toContain("[&_.grid:not([class*=grid-cols])]:grid-cols-[minmax(0,1fr)]")
  })
})

describe("ChartBuilder conditional controls (phase 2 QA R15)", () => {
  it("puts the bin of a number dimension in a row of its own, so Dimension and Breakdown stay in one row", async () => {
    renderBuilder({ ...barDef, dimension: "visitors", measures: ["stay"] })
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
    // `BI-58` T4: two columns where the inspector is wide enough (a container query), one in the narrow column.
    expect(row.className).toContain("@md:col-span-2")
    expect(dimension.parentElement?.className).toContain("@md:grid-cols-2")
  })

  it("draws no extra row when the dimension needs no bin or grain", async () => {
    renderBuilder(barDef)
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

  // `WEB-7` D6, `BI-58` T1: the page is named last in the header's breadcrumb, after the
  // dashboard, and the kind is named on the control that changes it.
  it("names the page in the header after the dashboard, the kind on its control, and no chevron in the title", async () => {
    const { unmount } = renderBuilder()
    const heading = screen.getByRole("heading", { level: 1 })
    expect(heading.textContent).toBe("New chart")
    expect(heading.querySelector("svg")).toBeNull()
    expect(heading.parentElement?.textContent).toBe("Main/New chart")
    expect(pressedKind()).toBe("Horizontal bar (ranking)")
    unmount()
    cleanup()
    renderBuilder(barDef)
    expect(screen.getByRole("heading", { level: 1 }).textContent).toBe("Edit chart")
    expect(pressedKind()).toBe("Bar")
  })
})

// `BI-58` T1: the resizable form-and-preview split (phase 2 QA R18) became three zones.
describe("ChartBuilder zones (BI-58 T1)", () => {
  it("puts the data panel, the chart with its preview, and the settings side by side", async () => {
    renderBuilder(barDef)
    await screen.findByLabelText("Aggregation of visitors")
    const zones = [...document.querySelectorAll("[data-slot=chart-builder] aside, [data-slot=chart-builder] section[aria-label=Chart]")]
    expect(zones.map((z) => z.getAttribute("aria-label"))).toEqual(["Data", "Chart", "Settings"])
    expect(zones[1]!.querySelector('section[aria-label="Preview"]')).not.toBeNull()
    expect(zones[2]!.querySelector('[aria-label="Aggregation of visitors"]')).not.toBeNull()
    expect(document.querySelector('[data-slot="resizable-handle"]')).toBeNull()
  })
})

// BI-17 part A, T7: the Format section.
const FULL_STYLE = {
  format: { style: "compact", currency: "IDR", decimals: 2 },
  valueFormats: { visitors: { style: "percent", decimals: 1 } },
  dateFormat: { style: "monthYear", abbreviate: true },
  colors: { palette: "muted", series: { visitors: "#112233" }, byValue: ["#dbeafe", "#1d4ed8"] },
  axes: { x: { title: "Province", rotate: 45 }, y: { title: "Visitors", min: 0, max: 5000, log: true, show: false } },
  labels: { show: true },
  annotations: [{ x: "Bali", text: "Peak season" }],
  tooltip: { fields: ["visitors"] },
}

describe("ChartBuilder Format section (BI-17 part A)", () => {
  it("reopens a chart saved with every field and sends the same style back", async () => {
    renderBuilder({ ...barDef, ...FULL_STYLE } as ChartDef)
    await screen.findByLabelText("Aggregation of visitors")
    openTab("Style")
    // The controls hold what was saved.
    expect((screen.getByLabelText("Number format decimals") as HTMLInputElement).value).toBe("2")
    expect((screen.getByLabelText("Number format currency code") as HTMLInputElement).value).toBe("IDR")
    expect((screen.getByLabelText("X axis title") as HTMLInputElement).value).toBe("Province")
    expect((screen.getByLabelText("Y axis maximum") as HTMLInputElement).value).toBe("5000")
    expect((screen.getByLabelText("Note 1 text") as HTMLInputElement).value).toBe("Peak season")
    expect(screen.getByLabelText("Palette").textContent).toContain("Muted")
    // The preview request carries the style exactly as saved.
    await waitFor(() => expect(bodies.some((b) => b.axes)).toBe(true))
    const sent = bodies[bodies.length - 1]!
    for (const [field, value] of Object.entries(FULL_STYLE)) expect(sent[field], field).toEqual(value)
  })

  it("a bar chart offers colours, axes, labels, notes and reference lines, and no conditional colours", async () => {
    renderBuilder(barDef)
    await screen.findByLabelText("Aggregation of visitors")
    openTab("Style")
    for (const title of ["Values", "Dates", "Colours", "Axes", "Labels", "Notes", "Tooltip"]) expect(screen.getByText(title), title).toBeTruthy()
    expect(screen.getByText("Reference lines")).toBeTruthy()
    expect(screen.queryByText("Conditional colours")).toBeNull()
  })

  it("a table offers conditional colours and number formats, and no axes or palette", async () => {
    renderBuilder({ title: "T", kind: "table", mart: "mart_visits", dimension: "provinsi", measures: ["visitors"], aggregate: "sum" })
    await screen.findByLabelText("Aggregation of visitors")
    openTab("Style")
    expect(screen.getByText("Conditional colours", { selector: "legend" })).toBeTruthy()
    expect(screen.getByText("Values")).toBeTruthy()
    for (const title of ["Axes", "Colours", "Labels", "Notes", "Tooltip"]) expect(screen.queryByText(title), title).toBeNull()
  })

  it("does not send a conditional rule that has no column yet", async () => {
    renderBuilder({ title: "T", kind: "table", mart: "mart_visits", dimension: "provinsi", measures: ["visitors"], aggregate: "sum" })
    await screen.findByLabelText("Aggregation of visitors")
    openTab("Style")
    fireEvent.click(screen.getByRole("button", { name: /Add a rule/ }))
    fireEvent.change(screen.getByLabelText("Rule 1 value"), { target: { value: "1000" } })
    await waitFor(() => expect(bodies.length).toBeGreaterThan(0))
    expect(bodies[bodies.length - 1]!.conditional).toBeUndefined()
  })

  it("a line added to a bar chart is a goal line at the number given", async () => {
    renderBuilder(barDef)
    await screen.findByLabelText("Aggregation of visitors")
    openTab("Style")
    fireEvent.click(screen.getByRole("button", { name: /Add a line/ }))
    fireEvent.change(screen.getByLabelText("Line 1 value"), { target: { value: "1000" } })
    fireEvent.change(screen.getByLabelText("Line 1 label"), { target: { value: "Goal" } })
    await waitFor(() => expect(bodies.some((b) => b.referenceLines)).toBe(true))
    expect(bodies[bodies.length - 1]!.referenceLines).toEqual([{ value: 1000, label: "Goal" }])
  })
})

describe("ChartBuilder freshness (BI-4 part B)", () => {
  it("shows the chart's own rule, opening the fold it sits in, and refuses minutes out of range in a sentence", async () => {
    renderBuilder({ ...barDef, cache: { mode: "minutes", minutes: 30 } })
    const minutes = (await screen.findByLabelText("Freshness: minutes")) as HTMLInputElement
    expect(minutes.value).toBe("30")
    fireEvent.change(minutes, { target: { value: "5000" } })
    expect((await screen.findByRole("alert")).textContent).toBe("Remember for between 1 minute and 24 hours (1 to 1440 minutes).")
  })

  it("offers 'Follow the default' to a chart with no rule of its own and no minutes box", async () => {
    renderBuilder(barDef)
    expect(await screen.findByLabelText("Freshness")).toBeTruthy()
    expect(screen.queryByLabelText("Freshness: minutes")).toBeNull()
  })
})

describe("ChartBuilder scatter (BI-17 B QA fix F2)", () => {
  it("asks for an X metric and a Y metric, and a size for a bubble chart", async () => {
    renderBuilder(barDef)
    await screen.findByLabelText("Aggregation of visitors")
    fireEvent.click(screen.getByRole("button", { name: "More chart types" }))
    fireEvent.click(await screen.findByRole("button", { name: "Scatter (X vs Y)" }))
    await waitFor(() => expect(screen.getByText("X metric")).toBeTruthy())
    expect(screen.getByText("Y metric")).toBeTruthy()
    expect(screen.queryByText("Size metric")).toBeNull()
  })

  it("does the same when the chart is new and the data source is picked after the kind", async () => {
    renderBuilder()
    fireEvent.click(screen.getByRole("button", { name: "More chart types" }))
    const scatter = await screen.findByRole("button", { name: "Scatter (X vs Y)" })
    fireEvent.click(scatter)
    await waitFor(() => expect(screen.getByText("X metric")).toBeTruthy())
    expect(screen.getByText("Y metric")).toBeTruthy()
  })

  it("keeps the Y metric when the data source is picked after the kind (it used to reset to one row)", async () => {
    renderBuilder()
    fireEvent.click(screen.getByRole("button", { name: "More chart types" }))
    fireEvent.click(await screen.findByRole("button", { name: "Bubble (X, Y, size)" }))
    fireEvent.click(await screen.findByRole("combobox", { name: /pick a mart/i }).catch(() => screen.getByText(/pick a mart/)))
    fireEvent.click(await screen.findByRole("option", { name: /mart_visits/ }))
    await screen.findByText("Data source")
    // `BI-58` T4: the Data tab is the one a new chart opens on.
    expect(selectedTab()).toBe("Data")
    await waitFor(() => expect(screen.getByText("X metric")).toBeTruthy())
    expect(screen.getByText("Y metric")).toBeTruthy()
    expect(screen.getByText("Size metric")).toBeTruthy()
  })
})

describe("ChartBuilder chosen mart mark (QA LH-DATA-12-17)", () => {
  /** The `beforeEach` stub, with the marts answer replaced. */
  function stubMarts(marts: unknown[]) {
    const base = global.fetch
    global.fetch = mock(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = String(input)
      if (url.includes("/api/dashboard/fields") && !url.includes("mart=")) {
        return new Response(JSON.stringify({ marts }), {
          status: 200,
          headers: { "Content-Type": "application/json" },
        })
      }
      return (base as typeof fetch)(input, init)
    }) as unknown as typeof fetch
  }

  // QA `LH-DATA-12-17` F1 (`DATA-54`): the picker's trigger shows the mart's
  // name only, so the mark of the list row was lost once a mart was chosen.
  it("shows Certified under the picker once a certified mart is chosen", async () => {
    stubMarts([{ name: "mart_visits", rows: 10, certification: "certified" }])
    renderBuilder(barDef)
    const mark = await screen.findByTestId("chosen-mart-mark")
    expect(mark.textContent).toContain("Certified")
  })

  it("shows no mark line for a mart without one", async () => {
    renderBuilder(barDef)
    // The form has loaded its marts once the chosen mart is on screen.
    expect((await screen.findAllByText(/mart_visits/)).length).toBeGreaterThan(0)
    expect(screen.queryByTestId("chosen-mart-mark")).toBeNull()
  })
})

// `BI-47` (QA `LH-BI-6-01`): a dashboard parameter slot (`{ukuran}`) is a
// dimension, a breakdown and a measure, never a filter column — the server has
// no column to list its values from.
describe("ChartBuilder and a parameter slot (QA LH-BI-6-01)", () => {
  /** The board's parameters: one list whose values are columns, so the builder offers it as a dimension. */
  function withListParam() {
    const inner = global.fetch
    global.fetch = mock(async (input: RequestInfo | URL, init?: RequestInit) => {
      if (String(input).includes("/api/dashboard/params")) {
        const params = [{
          id: "p1", name: "ukuran", label: "Ukuran", type: "list", default: "provinsi",
          values: [{ value: "provinsi", label: "provinsi" }, { value: "category", label: "category" }],
        }]
        return new Response(JSON.stringify({ params }), { status: 200, headers: { "Content-Type": "application/json" } })
      }
      return inner(input, init)
    }) as unknown as typeof fetch
  }

  it("a parameter slot is offered as a dimension and not as a filter column", async () => {
    withListParam()
    renderBuilder(barDef)
    await screen.findByLabelText("Aggregation of visitors")
    // The slot is a dimension of the picker.
    const dimensionField = screen.getByText(/^Dimension \(X\)/).closest("div") as HTMLElement
    fireEvent.click(dimensionField.querySelector('[role="combobox"]') as HTMLElement)
    expect(await screen.findByRole("option", { name: /\{ukuran\}/ })).toBeTruthy()
    // Close that list, open the filter section, and ask for a filter column.
    fireEvent.keyDown(document.activeElement ?? document.body, { key: "Escape" })
    openTab("Sort & filter")
    fireEvent.click(await screen.findByLabelText("Add a filter"))
    expect(await screen.findByRole("option", { name: "provinsi" })).toBeTruthy()
    expect(screen.queryByRole("option", { name: /\{ukuran\}/ })).toBeNull()
  })
})

// `BI-46` (QA `LH-BI-6-01`): a date breakdown can be grouped by a period, and
// the limit says what it counts when the chart has a breakdown.
describe("ChartBuilder breakdown grain and limit label (QA LH-BI-6-01)", () => {
  /** Adds a date column the base stub does not have. */
  function withDateColumn() {
    const inner = global.fetch
    global.fetch = mock(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = String(input)
      const res = await inner(input, init)
      if (url.includes("/api/dashboard/fields") && url.includes("mart=")) {
        const body = (await res.json()) as Record<string, unknown>
        return new Response(JSON.stringify({
          ...body,
          dimensions: [...(body.dimensions as string[]), "d"],
          kinds: { d: "date" },
          columns: [...(body.columns as { name: string; type: string }[]), { name: "d", type: "Date" }],
        }), { status: 200, headers: { "Content-Type": "application/json" } })
      }
      return res
    }) as unknown as typeof fetch
  }

  /** A base-ui Select choice: open, focus the option, click it (as format-section does). */
  function choose(trigger: HTMLElement, option: string) {
    fireEvent.click(trigger)
    const target = screen.getByRole("option", { name: option })
    fireEvent.focus(target)
    fireEvent.click(target)
  }

  it("choosing a date breakdown offers its grain and the preview request carries it", async () => {
    withDateColumn()
    renderBuilder({ ...barDef })
    await screen.findByLabelText("Aggregation of visitors")
    const breakdownField = document.querySelector('[data-slot="breakdown-field"]') as HTMLElement
    choose(breakdownField.querySelector('[role="combobox"]') as HTMLElement, "d")
    const groupBy = await screen.findByLabelText("Group the breakdown by")
    choose(groupBy, "Month")
    await waitFor(() => expect(bodies.some((b) => b.breakdownGrain === "month")).toBe(true))
  })

  it("the limit label names the breakdown when the chart has one", async () => {
    renderBuilder({ ...barDef, breakdown: "category" })
    await screen.findByLabelText("Aggregation of visitors")
    openTab("Sort & filter")
    expect(await screen.findByText("Limit (Top N, dimension and breakdown)")).toBeTruthy()
    cleanup()
    // A grain on a date dimension makes the limit the breakdown's alone.
    withDateColumn()
    renderBuilder({ ...barDef, dimension: "d", breakdown: "category", grain: "month" })
    await screen.findByLabelText("Aggregation of visitors")
    openTab("Sort & filter")
    expect(await screen.findByText("Breakdown values (Top N)")).toBeTruthy()
  })

  /** Adds a table-calculation measure the base stub does not have. */
  function withTableCalc() {
    const inner = global.fetch
    global.fetch = mock(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = String(input)
      const res = await inner(input, init)
      if (url.includes("/api/dashboard/fields") && url.includes("mart=")) {
        const body = (await res.json()) as Record<string, unknown>
        return new Response(JSON.stringify({
          ...body,
          measures: [...(body.measures as string[]), "running"],
          calculated: [{
            id: "f_1", sourceKind: "mart", sourceId: "mart_visits", name: "running",
            formula: "RunningTotal(Sum(visitors))", level: "table", type: "number", createdBy: "u",
          }],
        }), { status: 200, headers: { "Content-Type": "application/json" } })
      }
      return res
    }) as unknown as typeof fetch
  }

  // `BI-46` review round 1, R7: beside a breakdown the limit counts breakdown
  // values, so a grain pick keeps the untouched 20 instead of lifting it to the
  // grained maximum, which the preview request would carry as no cap at all.
  // A NEW chart: an edited one opens with its saved limit already "touched".
  it("a grain pick beside a breakdown keeps the untouched limit at twenty", async () => {
    withDateColumn()
    renderBuilder()
    fireEvent.click(await screen.findByRole("combobox", { name: /pick a mart/i }).catch(() => screen.getByText(/pick a mart/)))
    fireEvent.click(await screen.findByRole("option", { name: /mart_visits/ }))
    // The Data tab is open; its pickers wait for the source's columns.
    const dimensionField = await waitFor(() => {
      const field = document.querySelector("[data-slot=dimension-field]") as HTMLElement
      expect(field.querySelector('[role="combobox"]')?.hasAttribute("data-disabled")).toBe(false)
      return field
    })
    choose(dimensionField.querySelector('[role="combobox"]') as HTMLElement, "d")
    // The pickers' lists close after a choice; wait for it so the next list is
    // the only one with its options.
    await waitFor(() => expect(screen.queryAllByRole("option")).toHaveLength(0))
    const breakdownField = document.querySelector('[data-slot="breakdown-field"]') as HTMLElement
    choose(breakdownField.querySelector('[role="combobox"]') as HTMLElement, "category")
    await waitFor(() => expect(screen.queryAllByRole("option")).toHaveLength(0))
    choose(await screen.findByLabelText("Measure (Y)"), "visitors")
    await waitFor(() => expect(screen.queryAllByRole("option")).toHaveLength(0))
    choose(await screen.findByLabelText("Group by"), "Month")
    await waitFor(() => expect(bodies.some((b) => b.grain === "month" && b.limit === 20)).toBe(true))
    expect(bodies.some((b) => b.grain === "month" && b.limit === 1000)).toBe(false)
  })

  // `BI-46` review round 1, R4: the server builds no breakdown cap for a table
  // calculation, so the field must not be labelled as one.
  it("a table-calculation measure keeps the limit label the chart always had", async () => {
    withTableCalc()
    renderBuilder({ ...barDef, measures: ["running"], breakdown: "category" })
    await screen.findByLabelText("Aggregation of running")
    openTab("Sort & filter")
    expect(await screen.findByText("Limit (Top N)")).toBeTruthy()
    expect(screen.queryByText("Limit (Top N, dimension and breakdown)")).toBeNull()
    expect(screen.queryByText("Breakdown values (Top N)")).toBeNull()
  })
})

// QA `LH-BI-6-25` (`BI-49`): a radar compares measures like a bar does, so the
// builder offers "Add measure" and the preview carries both.
describe("ChartBuilder radar measures (QA LH-BI-6-25)", () => {
  /** A base-ui Select choice: open, focus the option, click it (as format-section does). */
  function choose(trigger: HTMLElement, option: string) {
    fireEvent.click(trigger)
    const target = screen.getByRole("option", { name: option })
    fireEvent.focus(target)
    fireEvent.click(target)
  }

  it("a radar offers Add measure and sends both measures", async () => {
    renderBuilder({ ...barDef, kind: "radar" })
    await screen.findByLabelText("Measure (Y)")
    fireEvent.click(await screen.findByRole("button", { name: /Add measure/ }))
    await waitFor(() => expect(screen.queryAllByRole("option")).toHaveLength(0))
    choose(await screen.findByLabelText("Measure 2"), "stay")
    await waitFor(() => expect(bodies.some((b) => Array.isArray(b.measures) && b.measures.length === 2)).toBe(true))
    const body = bodies[bodies.length - 1]!
    expect(body.kind).toBe("radar")
    expect(body.measures).toEqual(["visitors", "stay"])
  })
})
