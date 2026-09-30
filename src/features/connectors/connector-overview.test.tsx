import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
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
  function stubFetch() {
    global.fetch = mock(async (input: RequestInfo | URL) => {
      const url = String(input)
      if (url.includes("/ingest-spec")) return json(SPEC)
      if (url.includes("/probe-history")) {
        return json({
          results: [{ testedAt: "2026-09-30T05:00:00Z", ok: false, latencyMs: null, message: "authentication failed" }],
        })
      }
      if (url.includes("/ingest/runs")) return json([run("r1", "completed", "2026-09-30T05:00:00Z", "2026-09-30T05:01:00Z")])
      if (url.includes("/api/governance/ingest-runs")) return json([])
      throw new Error(`unexpected fetch: ${url}`)
    }) as unknown as typeof fetch
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
    expect(screen.getByText("Stored by lakehouse")).toBeDefined()
    expect(screen.getByText("2 tables")).toBeDefined()
    expect(screen.getByText("Every day at 02:00 UTC")).toBeDefined()
    expect(screen.getByText(/^Next /)).toBeDefined()
    await waitFor(() => expect(screen.getByText("Completed")).toBeDefined())
    expect(screen.getByRole("link", { name: "ingest_northwind" }).getAttribute("href")).toBe("/pipelines/pl-1")
    expect(screen.queryByText(/Discovered/)).toBeNull()
    expect(screen.queryByText(/Capabilities/)).toBeNull()

    await screen.findByText("Needs attention")
    expect(screen.getByText(/Connection test failed: authentication failed/)).toBeDefined()
    fireEvent.click(screen.getByRole("button", { name: "Test history" }))
    fireEvent.click(screen.getByRole("button", { name: "Manage" }))
    expect(opened).toEqual(["tests", "ingest"])
  })
})
