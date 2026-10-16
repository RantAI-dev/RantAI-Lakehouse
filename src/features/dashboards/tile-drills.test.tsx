import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"
import { DrillMenu } from "./drill"
import { DrillTrail } from "./tile-drills"

afterEach(cleanup)

const drill = { name: "Jawa Barat", column: "provinsi", mart: "m", x: 10, y: 10, builtin: false }

describe("DrillMenu", () => {
  it("offers the next level, and selecting it goes down", () => {
    const onSelect = mock(() => {})
    render(<DrillMenu drill={drill} onClose={() => {}} drillDown={{ label: "Drill down to kab_kota", onSelect }} />)
    fireEvent.click(screen.getByText("Drill down to kab_kota"))
    expect(onSelect).toHaveBeenCalledTimes(1)
  })

  it("says plainly why a chart cannot be drilled, with nothing to click", () => {
    render(<DrillMenu drill={drill} onClose={() => {}} drillDown={{ reason: "a chart grouped in bins cannot be drilled down." }} />)
    expect(screen.getByText(/Cannot drill down: a chart grouped in bins/)).toBeTruthy()
    expect(screen.queryByRole("menuitem")).toBeNull()
  })

  it("shows no drill line at all for a column that is in no hierarchy", () => {
    render(<DrillMenu drill={drill} onClose={() => {}} />)
    expect(screen.queryByText(/drill down/i)).toBeNull()
  })
})

describe("DrillTrail", () => {
  it("climbs back to any earlier entry and shows the current one as text", () => {
    const onClimb = mock<(index: number) => void>(() => {})
    render(<DrillTrail steps={[{ value: "Jawa Barat" }, { value: "k1", label: "Kota Bandung" }]} onClimb={onClimb} />)
    expect(screen.getByText("Kota Bandung").getAttribute("aria-current")).toBe("page")
    fireEvent.click(screen.getByText("Jawa Barat"))
    fireEvent.click(screen.getByText("All"))
    expect(onClimb.mock.calls.map((c) => c[0])).toEqual([1, 0])
  })
})
