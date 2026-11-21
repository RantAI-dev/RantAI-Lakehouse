// BLD-8: a failed run's real state and its reason, on the pipeline page.
//
// The page is rendered whole, with `fetch` stubbed: the state it shows must
// come from the runs route the API answers, and the reason it shows must be
// the classified one (this product's own fixed descriptions) or the fixed
// "see the run log" sentence -- never a driver's or a traceback's text.
//
// The first test is the regression for the state that started the backlog
// item: a run that ended stayed "Running" on the page because the run-list
// poll timer was re-armed on every one-second clock tick, so it never fired
// again. It uses real timers on purpose (one poll interval of waiting), so
// it exercises the same `setTimeout` the page relies on.
/** The query string `useSearchParams` answers with; each test sets it. */
let search = "tab=runs"
mock.module("next/navigation", () => ({
  usePathname: () => "/pipelines/pl-connector-1",
  useParams: () => ({ pipelineId: "pl-connector-1" }),
  // An authored pipeline opens on its Definition tab; the run state and
  // the failure reason under test live on the Runs tab.
  useSearchParams: () => new URLSearchParams(search),
  useRouter: () => ({
    push: () => {},
    replace: () => {},
    refresh: () => {},
    back: () => {},
    forward: () => {},
    prefetch: () => {},
  }),
}))

import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react"
import { afterEach, describe, expect, it, jest, mock, spyOn } from "bun:test"
import { AuthProvider } from "@/features/auth/auth-provider"
import type { PipelineDetail, PipelineRun } from "@/services/contracts/pipelines"
import { PipelineDetailPage } from "./pipeline-detail-page"

const originalFetch = global.fetch

afterEach(() => {
  // `mock.restore()` is deliberately NOT called: it would unregister the
  // `next/navigation` module mock above between tests. The fetch stub IS
  // put back, though: `spyOn(globalThis, "fetch")` stays installed
  // otherwise, and `bun test` runs every file in one process, so the next
  // file's own `spyOn` would see this file's calls (`PR #247 CI`:
  // `column-lineage.test.tsx` counted 26 fetch calls it never made).
  global.fetch = originalFetch
  search = "tab=runs"
  cleanup()
})

function json(body: unknown, status = 200) {
  return new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } })
}

/** An authored pipeline with one connector, as `GET /api/pipelines/{id}` sends it. */
const PIPELINE: PipelineDetail = {
  id: "pl-connector-1",
  name: "northwind_orders_clean",
  kind: "batch",
  status: "ready",
  owner: "system",
  source: "bronze.northwind_orders",
  target: "silver.northwind_orders",
  connectorId: "conn-pg-lakehouse",
  schedule: "manual",
  lastRunAt: "2026-10-23T04:00:00.000Z",
  slaOk: null,
  freshnessLagSeconds: { value: null, measured: false },
  engine: "authored",
  description: null,
  graph: null,
  config: [],
  definition: {
    // BLD-1 part 1 / D3: the detail read returns the definition as inputs
    // and steps, and in the form of before.
    inputs: [{ name: "northwind_orders", zone: "bronze", table: "northwind_orders" }],
    steps: [],
    sourceZone: "bronze",
    sourceTable: "northwind_orders",
    incrementalColumn: null,
    transforms: [],
    fbicEnabled: false,
    targetZone: "silver",
    targetTable: "northwind_orders",
    connectorId: "conn-pg-lakehouse",
  },
  orchestratorJob: "authored__pl_connector_1",
  dependsOn: ["ingest_job"],
  targetLag: null,
  runAsName: null,
  upstream: [],
  downstream: [],
  lag: undefined,
} as unknown as PipelineDetail

function run(status: PipelineRun["status"]): PipelineRun {
  return {
    id: "run-0001",
    pipelineId: PIPELINE.id,
    status,
    startedAt: "2026-10-23T04:00:00.000Z",
    endedAt: status === "running" ? undefined : "2026-10-23T04:00:05.000Z",
    processed: null,
    accepted: null,
    rejected: null,
    retried: null,
    costUnits: null,
    durationSeconds: status === "running" ? null : 5,
    queuedAt: "2026-10-23T03:59:58.000Z",
    parentRunId: null,
    rootRunId: null,
    trigger: { kind: "manual", name: null },
  } as PipelineRun
}

type Stub = {
  /** The run status the NEXT runs read answers; the test flips it. */
  status: () => PipelineRun["status"]
  /** The run's error lines, as `GET .../runs/{runId}/logs` answers them. */
  logLines?: { ts: number; level: string; stepKey: string | null; message: string }[]
  /** The pipeline the detail read answers; the test swaps its definition. */
  pipeline?: PipelineDetail
  /** Successive detail reads (the last repeats) — the job-load poll test. */
  details?: PipelineDetail[]
  /** A refused Run now: the 409 error text the trigger POST answers. */
  triggerError?: string
  /** The detail read after a status POST (Mark ready). */
  readyPipeline?: PipelineDetail
}

function stubApi(stub: Stub) {
  let detailIndex = 0
  let current = stub.pipeline ?? PIPELINE
  return spyOn(globalThis, "fetch").mockImplementation((async (input: RequestInfo | URL, init?: RequestInit) => {
    const path = String(input)
    const method = init?.method ?? "GET"
    if (path.includes("/api/auth/me")) {
      return json({ id: "u1", name: "Operator", email: null, roles: [], permissions: ["pipeline:read"], tenants: [] })
    }
    if (path.endsWith("/api/pipelines/pl-connector-1/status") && method === "POST") {
      current = stub.readyPipeline ?? { ...current, status: "ready" }
      return json(current)
    }
    if (path.endsWith("/api/pipelines/pl-connector-1")) {
      if (stub.details?.length) {
        const next = stub.details[Math.min(detailIndex, stub.details.length - 1)]
        detailIndex += 1
        return json(next)
      }
      return json(current)
    }
    if (path.endsWith("/api/pipelines/pl-connector-1/trigger") && method === "POST") {
      if (stub.triggerError) return json({ error: stub.triggerError }, 409)
      return json(run("running"))
    }
    if (path.endsWith("/api/pipelines/pl-connector-1/runs")) return json({ runs: [run(stub.status())], unavailable: null })
    if (path.includes("/runs/run-0001/logs")) {
      return json({ lines: stub.logLines ?? [], cursor: "0", truncated: false })
    }
    // `getRunSteps` unwraps the envelope (`{ steps: [...] }`).
    if (path.includes("/runs/run-0001/steps")) return json({ steps: [] })
    return json({ error: "not stubbed" }, 404)
  }) as unknown as typeof fetch)
}

function renderPage() {
  return render(
    <AuthProvider>
      <PipelineDetailPage />
    </AuthProvider>
  )
}

/** The masthead's own status badge — the legend below also says "Running". */
function banner() {
  return within(screen.getByRole("banner"))
}

const OUR_FAILURE =
  'Failure(description="authored pipeline \'pl-connector-1\' config rejected: this pipeline names a connector but its source zone is not bronze: a pipeline made from a connector reads one of that connector\'s Bronze tables. Edit the pipeline: choose bronze and one of the connector\'s tables, or clear its connector.")'

describe("the pipeline page's run state (BLD-8)", () => {
  it("shows a run that ended as failed without waiting for a manual reload", async () => {
    let status: PipelineRun["status"] = "running"
    stubApi({ status: () => status })
    renderPage()

    await waitFor(() => expect(banner().queryByText("Running")).not.toBeNull())

    // The orchestrator records the failure; the page must pick it up on its
    // own poll. Before the fix the poll timer was re-armed on every clock
    // tick and never fired while a run was live, so this stayed "Running".
    status = "failed"
    await waitFor(() => expect(banner().queryByText("Failed")).not.toBeNull(), { timeout: 10_000 })
    expect(banner().queryByText("Running")).toBeNull()
  }, 20_000)

  it("shows this product's own fixed reason for the failed run", async () => {
    stubApi({
      status: () => "failed",
      logLines: [{ ts: 1, level: "ERROR", stepKey: "authored_pl_connector_1", message: OUR_FAILURE }],
    })
    renderPage()

    // A longer wait than the 1 s default: this machine runs several
    // builds at once, and the log read is a second round trip. The
    // callout renders while that read is still in flight ("Reading the
    // log for the error…"), so the reason itself is awaited rather than
    // read from the first paint.
    const callout = await screen.findByRole("note", undefined, { timeout: 8000 })
    await waitFor(
      () =>
        expect(callout.textContent).toContain(
          "config rejected: this pipeline names a connector but its source zone is not bronze"
        ),
      { timeout: 8000 }
    )
    // The wrapper's own text (the `Failure(description=…)` shape) is not
    // what the reader gets.
    expect(callout.textContent).not.toContain("Failure(description=")
  }, 20_000)

  it("withholds upstream error text and points at the run log instead", async () => {
    stubApi({
      status: () => "failed",
      logLines: [
        { ts: 1, level: "ERROR", stepKey: "authored_pl_connector_1", message: "clickhouse: Code: 60. Table bronze.northwind_orders does not exist" },
        { ts: 2, level: "ERROR", stepKey: "authored_pl_connector_1", message: 'Execution of step "authored_pl_connector_1" failed.' },
      ],
    })
    renderPage()

    // A longer wait than the 1 s default: this machine runs several
    // builds at once, and the log read is a second round trip. The
    // callout renders before the log answers, so the fixed sentence is
    // awaited rather than read from the first paint.
    const callout = await screen.findByRole("note", undefined, { timeout: 8000 })
    await waitFor(
      () => expect(callout.textContent).toContain("The run failed; see the run log."),
      { timeout: 8000 }
    )
    // The log panel still shows the engine's words (that is what a log is);
    // the reason above it does not repeat them.
    expect(callout.textContent).not.toContain("Code: 60")
    expect(screen.getByRole("button", { name: /show errors in the log/i })).toBeTruthy()
  }, 20_000)
})

// BLD-11 F2/D3: the target lag is the trigger when one is set, whatever the
// stored `schedule` says. A stored cron with a target lag never fires, and
// the page must say neither "Manual only" nor offer a Pause for it.
describe("the pipeline page's trigger (BLD-11)", () => {
  it("a pipeline with a target lag and no schedule never says 'Manual only'", async () => {
    const pipeline: PipelineDetail = {
      ...PIPELINE,
      schedule: "manual",
      targetLag: { kind: "lag", minutes: 10 },
    }
    stubApi({ status: () => "completed", pipeline })
    renderPage()

    expect(await screen.findByText("Within 10 min of its inputs")).toBeTruthy()
    expect(screen.queryByText("Manual only")).toBeNull()
    expect(screen.getByText("Runs when an input changes")).toBeTruthy()
  })

  it("a pipeline with a target lag and a stored cron offers no Pause schedule button", async () => {
    const pipeline: PipelineDetail = {
      ...PIPELINE,
      schedule: "cron: 0 * * * * (RUNNING)",
      targetLag: { kind: "lag", minutes: 120 },
    }
    stubApi({ status: () => "completed", pipeline })
    renderPage()

    expect(await screen.findByText("Within 2 h of its inputs")).toBeTruthy()
    expect(screen.queryByRole("button", { name: /pause schedule/i })).toBeNull()
  })

  // BLD-11 review R1: D3 must hide Pause for a target-lag pipeline, but a
  // pipeline already paused still needs its Resume to get running again.
  it("a paused target-lag pipeline still offers Resume", async () => {
    const pipeline: PipelineDetail = {
      ...PIPELINE,
      schedule: "cron: 0 * * * * (PAUSED)",
      status: "paused",
      targetLag: { kind: "lag", minutes: 10 },
    }
    stubApi({ status: () => "completed", pipeline })
    renderPage()

    expect(await screen.findByRole("button", { name: /resume schedule/i })).toBeTruthy()
    expect(screen.queryByRole("button", { name: /pause schedule/i })).toBeNull()
  })

  it("a refused Run now shows the API's reason under the masthead", async () => {
    stubApi({
      status: () => "completed",
      triggerError: "the orchestrator has not loaded this pipeline's job yet; try again in a minute.",
    })
    renderPage()

    fireEvent.click(await screen.findByRole("button", { name: /run now/i }))
    const alert = await screen.findByRole("alert", undefined, { timeout: 8000 })
    expect(alert.textContent).toContain("try again in a minute")
  })

  it("a ready pipeline whose job is loading reloads its detail until the job is there", async () => {
    // The poll is on a 4 s timer: drive it with fake timers so the second
    // detail read is deterministic, then let React flush inside act().
    jest.useFakeTimers()
    try {
      const loading: PipelineDetail = { ...PIPELINE, orchestratorJob: null }
      const loaded: PipelineDetail = { ...PIPELINE, orchestratorJob: "authored__pl_connector_1" }
      // The harness double-mounts (StrictMode), so the first two reads are
      // still loading; only the bounded poll's read sees the job loaded.
      stubApi({ status: () => "completed", details: [loading, loading, loaded] })
      renderPage()

      await act(async () => {})
      expect(screen.getByText("The orchestrator is loading this pipeline's job.")).toBeTruthy()
      // Two separate act() flushes: one poll look per 4 s tick.
      await act(async () => {
        jest.advanceTimersByTime(4500)
      })
      await act(async () => {
        jest.advanceTimersByTime(4500)
      })
      // The detail poll saw the job load; no manual reload was needed.
      expect(screen.queryByText("The orchestrator is loading this pipeline's job.")).toBeNull()
    } finally {
      jest.useRealTimers()
    }
  }, 20_000)

  it("after Mark ready the page shows the pipeline as ready without a reload", async () => {
    const draft: PipelineDetail = { ...PIPELINE, status: "draft", orchestratorJob: null }
    const ready: PipelineDetail = { ...draft, status: "ready" }
    stubApi({ status: () => "completed", pipeline: draft, readyPipeline: ready })
    renderPage()

    fireEvent.click(await screen.findByRole("button", { name: /mark ready/i }))
    // The button's label flips to "Marking ready…" while pending, so wait for
    // the post-reload banner rather than for the label to disappear.
    expect(
      await screen.findByText("The orchestrator is loading this pipeline's job.", undefined, {
        timeout: 8000,
      })
    ).toBeTruthy()
    expect(screen.queryByRole("button", { name: /mark ready/i })).toBeNull()
  })
})

// BLD-10 D3: the definition page says the same as the form -- FBIC is not
// available, and a pipeline with no transform copies its source as is.
describe("the pipeline page's Definition tab (BLD-10)", () => {
  /** The stored pipeline with FBIC on or off, as the detail read answers it. */
  function withFbic(fbicEnabled: boolean): PipelineDetail {
    return { ...PIPELINE, definition: { ...PIPELINE.definition!, fbicEnabled } }
  }

  it("says FBIC is not available, and that a pipeline with no transform is a plain copy", async () => {
    search = "tab=definition"
    stubApi({ status: () => "completed", pipeline: withFbic(false) })
    renderPage()

    // The form's own name for it, not a third one.
    expect(await screen.findByText("FBIC enrichment")).toBeTruthy()
    expect(screen.getByText("Not available")).toBeTruthy()
    // The fixture has no transforms, so the target is a plain copy and the
    // chart says so.
    expect(screen.getByText("plain copy of the source")).toBeTruthy()
  })

  it("says a stored-on FBIC is not available either, and what is stored", async () => {
    search = "tab=definition"
    stubApi({ status: () => "completed", pipeline: withFbic(true) })
    renderPage()

    expect(await screen.findByText("Not available (stored as on; nothing runs it)")).toBeTruthy()
  })
})

// BLD-1 part 1 D17: the read view draws inputs, steps and the output, and a
// SQL step shows its SQL.
describe("the pipeline page's flow of inputs and steps (BLD-1 part 1)", () => {
  const SQL = "SELECT c.country, count() AS orders FROM paid AS p JOIN customers AS c ON p.customer_id = c.id GROUP BY c.country"
  const withSteps: PipelineDetail = {
    ...PIPELINE,
    definition: {
      ...PIPELINE.definition!,
      inputs: [
        { name: "orders", zone: "bronze", table: "northwind_orders" },
        { name: "customers", zone: "bronze", table: "northwind_customers" },
      ],
      steps: [
        { name: "paid", kind: "transform", reads: "orders", transforms: ["filter(status = 'paid')"] },
        { name: "by_country", kind: "sql", sql: SQL },
      ],
    },
  }

  it("draws each input and step, and shows a SQL step's SQL when it is selected", async () => {
    search = "tab=definition"
    stubApi({ status: () => "completed", pipeline: withSteps })
    renderPage()

    const chart = await screen.findByRole("group", { name: "Pipeline flowchart" })
    expect(within(chart).getByText("bronze.northwind_orders")).toBeTruthy()
    expect(within(chart).getByText("bronze.northwind_customers")).toBeTruthy()
    expect(within(chart).getByText("paid")).toBeTruthy()
    expect(within(chart).getByText("silver.northwind_orders")).toBeTruthy()

    within(chart).getByText("by_country").closest("button")?.click()
    expect(await screen.findByText("Step 2 of 2 · SQL")).toBeTruthy()
    // `CodeView` shows the step's text as stored.
    expect(document.body.textContent).toContain("GROUP BY c.country")
  })

  // BLD-1 part 1 S3 review R1-S2: a kind the console does not know does not
  // crash the page; it is shown as stored.
  it("shows a step of a kind this console does not know with its kind and stored record", async () => {
    search = "tab=definition"
    const unknown: PipelineDetail = {
      ...withSteps,
      definition: { ...withSteps.definition!, steps: [{ name: "joined", kind: "join", left: "orders" }] },
    }
    stubApi({ status: () => "completed", pipeline: unknown })
    renderPage()

    const chart = await screen.findByRole("group", { name: "Pipeline flowchart" })
    within(chart).getByText("joined").closest("button")?.click()
    expect(await screen.findByText("Step 1 of 1 · join")).toBeTruthy()
    expect(document.body.textContent).toContain('"left": "orders"')
  })

  it("lists a form step's operations and what it reads", async () => {
    search = "tab=definition"
    stubApi({ status: () => "completed", pipeline: withSteps })
    renderPage()

    const chart = await screen.findByRole("group", { name: "Pipeline flowchart" })
    within(chart).getByText("paid").closest("button")?.click()
    // BLD-16 / J9: the read view says the editor's word for the kind.
    expect(await screen.findByText("Step 1 of 2 · Operations")).toBeTruthy()
    expect(screen.getByText("filter(status = 'paid')")).toBeTruthy()
  })
})

// WEB-3 S3-D9: a pipeline that names its target table opens that table's
// Lineage tab; one that names none keeps the Lineage page.
describe("the pipeline page's lineage link (WEB-3 S3-D9)", () => {
  it("opens the target table's Lineage tab when the pipeline names one", async () => {
    search = "tab=settings"
    stubApi({ status: () => "completed", pipeline: { ...PIPELINE, targetAssetId: "silver.northwind_orders" } })
    renderPage()
    const link = await screen.findByRole("link", { name: "open its target table's Lineage tab" })
    expect(link.getAttribute("href")).toBe("/data/assets/silver.northwind_orders?tab=lineage")
  })

  it("keeps the Lineage page for a pipeline with no target table", async () => {
    search = "tab=settings"
    stubApi({ status: () => "completed" })
    renderPage()
    const link = await screen.findByRole("link", { name: "open in Lineage" })
    expect(link.getAttribute("href")).toBe("/lineage?focus=pl-connector-1")
  })
})

// ── BLD-16 (BLD-1 part 2) / J9: the new kinds in words ───────────────────

describe("the pipeline page's form steps in words (BLD-16)", () => {
  const SQL_BASE = {
    sourceZone: "bronze",
    sourceTable: "orders_raw",
    incrementalColumn: null,
    transforms: [],
    fbicEnabled: false,
    targetZone: "silver",
    targetTable: "sales_by_country",
    connectorId: null,
  }

  /** The plan's J2 example: the three new kinds, one after the other. */
  const formSteps: PipelineDetail = {
    ...PIPELINE,
    definition: {
      ...SQL_BASE,
      inputs: [
        { name: "orders", zone: "bronze", table: "orders_raw" },
        { name: "customers", zone: "bronze", table: "customers" },
      ],
      steps: [
        {
          name: "joined",
          kind: "join",
          left: "orders",
          right: "customers",
          joinType: "left",
          on: [{ left: "customer_id", right: "id" }],
          columns: [
            { side: "left", column: "order_id" },
            { side: "left", column: "amount" },
            { side: "right", column: "country" },
            { side: "right", column: "id", name: "customer_ref" },
          ],
        },
        {
          name: "by_country",
          kind: "aggregate",
          reads: "joined",
          groupBy: ["country"],
          measures: [
            { function: "sum", column: "amount", name: "revenue" },
            { function: "count", name: "order_count" },
          ],
          havingMatch: "all",
          having: [{ column: "revenue", operator: ">", value: "1000" }],
        },
        {
          name: "big_or_local",
          kind: "filter",
          reads: "by_country",
          match: "any",
          conditions: [
            { column: "revenue", operator: ">=", value: "5000" },
            { column: "country", operator: "=", value: "ID" },
          ],
        },
      ],
    },
  }

  it("shows each of the three kinds in words, with what it computes", async () => {
    search = "tab=definition"
    stubApi({ status: () => "completed", pipeline: formSteps })
    renderPage()

    const chart = await screen.findByRole("group", { name: "Pipeline flowchart" })
    within(chart).getByText("joined").closest("button")?.click()
    expect(await screen.findByText("Step 1 of 3 · Join")).toBeTruthy()
    // J9's own words: "left join of orders and customers on customer_id = id".
    // (The paragraph marks the names in spans, so the sentence is read from
    // the page's text, not from one element.)
    await waitFor(() => expect(document.body.textContent).toContain("left join of orders and customers on"))
    expect(document.body.textContent).toContain("customer_id = id")
    expect(document.body.textContent).toContain("id as customer_ref")

    within(chart).getByText("by_country").closest("button")?.click()
    expect(await screen.findByText("Step 2 of 3 · Aggregate")).toBeTruthy()
    await waitFor(() => expect(document.body.textContent).toContain("Group by country"))
    expect(document.body.textContent).toContain("revenue = sum of amount")
    expect(document.body.textContent).toContain("order_count = count")
    // BLD-16 review R2: a having drops groups, so the words say it does.
    expect(document.body.textContent).toContain("Keep groups where revenue > 1000")

    within(chart).getByText("big_or_local").closest("button")?.click()
    expect(await screen.findByText("Step 3 of 3 · Filter")).toBeTruthy()
    await waitFor(() => expect(document.body.textContent).toContain("any of 2 conditions"))
    expect(document.body.textContent).toContain("revenue >= 5000")
    expect(document.body.textContent).toContain("country = ID")
  })

  it("still shows a stored record for a kind it cannot show — a join whose fields do not parse", async () => {
    search = "tab=definition"
    const broken: PipelineDetail = {
      ...PIPELINE,
      definition: {
        ...SQL_BASE,
        inputs: [{ name: "orders", zone: "bronze", table: "orders_raw" }],
        steps: [{ name: "joined", kind: "join", left: "orders" }],
      },
    }
    stubApi({ status: () => "completed", pipeline: broken })
    renderPage()

    const chart = await screen.findByRole("group", { name: "Pipeline flowchart" })
    within(chart).getByText("joined").closest("button")?.click()
    expect(await screen.findByText("Step 1 of 1 · join")).toBeTruthy()
    expect(document.body.textContent).toContain('"left": "orders"')
  })
})
