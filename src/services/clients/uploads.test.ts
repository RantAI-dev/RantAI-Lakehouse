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

  it("opens a chunked upload with a JSON body of the name and the size", async () => {
    const calls = stubFetch(() =>
      json({ upload: { ...UPLOAD, status: "receiving" }, partSize: 8 * 1024 * 1024, partCount: 3 }, 201)
    )
    const session = await uploadService.createSession({ filename: "big.csv", sizeBytes: 20_000_000 })

    expect(session.partCount).toBe(3)
    expect(session.partSize).toBe(8 * 1024 * 1024)
    const { url, init } = calls[0]!
    expect(url).toBe("/api/uploads/sessions")
    expect(init.method).toBe("POST")
    expect((init.headers as Headers).get("Content-Type")).toBe("application/json")
    expect(JSON.parse(init.body as string)).toEqual({ filename: "big.csv", sizeBytes: 20_000_000 })
  })

  it("sends a piece as a raw PUT of the bytes to its numbered path", async () => {
    const calls = stubFetch(() => json({ partNumber: 2, sizeBytes: 4 }))
    const piece = new Blob(["abcd"])
    const receipt = await uploadService.putPart("up 1", 2, piece)

    expect(receipt).toEqual({ partNumber: 2, sizeBytes: 4 })
    const { url, init } = calls[0]!
    expect(url).toBe("/api/uploads/up%201/parts/2")
    expect(init.method).toBe("PUT")
    expect((init.headers as Headers).get("Content-Type")).toBe("application/octet-stream")
    expect(init.body).toBe(piece)
  })

  it("carries the 429's Retry-After on the error of a piece, and the API's sentence as its message", async () => {
    stubFetch(
      () =>
        new Response(JSON.stringify({ error: "Too many uploads are in progress. Try again in a moment." }), {
          status: 429,
          headers: { "Content-Type": "application/json", "Retry-After": "5" },
        })
    )
    const err = await rejection(uploadService.putPart("up-1", 1, new Blob(["a"])))
    expect(err.status).toBe(429)
    expect(err.retryAfterSecs).toBe(5)
    expect(err.message).toBe("Too many uploads are in progress. Try again in a moment.")
  })

  it("leaves retryAfterSecs unset for a 429 that names none, and for every other status", async () => {
    stubFetch(() => json({ error: "No." }, 429))
    expect((await rejection(uploadService.putPart("up-1", 1, new Blob(["a"])))).retryAfterSecs).toBeUndefined()
    stubFetch(
      () => new Response(JSON.stringify({ error: "No." }), { status: 503, headers: { "Retry-After": "7" } })
    )
    expect((await rejection(uploadService.putPart("up-1", 1, new Blob(["a"])))).retryAfterSecs).toBeUndefined()
  })

  it("does not show a proxy's plain-text answer to a piece: it says the upload did not reach the service", async () => {
    stubFetch(() => new Response("Request Entity Too Large", { status: 413 }))
    const err = await rejection(uploadService.putPart("up-1", 1, new Blob(["a"])))
    expect(err.status).toBe(413)
    expect(err.message).toContain("did not reach the service")
    // The chunked path names its own limit, 2 GB, not the 50 MB of `POST /api/uploads`.
    expect(err.message).toContain("2 GB")
    expect(err.message).not.toContain("Request Entity Too Large")
  })

  it("reads the pieces the API holds, and completes an upload with a POST that has no body", async () => {
    const calls = stubFetch(() => json({ received: [1, 3], partSize: 8, partCount: 3 }))
    expect(await uploadService.listParts("up-1")).toEqual({ received: [1, 3], partSize: 8, partCount: 3 })
    expect(calls[0]!.url).toBe("/api/uploads/up-1/parts")
    expect(calls[0]!.init.method).toBeUndefined()

    const done = stubFetch(() => json({ ...UPLOAD, duplicateOf: { ...UPLOAD, id: "up-0" } }))
    const completed = await uploadService.complete("up-1")
    expect(completed.duplicateOf?.id).toBe("up-0")
    expect(done[0]!.url).toBe("/api/uploads/up-1/complete")
    expect(done[0]!.init.method).toBe("POST")
    expect(done[0]!.init.body).toBeUndefined()
  })

  it("shows the API's sentence when completion is refused", async () => {
    stubFetch(() => json({ error: "Some pieces have not arrived: 2." }, 409))
    const err = await rejection(uploadService.complete("up-1"))
    expect(err.status).toBe(409)
    expect(err.message).toBe("Some pieces have not arrived: 2.")
  })

  it("checks a column list: the reading in force and the columns, as JSON, stored by nobody", async () => {
    const calls = stubFetch(() => json({ sampledRows: 3, count: 1, byColumn: [{ column: "qty", count: 1 }], rows: [{ row: 2, column: "qty", value: "abc" }] }))
    const columns = [{ source: "Qty", name: "qty", type: "integer" as const }]
    const result = await uploadService.check("up-1", { options: { encoding: "utf-8", delimiter: "\t", headerRow: 2 }, columns })
    expect(result.count).toBe(1)
    const { url, init } = calls[0]!
    expect(url).toBe("/api/uploads/up-1/check")
    expect(init.method).toBe("POST")
    expect(JSON.parse(init.body as string)).toEqual({ encoding: "utf-8", delimiter: "\t", headerRow: 2, columns })
    await uploadService.check("up-1", { columns })
    expect(JSON.parse(calls[1]!.init.body as string)).toEqual({ columns })
  })

  it("shows the API's sentence when a column list is refused by the check", async () => {
    stubFetch(() => json({ error: "The column list has 1 columns but the file has 2." }, 400))
    const err = await rejection(uploadService.check("up-1", { columns: [] }))
    expect(err.status).toBe(400)
    expect(err.message).toBe("The column list has 1 columns but the file has 2.")
  })

  it("compares the other files with the first: their ids, the reading and the column list when there is one", async () => {
    const mismatch = { uploadId: "up-2", file: "b.csv", missing: [], extra: ["notes"], typeConflicts: [] }
    const calls = stubFetch(() => json({ mismatches: [mismatch] }))
    const columns = [{ source: "Qty", name: "qty", type: "integer" as const }]
    const result = await uploadService.compare("up-1", {
      alsoUploads: ["up-2", "up-3"],
      options: { encoding: "utf-8", delimiter: ";", headerRow: 1, sheet: "Stock" },
      columns,
    })
    expect(result.mismatches).toEqual([mismatch])
    const { url, init } = calls[0]!
    expect(url).toBe("/api/uploads/up-1/compare")
    expect(init.method).toBe("POST")
    expect(JSON.parse(init.body as string)).toEqual({
      alsoUploads: ["up-2", "up-3"],
      encoding: "utf-8",
      delimiter: ";",
      headerRow: 1,
      sheet: "Stock",
      columns,
    })
    await uploadService.compare("up-1", { alsoUploads: ["up-2"] })
    expect(JSON.parse(calls[1]!.init.body as string)).toEqual({ alsoUploads: ["up-2"] })
  })

  it("shows the API's sentence when a comparison is refused, and a missing file as not found", async () => {
    stubFetch(() => json({ error: "One of the other files was not found." }, 404))
    const err = await rejection(uploadService.compare("up-1", { alsoUploads: ["up-9"] }))
    expect(err.status).toBe(404)
    expect(err.code).toBe("not_found")
    expect(err.message).toBe("One of the other files was not found.")
  })

  it("sends the other files with a load", async () => {
    const calls = stubFetch(() => json({ upload: UPLOAD, runId: "run-1" }))
    await uploadService.ingest("up-1", {
      encoding: "utf-8",
      delimiter: ",",
      headerRow: 0,
      bronzeTable: "stock",
      alsoUploads: ["up-2"],
    })
    expect(JSON.parse(calls[0]!.init.body as string)).toMatchObject({ bronzeTable: "stock", alsoUploads: ["up-2"] })
  })

  it("asks for a preview of the columns of a table the file will be added to", async () => {
    const calls = stubFetch(() => json({}))
    await uploadService.preview("up-1", { headerRow: 0, table: "stock_raw" })
    expect(calls[0]!.url).toBe("/api/uploads/up-1/preview?headerRow=0&table=stock_raw")
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

  it("carries the sentence and status of a 429 on create and ingest, as it does any other refusal (SEC-17)", async () => {
    const sentence = "Too many uploads are in progress. Try again in a moment."
    stubFetch(() => json({ error: sentence }, 429))
    const create = await rejection(uploadService.create(new File(["x"], "a.csv")))
    expect(create.message).toBe(sentence)
    expect(create.status).toBe(429)
    const ingest = await rejection(
      uploadService.ingest("up-1", { bronzeTable: "x", encoding: "utf-8", delimiter: ",", headerRow: 0 })
    )
    expect(ingest.message).toBe(sentence)
    expect(ingest.status).toBe(429)
  })

  it("carries the column-cap sentence of a preview as it is (SEC-17)", async () => {
    stubFetch(() => json({ error: "The file has more than 1,000 columns." }, 400))
    const err = await rejection(uploadService.preview("up-1"))
    expect(err.message).toBe("The file has more than 1,000 columns.")
    expect(err.status).toBe(400)
    expect(err.code).toBe("invalid_request")
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

describe("uploadService folders (DATA-35)", () => {
  const FOLDER = {
    id: "uf-1",
    parentId: null,
    name: "Sales",
    fileCount: 2,
    childCount: 0,
    createdAt: "2026-10-26T10:00:00Z",
    updatedAt: "2026-10-26T10:00:00Z",
  }

  it("sends only the filters that were chosen, so an unfiltered list is the URL it always was", async () => {
    const calls = stubFetch(() => json([]))
    await uploadService.list()
    await uploadService.list(undefined, {})
    await uploadService.list(undefined, { folderId: "uf-1" })
    await uploadService.list(undefined, { folderId: "none", q: " stock & co " })
    await uploadService.list(undefined, { q: "   " })
    expect(calls.map((c) => c.url)).toEqual([
      "/api/uploads",
      "/api/uploads",
      "/api/uploads?folderId=uf-1",
      "/api/uploads?folderId=none&q=stock+%26+co",
      "/api/uploads",
    ])
  })

  it("lists, creates, renames, moves and deletes folders on the folder routes", async () => {
    const calls = stubFetch(() => json(FOLDER))
    await expect(uploadService.listFolders()).resolves.toEqual(FOLDER)
    await uploadService.createFolder({ name: "Sales" })
    await uploadService.createFolder({ name: "Q1", parentId: "uf-1" })
    await uploadService.updateFolder("uf-1", { name: "Old" })
    await uploadService.updateFolder("a/b", { parentId: null })
    expect(calls.map((c) => `${c.init.method ?? "GET"} ${c.url}`)).toEqual([
      "GET /api/uploads/folders",
      "POST /api/uploads/folders",
      "POST /api/uploads/folders",
      "PATCH /api/uploads/folders/uf-1",
      "PATCH /api/uploads/folders/a%2Fb",
    ])
    expect(JSON.parse(String(calls[1]!.init.body))).toEqual({ name: "Sales" })
    expect(JSON.parse(String(calls[2]!.init.body))).toEqual({ name: "Q1", parentId: "uf-1" })
    expect(JSON.parse(String(calls[3]!.init.body))).toEqual({ name: "Old" })
    expect(JSON.parse(String(calls[4]!.init.body))).toEqual({ parentId: null })
  })

  it("deletes a folder with a 204 and moves files with the ids and the folder, null meaning none", async () => {
    let answer: () => Response = () => new Response(null, { status: 204 })
    const calls = stubFetch(() => answer())
    await uploadService.deleteFolder("uf-1")
    expect(calls[0]!.init.method).toBe("DELETE")
    expect(calls[0]!.url).toBe("/api/uploads/folders/uf-1")

    answer = () => json({ moved: 2 })
    await expect(uploadService.moveUploads({ uploadIds: ["up-1", "up-2"], folderId: null })).resolves.toEqual({ moved: 2 })
    expect(calls[1]!.url).toBe("/api/uploads/move")
    expect(JSON.parse(String(calls[1]!.init.body))).toEqual({ uploadIds: ["up-1", "up-2"], folderId: null })
  })

  it("shows the API's sentence for a folder it refuses, with its status", async () => {
    stubFetch(() => json({ error: "This folder still holds files or folders. Move them out and delete its folders first." }, 409))
    const err = await rejection(uploadService.deleteFolder("uf-1"))
    expect(err.status).toBe(409)
    expect(err.message).toBe("This folder still holds files or folders. Move them out and delete its folders first.")
    stubFetch(() => json({ error: "Folders can be nested at most 3 levels deep." }, 400))
    expect((await rejection(uploadService.createFolder({ name: "D", parentId: "uf-3" }))).message).toBe(
      "Folders can be nested at most 3 levels deep."
    )
  })
})
