import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"
import type { CondRule, StyleFields, ValueFormat } from "@/lib/chart-style"
import { formatRules } from "@/lib/style-draft"
import { ConditionalEditor, FormatSection, ValueFormatEditor } from "./format-section"

afterEach(cleanup)

describe("ConditionalEditor", () => {
  it("adds a rule on the only column there is, takes its number, and removes it", () => {
    const onChange = mock<(r: CondRule[]) => void>(() => {})
    const { rerender } = render(<ConditionalEditor value={[]} onChange={onChange} columns={["visitors"]} />)
    fireEvent.click(screen.getByRole("button", { name: /Add a rule/ }))
    const added = onChange.mock.calls[0]![0]
    expect(added).toEqual([{ column: "visitors", rule: { type: "compare", op: "lt", value: 0 }, color: "#dc2626" }])
    rerender(<ConditionalEditor value={added} onChange={onChange} columns={["visitors"]} />)
    fireEvent.change(screen.getByLabelText("Rule 1 value"), { target: { value: "1000" } })
    expect(onChange.mock.calls[1]![0]).toEqual([{ column: "visitors", rule: { type: "compare", op: "lt", value: 1000 }, color: "#dc2626" }])
    fireEvent.click(screen.getByRole("button", { name: "Remove rule 1" }))
    expect(onChange.mock.calls[2]![0]).toEqual([])
  })

  it("stops at twenty rules", () => {
    const rule: CondRule = { column: "v", rule: { type: "contains", text: "x" }, color: "#000000" }
    render(<ConditionalEditor value={Array.from({ length: 20 }, () => rule)} onChange={() => {}} columns={["v"]} />)
    expect(screen.queryByRole("button", { name: /Add a rule/ })).toBeNull()
  })
})

describe("ValueFormatEditor", () => {
  it("shows the currency controls only for the currency and compact styles", () => {
    const onChange = mock<(f: ValueFormat) => void>(() => {})
    const { rerender } = render(<ValueFormatEditor value={{ style: "number" }} onChange={onChange} label="Format" />)
    expect(screen.queryByLabelText("Format currency code")).toBeNull()
    rerender(<ValueFormatEditor value={{ style: "compact" }} onChange={onChange} label="Format" />)
    fireEvent.change(screen.getByLabelText("Format currency code"), { target: { value: "usd" } })
    expect(onChange).toHaveBeenCalledWith({ style: "compact", currency: "USD" })
  })

  it("turns the thousands separator off and sets decimals", () => {
    const onChange = mock<(f: ValueFormat) => void>(() => {})
    render(<ValueFormatEditor value={{ style: "number" }} onChange={onChange} label="Format" />)
    fireEvent.change(screen.getByLabelText("Format decimals"), { target: { value: "3" } })
    expect(onChange).toHaveBeenLastCalledWith({ style: "number", decimals: 3 })
    fireEvent.click(screen.getByLabelText("Format thousands separator"))
    expect(onChange).toHaveBeenLastCalledWith({ style: "number", thousands: false })
  })
})

describe("FormatSection", () => {
  it("edits the format of one value apart from the others", () => {
    const onChange = mock<(s: StyleFields) => void>(() => {})
    render(<FormatSection rules={formatRules({ kind: "bar", breakdown: false, measures: 2 })} style={{}} onChange={onChange} valueNames={["visitors", "nights"]} />)
    fireEvent.change(screen.getByLabelText("Number format decimals"), { target: { value: "1" } })
    expect(onChange).toHaveBeenLastCalledWith({ format: { decimals: 1 } })
  })

  it("lists a note, adds one, and stops at twenty", () => {
    const onChange = mock<(s: StyleFields) => void>(() => {})
    render(<FormatSection rules={formatRules({ kind: "bar", breakdown: false, measures: 1 })} style={{}} onChange={onChange} valueNames={["visitors"]} />)
    fireEvent.click(screen.getByRole("button", { name: /Add a note/ }))
    expect(onChange).toHaveBeenLastCalledWith({ annotations: [{ x: "", text: "" }] })
    cleanup()
    const twenty = Array.from({ length: 20 }, (_, i) => ({ x: `p${i}`, text: "n" }))
    render(<FormatSection rules={formatRules({ kind: "bar", breakdown: false, measures: 1 })} style={{ annotations: twenty }} onChange={onChange} valueNames={["visitors"]} />)
    expect(screen.queryByRole("button", { name: /Add a note/ })).toBeNull()
  })

  it("offers colour by value only when the chart can take it", () => {
    const noop = () => {}
    render(<FormatSection rules={formatRules({ kind: "bar", breakdown: true, measures: 1 })} style={{}} onChange={noop} valueNames={["visitors"]} />)
    expect(screen.queryByLabelText("Colour by value")).toBeNull()
    cleanup()
    render(<FormatSection rules={formatRules({ kind: "bar", breakdown: false, measures: 1 })} style={{}} onChange={noop} valueNames={["visitors"]} />)
    expect(screen.getByLabelText("Colour by value")).toBeTruthy()
  })
})
