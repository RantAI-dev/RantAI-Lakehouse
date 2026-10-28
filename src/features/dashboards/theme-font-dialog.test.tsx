import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"
import { ThemeFontDialog } from "./theme-font-dialog"

afterEach(cleanup)

function open(over: Partial<React.ComponentProps<typeof ThemeFontDialog>> = {}) {
  const onSave = mock<(c: { theme: string; font: string }) => void>(() => {})
  render(<ThemeFontDialog open onOpenChange={() => {}} allowedFonts={["inter", "system"]} onSave={onSave} {...over} />)
  return onSave
}

describe("ThemeFontDialog", () => {
  it("starts from the dashboard's saved theme and font and saves them unchanged", () => {
    const onSave = open({ theme: "dark", font: "inter" })
    fireEvent.click(screen.getByRole("button", { name: "Save" }))
    expect(onSave).toHaveBeenCalledWith({ theme: "dark", font: "inter" })
  })

  it("a dashboard with neither follows the console and its own font", () => {
    const onSave = open()
    fireEvent.click(screen.getByRole("button", { name: "Save" }))
    expect(onSave).toHaveBeenCalledWith({ theme: "auto", font: "" })
  })

  it("shows the server's sentence when the save was refused and disables Save while saving", () => {
    open({ error: "That font is not one an administrator allows for dashboards.", saving: true })
    expect(screen.getByRole("alert").textContent).toContain("administrator allows")
    expect((screen.getByRole("button", { name: "Saving…" }) as HTMLButtonElement).disabled).toBe(true)
  })
})
