// BLD-14 D8: the one sentence the pipeline page shows from the newest
// re-upload chain step. Each code has its own sentence; a widened code must
// never crash the page, and a null `last` renders nothing.

import { cleanup, render, screen, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, mock, spyOn } from "bun:test"

import { formatDateTime } from "@/lib/format"
import { pipelineService } from "@/services"
import type { ReuploadChainStep } from "@/services/contracts/pipelines"
import { ReuploadChainNote } from "./reupload-chain-note"

afterEach(() => {
  mock.restore()
  cleanup()
})

const WHEN = formatDateTime("2026-11-20T10:02:11Z")
const BASE: Omit<ReuploadChainStep, "outcome"> = {
  rootRunId: "root-1",
  sourceTable: "bronze.orders",
  reason: null,
  runId: null,
  causePipelineId: null,
  causePipelineName: null,
  recordedAt: "2026-11-20T10:02:11Z",
}

async function note(last: ReuploadChainStep | null) {
  const fetcher = mock(() => Promise.resolve({ last }))
  spyOn(pipelineService, "getReuploadChain").mockImplementation(fetcher)
  const view = render(<ReuploadChainNote pipelineId="pl-gold" />)
  await waitFor(() => expect(fetcher).toHaveBeenCalled())
  return view.container
}

describe("ReuploadChainNote", () => {
  it("renders the started sentence with a link that opens the run", async () => {
    const container = await note({ ...BASE, outcome: "started", runId: "run-42" })

    expect(container.textContent).toBe(`Ran after the re-upload of bronze.orders on ${WHEN}. run-42`)
    const link = screen.getByRole("link", { name: "run-42" }) as HTMLAnchorElement
    expect(link.getAttribute("href")).toBe("/pipelines/pl-gold?tab=runs&run=run-42")
  })

  it("renders the failed-parent sentence with the cause's name", async () => {
    const container = await note({
      ...BASE,
      outcome: "not_run",
      reason: "upstream_failed",
      causePipelineId: "pl-silver",
      causePipelineName: "Silver orders",
    })

    expect(container.textContent).toBe(
      `Not run after the re-upload of bronze.orders on ${WHEN}: Silver orders failed first.`
    )
    const link = screen.getByRole("link", { name: "Silver orders" }) as HTMLAnchorElement
    expect(link.getAttribute("href")).toBe("/pipelines/pl-silver")
  })

  it("renders the cancelled-parent sentence", async () => {
    const container = await note({
      ...BASE,
      outcome: "not_run",
      reason: "upstream_cancelled",
      causePipelineId: "pl-silver",
      causePipelineName: "Silver orders",
    })

    expect(container.textContent).toBe(
      `Not run after the re-upload of bronze.orders on ${WHEN}: Silver orders was cancelled.`
    )
  })

  it("renders the six-hour sentence", async () => {
    const container = await note({
      ...BASE,
      outcome: "not_run",
      reason: "wait_expired",
      causePipelineId: "pl-silver",
      causePipelineName: "Silver orders",
    })

    expect(container.textContent).toBe(
      `Not run after the re-upload of bronze.orders on ${WHEN}: Silver orders did not finish within 6 hours.`
    )
  })

  it("renders the superseded sentence without a cause link", async () => {
    const container = await note({ ...BASE, outcome: "superseded" })

    expect(container.textContent).toBe(
      `Not run for the re-upload of bronze.orders on ${WHEN}: a later re-upload of the same table replaced it.`
    )
    expect(container.querySelector("a")).toBeNull()
  })

  it("renders a widened unknown outcome instead of crashing", async () => {
    const container = await note({ ...BASE, outcome: "skipped_by_operator" })

    expect(container.textContent).toBe(
      `After the re-upload of bronze.orders on ${WHEN}: skipped_by_operator.`
    )
  })

  it("names a pipeline before it when the cause has no name", async () => {
    const container = await note({
      ...BASE,
      outcome: "not_run",
      reason: "upstream_failed",
      causePipelineId: "pl-deleted",
      causePipelineName: null,
    })

    expect(container.textContent).toBe(
      `Not run after the re-upload of bronze.orders on ${WHEN}: a pipeline before it failed first.`
    )
    expect(container.querySelector("a")).toBeNull()
  })

  it("renders nothing when there is no chain step", async () => {
    const container = await note(null)
    expect(container.textContent).toBe("")
  })
})
