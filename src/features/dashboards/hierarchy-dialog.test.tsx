import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"
import { HierarchyDialog } from "./hierarchy-dialog"
import type { Hierarchy } from "@/services/contracts/hierarchies"

const originalFetch = global.fetch
afterEach(() => {
  global.fetch = originalFetch
  cleanup()
})

const json = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } })
const SAVED: Hierarchy = { id: "h_1", sourceKind: "mart", sourceId: "m", name: "Geo", levels: ["a", "b"], createdBy: "" }

describe("HierarchyDialog", () => {
  it("needs a name and two columns before it can be saved, then sends the levels in order", async () => {
    const calls: RequestInit[] = []
    global.fetch = mock(async (_url: RequestInfo | URL, init?: RequestInit) => {
      calls.push(init ?? {})
      return json({ ok: true, hierarchy: SAVED })
    }) as unknown as typeof fetch
    const onSaved = mock<(h: Hierarchy) => void>(() => {})
    render(<HierarchyDialog open onOpenChange={() => {}} source={{ mart: "m" }} columns={["provinsi", "kab_kota"]} onSaved={onSaved} />)
    expect((screen.getByText("Create") as HTMLButtonElement).disabled).toBe(true)
    expect(screen.getByText(/Still needed: a name, 2 to 6 columns/)).toBeTruthy()
    fireEvent.change(screen.getByLabelText("Name"), { target: { value: "Geo" } })
    fireEvent.click(screen.getByLabelText("Level 1"))
    fireEvent.click((await screen.findAllByText("provinsi")).at(-1) as HTMLElement)
    fireEvent.click(screen.getByLabelText("Level 2"))
    fireEvent.click((await screen.findAllByText("kab_kota")).at(-1) as HTMLElement)
    const create = screen.getByText("Create") as HTMLButtonElement
    await waitFor(() => expect(create.disabled).toBe(false))
    fireEvent.click(create)
    await waitFor(() => expect(onSaved).toHaveBeenCalled())
    expect(JSON.parse(String(calls[0].body))).toEqual({ mart: "m", name: "Geo", levels: ["provinsi", "kab_kota"] })
  })

  it("adds levels up to six and never fewer than two", () => {
    render(<HierarchyDialog open onOpenChange={() => {}} source={{ mart: "m" }} columns={["a", "b", "c"]} onSaved={() => {}} />)
    expect((screen.getByLabelText("Remove level 1") as HTMLButtonElement).disabled).toBe(true)
    for (let i = 0; i < 4; i += 1) fireEvent.click(screen.getByText("Add a level"))
    expect(screen.queryByText("Add a level")).toBeNull()
    expect(screen.getAllByLabelText(/^Level \d$/)).toHaveLength(6)
  })

  it("shows a refusal as the server's own sentence", async () => {
    global.fetch = mock(async () => json({ error: "You do not have permission to do this." }, 403)) as unknown as typeof fetch
    render(<HierarchyDialog open onOpenChange={() => {}} source={{ mart: "m" }} columns={["a", "b"]} existing={SAVED} onSaved={() => {}} />)
    fireEvent.click(screen.getByText("Save"))
    expect((await screen.findByRole("alert")).textContent).toContain("permission")
  })
})
