import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"
import {
  ConnectorIngestPanel,
  cronProblem,
  defaultDiscoverSchema,
  defaultTarget,
  groupResultsByRun,
  targetProblem,
} from "./connector-ingest-panel"

const originalFetch = global.fetch

afterEach(() => {
  global.fetch = originalFetch
  cleanup()
})

function json(body: unknown, status = 200) {
  return new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } })
}

const SPEC = {
  adapter: "sql",
  ingestMode: "batch",
  dial: { driver: "postgres", host: "192.168.18.205", port: 55432, database: "northwind", user: "postgres", sslMode: null },
  sourceObjects: [] as { name: string; target: string }[],
  scheduleCron: null,
  secretRefs: { primary: "file:/run/secrets/connector_managed_x_password", secondary: null },
}

type Call = { url: string; method: string; body: unknown }

function stubFetch(spec: typeof SPEC = SPEC): Call[] {
  const calls: Call[] = []
  global.fetch = mock(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input)
    const method = init?.method ?? "GET"
    const body = init?.body ? JSON.parse(init.body as string) : undefined
    calls.push({ url, method, body })
    if (url.includes("/ingest-spec") && method === "GET") return json(spec)
    if (url.includes("/ingest-spec") && method === "PUT") return json({ ...spec, ...body, scheduleCron: body.scheduleCron ?? null })
    if (url.includes("/api/governance/ingest-runs")) return json([])
    if (url.includes("/ingest/runs")) return json([])
    if (url.includes("/discover")) {
      return json({
        supported: true,
        objects: [
          { name: "public.orders", columns: [{ name: "order_id", typeName: "smallint" }] },
          { name: "public.customers", columns: [{ name: "customer_id", typeName: "text" }] },
        ],
      })
    }
    if (url.includes("/ingest/run")) return json({ runId: "0123456789abcdef" })
    throw new Error(`unexpected fetch: ${method} ${url}`)
  }) as unknown as typeof fetch
  return calls
}

describe("ingest panel helpers", () => {
  it("names a Bronze table after the connector and the source table", () => {
    expect(defaultTarget("northwind", "public.orders")).toBe("northwind_orders")
    expect(defaultTarget("Northwind DB!", "public.Order Details")).toBe("northwind_db_order_details")
    expect(defaultTarget("2024 sales", "t")).toBe("t_2024_sales_t")
  })
  it("refuses targets that are not identifiers or land twice", () => {
    expect(targetProblem("northwind_orders", [])).toBeNull()
    expect(targetProblem("Orders", [])).not.toBeNull()
    expect(targetProblem("orders", ["orders"])).toBe("Another table already lands here.")
  })
  it("lists public for Postgres, the database for MySQL, dbo for SQL Server", () => {
    expect(defaultDiscoverSchema({ driver: "postgres" })).toBe("public")
    expect(defaultDiscoverSchema({ driver: "mysql", database: "shop" })).toBe("shop")
    expect(defaultDiscoverSchema({ driver: "mssql" })).toBe("dbo")
  })
  it("wants five cron fields", () => {
    expect(cronProblem("0 2 * * *")).toBeNull()
    expect(cronProblem("0 2 * *")).not.toBeNull()
  })
})

describe("ConnectorIngestPanel", () => {
  it("finds tables in the schema, saves the picked ones with their Bronze names, then runs", async () => {
    const calls = stubFetch()
    render(<ConnectorIngestPanel connectorId="conn-northwind" connectorName="northwind" />)

    // Nothing saved yet: running is pointless and says why.
    await screen.findByText("No tables selected yet. Find tables below and tick the ones to copy.")
    expect((screen.getByRole("button", { name: "Run now" }) as HTMLButtonElement).disabled).toBe(true)
    expect(screen.getByText("Save at least one table to run.")).toBeDefined()

    expect((screen.getByLabelText("Schema") as HTMLInputElement).value).toBe("public")
    fireEvent.click(screen.getByRole("button", { name: "Find tables" }))
    await screen.findByText("public.orders")
    const discover = calls.find((c) => c.url.includes("/discover"))
    expect(discover?.url).toContain("/api/connectors/conn-northwind/discover?schema=public")

    fireEvent.click(screen.getByLabelText(/public\.orders/))
    const target = screen.getByLabelText("Bronze table for public.orders") as HTMLInputElement
    expect(target.value).toBe("northwind_orders")

    fireEvent.change(screen.getByLabelText("Schedule"), { target: { value: "0 2 * * *" } })
    // Unsaved changes block a run.
    expect(screen.getByText("Save your changes before running.")).toBeDefined()

    fireEvent.click(screen.getByRole("button", { name: "Save tables and schedule" }))
    await waitFor(() => expect(calls.some((c) => c.method === "PUT")).toBe(true))
    const put = calls.find((c) => c.method === "PUT")
    expect(put?.body).toEqual({
      adapter: "sql",
      ingestMode: "batch",
      dial: SPEC.dial,
      sourceObjects: [{ name: "public.orders", target: "northwind_orders" }],
      scheduleCron: "0 2 * * *",
    })

    // Saved: the found tables stay on screen, and the run can start.
    await waitFor(() =>
      expect((screen.getByRole("button", { name: "Run now" }) as HTMLButtonElement).disabled).toBe(false)
    )
    expect(screen.getByText("public.customers")).toBeDefined()
    fireEvent.click(screen.getByRole("button", { name: "Run now" }))
    // Until Dagster lists the run, the panel waits for it and keeps Run now
    // disabled, so a second click cannot start a second run.
    await screen.findByText(/Starting run 01234567/)
    expect((screen.getByRole("button", { name: "Run now" }) as HTMLButtonElement).disabled).toBe(true)
    expect(calls.some((c) => c.url.endsWith("/api/connectors/conn-northwind/ingest/run") && c.method === "POST")).toBe(
      true
    )
  })

  it("does not save a Bronze name that is not an identifier", async () => {
    stubFetch({ ...SPEC, sourceObjects: [{ name: "public.orders", target: "northwind_orders" }] })
    render(<ConnectorIngestPanel connectorId="conn-northwind" connectorName="northwind" />)
    const target = (await screen.findByLabelText("Bronze table for public.orders")) as HTMLInputElement
    fireEvent.change(target, { target: { value: "Bad Name" } })
    expect(screen.getByText(/Lower-case letters, digits and _ only/)).toBeDefined()
    expect((screen.getByRole("button", { name: "Save tables and schedule" }) as HTMLButtonElement).disabled).toBe(true)
  })
})

describe("groupResultsByRun", () => {
  const result = (object: string, startedAt: string, status = "succeeded") => ({
    connectorId: "conn-northwind",
    job: "ingest_job",
    object,
    rows: 1,
    startedAt,
    endedAt: startedAt,
    status,
    error: "",
  })

  it("puts each table result under the run it happened in, oldest table first, and older results last", () => {
    const runs = [
      { runId: "run-2", status: "running" as const, startedAt: "2026-09-29T08:22:39.206Z", endedAt: null },
      { runId: "run-1", status: "completed" as const, startedAt: "2026-09-29T08:21:51.171Z", endedAt: "2026-09-29T08:22:33.646Z" },
    ]
    const groups = groupResultsByRun(runs, [
      result("public.orders", "2026-09-29T08:22:59.700710+00:00"),
      result("public.categories", "2026-09-29T08:22:42.972055+00:00"),
      result("public.us_states", "2026-09-29T08:22:30.023414+00:00"),
      result("public.categories", "2026-09-29T07:48:10.000000+00:00", "rejected"),
    ])
    expect(groups.map((g) => g.run?.runId ?? "earlier")).toEqual(["run-2", "run-1", "earlier"])
    expect(groups[0]!.results.map((r) => r.object)).toEqual(["public.categories", "public.orders"])
    expect(groups[1]!.results.map((r) => r.object)).toEqual(["public.us_states"])
    expect(groups[2]!.results[0]!.status).toBe("rejected")
  })

  it("keeps a run that reached no table, so a failure before the first table still shows", () => {
    const groups = groupResultsByRun(
      [{ runId: "run-401", status: "failed", startedAt: "2026-09-29T06:48:06.638Z", endedAt: "2026-09-29T06:48:09.000Z" }],
      []
    )
    expect(groups).toHaveLength(1)
    expect(groups[0]!.results).toEqual([])
  })
})
