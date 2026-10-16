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
})
