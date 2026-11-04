// The download menu of a result: the four file kinds it offers, and the
// reason it is closed for a result the server cannot download.
import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"
import { QueryDownloadMenu } from "./query-download-menu"

afterEach(() => {
  cleanup()
  mock.restore()
})

function renderMenu(over: Partial<React.ComponentProps<typeof QueryDownloadMenu>> = {}) {
  const props: React.ComponentProps<typeof QueryDownloadMenu> = {
    resultId: "q-1",
    engine: "clickhouse",
    busy: null,
    onDownload: () => {},
    ...over,
  }
  render(<QueryDownloadMenu {...props} />)
  return screen.getByRole("button", { name: /Download/ })
}

describe("the download menu of a result", () => {
  it("offers the four file kinds", async () => {
    renderMenu()
    fireEvent.click(screen.getByRole("button", { name: /Download/ }))
    for (const label of ["CSV", "TSV", "Excel", "Parquet"]) {
      expect(await screen.findByRole("menuitem", { name: label })).toBeTruthy()
    }
  })

  it("passes the file kind the person picked on", async () => {
    const picked: string[] = []
    renderMenu({ onDownload: (format) => picked.push(format) })
    fireEvent.click(screen.getByRole("button", { name: /Download/ }))
    fireEvent.click(await screen.findByRole("menuitem", { name: "Parquet" }))
    expect(picked).toEqual(["parquet"])
  })

  it("is closed for a result the server cannot download, and says why", () => {
    const trigger = renderMenu({ engine: "trino" })
    expect((trigger as HTMLButtonElement).disabled).toBe(true)
    expect(trigger.getAttribute("title")).toBe("Downloads run on ClickHouse only.")
  })

  it("is closed when there is no result to download", () => {
    const trigger = renderMenu({ resultId: null })
    expect((trigger as HTMLButtonElement).disabled).toBe(true)
  })

  it("is closed while a download runs, and says that it is downloading", () => {
    const trigger = renderMenu({ busy: "csv" })
    expect((trigger as HTMLButtonElement).disabled).toBe(true)
    expect(trigger.textContent).toContain("Downloading…")
  })
})
