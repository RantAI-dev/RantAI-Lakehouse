// A connector has a page of its own at `/connectors/<id>` (plan
// `docs/superpowers/plans/2026-10-05-connector-detail-page.md`, T1): the body
// of the side sheet the Sources list used to open, with a header, the three
// tabs in `?tab=`, a way back to Sources and, after a delete, the way back
// too.
const url = { search: "" }
const pushed: string[] = []
mock.module("next/navigation", () => ({
  usePathname: () => "/connectors/conn-a",
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

import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, mock, spyOn } from "bun:test"
import { ConnectorDetailPage } from "./connector-detail-page"

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
  return new Response(body === null ? null : JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  })
}

const DETAIL = {
  id: "conn-a",
  name: "db demo",
  type: "PostgreSQL",
  direction: "source",
  health: "healthy",
  environment: "production",
  tenant: "Acme Co",
  lastTestAt: null,
  lastActivityAt: null,
  capabilities: [],
  owner: "admin",
  discoveredAssets: 0,
  discoveredSchemas: [],
  recentErrors: [],
  dependentPipelines: [] as { id: string; name: string; kind: string }[],
  auditEventId: "evt-1",
  credentialManaged: true,
  credentialKind: "password",
  credentialSecondaryKind: null,
  residency: "id-jakarta",
  tenantId: "t-acme",
}

const SPEC = {
  adapter: "sql",
  ingestMode: "batch",
  dial: { driver: "postgres", host: "192.168.18.205", port: 5432, database: "insurance_db", user: "insurer", sslMode: null },
  sourceObjects: [] as { name: string; target: string }[],
  scheduleCron: null,
  secretRefs: { primary: "file:/run/secrets/connector_managed_x_password", secondary: null },
  nextRunAt: null,
}

type Call = { url: string; method: string }
type Probe = { testedAt: string; ok: boolean; latencyMs: number | null; message: string }

/**
 * The reads the page and its three tabs make, and the two writes it can:
 * a test appends a row to the history the way `POST .../test` does server
 * side. `overrides` are keyed `"<METHOD> <path>"`.
 */
function stubFetch(
  overrides: Record<string, () => Response | Promise<Response>> = {},
  detail: Record<string, unknown> = DETAIL
): { calls: Call[]; count: (key: string) => number } {
  const calls: Call[] = []
  const probes: Probe[] = []
  global.fetch = mock(async (input: RequestInfo | URL, init?: RequestInit) => {
    const target = String(input).replace(/^https?:\/\/[^/]+/, "")
    const method = init?.method ?? "GET"
    calls.push({ url: target, method })
    const key = `${method} ${target}`
    if (overrides[key]) return overrides[key]()
    if (key === "GET /api/connectors/conn-a") return json(detail)
    if (key === "GET /api/connectors/conn-a/ingest-spec") return json(SPEC)
    if (key.startsWith("GET /api/connectors/conn-a/probe-history")) {
      return json({ results: key.endsWith("?limit=1") ? probes.slice(0, 1) : probes })
    }
    if (key === "GET /api/connectors/conn-a/ingest/runs") return json([])
    if (key === "GET /api/governance/ingest-runs?connectorId=conn-a") return json([])
    if (key === "POST /api/connectors/conn-a/test") {
      probes.unshift({ testedAt: "2026-10-05T01:00:00.000Z", ok: true, latencyMs: 5, message: "Connected via PostgreSQL" })
      return json({ ok: true, supported: true, latencyMs: 5, message: "Connected via PostgreSQL", testedAt: probes[0].testedAt })
    }
    if (key === "DELETE /api/connectors/conn-a") return new Response(null, { status: 204 })
    throw new Error(`unexpected fetch: ${key}`)
  }) as unknown as typeof fetch
  return { calls, count: (key) => calls.filter((c) => `${c.method} ${c.url}` === key).length }
}

/** Lets the reads a tab makes on mount finish inside `act`, not after the test. */
const settle = () =>
  act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 0))
  })

function selected(name: string): string | null {
  return screen.getByRole("tab", { name }).getAttribute("aria-selected")
}

describe("ConnectorDetailPage", () => {
  it("shows a skeleton and the way back to Sources while the connector loads", () => {
    stubFetch({ "GET /api/connectors/conn-a": () => new Promise<Response>(() => {}) })
    render(<ConnectorDetailPage connectorId="conn-a" />)
    expect(screen.getByRole("status", { name: "Loading" })).toBeDefined()
    expect(screen.getByRole("link", { name: "Sources" }).getAttribute("href")).toBe("/connectors")
  })

  it("says the connector was not found, in the API's words, with a way back", async () => {
    stubFetch({ "GET /api/connectors/conn-a": () => json({ error: "connector conn-a not found" }, 404) })
    render(<ConnectorDetailPage connectorId="conn-a" />)
    expect(await screen.findByText("Not found")).toBeDefined()
    expect(screen.getByText(/connector conn-a not found/)).toBeDefined()
    expect(screen.getByRole("link", { name: "Sources" }).getAttribute("href")).toBe("/connectors")
    expect(screen.queryByRole("tablist")).toBeNull()
  })

  it("keeps the way back when the read is refused for any other reason", async () => {
    stubFetch({ "GET /api/connectors/conn-a": () => json({ error: "tenant scope does not include this connector" }, 403) })
    render(<ConnectorDetailPage connectorId="conn-a" />)
    expect(await screen.findByText(/tenant scope does not include this connector/)).toBeDefined()
    expect(screen.getByRole("link", { name: "Sources" }).getAttribute("href")).toBe("/connectors")
    expect(screen.queryByRole("tablist")).toBeNull()
  })

  it("heads the page with the name, its type, health, direction and environment, and the actions", async () => {
    stubFetch()
    render(<ConnectorDetailPage connectorId="conn-a" />)
    const title = await screen.findByRole("heading", { level: 1, name: "db demo" })
    const badges = within(title.parentElement as HTMLElement)
    expect(badges.getByText("Healthy")).toBeDefined()
    expect(badges.getByText("Source")).toBeDefined()
    expect(badges.getByText("production")).toBeDefined()
    // The header block: the name's wrapper, the text column, then the block.
    const header = within(((title.parentElement as HTMLElement).parentElement as HTMLElement).parentElement as HTMLElement)
    expect(header.getByText("PostgreSQL")).toBeDefined()
    expect(header.getByRole("link", { name: "Sources" }).getAttribute("href")).toBe("/connectors")
    // `Button render={<Link />}` is an anchor with `role="button"`.
    expect(header.getByRole("button", { name: "Edit" }).getAttribute("href")).toBe("/connectors/conn-a/edit")
    expect(header.getByRole("button", { name: "Create pipeline" }).getAttribute("href")).toBe(
      "/pipelines/create?connectorId=conn-a"
    )
    expect(header.getByRole("button", { name: "Audit" }).getAttribute("href")).toBe("/audit?event=evt-1")
    expect(header.getByRole("button", { name: "Test connection" })).toBeDefined()
    expect((header.getByRole("button", { name: "Delete" }) as HTMLButtonElement).disabled).toBe(false)
  })

  it("opens on Overview, with the address left clean", async () => {
    stubFetch()
    render(<ConnectorDetailPage connectorId="conn-a" />)
    expect(await screen.findByText("No pipeline reads from this connector.")).toBeDefined()
    expect(screen.getAllByRole("tab").map((t) => t.textContent)).toEqual(["Overview", "Ingest", "Connection tests"])
    expect(selected("Overview")).toBe("true")
    expect(selected("Ingest")).toBe("false")
    expect(selected("Connection tests")).toBe("false")
    expect(window.history.replaceState).not.toHaveBeenCalled()
  })

  it("opens the Ingest tab for ?tab=ingest and the Connection tests tab for ?tab=tests", async () => {
    url.search = "?tab=ingest"
    stubFetch()
    const first = render(<ConnectorDetailPage connectorId="conn-a" />)
    expect(await screen.findByText("No tables selected yet. Find tables below and tick the ones to copy.")).toBeDefined()
    expect(selected("Ingest")).toBe("true")
    first.unmount()

    url.search = "?tab=tests"
    stubFetch()
    render(<ConnectorDetailPage connectorId="conn-a" />)
    expect(await screen.findByText("Not tested yet. Only tests this build can run are recorded.")).toBeDefined()
    expect(selected("Connection tests")).toBe("true")
  })

  it("falls back to Overview for a tab it does not have", async () => {
    url.search = "?tab=nope"
    stubFetch()
    render(<ConnectorDetailPage connectorId="conn-a" />)
    expect(await screen.findByText("No pipeline reads from this connector.")).toBeDefined()
    expect(selected("Overview")).toBe("true")
  })

  it("writes the open tab to the address, and clears it for Overview", async () => {
    stubFetch()
    render(<ConnectorDetailPage connectorId="conn-a" />)
    await screen.findByText("No pipeline reads from this connector.")

    fireEvent.click(screen.getByRole("tab", { name: "Ingest" }))
    expect(window.history.replaceState).toHaveBeenLastCalledWith(null, "", "/connectors/conn-a?tab=ingest")
    await screen.findByText("No tables selected yet. Find tables below and tick the ones to copy.")
    fireEvent.click(screen.getByRole("tab", { name: "Connection tests" }))
    expect(window.history.replaceState).toHaveBeenLastCalledWith(null, "", "/connectors/conn-a?tab=tests")
    await screen.findByText("Not tested yet. Only tests this build can run are recorded.")
    fireEvent.click(screen.getByRole("tab", { name: "Overview" }))
    expect(window.history.replaceState).toHaveBeenLastCalledWith(null, "", "/connectors/conn-a")
    await settle()
  })

  it("lets the Overview open the Ingest tab of the page", async () => {
    stubFetch()
    render(<ConnectorDetailPage connectorId="conn-a" />)
    fireEvent.click(await screen.findByRole("button", { name: "Manage" }))
    await waitFor(() => expect(selected("Ingest")).toBe("true"))
    expect(window.history.replaceState).toHaveBeenLastCalledWith(null, "", "/connectors/conn-a?tab=ingest")
  })

  it("keeps the Ingest tab mounted, so a table search typed there survives a look at another tab", async () => {
    stubFetch()
    render(<ConnectorDetailPage connectorId="conn-a" />)
    await screen.findByText("No pipeline reads from this connector.")
    fireEvent.click(screen.getByRole("tab", { name: "Ingest" }))
    const schema = (await screen.findByLabelText("Schema")) as HTMLInputElement
    expect(schema.value).toBe("public")
    fireEvent.change(schema, { target: { value: "sales" } })

    fireEvent.click(screen.getByRole("tab", { name: "Connection tests" }))
    await screen.findByText("Not tested yet. Only tests this build can run are recorded.")
    fireEvent.click(screen.getByRole("tab", { name: "Ingest" }))
    expect(((await screen.findByLabelText("Schema")) as HTMLInputElement).value).toBe("sales")
  })

  it("tests the connection, shows the result line, reloads the detail and lists the test in the history", async () => {
    // The second read of the detail is held, to look at the page while the
    // reload is still out.
    let reads = 0
    let release: (r: Response) => void = () => {}
    const { count } = stubFetch({
      "GET /api/connectors/conn-a": () =>
        ++reads === 1
          ? json(DETAIL)
          : new Promise<Response>((resolve) => {
              release = resolve
            }),
    })
    render(<ConnectorDetailPage connectorId="conn-a" />)
    await screen.findByText("No pipeline reads from this connector.")

    fireEvent.click(screen.getByRole("button", { name: "Test connection" }))
    expect(await screen.findByText("Connected via PostgreSQL · 5 ms")).toBeDefined()
    await waitFor(() => expect(count("POST /api/connectors/conn-a/test")).toBe(1))
    await waitFor(() => expect(reads).toBe(2))
    // The page stays on screen for the reload: no skeleton in its place,
    // which would also unmount the Ingest tab.
    expect(screen.queryByRole("status", { name: "Loading" })).toBeNull()
    expect(screen.getByRole("heading", { level: 1, name: "db demo" })).toBeDefined()
    release(json({ ...DETAIL, health: "degraded" }))
    expect(await screen.findByText("Degraded")).toBeDefined()

    fireEvent.click(screen.getByRole("tab", { name: "Connection tests" }))
    expect(await screen.findByText(/^Passed · Connected via PostgreSQL/)).toBeDefined()
  })

  it("says plainly when the connector type cannot be tested", async () => {
    stubFetch({
      "POST /api/connectors/conn-a/test": () =>
        json({ ok: false, supported: false, latencyMs: null, message: "This build has no probe for MQTT", testedAt: null }),
    })
    render(<ConnectorDetailPage connectorId="conn-a" />)
    fireEvent.click(await screen.findByRole("button", { name: "Test connection" }))
    expect(await screen.findByText("Not testable · This build has no probe for MQTT")).toBeDefined()
  })

  it("disables Delete, with the reason, while pipelines use the connector", async () => {
    stubFetch({}, { ...DETAIL, dependentPipelines: [{ id: "pl-1", name: "ingest_demo", kind: "pipeline" }] })
    render(<ConnectorDetailPage connectorId="conn-a" />)
    const del = (await screen.findByRole("button", { name: "Delete" })) as HTMLButtonElement
    expect(del.disabled).toBe(true)
    expect(del.getAttribute("title")).toBe(
      "Used by 1 pipeline (see Used by); those must be deleted or moved first"
    )
    expect(screen.getByRole("link", { name: "ingest_demo" }).getAttribute("href")).toBe("/pipelines/pl-1")
  })

  it("goes back to Sources after the connector is deleted", async () => {
    const { count } = stubFetch()
    render(<ConnectorDetailPage connectorId="conn-a" />)
    fireEvent.click(await screen.findByRole("button", { name: "Delete" }))
    const dialog = await screen.findByRole("dialog")
    expect(within(dialog).getByText("Delete db demo?")).toBeDefined()
    expect(pushed).toEqual([])

    fireEvent.click(within(dialog).getByRole("button", { name: "Delete" }))
    await waitFor(() => expect(pushed).toEqual(["/connectors"]))
    expect(count("DELETE /api/connectors/conn-a")).toBe(1)
  })
})
