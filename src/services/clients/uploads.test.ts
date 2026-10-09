import { afterEach, beforeEach, describe, expect, it, mock } from "bun:test"
import { ServiceError } from "../errors"
import { uploadService } from "./uploads"

const originalFetch = global.fetch

beforeEach(() => {
  localStorage.clear()
})

afterEach(() => {
  global.fetch = originalFetch
  localStorage.clear()
})

type Call = { url: string; init: RequestInit }

function json(body: unknown, status = 200) {
  return new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } })
}

/** Answers every fetch with `answer` and records what was asked, in order. */
function stubFetch(answer: () => Response): Call[] {
  const calls: Call[] = []
  global.fetch = mock(async (input: RequestInfo | URL, init?: RequestInit) => {
    calls.push({ url: String(input), init: init ?? {} })
    return answer()
  }) as unknown as typeof fetch
  return calls
}

const UPLOAD = {
  id: "up-1",
  originalFilename: "stock.csv",
  sizeBytes: 10,
  uploadedBy: "Ana",
  status: "uploaded",
  createdAt: "2026-10-02T10:00:00Z",
  updatedAt: "2026-10-02T10:00:00Z",
}

async function rejection(run: Promise<unknown>): Promise<ServiceError> {
  try {
    await run
  } catch (err) {
    expect(err).toBeInstanceOf(ServiceError)
    return err as ServiceError
  }
  throw new Error("expected the call to be refused")
}

describe("uploadService requests", () => {
  it("lists the uploads from /api/uploads as the bare array the API sends", async () => {
    const calls = stubFetch(() => json([UPLOAD]))
    await expect(uploadService.list()).resolves.toEqual([UPLOAD])
    expect(calls[0]!.url).toBe("/api/uploads")
    expect(calls[0]!.init.method).toBeUndefined()
  })

  it("sends the active tenant with every request, as apiFetch does", async () => {
    localStorage.setItem("lh_active_tenant", "tenant-a-id")
    const calls = stubFetch(() => json([]))
    await uploadService.list()
    expect((calls[0]!.init.headers as Headers).get("X-Tenant")).toBe("tenant-a-id")
  })

  it("gets one upload by id, escaping the id", async () => {
    const calls = stubFetch(() => json(UPLOAD))
    await uploadService.get("up-1")
    await uploadService.get("a/b c")
    expect(calls[0]!.url).toBe("/api/uploads/up-1")
    expect(calls[1]!.url).toBe("/api/uploads/a%2Fb%20c")
  })

  it("creates an upload as a multipart form with one part named file, and sets no Content-Type", async () => {
    const calls = stubFetch(() => json({ ...UPLOAD, duplicateOf: { ...UPLOAD, id: "up-0" } }, 201))
    const file = new File(["a,b\n1,2\n"], "stock.csv", { type: "text/csv" })
    const created = await uploadService.create(file)

    expect(created.id).toBe("up-1")
    expect(created.duplicateOf?.id).toBe("up-0")
    const { url, init } = calls[0]!
    expect(url).toBe("/api/uploads")
    expect(init.method).toBe("POST")
    const body = init.body as FormData
    expect(body).toBeInstanceOf(FormData)
    expect([...body.keys()]).toEqual(["file"])
    const sent = body.get("file") as File
    expect(sent.name).toBe("stock.csv")
    expect(await sent.text()).toBe("a,b\n1,2\n")
    // A header set here would have no boundary; the browser writes it.
    expect((init.headers as Headers).has("Content-Type")).toBe(false)
  })

  it("asks for a preview with no query when nothing is overridden", async () => {
    const calls = stubFetch(() => json({}))
    await uploadService.preview("up-1")
    await uploadService.preview("up-1", {})
    expect(calls.map((c) => c.url)).toEqual(["/api/uploads/up-1/preview", "/api/uploads/up-1/preview"])
  })

  it("sends only the settings the caller chose, a tab as %09 and header row 0 as 0", async () => {
    const calls = stubFetch(() => json({}))
    await uploadService.preview("up-1", { encoding: "utf-16", delimiter: "\t", headerRow: 4 })
    await uploadService.preview("up-1", { delimiter: ";" })
    await uploadService.preview("up-1", { headerRow: 0 })
    await uploadService.preview("up-1", { delimiter: "|", encoding: "utf-8" })
    expect(calls.map((c) => c.url)).toEqual([
      "/api/uploads/up-1/preview?encoding=utf-16&delimiter=%09&headerRow=4",
      "/api/uploads/up-1/preview?delimiter=%3B",
      "/api/uploads/up-1/preview?headerRow=0",
      "/api/uploads/up-1/preview?encoding=utf-8&delimiter=%7C",
    ])
  })

  it("starts a load with a JSON body and returns the upload and its run", async () => {
    const calls = stubFetch(() => json({ upload: { ...UPLOAD, status: "ingesting" }, runId: "run-1" }))
    const input = { bronzeTable: "stock", mode: "append" as const, encoding: "utf-8" as const, delimiter: ",", headerRow: 0 }
    const result = await uploadService.ingest("up-1", input)

    expect(result).toEqual({ upload: { ...UPLOAD, status: "ingesting" }, runId: "run-1" })
    const { url, init } = calls[0]!
    expect(url).toBe("/api/uploads/up-1/ingest")
    expect(init.method).toBe("POST")
    expect((init.headers as Headers).get("Content-Type")).toBe("application/json")
    expect(JSON.parse(init.body as string)).toEqual(input)
  })

  it("leaves mode out of the body when none is given, so the API's default applies", async () => {
    const calls = stubFetch(() => json({ upload: UPLOAD, runId: "run-1" }))
    await uploadService.ingest("up-1", { bronzeTable: "stock", encoding: "utf-16", delimiter: "\t", headerRow: 5 })
    expect(JSON.parse(calls[0]!.init.body as string)).toEqual({
      bronzeTable: "stock",
      encoding: "utf-16",
      delimiter: "\t",
      headerRow: 5,
    })
  })

  it("deletes an upload and reads the 204 with no body as success", async () => {
    const calls = stubFetch(() => new Response(null, { status: 204 }))
    await expect(uploadService.remove("up-1")).resolves.toBeUndefined()
    expect(calls[0]!.url).toBe("/api/uploads/up-1")
    expect(calls[0]!.init.method).toBe("DELETE")
  })

  it("passes the abort signal on, so leaving the page cancels a slow upload", async () => {
    const calls = stubFetch(() => json(UPLOAD, 201))
    const controller = new AbortController()
    await uploadService.create(new File(["x"], "x.csv"), controller.signal)
    expect(calls[0]!.init.signal).toBe(controller.signal)
  })
})

describe("uploadService errors", () => {
  it("carries the API's sentence and status into a ServiceError, whatever the call", async () => {
    stubFetch(() => json({ error: "The file is larger than the 50 MB limit." }, 400))
    const create = await rejection(uploadService.create(new File(["x"], "x.csv")))
    expect(create.message).toBe("The file is larger than the 50 MB limit.")
    expect(create.status).toBe(400)
    expect(create.code).toBe("invalid_request")

    const ingest = await rejection(
      uploadService.ingest("up-1", { bronzeTable: "X", encoding: "utf-8", delimiter: ",", headerRow: 0 })
    )
    expect(ingest.message).toBe("The file is larger than the 50 MB limit.")
    expect((await rejection(uploadService.list())).message).toBe("The file is larger than the 50 MB limit.")
    expect((await rejection(uploadService.get("up-1"))).message).toBe("The file is larger than the 50 MB limit.")
    expect((await rejection(uploadService.preview("up-1"))).message).toBe("The file is larger than the 50 MB limit.")
    expect((await rejection(uploadService.remove("up-1"))).message).toBe("The file is larger than the 50 MB limit.")
  })

  it("maps the status to the code the pages branch on", async () => {
    const cases: [number, string][] = [
      [400, "invalid_request"],
      [409, "invalid_request"],
      [422, "invalid_request"],
      [401, "permission_denied"],
      [403, "permission_denied"],
      [404, "not_found"],
      [500, "unavailable"],
      [503, "unavailable"],
    ]
    for (const [status, code] of cases) {
      stubFetch(() => json({ error: "A sentence." }, status))
      const err = await rejection(uploadService.get("up-1"))
      expect(err.code).toBe(code as ServiceError["code"])
      expect(err.status).toBe(status)
    }
  })

  it("keeps a 409's status and a 403's own words", async () => {
    stubFetch(() => json({ error: "That table name cannot be used. Choose another name." }, 409))
    const conflict = await rejection(
      uploadService.ingest("up-1", { bronzeTable: "x", encoding: "utf-8", delimiter: ",", headerRow: 0 })
    )
    expect(conflict.status).toBe(409)
    expect(conflict.message).toBe(
      "That table name cannot be used. Choose another name."
    )

    stubFetch(() => json({ error: "permission_denied: connector:manage" }, 403))
    const refused = await rejection(uploadService.list())
    expect(refused.code).toBe("permission_denied")
    expect(refused.message).toBe("permission_denied: connector:manage")
  })

  it("says what failed when the body is not the API's JSON, and invents no sentence", async () => {
    stubFetch(() => new Response("<html>Bad gateway</html>", { status: 502 }))
    const gateway = await rejection(uploadService.list())
    expect(gateway.message).toBe("Failed (502)")
    expect(gateway.code).toBe("unavailable")

    for (const body of [{ error: "" }, { error: "   " }, { error: 5 }, { message: "x" }, null, "text"]) {
      stubFetch(() => json(body, 400))
      expect((await rejection(uploadService.get("up-1"))).message).toBe("Failed (400)")
    }
  })

  it("answers a create that a proxy cut off with a sentence naming the limit, not the body, and keeps the API's own sentence", async () => {
    stubFetch(() => new Response("Internal Server Error", { status: 500 }))
    const cut = await rejection(uploadService.create(new File(["x"], "a.csv")))
    expect(cut.status).toBe(500)
    expect(cut.message).toContain("did not reach the service")
    expect(cut.message).toContain("50 MB")
    expect(cut.message).not.toContain("Internal Server Error")

    stubFetch(() => json({ error: "The file is empty." }, 400))
    expect((await rejection(uploadService.create(new File(["x"], "a.csv")))).message).toBe("The file is empty.")
  })

  it("refuses a delete with the API's sentence and keeps its status", async () => {
    stubFetch(() => json({ error: "This upload is being loaded, so it cannot be deleted yet." }, 409))
    const err = await rejection(uploadService.remove("up-1"))
    expect(err.status).toBe(409)
    expect(err.message).toBe("This upload is being loaded, so it cannot be deleted yet.")
  })
})
