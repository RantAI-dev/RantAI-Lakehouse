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
mock.module("next/navigation", () => ({
  usePathname: () => "/pipelines/pl-connector-1",
  useParams: () => ({ pipelineId: "pl-connector-1" }),
  // An authored pipeline opens on its Definition tab; the run state and
  // the failure reason under test live on the Runs tab.
  useSearchParams: () => new URLSearchParams("tab=runs"),
  useRouter: () => ({
    push: () => {},
    replace: () => {},
    refresh: () => {},
    back: () => {},
    forward: () => {},
    prefetch: () => {},
  }),
}))

import { cleanup, render, screen, waitFor, within } from "@testing-library/react"
import { afterEach, describe, expect, it, mock, spyOn } from "bun:test"
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
}

function stubApi(stub: Stub) {
  return spyOn(globalThis, "fetch").mockImplementation((async (input: RequestInfo | URL) => {
    const path = String(input)
    if (path.includes("/api/auth/me")) {
      return json({ id: "u1", name: "Operator", email: null, roles: [], permissions: ["pipeline:read"], tenants: [] })
    }
    if (path.endsWith("/api/pipelines/pl-connector-1")) return json(PIPELINE)
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
