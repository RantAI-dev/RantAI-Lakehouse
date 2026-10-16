import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"
import { ParamControls } from "./param-controls"
import type { ParamControl } from "@/lib/params"

afterEach(cleanup)

const number: ParamControl = {
  param: { id: "p_1", name: "target", label: "Target", type: "number", default: 1000, min: 0, max: 5000 },
  value: 1000,
}
const pick: ParamControl = {
  param: { id: "p_2", name: "pick", label: "Measure", type: "list", default: "visitors", values: [{ value: "visitors", label: "Visitors" }, { value: "rooms", label: "Rooms" }] },
  value: "rooms",
}

describe("ParamControls", () => {
  it("draws one control per parameter, showing the value in force and the range", () => {
    render(<ParamControls controls={[number, pick]} onChange={mock(() => {})} />)
    const input = screen.getByLabelText("Target") as HTMLInputElement
    expect(input.value).toBe("1000")
    expect(input.getAttribute("min")).toBe("0")
    expect(input.getAttribute("max")).toBe("5000")
    expect(screen.getByRole("combobox", { name: "Measure" }).textContent).toContain("Rooms")
  })

  it("reports a number once the viewer leaves the field, as a number", () => {
    const onChange = mock(() => {})
    render(<ParamControls controls={[number]} onChange={onChange} />)
    const input = screen.getByLabelText("Target") as HTMLInputElement
    fireEvent.change(input, { target: { value: "500" } })
    expect(onChange).not.toHaveBeenCalled()
    fireEvent.blur(input)
    expect(onChange).toHaveBeenCalledWith("target", 500)
  })

  it("puts an emptied or unchanged field back instead of reporting it", () => {
    const onChange = mock(() => {})
    render(<ParamControls controls={[number]} onChange={onChange} />)
    const input = screen.getByLabelText("Target") as HTMLInputElement
    fireEvent.change(input, { target: { value: "" } })
    fireEvent.blur(input)
    expect(onChange).not.toHaveBeenCalled()
    expect(input.value).toBe("1000")
  })

  it("follows a value the server gave back", () => {
    const { rerender } = render(<ParamControls controls={[number]} onChange={mock(() => {})} />)
    rerender(<ParamControls controls={[{ ...number, value: 250 }]} onChange={mock(() => {})} />)
    expect((screen.getByLabelText("Target") as HTMLInputElement).value).toBe("250")
  })
})
