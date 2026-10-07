// The "Uploaded files" tab (plan T11): the columns the plan names, "Not
// measured" for a row count nobody measured, Load only where a load may
// start, Delete behind a confirmation that says the table stays and disabled
// while loading, polling only while a row is loading, and the API's refusal
// shown through the existing error state.
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"
import type { Upload } from "@/services/contracts/uploads"
import { UploadsPanel } from "./uploads-panel"

const originalFetch = global.fetch

afterEach(() => {
  global.fetch = originalFetch
  cleanup()
})

function json(body: unknown, status = 200) {
  return new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } })
}

function upload(over: Partial<Upload> = {}): Upload {
  return {
    id: "up-1",
    originalFilename: "stock.csv",
    sizeBytes: 2048,
    uploadedBy: "Ana",
    status: "uploaded",
    createdAt: "2026-10-02T10:00:00Z",
    updatedAt: "2026-10-02T10:00:00Z",
    ...over,
  }
}

type Call = { url: string; method: string }

function stubFetch(list: (n: number) => Upload[] | Response, remove?: () => Response): Call[] {
  const calls: Call[] = []
  let lists = 0
  global.fetch = mock(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input)
    const method = init?.method ?? "GET"
    calls.push({ url, method })
    if (url === "/api/uploads" && method === "GET") {
      lists += 1
      const out = list(lists)
      return out instanceof Response ? out : json(out)
    }
    if (method === "DELETE" && remove) return remove()
    throw new Error(`unexpected fetch: ${method} ${url}`)
  }) as unknown as typeof fetch
  return calls
}

function rowOf(name: string) {
  return screen.getByText(name).closest("tr") as HTMLElement
}

describe("UploadsPanel", () => {
  it("lists each upload with its table as a link, and Not measured where there is no row count", async () => {
    stubFetch(() => [
      upload({ id: "up-1", originalFilename: "loaded.csv", status: "ingested", bronzeTable: "stock", assetId: "stock", rows: 120 }),
      upload({ id: "up-2", originalFilename: "failed.csv", status: "failed", error: "The load into the table failed." }),
      upload({ id: "up-3", originalFilename: "fresh.csv" }),
    ])
    render(<UploadsPanel />)
    await screen.findByText("loaded.csv")

    for (const heading of ["File", "Size", "Status", "Table", "Rows", "Uploaded by", "Uploaded"]) {
      expect(screen.getByRole("columnheader", { name: heading })).toBeDefined()
    }
    const loaded = within(rowOf("loaded.csv"))
    expect(loaded.getByText("Loaded")).toBeDefined()
    expect(loaded.getByText("120")).toBeDefined()
    expect(loaded.getByRole("link", { name: "stock" }).getAttribute("href")).toBe("/data/assets/stock")
    expect(loaded.getByText("2.0 KB")).toBeDefined()
    expect(loaded.getByText("Ana")).toBeDefined()

    const failed = within(rowOf("failed.csv"))
    expect(failed.getByText("Not measured")).toBeDefined()
    expect(failed.queryByText("0")).toBeNull()
    expect(failed.getByText("The load into the table failed.")).toBeDefined()
    expect(within(rowOf("fresh.csv")).getByText("Not measured")).toBeDefined()
  })

  it("offers Load for an uploaded or failed file, opening the upload page on it, and for no other", async () => {
    stubFetch(() => [
      upload({ id: "up-1", originalFilename: "a.csv", status: "uploaded" }),
      upload({ id: "up-2", originalFilename: "b.csv", status: "failed" }),
      upload({ id: "up-3", originalFilename: "c.csv", status: "ingested", bronzeTable: "c", assetId: "c" }),
      upload({ id: "up-4", originalFilename: "d.csv", status: "ingesting" }),
    ])
    render(<UploadsPanel />)
    await screen.findByText("a.csv")
    expect(within(rowOf("a.csv")).getByText("Load").closest("a")?.getAttribute("href")).toBe("/connectors/upload?id=up-1")
    expect(within(rowOf("b.csv")).getByText("Load").closest("a")?.getAttribute("href")).toBe("/connectors/upload?id=up-2")
    expect(within(rowOf("c.csv")).queryByText("Load")).toBeNull()
    expect(within(rowOf("d.csv")).queryByText("Load")).toBeNull()
  })

  it("asks first, says the table stays, and deletes only after the confirmation", async () => {
    let removed = false
    const calls = stubFetch(
      () => (removed ? [] : [upload({ status: "ingested", bronzeTable: "stock", assetId: "stock", rows: 3 })]),
      () => {
        removed = true
        return new Response(null, { status: 204 })
      }
    )
    render(<UploadsPanel />)
    await screen.findByText("stock.csv")

    fireEvent.click(screen.getByRole("button", { name: "Delete stock.csv" }))
    expect(await screen.findByText("Delete stock.csv?")).toBeDefined()
    expect(screen.getByText("The table stock stays, with its rows.")).toBeDefined()
    expect(calls.some((c) => c.method === "DELETE")).toBe(false)

    fireEvent.click(screen.getByRole("button", { name: "Delete" }))
    await waitFor(() => expect(calls.filter((c) => c.method === "DELETE").length).toBe(1))
    expect(calls.find((c) => c.method === "DELETE")?.url).toBe("/api/uploads/up-1")
    expect(await screen.findByText("No uploaded files")).toBeDefined()
  })

  it("does not delete when the confirmation is cancelled, and shows a refusal in the dialog", async () => {
    const sentence = "This upload is being loaded, so it cannot be deleted yet."
    const calls = stubFetch(
      () => [upload()],
      () => json({ error: sentence }, 409)
    )
    render(<UploadsPanel />)
    await screen.findByText("stock.csv")

    fireEvent.click(screen.getByRole("button", { name: "Delete stock.csv" }))
    await screen.findByText("Delete stock.csv?")
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }))
    await waitFor(() => expect(screen.queryByText("Delete stock.csv?")).toBeNull())
    expect(calls.some((c) => c.method === "DELETE")).toBe(false)

    fireEvent.click(screen.getByRole("button", { name: "Delete stock.csv" }))
    await screen.findByText("Delete stock.csv?")
    fireEvent.click(screen.getByRole("button", { name: "Delete" }))
    expect((await screen.findByRole("alert")).textContent).toBe(sentence)
    expect(screen.getByText("stock.csv")).toBeDefined()
  })

  it("disables Delete while a row is loading", async () => {
    stubFetch(() => [upload({ status: "ingesting" })])
    render(<UploadsPanel pollMs={1000} />)
    await screen.findByText("stock.csv")
    expect((screen.getByRole("button", { name: "Delete stock.csv" }) as HTMLButtonElement).disabled).toBe(true)
  })

  it("refreshes while a row is loading and stops when none is", async () => {
    const calls = stubFetch((n) => [n < 3 ? upload({ status: "ingesting" }) : upload({ status: "ingested", bronzeTable: "stock", assetId: "stock", rows: 3 })])
    render(<UploadsPanel pollMs={20} />)
    await screen.findByText("Loaded")
    const settled = calls.length
    expect(settled).toBe(3)
    await new Promise((resolve) => setTimeout(resolve, 120))
    expect(calls.length).toBe(settled)
  })

  it("does not refresh a list that has nothing loading", async () => {
    const calls = stubFetch(() => [upload()])
    render(<UploadsPanel pollMs={20} />)
    await screen.findByText("stock.csv")
    await new Promise((resolve) => setTimeout(resolve, 120))
    expect(calls.length).toBe(1)
  })

  it("stops refreshing when the tab is left", async () => {
    const calls = stubFetch(() => [upload({ status: "ingesting" })])
    const view = render(<UploadsPanel pollMs={20} />)
    await screen.findByText("stock.csv")
    view.unmount()
    const after = calls.length
    await new Promise((resolve) => setTimeout(resolve, 120))
    expect(calls.length).toBe(after)
  })

  it("shows the upload button in the empty state", async () => {
    stubFetch(() => [])
    render(<UploadsPanel />)
    expect(await screen.findByText("No uploaded files")).toBeDefined()
    expect(screen.getByText("Upload file").closest("a")?.getAttribute("href")).toBe("/connectors/upload")
  })

  it("says a missing permission through the existing state, never a blank page", async () => {
    stubFetch(() => json({ error: "You do not have permission to do this." }, 403))
    render(<UploadsPanel />)
    expect(await screen.findByText("You don't have access")).toBeDefined()
  })

  it("shows the API's sentence for an unavailable list, with Retry", async () => {
    stubFetch(() => json({ error: "database error" }, 500))
    render(<UploadsPanel />)
    expect(await screen.findByText("Service unavailable")).toBeDefined()
    expect(screen.getByText(/database error/)).toBeDefined()
    expect(screen.getByRole("button", { name: "Retry" })).toBeDefined()
  })
})
