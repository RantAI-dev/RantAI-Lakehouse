import * as React from "react"
import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"
import { Label } from "@/components/ui/label"
import { SelectField, type SelectOption, type SelectOptionGroup } from "./select-field"
import { chooseOption } from "./select-field.testing"

afterEach(cleanup)

/** The words inside the trigger's value span; the chevron is decoration beside it. */
function shownLabel(): string | null | undefined {
  return screen
    .getByRole("combobox", { name: "Fruit" })
    .querySelector('[data-slot="select-value"]')?.textContent
}

const FRUITS: SelectOption[] = [
  { value: "fruit-apple", label: "Apple" },
  { value: "fruit-pear", label: "Pear" },
]

/** A controlled `SelectField`, so a choice moves the trigger as a person sees it. */
function Harness({
  initial,
  options,
  placeholder,
  onValueChange,
}: {
  initial: string
  options: readonly (SelectOption | SelectOptionGroup)[]
  placeholder?: string
  onValueChange: (value: string) => void
}) {
  const [value, setValue] = React.useState(initial)
  return (
    <SelectField
      aria-label="Fruit"
      value={value}
      options={options}
      placeholder={placeholder}
      onValueChange={(next) => {
        onValueChange(next)
        setValue(next)
      }}
    />
  )
}

describe("SelectField", () => {
  it("shows the chosen option's label in the trigger, not its value", () => {
    render(
      <SelectField aria-label="Fruit" value="fruit-apple" options={FRUITS} onValueChange={() => {}} />
    )
    const trigger = screen.getByRole("combobox", { name: "Fruit" })
    expect(shownLabel()).toBe("Apple")
    expect(trigger.textContent).not.toContain("fruit-apple")
  })

  it("calls onValueChange with the chosen option's value", () => {
    const onValueChange = mock<(value: string) => void>(() => {})
    render(<Harness initial="fruit-apple" options={FRUITS} onValueChange={onValueChange} />)

    chooseOption(screen.getByRole("combobox", { name: "Fruit" }), "Pear")

    expect(onValueChange).toHaveBeenCalledWith("fruit-pear")
    expect(shownLabel()).toBe("Pear")
  })

  it("shows the placeholder while the value matches no option", () => {
    render(
      <SelectField
        aria-label="Fruit"
        value="not-in-the-list"
        options={FRUITS}
        placeholder="Pick a fruit"
        onValueChange={() => {}}
      />
    )
    expect(shownLabel()).toBe("Pick a fruit")
  })

  it("cannot choose a disabled option", () => {
    const onValueChange = mock<(value: string) => void>(() => {})
    render(
      <Harness
        initial="fruit-apple"
        options={[...FRUITS, { value: "fruit-plum", label: "Plum", disabled: true }]}
        onValueChange={onValueChange}
      />
    )
    const trigger = screen.getByRole("combobox", { name: "Fruit" })
    fireEvent.click(trigger)
    const plum = screen.getByRole("option", { name: "Plum" })
    expect(plum.getAttribute("aria-disabled")).toBe("true")

    fireEvent.pointerDown(plum, { button: 0, pointerType: "mouse" })
    fireEvent.mouseDown(plum, { button: 0 })
    fireEvent.pointerUp(plum, { button: 0, pointerType: "mouse" })
    fireEvent.mouseUp(plum, { button: 0 })
    fireEvent.click(plum)

    expect(onValueChange).not.toHaveBeenCalled()
  })

  it("can choose an option whose value is the empty string, and shows its label", () => {
    const onValueChange = mock<(value: string) => void>(() => {})
    render(
      <Harness
        initial="fruit-apple"
        options={[{ value: "", label: "None" }, ...FRUITS]}
        onValueChange={onValueChange}
      />
    )

    chooseOption(screen.getByRole("combobox", { name: "Fruit" }), "None")

    expect(onValueChange).toHaveBeenCalledWith("")
    expect(shownLabel()).toBe("None")
  })

  it("shows a group's label and its options", () => {
    const groups: SelectOptionGroup[] = [
      { label: "Fruits", options: FRUITS },
      { label: "Vegetables", options: [{ value: "veg-carrot", label: "Carrot" }] },
    ]
    render(<Harness initial="fruit-apple" options={groups} onValueChange={() => {}} />)

    fireEvent.click(screen.getByRole("combobox", { name: "Fruit" }))

    expect(screen.getByText("Fruits")).toBeTruthy()
    expect(screen.getByText("Vegetables")).toBeTruthy()
    expect(screen.getByRole("option", { name: "Apple" })).toBeTruthy()
    expect(screen.getByRole("option", { name: "Carrot" })).toBeTruthy()
  })

  it("is named by a Label htmlFor, through its id", () => {
    render(
      <>
        <Label htmlFor="fruit-field">Fruit</Label>
        <SelectField id="fruit-field" value="fruit-apple" options={FRUITS} onValueChange={() => {}} />
      </>
    )
    expect(screen.getByRole("combobox", { name: "Fruit" })).toBeTruthy()
  })
})
