import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react"
import { afterAll, afterEach, beforeEach, describe, expect, it, mock, spyOn } from "bun:test"

import * as realAuth from "@/features/auth/auth-provider"
import * as realEchartModule from "./echart"

// As in chart-builder.test.tsx: a plain editor, and the real module put back afterwards.
const real = { ...realAuth }
mock.module("@/features/auth/auth-provider", () => ({ ...real, useAuth: () => ({ hasPermission: () => false }) }))
// ECharts draws on a canvas, which the test DOM does not have; each chart is a marker with its kind.
const realEchart = { ...realEchartModule }
type DrawnOption = { series?: { type?: string; data?: unknown[] }[] }
let drawn: DrawnOption[] = []
mock.module("./echart", () => ({
  EChart: (props: { option: DrawnOption }) => {
    drawn.push(props.option)
    return <div data-testid="chart" data-type={props.option.series?.[0]?.type ?? ""} />
  },
}))
afterAll(() => {
  mock.module("@/features/auth/auth-provider", () => real)
  mock.module("./echart", () => realEchart)
})

import { dashboardService, metricService, paramService } from "@/services"
import { formatLocaleNumber } from "@/lib/format"
import { ChartBuilder, type ChartDef } from "./chart-builder"
import { ReportingProvider } from "./reporting-context"

// `BI-58`: the builder's new layout keeps every saved chart as it was (spec T5).
const originalFetch = global.fetch
const happyDOM = (window as unknown as { happyDOM?: { settings: { disableIframePageLoading: boolean } } }).happyDOM
if (happyDOM) happyDOM.settings.disableIframePageLoading = true

/** The fields of the stub source: two text columns, a date, a timestamp and three numbers. */
const FIELDS = {
  dimensions: ["provinsi", "category", "begins", "ends"],
  measures: ["visitors", "stay", "target"],
  calculated: [], hierarchies: [],
  columns: [
    { name: "provinsi", type: "String" }, { name: "category", type: "String" },
    { name: "begins", type: "Date" }, { name: "ends", type: "DateTime" },
    { name: "visitors", type: "UInt64" }, { name: "stay", type: "Float64" }, { name: "target", type: "Float64" },
  ],
}

/** The fields the stub answers; a test may take columns away. */
let fieldsAnswer: typeof FIELDS = FIELDS
/** The dashboard's parameters the stub answers. */
let paramsAnswer: unknown[] = []
let fetches: string[] = []
let previews: Record<string, unknown>[] = []
/** The preview route's answer: an error unless a test draws a chart. */
let previewAnswer: ((body: Record<string, unknown>) => { status: number; body: unknown }) | null = null

beforeEach(() => {
  fetches = []
  previews = []
  drawn = []
  previewAnswer = null
  fieldsAnswer = FIELDS
  paramsAnswer = []
  window.localStorage.clear()
  global.fetch = mock(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input)
    fetches.push(url)
    const json = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } })
    if (url.includes("/specs/preview")) {
      const body = init?.body ? JSON.parse(String(init.body)) as Record<string, unknown> : {}
      previews.push(body)
      if (previewAnswer) {
        const answer = previewAnswer(body)
        return json(answer.body, answer.status)
      }
      return json({ error: "not in this test" }, 400)
    }
    if (url.includes("/api/dashboard/fields") && url.includes("mart=")) return json(fieldsAnswer)
    if (url.includes("/api/dashboard/fields")) return json({ marts: [{ name: "mart_visits", rows: 1571564 }] })
    if (url.includes("/api/dashboard/sources")) return json({ sources: [] })
    if (url.includes("/api/dashboard/metrics") && !url.includes("usage")) return json({ metrics: [] })
    if (url.includes("/api/dashboard/params")) return json({ params: paramsAnswer })
    return json({ error: "not in this test" }, 500)
  }) as unknown as typeof fetch
})
afterEach(() => {
  global.fetch = originalFetch
  cleanup()
})

function renderBuilder(initial?: ChartDef, onSaved: (saved: ChartDef) => void = () => {}) {
  return render(
    <ReportingProvider reporting={{ timeZone: "UTC", weekStart: "monday" }}>
      <ChartBuilder onSaved={onSaved} onCancel={() => {}} initial={initial} editId={initial ? "c_1" : undefined} />
    </ReportingProvider>,
  )
}

/** What travels over the wire: `undefined` fields are dropped, as `JSON.stringify` drops them. */
const wire = (v: unknown) => JSON.parse(JSON.stringify(v)) as Record<string, unknown>

/**
 * One saved chart of each family, written as the builder writes it (every
 * field it sends, in its own shape), so a save of the unchanged chart must
 * send exactly this back.
 */
const SAVED: Record<string, ChartDef> = {
  bar: {
    title: "Visitors by province", mart: "mart_visits", kind: "bar", dimension: "provinsi",
    measures: ["visitors"], aggregate: "sum", span: 1, board: "default", order: "desc", limit: 20,
  } as ChartDef,
  line: {
    title: "Visitors by month", mart: "mart_visits", kind: "line", dimension: "begins",
    measures: ["visitors", { column: "stay", aggregate: "avg" }], aggregate: "sum", span: 2, board: "default",
    breakdown: "category", order: "none", limit: 12, grain: "month",
  } as unknown as ChartDef,
  kpi: {
    title: "Visitors", kind: "kpi", mart: "mart_visits", measures: ["visitors"], aggregate: "sum", span: 1, board: "default",
    caption: "this month", compare: { kind: "previous", dateColumn: "begins", period: "month" }, goodDirection: "down",
  } as unknown as ChartDef,
  table: {
    title: "Province table", mart: "mart_visits", kind: "table", dimension: "provinsi",
    measures: ["visitors", { column: "stay", aggregate: "max" }], aggregate: "sum", span: 2, board: "default",
    order: "desc", limit: 50,
  } as unknown as ChartDef,
  gantt: {
    title: "Plan", mart: "mart_visits", kind: "gantt", dimension: "provinsi", span: 1, board: "default",
    breakdown: "category", start: "begins", end: "ends", progress: "stay",
  } as unknown as ChartDef,
  combined: {
    title: "Visitors and stay", mart: "mart_visits", kind: "bar", dimension: "provinsi",
    measures: ["visitors"], aggregate: "sum", span: 2, board: "default", order: "desc", limit: 20,
    layers: [{ mart: "mart_visits", dimension: "provinsi", measures: [{ column: "stay", aggregate: "avg" }], aggregate: "sum", draw: "line", axis: "right" }],
  } as unknown as ChartDef,
}

describe("ChartBuilder keeps a saved chart as it was (BI-58, spec T5)", () => {
  for (const [family, def] of Object.entries(SAVED)) {
    it(`saves an unchanged ${family} chart back as the same definition`, async () => {
      const save = spyOn(dashboardService, "saveChart").mockResolvedValue({ id: "c_1" })
      let saved: ChartDef | null = null
      try {
        renderBuilder(def, (d) => { saved = d })
        // The source's columns are loaded once its own picks are back on screen; the preview asks for them.
        await waitFor(() => expect(previews.length).toBeGreaterThan(0))
        fireEvent.click(screen.getByRole("button", { name: "Save changes" }))
        await waitFor(() => expect(save).toHaveBeenCalledTimes(1))
        const [payload, edit] = save.mock.calls[0]!
        expect(edit).toBe(true)
        expect(wire(payload)).toEqual({ ...wire(def), id: "c_1" })
        await waitFor(() => expect(saved).not.toBeNull())
      } finally {
        save.mockRestore()
      }
    })
  }
})

/** A bar-like preview answer over the stub's rows: the X column and the first value, as the preview route returns them. */
function drawRows(body: Record<string, unknown>) {
  const x = String(body.dimension)
  const y = String((body.measures as unknown[])[0])
  return {
    status: 200,
    body: {
      spec: { id: "preview", title: "Preview", kind: body.kind, mart: "mart_visits", sql: "", x, y, source: "ui" },
      result: { columns: [x, y], rows: [{ [x]: "2024-01-01", [y]: 1571564 }, { [x]: "2024-02-01", [y]: 1204311 }] },
    },
  }
}

describe("the data panel (BI-58 T2, spec T4)", () => {
  async function pickSource() {
    renderBuilder()
    const panel = screen.getByRole("complementary", { name: "Data" })
    fireEvent.click(within(panel).getByRole("combobox"))
    fireEvent.click(await screen.findByRole("option", { name: /mart_visits/ }))
    await within(panel).findByRole("group", { name: "Dates" })
    return panel
  }

  it("lists the source's columns as Dates, Text and Numbers", async () => {
    const panel = await pickSource()
    const names = (group: string) => [...within(panel).getByRole("group", { name: group }).querySelectorAll("button")].map((b) => b.getAttribute("aria-label"))
    expect(names("Dates")).toEqual(["begins", "ends"])
    expect(names("Text")).toEqual(["provinsi", "category"])
    expect(names("Numbers")).toEqual(["visitors", "stay", "target"])
  })

  it("gives a preview from a source, one click on a date column and one on a number column", async () => {
    previewAnswer = drawRows
    const panel = await pickSource()
    fireEvent.click(within(panel).getByRole("button", { name: "begins" }))
    fireEvent.click(within(panel).getByRole("button", { name: "visitors" }))
    // The chips say where each column went, and no menu opened on the way.
    expect(within(panel).getByRole("button", { name: "begins, X axis" })).toBeTruthy()
    expect(within(panel).getByRole("button", { name: "visitors, Value" })).toBeTruthy()
    expect(screen.queryByRole("menu")).toBeNull()
    await waitFor(() => expect(previews.some((b) => b.dimension === "begins" && JSON.stringify(b.measures) === '["visitors"]')).toBe(true))
    const preview = screen.getByRole("region", { name: "Preview" })
    await waitFor(() => expect(within(preview).getByTestId("chart")).toBeTruthy())
    expect(preview.textContent).not.toContain("Preview unavailable")
  })

  it("opens a menu of the uses the chart offers for a column once its slot is taken", async () => {
    const panel = await pickSource()
    fireEvent.click(within(panel).getByRole("button", { name: "provinsi" }))
    expect(within(panel).getByRole("button", { name: "provinsi, X axis" })).toBeTruthy()
    // The X axis is taken: a second text column asks.
    fireEvent.click(within(panel).getByRole("button", { name: "category" }))
    const menu = await screen.findByRole("menu")
    expect([...menu.querySelectorAll("[role=menuitem]")].map((i) => i.textContent)).toEqual(["Use as X axis", "Use as split", "Use as filter"])
    fireEvent.click(within(menu).getByRole("menuitem", { name: "Use as split" }))
    await waitFor(() => expect(within(panel).getByRole("button", { name: "category, Split" })).toBeTruthy())
  })
})

describe("the chart-type gallery (BI-58 T3, spec T3)", () => {
  /** A preview of `n` provinces, as the preview route answers a bar chart. */
  function provinces(n: number) {
    return (body: Record<string, unknown>) => ({
      status: 200,
      body: {
        spec: { id: "preview", title: "Preview", kind: body.kind, mart: "mart_visits", sql: "", x: "provinsi", y: "visitors", source: "ui" },
        result: { columns: ["provinsi", "visitors"], rows: Array.from({ length: n }, (_, i) => ({ provinsi: `P${i}`, visitors: 1000 + i })) },
      },
    })
  }

  it("draws each type it can from the preview's rows, with no request of its own and at most 200 rows", async () => {
    previewAnswer = provinces(250)
    renderBuilder(SAVED.bar)
    const preview = screen.getByRole("region", { name: "Preview" })
    await waitFor(() => expect(within(preview).getByTestId("chart")).toBeTruthy())
    // Every way the builder reaches the server, watched from here on.
    const services = [dashboardService, metricService, paramService].flatMap((svc) =>
      Object.keys(svc).filter((k) => typeof (svc as Record<string, unknown>)[k] === "function").map((k) => spyOn(svc as unknown as Record<string, () => unknown>, k)))
    const before = fetches.length
    drawn = []
    try {
      fireEvent.click(screen.getByRole("button", { name: "More chart types" }))
      const gallery = await screen.findByRole("dialog", { name: "Chart types" })
      const card = (name: string) => within(gallery).getByRole("button", { name })
      // Drawn from the rows: a pie and a line; a KPI needs its own query, so it shows its icon until it is picked.
      await waitFor(() => expect(within(card("Donut / pie")).getByTestId("chart")).toBeTruthy())
      expect(within(card("Line")).getByTestId("chart")).toBeTruthy()
      expect(within(card("KPI — big number")).queryByTestId("chart")).toBeNull()
      // At most 200 of the 250 rows reach a picture.
      const pie = drawn.find((o) => o.series?.[0]?.type === "pie")
      expect(pie?.series?.[0]?.data?.length).toBe(200)
      // The gallery asked for nothing.
      expect(fetches.length).toBe(before)
      for (const spy of services) expect(spy).not.toHaveBeenCalled()
    } finally {
      for (const spy of services) spy.mockRestore()
    }
  })

  it("greys out a type the source cannot make, with one line saying why, and picking another updates the preview", async () => {
    previewAnswer = provinces(5)
    // A source without date columns: no calendar and no Gantt chart.
    fieldsAnswer = {
      ...FIELDS,
      dimensions: ["provinsi", "category"],
      columns: FIELDS.columns.filter((c) => c.type !== "Date" && c.type !== "DateTime"),
    }
    renderBuilder(SAVED.bar)
    await waitFor(() => expect(previews.length).toBeGreaterThan(0))
    fireEvent.click(screen.getByRole("button", { name: "More chart types" }))
    const gallery = await screen.findByRole("dialog", { name: "Chart types" })
    for (const name of ["Calendar heatmap (daily)", "Gantt (tasks over time)"]) {
      const card = within(gallery).getByRole("button", { name }) as HTMLButtonElement
      expect(card.disabled, name).toBe(true)
      expect(document.getElementById(card.getAttribute("aria-describedby") ?? "")?.textContent, name).toBe("Needs a date column")
    }
    // The stub source has three number columns: a bubble chart (three values) can be made, so it is not greyed.
    expect((within(gallery).getByRole("button", { name: "Bubble (X, Y, size)" }) as HTMLButtonElement).disabled).toBe(false)
    const asked = previews.length
    fireEvent.click(within(gallery).getByRole("button", { name: "Area" }))
    await waitFor(() => expect(previews.length).toBeGreaterThan(asked))
    expect(previews[previews.length - 1]!.kind).toBe("area")
  })
})

describe("the folded data panel (BI-58 T5)", () => {
  it("opens the data panel as a sheet from Pick data, where the source and its columns work as in the panel", async () => {
    previewAnswer = drawRows
    renderBuilder()
    // The toolbar's button; the empty canvas offers the same one.
    const buttons = screen.getAllByRole("button", { name: "Pick data" })
    expect(buttons).toHaveLength(2)
    fireEvent.click(buttons[0]!)
    const sheet = await screen.findByRole("dialog", { name: "Data" })
    fireEvent.click(within(sheet).getByRole("combobox"))
    fireEvent.click(await screen.findByRole("option", { name: /mart_visits/ }))
    fireEvent.click(await within(sheet).findByRole("button", { name: "begins" }))
    fireEvent.click(within(sheet).getByRole("button", { name: "visitors" }))
    // Closed, the toolbar's button names the source, and the chart was drawn behind the sheet.
    fireEvent.keyDown(sheet, { key: "Escape" })
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "Data" }) === null).toBe(true))
    expect(screen.getByRole("button", { name: "Data: mart_visits" })).toBeTruthy()
    await waitFor(() => expect(within(screen.getByRole("region", { name: "Preview" })).getByTestId("chart")).toBeTruthy())
  })
})

// `BI-58` T6: the builder findings of the 2026-11-14 QA.
describe("the builder findings of the 2026-11-14 QA (BI-58 T6)", () => {
  const openTab = (name: string) => fireEvent.click(screen.getByRole("tab", { name }))

  it("lets the Limit be emptied and typed into, and puts back the last number when left empty", async () => {
    renderBuilder(SAVED.bar)
    await waitFor(() => expect(previews.length).toBeGreaterThan(0))
    openTab("Sort & filter")
    const limit = (await screen.findByLabelText("Limit")) as HTMLInputElement
    expect(limit.value).toBe("20")
    fireEvent.focus(limit)
    fireEvent.change(limit, { target: { value: "" } })
    expect(limit.value).toBe("")
    fireEvent.change(limit, { target: { value: "7" } })
    expect(limit.value).toBe("7")
    await waitFor(() => expect(previews[previews.length - 1]!.limit).toBe(7))
    fireEvent.change(limit, { target: { value: "" } })
    fireEvent.blur(limit)
    expect(limit.value).toBe("7")
  })

  it("offers a number parameter as the limit, but not on a chart with steps, and says why", async () => {
    paramsAnswer = [{ id: "p1", name: "top", label: "Top", type: "number", default: 10 }]
    const { unmount } = renderBuilder(SAVED.bar)
    await waitFor(() => expect(previews.length).toBeGreaterThan(0))
    openTab("Sort & filter")
    expect(await screen.findByLabelText("Limit from a parameter")).toBeTruthy()
    unmount()
    cleanup()
    renderBuilder({ ...SAVED.bar, steps: [{ filter: [{ column: "visitors", op: "gt", value: "5" }] }] } as ChartDef)
    await waitFor(() => expect(previews.length).toBeGreaterThan(0))
    openTab("Sort & filter")
    expect(await screen.findByText("A parameter cannot set the limit of a chart with steps.")).toBeTruthy()
    expect(screen.queryByLabelText("Limit from a parameter")).toBeNull()
  })

  it("shows the server's lower-case error as a sentence", async () => {
    previewAnswer = () => ({ status: 400, body: { error: "invalid or missing measure column" } })
    renderBuilder(SAVED.bar)
    expect(await screen.findByText("Invalid or missing measure column.")).toBeTruthy()
  })

  it("shows the values of the Rows table with the browser's thousands separators, and the X axis as it is", async () => {
    previewAnswer = drawRows
    renderBuilder(SAVED.bar)
    const preview = screen.getByRole("region", { name: "Preview" })
    await waitFor(() => expect(within(preview).getByTestId("chart")).toBeTruthy())
    fireEvent.click(screen.getByRole("button", { name: "Data" }))
    const rows = await within(preview).findByRole("table")
    const cells = [...rows.querySelectorAll("tbody tr:first-child td")].map((td) => td.textContent)
    expect(cells).toEqual(["2024-01-01", formatLocaleNumber(1571564)])
    expect(cells[1]).not.toBe("1571564")
  })
})

// BI-58 review round 1, SHOULD-FIX 4 and 5 (owner).
describe("leaving and the target dashboard (BI-58 review round 1)", () => {
  const boards = [{ id: "default", name: "Main" }, { id: "b_other", name: "Other board" }]
  function renderLeaving(initial: ChartDef) {
    const onCancel = mock(() => {})
    render(
      <ReportingProvider reporting={{ timeZone: "UTC", weekStart: "monday" }}>
        <ChartBuilder onSaved={() => {}} onCancel={onCancel} initial={initial} editId="c_1" boards={boards} />
      </ReportingProvider>,
    )
    return onCancel
  }
  /** A person's edit of the title: a press in the field, then the new text. */
  async function editTitle(text: string) {
    const title = await screen.findByLabelText("Title")
    fireEvent.pointerDown(title)
    fireEvent.change(title, { target: { value: text } })
  }

  it("leaves at once by the back arrow when nothing changed", async () => {
    const onCancel = renderLeaving(SAVED.bar)
    await waitFor(() => expect(previews.length).toBeGreaterThan(0))
    fireEvent.click(screen.getByRole("button", { name: "Back to dashboard" }))
    expect(onCancel).toHaveBeenCalledTimes(1)
    expect(screen.queryByRole("dialog", { name: "Discard changes?" }) === null).toBe(true)
  })

  it("asks before leaving with a change: Keep editing stays, Discard leaves", async () => {
    const onCancel = renderLeaving(SAVED.bar)
    await waitFor(() => expect(previews.length).toBeGreaterThan(0))
    await editTitle("Visitors, renamed")
    fireEvent.click(screen.getByRole("button", { name: "Back to dashboard" }))
    const confirm = await screen.findByRole("dialog", { name: "Discard changes?" })
    fireEvent.click(within(confirm).getByRole("button", { name: "Keep editing" }))
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "Discard changes?" }) === null).toBe(true))
    expect(onCancel).not.toHaveBeenCalled()
    expect((screen.getByLabelText("Title") as HTMLInputElement).value).toBe("Visitors, renamed")
    fireEvent.click(screen.getByRole("button", { name: "Back to dashboard" }))
    fireEvent.click(within(await screen.findByRole("dialog", { name: "Discard changes?" })).getByRole("button", { name: "Discard" }))
    expect(onCancel).toHaveBeenCalledTimes(1)
  })

  it("leaves by Esc when nothing is open over the page, asking first after a change", async () => {
    const onCancel = renderLeaving(SAVED.bar)
    await waitFor(() => expect(previews.length).toBeGreaterThan(0))
    await editTitle("Changed")
    fireEvent.keyDown(document.body, { key: "Escape" })
    expect(await screen.findByRole("dialog", { name: "Discard changes?" })).toBeTruthy()
    expect(onCancel).not.toHaveBeenCalled()
  })

  it("saves the chart on the dashboard picked in the breadcrumb", async () => {
    const save = spyOn(dashboardService, "saveChart").mockResolvedValue({ id: "c_1" })
    try {
      renderLeaving(SAVED.bar)
      await waitFor(() => expect(previews.length).toBeGreaterThan(0))
      const picker = screen.getByRole("combobox", { name: "Add to dashboard" })
      expect(picker.textContent).toContain("Main")
      fireEvent.click(picker)
      const option = await screen.findByRole("option", { name: "Other board" })
      fireEvent.focus(option)
      fireEvent.click(option)
      await waitFor(() => expect(picker.textContent).toContain("Other board"))
      fireEvent.click(screen.getByRole("button", { name: "Save changes" }))
      await waitFor(() => expect(save).toHaveBeenCalledTimes(1))
      expect(wire(save.mock.calls[0]![0]).board).toBe("b_other")
    } finally {
      save.mockRestore()
    }
  })
})
