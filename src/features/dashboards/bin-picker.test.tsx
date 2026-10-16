import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"
import { BinPicker, GridPicker } from "./bin-picker"
import type { BinDraft } from "@/lib/chart-bins"

afterEach(cleanup)

describe("BinPicker", () => {
  it("shows the width box only for a width of the person's own, and keeps what was typed", () => {
    const onChange = mock<(next: BinDraft | null) => void>(() => {})
    const { rerender } = render(<BinPicker value={{ mode: "auto", count: "10", width: "" }} onChange={onChange} sqlSource={false} aria="Group in bins" />)
    expect(screen.queryByLabelText("Group in bins: width")).toBeNull()
    rerender(<BinPicker value={{ mode: "width", count: "10", width: "" }} onChange={onChange} sqlSource={false} aria="Group in bins" />)
    fireEvent.change(screen.getByLabelText("Group in bins: width"), { target: { value: "500" } })
    expect(onChange.mock.calls[0][0]).toEqual({ mode: "width", count: "10", width: "500" })
  })

  it("is a plain control with the picked choice as its value", () => {
    render(<BinPicker value={{ mode: "count", count: "50", width: "" }} onChange={() => {}} sqlSource={false} aria="Group in bins" />)
    expect(screen.getByLabelText("Group in bins").textContent).toContain("50 bins")
  })
})

describe("GridPicker", () => {
  it("shows the chosen size", () => {
    render(<GridPicker value="0.1" onChange={() => {}} />)
    expect(screen.getByLabelText("Marks").textContent).toContain("0.1")
  })
})
