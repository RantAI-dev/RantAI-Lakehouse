// Sources keeps the Connectors tab as it was and gains "Uploaded files" and
// an "Upload file" button (plan T11). The open tab is `?tab=uploads`.
const url = { search: "" }
mock.module("next/navigation", () => ({
  usePathname: () => "/connectors",
  useSearchParams: () => new URLSearchParams(url.search),
  useRouter: () => ({
    push: () => {},
    replace: () => {},
    refresh: () => {},
    back: () => {},
    forward: () => {},
    prefetch: () => {},
  }),
}))

import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, mock, spyOn } from "bun:test"
import { TableProviders } from "@/components/app-shell/table-providers"
import { ConnectorsPage } from "./connectors-page"

const originalFetch = global.fetch

beforeEach(() => {
  spyOn(window.history, "replaceState").mockImplementation(() => {})
})

afterEach(() => {
  global.fetch = originalFetch
  url.search = ""
  mock.restore()
  cleanup()
})

function json(body: unknown, status = 200) {
  return new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } })
}

function stubFetch(): string[] {
  const urls: string[] = []
  global.fetch = mock(async (input: RequestInfo | URL) => {
    const target = String(input)
    urls.push(target)
    if (target === "/api/connectors") return json([])
    if (target === "/api/uploads") return json([])
    throw new Error(`unexpected fetch: ${target}`)
  }) as unknown as typeof fetch
  return urls
}

function renderPage() {
  return render(
    <TableProviders>
      <ConnectorsPage />
    </TableProviders>
  )
}

describe("ConnectorsPage tabs", () => {
  it("opens on Connectors, offers Upload file before New Connector, and does not read the uploads", async () => {
    const urls = stubFetch()
    renderPage()
    expect(await screen.findByText("No connectors")).toBeDefined()
    expect(screen.getByRole("tab", { name: "Connectors" }).getAttribute("aria-selected")).toBe("true")
    expect(screen.getByRole("tab", { name: "Uploaded files" }).getAttribute("aria-selected")).toBe("false")
    const upload = screen.getByText("Upload file").closest("a")
    const create = screen.getAllByText("New Connector")[0].closest("a")
    expect(upload?.getAttribute("href")).toBe("/connectors/upload")
    expect(create?.getAttribute("href")).toBe("/connectors/create")
    expect(upload?.compareDocumentPosition(create as Node)).toBe(Node.DOCUMENT_POSITION_FOLLOWING)
    expect(urls).not.toContain("/api/uploads")
  })

  it("opens on Uploaded files for ?tab=uploads", async () => {
    url.search = "?tab=uploads"
    stubFetch()
    renderPage()
    expect(await screen.findByText("No uploaded files")).toBeDefined()
    expect(screen.getByRole("tab", { name: "Uploaded files" }).getAttribute("aria-selected")).toBe("true")
  })

  it("switches tabs and writes the tab to the address", async () => {
    stubFetch()
    renderPage()
    await screen.findByText("No connectors")
    fireEvent.click(screen.getByRole("tab", { name: "Uploaded files" }))
    expect(await screen.findByText("No uploaded files")).toBeDefined()
    expect(window.history.replaceState).toHaveBeenLastCalledWith(null, "", "/connectors?tab=uploads")

    fireEvent.click(screen.getByRole("tab", { name: "Connectors" }))
    await waitFor(() => expect(screen.getByRole("tab", { name: "Connectors" }).getAttribute("aria-selected")).toBe("true"))
    expect(window.history.replaceState).toHaveBeenLastCalledWith(null, "", "/connectors")
  })
})
