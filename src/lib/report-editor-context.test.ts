import { describe, expect, it } from "bun:test"

import { PAGE_CONTEXT_MAX_CHARS } from "./page-context-summary"
import { DESCRIBE_MAX_CHARS, buildReportEditorContext, describeRequest, describedRows, writeRequest } from "./report-editor-context"

// BI-62 PR B (T8, D8): the report editor's page context for the assistant.

const label = (id: string) => (id === "c_1" ? { title: "Visits by month", dashboard: "Sales" } : undefined)

describe("buildReportEditorContext", () => {
  it("names the report, says how drafts are answered, and holds the report as Markdown", () => {
    const text = buildReportEditorContext({
      id: "r1",
      title: "Q3 visitors",
      blocks: [
        { type: "heading", text: "Summary", level: 1 },
        { type: "paragraph", text: "Visits **rose**." },
        { type: "chart", chart: "c_1" },
      ],
      chartLabel: label,
    })
    expect(text).toContain('The user is writing the report "Q3 visitors" in the report editor (report id r1, 1 of 30 charts).')
    expect(text).toContain("do not change it with update_report")
    expect(text).toContain('A message that starts "Write for this report:"')
    expect(text.endsWith("The report so far, as Markdown:\n# Summary\n\nVisits **rose**.\n\n[chart: Visits by month · Sales]")).toBe(true)
    expect(text).not.toContain("The chart to describe")
  })

  it("says a new report is not saved yet, and an empty one is empty", () => {
    const text = buildReportEditorContext({ id: null, title: "", blocks: [], chartLabel: label })
    expect(text).toContain('"Untitled report" in the report editor (not saved yet, 0 of 30 charts)')
    expect(text.endsWith("as Markdown:\n(empty)")).toBe(true)
  })

  it("gives the described chart's rows as the reader sees them", () => {
    const text = buildReportEditorContext({
      id: "r1",
      title: "T",
      blocks: [{ type: "chart", chart: "c_1" }],
      chartLabel: label,
      chart: {
        id: "c_1",
        title: "Visits by month",
        dashboard: "Sales",
        rows: { kind: "rows", columns: ["month", "visits"], rows: [{ month: "Jan", visits: 10 }, { month: "Feb", visits: 12 }] },
      },
    })
    expect(text).toContain('The chart to describe: "Visits by month" (chart id c_1) from the dashboard "Sales". Its rows as the reader sees them: rows: Jan=10, Feb=12.')
  })

  it("says so when the rows are not loaded, or cannot be read, and invents none", () => {
    const base = { id: "r1", title: "T", blocks: [], chartLabel: label }
    const waiting = buildReportEditorContext({ ...base, chart: { id: "c_1", title: "V", dashboard: "", rows: { kind: "not loaded" } } })
    expect(waiting).toContain('The chart to describe: "V" (chart id c_1). Its rows are not loaded on the page yet (rows not loaded)')
    const hidden = buildReportEditorContext({ ...base, chart: { id: "c_1", title: "V", dashboard: "", rows: { kind: "not readable" } } })
    expect(hidden).toContain("Its rows could not be read for this person: give no numbers for it.")
  })

  it("fits the server's cap: a long report is cut at its end, the instructions and the chart never", () => {
    const blocks = Array.from({ length: 300 }, (_, i) => ({ type: "paragraph" as const, text: `Paragraph ${i} `.repeat(20) }))
    const rows = Array.from({ length: 500 }, (_, i) => ({ k: `key ${i} ${"x".repeat(30)}`, v: i }))
    const text = buildReportEditorContext({
      id: "r1",
      title: "Long",
      blocks,
      chartLabel: label,
      chart: { id: "c_1", title: "Big", dashboard: "Sales", rows: { kind: "rows", columns: ["k", "v"], rows } },
    })
    expect(text.length).toBeLessThanOrEqual(PAGE_CONTEXT_MAX_CHARS)
    expect(text).toContain("do not change it with update_report")
    expect(text).toContain('The chart to describe: "Big" (chart id c_1)')
    expect(text.endsWith("… (the rest of the report is not shown)")).toBe(true)
    const chartPart = text.slice(text.indexOf("The chart to describe"), text.indexOf(" The report so far"))
    expect(chartPart.length).toBeLessThanOrEqual(DESCRIBE_MAX_CHARS)
  })
})

describe("the requests a draft sends", () => {
  it("reads as the person would say it, in one line", () => {
    expect(writeRequest("  a short\nintro ")).toBe("Write for this report: a short intro")
    expect(describeRequest("Visits\nby month")).toBe("Describe the chart “Visits by month” for this report")
    expect(describeRequest("")).toBe("Describe the chart “Untitled chart” for this report")
  })
})

describe("describedRows", () => {
  const spec = { id: "c_1", title: "V", kind: "bar", source: "ui" } as never
  it("reads the rows the page drew the chart with, and never an error's text", () => {
    expect(describedRows(undefined)).toEqual({ kind: "not loaded" })
    expect(describedRows({ unavailable: "This chart is not available." })).toEqual({ kind: "not readable" })
    expect(describedRows({ spec, cell: null, board: "b", boardName: "B" })).toEqual({ kind: "not loaded" })
    expect(describedRows({ spec, cell: { error: "secret upstream text" }, board: "b", boardName: "B" })).toEqual({ kind: "not readable" })
    expect(describedRows({ spec, cell: { columns: ["a"], rows: [{ a: 1 }] }, board: "b", boardName: "B" })).toEqual({
      kind: "rows",
      columns: ["a"],
      rows: [{ a: 1 }],
    })
  })
})
