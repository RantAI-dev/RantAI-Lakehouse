import { afterEach, describe, expect, it, mock } from "bun:test"
import { ServiceError } from "../errors"
import type { CreatePipelineInput } from "../contracts/pipelines"
import { dagsterPipelineService } from "./pipelines"

const originalFetch = global.fetch

afterEach(() => {
  global.fetch = originalFetch
})

type Call = { url: string; init: RequestInit }

function json(body: unknown, status = 200) {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  })
}

function stubFetch(answer: (call: Call) => Response): Call[] {
  const calls: Call[] = []
  global.fetch = mock(async (input: RequestInfo | URL, init?: RequestInit) => {
    const call: Call = { url: String(input), init: init ?? {} }
    calls.push(call)
    return answer(call)
  }) as unknown as typeof fetch
  return calls
}

async function rejection(run: Promise<unknown>): Promise<ServiceError> {
  try {
    await run
  } catch (err) {
    if (err instanceof ServiceError) return err
    throw new Error(`expected ServiceError, got: ${String(err)}`)
  }
  throw new Error("expected rejection, call resolved")
}

describe("dagsterPipelineService.setTargetLag", () => {
  it("sends PUT /api/pipelines/{id}/target-lag with { targetLag } and returns updated pipeline", async () => {
    const calls = stubFetch(() =>
      json({
        id: "pl-1",
        name: "pl-1",
        targetLag: { kind: "lag", minutes: 15 },
        orchestratorReloaded: true,
      })
    )

    const res = await dagsterPipelineService.setTargetLag("pl-1", { kind: "lag", minutes: 15 })

    expect(calls).toHaveLength(1)
    expect(calls[0].url).toBe("/api/pipelines/pl-1/target-lag")
    expect(calls[0].init.method).toBe("PUT")
    expect(JSON.parse(calls[0].init.body as string)).toEqual({
      targetLag: { kind: "lag", minutes: 15 },
    })
    expect(res.targetLag).toEqual({ kind: "lag", minutes: 15 })
  })

  it("sends PUT with null targetLag to clear target lag", async () => {
    const calls = stubFetch(() =>
      json({
        id: "pl-1",
        name: "pl-1",
        targetLag: null,
        orchestratorReloaded: true,
      })
    )

    const res = await dagsterPipelineService.setTargetLag("pl-1", null)

    expect(calls).toHaveLength(1)
    expect(calls[0].url).toBe("/api/pipelines/pl-1/target-lag")
    expect(calls[0].init.method).toBe("PUT")
    expect(JSON.parse(calls[0].init.body as string)).toEqual({
      targetLag: null,
    })
    expect(res.targetLag).toBeNull()
  })

  it("throws invalid_request on 400 (e.g. cycle detected or invalid minutes)", async () => {
    stubFetch(() =>
      json({ error: "target lag would create a cycle: pl-1 -> pl-2 -> pl-1" }, 400)
    )

    const err = await rejection(
      dagsterPipelineService.setTargetLag("pl-1", { kind: "lag", minutes: 15 })
    )
    expect(err.code).toBe("invalid_request")
    expect(err.message).toBe("target lag would create a cycle: pl-1 -> pl-2 -> pl-1")
  })

  it("throws not_found on 404 (e.g. non-authored or nonexistent pipeline)", async () => {
    stubFetch(() => json({ error: "Pipeline not found or not authored" }, 404))

    const err = await rejection(
      dagsterPipelineService.setTargetLag("unknown-pl", { kind: "downstream" })
    )
    expect(err.code).toBe("not_found")
  })
})

// BLD-1 part 1 / D16: the preview of one input or step of an unsaved
// definition. The body and the answer are plan section 3's contract.
describe("dagsterPipelineService.previewPipelineStep", () => {
  const body = {
    inputs: [{ name: "orders", zone: "bronze", table: "orders" }],
    steps: [{ name: "paid", kind: "sql" as const, sql: "SELECT * FROM orders" }],
    upTo: "paid",
  }

  it("posts the definition and upTo to /api/pipelines/preview and returns columns, rows and truncated", async () => {
    const calls = stubFetch(() =>
      json({ columns: [{ name: "id", type: "Int64" }], rows: [[1], [2]], truncated: false })
    )

    const res = await dagsterPipelineService.previewPipelineStep(body)

    expect(calls).toHaveLength(1)
    expect(calls[0].url).toBe("/api/pipelines/preview")
    expect(calls[0].init.method).toBe("POST")
    expect(JSON.parse(calls[0].init.body as string)).toEqual(body)
    expect(res).toEqual({ columns: [{ name: "id", type: "Int64" }], rows: [[1], [2]], truncated: false })
  })

  it("keeps the API's sentence on a 400, so the editor can read the step it names (D7)", async () => {
    stubFetch(() => json({ error: "steps[0]: this step reads a table that is not an input; add it under Inputs" }, 400))

    const err = await rejection(dagsterPipelineService.previewPipelineStep(body))

    expect(err.code).toBe("invalid_request")
    expect(err.message).toBe("steps[0]: this step reads a table that is not an input; add it under Inputs")
  })

  it("throws invalid_request on a 422 refusal by the access check, with its message", async () => {
    stubFetch(() => json({ error: "inputs[0]: you may not read this table" }, 422))

    const err = await rejection(dagsterPipelineService.previewPipelineStep(body))

    expect(err.code).toBe("invalid_request")
    expect(err.message).toBe("inputs[0]: you may not read this table")
  })

  // BLD-1-AC11 and WEB-11: a caller without `pipeline:write` is refused with
  // the shared sentence (`deniedFor`), not the API's code.
  it("throws permission_denied with the shared sentence on a 403", async () => {
    stubFetch(() => json({ error: "permission_denied: pipeline:write" }, 403))

    const err = await rejection(dagsterPipelineService.previewPipelineStep(body))

    expect(err.code).toBe("permission_denied")
    expect(err.status).toBe(403)
    expect(err.message).toBe("You don't have access. This needs the pipeline:write permission.")
  })
})

// QA TC-1734 F3, LH-DATA-19-03, LH-BLD-2-03 F1 (WEB-11): a refusal is not an
// outage and not the API's code.
describe("dagsterPipelineService.listPipelines", () => {
  it("listPipelines throws permission_denied on a 403 and does not answer an empty list", async () => {
    stubFetch(() => json({ error: "permission_denied: pipeline:read" }, 403))

    const err = await rejection(dagsterPipelineService.listPipelines())
    expect(err.code).toBe("permission_denied")
    expect(err.status).toBe(403)
    expect(err.message).toBe("You don't have access. This needs the pipeline:read permission.")
  })

  it("listPipelines still answers the 503 body, so the page can say the orchestrator was not reached", async () => {
    stubFetch(() => json({ pipelines: [], error: "The orchestrator did not answer." }, 503))

    const res = await dagsterPipelineService.listPipelines()
    expect(res.error).toBe("The orchestrator did not answer.")
    expect(res.pipelines).toEqual([])
  })
})

describe("dagsterPipelineService.getPipeline", () => {
  it("getPipeline throws permission_denied on a 403, not unavailable", async () => {
    stubFetch(() => json({ error: "permission_denied: pipeline:read" }, 403))

    const err = await rejection(dagsterPipelineService.getPipeline("pl-1"))
    expect(err.code).toBe("permission_denied")
    expect(err.status).toBe(403)
    expect(err.message).toBe("You don't have access. This needs the pipeline:read permission.")
  })
})

describe("dagsterPipelineService refusals on a write", () => {
  it("setTargetLag throws permission_denied with the sentence on a 403", async () => {
    stubFetch(() => json({ error: "permission_denied: pipeline:write" }, 403))

    const err = await rejection(dagsterPipelineService.setTargetLag("pl-1", { kind: "lag", minutes: 7 }))
    expect(err.code).toBe("permission_denied")
    expect(err.status).toBe(403)
    expect(err.message).toBe("You don't have access. This needs the pipeline:write permission.")
  })

  it("createPipeline throws the sentence, not the API's code, on a 403", async () => {
    stubFetch(() => json({ error: "permission_denied: pipeline:write" }, 403))

    const err = await rejection(dagsterPipelineService.createPipeline({} as CreatePipelineInput))
    expect(err.code).toBe("permission_denied")
    expect(err.message).toBe("You don't have access. This needs the pipeline:write permission.")
    expect(err.message).not.toContain("permission_denied")
  })
})
