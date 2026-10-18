import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"
import { DashboardActionsMenu, EditLayoutButton, NewChartButton } from "./dashboard-actions"
import { chartWriteItems } from "./tile-write-items"

afterEach(cleanup)

function renderMenu(over: Partial<React.ComponentProps<typeof DashboardActionsMenu>> = {}) {
  const props: React.ComponentProps<typeof DashboardActionsMenu> = {
    isDefault: false, canWrite: true, loading: false, fullscreen: false, autoSec: "0", savedSec: "300",
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

describe("DashboardActionsMenu theme and font", () => {
  it("offers Theme and font on a user dashboard and not on the built-in one", async () => {
    const onTheme = mock(() => {})
    renderMenu({ onTheme })
    fireEvent.click(await screen.findByText("Theme and font…"))
    expect(onTheme).toHaveBeenCalledTimes(1)
    cleanup()
    renderMenu({ isDefault: true, onTheme })
    await screen.findByText("Manual")
    expect(screen.queryByText("Theme and font…")).toBeNull()
  })
})

describe("DashboardActionsMenu history and personal space", () => {
  it("offers the version history on a user dashboard and not on the built-in one", async () => {
    const onHistory = mock(() => {})
    renderMenu({ onHistory })
    fireEvent.click(await screen.findByText("Version history…"))
    expect(onHistory).toHaveBeenCalledTimes(1)
    cleanup()
    renderMenu({ isDefault: true, onHistory })
    await screen.findByText("Manual")
    expect(screen.queryByText("Version history…")).toBeNull()
  })

  it("disables Share on a private dashboard and says why", async () => {
    const onShare = mock(() => {})
    renderMenu({ isPrivate: true, onShare })
    const share = await screen.findByText("Share…")
    fireEvent.click(share)
    expect(onShare).not.toHaveBeenCalled()
  })

  it("keeps Share working on a shared dashboard", async () => {
    const onShare = mock(() => {})
    renderMenu({ isPrivate: false, onShare })
    fireEvent.click(await screen.findByText("Share…"))
    expect(onShare).toHaveBeenCalledTimes(1)
  })
})

describe("DashboardActionsMenu downloads (BI-1)", () => {
  it("offers Download PDF and Print when the deployment can make a PDF", async () => {
    const onDownloadPdf = mock(() => {})
    const onExportPdf = mock(() => {})
    renderMenu({ onDownloadPdf, onExportPdf, pdfSupported: true })
    fireEvent.click(await screen.findByText("Download PDF…"))
    expect(onDownloadPdf).toHaveBeenCalledTimes(1)
    cleanup()
    renderMenu({ onDownloadPdf, onExportPdf, pdfSupported: true })
    fireEvent.click(await screen.findByText("Print…"))
    expect(onExportPdf).toHaveBeenCalledTimes(1)
    expect(screen.queryByText("Export PDF (print)")).toBeNull()
  })

  it("keeps Download PDF disabled, saying why, when the deployment has no renderer, and Print still works", async () => {
    const onDownloadPdf = mock(() => {})
    const onExportPdf = mock(() => {})
    renderMenu({ onDownloadPdf, onExportPdf, pdfSupported: false })
    const item = (await screen.findByText("Download PDF…")).closest("[role='menuitem']") as HTMLElement
    expect(item.getAttribute("aria-disabled") === "true" || item.hasAttribute("data-disabled")).toBe(true)
    fireEvent.click(item)
    expect(onDownloadPdf).not.toHaveBeenCalled()
    fireEvent.click(screen.getByText("Print…"))
    expect(onExportPdf).toHaveBeenCalledTimes(1)
  })

  it("hides Download PDF for a person whose every role is switched off from downloading, and keeps Print", async () => {
    renderMenu({ onDownloadPdf: () => {}, pdfSupported: true, canDownload: false })
    await screen.findByText("Print…")
    expect(screen.queryByText("Download PDF…")).toBeNull()
  })
})

// BI-18 part C review fix (SHOULD-FIX) R18: no write control for a caller without dashboard:write.
describe("write controls (R18)", () => {
  it("the dashboard menu leaves out Rename, Duplicate and Delete for a viewer, and keeps the read actions", async () => {
    renderMenu({ canWrite: false })
    await screen.findByText("Manual")
    for (const label of ["Rename", "Duplicate", "Delete dashboard"]) expect(screen.queryByText(label)).toBeNull()
    for (const label of ["Refresh now", "Fullscreen", "Print…", "Export YAML", "Share…"]) {
      expect(screen.getByText(label)).toBeTruthy()
    }
  })

  it("the dashboard menu shows Rename, Duplicate and Delete to an editor", async () => {
    renderMenu({ canWrite: true })
    for (const label of ["Rename", "Duplicate", "Delete dashboard"]) expect(await screen.findByText(label)).toBeTruthy()
  })

  it("Edit layout and New chart are shown to an editor and not to a viewer", () => {
    render(<><EditLayoutButton canWrite edit={false} onToggle={() => {}} /><NewChartButton canWrite onClick={() => {}} /></>)
    expect(screen.getByText("Edit layout")).toBeTruthy()
    expect(screen.getByText("New chart")).toBeTruthy()
    cleanup()
    render(<><EditLayoutButton canWrite={false} edit={false} onToggle={() => {}} /><NewChartButton canWrite={false} onClick={() => {}} /></>)
    expect(screen.queryByText("Edit layout")).toBeNull()
    expect(screen.queryByText("New chart")).toBeNull()
  })

  it("a chart tile's menu has Edit chart and Delete chart for an editor of its own chart only", () => {
    const base = { own: true, hasDef: true, onEdit: () => {}, onDelete: () => {} }
    const editor = chartWriteItems({ ...base, canWrite: true })
    expect(editor.edit.map((i) => i.label)).toEqual(["Edit chart"])
    expect(editor.remove.map((i) => i.label)).toEqual(["Delete chart"])
    expect(chartWriteItems({ ...base, canWrite: false })).toEqual({ edit: [], remove: [] })
    expect(chartWriteItems({ ...base, canWrite: true, own: false })).toEqual({ edit: [], remove: [] })
  })
})
