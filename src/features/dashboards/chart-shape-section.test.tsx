import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"
import { ChartFilters, GroupingsEditor, MeasureRows } from "./chart-shape-section"
import { OwnFiltersMarker } from "./filters/own-filters-marker"
import { TileBody } from "./tile-body"
import { recordsQuery } from "./records"
import type { ChartRenderSpec } from "@/lib/dashboard-specs"
import type { MeasureRow } from "@/lib/chart-shape"

afterEach(cleanup)

const noLabel = (name: string) => name
const rows3: MeasureRow[] = [
  { column: "visitors", aggregate: "" },
  { column: "stay", aggregate: "avg" },
  { column: "orders", aggregate: "" },
]

describe("MeasureRows", () => {
  const baseProps = {
    labels: (i: number) => `Measure ${i + 1}`, min: 1, max: 4, own: true, columns: ["visitors", "stay", "orders"],
    defaultAggregate: "sum", disabled: false, placeholder: "pick", wholeMeasure: new Set<string>(), fieldLabel: noLabel,
  }

  it("moves a measure down and up, and removes one, keeping the aggregates with their columns", () => {
    const onChange = mock<(rows: MeasureRow[]) => void>(() => {})
    render(<MeasureRows {...baseProps} rows={rows3} onChange={onChange} />)
    fireEvent.click(screen.getByLabelText("Move measure 1 down"))
    expect(onChange.mock.calls[0][0].map((r) => r.column)).toEqual(["stay", "visitors", "orders"])
    expect(onChange.mock.calls[0][0][0]).toEqual({ column: "stay", aggregate: "avg" })
    fireEvent.click(screen.getByLabelText("Move measure 3 up"))
    expect(onChange.mock.calls[1][0].map((r) => r.column)).toEqual(["visitors", "orders", "stay"])
    fireEvent.click(screen.getByLabelText("Remove measure 2"))
    expect(onChange.mock.calls[2][0].map((r) => r.column)).toEqual(["visitors", "orders"])
  })

  it("adds an empty measure up to the maximum and removes none below the minimum", () => {
    const onChange = mock<(rows: MeasureRow[]) => void>(() => {})
    const { rerender } = render(<MeasureRows {...baseProps} rows={rows3} onChange={onChange} />)
    fireEvent.click(screen.getByText("Add measure"))
    expect(onChange.mock.calls[0][0]).toHaveLength(4)
    expect(onChange.mock.calls[0][0][3]).toEqual({ column: "", aggregate: "" })
    rerender(<MeasureRows {...baseProps} rows={[...rows3, { column: "x", aggregate: "" }]} onChange={onChange} />)
    expect(screen.queryByText("Add measure")).toBeNull()
    rerender(<MeasureRows {...baseProps} rows={[rows3[0]]} onChange={onChange} />)
    expect((screen.getByLabelText("Remove measure 1") as HTMLButtonElement).disabled).toBe(true)
  })

  it("shows no add, move or remove for a fixed number of measures", () => {
    render(<MeasureRows {...baseProps} min={2} max={2} rows={rows3.slice(0, 2)} onChange={() => {}} />)
    expect(screen.queryByText("Add measure")).toBeNull()
    expect(screen.queryByLabelText("Remove measure 1")).toBeNull()
  })

  it("offers an aggregate of its own only to a kind that takes several measures", () => {
    const { rerender } = render(<MeasureRows {...baseProps} rows={rows3} onChange={() => {}} />)
    expect(screen.getByLabelText("Aggregation of visitors")).toBeTruthy()
    rerender(<MeasureRows {...baseProps} max={1} rows={[rows3[0]]} onChange={() => {}} />)
    expect(screen.queryByLabelText("Aggregation of visitors")).toBeNull()
  })
})

describe("GroupingsEditor", () => {
  const columns = [
    { name: "kab", kind: "text" as const }, { name: "cat", kind: "text" as const }, { name: "d", kind: "date" as const },
  ]
  it("lists the groupings in order and moves, removes and adds them", () => {
    const onChange = mock<(groupings: { column: string; grain?: string }[]) => void>(() => {})
    render(<GroupingsEditor value={[{ column: "kab" }, { column: "d", grain: "month" }]} onChange={onChange} columns={columns} disabled={false} placeholder="pick" fieldLabel={noLabel} />)
    fireEvent.click(screen.getByLabelText("Move kab down"))
    expect(onChange.mock.calls[0][0]).toEqual([{ column: "d", grain: "month" }, { column: "kab" }])
    fireEvent.click(screen.getByLabelText("Remove d"))
    expect(onChange.mock.calls[1][0]).toEqual([{ column: "kab" }])
  })

  it("offers a grain only for a date column", () => {
    render(<GroupingsEditor value={[{ column: "kab" }, { column: "d" }]} onChange={() => {}} columns={columns} disabled={false} placeholder="pick" fieldLabel={noLabel} />)
    expect(screen.queryByLabelText("Group kab by")).toBeNull()
    expect(screen.getByLabelText("Group d by")).toBeTruthy()
  })
})

describe("ChartFilters", () => {
  const columns = [{ name: "category", kind: "text" as const }, { name: "visitors", kind: "number" as const }]
  it("lists each filter like a dashboard chip and removes one", () => {
    const onChange = mock<(filters: unknown[]) => void>(() => {})
    render(<ChartFilters value={[{ column: "category", values: ["Office"] }]} onChange={onChange} columns={columns} disabled={false} />)
    expect(screen.getByText("category is Office")).toBeTruthy()
    fireEvent.click(screen.getByLabelText("Remove category filter"))
    expect(onChange.mock.calls[0][0]).toEqual([])
  })
})

describe("OwnFiltersMarker", () => {
  it("says on the tile that the chart is filtered and how", () => {
    render(<OwnFiltersMarker filters={[{ column: "category", values: ["Office", "Retail"] }]} />)
    expect(screen.getByLabelText("This chart is filtered: category is Office, Retail")).toBeTruthy()
  })
  it("draws nothing for a chart without filters of its own", () => {
    const { container } = render(<OwnFiltersMarker filters={[]} />)
    expect(container.innerHTML).toBe("")
  })
})

describe("a records list for a tile with filters of its own", () => {
  it("names the tile so the server adds its filters", () => {
    const q = recordsQuery({ title: "t", mart: "mart_x", column: "kab", value: "Bali", filters: [], chart: "u_1" }, 0)
    expect(q.get("chart")).toBe("u_1")
    expect(recordsQuery({ title: "t", mart: "mart_x", filters: [] }, 0).has("chart")).toBe(false)
  })
})

describe("TileBody with the BI-6 fields", () => {
  const spec = (extra: Record<string, unknown> = {}) => ({
    id: "u_1", title: "t", kind: "table", mart: "m", x: "kab", y: ["visitors", "avg_stay"], source: "ui", ...extra,
  }) as unknown as ChartRenderSpec & { def?: unknown }
  const cell = {
    columns: ["kab", "visitors", "avg_stay", "__other"],
    rows: [
      { kab: "Bali", visitors: 10, avg_stay: 2, __other: 0 },
      { kab: "Other", visitors: 90, avg_stay: 3, __other: 1 },
    ],
  }

  it("shows a grouped table's columns by their labels and leaves the Other flag out", () => {
    render(<TileBody spec={{ ...spec(), def: { measures: ["visitors", { column: "stay", aggregate: "avg" }] } } as never} cell={cell} dark={false} loading={false} />)
    expect(screen.getByText("avg of stay")).toBeTruthy()
    expect(screen.queryByText("__other")).toBeNull()
    expect(screen.getByText("Other")).toBeTruthy()
  })

  it("keeps a column setting's own label over the generated one", () => {
    render(<TileBody spec={{ ...spec(), def: { measures: ["visitors", { column: "stay", aggregate: "avg" }], columnSettings: { avg_stay: { label: "Stay" } } } } as never} cell={cell} dark={false} loading={false} />)
    expect(screen.getByText("Stay")).toBeTruthy()
    expect(screen.queryByText("avg of stay")).toBeNull()
  })
})
