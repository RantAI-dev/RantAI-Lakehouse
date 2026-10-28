import { cleanup, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"
import type { ChartRenderSpec } from "@/lib/dashboard-specs"
import { emptyTypeDraft, typeDraftFromDef, typePayload, typeProblems } from "@/lib/type-b"
import { drillTarget } from "@/lib/chart-click"
import { ganttBarItem, ganttDuration, ganttOption, ganttTime, type GanttDrawn } from "./gantt-option"
import { ReportingProvider } from "./reporting-context"
import type { Rows } from "./tile-dialogs"

// BI-16 part C1, T3 (plan D3): the option tests of the Gantt chart. The chart itself needs a canvas; the option it is handed does not.
const charts: { option: Record<string, unknown>; onDataClick?: (hit: unknown, pos: unknown) => void }[] = []
mock.module("./echart", () => ({
  EChart: (props: { option: Record<string, unknown>; onDataClick?: (hit: unknown, pos: unknown) => void }) => {
    charts.push(props)
    return <div data-testid="chart" />
  },
}))
const { TileBody } = await import("./tile-body")
afterEach(() => { cleanup(); charts.length = 0 })

const NOW = Date.UTC(2026, 2, 10, 12, 0) // 2026-03-10 12:00 UTC
const row = (task: string, group: string | null, start: string, end: string, progress?: number) => ({ task, group, start, end, ...(progress === undefined ? {} : { progress }) })
const drawn = (d: GanttDrawn) => {
  if ("message" in d) throw new Error(d.message)
  return d
}
type Item = { name: string; value: (number | string)[]; group: string }
const seriesOf = (d: ReturnType<typeof drawn>) => d.option.series as { type: string; name?: string; color?: string; data: Item[]; markLine?: { data: { xAxis: number }[] } }[]
const bars = (d: ReturnType<typeof drawn>) => seriesOf(d).filter((s) => s.type === "custom")

describe("ganttOption", () => {
  it("draws one bar per task from its start to its end, the tasks down the axis in start order", () => {
    const d = drawn(ganttOption({ rows: [row("Build", null, "2026-03-05", "2026-03-20"), row("Design", null, "2026-03-01", "2026-03-04")], dark: false, timeZone: "UTC", now: NOW }))
    expect((d.option.yAxis as { data: string[] }).data).toEqual(["Design", "Build"])
    expect(bars(d)).toHaveLength(1)
    expect(bars(d)[0].data.map((b) => b.value)).toEqual([
      [0, Date.UTC(2026, 2, 1), Date.UTC(2026, 2, 4), "-"],
      [1, Date.UTC(2026, 2, 5), Date.UTC(2026, 2, 20), "-"],
    ])
    expect((d.option.xAxis as { type: string }).type).toBe("time")
    expect(d.option.useUTC).toBe(true)
    expect(d.option.legend).toBeUndefined()
  })

  it("reads a date-time start and end to the minute", () => {
    expect(ganttTime("2026-03-01 08:30:15")).toBe(Date.UTC(2026, 2, 1, 8, 30, 15))
    expect(ganttTime("2026-03-01T08:30:00")).toBe(Date.UTC(2026, 2, 1, 8, 30))
    expect(ganttTime("not a date")).toBeNull()
    expect(ganttTime(null)).toBeNull()
    expect(ganttDuration(2.5 * 3_600_000)).toBe("2 h 30 min")
    expect(ganttDuration(5 * 86_400_000)).toBe("5 d")
  })

  it("colours each group with its own palette colour and lists the groups in a legend", () => {
    const d = drawn(ganttOption({
      rows: [row("A", "Team 1", "2026-03-01", "2026-03-02"), row("B", "Team 2", "2026-03-03", "2026-03-04"), row("C", "Team 1", "2026-03-05", "2026-03-06")],
      dark: false, timeZone: "UTC", now: NOW, colors: ["#111111", "#222222"],
    }))
    expect(d.groups).toEqual(["Team 1", "Team 2"])
    expect(bars(d).map((s) => [s.name, s.color, s.data.map((b) => b.name)])).toEqual([
      ["Team 1", "#111111", ["A", "C"]],
      ["Team 2", "#222222", ["B"]],
    ])
    expect((d.option.legend as { data: string[] }).data).toEqual(["Team 1", "Team 2"])
  })

  it("draws a line at today in the report zone, and none outside the range of the tasks", () => {
    const inside = drawn(ganttOption({ rows: [row("A", null, "2026-03-01", "2026-03-20")], dark: false, timeZone: "UTC", now: NOW }))
    const mark = seriesOf(inside).find((s) => s.markLine)
    expect(inside.todayDrawn).toBe(true)
    expect(mark?.markLine?.data[0].xAxis).toBe(Date.UTC(2026, 2, 10, 12, 0))
    // 2026-03-10 23:00 UTC is already the 11th at +02:00 (Africa/Johannesburg), and the line follows the zone, not the browser.
    const late = Date.UTC(2026, 2, 10, 23, 0)
    const zoned = drawn(ganttOption({ rows: [row("A", null, "2026-03-01", "2026-03-20")], dark: false, timeZone: "Africa/Johannesburg", now: late }))
    expect(seriesOf(zoned).find((s) => s.markLine)?.markLine?.data[0].xAxis).toBe(Date.UTC(2026, 2, 11, 1, 0))
    const outside = drawn(ganttOption({ rows: [row("A", null, "2025-01-01", "2025-02-01")], dark: false, timeZone: "UTC", now: NOW }))
    expect(outside.todayDrawn).toBe(false)
    expect(seriesOf(outside).some((s) => s.markLine)).toBe(false)
  })

  it("leaves the line at today out when the chart's todayLine is false", () => {
    const rows = [row("A", null, "2026-03-01", "2026-03-20")]
    expect(drawn(ganttOption({ rows, dark: false, timeZone: "UTC", now: NOW, todayLine: false })).todayDrawn).toBe(false)
    expect(drawn(ganttOption({ rows, dark: false, timeZone: "UTC", now: NOW, todayLine: true })).todayDrawn).toBe(true)
  })

  it("shades the bar by its progress and leaves a bar without progress solid", () => {
    const api = (progress: number | string) => ({
      value: (dim: number) => [0, 0, 100, progress as number][dim],
      coord: ([x, y]: [number, number]): [number, number] => [x, 50 + y * 20],
      size: (): [number, number] => [0, 20],
    })
    const params = { coordSys: { x: 0, y: 0, width: 400, height: 100 } }
    const shaded = ganttBarItem("#123456", params, api(40)) as { children: { shape: { width: number }; style: { opacity?: number } }[] }
    expect(shaded.children).toHaveLength(2)
    expect(shaded.children[0].style.opacity).toBe(0.35)
    expect(shaded.children[1].shape.width).toBeCloseTo(shaded.children[0].shape.width * 0.4)
    const solid = ganttBarItem("#123456", params, api("-")) as { children: { style: { opacity?: number } }[] }
    expect(solid.children).toHaveLength(1)
    expect(solid.children[0].style.opacity).toBe(1)
    // The data carries the clamped percentage.
    const d = drawn(ganttOption({ rows: [row("A", null, "2026-03-01", "2026-03-02", 140)], dark: false, timeZone: "UTC", now: NOW }))
    expect(bars(d)[0].data[0].value[3]).toBe(100)
  })

  it("keeps the first 1,000 tasks by start and says so when there are more", () => {
    const rows = Array.from({ length: 1001 }, (_, n) => row(`T${String(n).padStart(4, "0")}`, null, "2026-03-01", "2026-03-02"))
    // The latest start is the one cut, whatever the order the rows came in.
    rows[0] = row("Last", null, "2026-04-01", "2026-04-02")
    const d = drawn(ganttOption({ rows, dark: false, timeZone: "UTC", now: NOW }))
    expect(d.bars).toBe(1000)
    expect(d.notes).toContain("Showing the first 1,000 tasks by start")
    expect((d.option.yAxis as { data: string[] }).data).not.toContain("Last")
    const exact = drawn(ganttOption({ rows: rows.slice(0, 1000), dark: false, timeZone: "UTC", now: NOW }))
    expect(exact.notes).toEqual([])
  })

  it("counts the tasks that end before they start, from the server's count and from any row that got through", () => {
    const d = drawn(ganttOption({ rows: [row("A", null, "2026-03-01", "2026-03-02"), row("Bad", null, "2026-03-09", "2026-03-02")], dark: false, timeZone: "UTC", now: NOW, reversedRows: 2 }))
    expect(d.bars).toBe(1)
    expect(d.notes).toEqual(["3 tasks end before they start and are left out"])
    const one = drawn(ganttOption({ rows: [row("A", null, "2026-03-01", "2026-03-02")], dark: false, timeZone: "UTC", now: NOW, reversedRows: 1 }))
    expect(one.notes).toEqual(["1 task ends before it starts and is left out"])
  })

  it("says so when no row can be drawn, instead of an empty axis", () => {
    expect(ganttOption({ rows: [], dark: false, timeZone: "UTC", now: NOW })).toEqual({ message: "No data" })
    const m = ganttOption({ rows: [row("Bad", null, "2026-03-09", "2026-03-02")], dark: false, timeZone: "UTC", now: NOW })
    expect(m).toEqual({ message: "No task has a start and an end in that order" })
  })

  it("escapes task and group names in the tooltip", () => {
    const d = drawn(ganttOption({ rows: [row("<img src=x onerror=alert(1)>", "<b>Team</b>", "2026-03-01", "2026-03-02", 50)], dark: false, timeZone: "UTC", now: NOW }))
    const formatter = (d.option.tooltip as { formatter: (p: unknown) => string }).formatter
    const html = formatter({ data: bars(d)[0].data[0] })
    expect(html).not.toContain("<img")
    expect(html).not.toContain("<b>Team")
    expect(html).toContain("&lt;img")
    expect(html).toContain("2026-03-01 to 2026-03-02")
    expect(html).toContain("Duration: 1 d")
    expect(html).toContain("Progress: 50%")
  })
})

describe("a Gantt tile", () => {
  const spec = (def: unknown = {}) => ({ id: "g_1", title: "Plan", kind: "gantt", mart: "m", x: "task", y: "", series: "team", source: "ui", def } as unknown as ChartRenderSpec)
  const mount = (cell: Rows) => render(
    <ReportingProvider reporting={{ timeZone: "UTC", weekStart: "monday" }}>
      <TileBody spec={spec()} cell={cell} dark={false} loading={false} onDataClick={() => {}} />
    </ReportingProvider>,
  )

  it("draws the chart and prints the notes under it", () => {
    mount({ columns: ["task", "group", "start", "end"], rows: [row("A", null, "2026-03-01", "2026-03-02")], reversedRows: 3 })
    expect(screen.getByTestId("chart")).toBeTruthy()
    expect(screen.getByText("3 tasks end before they start and are left out.")).toBeTruthy()
  })

  it("says there is no data for an empty result", () => {
    mount({ columns: [], rows: [] })
    expect(screen.getByText("No data.")).toBeTruthy()
  })

  it("names the task of a clicked bar, so the drill lists that task's rows", () => {
    const clicks: unknown[] = []
    render(
      <ReportingProvider reporting={{ timeZone: "UTC", weekStart: "monday" }}>
        <TileBody spec={spec()} cell={{ columns: [], rows: [row("Build", null, "2026-03-01", "2026-03-02")] }} dark={false} loading={false} onDataClick={(hit, _pos, ctx) => clicks.push([hit, ctx])} />
      </ReportingProvider>,
    )
    charts[0].onDataClick?.({ name: "Build", seriesType: "custom" }, { x: 1, y: 2 })
    expect(clicks).toHaveLength(1)
    const [hit] = clicks[0] as [{ name: string }]
    expect(drillTarget({ kind: "gantt", dimension: "task", source: "ui", y: "" } as never, hit as never)).toEqual({ column: "task", value: "Build" })
  })
})

describe("the Gantt fields of a chart definition", () => {
  it("round-trips start, end and progress through the draft and the payload", () => {
    const draft = typeDraftFromDef({ start: "begins", end: "ends", progress: "done_pct" })
    expect(draft).toMatchObject({ ganttStart: "begins", ganttEnd: "ends", ganttProgress: "done_pct" })
    expect(typePayload("gantt", draft)).toEqual({ start: "begins", end: "ends", progress: "done_pct" })
    expect(typePayload("bar", draft)).toEqual({})
  })

  it("asks for a start and an end, and not for progress", () => {
    expect(typeProblems("gantt", emptyTypeDraft())).toEqual(["Start column", "End column"])
    expect(typeProblems("gantt", { ...emptyTypeDraft(), ganttStart: "a", ganttEnd: "b" })).toEqual([])
    expect(typePayload("gantt", { ...emptyTypeDraft(), ganttStart: "a", ganttEnd: "b" })).toEqual({ start: "a", end: "b" })
  })
})
