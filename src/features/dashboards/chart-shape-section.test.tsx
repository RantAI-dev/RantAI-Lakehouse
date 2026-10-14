import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"
import { ChartFilters, GroupingsEditor, MeasureRows, PreviewRowsTable, StepsEditor } from "./chart-shape-section"
import { OwnFiltersMarker } from "./filters/own-filters-marker"
import { TileBody } from "./tile-body"
import { recordsQuery } from "./records"
import type { ChartRenderSpec } from "@/lib/dashboard-specs"
import type { MeasureRow, StepDef } from "@/lib/chart-shape"

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

  it("shows each measure's own aggregation, defaulting to the chart's, and stores none when the chart's is picked again", () => {
    const onChange = mock<(rows: MeasureRow[]) => void>(() => {})
    render(<MeasureRows {...baseProps} defaultAggregate="avg" rows={rows3} onChange={onChange} />)
    // Phase 2 QA R4: no "Default (avg)" entry; the row reads the aggregate it is computed with.
    expect(screen.getByLabelText("Aggregation of visitors").textContent).toContain("avg")
    expect(screen.getByLabelText("Aggregation of stay").textContent).toContain("avg")
    expect(screen.queryByText(/Default \(/)).toBeNull()
  })

  it("has one aggregation per measure for the kinds that take several, and one for the chart for the rest", () => {
    const onDefault = mock<(a: string) => void>(() => {})
    const { rerender } = render(<MeasureRows {...baseProps} own={false} max={1} rows={[rows3[0]]} onChange={() => {}} />)
    // A kind without aggregates of its own and no chart-wide control: none shown.
    expect(screen.queryByLabelText("Aggregation of visitors")).toBeNull()
    rerender(<MeasureRows {...baseProps} own={false} max={1} onDefaultAggregate={onDefault} rows={[rows3[0]]} onChange={() => {}} />)
    expect(screen.getByLabelText("Aggregation of visitors")).toBeTruthy()
    rerender(<MeasureRows {...baseProps} own={false} max={1} fixedNote="min, quartiles, max" rows={[rows3[0]]} onChange={() => {}} />)
    expect(screen.queryByLabelText("Aggregation of visitors")).toBeNull()
    expect(screen.getByText("min, quartiles, max")).toBeTruthy()
  })

  it("shows no aggregation for a metric or a calculated aggregate, which carry their own", () => {
    render(<MeasureRows {...baseProps} wholeMeasure={new Set(["visitors"])} rows={[rows3[0]]} onChange={() => {}} />)
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

describe("ChartFilters with segments (BI-2)", () => {
  const columns = [{ name: "category", kind: "text" as const }]
  const segments = [{ name: "domestic", label: "Domestic visitors" }, { name: "foreign", label: "Foreign visitors" }]
  it("adds a segment as a reference by name and lists it as a chip that can be removed", () => {
    const onChange = mock<(filters: unknown[]) => void>(() => {})
    const { rerender } = render(<ChartFilters value={[]} onChange={onChange} columns={columns} disabled={false} segments={segments} />)
    expect(screen.getByLabelText("Add a segment")).toBeTruthy()
    rerender(<ChartFilters value={[{ column: "", values: [], segment: "domestic" }]} onChange={onChange} columns={columns} disabled={false} segments={segments} />)
    expect(screen.getByText("Segment: Domestic visitors")).toBeTruthy()
    fireEvent.click(screen.getByLabelText("Remove segment domestic"))
    expect(onChange.mock.calls[0][0]).toEqual([])
  })
  it("offers no segment picker when the table has none", () => {
    render(<ChartFilters value={[]} onChange={() => {}} columns={columns} disabled={false} />)
    expect(screen.queryByLabelText("Add a segment")).toBeNull()
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

describe("StepsEditor", () => {
  const base = {
    groups: [{ name: "kab", kind: "text" }, { name: "d", kind: "date" }],
    measures: ["visitors"], labels: {},
  }
  const noop = () => {}

  it("starts a filter step on the first measure and offers at most two steps", () => {
    const onChange = mock<(steps: StepDef[]) => void>(() => {})
    const { rerender } = render(<StepsEditor value={[]} onChange={onChange} base={base} disabled={false} />)
    fireEvent.click(screen.getByText("Add a step"))
    expect(onChange.mock.calls[0][0]).toEqual([{ filter: [{ column: "visitors", op: "gt", value: "" }] }])
    const two: StepDef[] = [
      { filter: [{ column: "visitors", op: "gt", value: "1" }] },
      { filter: [{ column: "visitors", op: "lt", value: "9" }] },
    ]
    rerender(<StepsEditor value={two} onChange={onChange} base={base} disabled={false} />)
    expect(screen.queryByText("Add a step")).toBeNull()
  })

  it("types a value into a condition and removes a step", () => {
    const onChange = mock<(steps: StepDef[]) => void>(() => {})
    render(<StepsEditor value={[{ filter: [{ column: "visitors", op: "gt", value: "" }] }]} onChange={onChange} base={base} disabled={false} />)
    fireEvent.change(screen.getByLabelText("Value"), { target: { value: "1000" } })
    expect(onChange.mock.calls[0][0]).toEqual([{ filter: [{ column: "visitors", op: "gt", value: "1000" }] }])
    fireEvent.click(screen.getByLabelText("Remove step 1"))
    expect(onChange.mock.calls[1][0]).toEqual([])
  })

  it("adds a second condition and a range shows two bounds", () => {
    const onChange = mock<(steps: StepDef[]) => void>(() => {})
    render(<StepsEditor value={[{ filter: [{ column: "visitors", op: "between" }] }]} onChange={onChange} base={base} disabled={false} />)
    expect(screen.getByLabelText("From")).toBeTruthy()
    expect(screen.getByLabelText("To")).toBeTruthy()
    fireEvent.click(screen.getByText("Add condition"))
    expect(onChange.mock.calls[0][0][0].filter).toHaveLength(2)
  })

  it("offers the second step the columns the first one left", () => {
    render(
      <StepsEditor
        value={[
          { summarise: { by: ["kab"], measures: [{ column: "visitors", aggregate: "sum" }] } },
          { filter: [{ column: "sum_visitors", op: "gt", value: "5" }] },
        ]}
        onChange={noop} base={base} disabled={false}
      />,
    )
    // Step 2 reads the summary: its condition column is the summarised measure.
    expect(screen.getAllByLabelText("Condition column").length).toBe(1)
    // (The names it offers are pinned in chart-shape.test.ts, `columnsAfterSteps`.)
    expect(screen.getByLabelText("Step 2")).toBeTruthy()
  })
})

describe("StepsEditor wording (phase 2 QA R8)", () => {
  const noop = () => {}
  it("reads an earlier step's columns as plain labels, not stacked names", () => {
    const base = { groups: [{ name: "kab", kind: "text" }], measures: ["avg_visitors"], labels: { avg_visitors: "avg of visitors" }, bases: { avg_visitors: "visitors" } }
    render(
      <StepsEditor
        value={[
          { summarise: { by: ["kab"], measures: [{ column: "avg_visitors", aggregate: "sum" }] } },
          { filter: [{ column: "sum_avg_visitors", op: "gt", value: "5" }] },
        ]}
        onChange={noop} base={base} disabled={false}
      />,
    )
    expect(screen.getByLabelText("Condition column").textContent).toContain("total visitors")
    expect(screen.queryByText(/sum of avg of/)).toBeNull()
  })

  it("says once that there is nothing to keep when the chart has no groupings, and offers no list", () => {
    const base = { groups: [], measures: ["visitors"], labels: {} }
    render(<StepsEditor value={[{ summarise: { by: [], measures: [{ column: "visitors", aggregate: "sum" }] } }]} onChange={noop} base={base} disabled={false} />)
    expect(screen.getByText("There is no grouping to keep: group the chart by a column first.")).toBeTruthy()
    expect(screen.queryByText("Keep")).toBeNull()
  })
})

describe("PreviewRowsTable and the internal columns", () => {
  it("lists the rows of the last result without internal columns and says when it was cut", () => {
    render(<PreviewRowsTable open data={{ columns: ["kab", "v", "__other"], rows: [{ kab: "Bali", v: 3, __other: 0 }], truncated: true, limit: 2000 }} />)
    expect(screen.getByText("Rows (1 of more than 2,000)")).toBeTruthy()
    expect(screen.getByText("Only the first 2,000 rows are shown.")).toBeTruthy()
    expect(screen.queryByText("__other")).toBeNull()
    expect(screen.getByText("Bali")).toBeTruthy()
  })

  it("keeps the internal columns out of View data's file", async () => {
    const saved: string[] = []
    const { downloadRowsCsv } = await import("./tile-dialogs")
    const original = URL.createObjectURL
    URL.createObjectURL = ((blob: Blob) => { void blob.text().then((t) => saved.push(t)); return "blob:x" }) as typeof URL.createObjectURL
    try {
      downloadRowsCsv("t", { columns: ["kab", "v", "__other"], rows: [{ kab: "Bali", v: 3, __other: 1 }] })
      await new Promise((r) => setTimeout(r, 20))
    } finally {
      URL.createObjectURL = original
    }
    expect(saved.join("")).not.toContain("__other")
    expect(saved.join("")).toContain("kab,v")
  })
})
