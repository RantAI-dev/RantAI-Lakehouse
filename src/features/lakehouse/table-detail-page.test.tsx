// OPS-21: the table page must warn only from a MEASURED small-file count and
// a confirmed-off Trino. The page is rendered whole with `fetch` stubbed, so
// what is under test is what a reader sees for each API answer: an honest,
// actionable warning naming the `trino` profile, and silence when the count
// is unmeasured, below the need count, or Trino can compact.
mock.module("next/navigation", () => ({
  usePathname: () => "/lakehouse/tables/bronze/orders",
  useParams: () => ({ namespace: "bronze", table: "orders" }),
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

import { cleanup, render, screen } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, mock, spyOn } from "bun:test"
import { LakehouseTableDetailPage } from "./table-detail-page"

const originalFetch = global.fetch
const DETAIL_URL = "/api/lakehouse/tables/bronze/orders"
const MAINTENANCE_URL = "/api/lakehouse/tables/bronze/orders/maintenance"

const json = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } })

const MAINTENANCE = {
  namespace: "bronze",
  tableName: "orders",
  configured: false,
  snapshotsToKeep: null,
  orphanAgeHours: null,
  compactSmallFiles: false,
  schedule: null,
  lastRun: null,
  lastVerbRuns: [],
}

function detail(smallFileCount: number | null, compactionAvailable: boolean) {
  return {
    schema: [],
    partitionSpec: [],
    properties: {},
    snapshots: [],
    schemaVersions: [],
    stats: {
      fileCount: 30,
      smallFileCount,
      smallFileThresholdBytes: 32 * 1024 * 1024,
      recordCount: 1000,
      totalBytes: 1024,
      snapshotCount: 1,
      metadataLogCount: 1,
    },
    compactionAvailable,
  }
}

/** Per-URL answers; a test overrides the detail answer. */
let answers: Record<string, () => Response> = {}

beforeEach(() => {
  answers = {
    [DETAIL_URL]: () => json(detail(null, false)),
    [MAINTENANCE_URL]: () => json(MAINTENANCE),
  }
  spyOn(globalThis, "fetch").mockImplementation((async (input: RequestInfo | URL) => {
    const url = String(input)
    const answer = answers[url] ?? answers[url.split("?")[0]]
    return answer ? answer() : json({ error: "not stubbed" }, 404)
  }) as unknown as typeof fetch)
})

afterEach(() => {
  // `mock.restore()` would unregister the `next/navigation` module mock.
  global.fetch = originalFetch
  cleanup()
})

/** Wait for the detail to render, then assert no warning banner appeared. */
async function renderAndExpectSilence() {
  render(<LakehouseTableDetailPage />)
  await screen.findByText("Files")
  expect(screen.queryByRole("alert")).toBeNull()
}

describe("the Lakehouse table detail page's compaction warning", () => {
  it("warns with the measured count and names the trino profile when Trino is off", async () => {
    answers[DETAIL_URL] = () => json(detail(42, false))
    render(<LakehouseTableDetailPage />)

    const alert = await screen.findByRole("alert")
    expect(alert.textContent).toContain("42")
    expect(alert.textContent).toContain("32.0 MB")
    expect(alert.textContent).toContain("--profile trino")
    expect(alert.textContent).toContain("trino profile")
    // The threshold is displayed cleanly on the stat itself.
    expect(screen.getByText("Small files (< 32.0 MB)")).toBeTruthy()
  })

  it("is silent when the small-file count was not measured", async () => {
    answers[DETAIL_URL] = () => json(detail(null, false))
    await renderAndExpectSilence()
  })

  it("is silent when the measured count is below the need count", async () => {
    answers[DETAIL_URL] = () => json(detail(19, false))
    await renderAndExpectSilence()
  })

  it("is silent when Trino can compact, however high the measured count", async () => {
    answers[DETAIL_URL] = () => json(detail(500, true))
    await renderAndExpectSilence()
  })
})
