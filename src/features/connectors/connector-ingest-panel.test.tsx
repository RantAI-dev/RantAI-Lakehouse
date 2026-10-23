import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"
import {
  ConnectorIngestPanel,
  cronProblem,
  defaultDiscoverSchema,
  defaultTarget,
  labelTarget,
  isAutoDisablePause,
  loadModeProblem,
  lockedColumns,
  REST_OLD_OBJECT_NOTE,
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

type Found = { name: string; columns: { name: string; typeName: string }[]; primaryKey?: string[] }

/** What the source holds: the tables a test passes as `discovered`, else two small ones. */
function found(options: { discovered?: unknown[] }): Found[] {
  return (
    (options.discovered as Found[] | undefined) ?? [
      { name: "public.orders", columns: [{ name: "order_id", typeName: "smallint" }] },
      { name: "public.customers", columns: [{ name: "customer_id", typeName: "text" }] },
    ]
  )
}

function stubFetch(
  spec: typeof SPEC = SPEC,
  // SRC-11: the run record (`GET .../runs`) and the log of each run by id (`GET .../runs/{id}/log`).
  history: { runs: unknown[]; logs?: Record<string, unknown> } = { runs: [] },
  // SRC-10: what discovery finds, and how a run of the connector is answered (the default launches run 0123456789abcdef).
  // SRC-11: how Resume is answered (the default lifts the pause).
  options: {
    discovered?: unknown[]
    /** SRC-13: a listing of objects that are not tables (topics, endpoints, collections); wins over `discovered` for the list. */
    listing?: unknown[]
    onRun?: (body: unknown) => Response
    onResume?: () => Response
  } = {}
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
    if (url.endsWith("/runs")) return json({ runs: history.runs })
    const log = /\/runs\/([^/]+)\/log(\?format=text)?$/.exec(url)
    if (log) {
      if (log[2]) return new Response("run 0123 (schedule), succeeded\n", { status: 200, headers: { "Content-Type": "text/plain" } })
      const body = history.logs?.[log[1]!]
      return body ? json(body) : json({ error: "not found" }, 404)
    }
    if (url.endsWith("/resume") && method === "POST") return options.onResume ? options.onResume() : json({ resumed: true })
    if (url.includes("/debezium-properties")) {
      return json({ table: "public.orders", properties: "database.password=${DB_PASSWORD}", note: "References only." })
    }
    // SRC-13: names first (`/discovery/list`), the columns of one object when it is opened (`/discovery/preview`).
    if (url.endsWith("/discovery/list") && method === "POST") {
      return json({
        kind: "list",
        state: "done",
        objects: options.listing ?? found(options).map((o) => ({ name: o.name, kind: "table" })),
      })
    }
    if (url.endsWith("/discovery/preview") && method === "POST") {
      // Only a table a test names in `discovered` has columns to read; the default listing's tables answer
      // that the source cannot say, so a table saved or typed in a test keeps the typed key and cursor fields.
      const object = ((options.discovered as Found[] | undefined) ?? []).find((o) => o.name === body.object)
      if (!object) {
        return json({ kind: "preview", state: "failed", object: body.object, errorClass: "failed", reason: "The source could not be read." })
      }
      return json({
        kind: "preview",
        state: "done",
        object: object.name,
        columns: object.columns.map((c) => ({ ...c, inferred: false })),
        primaryKey: object.primaryKey ?? [],
      })
    }
    if (url.includes("/ingest/run")) return options.onRun ? options.onRun(body) : json({ runId: "0123456789abcdef" })
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
    // SRC-10: the merging mode needs the same cursor column; the replacing one needs none.
    expect(loadModeProblem({ loadMode: "incremental_dedup" })).toMatch(/Pick the column/)
    // ... and a key (the two lines below named no key before the key field existed).
    expect(loadModeProblem({ loadMode: "incremental_dedup", incrementalKey: "updated_at", primaryKey: ["id"] })).toBeNull()
    expect(loadModeProblem({ loadMode: "replace_dedup", primaryKey: ["id"] })).toBeNull()
    // PR #216 final review BLOCKER 18: `incremental` appends a changed business-app record as a second row.
    expect(loadModeProblem({ loadMode: "incremental", incrementalKey: "updated_at", adapter: "saas" })).toMatch(/one per key/)
    expect(loadModeProblem({ loadMode: "incremental", incrementalKey: "updated_at", adapter: "sql" })).toBeNull()
  })

  it("names a Bronze table after the connector and the source table", () => {
    expect(defaultTarget("northwind", "public.orders")).toBe("northwind_orders")
    expect(defaultTarget("Northwind DB!", "public.Order Details")).toBe("northwind_db_order_details")
    expect(defaultTarget("2024 sales", "t")).toBe("t_2024_sales_t")
  })
  it("names the table of a labelled object from the whole label (slice 5 review BLOCKER 14)", () => {
    expect(labelTarget("intranet", "Team Site / Tasks")).toBe("intranet_team_site_tasks")
    expect(labelTarget("Intranet", "v1.2 Plan / Q3 2026")).toBe("intranet_v1_2_plan_q3_2026")
    expect(labelTarget("2024", "HR / Staff")).toBe("t_2024_hr_staff")
    expect(labelTarget("", "!!!")).toBe("table")
  })
  // The edge cases below are pinned by `default_bronze_target`'s Rust test as
  // well (`lakehouse-store/src/ingest_spec.rs`): both rules must agree.
  it("falls back the way the Rust rule does when a part has nothing usable", () => {
    expect(defaultTarget("!!!", "dbo.Orders")).toBe("orders")
    expect(defaultTarget("shop", "public.")).toBe("shop")
    expect(defaultTarget("", "...")).toBe("table")
    expect(defaultTarget("shop", "orders")).toBe("shop_orders")
    expect(defaultTarget("  a--b  ", "s.__x__y__")).toBe("a_b_x_y")
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
    // SRC-13: the listing is the new route, asked for the schema in the field.
    const discover = calls.find((c) => c.url.endsWith("/discovery/list"))
    expect(discover?.url).toBe("/api/connectors/conn-northwind/discovery/list")
    expect(discover?.method).toBe("POST")
    expect(discover?.body).toEqual({ schema: "public" })

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
    expect([...mode.options].map((o) => o.value)).toEqual([
      "replace",
      "incremental",
      "append",
      "replace_dedup",
      "incremental_dedup",
      "history",
    ])
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
    expect([...restMode.options].map((o) => o.value)).toEqual(["replace", "append", "replace_dedup", "history"])
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
    // The runs card loads its own two lists after the spec, and shows a
    // skeleton until both are in: wait for the sentence, do not read it at once.
    expect(await card("Runs").findByText("No runs yet.")).toBeDefined()
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
        {
          runId: "run-1234567890",
          trigger: "schedule",
          status: "succeeded",
          startedAt: "2026-09-29T08:21:51.171Z",
          endedAt: "2026-09-29T08:22:33.646Z",
          durationSeconds: 42,
          rowsLoaded: 7,
          attempts: 1,
          endReason: "completed",
        },
      ],
    }

    it("has two lines in the card layout: the verdict, id and untruncated summary, then when and how long", async () => {
      stubFetch(SPEC, history)
      render(<ConnectorIngestPanel connectorId="conn-northwind" connectorName="northwind" layout="page" />)
      const summaryText = await screen.findByText("7 rows · 1 attempt")
      expect(summaryText.className).not.toContain("truncate")
      const firstLine = summaryText.parentElement as HTMLElement
      expect(within(firstLine).getByText("Succeeded")).toBeDefined()
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
      const summaryText = await screen.findByText("7 rows · 1 attempt")
      expect(summaryText.className).toContain("truncate")
      const line = summaryText.parentElement as HTMLElement
      expect(within(line).getByText("Succeeded")).toBeDefined()
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
    // `onTableCount` is called from an effect that runs after the table
    // appears, so wait for the call instead of reading it at once.
    await waitFor(() => expect(counts).toEqual([1]))

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

/**
 * SRC-10 decisions 2, 3 and 4: the modes a source offers, the key of a
 * deduplicating table (filled from discovery, typed otherwise), and the problems
 * the API would refuse shown before saving.
 */
describe("ConnectorIngestPanel load modes and keys", () => {
  const saved = (extra: Record<string, unknown> = {}) => ({
    ...SPEC,
    sourceObjects: [{ name: "public.orders", target: "northwind_orders", ...extra }],
  })
  const saveButton = () => screen.getByRole("button", { name: "Save tables and schedule" }) as HTMLButtonElement
  const modeOf = async (name: string) => (await screen.findByLabelText(`Load mode for ${name}`)) as HTMLSelectElement

  it("offers six modes to a SQL source, labelled in plain words", async () => {
    stubFetch(saved())
    render(<ConnectorIngestPanel connectorId="conn-northwind" connectorName="northwind" />)
    const mode = await modeOf("public.orders")
    expect([...mode.options].map((o) => o.textContent)).toEqual([
      "Replace all rows",
      "Add only new rows",
      "Add all rows again",
      "Replace all rows, one per key",
      "Add only new rows, one per key",
      "Keep history",
    ])
    // Nothing is missing, so there is no line about a missing mode.
    expect(screen.queryByText(/offered for SQL sources and business apps only/)).toBeNull()
  })

  it("offers four modes to any other batch source and says why the incremental ones are missing", async () => {
    stubFetch({
      ...SPEC,
      adapter: "rest",
      dial: { baseUrl: "https://api.example.com" } as unknown as typeof SPEC.dial,
      sourceObjects: [{ name: "orders", target: "api_orders" }],
    })
    render(<ConnectorIngestPanel connectorId="conn-api" connectorName="api" />)
    const mode = await modeOf("orders")
    expect([...mode.options].map((o) => o.value)).toEqual(["replace", "append", "replace_dedup", "history"])
    expect(screen.getByText(/Adding only new rows, with or without one row per key, is offered for SQL sources and business apps only/)).toBeDefined()
  })

  it("offers five modes to a business app, without the append-only incremental one (final review BLOCKER 18)", async () => {
    stubFetch({
      ...SPEC,
      adapter: "saas",
      dial: { vendor: "zendesk", account: "acme", auth: { type: "integration" } } as unknown as typeof SPEC.dial,
      sourceObjects: [{ name: "tickets", target: "saas_tickets" }],
    })
    render(<ConnectorIngestPanel connectorId="conn-saas" connectorName="acme" />)
    const mode = await modeOf("tickets")
    expect([...mode.options].map((o) => o.value)).toEqual(["replace", "append", "replace_dedup", "incremental_dedup", "history"])
    expect(screen.queryByText(/offered for SQL sources and business apps only/)).toBeNull()
  })

  it("says in each hint what happens to the rows already in the table", async () => {
    stubFetch(saved())
    render(<ConnectorIngestPanel connectorId="conn-northwind" connectorName="northwind" />)
    const mode = await modeOf("public.orders")
    const hints: Record<string, RegExp> = {
      replace: /replaces the rows already in the table/,
      append: /below the rows already in the table/,
      replace_dedup: /replaces the rows already in the table with a fresh copy of the source, one row per key/,
      incremental: /keep the rows already there/,
      incremental_dedup: /merge .* into the rows already there/,
    }
    for (const [value, hint] of Object.entries(hints)) {
      fireEvent.change(mode, { target: { value } })
      if (value.startsWith("incremental")) {
        fireEvent.change(screen.getByLabelText("New-row column for public.orders"), { target: { value: "order_id" } })
      }
      if (value.endsWith("_dedup")) {
        fireEvent.change(screen.getByLabelText("Key columns for public.orders"), { target: { value: "order_id" } })
      }
      expect(screen.getByText(hint)).toBeDefined()
    }
  })

  it("blocks Save for a deduplicating mode with no key, and for incremental and deduplicate with no cursor", async () => {
    stubFetch(saved())
    render(<ConnectorIngestPanel connectorId="conn-northwind" connectorName="northwind" />)
    const mode = await modeOf("public.orders")

    fireEvent.change(mode, { target: { value: "replace_dedup" } })
    expect(screen.getByText(/cannot be saved without a key/)).toBeDefined()
    expect(saveButton().disabled).toBe(true)
    fireEvent.change(screen.getByLabelText("Key columns for public.orders"), { target: { value: "order_id" } })
    expect(screen.queryByText(/cannot be saved without a key/)).toBeNull()
    expect(saveButton().disabled).toBe(false)

    fireEvent.change(mode, { target: { value: "incremental_dedup" } })
    expect(screen.getByText(/Pick the column that marks new rows/)).toBeDefined()
    expect(saveButton().disabled).toBe(true)
    fireEvent.change(screen.getByLabelText("New-row column for public.orders"), { target: { value: "updated_at" } })
    expect(saveButton().disabled).toBe(false)

    // Clearing the key again blocks Save again.
    fireEvent.change(screen.getByLabelText("Key columns for public.orders"), { target: { value: "" } })
    expect(screen.getByText(/cannot be saved without a key/)).toBeDefined()
    expect(saveButton().disabled).toBe(true)
  })

  it("saves a typed key as a list of columns, with the cursor, for a table typed by hand", async () => {
    const calls = stubFetch({ ...SPEC, adapter: "rest", dial: {} as unknown as typeof SPEC.dial, sourceObjects: [] })
    render(<ConnectorIngestPanel connectorId="conn-api" connectorName="api" />)
    fireEvent.change(await screen.findByLabelText("Add by name"), { target: { value: "orders" } })
    fireEvent.click(screen.getByRole("button", { name: "Add" }))
    fireEvent.change(await modeOf("orders"), { target: { value: "replace_dedup" } })
    fireEvent.change(screen.getByLabelText("Key columns for orders"), { target: { value: "id, region" } })
    fireEvent.click(saveButton())
    await waitFor(() => expect(calls.some((c) => c.method === "PUT")).toBe(true))
    expect((calls.find((c) => c.method === "PUT")?.body as { sourceObjects: unknown }).sourceObjects).toEqual([
      { name: "orders", target: "api_orders", loadMode: "replace_dedup", primaryKey: ["id", "region"] },
    ])
  })

  const FOUND = [
    {
      name: "public.orders",
      columns: [
        { name: "order_id", typeName: "integer" },
        { name: "region", typeName: "text" },
        { name: "updated_at", typeName: "timestamp" },
      ],
      primaryKey: ["order_id", "region"],
    },
    { name: "public.customers", columns: [{ name: "customer_id", typeName: "text" }], primaryKey: [] },
  ]

  it("fills the key from the source's own when a found table is ticked, and lets the user change it", async () => {
    const calls = stubFetch(SPEC, undefined, { discovered: FOUND })
    render(<ConnectorIngestPanel connectorId="conn-northwind" connectorName="northwind" />)
    fireEvent.click(await screen.findByRole("button", { name: /Find tables/ }))
    fireEvent.click(await screen.findByLabelText(/public\.orders/))
    fireEvent.change(await modeOf("public.orders"), { target: { value: "incremental_dedup" } })
    fireEvent.change(screen.getByLabelText("New-row column for public.orders"), { target: { value: "updated_at" } })

    const ticked = (name: string) =>
      (screen.getByLabelText(`${name} is part of the key of public.orders`) as HTMLInputElement).checked
    expect([ticked("order_id"), ticked("region"), ticked("updated_at")]).toEqual([true, true, false])
    expect(screen.getByText("Filled in from the source, which declares order_id, region as this table's key.")).toBeDefined()

    // Change it: untick region.
    fireEvent.click(screen.getByLabelText("region is part of the key of public.orders"))
    expect(screen.getByText(/The source declares order_id, region as this table's key; this table uses another\./)).toBeDefined()
    fireEvent.click(saveButton())
    await waitFor(() => expect(calls.some((c) => c.method === "PUT")).toBe(true))
    expect((calls.find((c) => c.method === "PUT")?.body as { sourceObjects: unknown }).sourceObjects).toEqual([
      {
        name: "public.orders",
        target: "northwind_orders",
        loadMode: "incremental_dedup",
        incrementalKey: "updated_at",
        primaryKey: ["order_id"],
      },
    ])
  })

  it("asks for a key when the source declares none, and fills it when a table already picked is switched to a mode that needs one", async () => {
    stubFetch(SPEC, undefined, { discovered: FOUND })
    render(<ConnectorIngestPanel connectorId="conn-northwind" connectorName="northwind" />)
    fireEvent.click(await screen.findByRole("button", { name: /Find tables/ }))
    fireEvent.click(await screen.findByLabelText(/public\.customers/))
    fireEvent.change(await modeOf("public.customers"), { target: { value: "replace_dedup" } })
    // Declares no key: nothing is ticked, and Save says why it is off.
    expect((screen.getByLabelText("customer_id is part of the key of public.customers") as HTMLInputElement).checked).toBe(false)
    expect(screen.getByText(/cannot be saved without a key/)).toBeDefined()
    expect(saveButton().disabled).toBe(true)
  })

  it("keeps a saved key and mode as they are", async () => {
    stubFetch(saved({ loadMode: "replace_dedup", primaryKey: ["id"] }))
    render(<ConnectorIngestPanel connectorId="conn-northwind" connectorName="northwind" />)
    const mode = await modeOf("public.orders")
    expect(mode.value).toBe("replace_dedup")
    expect((screen.getByLabelText("Key columns for public.orders") as HTMLInputElement).value).toBe("id")
    expect(saveButton().disabled).toBe(true)
  })
})

/**
 * SRC-10 sixth pass, decisions 1, 9 and 10: the history mode. It reads the whole table, so it takes a key and
 * no cursor, and a table cannot cross to or from it without "Refresh, removing rows".
 */
describe("ConnectorIngestPanel history mode", () => {
  const saved = (extra: Record<string, unknown> = {}) => ({
    ...SPEC,
    sourceObjects: [{ name: "public.orders", target: "northwind_orders", ...extra }],
  })
  const saveButton = () => screen.getByRole("button", { name: "Save tables and schedule" }) as HTMLButtonElement
  const modeOf = async (name: string) => (await screen.findByLabelText(`Load mode for ${name}`)) as HTMLSelectElement
  const BOUNDARY = "After saving, use Refresh, removing rows on this table: its rows must be rebuilt for the new mode."

  it("says in one line what is kept and that each run reads the whole table", async () => {
    stubFetch(saved())
    render(<ConnectorIngestPanel connectorId="conn-northwind" connectorName="northwind" />)
    fireEvent.change(await modeOf("public.orders"), { target: { value: "history" } })
    fireEvent.change(screen.getByLabelText("Key columns for public.orders"), { target: { value: "order_id" } })
    expect(
      screen.getByText(
        "Every version of a row is kept, with the time a load first saw it (_valid_from), the time it was replaced (_valid_to) and a mark on the current one (_is_current). Each run reads the whole table."
      )
    ).toBeDefined()
  })

  it("asks for a key like the deduplicating modes, and shows no cursor field", async () => {
    stubFetch(saved())
    render(<ConnectorIngestPanel connectorId="conn-northwind" connectorName="northwind" />)
    fireEvent.change(await modeOf("public.orders"), { target: { value: "history" } })
    expect(screen.getByText(/cannot be saved without a key/)).toBeDefined()
    expect(saveButton().disabled).toBe(true)
    expect(screen.queryByLabelText("New-row column for public.orders")).toBeNull()
    fireEvent.change(screen.getByLabelText("Key columns for public.orders"), { target: { value: "order_id" } })
    expect(screen.queryByText(/cannot be saved without a key/)).toBeNull()
    expect(saveButton().disabled).toBe(false)
  })

  it("saves the key and no cursor, even if the table had a cursor before", async () => {
    const calls = stubFetch(saved({ loadMode: "incremental", incrementalKey: "updated_at" }))
    render(<ConnectorIngestPanel connectorId="conn-northwind" connectorName="northwind" />)
    fireEvent.change(await modeOf("public.orders"), { target: { value: "history" } })
    fireEvent.change(screen.getByLabelText("Key columns for public.orders"), { target: { value: "order_id, region" } })
    fireEvent.click(saveButton())
    await waitFor(() => expect(calls.some((c) => c.method === "PUT")).toBe(true))
    expect((calls.find((c) => c.method === "PUT")?.body as { sourceObjects: unknown }).sourceObjects).toEqual([
      { name: "public.orders", target: "northwind_orders", loadMode: "history", primaryKey: ["order_id", "region"] },
    ])
  })

  it("fills the key from the source's own when a found table is switched to history", async () => {
    stubFetch(SPEC, undefined, {
      discovered: [
        {
          name: "public.orders",
          columns: [{ name: "order_id", typeName: "integer" }],
          primaryKey: ["order_id"],
        },
      ],
    })
    render(<ConnectorIngestPanel connectorId="conn-northwind" connectorName="northwind" />)
    fireEvent.click(await screen.findByRole("button", { name: /Find tables/ }))
    fireEvent.click(await screen.findByLabelText(/public\.orders/))
    fireEvent.change(await modeOf("public.orders"), { target: { value: "history" } })
    expect((screen.getByLabelText("order_id is part of the key of public.orders") as HTMLInputElement).checked).toBe(true)
    expect(saveButton().disabled).toBe(false)
  })

  it("says to use Refresh, removing rows when a saved table is switched to history, and not before", async () => {
    stubFetch(saved({ loadMode: "replace" }))
    render(<ConnectorIngestPanel connectorId="conn-northwind" connectorName="northwind" />)
    const mode = await modeOf("public.orders")
    expect(screen.queryByText(BOUNDARY)).toBeNull()
    fireEvent.change(mode, { target: { value: "append" } })
    expect(screen.queryByText(BOUNDARY)).toBeNull()
    fireEvent.change(mode, { target: { value: "history" } })
    expect(screen.getByText(BOUNDARY)).toBeDefined()
    fireEvent.change(mode, { target: { value: "replace" } })
    expect(screen.queryByText(BOUNDARY)).toBeNull()
  })

  it("says it too when a saved history table is switched to another mode", async () => {
    stubFetch(saved({ loadMode: "history", primaryKey: ["id"] }))
    render(<ConnectorIngestPanel connectorId="conn-northwind" connectorName="northwind" />)
    const mode = await modeOf("public.orders")
    expect(mode.value).toBe("history")
    expect(screen.queryByText(BOUNDARY)).toBeNull()
    fireEvent.change(mode, { target: { value: "replace_dedup" } })
    expect(screen.getByText(BOUNDARY)).toBeDefined()
  })

  it("says nothing for a table that is not saved yet", async () => {
    stubFetch(SPEC, undefined, {
      discovered: [{ name: "public.orders", columns: [{ name: "order_id", typeName: "integer" }], primaryKey: ["order_id"] }],
    })
    render(<ConnectorIngestPanel connectorId="conn-northwind" connectorName="northwind" />)
    fireEvent.click(await screen.findByRole("button", { name: /Find tables/ }))
    fireEvent.click(await screen.findByLabelText(/public\.orders/))
    fireEvent.change(await modeOf("public.orders"), { target: { value: "history" } })
    expect(screen.queryByText(BOUNDARY)).toBeNull()
  })

  it("shows the refusal of a run as the API answered it, and keeps the dialog open", async () => {
    const refusal =
      'table \'orders\' holds rows that were not loaded in the history mode; use "Refresh, removing rows" on it to load it with history'
    stubFetch(saved({ loadMode: "history", primaryKey: ["id"] }), undefined, { onRun: () => json({ error: refusal }, 422) })
    render(<ConnectorIngestPanel connectorId="conn-northwind" connectorName="northwind" layout="page" />)
    await waitFor(() =>
      expect((screen.getByRole("button", { name: "Refresh or clear public.orders" }) as HTMLButtonElement).disabled).toBe(false)
    )
    await act(async () => {})
    const trigger = screen.getByRole("button", { name: "Refresh or clear public.orders" })
    fireEvent.pointerDown(trigger, { button: 0, pointerType: "mouse" })
    fireEvent.mouseDown(trigger, { button: 0 })
    fireEvent.pointerUp(trigger, { button: 0, pointerType: "mouse" })
    fireEvent.mouseUp(trigger, { button: 0 })
    fireEvent.click(trigger)
    fireEvent.click(await screen.findByText("Refresh, keeping rows"))
    fireEvent.click(await screen.findByRole("button", { name: "Refresh" }))
    expect(await screen.findByText(refusal)).toBeDefined()
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }))
    await waitFor(() => expect(screen.queryByRole("button", { name: "Cancel" })).toBeNull())
  })
})

/**
 * SRC-10 decision 5: refresh or clear one table, each behind a confirmation, and
 * the run view naming the table and the action.
 */
/** A run record as `GET .../runs` answers it; a test overrides what it is about. */
function record(extra: Record<string, unknown> = {}) {
  return {
    runId: "run-0000000001",
    trigger: "schedule",
    table: null,
    status: "succeeded",
    startedAt: "2026-10-12T08:00:00.000Z",
    endedAt: "2026-10-12T08:00:30.000Z",
    durationSeconds: 30,
    rowsLoaded: 100,
    attempts: 1,
    endReason: "completed",
    ...extra,
  }
}

describe("ConnectorIngestPanel refresh and clear", () => {
  const SAVED = {
    ...SPEC,
    sourceObjects: [{ name: "public.orders", target: "northwind_orders", loadMode: "replace" }],
  }
  const menu = () => screen.getByRole("button", { name: "Refresh or clear public.orders" }) as HTMLButtonElement

  async function pick(item: string) {
    // Until the run list and the schema changes have loaded the row shows a disabled placeholder, so wait for
    // the enabled trigger. That alone is not enough (measured: a single press still failed about one run in
    // three of this folder): the state updates that follow the enabling were still pending, and a press
    // before they were flushed did not open the menu. Flushing them with `act` makes ONE press open it
    // (20 runs of this folder, 0 failures). Which update it is was not identified.
    await waitFor(() => expect(menu().disabled).toBe(false))
    await act(async () => {})
    const trigger = menu()
    fireEvent.pointerDown(trigger, { button: 0, pointerType: "mouse" })
    fireEvent.mouseDown(trigger, { button: 0 })
    fireEvent.pointerUp(trigger, { button: 0, pointerType: "mouse" })
    fireEvent.mouseUp(trigger, { button: 0 })
    fireEvent.click(trigger)
    await screen.findByText(item)
    fireEvent.click(screen.getByText(item))
  }

  it.each([
    ["Refresh, keeping rows", "refresh_keep", /Rows already in bronze\.northwind_orders are kept/, "Refresh"],
    ["Refresh, removing rows", "refresh_remove", /Removes every row in bronze\.northwind_orders, then loads the whole table again\. The removed rows cannot be brought back from here\./, "Remove rows and refresh"],
    ["Clear", "clear", /Removes every row in bronze\.northwind_orders and loads nothing\. The removed rows cannot be brought back from here/, "Clear table"],
  ])("%s asks first, then sends the table and the action", async (item, action, sentence, confirmLabel) => {
    const calls = stubFetch(SAVED)
    render(<ConnectorIngestPanel connectorId="conn-northwind" connectorName="northwind" layout="page" />)
    await pick(item)

    // Nothing was sent before the confirmation, and the dialog says what happens to the rows.
    expect(calls.some((c) => c.url.endsWith("/ingest/run"))).toBe(false)
    expect(await screen.findByText(sentence)).toBeDefined()
    fireEvent.click(screen.getByRole("button", { name: confirmLabel }))

    await waitFor(() => expect(calls.some((c) => c.url.endsWith("/ingest/run") && c.method === "POST")).toBe(true))
    const post = calls.find((c) => c.url.endsWith("/ingest/run") && c.method === "POST")
    expect(post?.body).toEqual({ object: "public.orders", action })
    // The run is being waited for, as for Run now.
    await screen.findByText(/Starting run 01234567/)
    expect((screen.getByRole("button", { name: "Run now" }) as HTMLButtonElement).disabled).toBe(true)
  })

  it("sends nothing when the confirmation is cancelled", async () => {
    const calls = stubFetch(SAVED)
    render(<ConnectorIngestPanel connectorId="conn-northwind" connectorName="northwind" layout="page" />)
    await pick("Clear")
    fireEvent.click(await screen.findByRole("button", { name: "Cancel" }))
    expect(calls.some((c) => c.url.endsWith("/ingest/run"))).toBe(false)
  })

  it("is disabled, with the reason as its title, while a run of this connector is going", async () => {
    stubFetch(SAVED, {
      runs: [record({ runId: "run-live-0001", status: "running", startedAt: new Date().toISOString(), endedAt: null })],
    })
    render(<ConnectorIngestPanel connectorId="conn-northwind" connectorName="northwind" layout="page" />)
    await waitFor(() => expect(menu().disabled).toBe(true))
    expect(menu().title).toBe("A run of this connector is going; wait for it to finish.")
  })

  it("is disabled while the connector is paused, with the pause as the reason", async () => {
    stubFetch(SAVED)
    render(<ConnectorIngestPanel connectorId="conn-northwind" connectorName="northwind" layout="page" pausedReason="schema changed" />)
    await waitFor(() => expect(menu().disabled).toBe(true))
    expect(menu().title).toBe("Paused: schema changed")
  })

  it("is disabled for a table that waits for a schema decision, and not for the others", async () => {
    stubFetch({
      ...SPEC,
      sourceObjects: [
        { name: "public.orders", target: "northwind_orders" },
        { name: "public.items", target: "northwind_items" },
      ],
    })
    render(
      <ConnectorIngestPanel
        connectorId="conn-northwind"
        connectorName="northwind"
        layout="page"
        waitingTables={new Set(["public.orders"])}
      />
    )
    await waitFor(() => expect(menu().disabled).toBe(true))
    expect(menu().title).toMatch(/waits for a schema decision/)
    expect((screen.getByRole("button", { name: "Refresh or clear public.items" }) as HTMLButtonElement).disabled).toBe(false)
  })

  it("is disabled until unsaved changes are saved", async () => {
    stubFetch(SAVED)
    render(<ConnectorIngestPanel connectorId="conn-northwind" connectorName="northwind" layout="page" />)
    await waitFor(() => expect(menu().disabled).toBe(false))
    fireEvent.change(await screen.findByLabelText("Bronze table for public.orders"), { target: { value: "orders_two" } })
    await waitFor(() => expect(menu().disabled).toBe(true))
    expect(menu().title).toBe("Save your changes before refreshing or clearing a table.")
  })

  it("shows the API's refusal as it answered and keeps the dialog open", async () => {
    const refusal = "a run for this connector is already running (run 7bd3a0c4); wait for it to finish before starting another"
    stubFetch(SAVED, undefined, { onRun: () => json({ error: refusal }, 409) })
    render(<ConnectorIngestPanel connectorId="conn-northwind" connectorName="northwind" layout="page" />)
    await pick("Refresh, removing rows")
    fireEvent.click(await screen.findByRole("button", { name: "Remove rows and refresh" }))
    expect(await screen.findByText(refusal)).toBeDefined()
    expect(screen.getByRole("button", { name: "Remove rows and refresh" })).toBeDefined()
    // Closed before the test ends: an open modal leaves the page inert for the next test.
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }))
    await waitFor(() => expect(screen.queryByRole("button", { name: "Cancel" })).toBeNull())
  })

  it("shows a table that waits as refused too, with the API's own words", async () => {
    const refusal = "This table is waiting for a schema decision, so it cannot be refreshed or cleared yet. Approve or resolve the change first."
    stubFetch(SAVED, undefined, { onRun: () => json({ error: refusal }, 409) })
    render(<ConnectorIngestPanel connectorId="conn-northwind" connectorName="northwind" layout="page" />)
    await pick("Clear")
    fireEvent.click(await screen.findByRole("button", { name: "Clear table" }))
    expect(await screen.findByText(refusal)).toBeDefined()
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }))
    await waitFor(() => expect(screen.queryByRole("button", { name: "Cancel" })).toBeNull())
  })

  it("has no table menu for a CDC connector, whose tables stream", async () => {
    stubFetch({ ...SPEC, adapter: "cdc", ingestMode: "cdc", sourceObjects: [{ name: "public.orders", target: "northwind_orders" }] })
    render(<ConnectorIngestPanel connectorId="conn-northwind" connectorName="northwind" layout="page" />)
    await screen.findByText("public.orders")
    expect(screen.queryByRole("button", { name: "Refresh or clear public.orders" })).toBeNull()
  })
})

/**
 * SRC-11 decision 7: the Runs card reads the product's own run record (newest 50), with how each run was started,
 * its rows, duration and attempts, and no longer matches table results to orchestrator runs by time. A run whose
 * record says running for longer than the bound is "no end reported", not active forever.
 */
describe("ConnectorIngestPanel runs from the run record", () => {
  const panel = () =>
    render(<ConnectorIngestPanel connectorId="conn-northwind" connectorName="northwind" layout="page" />)

  it("lists each run with its trigger, rows, attempts and duration, from the run record only", async () => {
    const calls = stubFetch(SPEC, {
      runs: [
        record({ runId: "run-sched-0001", trigger: "schedule", rowsLoaded: 1200, attempts: 3, durationSeconds: 95 }),
        record({ runId: "run-user-00002", trigger: "user", status: "failed", endReason: "table_failed", rowsLoaded: 0, attempts: 4 }),
        record({ runId: "run-clear-0003", trigger: "clear", rowsLoaded: 0 }),
        record({ runId: "run-paused-004", trigger: "other", status: "paused", endReason: "connector_paused", rowsLoaded: 0, attempts: 0 }),
      ],
    })
    panel()
    expect(await screen.findByText("1,200 rows · 3 attempts")).toBeDefined()
    // "Schedule" is also the title of the card beside this one.
    const runsCard = screen.getByText("Runs", { selector: "[data-slot=card-title]" }).closest("[data-slot=card]") as HTMLElement
    expect(within(runsCard).getByText("Schedule")).toBeDefined()
    expect(screen.getByText("Started by a user")).toBeDefined()
    expect(screen.getByText("Clear")).toBeDefined()
    expect(screen.getByText("Started from the orchestrator")).toBeDefined()
    expect(screen.getByText(/took 1m 35s/)).toBeDefined()
    expect(screen.getByText("0 rows · 4 attempts")).toBeDefined()
    expect(screen.getByText("Failed")).toBeDefined()
    expect(screen.getByText("Paused")).toBeDefined()
    // The old reads are gone: no per-table results, no orchestrator run list.
    expect(calls.some((c) => c.url.includes("/api/governance/ingest-runs") || c.url.includes("/ingest/runs"))).toBe(false)
    expect(calls.some((c) => c.url.endsWith("/api/connectors/conn-northwind/runs"))).toBe(true)
  })

  // SRC-10 decision 5, restored by SRC-11 review 1 BLOCKER 2: the run record now carries the table.
  it("names the table and the action in the run list", async () => {
    stubFetch(SPEC, {
      runs: [
        record({ runId: "run-clear-0001", trigger: "clear", table: "public.orders", rowsLoaded: 0 }),
        record({ runId: "run-refresh-002", trigger: "refresh", table: "public.customers" }),
        record({ runId: "run-all-0000003", trigger: "schedule" }),
      ],
    })
    panel()
    expect(await screen.findByText("Clear · public.orders")).toBeDefined()
    expect(screen.getByText("Refresh · public.customers")).toBeDefined()
    // A run of every table names no table.
    expect(screen.queryByText(/^Schedule · /)).toBeNull()
  })

  it("says why each run ended, in plain words", async () => {
    stubFetch(SPEC, {
      runs: [
        record({ runId: "run-0000000001", endReason: "completed" }),
        record({ runId: "run-0000000002", status: "failed", endReason: "table_failed" }),
        record({ runId: "run-0000000003", status: "paused", endReason: "connector_paused" }),
      ],
    })
    panel()
    expect(await screen.findAllByText("Every table was loaded.")).toHaveLength(1)
    expect(screen.getByText("At least one table could not be loaded.")).toBeDefined()
    expect(screen.getByText("The connector was paused, so nothing was loaded.")).toBeDefined()
  })

  it("says there are no runs yet when the record is empty", async () => {
    stubFetch(SPEC)
    panel()
    expect(await screen.findByText("No runs yet.")).toBeDefined()
  })

  it("shows a run recorded as running but started long ago as no end reported, and leaves Run now enabled", async () => {
    stubFetch({ ...SPEC, sourceObjects: [{ name: "public.orders", target: "northwind_orders" }] }, {
      runs: [record({ runId: "run-stuck-0001", status: "running", startedAt: "2026-10-01T08:00:00.000Z", endedAt: null, durationSeconds: null, endReason: null })],
    })
    panel()
    expect(await screen.findAllByText("No end reported")).toHaveLength(2) // the pill and the summary
    expect(screen.getByText("This run never reported its end. It may have been stopped before it finished.")).toBeDefined()
    expect(screen.queryByText(/is running/)).toBeNull()
    await waitFor(() => expect((screen.getByRole("button", { name: "Run now" }) as HTMLButtonElement).disabled).toBe(false))
  })

  it("shows a run recorded as running a moment ago as active, with Run now disabled", async () => {
    stubFetch({ ...SPEC, sourceObjects: [{ name: "public.orders", target: "northwind_orders" }] }, {
      runs: [record({ runId: "run-live-00001", status: "running", startedAt: new Date().toISOString(), endedAt: null, durationSeconds: null, endReason: null })],
    })
    panel()
    expect(await screen.findByText(/Run run-live is running/)).toBeDefined()
    expect(screen.getByText("Running")).toBeDefined()
    expect((screen.getByRole("button", { name: "Run now" }) as HTMLButtonElement).disabled).toBe(true)
  })

  it("says the run record could not be loaded, with the API's words", async () => {
    stubFetch(SPEC)
    const original = global.fetch
    global.fetch = mock(async (input: RequestInfo | URL, init?: RequestInit) => {
      if (String(input).endsWith("/runs")) return json({ error: "the store is unavailable" }, 500)
      return original(input, init)
    }) as unknown as typeof fetch
    panel()
    expect(await screen.findByText(/the store is unavailable/)).toBeDefined()
  })
})

describe("ConnectorIngestPanel run log", () => {
  const LOG = {
    run: record({ runId: "run-log-000001", status: "succeeded", attempts: 3 }),
    attempts: [
      { table: "public.orders", attempt: 1, startedAt: "2026-10-12T08:00:01.000Z", endedAt: "2026-10-12T08:00:02.000Z", rowsLoaded: 0, result: "failed", errorClass: "unreachable", waitSeconds: 60 },
      { table: "public.orders", attempt: 2, startedAt: "2026-10-12T08:01:05.000Z", endedAt: "2026-10-12T08:01:06.000Z", rowsLoaded: 0, result: "failed", errorClass: "sign_in_refused", waitSeconds: null },
      { table: "public.customers", attempt: 1, startedAt: "2026-10-12T08:01:10.000Z", endedAt: "2026-10-12T08:01:20.000Z", rowsLoaded: 1200, result: "succeeded", errorClass: null, waitSeconds: null },
    ],
    lines: [
      { at: "2026-10-12T08:00:00.000Z", event: "run_started", table: null, attempt: null, text: "Run started by the schedule" },
      { at: "2026-10-12T08:00:02.000Z", event: "retry_wait", table: "public.orders", attempt: 1, text: "Next attempt in 1 min" },
      { at: "2026-10-12T08:01:20.000Z", event: "run_ended", table: null, attempt: null, text: "Run ended: every table loaded" },
    ],
  }
  const panel = () =>
    render(<ConnectorIngestPanel connectorId="conn-northwind" connectorName="northwind" layout="page" />)

  it("opens a run's log on request: each attempt with its result in product words, then the lines", async () => {
    const calls = stubFetch(SPEC, { runs: [LOG.run], logs: { "run-log-000001": LOG } })
    panel()
    // Nothing is read until the log is asked for.
    await screen.findByText("Show log")
    expect(calls.some((c) => c.url.includes("/log"))).toBe(false)
    fireEvent.click(screen.getByRole("button", { name: "Show log" }))

    expect(await screen.findByText("The source could not be reached. Next attempt in 1 min.")).toBeDefined()
    expect(screen.getByText("The source refused the sign-in")).toBeDefined()
    expect(screen.getByText("1,200 rows")).toBeDefined()
    expect(screen.getAllByText("attempt 1")).toHaveLength(2)
    expect(screen.getByText("Run started")).toBeDefined()
    expect(screen.getByText("Waiting to retry")).toBeDefined()
    expect(screen.getByText("Run ended")).toBeDefined()
    expect(screen.getByText("Next attempt in 1 min")).toBeDefined()
    expect(calls.some((c) => c.url.endsWith("/runs/run-log-000001/log"))).toBe(true)
  })

  it("words the two business-app classes (SRC-3, migration 0097) and says a rate limit used its retries", async () => {
    const log = {
      ...LOG,
      attempts: [
        { table: "tickets", attempt: 1, startedAt: "2026-10-12T08:00:01.000Z", endedAt: "2026-10-12T08:00:02.000Z", rowsLoaded: 0, result: "failed", errorClass: "rate_limited", waitSeconds: null },
        { table: "owners", attempt: 1, startedAt: "2026-10-12T08:01:05.000Z", endedAt: "2026-10-12T08:01:06.000Z", rowsLoaded: 0, result: "failed", errorClass: "permission_missing", waitSeconds: null },
      ],
    }
    stubFetch(SPEC, { runs: [LOG.run], logs: { "run-log-000001": log } })
    panel()
    fireEvent.click(await screen.findByRole("button", { name: "Show log" }))
    // A rate limit is retried, so with no wait left it says the retries are used up; a missing permission is not.
    expect(await screen.findByText("The source limited the rate of requests. Retries are used up.")).toBeDefined()
    expect(screen.getByText("The account lacks a permission at the source")).toBeDefined()
  })

  it("downloads the same log as text from the same route with ?format=text", async () => {
    const calls = stubFetch(SPEC, { runs: [LOG.run], logs: { "run-log-000001": LOG } })
    const created: Blob[] = []
    const originalCreate = URL.createObjectURL
    const originalRevoke = URL.revokeObjectURL
    URL.createObjectURL = ((blob: Blob) => {
      created.push(blob)
      return "blob:log"
    }) as typeof URL.createObjectURL
    URL.revokeObjectURL = (() => {}) as typeof URL.revokeObjectURL
    try {
      panel()
      fireEvent.click(await screen.findByRole("button", { name: "Show log" }))
      fireEvent.click(await screen.findByRole("button", { name: "Download log" }))
      await waitFor(() => expect(created).toHaveLength(1))
      expect(await created[0]!.text()).toBe("run 0123 (schedule), succeeded\n")
      expect(calls.some((c) => c.url.endsWith("/runs/run-log-000001/log?format=text"))).toBe(true)
    } finally {
      URL.createObjectURL = originalCreate
      URL.revokeObjectURL = originalRevoke
    }
  })

  it("says so, and still offers the download, when a run has no log lines left", async () => {
    stubFetch(SPEC, { runs: [LOG.run], logs: { "run-log-000001": { run: LOG.run, attempts: [], lines: [] } } })
    panel()
    fireEvent.click(await screen.findByRole("button", { name: "Show log" }))
    expect(await screen.findByText(/This run has no log lines\. Logs are kept for 30 days/)).toBeDefined()
    expect(screen.getByRole("button", { name: "Download log" })).toBeDefined()
  })

  it("shows the API's words when the log cannot be read", async () => {
    stubFetch(SPEC, { runs: [LOG.run], logs: {} })
    panel()
    fireEvent.click(await screen.findByRole("button", { name: "Show log" }))
    expect(await screen.findByText(/not found/)).toBeDefined()
  })
})

describe("ConnectorIngestPanel resume", () => {
  const AUTO = "Paused after 30 failed runs in a row"
  const panel = (props: Partial<React.ComponentProps<typeof ConnectorIngestPanel>> = {}) =>
    render(
      <ConnectorIngestPanel
        connectorId="conn-northwind"
        connectorName="northwind"
        layout="page"
        pausedReason={AUTO}
        {...props}
      />
    )

  it("recognises the pause repeated failures set, whatever the number, and no other", () => {
    expect(isAutoDisablePause("Paused after 30 failed runs in a row")).toBe(true)
    expect(isAutoDisablePause("Paused after 3 failed runs in a row")).toBe(true)
    expect(isAutoDisablePause("Paused after  failed runs in a row")).toBe(false)
    expect(isAutoDisablePause("Paused by the schema-change policy until a change is approved")).toBe(false)
    expect(isAutoDisablePause(null)).toBe(false)
  })

  it("offers Resume on the auto-disable pause, posts it, and tells the page", async () => {
    const calls = stubFetch(SPEC)
    const onResumed = mock(() => {})
    panel({ onResumed })
    expect(await screen.findByText(`Paused: ${AUTO}`)).toBeDefined()
    fireEvent.click(screen.getByRole("button", { name: "Resume" }))
    await waitFor(() => expect(onResumed).toHaveBeenCalled())
    const post = calls.find((c) => c.url.endsWith("/api/connectors/conn-northwind/resume"))
    expect(post?.method).toBe("POST")
    expect(post?.body).toBeUndefined()
  })

  it("shows the 409's fixed text as it came, and does not tell the page", async () => {
    const refusal =
      "This connector is not paused because of repeated failures. If a schema change is waiting, approve it under Schema changes to resume it."
    stubFetch(SPEC, undefined, { onResume: () => json({ error: refusal }, 409) })
    const onResumed = mock(() => {})
    panel({ onResumed })
    fireEvent.click(await screen.findByRole("button", { name: "Resume" }))
    expect((await screen.findByRole("alert")).textContent).toBe(refusal)
    expect(onResumed).not.toHaveBeenCalled()
  })

  it("offers no Resume for a pause the schema-change policy set", async () => {
    stubFetch(SPEC)
    panel({ pausedReason: "Paused by the schema-change policy until a change is approved" })
    expect(await screen.findByText(/Paused: Paused by the schema-change policy/)).toBeDefined()
    expect(screen.queryByRole("button", { name: "Resume" })).toBeNull()
  })

  it("disables Run now while paused", async () => {
    stubFetch({ ...SPEC, sourceObjects: [{ name: "public.orders", target: "northwind_orders" }] })
    panel()
    await screen.findByText(`Paused: ${AUTO}`)
    expect((screen.getByRole("button", { name: "Run now" }) as HTMLButtonElement).disabled).toBe(true)
  })
})

/**
 * SRC-11 decisions 9 to 11: column choice per table for PostgreSQL, MySQL (MariaDB) and SQL Server; key and
 * cursor columns ticked and locked with the reason; other types say it is not available.
 */
describe("ConnectorIngestPanel column choice", () => {
  const FOUND = [
    {
      name: "public.orders",
      columns: [
        { name: "order_id", typeName: "integer" },
        { name: "region", typeName: "text" },
        { name: "updated_at", typeName: "timestamp" },
        { name: "discount", typeName: "numeric" },
      ],
      primaryKey: ["order_id"],
    },
  ]
  const saved = (extra: Record<string, unknown> = {}, spec: typeof SPEC = SPEC) => ({
    ...spec,
    sourceObjects: [{ name: "public.orders", target: "northwind_orders", loadMode: "replace", ...extra }],
  })
  const saveButton = () => screen.getByRole("button", { name: "Save tables and schedule" })
  const box = (column: string) => screen.getByLabelText(`Load column ${column} of public.orders`) as HTMLInputElement

  async function openColumns() {
    fireEvent.click(await screen.findByRole("button", { name: /Find tables/ }))
    await screen.findByText("1 tables found")
    fireEvent.click(await screen.findByText(/^Columns/, { selector: "summary" }))
    // The columns are read through the preview route when the section opens (SRC-13), so they arrive after the click.
    await screen.findAllByLabelText(/^Load column /)
  }

  it("lists the columns discovery found, ticked, and saves an unticked one as excluded", async () => {
    const calls = stubFetch(saved(), undefined, { discovered: FOUND })
    render(<ConnectorIngestPanel connectorId="conn-northwind" connectorName="northwind" layout="page" />)
    await openColumns()
    expect(["order_id", "region", "updated_at", "discount"].map((c) => box(c).checked)).toEqual([true, true, true, true])

    fireEvent.click(box("discount"))
    expect(box("discount").checked).toBe(false)
    expect(screen.getByText(/^Columns · 1 left out/)).toBeDefined()
    fireEvent.click(saveButton())
    await waitFor(() => expect(calls.some((c) => c.method === "PUT")).toBe(true))
    expect((calls.find((c) => c.method === "PUT")?.body as { sourceObjects: unknown }).sourceObjects).toEqual([
      { name: "public.orders", target: "northwind_orders", loadMode: "replace", excludedColumns: ["discount"] },
    ])
  })

  it("shows a saved exclusion unticked, lets it be ticked again, and then saves none", async () => {
    const calls = stubFetch(saved({ excludedColumns: ["discount"] }), undefined, { discovered: FOUND })
    render(<ConnectorIngestPanel connectorId="conn-northwind" connectorName="northwind" layout="page" />)
    await openColumns()
    expect(box("discount").checked).toBe(false)
    fireEvent.click(box("discount"))
    fireEvent.click(saveButton())
    await waitFor(() => expect(calls.some((c) => c.method === "PUT")).toBe(true))
    expect((calls.find((c) => c.method === "PUT")?.body as { sourceObjects: unknown[] }).sourceObjects[0]).toEqual({
      name: "public.orders",
      target: "northwind_orders",
      loadMode: "replace",
    })
  })

  it("locks the key columns of a deduplicating table and the cursor column, ticked, with the reason", async () => {
    stubFetch(
      saved({ loadMode: "incremental_dedup", incrementalKey: "updated_at", primaryKey: ["order_id"] }),
      undefined,
      { discovered: FOUND }
    )
    render(<ConnectorIngestPanel connectorId="conn-northwind" connectorName="northwind" layout="page" />)
    await openColumns()
    for (const locked of ["order_id", "updated_at"]) {
      expect(box(locked).checked).toBe(true)
      expect(box(locked).disabled).toBe(true)
    }
    expect(screen.getByText("Key column: it identifies a row of this table.")).toBeDefined()
    expect(screen.getByText("Cursor column: it marks which rows are new.")).toBeDefined()
    expect(box("region").disabled).toBe(false)
  })

  it("never sends a key or cursor column as excluded, even if it was unticked before the mode needed it", async () => {
    stubFetch(
      saved({ loadMode: "replace_dedup", primaryKey: ["order_id"], excludedColumns: ["order_id", "discount"] }),
      undefined,
      { discovered: FOUND }
    )
    render(<ConnectorIngestPanel connectorId="conn-northwind" connectorName="northwind" layout="page" />)
    await openColumns()
    expect(box("order_id").checked).toBe(true)
    // The saved list held a key column; what would be saved now drops it, so the page is changed, not silently different.
    expect(screen.getByText(/^Columns · 1 left out/)).toBeDefined()
  })

  // SRC-13 lifts SRC-11's limit that only a table found in this visit lists its columns: opening the section of a
  // saved table reads them through the preview route.
  it("reads the columns of a saved table when its section is opened, without finding it first", async () => {
    const calls = stubFetch(saved({ excludedColumns: ["discount"] }), undefined, { discovered: FOUND })
    render(<ConnectorIngestPanel connectorId="conn-northwind" connectorName="northwind" layout="page" />)
    expect(calls.some((c) => c.url.endsWith("/discovery/preview"))).toBe(false)
    fireEvent.click(await screen.findByText(/^Columns/, { selector: "summary" }))
    expect((await screen.findByLabelText("Load column region of public.orders")) as HTMLInputElement).toBeDefined()
    expect(box("discount").checked).toBe(false)
    const preview = calls.filter((c) => c.url.endsWith("/discovery/preview"))
    expect(preview.map((c) => c.body)).toEqual([{ object: "public.orders" }])
  })

  it("says what failed when the columns of a saved table cannot be read, and keeps the left-out list", async () => {
    stubFetch(saved({ excludedColumns: ["discount"] }))
    render(<ConnectorIngestPanel connectorId="conn-northwind" connectorName="northwind" layout="page" />)
    fireEvent.click(await screen.findByText(/^Columns/, { selector: "summary" }))
    expect(await screen.findByText(/Left out: discount\. Cannot read the columns · The source could not be read\./)).toBeDefined()
  })

  it.each(["mysql", "mssql"])("is offered for the %s driver too", async (driver) => {
    stubFetch(saved({}, { ...SPEC, dial: { ...SPEC.dial, driver } }), undefined, { discovered: FOUND })
    render(<ConnectorIngestPanel connectorId="conn-northwind" connectorName="northwind" layout="page" />)
    await openColumns()
    expect(box("region").checked).toBe(true)
  })

  it.each([
    ["Oracle", { ...SPEC, dial: { ...SPEC.dial, driver: "oracle" } }],
    ["files", { ...SPEC, adapter: "files", dial: {} }],
    ["REST", { ...SPEC, adapter: "rest", dial: {} }],
  ])("says column choice is not available for a %s source and offers no column list", async (_label, spec) => {
    stubFetch(saved({}, spec as typeof SPEC))
    render(<ConnectorIngestPanel connectorId="conn-northwind" connectorName="northwind" layout="page" />)
    expect(await screen.findByText("Column choice is not available for this source type.")).toBeDefined()
    expect(screen.queryByText(/^Columns/)).toBeNull()
  })

  it("does not say it for a source that has no tables selected, nor offer it for a change-capture one", async () => {
    stubFetch({ ...SPEC, adapter: "cdc", ingestMode: "cdc", sourceObjects: [{ name: "public.orders", target: "northwind_orders" }] })
    render(<ConnectorIngestPanel connectorId="conn-northwind" connectorName="northwind" layout="page" />)
    await screen.findByText("public.orders")
    expect(screen.queryByText("Column choice is not available for this source type.")).toBeNull()
  })

  it("works out what a mode locks", () => {
    expect(lockedColumns({ loadMode: "replace", primaryKey: ["id"], incrementalKey: "ts" }).size).toBe(0)
    expect([...lockedColumns({ loadMode: "history", primaryKey: ["id", "region"] }).keys()]).toEqual(["id", "region"])
    expect([...lockedColumns({ loadMode: "incremental", incrementalKey: " ts " }).keys()]).toEqual(["ts"])
  })
})

/**
 * SRC-13 in the panel: the Find flow for each type, Kafka's topic choice (D4), Oracle's add by name (D5), and the
 * REST row note (D3).
 */
describe("ConnectorIngestPanel table discovery", () => {
  const KAFKA = {
    ...SPEC,
    adapter: "kafka",
    dial: {
      bootstrapServers: ["k1.internal:9092"],
      topic: "orders",
      auth: { type: "none" },
      groupId: "g",
      microBatchSeconds: 30,
    } as unknown as typeof SPEC.dial,
    sourceObjects: [{ name: "orders", target: "kafka_orders" }],
  }

  it("lists a Kafka connector's topics, and picking one saves the dial's topic with no credential", async () => {
    const calls = stubFetch(KAFKA, undefined, {
      listing: [
        { name: "orders", kind: "topic" },
        { name: "payments", kind: "topic" },
      ],
    })
    render(<ConnectorIngestPanel connectorId="conn-kafka" connectorName="events" />)
    expect((await screen.findByTestId("kafka-topic")).textContent).toBe("orders")
    fireEvent.click(screen.getByRole("button", { name: "Find topics" }))
    fireEvent.click(await screen.findByLabelText("payments"))
    await waitFor(() => expect(calls.some((c) => c.method === "PUT")).toBe(true))
    const put = calls.find((c) => c.method === "PUT")?.body as Record<string, unknown>
    expect(put.dial).toEqual({ ...KAFKA.dial, topic: "payments" })
    expect(put.sourceObjects).toEqual(KAFKA.sourceObjects)
    // SEC-14: a topic is not a target, so nothing but the topic changes and no credential travels.
    expect("credential" in put).toBe(false)
    await waitFor(() => expect(screen.getByTestId("kafka-topic").textContent).toBe("payments"))
  })

  it("lists an Oracle connector's tables and also takes one added by name", async () => {
    const calls = stubFetch(
      { ...SPEC, dial: { ...SPEC.dial, driver: "oracle", user: "hr" } },
      undefined,
      { listing: [{ name: "HR.EMPLOYEES", kind: "table" }] }
    )
    render(<ConnectorIngestPanel connectorId="conn-ora" connectorName="erp" />)
    expect((await screen.findByLabelText<HTMLInputElement>("Schema")).value).toBe("HR")
    fireEvent.click(screen.getByRole("button", { name: "Find tables" }))
    await screen.findByText("HR.EMPLOYEES")
    expect(calls.find((c) => c.url.endsWith("/discovery/list"))?.body).toEqual({ schema: "HR" })

    fireEvent.change(screen.getByLabelText("Add by name"), { target: { value: "SALES.ORDERS" } })
    fireEvent.click(screen.getByRole("button", { name: "Add" }))
    expect(screen.getByLabelText("Bronze table for SALES.ORDERS")).toBeDefined()
    fireEvent.click(screen.getByRole("button", { name: "Save tables and schedule" }))
    await waitFor(() => expect(calls.some((c) => c.method === "PUT")).toBe(true))
    expect((calls.find((c) => c.method === "PUT")?.body as { sourceObjects: unknown }).sourceObjects).toEqual([
      { name: "SALES.ORDERS", target: "erp_orders", loadMode: "replace" },
    ])
  })

  it("gives a SQL source no add-by-name field, since its list is the whole source", async () => {
    stubFetch()
    render(<ConnectorIngestPanel connectorId="conn-northwind" connectorName="northwind" />)
    await screen.findByRole("button", { name: "Find tables" })
    expect(screen.queryByLabelText("Add by name")).toBeNull()
  })

  it("loads the columns of a ticked SQL table once, to pick its key and cursor from", async () => {
    const calls = stubFetch(SPEC, undefined, {
      discovered: [{ name: "public.orders", columns: [{ name: "order_id", typeName: "integer" }], primaryKey: ["order_id"] }],
    })
    render(<ConnectorIngestPanel connectorId="conn-northwind" connectorName="northwind" />)
    fireEvent.click(await screen.findByRole("button", { name: "Find tables" }))
    fireEvent.click(await screen.findByLabelText(/public\.orders/))
    await waitFor(() => expect(calls.filter((c) => c.url.endsWith("/discovery/preview")).length).toBe(1))
    fireEvent.change(await screen.findByLabelText("Load mode for public.orders"), { target: { value: "history" } })
    await screen.findByLabelText("order_id is part of the key of public.orders")
    expect(calls.filter((c) => c.url.endsWith("/discovery/preview")).length).toBe(1)
  })

  const REST = {
    ...SPEC,
    adapter: "rest",
    dial: {
      baseUrl: "https://api.example.com",
      auth: { type: "bearer" },
      pagination: { type: "none" },
      endpoints: [{ path: "/orders" }, { path: "/users" }],
    } as unknown as typeof SPEC.dial,
  }

  it("says a REST table saved before endpoints could be picked loads every endpoint, and not one that names an endpoint", async () => {
    stubFetch({
      ...REST,
      sourceObjects: [
        { name: "orders_table", target: "api_orders_table" },
        { name: "/users", target: "api_users" },
      ],
    })
    render(<ConnectorIngestPanel connectorId="conn-api" connectorName="api" />)
    await screen.findByLabelText("Bronze table for orders_table")
    expect(screen.getAllByText(REST_OLD_OBJECT_NOTE).length).toBe(1)
    expect(within(screen.getByLabelText("Bronze table for orders_table").closest("li") as HTMLElement).getByText(REST_OLD_OBJECT_NOTE)).toBeDefined()
  })

  it("picks a REST endpoint by its path from the list", async () => {
    const calls = stubFetch(REST, undefined, {
      listing: [
        { name: "/orders", kind: "endpoint" },
        { name: "/users", kind: "endpoint" },
      ],
    })
    render(<ConnectorIngestPanel connectorId="conn-api" connectorName="api" />)
    fireEvent.click(await screen.findByRole("button", { name: "Find endpoints" }))
    fireEvent.click(await screen.findByLabelText("/users"))
    fireEvent.click(screen.getByRole("button", { name: "Save tables and schedule" }))
    await waitFor(() => expect(calls.some((c) => c.method === "PUT")).toBe(true))
    expect((calls.find((c) => c.method === "PUT")?.body as { sourceObjects: unknown }).sourceObjects).toEqual([
      { name: "/users", target: "api_users", loadMode: "replace" },
    ])
  })

  it("keeps the typed-name field and its explanation for a type that cannot list", async () => {
    stubFetch({ ...SPEC, adapter: "sheets", dial: {} as unknown as typeof SPEC.dial })
    render(<ConnectorIngestPanel connectorId="conn-sheets" connectorName="sheet" />)
    expect(await screen.findByLabelText("Add by name")).toBeDefined()
    expect(screen.getByText("This connector type cannot list its objects, so type each one as the source names it.")).toBeDefined()
    expect(screen.queryByRole("button", { name: /^Find / })).toBeNull()
  })
})
