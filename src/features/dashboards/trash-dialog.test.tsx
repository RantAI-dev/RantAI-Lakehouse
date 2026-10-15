import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"

import { TrashDialog } from "./trash-dialog"

const originalFetch = global.fetch

afterEach(() => {
  global.fetch = originalFetch
  cleanup()
})

const inDays = (n: number) => new Date(Date.now() + n * 86_400_000).toISOString().slice(0, 19).replace("T", " ")

const trash = {
  retentionDays: 30,
  items: [
    { kind: "board", id: "b1", name: "Old board", board: "", boardName: "", boardState: "", deletedBy: "Ann", deletedAt: "t", expiresAt: inDays(12), personal: false },
    { kind: "chart", id: "u1", name: "Lost chart", board: "b9", boardName: "", boardState: "gone", deletedBy: "Bob", deletedAt: "t", expiresAt: inDays(3), personal: false },
  ],
}

function stub(restore: Response | null = null) {
  const calls: { url: string; body?: string }[] = []
  global.fetch = mock(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input)
    calls.push({ url, body: init?.body as string | undefined })
    if (url.includes("/trash/restore")) return restore ?? new Response(JSON.stringify({ ok: true, restored: { kind: "board", id: "b1", charts: 2, boardState: "" } }), { status: 200 })
    return new Response(JSON.stringify(trash), { status: 200, headers: { "Content-Type": "application/json" } })
  }) as unknown as typeof fetch
  return calls
}

describe("TrashDialog", () => {
  it("lists what was deleted, by whom, how long is left and where a chart will land", async () => {
    stub()
    render(<TrashDialog canWrite onOpenChange={() => {}} onRestored={() => {}} />)
    expect(await screen.findByText("Old board")).toBeTruthy()
    expect(screen.getByText(/With its charts · by Ann · /)).toBeTruthy()
    expect(screen.getByText(/Its dashboard is gone: on no dashboard · by Bob · 3 days left/)).toBeTruthy()
  })

  it("shows a reader the list without the Restore buttons", async () => {
    stub()
    render(<TrashDialog canWrite={false} onOpenChange={() => {}} onRestored={() => {}} />)
    await screen.findByText("Old board")
    expect(screen.queryByText("Restore")).toBeNull()
  })

  it("restores one item and tells the page", async () => {
    const calls = stub()
    const onRestored = mock(() => {})
    render(<TrashDialog canWrite onOpenChange={() => {}} onRestored={onRestored} />)
    fireEvent.click((await screen.findAllByText("Restore"))[0])
    await waitFor(() => expect(onRestored).toHaveBeenCalledTimes(1))
    const call = calls.find((c) => c.url.includes("/trash/restore"))
    expect(JSON.parse(call?.body ?? "{}")).toEqual({ kind: "board", id: "b1" })
  })

  it("says the trash is empty", async () => {
    global.fetch = mock(async () => new Response(JSON.stringify({ items: [], retentionDays: 30 }), { status: 200 })) as unknown as typeof fetch
    render(<TrashDialog canWrite onOpenChange={() => {}} onRestored={() => {}} />)
    expect(await screen.findByText(/The trash is empty/)).toBeTruthy()
  })

  it("shows the refusal when the item is not in the trash any more", async () => {
    stub(new Response(JSON.stringify({ error: "That item is not in the trash." }), { status: 404 }))
    render(<TrashDialog canWrite onOpenChange={() => {}} onRestored={() => {}} />)
    fireEvent.click((await screen.findAllByText("Restore"))[0])
    expect((await screen.findByRole("alert")).textContent).toContain("not in the trash")
  })
})
