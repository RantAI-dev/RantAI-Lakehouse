import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"
import { RecordsDialog } from "./drill"
import type { RecordsRequest } from "./records"

const originalFetch = global.fetch
afterEach(() => {
  global.fetch = originalFetch
  cleanup()
})

const REQUEST: RecordsRequest = { title: "Bali", mart: "mart_x", column: "region", value: "Bali", filters: [] }

function json(body: unknown, status = 200) {
  return new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } })
}

/** Serves `total` rows of `{ n }`, 50 per page, and records every URL. */
function stubPages(total: number): string[] {
  const urls: string[] = []
  global.fetch = mock(async (input: RequestInfo | URL) => {
    const url = new URL(String(input), "http://console.example")
    urls.push(`${url.pathname}?${url.searchParams.toString()}`)
    const offset = Number(url.searchParams.get("offset") ?? 0)
    const count = Math.max(0, Math.min(50, total - offset))
    const rows = Array.from({ length: count }, (_, i) => ({ n: `row-${offset + i + 1}` }))
    return json({ columns: ["n"], rows, total, limit: 50, offset, filtersSkipped: [] })
  }) as unknown as typeof fetch
  return urls
}

describe("RecordsDialog", () => {
  it("shows the first page with its range and total, and pages forward and back", async () => {
    const urls = stubPages(58)
    render(<RecordsDialog request={REQUEST} onClose={() => {}} />)

    await screen.findByText("1–50 of 58")
    expect(screen.getByText("row-1")).toBeTruthy()
    expect((screen.getByRole("button", { name: /Previous/ }) as HTMLButtonElement).disabled).toBe(true)

    fireEvent.click(screen.getByRole("button", { name: /Next/ }))
    await screen.findByText("51–58 of 58")
    expect(screen.getByText("row-58")).toBeTruthy()
    // The last page is short and has no next page.
    expect((screen.getByRole("button", { name: /Next/ }) as HTMLButtonElement).disabled).toBe(true)
    expect(urls[1]).toContain("offset=50")

    fireEvent.click(screen.getByRole("button", { name: /Previous/ }))
    await screen.findByText("1–50 of 58")
    expect(urls[2]).toContain("offset=0")
  })

  it("says so when there are no records, with a total of zero", async () => {
    stubPages(0)
    render(<RecordsDialog request={REQUEST} onClose={() => {}} />)

    await screen.findByText("No records found.")
    expect(screen.getByText("0 of 0")).toBeTruthy()
  })

  it("shows the server's sentence and its reference, not the upstream's text", async () => {
    global.fetch = mock(async () => json({ error: "The dashboard query failed. Reference: ee0f295936" }, 422)) as unknown as typeof fetch
    render(<RecordsDialog request={REQUEST} onClose={() => {}} />)

    await waitFor(() => expect(screen.getByRole("alert").textContent).toContain("The dashboard query failed."))
    expect(screen.getByText("ee0f295936")).toBeTruthy()
    expect(screen.queryByText("No records found.")).toBeNull()
  })

  it("lists a SQL source's rows by the source id", async () => {
    const urls = stubPages(3)
    render(<RecordsDialog request={{ title: "Tile", mart: "", sqlSource: "s_1", filters: [] }} onClose={() => {}} />)

    await screen.findByText("1–3 of 3")
    expect(urls[0]).toContain("sqlSource=s_1")
    expect(urls[0]).not.toContain("column=")
  })

  it("renders nothing while there is no request", () => {
    stubPages(1)
    render(<RecordsDialog request={null} onClose={() => {}} />)
    expect(screen.queryByText(/Records/)).toBeNull()
  })
})
