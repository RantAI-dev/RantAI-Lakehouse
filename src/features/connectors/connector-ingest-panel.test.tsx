import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"
import {
  ConnectorIngestPanel,
  cronProblem,
  defaultDiscoverSchema,
  defaultTarget,
  groupResultsByRun,
  loadModeProblem,
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
  scheduleCron: null as string | null,
  secretRefs: { primary: "file:/run/secrets/connector_managed_x_password", secondary: null },
  nextRunAt: null as string | null,
}

type Call = { url: string; method: string; body: unknown }

function stubFetch(
  spec: typeof SPEC = SPEC,
  history: { runs: unknown[]; results: unknown[] } = { runs: [], results: [] }
): Call[] {
  const calls: Call[] = []
  global.fetch = mock(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input)
    const method = init?.method ?? "GET"
    const body = init?.body ? JSON.parse(init.body as string) : undefined
    calls.push({ url, method, body })
    if (url.includes("/ingest-spec") && method === "GET") return json(spec)
    if (url.includes("/ingest-spec") && method === "PUT") {
      const scheduleCron = body.scheduleCron ?? null
      return json({ ...spec, ...body, scheduleCron, nextRunAt: scheduleCron ? "2026-10-01T02:00:00Z" : null })
    }
    if (url.includes("/api/governance/ingest-runs")) return json(history.results)
    if (url.includes("/ingest/runs")) return json(history.runs)
    if (url.includes("/debezium-properties")) {
      return json({ table: "public.orders", properties: "database.password=${DB_PASSWORD}", note: "References only." })
    }
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
  it("asks an incremental table for its new-row column", () => {
    expect(loadModeProblem({ loadMode: "replace" })).toBeNull()
    expect(loadModeProblem({ loadMode: "append", incrementalKey: "" })).toBeNull()
    expect(loadModeProblem({ loadMode: "incremental", incrementalKey: "updated_at" })).toBeNull()
    expect(loadModeProblem({ loadMode: "incremental", incrementalKey: "  " })).toMatch(/Pick the column/)
    expect(loadModeProblem({ loadMode: "incremental" })).toMatch(/Pick the column/)
  })

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
    // A saved schedule applies on its own, with no orchestrator reload.
    expect(screen.getByText("Runs on this schedule within a minute of saving.")).toBeDefined()
    // Unsaved changes block a run.
    expect(screen.getByText("Save your changes before running.")).toBeDefined()

    fireEvent.click(screen.getByRole("button", { name: "Save tables and schedule" }))
    await waitFor(() => expect(calls.some((c) => c.method === "PUT")).toBe(true))
    const put = calls.find((c) => c.method === "PUT")
    expect(put?.body).toEqual({
      adapter: "sql",
      ingestMode: "batch",
      dial: SPEC.dial,
      // A ticked table replaces by default: one copy of the source per run.
      sourceObjects: [{ name: "public.orders", target: "northwind_orders", loadMode: "replace" }],
      scheduleCron: "0 2 * * *",
    })

    // Saved: the found tables stay on screen, and the run can start.
    await waitFor(() =>
      expect((screen.getByRole("button", { name: "Run now" }) as HTMLButtonElement).disabled).toBe(false)
    )
    expect(screen.getByText("public.customers")).toBeDefined()
    // The saved schedule says when it next runs, in the viewer's time.
    expect(screen.getByText(/Next run/).textContent).toMatch(/Next run Oct 0?1, 2026, \d\d:00 your time \((in \d+[mhd]|now)\)/)
    fireEvent.click(screen.getByRole("button", { name: "Run now" }))
    // Until Dagster lists the run, the panel waits for it and keeps Run now
    // disabled, so a second click cannot start a second run.
    await screen.findByText(/Starting run 01234567/)
    expect((screen.getByRole("button", { name: "Run now" }) as HTMLButtonElement).disabled).toBe(true)
    expect(calls.some((c) => c.url.endsWith("/api/connectors/conn-northwind/ingest/run") && c.method === "POST")).toBe(
      true
    )
  })

  /**
   * "Add only new rows" needs the column that marks them. It is picked
   * from the table's own columns once the table has been found, and the
   * save carries both the mode and the column.
   */
  it("saves an incremental table with its new-row column", async () => {
    const calls = stubFetch()
    render(<ConnectorIngestPanel connectorId="conn-northwind" connectorName="northwind" />)
    fireEvent.click(await screen.findByRole("button", { name: /Find tables/ }))
    fireEvent.click(await screen.findByLabelText(/public\.orders/))

    const mode = screen.getByLabelText("Load mode for public.orders") as HTMLSelectElement
    expect(mode.value).toBe("replace")
    fireEvent.change(mode, { target: { value: "incremental" } })
    // No column yet: the reason is shown and the save is off.
    expect(screen.getByText(/Pick the column that marks new rows/)).toBeDefined()
    const save = screen.getByRole("button", { name: "Save tables and schedule" }) as HTMLButtonElement
    expect(save.disabled).toBe(true)

    fireEvent.change(screen.getByLabelText("New-row column for public.orders"), { target: { value: "order_id" } })
    expect(screen.getByText(/later runs add rows whose order_id is higher/)).toBeDefined()
    expect(save.disabled).toBe(false)
    fireEvent.click(save)
    await waitFor(() => expect(calls.some((c) => c.method === "PUT")).toBe(true))
    expect((calls.find((c) => c.method === "PUT")?.body as { sourceObjects: unknown }).sourceObjects).toEqual([
      { name: "public.orders", target: "northwind_orders", loadMode: "incremental", incrementalKey: "order_id" },
    ])
  })

  /**
   * A table saved before load modes existed is run as "replace", so that
   * is what it shows, without counting as an unsaved change. Only SQL
   * connectors are offered "Add only new rows".
   */
  it("shows a table saved without a mode as replace, and offers incremental to SQL only", async () => {
    stubFetch({ ...SPEC, sourceObjects: [{ name: "public.orders", target: "northwind_orders" }] })
    const sql = render(<ConnectorIngestPanel connectorId="conn-northwind" connectorName="northwind" />)
    const mode = (await screen.findByLabelText("Load mode for public.orders")) as HTMLSelectElement
    expect(mode.value).toBe("replace")
    expect([...mode.options].map((o) => o.value)).toEqual(["replace", "incremental", "append"])
    expect((screen.getByRole("button", { name: "Save tables and schedule" }) as HTMLButtonElement).disabled).toBe(true)
    sql.unmount()

    stubFetch({
      ...SPEC,
      adapter: "rest",
      dial: { baseUrl: "https://api.example.com" } as unknown as typeof SPEC.dial,
      sourceObjects: [{ name: "orders", target: "api_orders" }],
    })
    render(<ConnectorIngestPanel connectorId="conn-api" connectorName="api" />)
    const restMode = (await screen.findByLabelText("Load mode for orders")) as HTMLSelectElement
    expect([...restMode.options].map((o) => o.value)).toEqual(["replace", "append"])
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

/**
 * Where the panel sits (plan `2026-10-05-connector-detail-page.md`, section 10,
 * U5): the page shows three cards, two columns from `xl`; the page after
 * creating a connector, which already wraps the panel in a card, keeps one
 * column and no card of its own. Nothing but the arrangement differs.
 */
describe("ConnectorIngestPanel layout", () => {
  const cardTitles = (root: HTMLElement) =>
    [...root.querySelectorAll("[data-slot=card-title]")].map((t) => t.textContent)

  it("is one column with no card of its own by default, as on the page after creating a connector", async () => {
    stubFetch()
    const { container } = render(<ConnectorIngestPanel connectorId="conn-northwind" connectorName="northwind" />)
    await screen.findByText("No tables selected yet. Find tables below and tick the ones to copy.")
    expect(container.querySelector("[data-slot=card]")).toBeNull()
    expect(container.querySelector("[class*=xl\\:grid-cols]")).toBeNull()
    expect(screen.getByRole("heading", { name: "Tables to ingest" })).toBeDefined()
    expect(screen.getByRole("button", { name: "Run now" })).toBeDefined()
  })

  it("is three cards on the connector page, Tables to ingest, Schedule and Runs, with Save in Schedule and Run now in Runs", async () => {
    stubFetch()
    const { container } = render(
      <ConnectorIngestPanel connectorId="conn-northwind" connectorName="northwind" layout="page" />
    )
    await screen.findByText("No tables selected yet. Find tables below and tick the ones to copy.")
    expect(cardTitles(container)).toEqual(["Tables to ingest", "Schedule", "Runs"])
    const card = (title: string) =>
      within(
        screen.getByText(title, { selector: "[data-slot=card-title]" }).closest("[data-slot=card]") as HTMLElement
      )
    expect(card("Tables to ingest").getByLabelText("Schema")).toBeDefined()
    expect(card("Tables to ingest").getByRole("button", { name: "Find tables" })).toBeDefined()
    expect(card("Schedule").getByLabelText("Schedule")).toBeDefined()
    expect(card("Schedule").getByRole("button", { name: "Save tables and schedule" })).toBeDefined()
    expect(card("Runs").getByRole("button", { name: "Run now" })).toBeDefined()
    expect(card("Runs").getByText("No runs yet.")).toBeDefined()
  })

  it("puts tables and schedule in a wider left column and the runs in the right one from xl up", async () => {
    stubFetch()
    const { container } = render(
      <ConnectorIngestPanel connectorId="conn-northwind" connectorName="northwind" layout="page" />
    )
    await screen.findByText("No tables selected yet. Find tables below and tick the ones to copy.")
    const grid = container.firstElementChild as HTMLElement
    expect(grid.className).toContain("xl:grid-cols-[minmax(0,3fr)_minmax(0,2fr)]")
    const [left, right] = [...grid.children] as HTMLElement[]
    expect(cardTitles(left)).toEqual(["Tables to ingest", "Schedule"])
    expect(cardTitles(right)).toEqual(["Runs"])
  })

  it("keeps every behaviour in the card layout: find, tick, save, with the same labels", async () => {
    const calls = stubFetch()
    render(<ConnectorIngestPanel connectorId="conn-northwind" connectorName="northwind" layout="page" />)
    fireEvent.click(await screen.findByRole("button", { name: "Find tables" }))
    fireEvent.click(await screen.findByLabelText(/public\.orders/))
    expect((screen.getByLabelText("Bronze table for public.orders") as HTMLInputElement).value).toBe("northwind_orders")
    fireEvent.change(screen.getByLabelText("Schedule"), { target: { value: "0 2 * * *" } })
    fireEvent.click(screen.getByRole("button", { name: "Save tables and schedule" }))
    await waitFor(() => expect(calls.some((c) => c.method === "PUT")).toBe(true))
    expect((calls.find((c) => c.method === "PUT")?.body as { scheduleCron: string }).scheduleCron).toBe("0 2 * * *")
  })

  it("stops its inputs stretching across the page, and keeps the embedded ones as they were", async () => {
    stubFetch({ ...SPEC, sourceObjects: [{ name: "public.orders", target: "northwind_orders" }] })
    const page = render(
      <ConnectorIngestPanel connectorId="conn-northwind" connectorName="northwind" layout="page" />
    )
    expect((await screen.findByLabelText("Bronze table for public.orders")).className).toContain("max-w-xs")
    expect(screen.getByLabelText("Schedule").className).toContain("max-w-xs")
    page.unmount()

    render(<ConnectorIngestPanel connectorId="conn-northwind" connectorName="northwind" />)
    expect((await screen.findByLabelText("Bronze table for public.orders")).className).not.toContain("max-w-")
    expect(screen.getByLabelText("Schedule").className).not.toContain("max-w-")
  })

  /**
   * Reviewer SHOULD-FIX R3: in the half-width Runs card the old single line
   * wrapped "2d ago · took 6.0 s" mid-phrase and cut the summary. The card
   * layout gives a run two lines; the embedded one keeps its single line.
   */
  describe("a run's row", () => {
    const history = {
      runs: [
        { runId: "run-1234567890", status: "completed", startedAt: "2026-09-29T08:21:51.171Z", endedAt: "2026-09-29T08:22:33.646Z" },
      ],
      results: [
        {
          connectorId: "conn-northwind",
          job: "ingest_job",
          object: "public.orders",
          rows: 7,
          startedAt: "2026-09-29T08:22:00.000Z",
          endedAt: "2026-09-29T08:22:10.000Z",
          status: "succeeded",
          error: "",
        },
      ],
    }

    it("has two lines in the card layout: the verdict, id and untruncated summary, then when and how long", async () => {
      stubFetch(SPEC, history)
      render(<ConnectorIngestPanel connectorId="conn-northwind" connectorName="northwind" layout="page" />)
      const summaryText = await screen.findByText("1 of 1 tables · 7 rows")
      expect(summaryText.className).not.toContain("truncate")
      const firstLine = summaryText.parentElement as HTMLElement
      expect(within(firstLine).getByText("Completed")).toBeDefined()
      expect(within(firstLine).getByText("run-1234")).toBeDefined()
      // When and how long is on its own, muted line below, whole.
      const second = screen.getByText(/took 42 s|took 42\.\d s/)
      expect(firstLine.contains(second)).toBe(false)
      expect(second.parentElement).toBe(firstLine.parentElement)
      expect(second.className).toContain("block")
      expect(second.className).toContain("text-muted-foreground")
    })

    it("keeps one line, with the summary truncated, in the embedded layout", async () => {
      stubFetch(SPEC, history)
      render(<ConnectorIngestPanel connectorId="conn-northwind" connectorName="northwind" />)
      const summaryText = await screen.findByText("1 of 1 tables · 7 rows")
      expect(summaryText.className).toContain("truncate")
      const line = summaryText.parentElement as HTMLElement
      expect(within(line).getByText("Completed")).toBeDefined()
      expect(within(line).getByText(/took /)).toBeDefined()
    })
  })

  it("has no Schedule card for a change-data-capture connector, whose Save ends the tables card", async () => {
    stubFetch({
      ...SPEC,
      adapter: "cdc",
      ingestMode: "stream",
      dial: { ...SPEC.dial, driver: "postgres" },
      sourceObjects: [{ name: "public.orders", target: "northwind_orders" }],
    })
    const { container } = render(
      <ConnectorIngestPanel connectorId="conn-northwind" connectorName="northwind" layout="page" />
    )
    await screen.findByText(/Debezium properties/)
    expect(cardTitles(container)).toEqual(["Tables to ingest", "Runs"])
    const tables = (screen.getByText("Tables to ingest", { selector: "[data-slot=card-title]" }).closest(
      "[data-slot=card]"
    ) as HTMLElement)
    expect(within(tables).getByRole("button", { name: "Save tables and schedule" })).toBeDefined()
    expect(screen.queryByLabelText("Schedule")).toBeNull()
  })

  it("says in the tables card that a Kafka connector reads its topic, and has no table list to edit", async () => {
    stubFetch({ ...SPEC, adapter: "kafka", ingestMode: "stream", sourceObjects: [] })
    const { container } = render(
      <ConnectorIngestPanel connectorId="conn-northwind" connectorName="northwind" layout="page" />
    )
    expect(await screen.findByText("A Kafka connector reads its topic; each run takes one micro-batch.")).toBeDefined()
    expect(cardTitles(container)).toEqual(["Tables to ingest", "Schedule", "Runs"])
    expect(screen.queryByText(/Selected ·/)).toBeNull()
  })
})

describe("ConnectorIngestPanel onTableCount", () => {
  it("reports how many tables are saved once the spec is read, and the new number after a save", async () => {
    const counts: number[] = []
    const calls = stubFetch({ ...SPEC, sourceObjects: [{ name: "public.orders", target: "northwind_orders" }] })
    render(
      <ConnectorIngestPanel
        connectorId="conn-northwind"
        connectorName="northwind"
        onTableCount={(n) => counts.push(n)}
      />
    )
    await screen.findByLabelText("Bronze table for public.orders")
    expect(counts).toEqual([1])

    fireEvent.click(screen.getByRole("button", { name: "Find tables" }))
    fireEvent.click(await screen.findByLabelText(/public\.customers/))
    fireEvent.click(screen.getByRole("button", { name: "Save tables and schedule" }))
    await waitFor(() => expect(calls.some((c) => c.method === "PUT")).toBe(true))
    await waitFor(() => expect(counts).toEqual([1, 2]))
  })

  it("reports nothing, rather than a zero, for a connector with no connection saved", async () => {
    const counts: number[] = []
    stubFetch({ ...SPEC, adapter: null as unknown as string, ingestMode: null as unknown as string })
    render(
      <ConnectorIngestPanel
        connectorId="conn-northwind"
        connectorName="northwind"
        onTableCount={(n) => counts.push(n)}
      />
    )
    await screen.findByText(/has no connection settings saved yet/)
    expect(counts).toEqual([])
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
