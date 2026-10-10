// A catalog table's age used to be judged on streaming thresholds: anything
// over an hour old read "Stale", so every daily-refreshed table was red.
// An asset is now judged against its own target, and one with no target
// gets no verdict at all.
import { cleanup, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it } from "bun:test"
import { FreshnessIndicator } from "./freshness-indicator"

afterEach(cleanup)

const HOUR = 3600

describe("FreshnessIndicator", () => {
  it("calls a daily table loaded yesterday on time, and one loaded last week late", () => {
    const { rerender } = render(
      <FreshnessIndicator lagSeconds={25 * HOUR} targetSeconds={36 * HOUR} targetSource="frequency" />
    )
    const onTime = screen.getByText("On time · 25h 00m")
    expect(onTime.className).toContain("text-emerald")
    expect(onTime.getAttribute("title")).toBe(
      "Last written 25h 00m ago. Expected within 36h 00m (refresh frequency)."
    )

    rerender(<FreshnessIndicator lagSeconds={194 * HOUR} targetSeconds={36 * HOUR} targetSource="sla" />)
    const late = screen.getByText("Late · 8d 2h")
    expect(late.className).toContain("text-destructive")
    expect(late.getAttribute("title")).toContain("(freshness SLA)")
  })

  it("shows the age with no verdict when nothing says how often the asset refreshes", () => {
    render(<FreshnessIndicator lagSeconds={194 * HOUR} targetSeconds={null} />)
    const age = screen.getByText("8d 2h ago")
    expect(age.className).toContain("text-muted-foreground")
    expect(screen.queryByText(/Stale|Late|On time/)).toBeNull()
  })

  it("keeps the streaming thresholds where no target is passed at all", () => {
    render(<FreshnessIndicator lagSeconds={2 * HOUR} />)
    expect(screen.getByText("Stale · 2h 00m")).toBeTruthy()
  })

  it("never judges an age nobody measured", () => {
    render(<FreshnessIndicator lagSeconds={null} targetSeconds={36 * HOUR} />)
    expect(screen.getByText("Not measured")).toBeTruthy()
  })
})
