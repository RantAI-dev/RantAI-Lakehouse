// The pipeline detail page's header actions follow where a pipeline came
// from: an authored pipeline (a `pl-` id) runs one run at a time and pauses
// by status; a Dagster job pauses its schedule, which only
// `schedulePaused` reports. The Runs tab says so when the orchestrator
// could not be asked instead of reading "No runs".

// `useParams` names the pipeline; `useSearchParams` feeds the data table's
// URL state. `mock.module` is hoisted above the imports by bun's runner.
const params = { pipelineId: "" }
mock.module("next/navigation", () => ({
  useParams: () => params,
  usePathname: () => `/pipelines/${params.pipelineId}`,
  useSearchParams: () => new URLSearchParams(),
  useRouter: () => ({
    push: () => {},
    replace: () => {},
    refresh: () => {},
    back: () => {},
    forward: () => {},
    prefetch: () => {},
  }),
}))

import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"
import { TableProviders } from "@/components/app-shell/table-providers"
import { PipelineDetailPage } from "./pipeline-detail-page"

const originalFetch = global.fetch

afterEach(() => {
  global.fetch = originalFetch
  cleanup()
})

const AUTHORED = {
  id: "pl-orders-clean-abc",
  name: "orders_clean",
  kind: "batch",
  status: "ready",
  owner: "Current user",
  source: "bronze.orders",
  target: "silver.orders_clean",
  schedule: "manual",
  lastRunAt: null,
  slaOk: null,
  freshnessLagSeconds: null,
  description: "Clean orders for the daily report",
  engine: "authored",
  graph: null,
  config: [],
  definition: null,
}

const DAGSTER = {
  id: "gold_export_job",
  name: "gold_export_job",
  kind: "batch",
  status: "completed",
  owner: "Tenant",
  source: null,
  target: null,
  schedule: "cron: 0 4 * * * (STOPPED)",
  lastRunAt: null,
  nextRunAt: null,
  schedulePaused: true,
  slaOk: null,
  freshnessLagSeconds: null,
  description: null,
  engine: "dagster",
  graph: { ops: [], edges: [] },
  config: [],
  definition: null,
}

const RUN = {
  id: "run-new",
  status: "running",
  startedAt: "2026-09-30T06:00:00.000Z",
  processed: null,
  accepted: null,
  rejected: null,
  retried: null,
  costUnits: null,
}

function json(body: unknown, status = 200) {
  return new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } })
}

type Call = { url: string; method: string }

/**
 * Serve `detail` and a runs list that changes once a run is triggered;
 * record every call.
 */
function stubFetch(detail: object, { runsUnavailable = false } = {}) {
  const calls: Call[] = []
  let triggered = false
  global.fetch = mock(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = typeof input === "string" ? input : input.toString()
    const method = init?.method ?? "GET"
    calls.push({ url, method })
    const path = url.replace(/^https?:\/\/[^/]+/, "")
    const id = (detail as { id: string }).id
    if (method === "POST" && path === `/api/pipelines/${id}/trigger`) {
      triggered = true
      return json({ ...RUN, pipelineId: id })
    }
    if (method === "POST" && path === `/api/pipelines/${id}/status`) {
      return json({ ...detail, status: "ready" })
    }
    if (path === `/api/pipelines/${id}/runs`) {
      if (runsUnavailable) return json({ runs: [], unavailable: "connection refused" })
      return json({ runs: triggered ? [{ ...RUN, pipelineId: id }] : [], unavailable: null })
    }
    if (path.startsWith(`/api/pipelines/${id}/runs/`)) return json({ steps: [] })
    if (path === `/api/pipelines/${id}`) return json(detail)
    return json({ error: `unexpected ${method} ${path}` }, 404)
  }) as unknown as typeof fetch
  return calls
}

function renderPage(id: string) {
  params.pipelineId = id
  return render(
    <TableProviders>
      <PipelineDetailPage />
    </TableProviders>
  )
}

describe("PipelineDetailPage", () => {
  it("runs an active authored pipeline and shows the new run on the Runs tab", async () => {
    const calls = stubFetch(AUTHORED)
    renderPage(AUTHORED.id)

    expect(await screen.findByText("Clean orders for the daily report")).toBeTruthy()
    expect(screen.getByRole("button", { name: /Pause/ })).toBeTruthy()
    fireEvent.click(screen.getByRole("button", { name: /Run now/ }))

    await waitFor(() => {
      expect(calls.some((c) => c.method === "POST" && c.url.endsWith(`/api/pipelines/${AUTHORED.id}/trigger`))).toBe(
        true
      )
    })
    // The run is going, so a second one is not offered until it ends.
    expect(await screen.findByRole("button", { name: /Running/ })).toBeTruthy()
    expect(screen.getByRole("tab", { name: "Runs" }).getAttribute("aria-selected")).toBe("true")
  })

  it("activates a draft", async () => {
    const calls = stubFetch({ ...AUTHORED, status: "draft" })
    renderPage(AUTHORED.id)

    fireEvent.click(await screen.findByRole("button", { name: /Activate/ }))
    await waitFor(() => {
      expect(calls.some((c) => c.method === "POST" && c.url.endsWith(`/api/pipelines/${AUTHORED.id}/status`))).toBe(
        true
      )
    })
  })

  it("does not run a paused authored pipeline", async () => {
    stubFetch({ ...AUTHORED, status: "paused" })
    renderPage(AUTHORED.id)

    expect(await screen.findByRole("button", { name: /Resume/ })).toBeTruthy()
    expect((screen.getByRole("button", { name: /Run now/ }) as HTMLButtonElement).disabled).toBe(true)
  })

  it("offers Resume for a Dagster job whose schedule is stopped, and still runs it", async () => {
    stubFetch(DAGSTER)
    renderPage(DAGSTER.id)

    expect(await screen.findByRole("button", { name: /Resume/ })).toBeTruthy()
    expect(screen.queryByRole("button", { name: /Pause/ })).toBeNull()
    expect((screen.getByRole("button", { name: /Run now/ }) as HTMLButtonElement).disabled).toBe(false)
  })

  it("offers no pause for a Dagster job without a schedule", async () => {
    stubFetch({ ...DAGSTER, schedule: "manual", schedulePaused: null })
    renderPage(DAGSTER.id)

    expect(await screen.findByRole("button", { name: /Run now/ })).toBeTruthy()
    expect(screen.queryByRole("button", { name: /Pause/ })).toBeNull()
    expect(screen.queryByRole("button", { name: /Resume/ })).toBeNull()
  })

  it("says the orchestrator could not be reached instead of showing no runs", async () => {
    stubFetch(AUTHORED, { runsUnavailable: true })
    renderPage(AUTHORED.id)

    fireEvent.click(await screen.findByRole("tab", { name: "Runs" }))
    expect(await screen.findByText(/could not be reached/)).toBeTruthy()
    expect(screen.queryByText("No runs")).toBeNull()
  })
})
