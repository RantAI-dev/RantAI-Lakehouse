// WEB-3 S2-D2: System status replaced Health, Services, Capacity, Ingestion
// (CDC) and Table Maintenance. The page is rendered whole with `fetch`
// stubbed per API, so what is under test is what a reader sees: each
// section from its own API, a failing section failing alone (S2-D4), and
// no value the API did not send (S2-D2: "Not measured").
mock.module("next/navigation", () => ({
  usePathname: () => "/health",
  useSearchParams: () => new URLSearchParams(""),
  useRouter: () => ({
    push: () => {},
    replace: () => {},
    refresh: () => {},
    back: () => {},
    forward: () => {},
    prefetch: () => {},
  }),
}))

import { cleanup, render, screen, within } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, mock, spyOn } from "bun:test"
import { SystemStatusPage } from "./system-status-page"

const json = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } })

const SERVICES = [
  {
    id: "clickhouse",
    name: "ClickHouse",
    health: "healthy",
    checked: true,
    version: null,
    site: "local",
    replicas: null,
    errorRate: null,
    latencyMs: null,
    dependencies: [],
  },
  {
    id: "trino",
    name: "Trino",
    health: "unknown",
    checked: false,
    version: null,
    site: "local",
    replicas: null,
    errorRate: null,
    latencyMs: null,
    dependencies: [],
  },
]

const SUMMARY = {
  assetsTotal: 1,
  staleAssets: 0,
  assetsByTier: {},
  pipelines: { active: 0, failed: 0, delayed: null },
  queries: { volume24h: 42, p95Ms: 120, failureRate: 0, cacheAssistRate: null, scannedBytes24h: 0 },
  policyViolations7d: null,
  pendingApprovals: null,
  agents: { activeRuns: null, budgetUsedRate: null },
  services: { healthy: 1, degraded: 0, unhealthy: 0 },
}

/** Per-URL answers; a test overrides one to make that section fail. */
let answers: Record<string, () => Response> = {}

function defaultAnswers(): Record<string, () => Response> {
  return {
    "/api/ops/services": () => json({ services: SERVICES }),
    "/api/overview/alerts": () =>
      json([
        { id: "a1", status: "open" },
        { id: "a2", status: "resolved" },
      ]),
    "/api/lakehouse/capacity": () =>
      json({ buckets: [], clickhouse: { bytesOnDisk: 2048 }, growth7d: null }),
    "/api/governance/replication": () => json({ replicationSlots: [] }),
    "/api/governance/maintenance": () =>
      json({
        maintenance: [
          {
            tableName: "bronze.orders",
            runAt: "2026-11-04T02:00:00Z",
            dryRun: { deletedDataFiles: "1", deletedManifestFiles: "0" },
            applied: { deletedDataFiles: "1", deletedManifestFiles: "0" },
            skippedVerbs: "expire_snapshots",
          },
        ],
      }),
    "/api/governance/sla": () => json([]),
    "/api/overview": () => json(SUMMARY),
  }
}

beforeEach(() => {
  answers = defaultAnswers()
  spyOn(globalThis, "fetch").mockImplementation((async (input: RequestInfo | URL) => {
    const path = String(input).split("?")[0]
    const answer = answers[path]
    return answer ? answer() : json({ error: "not stubbed" }, 404)
  }) as unknown as typeof fetch)
})

afterEach(() => {
  cleanup()
  mock.restore()
})

function sectionById(id: string): HTMLElement {
  const el = document.getElementById(id)
  if (!el) throw new Error(`no section #${id}`)
  return el
}

describe("the System status page", () => {
  it("says the probed services are up, and does not count the unprobed one", async () => {
    render(<SystemStatusPage />)

    expect(await screen.findByText("The one probed service is up.")).toBeTruthy()
    expect(screen.getByText("Not probed, so not counted: Trino.")).toBeTruthy()
  })

  it("counts only open alerts, and links them to Alerts", async () => {
    render(<SystemStatusPage />)

    const link = await screen.findByRole("button", { name: "1 open alert" })
    expect(link.getAttribute("href")).toBe("/alerts")
  })

  it("shows Not measured for a version or latency the API did not return", async () => {
    render(<SystemStatusPage />)

    await screen.findByText("The one probed service is up.")
    const services = within(sectionById("services"))
    // Two services × version, latency, error rate, replicas.
    expect(services.getAllByText("Not measured").length).toBe(8)
  })

  it("shows the 7-day growth as Not measured when the API sends null", async () => {
    render(<SystemStatusPage />)

    const storage = within(sectionById("storage"))
    expect(await storage.findByText("Not measured")).toBeTruthy()
    expect(storage.getByText("2.0 KB")).toBeTruthy()
  })

  it("links each maintenance run to its table's page", async () => {
    render(<SystemStatusPage />)

    const link = await within(sectionById("maintenance")).findByRole("link", { name: "bronze.orders" })
    expect(link.getAttribute("href")).toBe("/lakehouse/tables/bronze/orders")
  })

  it("shrinks change capture to one line when no CDC connector reports a slot", async () => {
    render(<SystemStatusPage />)

    expect(
      await screen.findByText("Live change capture: no CDC connector has reported a replication slot.")
    ).toBeTruthy()
  })

  it("lets one section fail with its own Retry while the others still render", async () => {
    answers["/api/lakehouse/capacity"] = () => json({ error: "down" }, 503)
    render(<SystemStatusPage />)

    const storage = within(sectionById("storage"))
    expect(await storage.findByRole("button", { name: /Retry/ })).toBeTruthy()
    expect(
      await within(sectionById("maintenance")).findByRole("link", { name: "bronze.orders" })
    ).toBeTruthy()
    expect(screen.getByText("The one probed service is up.")).toBeTruthy()
  })
})
