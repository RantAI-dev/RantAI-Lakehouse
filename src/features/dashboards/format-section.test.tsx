import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"
import type { CondRule, StyleFields, ValueFormat } from "@/lib/chart-style"
import { formatRules } from "@/lib/style-draft"
import { ConditionalEditor, FormatSection, ValueFormatEditor } from "./format-section"

afterEach(cleanup)

/**
 * Chooses an option of a base-ui Select the way a pointer does: the list
 * opens, the option takes focus (which makes it the active one), and it is
 * clicked. In this jsdom a click on an option that was not focused first does
 * not reach `onValueChange`, so the focus step is part of the choice.
 */
function choose(trigger: HTMLElement, option: string) {
  fireEvent.click(trigger)
  const target = screen.getByRole("option", { name: option })
  fireEvent.focus(target)
  fireEvent.click(target)
}

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
    render(<FormatSection rules={formatRules({ kind: "bar", breakdown: false, measures: 2 })} style={{}} onChange={onChange} valueNames={["visitors", "nights"]} paletteValue="default" />)
    fireEvent.change(screen.getByLabelText("Number format decimals"), { target: { value: "1" } })
    expect(onChange).toHaveBeenLastCalledWith({ format: { decimals: 1 } })
  })

  it("lists a note, adds one, and stops at twenty", () => {
    const onChange = mock<(s: StyleFields) => void>(() => {})
    render(<FormatSection rules={formatRules({ kind: "bar", breakdown: false, measures: 1 })} style={{}} onChange={onChange} valueNames={["visitors"]} paletteValue="default" />)
    fireEvent.click(screen.getByRole("button", { name: /Add a note/ }))
    expect(onChange).toHaveBeenLastCalledWith({ annotations: [{ x: "", text: "" }] })
    cleanup()
    const twenty = Array.from({ length: 20 }, (_, i) => ({ x: `p${i}`, text: "n" }))
    render(<FormatSection rules={formatRules({ kind: "bar", breakdown: false, measures: 1 })} style={{ annotations: twenty }} onChange={onChange} valueNames={["visitors"]} paletteValue="default" />)
    expect(screen.queryByRole("button", { name: /Add a note/ })).toBeNull()
  })

  it("offers colour by value only when the chart can take it", () => {
    const noop = () => {}
    render(<FormatSection rules={formatRules({ kind: "bar", breakdown: true, measures: 1 })} style={{}} onChange={noop} valueNames={["visitors"]} paletteValue="default" />)
    expect(screen.queryByLabelText("Colour by value")).toBeNull()
    cleanup()
    render(<FormatSection rules={formatRules({ kind: "bar", breakdown: false, measures: 1 })} style={{}} onChange={noop} valueNames={["visitors"]} paletteValue="default" />)
    expect(screen.getByLabelText("Colour by value")).toBeTruthy()
  })

  // PR #199 review O SHOULD-FIX 4: picking "Default" has to be a choice of
  // its own, or a company palette takes the chart over.
  it("keeps the default palette a person chose, and shows the palette the chart draws with", () => {
    const onChange = mock<(s: StyleFields) => void>(() => {})
    render(<FormatSection rules={formatRules({ kind: "bar", breakdown: false, measures: 1 })} style={{}} onChange={onChange} valueNames={["visitors"]} paletteValue="company" />)
    expect(screen.getByRole("combobox", { name: "Palette" }).textContent).toContain("Company palette")
    choose(screen.getByRole("combobox", { name: "Palette" }), "Default")
    expect(onChange).toHaveBeenLastCalledWith({ colors: { palette: "default" } })
  })
})

describe("FormatSection clusters (BI-17 part B)", () => {
  const scatter = formatRules({ kind: "scatter", breakdown: false, measures: 2 })

  it("switches clusters on at Automatic, takes a number of groups, and switches them off", () => {
    const onChange = mock<(s: StyleFields) => void>(() => {})
    const { rerender } = render(<FormatSection rules={scatter} style={{}} onChange={onChange} valueNames={["x", "y"]} seriesNames={[]} paletteValue="default" />)
    fireEvent.click(screen.getByRole("switch", { name: "Colour points by cluster" }))
    expect(onChange.mock.calls[0]![0]).toEqual({ clusters: { k: "auto" } })
    rerender(<FormatSection rules={scatter} style={{ clusters: { k: "auto" } }} onChange={onChange} valueNames={["x", "y"]} seriesNames={[]} paletteValue="default" />)
    choose(screen.getByLabelText("Number of clusters"), "4")
    expect(onChange.mock.calls[1]![0]).toEqual({ clusters: { k: 4 } })
    rerender(<FormatSection rules={scatter} style={{ clusters: { k: 4 } }} onChange={onChange} valueNames={["x", "y"]} seriesNames={[]} paletteValue="default" />)
    fireEvent.click(screen.getByRole("switch", { name: "Colour points by cluster" }))
    expect(onChange.mock.calls[2]![0].clusters).toBeUndefined()
  })

  it("offers no clusters on a line chart", () => {
    render(<FormatSection rules={formatRules({ kind: "line", breakdown: false, measures: 1 })} style={{}} onChange={() => {}} valueNames={["v"]} seriesNames={[]} paletteValue="default" />)
    expect(screen.queryByRole("switch", { name: "Colour points by cluster" })).toBeNull()
  })
})

describe("Clusters (BI-17 part B)", () => {
  const scatter = formatRules({ kind: "scatter", breakdown: false, measures: 2 })
  const section = (style: StyleFields, onChange: (s: StyleFields) => void, rules = scatter) =>
    <FormatSection rules={rules} style={style} onChange={onChange} valueNames={["sales", "visits"]} paletteValue="default" />

  it("switches clusters on at automatic, then takes 2 to 10 groups, and switches them off", () => {
    const onChange = mock<(s: StyleFields) => void>(() => {})
    const { rerender } = render(section({}, onChange))
    expect(screen.queryByLabelText("Number of clusters")).toBeNull()
    fireEvent.click(screen.getByRole("switch", { name: "Colour points by cluster" }))
    expect(onChange.mock.calls[0]![0].clusters).toEqual({ k: "auto" })
    rerender(section({ clusters: { k: "auto" } }, onChange))
    choose(screen.getByLabelText("Number of clusters"), "4")
    expect(onChange.mock.calls[1]![0].clusters).toEqual({ k: 4 })
    fireEvent.click(screen.getByRole("switch", { name: "Colour points by cluster" }))
    expect(onChange.mock.calls[2]![0].clusters).toBeUndefined()
  })

  it("is not offered on a chart kind that cannot cluster", () => {
    render(section({}, () => {}, formatRules({ kind: "bar", breakdown: false, measures: 1 })))
    expect(screen.queryByRole("switch", { name: "Colour points by cluster" })).toBeNull()
  })
})
