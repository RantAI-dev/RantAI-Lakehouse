import { cleanup, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it } from "bun:test"

import { GrainMarkers } from "./grain-marker"

afterEach(cleanup)

// QA LH-BI-9-05 (BI-50): the quiet mark a chart the dashboard's date grouping
// does not change carries, with the sentence as its accessible name.
describe("GrainMarkers and the date grouping mark (BI-50)", () => {
  it("a chart the control does not change carries the mark with the sentence as its label", () => {
    render(<GrainMarkers notApplied="Group dates by does not apply: this chart's dimension is not a date." />)
    expect(screen.getByLabelText("Group dates by does not apply: this chart's dimension is not a date.")).toBeDefined()
  })

  it("without a reason no such mark is drawn", () => {
    const { container } = render(<GrainMarkers />)
    expect(container.querySelector("svg")).toBeNull()
    expect(container.textContent).toBe("")
  })
})
