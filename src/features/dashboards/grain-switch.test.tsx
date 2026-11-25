import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"
import { GrainMarkers } from "./grain-marker"
import { GrainSwitch } from "./grain-switch"

afterEach(cleanup)

const CHOICES = ["day", "week", "month", "quarter", "year"] as const

function setup(props: Partial<React.ComponentProps<typeof GrainSwitch>> = {}) {
  const onChange = mock(() => {})
  render(
    <GrainSwitch
      choices={CHOICES}
      value="month"
      onChange={onChange}
      {...props}
    />,
  )
  return { onChange }
}

describe("GrainSwitch", () => {
  it("shows the grain in force", () => {
    setup()
    expect(screen.getByRole("button", { name: "Group dates by" }).textContent).toContain("Month")
  })

  it("shows each chart's own when no grain is chosen", () => {
    setup({ value: "" })
    expect(screen.getByRole("button", { name: "Group dates by" }).textContent).toContain("Each chart")
  })

  // BI-61 T3: a chip whose panel lists the choices, the one in force ticked.
  it("lists the choices in its panel and reports the one picked", () => {
    const { onChange } = setup()
    fireEvent.click(screen.getByRole("button", { name: "Group dates by" }))
    expect(screen.getByRole("radio", { name: "Month" }).getAttribute("aria-checked")).toBe("true")
    fireEvent.click(screen.getByRole("radio", { name: "Week" }))
    expect(onChange).toHaveBeenCalledWith("week")
    fireEvent.click(screen.getByRole("button", { name: "Group dates by" }))
    fireEvent.click(screen.getByRole("radio", { name: "Each chart's own" }))
    expect(onChange).toHaveBeenCalledWith("")
  })

  it("has no Save as default of its own: the filter bar's one button saves both (R4)", () => {
    setup()
    expect(screen.queryByRole("button", { name: "Save as default" })).toBeNull()
  })
})

describe("GrainMarkers", () => {
  it("says a chart kept its own grouping, naming the grain it could not take", () => {
    render(<GrainMarkers skipped="hour" />)
    expect(screen.getByLabelText("Grouping by hour not applied to this chart")).toBeDefined()
    expect(screen.queryByLabelText("Only the latest buckets are shown")).toBeNull()
  })

  it("marks a cut-off chart", () => {
    render(<GrainMarkers truncated limit={1000} />)
    expect(screen.getByLabelText("Only the latest buckets are shown")).toBeDefined()
  })

  it("draws nothing for a chart that fits", () => {
    const { container } = render(<GrainMarkers />)
    expect(container.textContent).toBe("")
    expect(container.querySelector("svg")).toBeNull()
  })
})
