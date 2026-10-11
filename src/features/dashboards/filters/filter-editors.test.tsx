import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it } from "bun:test"
import type { FilterDef } from "@/services/clients/bi-store"
import { DateEditor, NumberEditor, blankFilter } from "./filter-editors"

afterEach(cleanup)

function applied(ui: (onApply: (f: FilterDef) => void) => React.ReactElement): FilterDef[] {
  const out: FilterDef[] = []
  render(ui((f) => out.push(f)))
  return out
}

describe("the date and number editors", () => {
  it("reopens a stored 'before' filter on its date and applies it unchanged", () => {
    const initial: FilterDef = { column: "d", op: "between", values: [], max: "2026-10-03", maxExclusive: true }
    const out = applied((onApply) => (
      <DateEditor board="b" column="d" others={[]} initial={initial} onApply={onApply} />
    ))
    expect((screen.getByLabelText("Before") as HTMLInputElement).value).toBe("2026-10-03")
    expect((screen.getByLabelText("Date filter") as HTMLSelectElement).value).toBe("before")
    fireEvent.click(screen.getByText("Apply"))
    expect(out).toEqual([initial])
  })

  it("stores 'Next 7 days' from the relative tab", () => {
    const out = applied((onApply) => (
      <DateEditor board="b" column="d" others={[]} initial={blankFilter("d", "date")} onApply={onApply} />
    ))
    fireEvent.change(screen.getByLabelText("Direction"), { target: { value: "next" } })
    fireEvent.change(screen.getByLabelText("How many"), { target: { value: "7" } })
    fireEvent.click(screen.getAllByText("Apply")[0])
    expect(out).toEqual([{ column: "d", values: [], op: "relative", anchor: "next", n: 7, unit: "day" }])
  })

  it("keeps Apply disabled until the number is a number, then stores 'greater than' as an exclusive bound", () => {
    const out = applied((onApply) => (
      <NumberEditor board="b" column="n" others={[]} initial={blankFilter("n", "number")} onApply={onApply} />
    ))
    const apply = screen.getByText("Apply") as HTMLButtonElement
    expect(apply.disabled).toBe(true)
    fireEvent.change(screen.getByLabelText("Comparison"), { target: { value: "gt" } })
    fireEvent.change(screen.getByLabelText("Number"), { target: { value: "5" } })
    expect(apply.disabled).toBe(false)
    fireEvent.click(apply)
    expect(out).toEqual([{ column: "n", values: [], op: "between", min: "5", minExclusive: true }])
  })
})
