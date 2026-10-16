import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"
import { ValueGroupDialog } from "./value-group-dialog"
import type { CalcField } from "@/services/contracts/calc-fields"

const originalFetch = global.fetch
afterEach(() => {
  global.fetch = originalFetch
  cleanup()
})

const json = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } })
const FIELD = { id: "f_1", sourceKind: "mart", sourceId: "m", name: "region", formula: "", level: "group", type: "text", createdBy: "" } as CalcField

/** Serves a value list for any column and records every other request. */
function stub(values = ["Jawa Barat", "Jawa Timur", "Aceh"]) {
  const calls: { url: string; init?: RequestInit }[] = []
  global.fetch = mock(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input)
    calls.push({ url, init })
    if (url.includes("/api/dashboard/values")) return json({ values, truncated: false })
    return json({ ok: true, field: FIELD })
  }) as unknown as typeof fetch
  return calls
}

describe("ValueGroupDialog", () => {
  it("gathers chosen values under a name and saves the group on the source", async () => {
    const calls = stub()
    const onSaved = mock<(f: CalcField) => void>(() => {})
    render(<ValueGroupDialog open onOpenChange={() => {}} source={{ mart: "m" }} textColumns={["provinsi"]} onSaved={onSaved} />)
    fireEvent.change(screen.getByLabelText("Name"), { target: { value: "region" } })
    // The column is picked from the select; the list of values follows it.
    fireEvent.click(screen.getByLabelText("Column"))
    fireEvent.click(await screen.findByText("provinsi"))
    await waitFor(() => expect(screen.getByLabelText("Jawa Barat")).toBeTruthy())
    fireEvent.change(screen.getByLabelText("Name of group 1"), { target: { value: "Java" } })
    fireEvent.click(screen.getByLabelText("Jawa Barat"))
    fireEvent.click(screen.getByLabelText("Jawa Timur"))
    const create = screen.getByText("Create") as HTMLButtonElement
    await waitFor(() => expect(create.disabled).toBe(false))
    fireEvent.click(create)
    await waitFor(() => expect(onSaved).toHaveBeenCalled())
    const post = calls.find((c) => c.init?.method === "POST")
    expect(JSON.parse(String(post?.init?.body))).toEqual({
      mart: "m", name: "region",
      group: { column: "provinsi", rest: "keep", groups: [{ name: "Java", values: ["Jawa Barat", "Jawa Timur"] }] },
    })
  })

  it("keeps a value in one group only and says what is still needed", async () => {
    stub()
    const existing = { ...FIELD, formula: JSON.stringify({ column: "provinsi", rest: "other", groups: [{ name: "Java", values: ["Jawa Barat"] }, { name: "Sumatra", values: [] }] }) }
    render(<ValueGroupDialog open onOpenChange={() => {}} source={{ mart: "m" }} textColumns={["provinsi"]} existing={existing} onSaved={() => {}} />)
    await waitFor(() => expect(screen.getByLabelText("Jawa Barat")).toBeTruthy())
    expect(screen.getByText(/Still needed: values in every group/)).toBeTruthy()
    // Group 1 holds Jawa Barat; with group 2 active it is shown as taken and cannot be moved by a click.
    fireEvent.focus(screen.getByLabelText("Name of group 2"))
    await waitFor(() => expect((screen.getByLabelText("Jawa Barat") as HTMLInputElement).getAttribute("aria-disabled") ?? (screen.getByLabelText("Jawa Barat") as HTMLInputElement).disabled).toBeTruthy())
  })

  it("shows the server's plain sentence when saving fails", async () => {
    global.fetch = mock(async (input: RequestInfo | URL) => {
      if (String(input).includes("/values")) return json({ values: ["a"], truncated: false })
      return json({ error: "You do not have permission to do this." }, 403)
    }) as unknown as typeof fetch
    const existing = { ...FIELD, formula: JSON.stringify({ column: "provinsi", rest: "keep", groups: [{ name: "Java", values: ["a"] }] }) }
    render(<ValueGroupDialog open onOpenChange={() => {}} source={{ mart: "m" }} textColumns={["provinsi"]} existing={existing} onSaved={() => {}} />)
    fireEvent.click(await screen.findByText("Save"))
    expect((await screen.findByRole("alert")).textContent).toContain("permission")
  })
})
