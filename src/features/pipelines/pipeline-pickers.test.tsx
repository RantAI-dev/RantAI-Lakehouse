// WEB-8 D8: the three pipeline pickers. The page-level wiring is proved in
// `pipeline-create-page.test.tsx`; what is pinned here is each control's own
// contract — a picker over the list it is given, a stored value the list does
// not have kept and shown, and free text back when there is no list at all.
import { useState } from "react"
import { cleanup, fireEvent, render, screen, within } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"
import { ColumnField, ColumnsField, ZoneField } from "./pipeline-pickers"

afterEach(cleanup)

// cmdk scrolls the selected item into view; jsdom/happy-dom has no such method.
Element.prototype.scrollIntoView = Element.prototype.scrollIntoView ?? (() => {})

/** base-ui's Select commits on the pointer sequence, not on a bare click. */
function pickOption(option: HTMLElement) {
  fireEvent.pointerDown(option, { button: 0, pointerType: "mouse" })
  fireEvent.mouseDown(option, { button: 0 })
  fireEvent.pointerUp(option, { button: 0, pointerType: "mouse" })
  fireEvent.mouseUp(option, { button: 0 })
  fireEvent.click(option)
}

const COLUMNS = ["id", "status", "amount"]

describe("ZoneField", () => {
  it("picks from the namespaces it is given", async () => {
    const onChange = mock<(next: string) => void>(() => {})
    render(<ZoneField id="zone" label="Source zone" value="bronze" onChange={onChange} zones={["bronze", "silver"]} />)
    const trigger = screen.getByRole("combobox", { name: "Source zone" })
    expect(trigger.textContent).toContain("bronze")
    fireEvent.click(trigger)
    pickOption(await screen.findByRole("option", { name: "silver" }))
    expect(onChange.mock.calls[0][0]).toBe("silver")
  })

  it("offers a stored zone the list does not have, marked, instead of dropping it", async () => {
    render(<ZoneField id="zone" label="Target zone" value="gold" onChange={() => {}} zones={["bronze", "silver"]} />)
    const trigger = screen.getByRole("combobox", { name: "Target zone" })
    expect(trigger.textContent).toContain("gold (not a namespace this lakehouse lists)")
    fireEvent.click(trigger)
    expect(screen.getAllByRole("option").map((o) => o.textContent)).toContain(
      "gold (not a namespace this lakehouse lists)"
    )
  })

  it("is free text, with the note, when there is no list", () => {
    const onChange = mock<(next: string) => void>(() => {})
    render(
      <ZoneField
        id="zone"
        label="Source zone"
        value="bronze"
        onChange={onChange}
        zones={null}
        note="Could not read the lakehouse's namespaces; type the zone name instead."
      />
    )
    expect(screen.queryByRole("combobox")).toBeNull()
    fireEvent.change(screen.getByRole("textbox", { name: "Source zone" }), { target: { value: "silver" } })
    expect(onChange.mock.calls[0][0]).toBe("silver")
    expect(screen.getByText(/type the zone name instead/)).toBeTruthy()
  })
})

describe("ColumnField", () => {
  it("picks from the table's columns", async () => {
    const onChange = mock<(next: string) => void>(() => {})
    render(<ColumnField label="Key column" value="" onChange={onChange} columns={COLUMNS} placeholder="order_id" />)
    const trigger = screen.getByRole("combobox", { name: "Key column" })
    expect(trigger.textContent).toContain("order_id")
    fireEvent.click(trigger)
    expect(screen.getAllByRole("option").map((o) => o.textContent)).toEqual(COLUMNS)
    pickOption(screen.getByRole("option", { name: "status" }))
    expect(onChange.mock.calls[0][0]).toBe("status")
  })

  it("shows and keeps a column the table does not have", async () => {
    render(<ColumnField label="From column" value="legacy_col" onChange={() => {}} columns={COLUMNS} />)
    const trigger = screen.getByRole("combobox", { name: "From column" })
    expect(trigger.textContent).toContain("legacy_col (not in this table)")
    fireEvent.click(trigger)
    expect(screen.getAllByRole("option").map((o) => o.textContent)).toContain(
      "legacy_col (not in this table)"
    )
  })

  it("is free text when the table is not known", () => {
    const onChange = mock<(next: string) => void>(() => {})
    render(<ColumnField label="Column" value="" onChange={onChange} columns={null} placeholder="status" />)
    expect(screen.queryByRole("combobox")).toBeNull()
    fireEvent.change(screen.getByRole("textbox", { name: "Column" }), { target: { value: "status" } })
    expect(onChange.mock.calls[0][0]).toBe("status")
  })
})

/** The form's own state behind the field: the picker is controlled. */
function ColumnsHarness({ onChange }: { onChange: (next: string) => void }) {
  const [value, setValue] = useState("")
  return (
    <ColumnsField
      label="Columns"
      value={value}
      onChange={(next) => {
        setValue(next)
        onChange(next)
      }}
      columns={COLUMNS}
    />
  )
}

describe("ColumnsField", () => {
  it("is a multi-select over the table's columns and writes the comma list the grammar takes", async () => {
    const onChange = mock<(next: string) => void>(() => {})
    render(<ColumnsHarness onChange={onChange} />)
    const trigger = screen.getByRole("button", { name: "Columns" })
    expect(trigger.textContent).toContain("Pick columns")
    fireEvent.click(trigger)
    const list = await screen.findByRole("listbox")
    fireEvent.click(within(list).getByText("id"))
    expect(onChange.mock.calls[0][0]).toBe("id")
    fireEvent.click(within(list).getByText("amount"))
    expect(onChange.mock.calls[1][0]).toBe("id,amount")
  })

  it("lists a column the draft names that the table does not have, so it can be taken out", async () => {
    render(<ColumnsField label="Columns" value="id,gone" onChange={() => {}} columns={COLUMNS} />)
    const trigger = screen.getByRole("button", { name: "Columns" })
    // Both the known and the unknown one are visible as chips.
    expect(trigger.textContent).toContain("id")
    expect(trigger.textContent).toContain("gone (not in this table)")
    fireEvent.click(trigger)
    const list = await screen.findByRole("listbox")
    expect(within(list).getByText("gone (not in this table)")).toBeTruthy()
  })

  it("is a comma-separated text field when the table is not known", () => {
    const onChange = mock<(next: string) => void>(() => {})
    render(<ColumnsField label="Columns" value="" onChange={onChange} columns={null} />)
    fireEvent.change(screen.getByRole("textbox", { name: "Columns" }), { target: { value: "id,name" } })
    expect(onChange.mock.calls[0][0]).toBe("id,name")
  })
})
