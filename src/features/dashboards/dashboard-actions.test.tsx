import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"
import { DashboardActionsMenu } from "./dashboard-actions"

afterEach(cleanup)

function renderMenu(over: Partial<React.ComponentProps<typeof DashboardActionsMenu>> = {}) {
  const props: React.ComponentProps<typeof DashboardActionsMenu> = {
    isDefault: false, loading: false, fullscreen: false, autoSec: "0", savedSec: "300",
    canSaveRefresh: false, savingRefresh: false,
    onRefresh: () => {}, onToggleFullscreen: () => {}, onAutoSec: () => {}, onSaveRefresh: () => {},
    onRename: () => {}, onShare: () => {}, onExportPdf: () => {}, onDuplicate: () => {}, onDelete: () => {},
    ...over,
  }
  render(<DashboardActionsMenu {...props} />)
  fireEvent.click(screen.getByRole("button", { name: "More actions" }))
}

describe("DashboardActionsMenu auto-refresh", () => {
  it("lists the server's intervals and marks the saved default", async () => {
    renderMenu()
    for (const label of ["Manual", "Every 1m", "Every 5m", "Every 10m", "Every 15m", "Every 30m", "Every 60m"]) {
      expect(await screen.findByText(label)).toBeTruthy()
    }
    // The old 30-second choice is gone: the server refuses it.
    expect(screen.queryByText("Every 30s")).toBeNull()
    expect(screen.getByText("default")).toBeTruthy()
  })

  it("offers saving the choice as the default only when asked to", async () => {
    renderMenu({ canSaveRefresh: false })
    await screen.findByText("Manual")
    expect(screen.queryByText("Save as dashboard default")).toBeNull()
    cleanup()

    const onSaveRefresh = mock(() => {})
    renderMenu({ canSaveRefresh: true, onSaveRefresh })
    fireEvent.click(await screen.findByText("Save as dashboard default"))
    expect(onSaveRefresh).toHaveBeenCalledTimes(1)
  })

  it("reports the interval a viewer picks as seconds", async () => {
    const onAutoSec = mock<(v: string) => void>(() => {})
    renderMenu({ onAutoSec })
    fireEvent.click(await screen.findByText("Every 10m"))
    expect(onAutoSec).toHaveBeenCalledWith("600")
  })
})
