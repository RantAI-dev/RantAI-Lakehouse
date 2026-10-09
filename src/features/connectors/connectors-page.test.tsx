// Sources keeps the Connectors tab and gains "Uploaded files" and an
// "Upload file" button (plan T11). The open tab is `?tab=uploads`. A connector
// opens on its own page, `/connectors/<id>`, not in a side sheet (connector
// page plan T2).
const url = { search: "" }
const pushed: string[] = []
mock.module("next/navigation", () => ({
  usePathname: () => "/connectors",
  useSearchParams: () => new URLSearchParams(url.search),
  useRouter: () => ({
    push: (to: string) => {
      pushed.push(to)
    },
    replace: () => {},
    refresh: () => {},
    back: () => {},
    forward: () => {},
    prefetch: () => {},
  }),
}))

import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react"
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
  pushed.length = 0
  mock.restore()
  cleanup()
})

function json(body: unknown, status = 200) {
  return new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } })
}

const CONNECTORS = [
  {
    id: "conn-a",
    name: "db demo",
    type: "PostgreSQL",
    direction: "source",
    health: "healthy",
    environment: "production",
    tenant: "Acme Co",
    lastTestAt: null,
    lastActivityAt: null,
    lastRunSuccessAt: null,
    lastRunFailureAt: null,
    failureStreak: 0,
    schemaChangePolicy: "apply_non_breaking",
    pausedReason: null,
    pausedAt: null,
    capabilities: [],
    owner: "admin",
  },
  {
    id: "conn-b",
    name: "events",
    type: "Kafka",
    direction: "sink",
    health: "unknown",
    environment: "staging",
    tenant: "Acme Co",
    lastTestAt: null,
    lastActivityAt: null,
    lastRunSuccessAt: null,
    lastRunFailureAt: null,
    failureStreak: 0,
    schemaChangePolicy: "apply_non_breaking",
    pausedReason: null,
    pausedAt: null,
    capabilities: [],
    owner: "admin",
  },
]

function stubFetch(connectors: unknown[] = []): string[] {
  const urls: string[] = []
  global.fetch = mock(async (input: RequestInfo | URL) => {
    const target = String(input)
    urls.push(target)
    if (target === "/api/connectors") return json(connectors)
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
  it("describes only what the picker offers, with no SaaS or federation promise (SRC-6 F7)", async () => {
    stubFetch()
    renderPage()
    expect(
      await screen.findByText(
        "Databases, change capture, object storage, files, REST APIs and message topics. Data enters the platform here before processing."
      )
    ).toBeDefined()
    expect(screen.queryByText(/SaaS/)).toBeNull()
    expect(screen.queryByText(/federation/)).toBeNull()
  })

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

describe("ConnectorsPage list", () => {
  function rowOf(name: string): HTMLElement {
    return screen.getByRole("link", { name }).closest("tr") as HTMLElement
  }

  it("links each connector's name to its own page", async () => {
    stubFetch(CONNECTORS)
    renderPage()
    expect((await screen.findByRole("link", { name: "db demo" })).getAttribute("href")).toBe("/connectors/conn-a")
    expect(screen.getByRole("link", { name: "events" }).getAttribute("href")).toBe("/connectors/conn-b")
    // The name is a real link now, not a button that opened a sheet.
    expect(screen.queryByRole("button", { name: "db demo" })).toBeNull()
    expect(screen.queryByRole("dialog")).toBeNull()
  })

  it("opens the page on a press anywhere on the row, and no sheet", async () => {
    stubFetch(CONNECTORS)
    renderPage()
    await screen.findByRole("link", { name: "events" })

    fireEvent.click(within(rowOf("events")).getByText("staging"))
    expect(pushed).toEqual(["/connectors/conn-b"])
    fireEvent.click(within(rowOf("db demo")).getByText("production"))
    expect(pushed).toEqual(["/connectors/conn-b", "/connectors/conn-a"])
    expect(screen.queryByRole("dialog")).toBeNull()
  })

  it("leaves the row's own controls to themselves: the name link does not also press the row", async () => {
    stubFetch(CONNECTORS)
    renderPage()
    fireEvent.click(await screen.findByRole("link", { name: "db demo" }))
    expect(pushed).toEqual([])
  })

  it("keeps the actions menu from pressing the row, and offers View details as a link", async () => {
    stubFetch(CONNECTORS)
    renderPage()
    await screen.findByRole("link", { name: "db demo" })
    fireEvent.click(within(rowOf("db demo")).getByRole("button", { name: "Connector actions" }))
    expect(pushed).toEqual([])

    const details = await screen.findByRole("menuitem", { name: "View details" })
    expect(details.getAttribute("href")).toBe("/connectors/conn-a")
    expect(screen.getByRole("menuitem", { name: "Create pipeline" }).getAttribute("href")).toBe(
      "/pipelines/create?connectorId=conn-a"
    )
    fireEvent.click(details)
    expect(pushed).toEqual([])
  })
})

// SRC-7 task 9: run health from the run reports, not from a manual Test.
describe("ConnectorsPage run health", () => {
  function rowOf(name: string): HTMLElement {
    return screen.getByRole("link", { name }).closest("tr") as HTMLElement
  }

  const failing = {
    ...CONNECTORS[0],
    health: "degraded",
    lastRunSuccessAt: "2026-10-01T02:00:00Z",
    lastRunFailureAt: "2026-10-02T02:00:00Z",
    failureStreak: 2,
  }
  const recovered = {
    ...CONNECTORS[1],
    health: "healthy",
    lastRunSuccessAt: "2026-10-03T02:00:00Z",
    lastRunFailureAt: "2026-10-02T02:00:00Z",
    failureStreak: 0,
    schemaChangePolicy: "apply_non_breaking",
    pausedReason: null,
    pausedAt: null,
  }

  it("shows the later of the last success and the last failure, and how many failed in a row", async () => {
    stubFetch([failing, recovered])
    renderPage()
    await screen.findByRole("link", { name: "db demo" })
    const bad = within(rowOf("db demo"))
    expect(bad.getByText("Failed")).toBeDefined()
    expect(bad.getByText("2 failed in a row")).toBeDefined()
    const good = within(rowOf("events"))
    expect(good.getByText("Succeeded")).toBeDefined()
    expect(good.queryByText(/failed in a row/)).toBeNull()
  })

  it("says there are no runs yet when a connector has neither a success nor a failure", async () => {
    stubFetch([CONNECTORS[0]])
    renderPage()
    await screen.findByRole("link", { name: "db demo" })
    expect(within(rowOf("db demo")).getByText("No runs yet")).toBeDefined()
  })
})
