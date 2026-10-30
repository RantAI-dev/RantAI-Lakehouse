import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it } from "bun:test"
import { ImportDashboardDialog } from "./import-dashboard-dialog"
import { initialMapping, readDashboardFile, MAX_FILE_BYTES, type ImportPreview } from "./dashboard-file"

const originalFetch = global.fetch
type Call = { url: string; body: Record<string, unknown> }

const preview: ImportPreview = {
  name: "Quarterly sales",
  charts: [
    { id: "a", title: "By region", kind: "bar", tables: ["mart_a"], readsSqlSource: false },
    { id: "b", title: "From SQL", kind: "bar", tables: [], readsSqlSource: true },
  ],
  tables: [
    { name: "mart_a", columns: ["region"], suggested: "mart_a", missingColumns: ["amount"] },
    { name: "mart_z", columns: [], suggested: null, missingColumns: [] },
  ],
  readableTables: ["mart_a", "gold_a"],
  canImportSql: false,
}

function serve(importBody: unknown = null) {
  const calls: Call[] = []
  global.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input)
    calls.push({ url, body: init?.body ? JSON.parse(String(init.body)) : {} })
    const body = url.endsWith("/preview") ? preview : importBody
    const status = importBody === "refuse" && !url.endsWith("/preview") ? 400 : 200
    return new Response(JSON.stringify(status === 400 ? { error: "That is not a dashboard file." } : body), { status })
  }) as unknown as typeof fetch
  return calls
}

function open() {
  render(<ImportDashboardDialog open onOpenChange={() => {}} folders={[]} />)
}

async function choose(content: string) {
  const input = screen.getByLabelText("Dashboard file") as HTMLInputElement
  const file = new File([content], "sales.rantai-dashboard.json", { type: "application/json" })
  Object.defineProperty(input, "files", { value: [file], configurable: true })
  fireEvent.change(input)
}

beforeEach(() => cleanup())
afterEach(() => { global.fetch = originalFetch; cleanup() })

describe("the import dialog", () => {
  it("previews the file, starts the mapping from the suggestions, and imports what was confirmed", async () => {
    const calls = serve({ board: { id: "b_new", name: "Quarterly sales" }, imported: [{ id: "u_1", title: "By region" }], leftOut: [{ title: "From SQL", reason: "It reads a SQL source, which needs the dashboard SQL permission." }] })
    open()
    await choose(JSON.stringify({ kind: "rantai-dashboard", version: 1 }))
    await screen.findByText("Quarterly sales")
    expect(calls[0].url).toBe("/api/dashboard/import/preview")
    // The suggested table is chosen, the other has none, and the missing column is said.
    expect(screen.getByLabelText("Table here for mart_a").textContent).toContain("mart_a")
    expect(screen.getByLabelText("Table here for mart_z").textContent).toContain("None")
    expect(screen.getByText(/Missing here: amount/)).toBeTruthy()
    expect(screen.getByRole("status").textContent).toContain("dashboard SQL permission")
    fireEvent.click(screen.getByRole("button", { name: "Import" }))
    await screen.findByTestId("import-result")
    expect(calls[1].url).toBe("/api/dashboard/import")
    expect(calls[1].body).toMatchObject({ mapping: { mart_a: "mart_a" }, folderId: "" })
    expect(screen.getByText(/was made with 1 of 2 charts/)).toBeTruthy()
    expect(screen.getByText(/It reads a SQL source/)).toBeTruthy()
  })

  it("shows the server's sentence for a file it refuses and creates nothing", async () => {
    const calls = serve("refuse")
    open()
    await choose("{}")
    await screen.findByText("Quarterly sales")
    fireEvent.click(screen.getByRole("button", { name: "Import" }))
    await waitFor(() => expect(screen.getByRole("alert").textContent).toContain("That is not a dashboard file."))
    expect(calls.length).toBe(2)
    expect(screen.queryByTestId("import-result")).toBeNull()
  })

  it("refuses a file that is not JSON or is over 1 MB before sending it", async () => {
    const calls = serve()
    open()
    await choose("not json")
    await waitFor(() => expect(screen.getByRole("alert").textContent).toContain("That is not a dashboard file."))
    expect(calls.length).toBe(0)
    await expect(readDashboardFile(new File(["x".repeat(MAX_FILE_BYTES + 1)], "big.json"))).rejects.toThrow("That file is larger than 1 MB.")
  })

  it("starts the mapping from the suggestions only", () => {
    expect(initialMapping(preview)).toEqual({ mart_a: "mart_a" })
  })
})
