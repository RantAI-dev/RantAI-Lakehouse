import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"
import { ReferenceLinesEditor } from "./reference-lines-editor"

afterEach(cleanup)

const TARGET = { id: "p_1", name: "target", label: "Target", type: "number", default: 20000 }

describe("ReferenceLinesEditor", () => {
  it("adds a blank line, up to five", () => {
    const onChange = mock(() => {})
    render(<ReferenceLinesEditor value={[]} onChange={onChange} numberParams={[]} />)
    fireEvent.click(screen.getByRole("button", { name: /Add a line/ }))
    expect(onChange).toHaveBeenCalledWith([{ value: "", param: "", label: "" }])
    cleanup()
    const five = Array.from({ length: 5 }, () => ({ value: "1", param: "", label: "" }))
    render(<ReferenceLinesEditor value={five} onChange={onChange} numberParams={[]} />)
    expect(screen.queryByRole("button", { name: /Add a line/ })).toBeNull()
  })

  it("takes a plain number and a label without any parameter defined", () => {
    const onChange = mock(() => {})
    render(<ReferenceLinesEditor value={[{ value: "", param: "", label: "" }]} onChange={onChange} numberParams={[]} />)
    expect(screen.queryByLabelText("Line 1 follows")).toBeNull()
    fireEvent.change(screen.getByLabelText("Line 1 value"), { target: { value: "300" } })
    expect(onChange).toHaveBeenCalledWith([{ value: "300", param: "", label: "" }])
  })

  it("shows the parameter a line follows in place of its number, and removes a line", () => {
    const onChange = mock(() => {})
    render(<ReferenceLinesEditor value={[{ value: "", param: "target", label: "Goal" }]} onChange={onChange} numberParams={[TARGET]} />)
    expect(screen.getByLabelText("Line 1 follows").textContent).toContain("Target")
    expect(screen.queryByLabelText("Line 1 value")).toBeNull()
    fireEvent.click(screen.getByRole("button", { name: "Remove line 1" }))
    expect(onChange).toHaveBeenCalledWith([])
  })

  // BI-17 part B (plan D1)
  it("offers Forecast, starts it at 6 periods, and takes a horizon", () => {
    const onChange = mock(() => {})
    render(<ReferenceLinesEditor value={[{ value: "", param: "", label: "", kind: "forecast", horizon: "6" }]} onChange={onChange} numberParams={[]} />)
    expect(screen.queryByLabelText("Line 1 value")).toBeNull()
    fireEvent.change(screen.getByLabelText("Line 1 horizon"), { target: { value: "12" } })
    expect(onChange).toHaveBeenCalledWith([{ value: "", param: "", label: "", kind: "forecast", horizon: "12" }])
  })

  it("does not offer a second forecast, nor any forecast on a chart that cannot draw one", () => {
    const lines = [{ value: "", param: "", label: "", kind: "forecast" as const, horizon: "6" }, { value: "1", param: "", label: "" }]
    render(<ReferenceLinesEditor value={lines} onChange={() => {}} numberParams={[]} />)
    fireEvent.click(screen.getByLabelText("Line 2 kind"))
    expect(screen.queryAllByRole("option", { name: "Forecast" }).length).toBe(0)
    cleanup()
    render(<ReferenceLinesEditor value={[{ value: "1", param: "", label: "" }]} onChange={() => {}} numberParams={[]} takesForecast={false} />)
    fireEvent.click(screen.getByLabelText("Line 1 kind"))
    expect(screen.queryAllByRole("option", { name: "Forecast" }).length).toBe(0)
    expect(screen.getAllByRole("option").length).toBe(7)
  })

  it("offers Forecast to a line of a chart that can draw one", () => {
    render(<ReferenceLinesEditor value={[{ value: "1", param: "", label: "" }]} onChange={() => {}} numberParams={[]} />)
    fireEvent.click(screen.getByLabelText("Line 1 kind"))
    expect(screen.getAllByRole("option", { name: "Forecast" }).length).toBe(1)
  })
})
