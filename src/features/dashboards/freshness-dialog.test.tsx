import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"
import { FreshnessDialog } from "./freshness-dialog"

afterEach(cleanup)

describe("the dashboard's Freshness dialog (BI-4 part B)", () => {
  it("starts from the dashboard's own rule and saves the changed minutes", () => {
    const onSave = mock(() => undefined)
    render(<FreshnessDialog open onOpenChange={() => {}} rule={{ mode: "minutes", minutes: 30 }} onSave={onSave} />)
    const minutes = screen.getByLabelText("Freshness: minutes") as HTMLInputElement
    expect(minutes.value).toBe("30")
    fireEvent.change(minutes, { target: { value: "90" } })
    fireEvent.click(screen.getByRole("button", { name: "Save" }))
    expect((onSave.mock.calls[0] as unknown[])[0]).toEqual({ mode: "minutes", minutes: 90 })
  })

  it("says the range and keeps Save off for minutes out of range", () => {
    render(<FreshnessDialog open onOpenChange={() => {}} rule={{ mode: "minutes", minutes: 30 }} onSave={() => {}} />)
    fireEvent.change(screen.getByLabelText("Freshness: minutes"), { target: { value: "2000" } })
    expect(screen.getByRole("alert").textContent).toBe("Remember for between 1 minute and 24 hours (1 to 1440 minutes).")
    expect((screen.getByRole("button", { name: "Save" }) as HTMLButtonElement).disabled).toBe(true)
  })

  it("names the install's rule when the dashboard follows it, and shows the server's refusal", () => {
    render(<FreshnessDialog open onOpenChange={() => {}} rule={null} effective={{ mode: "fresh" }} error="Only editors can do that." onSave={() => {}} />)
    expect(screen.getByText(/Now: Always fresh/)).toBeTruthy()
    expect(screen.getByRole("alert").textContent).toBe("Only editors can do that.")
  })
})
