import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"
import type { ConnectorDetail, IngestJobRun, IngestRun } from "@/services/contracts/connectors"
import { ConnectorOverview, recentProblems } from "./connector-overview"

const originalFetch = global.fetch

afterEach(() => {
  global.fetch = originalFetch
  cleanup()
})

function json(body: unknown, status = 200) {
  return new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } })
}

const DETAIL: ConnectorDetail = {
  id: "conn-northwind",
  name: "northwind",
  type: "PostgreSQL",
  direction: "source",
  health: "healthy",
  environment: "production",
  tenant: "Meridian Group",
  lastTestAt: "2026-09-30T05:00:00Z",
  lastActivityAt: null,
  lastRunSuccessAt: null,
  lastRunFailureAt: null,
  failureStreak: 0,
  capabilities: [],
  owner: "admin",
  discoveredAssets: 0,
  discoveredSchemas: [],
  recentErrors: [],
  dependentPipelines: [{ id: "pl-1", name: "ingest_northwind", kind: "pipeline" }],
  credentialManaged: true,
  credentialKind: "password",
  credentialSecondaryKind: null,
  residency: "Depok",
  tenantId: "t-1",
}

const SPEC = {
  adapter: "sql",
  ingestMode: "batch",
  dial: { driver: "postgres", host: "192.168.18.205", port: 55432, database: "northwind", user: "postgres", sslMode: null },
  sourceObjects: [
    { name: "public.orders", target: "northwind_orders" },
    { name: "public.customers", target: "northwind_customers" },
  ],
  scheduleCron: "0 2 * * *",
  secretRefs: { primary: "file:/run/secrets/connector_managed_x_password", secondary: null },
  nextRunAt: "2026-10-01T02:00:00Z",
}

function run(runId: string, status: IngestJobRun["status"], startedAt: string, endedAt: string | null): IngestJobRun {
  return { runId, status, startedAt, endedAt }
}

function result(object: string, status: string, startedAt: string, error = ""): IngestRun {
  return { connectorId: "conn-northwind", job: "ingest_job", object, rows: 10, startedAt, endedAt: startedAt, status, error }
}

describe("recentProblems", () => {
  it("reports only the latest connection test, and only when it failed", () => {
    const failed = { testedAt: "2026-09-30T05:00:00Z", ok: false, latencyMs: null, message: "authentication failed" }
    const passed = { testedAt: "2026-09-30T06:00:00Z", ok: true, latencyMs: 4, message: "connected" }
    expect(recentProblems([failed], null, null).map((p) => p.message)).toEqual([
      "Connection test failed: authentication failed",
    ])
    // A later pass resolves the earlier failure.
    expect(recentProblems([passed, failed], null, null)).toEqual([])
  })

  it("reports the last finished run, naming the tables it did not load", () => {
    const runs = [
      // Still going: not judged yet.
      run("r3", "running", "2026-09-30T06:00:00Z", null),
      run("r2", "completed", "2026-09-30T05:00:00Z", "2026-09-30T05:01:00Z"),
    ]
    const results = [
      result("public.orders", "succeeded", "2026-09-30T05:00:10Z"),
      result("public.shippers", "failed", "2026-09-30T05:00:20Z", "relation does not exist"),
    ]
    expect(recentProblems(null, runs, results).map((p) => p.message)).toEqual([
      "The last ingest run did not load 1 table: public.shippers (relation does not exist).",
    ])
  })

  it("says when the last run failed before any table", () => {
    const runs = [run("r1", "failed", "2026-09-30T05:00:00Z", "2026-09-30T05:00:05Z")]
    expect(recentProblems(null, runs, []).map((p) => p.message)).toEqual([
      "The last ingest run failed before it reached any table.",
    ])
  })

  it("has nothing to say about a clean run or data it could not load", () => {
    const runs = [run("r1", "completed", "2026-09-30T05:00:00Z", "2026-09-30T05:01:00Z")]
    const results = [result("public.orders", "succeeded", "2026-09-30T05:00:10Z")]
    expect(recentProblems(null, runs, results)).toEqual([])
    expect(recentProblems(null, null, null)).toEqual([])
  })
})

describe("ConnectorOverview", () => {
  type Stub = {
    spec?: () => Response
    probes?: unknown[]
    runs?: () => Response
  }

  function stubFetch({ spec, probes, runs }: Stub = {}) {
    global.fetch = mock(async (input: RequestInfo | URL) => {
      const url = String(input)
      if (url.includes("/ingest-spec")) return spec ? spec() : json(SPEC)
      if (url.includes("/probe-history")) {
        return json({
          results: probes ?? [
            { testedAt: "2026-09-30T05:00:00Z", ok: false, latencyMs: null, message: "authentication failed" },
          ],
        })
      }
      if (url.includes("/ingest/runs")) {
        return runs ? runs() : json([run("r1", "completed", "2026-09-30T05:00:00Z", "2026-09-30T05:01:00Z")])
      }
      if (url.includes("/api/governance/ingest-runs")) return json([])
      throw new Error(`unexpected fetch: ${url}`)
    }) as unknown as typeof fetch
  }

  /** The tile strip's tile by its label: the button (or box) that holds it. */
  function tile(label: string): HTMLElement {
    return screen.getByText(label, { selector: "span" }).parentElement as HTMLElement
  }

  /**
   * Real values only: the connection from the saved settings, the ingest
   * summary from the spec and the orchestrator, and none of the former
   * placeholder sections (discovered assets/schemas, capabilities).
   */
  it("summarizes the connector from real data and points to what needs a look", async () => {
    stubFetch()
    const opened: string[] = []
    render(<ConnectorOverview detail={DETAIL} onOpenTab={(tab) => opened.push(tab)} />)

    await screen.findByText("192.168.18.205:55432")
    expect(screen.getByText("2 tables")).toBeDefined()
    expect(screen.getByText("Every day at 02:00 UTC")).toBeDefined()
    expect(screen.getByText(/^Next /)).toBeDefined()
    await waitFor(() => expect(screen.getByText("Completed")).toBeDefined())
    expect(screen.getByRole("link", { name: "ingest_northwind" }).getAttribute("href")).toBe("/pipelines/pl-1")
    expect(screen.queryByText(/Discovered/)).toBeNull()
    expect(screen.queryByText(/Capabilities/)).toBeNull()
    // What the connector is lives in the page header now, not here.
    expect(screen.queryByText("Details")).toBeNull()
    expect(screen.queryByText("Stored by lakehouse")).toBeNull()

    await screen.findByText("Needs attention")
    expect(screen.getByText(/Connection test failed: authentication failed/)).toBeDefined()
    fireEvent.click(screen.getByRole("button", { name: "Test history" }))
    fireEvent.click(screen.getByRole("button", { name: /^Tables/ }))
    expect(opened).toEqual(["tests", "ingest"])
  })

  it("shows the connector's health and when it was last tested, and opens the tests from it", async () => {
    stubFetch()
    const opened: string[] = []
    const { rerender } = render(
      <ConnectorOverview detail={{ ...DETAIL, health: "degraded" }} onOpenTab={(tab) => opened.push(tab)} />
    )
    const health = await screen.findByRole("button", { name: /^Health/ })
    expect(health.textContent).toContain("Degraded")
    expect(health.textContent).toMatch(/Tested \d+(m|h|d|mo) ago/)
    fireEvent.click(health)
    expect(opened).toEqual(["tests"])

    rerender(<ConnectorOverview detail={{ ...DETAIL, lastTestAt: null }} onOpenTab={() => {}} />)
    expect(screen.getByRole("button", { name: /^Health/ }).textContent).toContain("Never tested")
  })

  it("names the table count, in the singular too, and says when none is picked, never a zero", async () => {
    stubFetch({ spec: () => json({ ...SPEC, sourceObjects: [SPEC.sourceObjects[0]] }) })
    const first = render(<ConnectorOverview detail={DETAIL} onOpenTab={() => {}} />)
    expect(await screen.findByText("1 table")).toBeDefined()
    first.unmount()

    stubFetch({ spec: () => json({ ...SPEC, sourceObjects: [] }) })
    render(<ConnectorOverview detail={DETAIL} onOpenTab={() => {}} />)
    expect(await screen.findByText("None picked yet")).toBeDefined()
    expect(tile("Tables").textContent).not.toMatch(/\b0\b/)
  })

  it("says a change-data-capture connector streams, and counts the tables it streams", async () => {
    stubFetch({
      spec: () => json({ ...SPEC, adapter: "cdc", ingestMode: "stream", scheduleCron: null, nextRunAt: null }),
    })
    render(<ConnectorOverview detail={DETAIL} onOpenTab={() => {}} />)
    expect(await screen.findByText("Streams continuously")).toBeDefined()
    expect(screen.getByRole("button", { name: /^Tables streamed/ }).textContent).toContain("2 tables")
    expect(screen.queryByText(/^Next /)).toBeNull()
  })

  it("shows a manual-only schedule with no next run", async () => {
    stubFetch({ spec: () => json({ ...SPEC, scheduleCron: null, nextRunAt: null }) })
    render(<ConnectorOverview detail={DETAIL} onOpenTab={() => {}} />)
    expect(await screen.findByText("Manual only")).toBeDefined()
    expect(screen.queryByText(/^Next /)).toBeNull()
  })

  it("says the connection is not saved yet, in the tiles and the Connection card, rather than showing zeros", async () => {
    stubFetch({ spec: () => json({ ...SPEC, adapter: null, ingestMode: null, sourceObjects: [], scheduleCron: null, nextRunAt: null }) })
    render(<ConnectorOverview detail={DETAIL} onOpenTab={() => {}} />)
    expect(await screen.findByText("No connection settings saved yet.")).toBeDefined()
    expect(screen.getAllByText("Not set up yet").length).toBe(2)
    expect(screen.getAllByText("Save the connection first").length).toBe(2)
    expect(screen.queryByText("None picked yet")).toBeNull()
    expect(screen.queryByText(/\d+ tables?/)).toBeNull()
  })

  it("says the settings could not be loaded, in the two ingest tiles and the card, instead of an empty value", async () => {
    stubFetch({ spec: () => json({ error: "the store is unavailable" }, 500) })
    render(<ConnectorOverview detail={DETAIL} onOpenTab={() => {}} />)
    expect(await screen.findByText(/Connection settings could not be loaded/)).toBeDefined()
    expect(screen.getAllByText("Could not be loaded").length).toBe(2)
    expect(screen.queryByText("None picked yet")).toBeNull()
  })

  it("shows the last run as a pill with when it started, or that there was none", async () => {
    stubFetch()
    const first = render(<ConnectorOverview detail={DETAIL} onOpenTab={() => {}} />)
    const last = await screen.findByRole("button", { name: /^Last run/ })
    await waitFor(() => expect(last.textContent).toContain("Completed"))
    expect(last.textContent).toMatch(/\d+(m|h|d|mo) ago/)
    expect(last.querySelector("[title]")?.getAttribute("title")).toMatch(/2026/)
    first.unmount()

    stubFetch({ runs: () => json([]) })
    render(<ConnectorOverview detail={DETAIL} onOpenTab={() => {}} />)
    expect(await screen.findByText("Never run")).toBeDefined()
  })

  it("says the orchestrator could not be asked, rather than that nothing has run", async () => {
    stubFetch({ runs: () => json({ error: "orchestrator unreachable" }, 502) })
    render(<ConnectorOverview detail={DETAIL} onOpenTab={() => {}} />)
    expect(await screen.findByText("Could not ask the orchestrator")).toBeDefined()
    expect(screen.queryByText("Never run")).toBeNull()
  })

  it("puts Connection and Used by in cards, with Edit on Connection", async () => {
    stubFetch()
    render(<ConnectorOverview detail={DETAIL} onOpenTab={() => {}} />)
    await screen.findByText("192.168.18.205:55432")
    const connection = screen.getByText("Connection", { selector: "[data-slot=card-title]" }).closest("[data-slot=card]")
    expect(connection).not.toBeNull()
    expect(within(connection as HTMLElement).getByRole("button", { name: "Edit" }).getAttribute("href")).toBe(
      "/connectors/conn-northwind/edit"
    )
    expect(screen.getByText("Used by", { selector: "[data-slot=card-title]" })).toBeDefined()
    // The pipeline names are monospace at the card's own text size.
    const pipeline = screen.getByRole("link", { name: "ingest_northwind" })
    expect(pipeline.className).toContain("font-mono")
    expect(pipeline.className).toContain("text-sm")
    // The old Ingest section is gone: the tiles say what it said.
    expect(screen.queryByText("Manage")).toBeNull()
  })
})
