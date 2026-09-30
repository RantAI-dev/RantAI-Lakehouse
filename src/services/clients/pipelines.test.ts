import { afterEach, describe, expect, it, spyOn } from "bun:test"
import { isServiceError } from "../errors"
import { dagsterPipelineService } from "./pipelines"

function json(body: unknown, status = 200) {
  return new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } })
}

let fetchSpy: ReturnType<typeof spyOn> | null = null

afterEach(() => {
  fetchSpy?.mockRestore()
  fetchSpy = null
})

function respond(body: unknown, status = 200) {
  fetchSpy = spyOn(globalThis, "fetch").mockResolvedValue(json(body, status))
}

async function rejection(promise: Promise<unknown>) {
  try {
    await promise
  } catch (err) {
    return err
  }
  throw new Error("expected the call to reject")
}

describe("dagsterPipelineService.listPipelines", () => {
  it("keeps an unreachable orchestrator's 503 as a partial list with its reason", async () => {
    respond({ pipelines: [], error: "Dagster unreachable" }, 503)
    const list = await dagsterPipelineService.listPipelines()
    expect(list.pipelines).toEqual([])
    expect(list.error).toBe("Dagster unreachable")
  })

  it("rejects a refusal instead of blaming the orchestrator for it", async () => {
    respond({ error: "missing permission pipeline:read" }, 403)
    const err = await rejection(dagsterPipelineService.listPipelines())
    expect(isServiceError(err) && err.code).toBe("permission_denied")
  })
})

describe("dagsterPipelineService.listRuns", () => {
  it("returns the runs", async () => {
    respond({ runs: [{ id: "r1", status: "completed" }], unavailable: null })
    const runs = await dagsterPipelineService.listRuns("pl-a")
    expect(runs.map((r) => r.id)).toEqual(["r1"])
  })

  it("rejects when the orchestrator could not be asked, rather than reporting no runs", async () => {
    respond({ runs: [], unavailable: "connection refused" })
    const err = await rejection(dagsterPipelineService.listRuns("pl-a"))
    expect(isServiceError(err) && err.code).toBe("unavailable")
    expect((err as Error).message).toContain("connection refused")
  })
})
