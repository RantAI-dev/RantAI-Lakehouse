import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"

import { VersionHistoryDialog } from "./version-history-dialog"

const originalFetch = global.fetch

afterEach(() => {
  global.fetch = originalFetch
  cleanup()
})

const history = {
  keep: 15,
  versions: [
    { version: 3, author: "Bob", at: "2026-10-14 10:00:00", summary: "2 charts added", charts: 3 },
    { version: 2, author: "Ann", at: "2026-10-14 09:00:00", summary: "Renamed", charts: 1 },
  ],
}

/** Answers the history read, then whatever the revert is given. */
function stub(revert: Response | null = null) {
  const calls: { url: string; body?: string }[] = []
  global.fetch = mock(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input)
    calls.push({ url, body: init?.body as string | undefined })
    if (url.includes("/versions/revert")) return revert ?? new Response("{}", { status: 200 })
    return new Response(JSON.stringify(history), { status: 200, headers: { "Content-Type": "application/json" } })
  }) as unknown as typeof fetch
  return calls
}

function open(canWrite = true, onReverted = () => {}) {
  render(<VersionHistoryDialog boardId="b1" canWrite={canWrite} onOpenChange={() => {}} onReverted={onReverted} />)
}

describe("VersionHistoryDialog", () => {
  it("lists who changed what and marks the newest as current", async () => {
    stub()
    open()
    expect(await screen.findByText("2 charts added")).toBeTruthy()
    expect(screen.getByText("Renamed")).toBeTruthy()
    expect(screen.getByText("Current")).toBeTruthy()
    expect(screen.getByText(/Ann · /)).toBeTruthy()
  })

  it("offers going back only to someone who may write, and only for older versions", async () => {
    stub()
    open(false)
    await screen.findByText("Renamed")
    expect(screen.queryByText("Go back")).toBeNull()
    cleanup()
    stub()
    open(true)
    await screen.findByText("Renamed")
    expect(screen.getAllByText("Go back")).toHaveLength(1)
  })

  it("asks before going back, then restores and reports it", async () => {
    const calls = stub(new Response(JSON.stringify({ ok: true, restoredFrom: 2, version: 4, chartsRemoved: 0, chartsNotBrought: [] }), { status: 200 }))
    const onReverted = mock(() => {})
    open(true, onReverted)
    fireEvent.click(await screen.findByText("Go back"))
    expect(calls.some((c) => c.url.includes("/revert"))).toBe(false)
    fireEvent.click(screen.getByText("Restore"))
    await waitFor(() => expect(onReverted).toHaveBeenCalledTimes(1))
    const revert = calls.find((c) => c.url.includes("/versions/revert"))
    expect(JSON.parse(revert?.body ?? "{}")).toEqual({ board: "b1", version: 2 })
  })

  it("shows the server's sentence when the version no longer holds and changes nothing", async () => {
    stub(
      new Response(
        JSON.stringify({ error: 'Cannot go back to version 2: the chart "Hotels" no longer works (x). Nothing was changed.' }),
        { status: 422 },
      ),
    )
    const onReverted = mock(() => {})
    open(true, onReverted)
    fireEvent.click(await screen.findByText("Go back"))
    fireEvent.click(screen.getByText("Restore"))
    expect(await screen.findByRole("alert")).toBeTruthy()
    expect(screen.getByRole("alert").textContent).toContain("Nothing was changed.")
    expect(onReverted).not.toHaveBeenCalled()
  })
})
