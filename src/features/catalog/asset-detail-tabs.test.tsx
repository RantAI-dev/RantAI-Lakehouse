// Ten tabs became seven, and the open one lives in `?tab=` so a refresh or
// a shared link lands on it. Each tab shows what is recorded about the
// asset rather than a list the API always sends empty: Lineage draws the
// recorded graph (and says so when there is none), Activity lists the
// Iceberg table's snapshots with a query for each, Schema is one table in
// the table's own order, and Quality never invents a verdict for a rule
// nothing has run.
const url = { search: "" }
mock.module("next/navigation", () => ({
  usePathname: () => "/data/assets/demo-orders",
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

import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react"
import { afterEach, describe, expect, it, mock, spyOn } from "bun:test"
import { AuthProvider } from "@/features/auth/auth-provider"
import type { AssetDetail } from "@/services/contracts/assets"
import { AssetDetailTabs } from "./asset-detail-tabs"

afterEach(() => {
  cleanup()
  mock.restore()
  url.search = ""
})

const BRONZE: AssetDetail = {
  id: "demo-orders",
  name: "Demo Orders",
  namespace: "primer",
  type: "iceberg-table",
  layer: "raw",
  tier: "warm",
  classification: "internal",
  owner: "dagster",
  domain: "pariwisata",
  description: "Orders ingested from Postgres.",
  format: "Apache Iceberg (Parquet)",
  engine: "hot-store",
  rows: 2000,
  sizeBytes: 40287,
  columnCount: 2,
  freshnessLagSeconds: 3600,
  lastUpdated: "2026-09-23T04:57:42Z",
  health: "unknown",
  residency: "id-jakarta",
  // Registry order: alphabetical. The table's own order is id, amount.
  schema: [
    { name: "amount", dataType: "Decimal(12, 2)" },
    { name: "id", dataType: "Int64" },
  ],
  sample: [{ id: "1", amount: "1.5" }],
  qualityChecks: [],
  policySummary: [],
  usage: null,
  recentQueries: [],
  dependents: [],
  changeHistory: [],
  snapshots: [],
  schemaVersions: [],
  upstream: [],
  downstream: [],
  tableName: "demo_orders",
  tableKey: "bronze.demo_orders",
  queryTarget: { engine: "clickhouse", table: "icecat_api.`bronze.demo_orders`" },
}

const TABLE = {
  schema: [
    { id: 1, name: "id", type: "long", required: true },
    { id: 2, name: "amount", type: "decimal(12, 2)", required: false },
    { id: 3, name: "_ingested_at", type: "timestamptz", required: false },
  ],
  schemaVersions: [
    {
      schemaId: 0,
      sinceMs: Date.parse("2026-09-20T00:00:00Z"),
      current: false,
      fields: [
        { id: 1, name: "id", type: "long", required: true },
        { id: 3, name: "_ingested_at", type: "timestamptz", required: false },
      ],
    },
    {
      schemaId: 1,
      sinceMs: Date.parse("2026-09-23T03:37:39Z"),
      current: true,
      fields: [
        { id: 1, name: "id", type: "long", required: true },
        { id: 2, name: "amount", type: "decimal(12, 2)", required: false },
        { id: 3, name: "_ingested_at", type: "timestamptz", required: false },
      ],
    },
  ],
  partitionSpec: [{ sourceId: 3, transform: "day", name: "_ingested_at_day" }],
  properties: {},
  snapshots: [
    {
      id: "248842615326512766",
      parentId: null,
      timestampMs: Date.parse("2026-09-23T03:37:39Z"),
      operation: "append",
      summary: { addedRecords: 2000, deletedRecords: null, totalRecords: 2000, totalDataFiles: 1 },
    },
  ],
  stats: {
    fileCount: 1,
    smallFileCount: null,
    smallFileThresholdBytes: null,
    recordCount: 2000,
    totalBytes: 40287,
    snapshotCount: 1,
    metadataLogCount: 1,
  },
}

/**
 * connector → bronze.demo_orders → silver.orders_clean, in the shape
 * `routes/lineage.rs` answers: the asked-about node is `kind: "focus"`, and
 * an authored pipeline is an edge named in its `evidence`.
 */
const GRAPH = {
  focus: "bronze.demo_orders",
  nodes: [
    { id: "connector:conn-pg", label: "Postgres", kind: "connector" },
    { id: "bronze:demo_orders", label: "bronze.demo_orders", kind: "focus" },
    { id: "table:silver.orders_clean", label: "silver.orders_clean", kind: "silver" },
  ],
  edges: [
    { id: "e0", from: "connector:conn-pg", to: "bronze:demo_orders", kind: "ingest", evidence: "ingest spec (sql adapter)" },
    {
      id: "e1",
      from: "bronze:demo_orders",
      to: "table:silver.orders_clean",
      kind: "pipeline",
      evidence: "authored pipeline orders_clean (pl-clean)",
    },
  ],
  columnMappings: [],
  supported: true,
  coverage: ["only edges the platform recorded are drawn"],
}

const NO_LINEAGE = {
  ...GRAPH,
  nodes: [],
  edges: [],
  note: "no recorded lineage mentions bronze.demo_orders",
}

const PROFILE = {
  supported: true,
  source: "bronze.demo_orders",
  sourceKind: "iceberg",
  rowsProfiled: 2000,
  rowLimit: 100000,
  sampled: false,
  columnsCapped: false,
  columns: [
    { name: "id", dataType: "Int64", profiled: true, nullFraction: 0, distinctCount: 2000, min: "1", max: "2000", topValues: [] },
    { name: "amount", dataType: "Decimal(12, 2)", profiled: true, nullFraction: 0.25, distinctCount: 40, min: "1.5", max: "99", topValues: [] },
  ],
}

function json(body: unknown, status = 200) {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  })
}

function stubApi({
  permissions = ["*:*"],
  lineage = GRAPH,
}: { permissions?: string[]; lineage?: unknown } = {}) {
  return spyOn(globalThis, "fetch").mockImplementation((async (input: RequestInfo | URL, init?: RequestInit) => {
    const path = String(input)
    const method = init?.method ?? "GET"
    if (method === "PUT" && path.endsWith("/annotation")) return json({ ok: true })
    if (method === "PUT" && path.endsWith("/api/governance/sla")) return json({ ok: true })
    if (method === "DELETE" && path.includes("/api/governance/quality/")) return json({ ok: true })
    if (method === "PUT" && path.includes("/api/governance/quality/")) {
      return json({ id: "q3", name: "email_complete", ...JSON.parse(String(init?.body)), dimension: "completeness", lastStatus: null, lastRunAt: null, evaluable: true, hint: null })
    }
    if (method === "POST" && path.endsWith("/api/governance/policies")) return json({ id: "p-new" }, 201)
    if (method === "PUT" && path.includes("/api/governance/policies/")) return json({ id: "p1", status: JSON.parse(String(init?.body)).status })
    if (method === "DELETE" && path.includes("/api/governance/policies/")) return json({ ok: true })
    if (path.includes("/sample?limit=")) {
      const limit = Number(path.split("limit=")[1])
      return json({ rows: Array.from({ length: limit }, (_, i) => ({ id: String(i + 1), amount: "1.5" })), limit })
    }
    if (method === "POST" && path.endsWith("/run")) return json({ id: "q2", status: "passed", value: "0 repeated values in 50 rows" })
    if (method === "POST" && path.endsWith("/api/governance/quality")) return json({ id: "new" }, 201)
    if (method === "POST" && path.endsWith("/api/governance/classification")) return json({ id: "c1" }, 201)
    if (path.includes("/api/auth/me")) {
      return json({ id: "u1", name: "Reader", email: null, roles: ["Analyst"], permissions, tenants: [] })
    }
    if (path.includes("/api/governance/lineage")) return json(lineage)
    if (path.endsWith("/profile")) return json(PROFILE)
    if (path.endsWith("/maintenance")) return json({ configured: false })
    if (path.includes("/api/lakehouse/tables/bronze/demo_orders")) return json(TABLE)
    return json({ error: "not stubbed" }, 404)
  }) as unknown as typeof fetch)
}

function renderTabs(asset: AssetDetail = BRONZE, onAssetChanged: () => void = () => {}) {
  return render(
    <AuthProvider>
      <AssetDetailTabs asset={asset} onAssetChanged={onAssetChanged} />
    </AuthProvider>
  )
}

/** The `[method, path, body]` of every write the page sent. */
function writes(fetchSpy: ReturnType<typeof stubApi>) {
  return fetchSpy.mock.calls
    .filter((c) => (c[1]?.method ?? "GET") !== "GET")
    .map((c) => [c[1]?.method, String(c[0]), c[1]?.body ? JSON.parse(String(c[1].body)) : undefined])
}

function tabNames() {
  return screen.getAllByRole("tab").map((t) => t.textContent)
}

describe("AssetDetailTabs", () => {
  it("shows seven tabs, counting only what has a count", async () => {
    stubApi()
    renderTabs()

    // The connector upstream, the Silver table downstream, and the
    // pipeline that reads it as a dependent.
    await waitFor(() =>
      expect(tabNames()).toEqual([
        "Overview",
        "Schema2",
        "Sample",
        "Quality0",
        "Access0",
        "Lineage3",
        "Activity",
      ])
    )
  })

  it("opens the tab the URL names, and falls back to the overview for one it does not know", () => {
    stubApi()
    url.search = "tab=access"
    renderTabs()
    expect(screen.getByRole("tab", { name: /Access/ }).getAttribute("aria-selected")).toBe("true")
    cleanup()

    url.search = "tab=snapshots"
    renderTabs()
    expect(screen.getByRole("tab", { name: "Overview" }).getAttribute("aria-selected")).toBe("true")
  })

  it("writes the picked tab to the URL, and drops it for the overview", () => {
    stubApi()
    const replace = spyOn(window.history, "replaceState")
    renderTabs()

    fireEvent.click(screen.getByRole("tab", { name: /Access/ }))
    expect(replace).toHaveBeenLastCalledWith(null, "", "/data/assets/demo-orders?tab=access")
    expect(screen.getByText("No policy binds this asset")).toBeTruthy()

    fireEvent.click(screen.getByRole("tab", { name: "Overview" }))
    expect(replace).toHaveBeenLastCalledWith(null, "", "/data/assets/demo-orders")
  })
})

describe("Lineage tab", () => {
  it("draws the recorded graph around the asset, by its table key", async () => {
    const fetchSpy = stubApi()
    url.search = "tab=lineage"
    renderTabs()

    const graph = await screen.findByRole("list", { name: "Lineage graph" })
    expect(within(graph).getAllByRole("listitem").map((n) => n.textContent)).toEqual([
      "ConnectorPostgres",
      "Bronzebronze.demo_orders",
      "Silversilver.orders_clean",
    ])
    // Its neighbours open their own pages; the asset itself is not a link to itself.
    expect(within(graph).getByText("Postgres").closest("a")?.getAttribute("href")).toBe("/connectors/conn-pg/edit")
    expect(within(graph).getByText("silver.orders_clean").closest("a")?.getAttribute("href")).toBe(
      "/data/assets/silver.orders_clean"
    )
    expect(within(graph).getByText("bronze.demo_orders").closest("a")).toBeNull()
    expect(fetchSpy.mock.calls.some((c) => String(c[0]).endsWith("?focus=bronze.demo_orders"))).toBe(true)

    expect(screen.getByText("Open lineage graph").closest("a")?.getAttribute("href")).toBe(
      "/lineage?focus=bronze.demo_orders"
    )
    // The pipeline that reads it is a dependent, named by the edge's evidence.
    const dependents = screen.getByText("Dependents").closest("[data-slot=card]") as HTMLElement
    expect(within(dependents).getByText("orders_clean").closest("a")?.getAttribute("href")).toBe(
      "/pipelines/pl-clean"
    )
  })

  it("says nothing is recorded, and why, instead of drawing an empty graph", async () => {
    stubApi({ lineage: NO_LINEAGE })
    url.search = "tab=lineage"
    renderTabs()

    expect(await screen.findByText("No lineage recorded for bronze.demo_orders")).toBeTruthy()
    expect(screen.getByText(NO_LINEAGE.note)).toBeTruthy()
    expect(screen.getByRole("tab", { name: /Lineage/ }).textContent).toBe("Lineage0")
  })

  it("asks for nothing, and says what is missing, without lineage:read", async () => {
    const fetchSpy = stubApi({ permissions: ["catalog:read", "query:read"] })
    url.search = "tab=lineage"
    renderTabs()

    expect(await screen.findByText("Lineage needs lineage access")).toBeTruthy()
    expect(fetchSpy.mock.calls.some((c) => String(c[0]).includes("/api/governance/lineage"))).toBe(false)
  })
})

describe("Activity tab", () => {
  it("lists the Iceberg table's snapshots, each with a query pinned to it", async () => {
    stubApi()
    url.search = "tab=activity"
    renderTabs()

    const row = (await screen.findByText("append")).closest("tr") as HTMLElement
    expect(within(row).getByText("+2,000")).toBeTruthy()
    expect(within(row).getByText("248842615326512766")).toBeTruthy()
    const href = within(row).getByText("Query this version").closest("a")?.getAttribute("href") ?? ""
    expect(decodeURIComponent(href)).toContain("iceberg_snapshot_id+=+248842615326512766")
  })

  it("says a ClickHouse table keeps no snapshots, rather than that it has none", () => {
    stubApi()
    url.search = "tab=activity"
    renderTabs({
      ...BRONZE,
      id: "silver.orders",
      type: "table",
      layer: "silver",
      tableName: undefined,
      tableKey: "silver.orders",
      queryTarget: undefined,
    })

    expect(screen.getByText("Only Iceberg tables keep snapshots")).toBeTruthy()
  })
})

describe("Schema tab", () => {
  it("lists columns in the table's order with their statistics, and system columns apart", async () => {
    stubApi()
    url.search = "tab=schema"
    renderTabs()

    await screen.findByText("System columns")
    const columns = screen.getByText("Columns").closest("[data-slot=card]") as HTMLElement
    await waitFor(() => expect(within(columns).getByText("25.0%")).toBeTruthy())
    const names = within(columns)
      .getAllByRole("row")
      .slice(1)
      .map((r) => r.querySelector("td")?.textContent)
    expect(names).toEqual(["id", "amount"])
    // `amount` is optional in Iceberg; `id` is required.
    const amount = within(columns).getByText("amount").closest("tr") as HTMLElement
    expect(within(amount).getByText("Yes")).toBeTruthy()

    const system = screen.getByText("System columns").closest("[data-slot=card]") as HTMLElement
    expect(within(system).getByText("_ingested_at")).toBeTruthy()
    expect(within(system).getByText("partition · day")).toBeTruthy()

    // The table's own schema history, newest first, in words.
    const versions = screen.getByText("Schema versions").closest("[data-slot=card]") as HTMLElement
    expect(within(versions).getAllByRole("listitem").map((li) => li.textContent)).toEqual([
      expect.stringContaining("v1currentAdded amount (decimal(12, 2))"),
      expect.stringContaining("v0Created with 2 columns"),
    ])
  })
})

describe("Quality tab", () => {
  const checks: AssetDetail["qualityChecks"] = [
    { id: "q1", name: "row_count", dimension: "completeness", status: "failed", lastRun: "2026-09-30T00:00:00Z", origin: "observed" },
    { id: "q2", name: "orders_uniqueness", dimension: "uniqueness", status: null, lastRun: null, threshold: "id unique", severity: "high", origin: "rule" },
  ]

  it("turns the count red when a check fails", async () => {
    stubApi()
    renderTabs({ ...BRONZE, qualityChecks: checks })

    const count = screen.getByRole("tab", { name: /Quality/ }).querySelector("span:last-child")
    expect(count?.textContent).toBe("2")
    expect(count?.className).toContain("text-destructive")
  })

  it("shows a rule nothing has run as not evaluated, never as a verdict", () => {
    stubApi()
    url.search = "tab=quality"
    renderTabs({ ...BRONZE, qualityChecks: checks })

    const rule = screen.getByText("orders_uniqueness").closest("tr") as HTMLElement
    expect(within(rule).getByText("Not run yet")).toBeTruthy()
    expect(within(rule).getByText("Never")).toBeTruthy()
    expect(screen.getByText("1 failed · 1 without a result")).toBeTruthy()
  })

  it("marks a rule the evaluator cannot read, with how to rewrite it", () => {
    stubApi()
    url.search = "tab=quality"
    renderTabs({
      ...BRONZE,
      qualityChecks: [
        { id: "q3", name: "email_complete", dimension: "completeness", status: null, lastRun: null, threshold: ">= 95%", origin: "rule", evaluable: false, hint: "Write the threshold as one of: …" },
      ],
    })

    const rule = screen.getByText("email_complete").closest("tr") as HTMLElement
    expect(within(rule).getByText("Can't be run")).toBeTruthy()
    expect(within(rule).getByText("Write the threshold as one of: …")).toBeTruthy()
    // Nothing runnable: no button to run nothing.
    expect(screen.queryByText("Run checks")).toBeNull()
  })

  it("runs the authored rules that can be run, then reloads the asset", async () => {
    const fetchSpy = stubApi()
    const reloaded = mock(() => {})
    url.search = "tab=quality"
    renderTabs({ ...BRONZE, qualityChecks: checks }, reloaded)

    fireEvent.click(await screen.findByText("Run checks"))
    await waitFor(() => expect(reloaded).toHaveBeenCalled())
    // The observed check (q1) is the quality job's to run, not this page's.
    expect(writes(fetchSpy)).toEqual([["POST", "/api/governance/quality/q2/run", undefined]])
  })

  it("adds a rule in the form the evaluator reads, against the table key", async () => {
    const fetchSpy = stubApi()
    const reloaded = mock(() => {})
    url.search = "tab=quality"
    renderTabs(BRONZE, reloaded)

    fireEvent.click(screen.getByText("Add rule"))
    fireEvent.change(screen.getByLabelText("Check"), { target: { value: "unique" } })
    fireEvent.change(screen.getByLabelText("Column"), { target: { value: "id" } })
    expect(screen.getByText("id unique")).toBeTruthy()
    // Bronze is append-only: what "unique" means there is said up front.
    expect(screen.getByText(/only when it repeats within\s+one load/)).toBeTruthy()
    fireEvent.click(screen.getByRole("button", { name: "Add rule" }))

    await waitFor(() => expect(reloaded).toHaveBeenCalled())
    expect(writes(fetchSpy)).toEqual([
      [
        "POST",
        "/api/governance/quality",
        { name: "demo_orders_id_unique", asset: "bronze.demo_orders", dimension: "uniqueness", threshold: "id unique", severity: "medium" },
      ],
    ])
  })
})

describe("Access tab", () => {
  it("shows what the reader may do and the policies bound to the asset", async () => {
    stubApi({ permissions: ["catalog:read", "query:read"] })
    url.search = "tab=access"
    renderTabs({
      ...BRONZE,
      schema: [{ name: "email", dataType: "String", masked: true }],
      policySummary: [
        { id: "p1", name: "mask-email", effect: "Permit with obligation", kind: "Row filter", status: "ready", table: "bronze.demo_orders", appliesToYou: true, mask: ["email"] },
      ],
    })

    const lineage = (await screen.findByText("lineage:read")).closest("li") as HTMLElement
    expect(within(lineage).getByText("Not granted")).toBeTruthy()
    await waitFor(() => {
      const rows = screen.getByText("query:read").closest("li") as HTMLElement
      expect(within(rows).getByText("Granted")).toBeTruthy()
    })

    const policy = screen.getByText("mask-email").closest("li") as HTMLElement
    expect(within(policy).getByText("Applies to you")).toBeTruthy()
    // No `policy:read`: the name is not a link into the policies page.
    expect(screen.getByText("mask-email").closest("a")).toBeNull()
  })
})

describe("Activity tab: usage and history", () => {
  it("counts everyone's queries, lists only the reader's own, and shows who changed what", async () => {
    stubApi()
    url.search = "tab=activity"
    renderTabs({
      ...BRONZE,
      usage: { queries7d: 12, users7d: 3, avgLatencyMs: 240 },
      recentQueries: [
        { id: "q-9", sql: "SELECT count() FROM bronze.demo_orders", user: "Reader", at: "2026-09-30T08:00:00Z", status: "completed", auditEventId: "audit-77" },
      ],
      changeHistory: [{ id: "audit-1", at: "2026-09-30T09:00:00Z", actor: "Rina", summary: "Edited description, owner" }],
    })

    const usage = screen.getByText("Usage (7d)").closest("[data-slot=card]") as HTMLElement
    expect(within(usage).getByText("12")).toBeTruthy()
    expect(within(usage).getByText("240 ms")).toBeTruthy()
    const mine = within(usage).getByText("SELECT count() FROM bronze.demo_orders").closest("li") as HTMLElement
    expect(within(mine).getByText("Audit").getAttribute("href")).toBe("/audit?event=audit-77")

    const history = screen.getByText("Change history").closest("[data-slot=card]") as HTMLElement
    expect(within(history).getByText("Rina")).toBeTruthy()
    expect(within(history).getByText("Edited description, owner")).toBeTruthy()
  })

  it("says nobody queried the table, rather than that usage is unknown", () => {
    stubApi()
    url.search = "tab=activity"
    renderTabs({ ...BRONZE, usage: { queries7d: 0, users7d: 0, avgLatencyMs: 0 } })
    expect(screen.getByText("No queries in the last 7 days")).toBeTruthy()
  })
})

describe("Lineage tab: dependents", () => {
  it("links a saved query and a dashboard that read the asset", async () => {
    stubApi({ lineage: NO_LINEAGE })
    url.search = "tab=lineage"
    renderTabs({
      ...BRONZE,
      dependents: [
        { id: "sq-1", name: "Orders by day", kind: "saved query" },
        { id: "b_42", name: "Sales board", kind: "dashboard", detail: "3 charts" },
      ],
    })

    const card = (await screen.findByText("Dependents")).closest("[data-slot=card]") as HTMLElement
    expect(within(card).getByText("Orders by day").closest("a")?.getAttribute("href")).toBe("/query-studio?saved=sq-1")
    expect(within(card).getByText("Sales board").closest("a")?.getAttribute("href")).toBe("/dashboards?board=b_42")
    expect(within(card).getByText("3 charts")).toBeTruthy()
    await waitFor(() => expect(screen.getByRole("tab", { name: /Lineage/ }).textContent).toBe("Lineage2"))
  })
})

describe("About card", () => {
  it("lets someone with catalog:write edit the details, and reloads the asset", async () => {
    const fetchSpy = stubApi()
    const reloaded = mock(() => {})
    renderTabs(
      {
        ...BRONZE,
        owner: "Data Platform",
        steward: "Rina",
        tags: ["finance"],
        annotation: { owner: "Data Platform", steward: "Rina", tags: ["finance"], description: null },
        registry: { owner: "dagster", description: "Orders ingested from Postgres." },
      },
      reloaded
    )

    expect(screen.getByText("finance")).toBeTruthy()
    fireEvent.click(await screen.findByText("Edit"))
    // The form holds the annotation; the registry's words are the placeholder.
    expect((screen.getByLabelText("Owner") as HTMLInputElement).value).toBe("Data Platform")
    expect(screen.getByLabelText("Description").getAttribute("placeholder")).toBe("Orders ingested from Postgres.")
    fireEvent.change(screen.getByLabelText("Description"), { target: { value: " One row per order. " } })
    fireEvent.change(screen.getByLabelText("Tags"), { target: { value: "finance, monthly-report" } })
    fireEvent.click(screen.getByRole("button", { name: "Save" }))

    await waitFor(() => expect(reloaded).toHaveBeenCalled())
    expect(writes(fetchSpy)).toEqual([
      [
        "PUT",
        "/api/catalog/demo-orders/annotation",
        { description: "One row per order.", owner: "Data Platform", steward: "Rina", tags: ["finance", "monthly-report"] },
      ],
    ])
  })

  it("refuses a tag the API would refuse, before sending anything", async () => {
    const fetchSpy = stubApi()
    renderTabs()

    fireEvent.click(await screen.findByText("Edit"))
    fireEvent.change(screen.getByLabelText("Tags"), { target: { value: "Finance Team" } })
    expect(screen.getByText(/"Finance" is not a valid tag/)).toBeTruthy()
    expect((screen.getByRole("button", { name: "Save" }) as HTMLButtonElement).disabled).toBe(true)
    expect(writes(fetchSpy)).toEqual([])
  })

  it("offers no edit without catalog:write", async () => {
    stubApi({ permissions: ["catalog:read"] })
    renderTabs()
    await screen.findByText("About")
    await waitFor(() => expect(screen.queryByText("Edit")).toBeNull())
  })
})

describe("Health and classification", () => {
  it("says what the health rests on, signal by signal", async () => {
    stubApi()
    renderTabs({
      ...BRONZE,
      health: "degraded",
      healthReasons: ["Late: written 8d 3h ago, expected within 36h 00m", "1 of 2 quality checks passed"],
    })

    const tile = (await screen.findByText("Health")).closest("div") as HTMLElement
    expect(within(tile).getByText("Degraded")).toBeTruthy()
    expect(within(tile).getByText("Late: written 8d 3h ago, expected within 36h 00m")).toBeTruthy()
    expect(within(tile).getByText("1 of 2 quality checks passed")).toBeTruthy()
  })

  it("says nothing measures the asset, rather than leaving Unknown unexplained", async () => {
    stubApi()
    renderTabs({ ...BRONZE, health: "unknown", healthReasons: [] })

    const tile = (await screen.findByText("Health")).closest("div") as HTMLElement
    expect(within(tile).getByText(/Nothing measures this asset yet/)).toBeTruthy()
  })

  it("shows the asset's and its columns' classification, and where it comes from", () => {
    stubApi()
    url.search = "tab=access"
    renderTabs({
      ...BRONZE,
      classification: "restricted",
      classificationSource: "rule",
      schema: [
        { name: "id", dataType: "Int64" },
        { name: "email", dataType: "String", classification: "restricted" },
      ],
    })

    const card = screen.getByText("Classification").closest("[data-slot=card]") as HTMLElement
    expect(within(card).getByText(/Set by a classification rule/)).toBeTruthy()
    const email = within(card).getByText("email").closest("li") as HTMLElement
    expect(within(email).getByText("Restricted")).toBeTruthy()
  })

  it("classifies a column by adding a rule for the table key, then reloads", async () => {
    const fetchSpy = stubApi()
    const reloaded = mock(() => {})
    url.search = "tab=access"
    renderTabs(BRONZE, reloaded)

    const card = screen.getByText("Classification").closest("[data-slot=card]") as HTMLElement
    expect(within(card).getByText(/The default level/)).toBeTruthy()
    fireEvent.click(within(card).getByText("Classify"))
    fireEvent.change(screen.getByLabelText("Applies to"), { target: { value: "amount" } })
    fireEvent.change(screen.getByLabelText("Classification"), { target: { value: "confidential" } })
    fireEvent.click(screen.getByRole("button", { name: "Save" }))

    await waitFor(() => expect(reloaded).toHaveBeenCalled())
    expect(writes(fetchSpy)).toEqual([
      ["POST", "/api/governance/classification", { asset: "bronze.demo_orders", column: "amount", classification: "confidential" }],
    ])
  })
})

describe("Quality tab: deleting a rule", () => {
  const checks: AssetDetail["qualityChecks"] = [
    { id: "q1", name: "row_count", dimension: "completeness", status: "passed", lastRun: "2026-09-30T00:00:00Z", origin: "observed" },
    { id: "q3", name: "email_complete", dimension: "completeness", status: null, lastRun: null, threshold: ">= 95%", origin: "rule", evaluable: false, hint: "Write the threshold as one of: …" },
  ]

  it("deletes an authored rule after a confirmation, then reloads the asset", async () => {
    const fetchSpy = stubApi()
    const reloaded = mock(() => {})
    url.search = "tab=quality"
    renderTabs({ ...BRONZE, qualityChecks: checks }, reloaded)

    // The observed check is the quality job's own: nothing to delete.
    expect(screen.queryByLabelText("Delete rule row_count")).toBeNull()
    fireEvent.click(await screen.findByLabelText("Delete rule email_complete"))
    expect(writes(fetchSpy)).toEqual([])
    fireEvent.click(screen.getByRole("button", { name: "Delete rule" }))

    await waitFor(() => expect(reloaded).toHaveBeenCalled())
    expect(writes(fetchSpy)).toEqual([["DELETE", "/api/governance/quality/q3", undefined]])
  })

  it("offers no delete without governance:write", async () => {
    stubApi({ permissions: ["catalog:read", "query:read", "governance:read"] })
    url.search = "tab=quality"
    renderTabs({ ...BRONZE, qualityChecks: checks })

    await screen.findByText("email_complete")
    await waitFor(() => expect(screen.queryByLabelText("Delete rule email_complete")).toBeNull())
  })
})

describe("Activity tab: freshness target", () => {
  it("says no target is set, and sets one as an SLA on the table key", async () => {
    const fetchSpy = stubApi()
    const reloaded = mock(() => {})
    url.search = "tab=activity"
    renderTabs(BRONZE, reloaded)

    expect(screen.getByText(/No target is set/)).toBeTruthy()
    fireEvent.click(await screen.findByText("Set target"))
    fireEvent.change(screen.getByLabelText("New data at least every"), { target: { value: "0" } })
    expect((screen.getByRole("button", { name: "Save target" }) as HTMLButtonElement).disabled).toBe(true)
    fireEvent.change(screen.getByLabelText("New data at least every"), { target: { value: "6" } })
    fireEvent.change(screen.getByLabelText("Unit"), { target: { value: "hours" } })
    fireEvent.click(screen.getByRole("button", { name: "Save target" }))

    await waitFor(() => expect(reloaded).toHaveBeenCalled())
    expect(writes(fetchSpy)).toEqual([
      ["PUT", "/api/governance/sla", { tableName: "bronze.demo_orders", expectedIntervalMinutes: 360 }],
    ])
  })

  it("says where the target comes from, and opens on the one already set", async () => {
    stubApi()
    url.search = "tab=activity"
    renderTabs({ ...BRONZE, freshnessTargetSeconds: 172800, freshnessTargetSource: "sla" })

    expect(screen.getByText(/by its freshness SLA/)).toBeTruthy()
    fireEvent.click(await screen.findByText("Change target"))
    expect((screen.getByLabelText("New data at least every") as HTMLInputElement).value).toBe("2")
    expect((screen.getByLabelText("Unit") as HTMLSelectElement).value).toBe("days")
  })
})

describe("Sample tab", () => {
  it("shows the rows the asset carries, and asks for more only when asked", async () => {
    const fetchSpy = stubApi()
    url.search = "tab=sample"
    renderTabs()

    const sizes = screen.getByRole("group", { name: "Rows to show" })
    expect(within(sizes).getByText("5").getAttribute("aria-pressed")).toBe("true")
    expect(screen.getAllByRole("row")).toHaveLength(2)
    expect(fetchSpy.mock.calls.some((c) => String(c[0]).includes("/sample"))).toBe(false)

    fireEvent.click(within(sizes).getByText("25"))
    await waitFor(() => expect(screen.getAllByRole("row")).toHaveLength(26))
    expect(fetchSpy.mock.calls.some((c) => String(c[0]).endsWith("/api/catalog/demo-orders/sample?limit=25"))).toBe(true)
  })

  it("offers no row count to a reader who may not read rows", () => {
    stubApi()
    url.search = "tab=sample"
    renderTabs({ ...BRONZE, sample: [], sampleRestricted: true })

    expect(screen.getByText("Sample rows need query access")).toBeTruthy()
    expect(screen.queryByRole("group", { name: "Rows to show" })).toBeNull()
  })
})

describe("Access tab: adding a policy", () => {
  it("adds a masking policy bound to the table its rows are read from", async () => {
    const fetchSpy = stubApi()
    const reloaded = mock(() => {})
    url.search = "tab=access"
    renderTabs(BRONZE, reloaded)

    fireEvent.click(await screen.findByText("Add policy"))
    // Nothing to enforce yet: no role, no column, no filter.
    const save = () => screen.getByRole("button", { name: "Save draft" }) as HTMLButtonElement
    expect(save().disabled).toBe(true)
    fireEvent.change(screen.getByLabelText("Roles it applies to"), { target: { value: "Analyst, Dashboard Viewer" } })
    expect(save().disabled).toBe(true)
    fireEvent.click(screen.getByLabelText("amount"))
    fireEvent.click(save())

    await waitFor(() => expect(reloaded).toHaveBeenCalled())
    expect(writes(fetchSpy)).toEqual([
      [
        "POST",
        "/api/governance/policies",
        {
          name: "govern_bronze_demo_orders",
          kind: "Column mask",
          subjects: "Analyst, Dashboard Viewer",
          resources: "bronze.demo_orders",
          effect: "Permit with obligation",
          conditions: JSON.stringify({ roles: ["Analyst", "Dashboard Viewer"], table: "bronze.demo_orders", mask: ["amount"] }),
          activate: false,
        },
      ],
    ])
  })

  it("offers no policy form without policy:write", async () => {
    stubApi({ permissions: ["catalog:read", "policy:read"] })
    url.search = "tab=access"
    renderTabs()

    await screen.findByText("Open policies")
    expect(screen.queryByText("Add policy")).toBeNull()
  })
})

describe("Quality tab: rewriting a rule", () => {
  const checks: AssetDetail["qualityChecks"] = [
    { id: "q3", name: "email_complete", asset: "bronze.demo_orders", dimension: "completeness", status: null, lastRun: null, threshold: ">= 95%", severity: "high", origin: "rule", evaluable: false, hint: "Write the threshold as one of: …" },
  ]

  it("rewrites the threshold of a rule that cannot be run, then reloads the asset", async () => {
    const fetchSpy = stubApi()
    const reloaded = mock(() => {})
    url.search = "tab=quality"
    renderTabs({ ...BRONZE, qualityChecks: checks }, reloaded)

    fireEvent.click(await screen.findByLabelText("Edit rule email_complete"))
    // The form opens on the rule as it is; saving it unchanged is nothing to save.
    expect((screen.getByLabelText("Table") as HTMLInputElement).value).toBe("bronze.demo_orders")
    expect((screen.getByLabelText("Severity") as HTMLSelectElement).value).toBe("high")
    const save = screen.getByRole("button", { name: "Save rule" }) as HTMLButtonElement
    expect(save.disabled).toBe(true)
    fireEvent.change(screen.getByLabelText("Threshold"), { target: { value: " amount not null >= 95% " } })
    expect(screen.getByText(/results recorded for this rule are cleared/)).toBeTruthy()
    fireEvent.click(save)

    await waitFor(() => expect(reloaded).toHaveBeenCalled())
    expect(writes(fetchSpy)).toEqual([
      ["PUT", "/api/governance/quality/q3", { asset: "bronze.demo_orders", threshold: "amount not null >= 95%", severity: "high" }],
    ])
  })

  it("offers no edit without governance:write", async () => {
    stubApi({ permissions: ["catalog:read", "query:read", "governance:read"] })
    url.search = "tab=quality"
    renderTabs({ ...BRONZE, qualityChecks: checks })

    await screen.findByText("email_complete")
    await waitFor(() => expect(screen.queryByLabelText("Edit rule email_complete")).toBeNull())
  })
})

describe("Activity tab: one history of changes", () => {
  it("places the table's schema versions among what people changed, newest first", async () => {
    stubApi()
    url.search = "tab=activity"
    renderTabs({
      ...BRONZE,
      changeHistory: [
        { id: "audit-2", at: "2026-09-30T09:00:00Z", actor: "Rina", summary: "Edited description" },
        { id: "audit-1", at: "2026-09-21T09:00:00Z", actor: "Budi", summary: "Added quality rule orders_id_unique" },
      ],
    })

    const history = screen.getByText("Change history").closest("[data-slot=card]") as HTMLElement
    await waitFor(() => expect(within(history).getAllByRole("listitem")).toHaveLength(4))
    expect(within(history).getAllByRole("listitem").map((li) => li.textContent)).toEqual([
      expect.stringContaining("RinaEdited description"),
      expect.stringContaining("Schema v1Added amount (decimal(12, 2))"),
      expect.stringContaining("BudiAdded quality rule orders_id_unique"),
      expect.stringContaining("Schema v0Created with 2 columns"),
    ])
  })
})

describe("Access tab: enforcing and deleting a policy", () => {
  const policy = (status: string): AssetDetail["policySummary"][number] => ({
    id: "p1",
    name: "mask-amount",
    effect: "Permit with obligation",
    kind: "Column mask",
    status,
    table: "bronze.demo_orders",
    appliesToYou: false,
    roles: ["Analyst"],
    mask: ["amount"],
  })

  it("enforces a draft after saying what that changes, then reloads the asset", async () => {
    const fetchSpy = stubApi()
    const reloaded = mock(() => {})
    url.search = "tab=access"
    renderTabs({ ...BRONZE, policySummary: [policy("draft")] }, reloaded)

    const item = screen.getByText("mask-amount").closest("li") as HTMLElement
    expect(within(item).getByText("draft · not enforced")).toBeTruthy()
    fireEvent.click(await within(item).findByRole("button", { name: "Enforce" }))
    expect(screen.getByText(/read its masked columns as \*\*\*/)).toBeTruthy()
    expect(writes(fetchSpy)).toEqual([])
    fireEvent.click(screen.getByRole("button", { name: "Enforce" }))

    await waitFor(() => expect(reloaded).toHaveBeenCalled())
    expect(writes(fetchSpy)).toEqual([["PUT", "/api/governance/policies/p1/status", { status: "ready" }]])
  })

  it("stops enforcing a policy back to a draft", async () => {
    const fetchSpy = stubApi()
    const reloaded = mock(() => {})
    url.search = "tab=access"
    renderTabs({ ...BRONZE, policySummary: [policy("ready")] }, reloaded)

    fireEvent.click(await screen.findByRole("button", { name: "Stop enforcing" }))
    expect(screen.getByText(/read unmasked and unfiltered\. The policy is kept as a draft/)).toBeTruthy()
    fireEvent.click(within(screen.getByRole("dialog")).getByRole("button", { name: "Stop enforcing" }))

    await waitFor(() => expect(reloaded).toHaveBeenCalled())
    expect(writes(fetchSpy)).toEqual([["PUT", "/api/governance/policies/p1/status", { status: "draft" }]])
  })

  it("deletes a policy, warning when it is the one being enforced", async () => {
    const fetchSpy = stubApi()
    const reloaded = mock(() => {})
    url.search = "tab=access"
    renderTabs({ ...BRONZE, policySummary: [policy("ready")] }, reloaded)

    fireEvent.click(await screen.findByLabelText("Delete policy mask-amount"))
    expect(screen.getByText(/It is being enforced/)).toBeTruthy()
    fireEvent.click(screen.getByRole("button", { name: "Delete policy" }))

    await waitFor(() => expect(reloaded).toHaveBeenCalled())
    expect(writes(fetchSpy)).toEqual([["DELETE", "/api/governance/policies/p1", undefined]])
  })

  it("offers neither without policy:write", async () => {
    stubApi({ permissions: ["catalog:read", "policy:read"] })
    url.search = "tab=access"
    renderTabs({ ...BRONZE, policySummary: [policy("draft")] })

    await screen.findByText("Open policies")
    expect(screen.queryByRole("button", { name: "Enforce" })).toBeNull()
    expect(screen.queryByLabelText("Delete policy mask-amount")).toBeNull()
  })
})

describe("Overview: storage, and saying things once", () => {
  const GOLD: AssetDetail = {
    ...BRONZE,
    id: "serving.mart_orders",
    name: "Mart Orders",
    namespace: "serving",
    type: "table",
    layer: "gold",
    tier: "hot",
    format: "ClickHouse ReplacingMergeTree",
    tableName: undefined,
    tableKey: "serving.mart_orders",
    queryTarget: undefined,
    storage: {
      table: "serving.mart_orders",
      engine: "ReplacingMergeTree",
      partitionKey: null,
      sortingKey: "plnt, material",
      parts: 3,
      partitions: 1,
      bytesOnDisk: 807944,
      uncompressedBytes: 8000498,
      tableColumns: 18,
    },
  }

  it("shows how ClickHouse holds a Gold table, which used to have no storage card", async () => {
    stubApi()
    renderTabs(GOLD)

    const card = (await screen.findByText("Storage")).closest("[data-slot=card]") as HTMLElement
    expect(within(card).getByText("ClickHouse table serving.mart_orders")).toBeTruthy()
    expect(within(card).getByText("ReplacingMergeTree")).toBeTruthy()
    expect(within(card).getByText("plnt, material")).toBeTruthy()
    expect(within(card).getByText("Unpartitioned")).toBeTruthy()
    expect(within(card).getByText(/collapsed into the newest one when parts merge/)).toBeTruthy()
    // One partition of an unpartitioned table is not a count worth a row.
    expect(within(card).queryByText("Partitions")).toBeNull()
  })

  it("says a view stores nothing, rather than listing zeroes for it", async () => {
    stubApi()
    renderTabs({
      ...GOLD,
      type: "view",
      storage: { ...GOLD.storage!, engine: "View", sortingKey: null, parts: 0, bytesOnDisk: 0, uncompressedBytes: 0 },
    })

    const card = (await screen.findByText("Storage")).closest("[data-slot=card]") as HTMLElement
    expect(within(card).getByText(/A view stores no rows of its own/)).toBeTruthy()
    expect(within(card).queryByText("Size on disk")).toBeNull()
  })

  it("leaves a key it could not read unknown, instead of calling the table unsorted", async () => {
    stubApi()
    renderTabs({ ...GOLD, storage: { ...GOLD.storage!, engine: null, sortingKey: null, parts: null, bytesOnDisk: null } })

    const card = (await screen.findByText("Storage")).closest("[data-slot=card]") as HTMLElement
    expect(within(card).queryByText("Unsorted")).toBeNull()
    expect(within(card).queryByText("Unpartitioned")).toBeNull()
  })

  it("does not repeat the description the page header already shows", async () => {
    stubApi()
    renderTabs()

    const about = (await screen.findByText("About")).closest("[data-slot=card]") as HTMLElement
    expect(within(about).queryByText("Orders ingested from Postgres.")).toBeNull()
    expect(within(about).queryByText(/No description yet/)).toBeNull()
  })

  it("asks for a description when there is none", async () => {
    stubApi()
    renderTabs({ ...BRONZE, description: "" })

    const about = (await screen.findByText("About")).closest("[data-slot=card]") as HTMLElement
    expect(within(about).getByText(/No description yet/)).toBeTruthy()
  })

  it("drops the schema row for a table that records no schema version", async () => {
    stubApi()
    renderTabs(GOLD)

    const about = (await screen.findByText("About")).closest("[data-slot=card]") as HTMLElement
    expect(within(about).queryByText("Schema")).toBeNull()
    expect(within(about).getByText("Columns")).toBeTruthy()
  })
})
