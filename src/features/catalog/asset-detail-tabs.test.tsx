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
import { toast } from "sonner"
import { AuthProvider } from "@/features/auth/auth-provider"
import { formatDate } from "@/lib/format"
import { assetService } from "@/services"
import type { AssetDetail } from "@/services/contracts/assets"
import { AssetDetailTabs } from "./asset-detail-tabs"
import { relatedTables } from "./asset-lineage"

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

/** What the sample route answers for a `limit`: the rows, or a response of its own (a failure). */
type SampleAnswer = (limit: number) => Record<string, string | null>[] | Response

function stubApi({
  permissions = ["*:*"],
  lineage = GRAPH,
  profile = PROFILE,
  sample = (limit) => Array.from({ length: limit }, (_, i) => ({ id: String(i + 1), amount: "1.5" })),
}: { permissions?: string[]; lineage?: unknown; profile?: unknown; sample?: SampleAnswer } = {}) {
  return spyOn(globalThis, "fetch").mockImplementation((async (input: RequestInfo | URL, init?: RequestInit) => {
    const path = String(input)
    const method = init?.method ?? "GET"
    if (method === "PUT" && path.endsWith("/annotation")) return json({ ok: true })
    if (method === "PUT" && path.endsWith("/api/governance/sla")) return json({ ok: true })
    if (method === "DELETE" && path.includes("/api/governance/quality/")) return json({ ok: true })
    if (method === "DELETE" && path.includes("/api/governance/sla/")) return json({ ok: true })
    if (method === "DELETE" && path.includes("/api/governance/classification/")) return json({ ok: true })
    if (method === "PUT" && path.includes("/api/governance/quality/")) {
      return json({ id: "q3", name: "email_complete", ...JSON.parse(String(init?.body)), dimension: "completeness", lastStatus: null, lastRunAt: null, evaluable: true, hint: null })
    }
    if (method === "POST" && path.endsWith("/api/governance/policies")) return json({ id: "p-new" }, 201)
    if (method === "PUT" && path.includes("/api/governance/policies/")) return json({ id: "p1", status: JSON.parse(String(init?.body)).status })
    if (method === "DELETE" && path.includes("/api/governance/policies/")) return json({ ok: true })
    if (path.includes("/sample?limit=")) {
      const limit = Number(path.split("limit=")[1])
      const answer = sample(limit)
      return answer instanceof Response ? answer : json({ rows: answer, limit })
    }
    if (method === "POST" && path.endsWith("/run")) return json({ id: "q2", status: "passed", value: "0 repeated values in 50 rows" })
    if (method === "POST" && path.endsWith("/api/governance/quality")) return json({ id: "new" }, 201)
    if (method === "POST" && path.endsWith("/api/governance/classification")) return json({ id: "c1" }, 201)
    if (path.includes("/api/auth/me")) {
      return json({ id: "u1", name: "Reader", email: null, roles: ["Analyst"], permissions, tenants: [] })
    }
    if (path.includes("/api/governance/lineage")) return json(lineage)
    if (path.endsWith("/profile")) return profile instanceof Response ? profile.clone() : json(profile)
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
    // Left to right: where it comes from, the asset, then what is built
    // from it — the pipeline as a stop of its own on the way.
    // A name drops the schema its caption already states; the full name is its tooltip.
    expect(within(graph).getAllByRole("listitem").map((n) => n.textContent)).toEqual([
      "ConnectorPostgres",
      "Bronzedemo_orders",
      "Pipelineorders_clean",
      "Silverorders_clean",
    ])
    expect(within(graph).getByTitle("orders_clean").closest("a")?.getAttribute("href")).toBe("/pipelines/pl-clean")
    // Its neighbours open their own pages; the asset itself is not a link to itself.
    expect(within(graph).getByText("Postgres").closest("a")?.getAttribute("href")).toBe("/connectors/conn-pg")
    expect(within(graph).getByTitle("silver.orders_clean").closest("a")?.getAttribute("href")).toBe(
      "/data/assets/silver.orders_clean"
    )
    expect(within(graph).getByTitle("bronze.demo_orders").closest("a")).toBeNull()
    expect(fetchSpy.mock.calls.some((c) => String(c[0]).endsWith("?focus=bronze.demo_orders"))).toBe(true)

    expect(screen.getByText("Open lineage graph").closest("a")?.getAttribute("href")).toBe(
      "/lineage?focus=bronze.demo_orders"
    )
    // What the graph cannot show is said under it.
    expect(screen.getByText("only edges the platform recorded are drawn")).toBeTruthy()
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
    // One row per column after the header, each led by the button that names it.
    const names = within(columns)
      .getAllByRole("row")
      .slice(1)
      .map((r) => within(r).getByRole("button").textContent)
    expect(names).toEqual(["id", "amount"])
    // `amount` is optional in Iceberg; `id` is required. The row has no Nullable column any
    // more: whether a column can be null is a fact of the opened column.
    const canBeNull = (name: string) => {
      const button = within(columns).getByRole("button", { name })
      fireEvent.click(button)
      const panel = document.getElementById(button.getAttribute("aria-controls") ?? "") as HTMLElement
      return within(panel).getByText("Can be null").nextElementSibling?.textContent
    }
    expect(canBeNull("amount")).toBe("Yes")
    expect(canBeNull("id")).toBe("No")

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
    expect(screen.getByText("No Query Studio or copilot query in the last 7 days")).toBeTruthy()
    expect(screen.queryByText(/Not counted/)).toBeNull()
  })

  // A mart read only by dashboards said "No queries", as if nothing used it.
  it("says which reads the count leaves out, when dashboards use the asset", () => {
    stubApi()
    url.search = "tab=activity"
    renderTabs({
      ...BRONZE,
      usage: { queries7d: 0, users7d: 0, avgLatencyMs: 0 },
      dependents: [
        { id: "default", name: "default", kind: "dashboard", detail: "2 charts" },
        { id: "b_42", name: "Sales board", kind: "dashboard" },
      ],
    })
    expect(screen.getByText("Not counted: reads by the 2 dashboards that use this asset.")).toBeTruthy()
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
  /** The `limit` of every request the page made to the sample route, in order. */
  const sampleLimits = (fetchSpy: ReturnType<typeof stubApi>) =>
    fetchSpy.mock.calls
      .map((c) => String(c[0]))
      .filter((path) => path.includes("/sample?limit="))
      .map((path) => Number(path.split("limit=")[1]))

  it("asks for 25 rows on opening, and shows the rows the page carries until they arrive", async () => {
    const fetchSpy = stubApi()
    url.search = "tab=sample"
    renderTabs()

    const sizes = screen.getByRole("group", { name: "Rows to show" })
    // Labelled, so the numbers read as a choice of how many rows to show.
    expect(within(sizes).getByText("Rows")).toBeTruthy()
    expect(within(sizes).getAllByRole("button").map((b) => b.textContent)).toEqual(["25", "50", "100"])
    expect(within(sizes).getByText("25").getAttribute("aria-pressed")).toBe("true")
    // The detail body's one row is on screen at once (the header row and that row) while 25 load.
    expect(screen.getAllByRole("row")).toHaveLength(2)
    expect(screen.getByText(/Loading 25 rows… showing the first 1 row,/)).toBeTruthy()

    await waitFor(() => expect(screen.getAllByRole("row")).toHaveLength(26))
    expect(screen.getByText(/The first 25 rows, as you would read them/)).toBeTruthy()
    expect(sampleLimits(fetchSpy)).toEqual([25])
    expect(fetchSpy.mock.calls.some((c) => String(c[0]).endsWith("/api/catalog/demo-orders/sample?limit=25"))).toBe(true)
  })

  it("asks again for 50 and for 100 rows when they are chosen", async () => {
    const fetchSpy = stubApi()
    url.search = "tab=sample"
    renderTabs()
    const sizes = screen.getByRole("group", { name: "Rows to show" })
    await waitFor(() => expect(screen.getAllByRole("row")).toHaveLength(26))

    fireEvent.click(within(sizes).getByText("50"))
    await waitFor(() => expect(screen.getAllByRole("row")).toHaveLength(51))
    expect(within(sizes).getByText("50").getAttribute("aria-pressed")).toBe("true")
    fireEvent.click(within(sizes).getByText("100"))
    await waitFor(() => expect(screen.getAllByRole("row")).toHaveLength(101))
    expect(sampleLimits(fetchSpy)).toEqual([25, 50, 100])
  })

  it("shows the rows the page carries, with no size to choose, where the deployment cannot fetch more", () => {
    const fetchSpy = stubApi()
    const fetchMore = assetService.getAssetSample
    assetService.getAssetSample = undefined
    try {
      url.search = "tab=sample"
      renderTabs()

      expect(screen.queryByRole("group", { name: "Rows to show" })).toBeNull()
      expect(screen.getAllByRole("row")).toHaveLength(2)
      expect(screen.getByText(/The first 1 row, as you would read them/)).toBeTruthy()
      expect(screen.queryByText(/Loading/)).toBeNull()
      expect(sampleLimits(fetchSpy)).toEqual([])
    } finally {
      assetService.getAssetSample = fetchMore
    }
  })

  it("offers no row count to a reader who may not read rows, and asks for no rows", () => {
    const fetchSpy = stubApi()
    url.search = "tab=sample"
    renderTabs({ ...BRONZE, sample: [], sampleRestricted: true })

    expect(screen.getByText("Sample rows need query access")).toBeTruthy()
    expect(screen.queryByRole("group", { name: "Rows to show" })).toBeNull()
    expect(sampleLimits(fetchSpy)).toEqual([])
  })

  it("says there are no rows when the table has none, and why when the rows cannot be read", async () => {
    stubApi({ sample: () => [] })
    url.search = "tab=sample"
    renderTabs({ ...BRONZE, sample: [] })

    expect(await screen.findByText("No sample rows available")).toBeTruthy()
    expect(screen.queryByRole("grid")).toBeNull()
    cleanup()

    // No rows at all: the error stands alone.
    stubApi({ sample: () => json({ error: "sample read failed" }, 502) })
    renderTabs({ ...BRONZE, sample: [] })
    expect(await screen.findByText(/sample read failed/)).toBeTruthy()
    expect(screen.getByRole("button", { name: /Retry/ })).toBeTruthy()
    expect(screen.queryByRole("grid")).toBeNull()
  })

  it("keeps the rows the page has when the larger sample fails, and says so above them with Retry", async () => {
    const fetchSpy = stubApi({ sample: () => json({ error: "sample read failed" }, 502) })
    url.search = "tab=sample"
    renderTabs()

    const alert = await screen.findByRole("alert")
    expect(alert.textContent).toContain("The larger sample could not be loaded. Showing the 1 row the page already has.")
    // Upstream text is not repeated, and the grid and its description say only what is shown.
    expect(alert.textContent).not.toContain("sample read failed")
    expect(document.querySelectorAll("table[role=grid] tbody tr")).toHaveLength(1)
    expect(screen.getByText(/^The first 1 row, as you would read them/)).toBeTruthy()
    fireEvent.click(within(alert).getByRole("button", { name: "Retry" }))
    await waitFor(() => expect(fetchSpy.mock.calls.filter((c) => String(c[0]).includes("/sample?limit=")).length).toBe(2))
  })

  it("puts the row count and Query Studio in the card's body, not the header slot that cannot shrink", () => {
    stubApi()
    url.search = "tab=sample"
    renderTabs()

    const card = screen.getByText("Sample rows").closest("[data-slot=card]") as HTMLElement
    const controls = within(card).getByRole("group", { name: "Rows to show" })
    expect(card.querySelector("[data-slot=card-header]")?.contains(controls)).toBe(false)
    expect(card.querySelector("[data-slot=card-content]")?.contains(controls)).toBe(true)
    const studio = within(card).getByText("Open in Query Studio")
    expect(card.querySelector("[data-slot=card-header]")?.contains(studio)).toBe(false)
  })
})

describe("Sample tab: the data preview", () => {
  const LONG = "a-value-of-sixty-characters-that-no-column-is-wide-enough-for-"
  /**
   * Three rows over a number column, a text column that can be null, a number
   * column with a gap, a masked column, and `free`, which the schema does not
   * list. The ids are as stored: `0250161` is not 250,161.
   */
  const ROWS: Record<string, string | null>[] = [
    { id: "10", city: "Jakarta", amount: "9.5", email: "***", free: "NULL" },
    { id: "9", city: null, amount: "100", email: "***", free: "" },
    { id: "0250161", city: "", amount: null, email: "***", free: LONG },
  ]
  const PREVIEW: AssetDetail = {
    ...BRONZE,
    schema: [
      { name: "id", dataType: "Int64" },
      { name: "city", dataType: "Nullable(String)" },
      { name: "amount", dataType: "Decimal(12, 2)" },
      { name: "email", dataType: "String", masked: true },
    ],
    sample: ROWS,
  }

  // The grid's structure is read through the DOM, not role queries: a role query walks every
  // ancestor's computed style, and a few hundred of them over a 15-cell grid time the test out.
  // The roles themselves are asserted once, in the first tests.
  const grid = () => document.querySelector("table[role=grid]") as HTMLElement
  const headers = () => Array.from(grid().querySelectorAll<HTMLElement>("thead th[scope=col]")).slice(1)
  const nameOf = (h: HTMLElement) => h.querySelector("[data-head=name]")?.textContent
  const headerOf = (name: string) => headers().find((h) => nameOf(h) === name) as HTMLElement
  const bodyRows = () => Array.from(grid().querySelectorAll<HTMLElement>("tbody tr"))
  const cellsOf = (row: HTMLElement) => Array.from(row.querySelectorAll<HTMLElement>("td[role=gridcell]"))
  const cellOf = (row: number, name: string) => cellsOf(bodyRows()[row])[headers().indexOf(headerOf(name))]
  const columnOf = (name: string) => {
    const at = headers().indexOf(headerOf(name))
    return bodyRows().map((r) => cellsOf(r)[at].textContent)
  }
  const rowNumbers = () => bodyRows().map((r) => r.querySelector("th")?.textContent)
  const amountSort = (name: string) => within(headerOf("amount")).getByRole("button", { name }) as HTMLElement
  const inspector = () => document.querySelector<HTMLElement>("[data-slot=sample-inspector]")
  const profileRequests = (fetchSpy: ReturnType<typeof stubApi>) =>
    fetchSpy.mock.calls.filter((c) => String(c[0]).endsWith("/profile")).length

  /** Opens the tab on `PREVIEW` and waits until the 25-row answer has replaced the page's own rows. */
  async function openPreview(api: Parameters<typeof stubApi>[0] = {}, asset: AssetDetail = PREVIEW) {
    const fetchSpy = stubApi({ sample: () => ROWS, ...api })
    url.search = "tab=sample"
    renderTabs(asset)
    // The card's description says "The first 3 rows" once the 25-row answer is in.
    await screen.findByText(/^The first 3 rows, as you would read them/)
    return fetchSpy
  }

  it("heads each column with a glyph, its name and its type, and a column the schema does not list with the name alone", async () => {
    await openPreview()

    expect(screen.getByRole("grid", { name: "Sample rows" })).toBe(grid())
    // The grid's sticky z-indexes stay inside its frame, under the page's own sticky tab bar.
    expect(grid().closest(".isolate")).not.toBeNull()
    expect(headers().map(nameOf)).toEqual(["id", "city", "amount", "email", "free"])
    const amount = headerOf("amount")
    expect(within(amount).getByRole("img", { name: "Number" })).toBeTruthy()
    expect(within(amount).getByText("Decimal(12, 2)")).toBeTruthy()
    expect(within(headerOf("city")).getByRole("img", { name: "Text" })).toBeTruthy()
    expect(within(headerOf("city")).getByText("Nullable(String)")).toBeTruthy()
    // Not listed: no glyph, no type, never a guess from the values.
    const free = headerOf("free")
    expect(within(free).queryByRole("img")).toBeNull()
    expect(free.textContent).toBe("free")
  })

  it("writes NULL and an empty text out, right-aligns numbers, and shows a masked value and an id as they come", async () => {
    await openPreview()

    expect(columnOf("city")).toEqual(["Jakarta", "NULL", "(empty)"])
    const nullCell = cellOf(1, "city")
    expect(nullCell.querySelector(".italic")?.textContent).toBe("NULL")
    expect(cellOf(2, "city").querySelector(".italic")?.textContent).toBe("(empty)")
    // The text "NULL" in a cell is a value, not the absence of one: it is not set in the quiet tone.
    expect(cellOf(0, "free").textContent).toBe("NULL")
    expect(cellOf(0, "free").querySelector(".italic")).toBeNull()
    expect(columnOf("email")).toEqual(["***", "***", "***"])
    // An id is not reformatted.
    expect(columnOf("id")).toEqual(["10", "9", "0250161"])
    // Numbers are right-aligned (in a number column, `NULL` too); text and an unlisted column are not.
    expect(cellOf(0, "amount").className).toContain("text-right")
    expect(cellOf(0, "amount").firstElementChild?.className).toContain("tabular-nums")
    expect(cellOf(2, "amount").className).toContain("text-right")
    expect(cellOf(0, "city").className).not.toContain("text-right")
    expect(cellOf(0, "free").className).not.toContain("text-right")
  })

  it("gives a long value its whole as a title, since the cell cuts it", async () => {
    await openPreview()

    expect(cellOf(2, "free").textContent).toBe(LONG)
    expect(cellOf(2, "free").firstElementChild?.getAttribute("title")).toBe(LONG)
    // A short value needs no title.
    expect(cellOf(0, "city").firstElementChild?.getAttribute("title")).toBeNull()
  })

  it("numbers the rows from 1, and the numbers follow the order the rows are shown in", async () => {
    await openPreview()
    expect(rowNumbers()).toEqual(["1", "2", "3"])

    fireEvent.click(within(headerOf("id")).getByRole("button", { name: "Sort id ascending" }))
    expect(columnOf("id")).toEqual(["9", "10", "0250161"])
    expect(rowNumbers()).toEqual(["1", "2", "3"])
    // The row that was second is first now.
    expect(within(bodyRows()[0]).getByText("NULL", { selector: ".italic" })).toBeTruthy()
  })

  it("sorts the rows shown, ascending, descending, then back to the table's order, and says so", async () => {
    const fetchSpy = await openPreview()
    // The page's other loads (the signed-in user, the table, the lineage) are
    // still settling when the rows arrive, so the total request count is not
    // steady here. What a sort must not ask for is rows or the profile.
    const dataRequests = () =>
      fetchSpy.mock.calls.filter((c) => /\/sample\?limit=|\/profile$/.test(String(c[0]))).length
    const calls = dataRequests()
    const idHeader = headerOf("id")
    expect(idHeader.getAttribute("aria-sort")).toBeNull()

    fireEvent.click(within(idHeader).getByRole("button", { name: "Sort id ascending" }))
    // Numbers as numbers: 9 before 10 before 250,161, and the id as stored.
    expect(columnOf("id")).toEqual(["9", "10", "0250161"])
    expect(idHeader.getAttribute("aria-sort")).toBe("ascending")
    expect(screen.getByText(/Sorted by id \(ascending\), among the rows shown\./)).toBeTruthy()

    fireEvent.click(within(idHeader).getByRole("button", { name: "Sort id descending" }))
    expect(columnOf("id")).toEqual(["0250161", "10", "9"])
    expect(idHeader.getAttribute("aria-sort")).toBe("descending")
    expect(screen.getByText(/Sorted by id \(descending\), among the rows shown\./)).toBeTruthy()

    fireEvent.click(within(idHeader).getByRole("button", { name: "Stop sorting by id" }))
    expect(columnOf("id")).toEqual(["10", "9", "0250161"])
    expect(idHeader.getAttribute("aria-sort")).toBeNull()
    expect(screen.queryByText(/Sorted by/)).toBeNull()
    // Sorting orders what is on screen: it asks the API for no rows and no profile.
    expect(dataRequests()).toBe(calls)
  })

  it("starts another column at ascending, and puts NULL last either way", async () => {
    await openPreview()

    fireEvent.click(within(headerOf("id")).getByRole("button", { name: "Sort id ascending" }))
    fireEvent.click(amountSort("Sort amount ascending"))
    expect(headerOf("id").getAttribute("aria-sort")).toBeNull()
    expect(headerOf("amount").getAttribute("aria-sort")).toBe("ascending")
    expect(columnOf("amount")).toEqual(["9.5", "100", "NULL"])
    fireEvent.click(amountSort("Sort amount descending"))
    expect(columnOf("amount")).toEqual(["100", "9.5", "NULL"])
  })

  it("keeps a picked row picked when a sort moves it", async () => {
    await openPreview()

    // The NULL city is the second row; sorted by id it is the first.
    fireEvent.click(cellOf(1, "city"))
    expect(cellOf(1, "city").getAttribute("aria-selected")).toBe("true")
    fireEvent.click(within(headerOf("id")).getByRole("button", { name: "Sort id ascending" }))
    expect(cellOf(0, "city").getAttribute("aria-selected")).toBe("true")
    expect(cellOf(1, "city").getAttribute("aria-selected")).toBe("false")
    const panel = inspector() as HTMLElement
    expect(within(panel).getByText("Row").nextElementSibling?.textContent).toBe("1")
  })

  describe("the inspector", () => {
    const clipboard = { writeText: mock(async (value: string) => void value) }
    const original = Object.getOwnPropertyDescriptor(navigator, "clipboard")
    const withClipboard = () => {
      clipboard.writeText.mockClear()
      Object.defineProperty(navigator, "clipboard", { value: clipboard, configurable: true })
    }
    afterEach(() => {
      if (original) Object.defineProperty(navigator, "clipboard", original)
      else Reflect.deleteProperty(navigator, "clipboard")
    })

    it("is closed until something is picked, and a pressed cell opens it on its row, column, whole value and Copy", async () => {
      const fetchSpy = await openPreview()
      expect(inspector()).toBeNull()
      // Nothing is asked of the table's data for a profile until a column is shown.
      expect(profileRequests(fetchSpy)).toBe(0)

      fireEvent.click(cellOf(2, "free"))
      const panel = inspector() as HTMLElement
      expect(within(panel).getByText("Cell")).toBeTruthy()
      expect(within(panel).getByText("Row").nextElementSibling?.textContent).toBe("3")
      expect(within(panel).getByText("Column").nextElementSibling?.textContent).toBe("free")
      // The whole value, not the cell's cut of it.
      expect(within(panel).getByText(LONG)).toBeTruthy()
      expect(within(panel).getByRole("button", { name: "Copy value" })).toBeTruthy()
      expect(cellOf(2, "free").getAttribute("aria-selected")).toBe("true")
    })

    it("copies the value, and says it did", async () => {
      withClipboard()
      const toasts = spyOn(toast, "success").mockImplementation(() => 1)
      await openPreview()

      fireEvent.click(cellOf(2, "free"))
      fireEvent.click(within(inspector() as HTMLElement).getByRole("button", { name: "Copy value" }))

      await waitFor(() => expect(toasts).toHaveBeenCalled())
      expect(clipboard.writeText.mock.calls).toEqual([[LONG]])
      expect(toasts.mock.calls[0][0]).toBe("Copied value")
    })

    it("explains NULL and an empty text in a sentence, and offers Copy for a value only", async () => {
      await openPreview()

      fireEvent.click(cellOf(1, "city"))
      let panel = inspector() as HTMLElement
      expect(within(panel).getByText(/the table holds no value in this cell/)).toBeTruthy()
      expect(within(panel).queryByRole("button", { name: "Copy value" })).toBeNull()

      fireEvent.click(cellOf(2, "city"))
      panel = inspector() as HTMLElement
      expect(within(panel).getByText(/the cell holds an empty text/)).toBeTruthy()
      // Nothing to copy in an empty text either.
      expect(within(panel).queryByRole("button", { name: "Copy value" })).toBeNull()

      // A masked value and the text "NULL" are values.
      fireEvent.click(cellOf(0, "email"))
      expect(within(inspector() as HTMLElement).getByRole("button", { name: "Copy value" })).toBeTruthy()
      fireEvent.click(cellOf(0, "free"))
      expect(within(inspector() as HTMLElement).getByRole("button", { name: "Copy value" })).toBeTruthy()
    })

    it("opens on a column when its name is pressed, asking for the profile then and not before", async () => {
      const fetchSpy = await openPreview()
      expect(profileRequests(fetchSpy)).toBe(0)

      fireEvent.click(within(headerOf("amount")).getByRole("button", { name: "amount" }))
      const panel = inspector() as HTMLElement
      expect(within(panel).getByText("Column")).toBeTruthy()
      expect(within(panel).queryByText("Cell")).toBeNull()
      // The loading state is in words while the profile is on its way.
      expect(within(panel).getByText("Profiling columns…")).toBeTruthy()
      // The same opened column the Schema tab shows: the facts, from the table's own profile.
      await waitFor(() => expect(within(panel).getByText(/Statistics over 2,000 rows of/)).toBeTruthy())
      expect(within(panel).getByText("1.5 – 99")).toBeTruthy()
      expect(within(panel).getByText("Can be null").nextElementSibling?.textContent).toBe("Yes")
      expect(within(panel).getByText("Decimal(12, 2)")).toBeTruthy()
      expect(profileRequests(fetchSpy)).toBe(1)
      // The picked column's cells are tinted all the way down.
      expect(cellOf(0, "amount").className).toContain("color-mix")
      expect(cellOf(0, "city").className).not.toContain("color-mix")
    })

    it("shows the column of a picked cell too, and reads the profile once however many are picked", async () => {
      const fetchSpy = await openPreview()

      fireEvent.click(cellOf(0, "amount"))
      const panel = inspector() as HTMLElement
      expect(within(panel).getByText("About the column")).toBeTruthy()
      await waitFor(() => expect(within(panel).getByText("1.5 – 99")).toBeTruthy())
      fireEvent.click(within(panel).getByRole("button", { name: "Close inspector" }))
      expect(inspector()).toBeNull()
      fireEvent.click(cellOf(1, "id"))
      await waitFor(() => expect(within(inspector() as HTMLElement).getByText("1 – 2000", { exact: false })).toBeTruthy())
      expect(profileRequests(fetchSpy)).toBe(1)
    })

    it("opens a column the schema does not list, on the facts that need no schema", async () => {
      await openPreview()

      fireEvent.click(within(headerOf("free")).getByRole("button", { name: "free" }))
      const panel = inspector() as HTMLElement
      expect(within(panel).getByText("Not in the schema")).toBeTruthy()
      await waitFor(() => expect(within(panel).getByText("This column is not in the profile.")).toBeTruthy())
    })

    it("says in words that the profile needs query:read, and asks for none, yet opens the column", async () => {
      const fetchSpy = await openPreview({ permissions: ["catalog:read"] })

      fireEvent.click(within(headerOf("amount")).getByRole("button", { name: "amount" }))
      const panel = inspector() as HTMLElement
      expect(within(panel).getByText(/need the query:read permission/)).toBeTruthy()
      expect(within(panel).getByText("Type")).toBeTruthy()
      expect(profileRequests(fetchSpy)).toBe(0)
    })

    it("says in words when the table cannot be profiled", async () => {
      await openPreview({ profile: { supported: false, reason: "this table's engine cannot be profiled" } })

      fireEvent.click(within(headerOf("amount")).getByRole("button", { name: "amount" }))
      expect(await within(inspector() as HTMLElement).findByText(/No column statistics: this table's engine cannot be profiled/)).toBeTruthy()
    })

    it("says in words when the profile failed to load, with a way to try again", async () => {
      await openPreview({ profile: json({ error: "profile read failed" }, 502) })

      fireEvent.click(within(headerOf("amount")).getByRole("button", { name: "amount" }))
      const panel = inspector() as HTMLElement
      expect(await within(panel).findByText(/Column statistics failed to load: profile read failed/)).toBeTruthy()
      expect(within(panel).getByRole("button", { name: "Retry" })).toBeTruthy()
    })

    it("closes on its button and on Escape, and puts the focus back on the grid", async () => {
      await openPreview()

      fireEvent.click(cellOf(0, "id"))
      fireEvent.click(within(inspector() as HTMLElement).getByRole("button", { name: "Close inspector" }))
      expect(inspector()).toBeNull()
      expect(document.activeElement).toBe(cellOf(0, "id"))

      fireEvent.click(cellOf(0, "id"))
      expect(inspector()).not.toBeNull()
      fireEvent.keyDown(cellOf(0, "id"), { key: "Escape" })
      expect(inspector()).toBeNull()
      expect(cellOf(0, "id").getAttribute("aria-selected")).toBe("false")
      // Escape with nothing open does nothing.
      fireEvent.keyDown(cellOf(0, "id"), { key: "Escape" })
      expect(inspector()).toBeNull()
    })

    it("closes on Escape pressed inside the panel, and gives the focus to the grid", async () => {
      await openPreview()

      fireEvent.click(cellOf(1, "id"))
      const copy = within(inspector() as HTMLElement).getByRole("button", { name: "Copy value" })
      copy.focus()
      fireEvent.keyDown(copy, { key: "Escape" })
      expect(inspector()).toBeNull()
      // The tab stop is the first cell until the reader moves; here it has moved to the picked one.
      expect(document.activeElement).toBe(cellOf(1, "id"))
    })

    it("drops a picked cell when another size is chosen, and keeps a picked column", async () => {
      await openPreview()
      const sizes = screen.getByRole("group", { name: "Rows to show" })

      fireEvent.click(cellOf(0, "id"))
      fireEvent.click(within(sizes).getByText("50"))
      expect(inspector()).toBeNull()

      fireEvent.click(within(headerOf("amount")).getByRole("button", { name: "amount" }))
      fireEvent.click(within(sizes).getByText("100"))
      expect(inspector()).not.toBeNull()
    })
  })

  describe("the keyboard", () => {
    const tabStops = () => bodyRows().flatMap(cellsOf).filter((c) => c.getAttribute("tabindex") === "0")

    it("makes the grid one tab stop, and the header two", async () => {
      await openPreview()

      // 3 rows of 5 cells, and exactly one of them can be tabbed to: the first.
      expect(within(grid()).getAllByRole("gridcell")).toHaveLength(15)
      expect(tabStops()).toEqual([cellOf(0, "id")])
      // The header's tab stops are the active column's name and sort control, not one per column.
      const stops = Array.from(grid().querySelectorAll<HTMLElement>("thead button")).filter(
        (b) => b.getAttribute("tabindex") === "0"
      )
      expect(stops.map((b) => b.getAttribute("data-head"))).toEqual(["name", "sort"])
    })

    it("moves the picked cell with the arrow keys, Home and End, and the inspector follows", async () => {
      await openPreview()

      fireEvent.click(cellOf(0, "id"))
      const panel = () => inspector() as HTMLElement
      const at = () => [
        within(panel()).getByText("Row").nextElementSibling?.textContent,
        within(panel()).getByText("Column").nextElementSibling?.textContent,
      ]
      expect(at()).toEqual(["1", "id"])

      fireEvent.keyDown(cellOf(0, "id"), { key: "ArrowRight" })
      expect(at()).toEqual(["1", "city"])
      expect(document.activeElement).toBe(cellOf(0, "city"))
      // The tab stop moved with the focus.
      expect(tabStops()).toEqual([cellOf(0, "city")])
      fireEvent.keyDown(cellOf(0, "city"), { key: "ArrowDown" })
      expect(at()).toEqual(["2", "city"])
      fireEvent.keyDown(cellOf(1, "city"), { key: "End" })
      expect(at()).toEqual(["2", "free"])
      fireEvent.keyDown(cellOf(1, "free"), { key: "ArrowRight" })
      expect(at()).toEqual(["2", "free"])
      fireEvent.keyDown(cellOf(1, "free"), { key: "Home" })
      expect(at()).toEqual(["2", "id"])
      fireEvent.keyDown(cellOf(1, "id"), { key: "ArrowUp" })
      fireEvent.keyDown(cellOf(0, "id"), { key: "ArrowUp" })
      expect(at()).toEqual(["1", "id"])
    })

    it("moves the focus without opening the inspector, and opens it on Enter", async () => {
      const fetchSpy = await openPreview()

      fireEvent.keyDown(cellOf(0, "id"), { key: "ArrowRight" })
      expect(document.activeElement).toBe(cellOf(0, "city"))
      // Passing through the grid changes nothing on the page and reads nothing.
      expect(inspector()).toBeNull()
      expect(profileRequests(fetchSpy)).toBe(0)

      fireEvent.keyDown(cellOf(0, "city"), { key: "Enter" })
      expect(within(inspector() as HTMLElement).getByText("Column").nextElementSibling?.textContent).toBe("city")
    })

    it("moves along the header with the arrow keys, one column's two controls at a time", async () => {
      await openPreview()

      const idName = within(headerOf("id")).getByRole("button", { name: "id" })
      fireEvent.keyDown(idName, { key: "ArrowRight" })
      const cityName = within(headerOf("city")).getByRole("button", { name: "city" })
      expect(document.activeElement).toBe(cityName)
      expect(cityName.getAttribute("tabindex")).toBe("0")
      expect(idName.getAttribute("tabindex")).toBe("-1")
      fireEvent.keyDown(cityName, { key: "End" })
      expect(document.activeElement).toBe(within(headerOf("free")).getByRole("button", { name: "free" }))
    })
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

describe("Lineage tab: related tables", () => {
  const silver = { id: "silver.demo_orders", name: "silver.demo_orders" }
  const graphWith = (nodes: { id: string; label: string; kind: string }[]) => ({
    focus: "bronze.demo_orders",
    nodes,
    edges: [],
    columnMappings: [],
    supported: true,
  })

  it("leaves out a table the graph already draws, whose id there carries its kind", () => {
    const asset = { ...BRONZE, downstream: [silver] }
    const drawn = graphWith([
      { id: "bronze:demo_orders", label: "bronze.demo_orders", kind: "focus" },
      { id: "table:silver.demo_orders", label: "silver.demo_orders", kind: "silver" },
    ])
    expect(relatedTables(asset, drawn)).toEqual([])
  })

  it("lists a table the catalog relates to the asset when the graph does not record it", () => {
    const asset = { ...BRONZE, downstream: [silver] }
    const alone = graphWith([{ id: "bronze:demo_orders", label: "bronze.demo_orders", kind: "focus" }])
    expect(relatedTables(asset, alone).map((t) => t.id)).toEqual(["silver.demo_orders"])
    expect(relatedTables(asset, null).map((t) => t.id)).toEqual(["silver.demo_orders"])
  })
})

describe("A Bronze dataset is shown as its own table", () => {
  it("says nothing when the page reads the dataset's own Bronze table", async () => {
    stubApi()
    url.search = "tab=sample"
    renderTabs({
      ...BRONZE,
      queryTarget: { engine: "clickhouse", table: "icecat_api.`bronze.demo_orders`", policyTable: "bronze.demo_orders" },
    })

    await screen.findByText("Sample rows")
    expect(screen.queryByRole("note")).toBeNull()
  })

  it("says so when the Silver model stands in for a Bronze table that cannot be read", async () => {
    stubApi()
    url.search = "tab=sample"
    renderTabs({
      ...BRONZE,
      queryTarget: { engine: "clickhouse", table: "silver.`demo_orders`", policyTable: "silver.demo_orders" },
    })

    const note = await screen.findByRole("note")
    expect(note.textContent).toContain("Read from silver.demo_orders, this dataset's Silver model")
  })

  it("says the same above the columns, whose statistics come from that model", async () => {
    stubApi()
    url.search = "tab=schema"
    renderTabs({
      ...BRONZE,
      queryTarget: { engine: "clickhouse", table: "silver.`demo_orders`", policyTable: "silver.demo_orders" },
    })

    expect((await screen.findByRole("note")).textContent).toContain("silver.demo_orders")
  })

  it("has no stand-in for a Silver asset, which is read as itself", async () => {
    stubApi()
    url.search = "tab=sample"
    renderTabs({
      ...BRONZE,
      id: "silver.demo_orders",
      type: "table",
      layer: "silver",
      tableName: undefined,
      tableKey: "silver.demo_orders",
      queryTarget: { engine: "clickhouse", table: "silver.`demo_orders`", policyTable: "silver.demo_orders" },
    })

    await screen.findByText("Sample rows")
    expect(screen.queryByRole("note")).toBeNull()
  })
})

describe("Schema tab: a wide table", () => {
  const WIDE: AssetDetail = {
    ...BRONZE,
    tableName: undefined,
    schema: Array.from({ length: 59 }, (_, i) => ({
      name: `col_${String(i + 1).padStart(2, "0")}`,
      dataType: "Nullable(String)",
      ...(i === 40 ? { description: "Net weight in kilograms" } : {}),
    })),
  }
  const columnRows = (card: HTMLElement) => within(card).getAllByRole("row").length - 1

  it("shows the first 25 columns and says so, with the sizes that would cut this table short", async () => {
    stubApi()
    url.search = "tab=schema"
    renderTabs(WIDE)

    const card = (await screen.findByText("Columns")).closest("[data-slot=card]") as HTMLElement
    expect(columnRows(card)).toBe(25)
    expect(within(card).getByText("Showing the first 25 of 59 columns, in table order.")).toBeTruthy()
    const sizes = within(card).getByRole("group", { name: "Columns to show" })
    // 100 would be all 59, so it is not offered beside "All".
    expect(within(sizes).getAllByRole("button").map((b) => b.textContent)).toEqual(["25", "50", "All"])

    fireEvent.click(within(sizes).getByText("50"))
    expect(columnRows(card)).toBe(50)
    fireEvent.click(within(sizes).getByText("All"))
    expect(columnRows(card)).toBe(59)
    expect(within(card).getByText("59 columns, in table order.")).toBeTruthy()
  })

  it("filters columns by name or description, across the whole table", async () => {
    stubApi()
    url.search = "tab=schema"
    renderTabs(WIDE)

    const card = (await screen.findByText("Columns")).closest("[data-slot=card]") as HTMLElement
    // col_41 is past the first 25, and is found by what it is described as.
    fireEvent.change(within(card).getByLabelText("Filter columns"), { target: { value: "weight" } })
    expect(columnRows(card)).toBe(1)
    expect(within(card).getByText("col_41")).toBeTruthy()
    expect(within(card).getByText('1 of 59 columns match "weight".')).toBeTruthy()

    fireEvent.change(within(card).getByLabelText("Filter columns"), { target: { value: "nothing-like-this" } })
    expect(within(card).getByText('No column matches "nothing-like-this"')).toBeTruthy()
  })

  it("offers neither control for a table that fits the first page", async () => {
    stubApi()
    url.search = "tab=schema"
    renderTabs()

    const card = (await screen.findByText("Columns")).closest("[data-slot=card]") as HTMLElement
    expect(within(card).queryByRole("group", { name: "Columns to show" })).toBeNull()
    expect(within(card).queryByLabelText("Filter columns")).toBeNull()
  })
})

describe("Schema tab: the column explorer", () => {
  const SILVER_ORDERS: AssetDetail = {
    ...BRONZE,
    id: "silver.orders",
    name: "orders",
    namespace: "silver",
    type: "table",
    layer: "silver",
    tier: "hot",
    format: "ClickHouse MergeTree",
    tableName: undefined,
    tableKey: "silver.orders",
    queryTarget: undefined,
    schema: [
      { name: "city", dataType: "Nullable(String)", description: "Where the order shipped" },
      { name: "plnt", dataType: "String" },
      { name: "d", dataType: "Date" },
    ],
    storage: {
      table: "silver.orders",
      engine: "MergeTree",
      partitionKey: null,
      sortingKey: "toYYYYMM(d), plnt",
      parts: 1,
      partitions: 1,
      bytesOnDisk: 1000,
      uncompressedBytes: 4000,
      tableColumns: 3,
    },
  }
  const WITH_VALUES = {
    ...PROFILE,
    columns: [
      {
        name: "city",
        dataType: "Nullable(String)",
        profiled: true,
        nullCount: 100,
        nullFraction: 0.05,
        distinctCount: 4,
        topValues: [
          { value: "Jakarta", count: 1200 },
          { value: "Bandung", count: 500 },
        ],
      },
      { name: "plnt", dataType: "String", profiled: true, nullCount: 0, nullFraction: 0, distinctCount: 9, topValues: [] },
      { name: "d", dataType: "Date", profiled: true, nullCount: 0, nullFraction: 0, distinctCount: 30, min: "2026-09-01", max: "2026-09-30", topValues: [] },
    ],
  }
  const rowOf = (card: HTMLElement, name: string) =>
    within(card).getByRole("button", { name }).closest("[role=row]") as HTMLElement

  it("draws each column's most frequent values from the profile route, and opens the column", async () => {
    stubApi({ profile: WITH_VALUES })
    url.search = "tab=schema"
    renderTabs(SILVER_ORDERS)

    const card = (await screen.findByText("Columns")).closest("[data-slot=card]") as HTMLElement
    const city = rowOf(card, "city")
    await waitFor(() => expect(within(city).getByText("Jakarta")).toBeTruthy())
    expect(within(city).getByText("60.0%")).toBeTruthy()
    expect(within(city).getByText("≈ 4")).toBeTruthy()
    // The description is in the opened column, not under the name.
    expect(within(card).queryByText("Where the order shipped")).toBeNull()

    fireEvent.click(within(city).getByRole("button", { name: "city" }))
    const panel = city.nextElementSibling as HTMLElement
    expect(within(panel).getAllByRole("listitem").map((li) => li.textContent)).toEqual([
      "Jakarta1,20060.0%",
      "Bandung50025.0%",
      "Other values20010.0%",
      "Null1005.0%",
    ])
    expect(within(panel).getByText("Where the order shipped")).toBeTruthy()
  })

  it("marks the column the storage card's sorting key names, and not the one inside an expression", async () => {
    stubApi({ profile: WITH_VALUES })
    url.search = "tab=schema"
    renderTabs(SILVER_ORDERS)

    const card = (await screen.findByText("Columns")).closest("[data-slot=card]") as HTMLElement
    // A row renders its marks twice, beside the name and on the type's line (CSS shows one), so
    // one marked column is two "sort key" and an unmarked column none.
    expect(within(rowOf(card, "plnt")).getAllByText("sort key")).toHaveLength(2)
    expect(within(rowOf(card, "d")).queryByText("sort key")).toBeNull()
    expect(within(card).getAllByText("sort key")).toHaveLength(2)
  })

  it("marks no sort key for a table that has no storage card", async () => {
    stubApi({ profile: WITH_VALUES })
    url.search = "tab=schema"
    renderTabs({ ...SILVER_ORDERS, storage: undefined })

    await screen.findByText("Columns")
    expect(screen.queryByText("sort key")).toBeNull()
  })

  it("asks for no statistics, and says why, without query:read, yet still opens a column", async () => {
    const fetchSpy = stubApi({ permissions: ["catalog:read"], profile: WITH_VALUES })
    url.search = "tab=schema"
    renderTabs(SILVER_ORDERS)

    const card = (await screen.findByText("Columns")).closest("[data-slot=card]") as HTMLElement
    expect(within(card).getByText(/they need the query:read permission/)).toBeTruthy()
    expect(within(card).queryByRole("columnheader", { name: "Nulls" })).toBeNull()
    expect(fetchSpy.mock.calls.some((c) => String(c[0]).endsWith("/profile"))).toBe(false)

    const city = within(card).getByRole("button", { name: "city" })
    fireEvent.click(city)
    const panel = document.getElementById(city.getAttribute("aria-controls") ?? "") as HTMLElement
    expect(within(panel).getByText("Nullable(String)")).toBeTruthy()
    expect(within(panel).getByText("Where the order shipped")).toBeTruthy()
  })
})

describe("Activity tab: taking a target back, and long lists", () => {
  it("removes a freshness SLA, which could only be changed before", async () => {
    const fetchSpy = stubApi()
    const reloaded = mock(() => {})
    url.search = "tab=activity"
    renderTabs({ ...BRONZE, freshnessTargetSeconds: 172800, freshnessTargetSource: "sla" }, reloaded)

    fireEvent.click(await screen.findByText("Change target"))
    fireEvent.click(screen.getByRole("button", { name: "Remove target" }))

    await waitFor(() => expect(reloaded).toHaveBeenCalled())
    expect(writes(fetchSpy)).toEqual([["DELETE", "/api/governance/sla/bronze.demo_orders", undefined]])
  })

  it("offers no removal for a target that is not an SLA", async () => {
    stubApi()
    url.search = "tab=activity"
    renderTabs({ ...BRONZE, freshnessTargetSeconds: 129600, freshnessTargetSource: "frequency" })

    fireEvent.click(await screen.findByText("Set target"))
    expect(screen.queryByRole("button", { name: "Remove target" })).toBeNull()
  })

  it("lists the newest ten changes of a long history, with the rest one click away", async () => {
    stubApi()
    url.search = "tab=activity"
    renderTabs({
      ...BRONZE,
      tableName: undefined,
      changeHistory: Array.from({ length: 12 }, (_, i) => ({
        id: `audit-${i}`,
        at: `2026-09-${String(30 - i).padStart(2, "0")}T09:00:00Z`,
        actor: "Rina",
        summary: `Change number ${i + 1}`,
      })),
    })

    const card = (await screen.findByText("Change history")).closest("[data-slot=card]") as HTMLElement
    expect(within(card).getAllByRole("listitem")).toHaveLength(10)
    expect(within(card).getByText(/Showing the newest 10 of 12\./)).toBeTruthy()
    const sizes = within(card).getByRole("group", { name: "Changes to show" })
    expect(within(sizes).getAllByRole("button").map((b) => b.textContent)).toEqual(["10", "All"])
    fireEvent.click(within(sizes).getByText("All"))
    expect(within(card).getAllByRole("listitem")).toHaveLength(12)
  })

  it("shows no size control for a short history", async () => {
    stubApi()
    url.search = "tab=activity"
    renderTabs({ ...BRONZE, changeHistory: [{ id: "a1", at: "2026-09-30T09:00:00Z", actor: "Rina", summary: "Edited owner" }] })

    await screen.findByText("Change history")
    expect(screen.queryByRole("group", { name: "Changes to show" })).toBeNull()
  })
})

describe("Access tab: taking a classification back", () => {
  const CLASSIFIED: AssetDetail = {
    ...BRONZE,
    classification: "restricted",
    classificationSource: "rule",
    schema: [
      { name: "amount", dataType: "Decimal(12, 2)", classification: "restricted" },
      { name: "id", dataType: "Int64" },
    ],
    classificationRules: [
      { id: "rule-asset", classification: "public" },
      { id: "rule-amount", column: "amount", classification: "restricted" },
    ],
  }

  it("removes the rule in force for a column, after saying what applies instead", async () => {
    const fetchSpy = stubApi()
    const reloaded = mock(() => {})
    url.search = "tab=access"
    renderTabs(CLASSIFIED, reloaded)

    fireEvent.click(await screen.findByLabelText("Remove the classification of column amount"))
    expect(screen.getByText("Remove the Restricted classification of column amount?")).toBeTruthy()
    expect(screen.getByText(/An older rule for it applies again if there is one/)).toBeTruthy()
    expect(writes(fetchSpy)).toEqual([])
    fireEvent.click(screen.getByRole("button", { name: "Remove" }))

    await waitFor(() => expect(reloaded).toHaveBeenCalled())
    expect(writes(fetchSpy)).toEqual([["DELETE", "/api/governance/classification/rule-amount", undefined]])
  })

  it("removes the asset's own rule by its own id", async () => {
    const fetchSpy = stubApi()
    url.search = "tab=access"
    renderTabs(CLASSIFIED)

    fireEvent.click(await screen.findByLabelText("Remove the classification of this asset"))
    expect(screen.getByText("Remove the Public classification of this asset?")).toBeTruthy()
    fireEvent.click(screen.getByRole("button", { name: "Remove" }))
    await waitFor(() =>
      expect(writes(fetchSpy)).toEqual([["DELETE", "/api/governance/classification/rule-asset", undefined]])
    )
  })

  it("offers no removal without governance:write, or for the default level", async () => {
    stubApi({ permissions: ["catalog:read", "policy:read"] })
    url.search = "tab=access"
    renderTabs(CLASSIFIED)
    await screen.findByText("Classification")
    await waitFor(() => expect(screen.queryByLabelText(/Remove the classification/)).toBeNull())
    cleanup()

    stubApi()
    renderTabs({ ...BRONZE, classificationRules: [] })
    await screen.findByText("Classification")
    expect(screen.queryByLabelText(/Remove the classification/)).toBeNull()
  })
})

// ADR 0015: ClickHouse keeps only a table's current columns, so the console
// records each version it sees (`routes/schema_versions.rs`) and the page
// shows them where a raw table shows its Iceberg ones — saying since when,
// and never calling the list the table's whole history.
describe("Schema versions of a Silver or Gold table", () => {
  const FIRST = "2026-10-01T12:00:00Z"
  const SECOND = "2026-10-03T12:00:00Z"
  const SILVER: AssetDetail = {
    ...BRONZE,
    id: "silver.orders",
    name: "orders",
    namespace: "silver",
    type: "table",
    layer: "silver",
    tier: "hot",
    format: "ClickHouse MergeTree",
    tableName: undefined,
    tableKey: "silver.orders",
    queryTarget: undefined,
    schemaVersions: [
      { version: 2, at: SECOND, change: "Added email (String)", current: true },
      { version: 1, at: FIRST, change: "First recorded with 2 columns", current: false },
    ],
  }

  it("lists the recorded versions newest first, marks the current one, and says since when", () => {
    stubApi()
    url.search = "tab=schema"
    renderTabs(SILVER)

    const card = screen.getByText("Schema versions").closest("[data-slot=card]") as HTMLElement
    expect(within(card).getAllByRole("listitem").map((li) => li.textContent)).toEqual([
      expect.stringContaining("v2currentAdded email (String)recorded"),
      expect.stringContaining("v1First recorded with 2 columns"),
    ])
    // Only the newest carries the pill.
    expect(within(card).getAllByText("current")).toHaveLength(1)
    expect(
      within(card).getByText(
        `Recorded by the console each time this table's columns change, since ${formatDate(FIRST, { month: "short" })}. ` +
          "Changes before that are not known, and a renamed column shows as one dropped and one added."
      )
    ).toBeTruthy()
    // The sentence that an engine table keeps no history is gone.
    expect(screen.queryByText("Only Iceberg tables record schema history")).toBeNull()
  })

  it("says no version is recorded yet, and when the console records one, for a table it has not looked at", () => {
    stubApi()
    url.search = "tab=schema"
    renderTabs({ ...SILVER, schemaVersions: [] })

    const card = screen.getByText("Schema versions").closest("[data-slot=card]") as HTMLElement
    expect(within(card).getByText("No schema version recorded yet")).toBeTruthy()
    expect(
      within(card).getByText(/records a version the next time it looks at this table: when a run finishes, and on a schedule/)
    ).toBeTruthy()
    expect(within(card).queryByRole("listitem")).toBeNull()
    expect(screen.queryByText("Only Iceberg tables record schema history")).toBeNull()
  })

  it("places each recorded version among what people changed in the change history", async () => {
    stubApi()
    url.search = "tab=activity"
    renderTabs({
      ...SILVER,
      changeHistory: [
        { id: "audit-2", at: "2026-10-04T09:00:00.000Z", actor: "Rina", summary: "Edited description" },
        { id: "audit-1", at: "2026-10-02T09:00:00.000Z", actor: "Budi", summary: "Added quality rule orders_id_unique" },
      ],
    })

    const history = screen.getByText("Change history").closest("[data-slot=card]") as HTMLElement
    expect(within(history).getAllByRole("listitem").map((li) => li.textContent)).toEqual([
      expect.stringContaining("RinaEdited description"),
      expect.stringContaining("Schema v2Added email (String)"),
      expect.stringContaining("BudiAdded quality rule orders_id_unique"),
      expect.stringContaining("Schema v1First recorded with 2 columns"),
    ])
  })

  it("shows the current version in the About card as recorded, not as the table's age", async () => {
    stubApi()
    renderTabs(SILVER)

    const about = (await screen.findByText("About")).closest("[data-slot=card]") as HTMLElement
    expect(within(about).getByText("Schema")).toBeTruthy()
    expect(within(about).getByText(/^v2 · recorded /)).toBeTruthy()
  })

  it("leaves the About card without a schema row until a version is recorded", async () => {
    stubApi()
    renderTabs({ ...SILVER, schemaVersions: [] })

    const about = (await screen.findByText("About")).closest("[data-slot=card]") as HTMLElement
    expect(within(about).queryByText("Schema")).toBeNull()
  })

  it("leaves a raw table's cards as they were: its versions come from the catalog", async () => {
    stubApi()
    url.search = "tab=schema"
    renderTabs()

    const card = (await screen.findByText("Schema versions")).closest("[data-slot=card]") as HTMLElement
    await waitFor(() => expect(within(card).getAllByRole("listitem")).toHaveLength(2))
    expect(within(card).getByText("Every schema the table has had, most recent first.")).toBeTruthy()
    expect(within(card).queryByText(/recorded/i)).toBeNull()
  })

  it("still says a raw table has no schema history when the catalog has none, not that none is recorded", async () => {
    stubApi()
    url.search = "tab=schema"
    renderTabs({ ...BRONZE, tableName: "missing_table", tableKey: "bronze.missing_table" })

    expect(await screen.findByText("No schema history available")).toBeTruthy()
    expect(screen.queryByText("No schema version recorded yet")).toBeNull()
  })
})
