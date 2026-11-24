// WEB-3 S2-D2: System status replaced Health, Services, Capacity, Ingestion
// (CDC) and Table Maintenance. The page is rendered whole with `fetch`
// stubbed per API, so what is under test is what a reader sees: each
// section from its own API, a failing section failing alone (S2-D4), and
// no value the API did not send (S2-D2: "Not measured").
mock.module("next/navigation", () => ({
  usePathname: () => "/health",
  useParams: () => ({}),
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

const icebergTable = (namespace: string, name: string) => ({
  namespace,
  name,
  formatVersion: 2,
  currentSnapshotId: null,
  lastUpdatedAt: null,
  fileCount: null,
  recordCount: null,
  totalBytes: null,
})

/** Per-URL answers; a test overrides one to make that section fail. */
let answers: Record<string, () => Response> = {}

function defaultAnswers(): Record<string, () => Response> {
  return {
    "/api/ops/services": () => json({ services: SERVICES, auditWriteFailures: 0 }),
    "/api/overview/alerts": () =>
      json([
        { id: "a1", status: "open" },
        { id: "a2", status: "resolved" },
      ]),
    "/api/lakehouse/capacity": () =>
      json({ buckets: [], clickhouse: { bytesOnDisk: 2048 }, growth7d: null }),
    // OPS-21: no measured table needs compaction by default, so the warning
    // is absent; individual tests override this to exercise the warning.
    "/api/lakehouse/compaction-status": () =>
      json({
        compactionAvailable: false,
        smallFileThresholdBytes: 32 * 1024 * 1024,
        needsCompactionFileCount: 20,
        tables: [],
        measuredTables: 0,
        unmeasuredTables: 0,
      }),
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
    // DATA-45: the Iceberg tables list loads independently of the runs above.
    // By default Bronze is the only namespace and its only table already has
    // a run, so the subsection shows its honest "none without a run" line.
    "/api/lakehouse/namespaces": () => json({ namespaces: [{ name: "bronze", tableCount: 1 }] }),
    "/api/lakehouse/tables": () => json({ tables: [icebergTable("bronze", "orders")] }),
    "/api/governance/sla": () => json([]),
    "/api/overview": () => json(SUMMARY),
  }
}

beforeEach(() => {
  answers = defaultAnswers()
  spyOn(globalThis, "fetch").mockImplementation((async (input: RequestInfo | URL) => {
    const url = String(input)
    // A per-namespace answer wins; otherwise fall back to the path answer.
    const answer = answers[url] ?? answers[url.split("?")[0]]
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

  // GOV-1 Decision 4: a failed audit write is counted since the process
  // started; the line shows when the count is 0 too.
  it("says how many audit writes failed since the service started", async () => {
    answers["/api/ops/services"] = () => json({ services: SERVICES, auditWriteFailures: 7 })
    render(<SystemStatusPage />)

    expect(
      await within(sectionById("services")).findByText(
        "Audit writes failed since the service started: 7"
      )
    ).toBeTruthy()
  })

  it("shows the audit write line with 0 when none failed", async () => {
    render(<SystemStatusPage />)

    expect(
      await within(sectionById("services")).findByText(
        "Audit writes failed since the service started: 0"
      )
    ).toBeTruthy()
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

  // DATA-45: the runs list only reaches tables that have a run. A Gold
  // export table with no catalog entry and no run used to be reachable by
  // URL alone; it must now be listed here with its real link.
  it("lists an uncataloged Iceberg table with no run and links to its table page", async () => {
    answers["/api/lakehouse/namespaces"] = () =>
      json({ namespaces: [{ name: "gold", tableCount: 1 }] })
    answers["/api/lakehouse/tables"] = () => json({ tables: [icebergTable("gold", "revenue_export")] })
    render(<SystemStatusPage />)

    const maintenance = within(sectionById("maintenance"))
    const link = await maintenance.findByRole("link", { name: "gold.revenue_export" })
    expect(link.getAttribute("href")).toBe("/lakehouse/tables/gold/revenue_export")
    expect(maintenance.getByText("No run recorded yet")).toBeTruthy()
    // No invented maintenance counts or status on the Gold table's own row.
    const row = link.closest("tr")
    expect(row).not.toBeNull()
    expect(within(row as HTMLElement).queryByText(/data \/.*manifest/)).toBeNull()
    expect(within(row as HTMLElement).getByText("No run recorded yet")).toBeTruthy()
  })

  it("still renders the existing maintenance run rows with their metrics and link", async () => {
    render(<SystemStatusPage />)

    const maintenance = within(sectionById("maintenance"))
    const link = await maintenance.findByRole("link", { name: "bronze.orders" })
    expect(link.getAttribute("href")).toBe("/lakehouse/tables/bronze/orders")
    // Dry-run and applied columns of the one recorded run are unchanged.
    expect(maintenance.getAllByText("1 data / 0 manifest").length).toBe(2)
    expect(maintenance.getByText("expire_snapshots")).toBeTruthy()
    // Bronze already has a run, so it is not repeated in the new list.
    expect(maintenance.getByText("No Iceberg tables without a maintenance run were found.")).toBeTruthy()
  })

  it("reports a refused Lakekeeper list without hiding the maintenance runs", async () => {
    answers["/api/lakehouse/namespaces"] = () =>
      json({ error: "permission_denied: lakehouse:read" }, 403)
    render(<SystemStatusPage />)

    const maintenance = within(sectionById("maintenance"))
    expect(await maintenance.findByText(/The Iceberg table list could not be loaded here/)).toBeTruthy()
    expect(await maintenance.findByRole("link", { name: "bronze.orders" })).toBeTruthy()
  })

  it("handles an empty namespace list honestly, keeping the maintenance runs", async () => {
    answers["/api/lakehouse/namespaces"] = () => json({ namespaces: [] })
    render(<SystemStatusPage />)

    const maintenance = within(sectionById("maintenance"))
    expect(
      await maintenance.findByText("No Iceberg tables without a maintenance run were found.")
    ).toBeTruthy()
    expect(await maintenance.findByRole("link", { name: "bronze.orders" })).toBeTruthy()
  })

  it("shows tables from namespaces that answered when one namespace fails", async () => {
    answers["/api/lakehouse/namespaces"] = () =>
      json({
        namespaces: [
          { name: "bronze", tableCount: 1 },
          { name: "broken", tableCount: null },
          { name: "gold", tableCount: 1 },
        ],
      })
    answers["/api/lakehouse/tables?namespace=bronze"] = () =>
      json({ tables: [icebergTable("bronze", "orders")] })
    answers["/api/lakehouse/tables?namespace=broken"] = () => json({ error: "down" }, 503)
    answers["/api/lakehouse/tables?namespace=gold"] = () =>
      json({ tables: [icebergTable("gold", "revenue_export")] })
    render(<SystemStatusPage />)

    const maintenance = within(sectionById("maintenance"))
    // The failing namespace is named, but its failure does not hide Gold.
    expect(await maintenance.findByText(/Tables in broken could not be listed/)).toBeTruthy()
    expect(await maintenance.findByRole("link", { name: "gold.revenue_export" })).toBeTruthy()
    expect(await maintenance.findByRole("link", { name: "bronze.orders" })).toBeTruthy()
  })

  it("warns with the affected tables and measured counts, naming the trino profile, when Trino is off", async () => {
    answers["/api/lakehouse/compaction-status"] = () =>
      json({
        compactionAvailable: false,
        smallFileThresholdBytes: 32 * 1024 * 1024,
        needsCompactionFileCount: 20,
        tables: [
          { namespace: "bronze", name: "orders", smallFileCount: 42 },
          { namespace: "bronze", name: "events", smallFileCount: 20 },
          { namespace: "bronze", name: "clean", smallFileCount: 3 },
        ],
        measuredTables: 3,
        unmeasuredTables: 1,
      })
    render(<SystemStatusPage />)

    const alert = await screen.findByRole("alert")
    expect(within(alert).getByText("Compaction needed, but nothing can do it")).toBeTruthy()
    expect(within(alert).getByText(/--profile trino/)).toBeTruthy()
    expect(within(alert).getByText(/trino profile/)).toBeTruthy()
    expect(within(alert).getByText(/42 small files/)).toBeTruthy()
    expect(within(alert).getByText(/20 small files/)).toBeTruthy()
    // The below-threshold table is not listed; the unmeasured one is counted.
    expect(within(alert).queryByText(/clean/)).toBeNull()
    expect(within(alert).getByText(/1 more table was not measured/)).toBeTruthy()
    const orders = within(alert).getByRole("link", { name: "bronze.orders" })
    const events = within(alert).getByRole("link", { name: "bronze.events" })
    expect(orders.getAttribute("href")).toBe("/lakehouse/tables/bronze/orders")
    expect(events.getAttribute("href")).toBe("/lakehouse/tables/bronze/events")
  })

  it("is silent about compaction when Trino can compact, however high the counts", async () => {
    answers["/api/lakehouse/compaction-status"] = () =>
      json({
        compactionAvailable: true,
        smallFileThresholdBytes: 32 * 1024 * 1024,
        needsCompactionFileCount: 20,
        tables: [{ namespace: "bronze", name: "orders", smallFileCount: 99 }],
        measuredTables: 1,
        unmeasuredTables: 0,
      })
    render(<SystemStatusPage />)

    await screen.findByText("The one probed service is up.")
    expect(screen.queryByRole("alert")).toBeNull()
  })

  it("is silent about compaction when no measured table reaches the need count", async () => {
    answers["/api/lakehouse/compaction-status"] = () =>
      json({
        compactionAvailable: false,
        smallFileThresholdBytes: 32 * 1024 * 1024,
        needsCompactionFileCount: 20,
        tables: [{ namespace: "bronze", name: "orders", smallFileCount: 19 }],
        measuredTables: 1,
        unmeasuredTables: 3,
      })
    render(<SystemStatusPage />)

    await screen.findByText("The one probed service is up.")
    expect(screen.queryByRole("alert")).toBeNull()
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
