import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"
import { DownloadDataDialog, DownloadPdfDialog } from "./download-dialogs"

afterEach(cleanup)

describe("Download PDF…", () => {
  it("sends the chosen paper and orientation and closes when the file is saved", async () => {
    const onDownload = mock(async () => {})
    const onOpenChange = mock(() => {})
    render(<DownloadPdfDialog open onOpenChange={onOpenChange} onDownload={onDownload} />)
    fireEvent.click(screen.getByRole("radio", { name: "Letter" }))
    fireEvent.click(screen.getByRole("radio", { name: "Landscape" }))
    fireEvent.click(screen.getByRole("button", { name: "Download" }))
    await waitFor(() => expect(onDownload).toHaveBeenCalledWith({ paper: "letter", orientation: "landscape" }))
    await waitFor(() => expect(onOpenChange).toHaveBeenCalledWith(false))
  })

  it("defaults to A4 portrait and stays open with the server's sentence when the PDF fails", async () => {
    const onDownload = mock(async () => { throw new Error("The PDF service is not reachable. Reference: r1") })
    const onOpenChange = mock(() => {})
    render(<DownloadPdfDialog open onOpenChange={onOpenChange} onDownload={onDownload} />)
    fireEvent.click(screen.getByRole("button", { name: "Download" }))
    expect((await screen.findByRole("alert")).textContent).toBe("The PDF service is not reachable. Reference: r1")
    expect(onDownload).toHaveBeenCalledWith({ paper: "a4", orientation: "portrait" })
    expect(onOpenChange).not.toHaveBeenCalledWith(false)
    // Another try is possible.
    expect((screen.getByRole("button", { name: "Download" }) as HTMLButtonElement).disabled).toBe(false)
  })
})

describe("Download data…", () => {
  it("defaults to Excel with the values as on the chart, and sends the other choices", async () => {
    const onDownload = mock(async () => {})
    render(<DownloadDataDialog open onOpenChange={() => {}} title="Visits" onDownload={onDownload} />)
    fireEvent.click(screen.getByRole("button", { name: "Download" }))
    await waitFor(() => expect(onDownload).toHaveBeenCalledWith({ format: "xlsx", values: "formatted" }))
    cleanup()
    const again = mock(async () => {})
    render(<DownloadDataDialog open onOpenChange={() => {}} title="Visits" onDownload={again} />)
    fireEvent.click(screen.getByRole("radio", { name: "CSV" }))
    fireEvent.click(screen.getByRole("radio", { name: "Raw values" }))
    fireEvent.click(screen.getByRole("button", { name: "Download" }))
    await waitFor(() => expect(again).toHaveBeenCalledWith({ format: "csv", values: "raw" }))
  })

  it("shows the server's refusal and keeps the dialog open", async () => {
    const onDownload = mock(async () => { throw new Error("permission_denied: Downloads are switched off for your role.") })
    render(<DownloadDataDialog open onOpenChange={() => {}} title="Visits" onDownload={onDownload} />)
    fireEvent.click(screen.getByRole("button", { name: "Download" }))
    expect((await screen.findByRole("alert")).textContent).toContain("Downloads are switched off for your role.")
  })
})
