import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"
import type { ChartClick } from "@/services/clients/bi-store"
import { ClickSetting } from "./click-setting"

const originalFetch = global.fetch
afterEach(() => {
  global.fetch = originalFetch
  cleanup()
})

const BOARDS = [{ id: "b_1", name: "Sales" }]

function stubColumns(columns: string[]): string[] {
  const urls: string[] = []
  global.fetch = mock(async (input: RequestInfo | URL) => {
    urls.push(String(input))
    return new Response(JSON.stringify({ filterFields: columns.map((column) => ({ column, kind: "text", tiles: 1 })) }), {
      status: 200, headers: { "Content-Type": "application/json" },
    })
  }) as unknown as typeof fetch
  return urls
}

describe("ClickSetting", () => {
  it("shows only the control while the drill menu is chosen", () => {
    render(<ClickSetting value={undefined} onChange={() => {}} boards={BOARDS} />)
    expect(screen.getByText("Drill menu")).toBeTruthy()
    expect(screen.queryByLabelText("URL")).toBeNull()
  })

  it("passes a typed URL on, with one short line about the value", () => {
    const seen: (ChartClick | undefined)[] = []
    render(<ClickSetting value={{ kind: "url", url: "" }} onChange={(c) => seen.push(c)} boards={BOARDS} />)
    expect(screen.getByText(/is replaced by the clicked value/)).toBeTruthy()

    fireEvent.change(screen.getByLabelText("URL"), { target: { value: "https://example.com/?q={value}" } })
    expect(seen).toEqual([{ kind: "url", url: "https://example.com/?q={value}" }])
  })

  it("refuses a javascript: URL in plain words as soon as it is typed", () => {
    render(<ClickSetting value={{ kind: "url", url: "javascript:alert(1)" }} onChange={() => {}} boards={BOARDS} />)
    expect(screen.getByRole("alert").textContent).toContain("https://")
  })

  it("offers the destination dashboard's own columns and says when the saved board is gone", async () => {
    const urls = stubColumns(["kab", "visitors"])
    render(<ClickSetting value={{ kind: "dashboard", board: "b_gone", column: "kab" }} onChange={() => {}} boards={BOARDS} />)

    expect(screen.getByText("b_gone (no longer exists)")).toBeTruthy()
    await waitFor(() => expect(urls[0]).toContain("board=b_gone"))
  })
})
