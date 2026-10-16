import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"
import { LayersSection } from "./layers-section"
import { emptyLayer, layerDraftFromDef, type LayerDraft } from "@/lib/chart-layers"

const originalFetch = global.fetch
afterEach(() => {
  global.fetch = originalFetch
  cleanup()
})

/** The fields route, for `mart_rooms`: a month, a timestamp, a channel and two numbers. */
function stubFields(columns = [{ name: "month", type: "Date" }, { name: "at", type: "DateTime" }, { name: "channel", type: "String" }, { name: "rooms", type: "UInt32" }, { name: "rate", type: "Float64" }]) {
  const urls: string[] = []
  global.fetch = mock(async (input: RequestInfo | URL) => {
    urls.push(String(input))
    return new Response(JSON.stringify({
      columns, dimensions: ["month", "at", "channel"], measures: ["rooms", "rate"],
    }), { status: 200, headers: { "Content-Type": "application/json" } })
  }) as unknown as typeof fetch
  return urls
}

const props = {
  baseKind: "date" as const, grained: false, marts: ["mart_visits", "mart_rooms"], sqlSources: [{ id: "s_1", title: "Joined" }],
  ownDraw: "", ownAxis: "", onOwnChange: () => {}, disabled: false,
}
const saved: LayerDraft = layerDraftFromDef({ mart: "mart_rooms", dimension: "month", measures: ["rooms"], draw: "line", axis: "right" })

describe("LayersSection", () => {
  it("adds a blank question (a line on the right axis), up to four, and removes one", () => {
    const onChange = mock<(next: LayerDraft[]) => void>(() => {})
    const { rerender } = render(<LayersSection {...props} layers={[]} onChange={onChange} />)
    fireEvent.click(screen.getByText("Add a question"))
    expect(onChange.mock.calls[0][0]).toEqual([emptyLayer()])
    rerender(<LayersSection {...props} layers={[emptyLayer(), emptyLayer(), emptyLayer(), emptyLayer()]} onChange={onChange} />)
    expect(screen.queryByText("Add a question")).toBeNull()
    fireEvent.click(screen.getByLabelText("Remove question 2"))
    expect(onChange.mock.calls[1][0]).toHaveLength(3)
  })

  it("offers no source columns until a source is picked, and no way to add one while the chart forbids it", () => {
    stubFields()
    render(<LayersSection {...props} layers={[emptyLayer()]} onChange={() => {}} blocked="Remove a breakdown to combine questions." />)
    expect(screen.queryByLabelText("Question 1 dimension")).toBeNull()
    expect(screen.getByRole("status").textContent).toMatch(/Remove a breakdown/)
    expect((screen.getByText("Add a question").closest("button") as HTMLButtonElement).disabled).toBe(true)
  })

  it("loads the picked source's columns once and lists only the dimensions that line up with the chart's", async () => {
    const urls = stubFields()
    render(<LayersSection {...props} layers={[saved]} onChange={() => {}} />)
    await waitFor(() => expect(urls).toHaveLength(1))
    expect(urls[0]).toBe("/api/dashboard/fields?mart=mart_rooms")
    // The chart's dimension is a date and ungrained: a timestamp does not line up, and text never does.
    await waitFor(() => expect(screen.getByLabelText("Question 1 dimension").textContent).toContain("month"))
    fireEvent.click(screen.getByLabelText("Question 1 dimension"))
    const offered = (await screen.findAllByRole("option")).map((o) => o.textContent)
    expect(offered).toEqual(["month"])
  })

  it("lists a timestamp too once the chart is cut by a time grain", async () => {
    stubFields()
    render(<LayersSection {...props} grained layers={[saved]} onChange={() => {}} />)
    await waitFor(() => expect(screen.getByLabelText("Question 1 dimension").textContent).toContain("month"))
    fireEvent.click(screen.getByLabelText("Question 1 dimension"))
    const offered = (await screen.findAllByRole("option")).map((o) => o.textContent)
    expect(offered).toEqual(["month", "at"])
  })

  it("says so when the source has no column that lines up", async () => {
    stubFields([{ name: "month", type: "Date" }, { name: "channel", type: "String" }])
    render(<LayersSection {...props} baseKind="number" layers={[saved]} onChange={() => {}} />)
    expect(await screen.findByText(/no number column to line up/)).toBeTruthy()
  })

  it("shows how each question is drawn and on which axis, and the chart's own drawing", async () => {
    stubFields()
    render(<LayersSection {...props} ownDraw="bar" ownAxis="left" layers={[saved]} onChange={() => {}} />)
    await waitFor(() => expect(screen.getByLabelText("Question 1 dimension").textContent).toContain("month"))
    expect(screen.getByLabelText("Question 1 drawn as").textContent).toContain("Line")
    expect(screen.getByLabelText("Question 1 axis").textContent).toContain("Right axis")
    expect(screen.getByLabelText("This chart's own series drawn as").textContent).toContain("Bars")
    expect(screen.getByLabelText("This chart's own series axis").textContent).toContain("Left axis")
  })

  it("moves a question up and down and keeps the order it is given", () => {
    stubFields()
    const onChange = mock<(next: LayerDraft[]) => void>(() => {})
    const other = { ...saved, label: "Second" }
    render(<LayersSection {...props} layers={[saved, other]} onChange={onChange} />)
    fireEvent.click(screen.getByLabelText("Move question 2 up"))
    expect(onChange.mock.calls[0][0].map((l) => l.label)).toEqual(["Second", ""])
    expect((screen.getByLabelText("Move question 1 up") as HTMLButtonElement).disabled).toBe(true)
  })

  it("drops a dimension that no longer lines up with the chart's, once the columns are known", async () => {
    stubFields()
    const onChange = mock<(next: LayerDraft[]) => void>(() => {})
    // The question was lined up on a timestamp; the chart is a date without a grain now.
    render(<LayersSection {...props} layers={[{ ...saved, dimension: "at" }]} onChange={onChange} />)
    await waitFor(() => expect(onChange).toHaveBeenCalled())
    expect(onChange.mock.calls[0][0][0].dimension).toBe("")
  })

  it("keeps a dimension that still lines up", async () => {
    stubFields()
    const onChange = mock<(next: LayerDraft[]) => void>(() => {})
    render(<LayersSection {...props} layers={[saved]} onChange={onChange} />)
    await waitFor(() => expect(screen.getByLabelText("Question 1 dimension").textContent).toContain("month"))
    expect(onChange).not.toHaveBeenCalled()
  })
})
