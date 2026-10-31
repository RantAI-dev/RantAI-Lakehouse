// The upload page (plan T11, `docs/superpowers/plans/2026-10-02-upload-file.md`;
// checklist in `docs/core/features/upload-file.md`): a file over 50 MB is
// refused before anything is sent, a refusal of the API is shown as it is, the
// preview shows what was detected and what is in force and reads again when a
// setting changes, the choice to replace or add appears only for a table an
// upload created, and a load is shown Loading, Loaded or Failed.
//
// Files are sent in pieces (DATA-17): the stub answers the session, the piece
// and the completion routes, and a test that needs a bar to be seen moving
// holds a piece's answer back.
const url = { search: "" }
mock.module("next/navigation", () => ({
  usePathname: () => "/connectors/upload",
  useSearchParams: () => new URLSearchParams(url.search),
  useRouter: () => ({
    push: () => {},
    replace: () => {},
    refresh: () => {},
    back: () => {},
    forward: () => {},
    prefetch: () => {},
  }),
}))

import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, mock, spyOn } from "bun:test"
import { formatBytes } from "@/lib/format"
import { MAX_SESSION_BYTES, REGISTRATION_FAILED_REASON, TABLE_NAME_RULE } from "@/lib/uploads"
import type { Upload, UploadColumnSpec, UploadFolder, UploadPreview } from "@/services/contracts/uploads"
import { UploadFilePage } from "./upload-file-page"

const originalFetch = global.fetch

beforeEach(() => {
  // happy-dom starts on about:blank, where a path cannot be written.
  spyOn(window.history, "replaceState").mockImplementation(() => {})
})

afterEach(() => {
  global.fetch = originalFetch
  url.search = ""
  mock.restore()
  cleanup()
})

/**
 * Picks an option of a base-ui Select the way a pointer does. The item only commits a click that
 * began with a pointerdown on it (a bare `fireEvent.click` is dropped as a stray mouse click), so a
 * test that skips the pointerdown never changes the selection (PR #259 CI (folder picker tests)).
 */
function pickOption(option: HTMLElement) {
  fireEvent.pointerDown(option, { button: 0, pointerType: "mouse" })
  fireEvent.mouseDown(option, { button: 0 })
  fireEvent.pointerUp(option, { button: 0, pointerType: "mouse" })
  fireEvent.mouseUp(option, { button: 0 })
  fireEvent.click(option)
}

function json(body: unknown, status = 200) {
  return new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } })
}

const OPTIONS = { encoding: "utf-8" as const, delimiter: ",", headerRow: 0 }

function upload(over: Partial<Upload> = {}): Upload {
  return {
    id: "up-1",
    originalFilename: "stock.csv",
    sizeBytes: 2048,
    uploadedBy: "Ana",
    status: "uploaded",
    createdAt: "2026-10-02T10:00:00Z",
    updatedAt: "2026-10-02T10:00:00Z",
    ...over,
  }
}

/** The columns the API would offer for a header: every one text, named as given (the guess is the API's, tested there). */
function textSpecs(columns: string[]): UploadPreview["columnSpecs"] {
  return columns.map((source) => ({
    source,
    name: source.toLowerCase().replace(/[^a-z0-9]+/g, "_").replace(/^_+|_+$/g, ""),
    type: "text",
    guessedType: "text",
    askDateOrder: false,
  }))
}

function preview(over: Partial<UploadPreview> = {}): UploadPreview {
  return {
    detected: OPTIONS,
    using: OPTIONS,
    columns: ["sku", "qty"],
    columnSpecs: textSpecs(over.columns ?? ["sku", "qty"]),
    sampledRows: 2,
    rows: [
      ["a1", "3"],
      ["b2", "5"],
    ],
    truncated: false,
    ...over,
  }
}

type Call = { url: string; method: string; body: unknown }

/** What a route answers: a response, or a function making a fresh one (a body can be read once). */
type Answer<Args extends unknown[] = []> = Response | ((...args: Args) => Response | Promise<Response>)

type Routes = {
  list?: Upload[] | Response
  /** `POST /api/uploads/sessions`; the default opens `up-N` with 8 MiB pieces. */
  session?: Answer<[{ filename: string; sizeBytes: number }]>
  /** `PUT /api/uploads/{id}/parts/{n}`; the default stores the piece. */
  part?: Answer<[string, number]>
  /** `POST /api/uploads/{id}/complete`; the default is the stored upload. */
  complete?: Answer<[string]>
  get?: (n: number) => Upload | Response
  preview?: (query: URLSearchParams) => UploadPreview | Response
  /** `POST /api/uploads/{id}/check`: gets the body the page sent. */
  check?: (body: { columns: { name: string; type: string }[] }) => unknown | Response
  /** `POST /api/uploads/{id}/compare` (several files into one table): gets the body the page sent; the default finds no mismatch. */
  compare?: (body: { alsoUploads: string[]; columns?: { name: string; type: string }[] }) => unknown | Response
  ingest?: Response
  /** `GET /api/uploads/folders` (`DATA-35`); the default is no folders. */
  folders?: UploadFolder[] | Response
  /** `POST /api/uploads/folders`: gets the body the page sent. */
  createFolder?: (body: { name: string; parentId?: string }) => UploadFolder | Response
}

function folder(over: Partial<UploadFolder> & Pick<UploadFolder, "id" | "name">): UploadFolder {
  return {
    parentId: null,
    fileCount: 0,
    childCount: 0,
    createdAt: "2026-10-26T10:00:00Z",
    updatedAt: "2026-10-26T10:00:00Z",
    ...over,
  }
}

const PIECE = 8 * 1024 * 1024

async function answer<Args extends unknown[]>(value: Answer<Args>, ...args: Args): Promise<Response> {
  return typeof value === "function" ? value(...args) : value
}

/** Answers every request the page makes; records them in order. A request nobody expects throws. */
function stubFetch(routes: Routes = {}): Call[] {
  const calls: Call[] = []
  const names = new Map<string, string>()
  let gets = 0
  let sessions = 0
  global.fetch = mock(async (input: RequestInfo | URL, init?: RequestInit) => {
    const target = new URL(String(input), "http://console.test")
    const method = init?.method ?? "GET"
    const body = typeof init?.body === "string" ? JSON.parse(init.body) : init?.body
    calls.push({ url: `${target.pathname}${target.search}`, method, body })
    const reply = (value: unknown) => (value instanceof Response ? value : json(value))
    if (target.pathname === "/api/uploads" && method === "GET") return reply(routes.list ?? [])
    // Before the per-upload routes below: `folders` is not an upload id.
    if (target.pathname === "/api/uploads/folders" && method === "GET") return reply(routes.folders ?? [])
    if (target.pathname === "/api/uploads/folders" && method === "POST") {
      const asked = body as { name: string; parentId?: string }
      return reply(routes.createFolder ? routes.createFolder(asked) : folder({ id: "uf-new", name: asked.name, parentId: asked.parentId ?? null }))
    }
    if (target.pathname === "/api/uploads/sessions" && method === "POST") {
      const input = body as { filename: string; sizeBytes: number }
      if (routes.session) return answer(routes.session, input)
      sessions += 1
      const id = `up-${sessions}`
      names.set(id, input.filename)
      return json(
        {
          upload: upload({ id, originalFilename: input.filename, status: "receiving", declaredSize: input.sizeBytes, sizeBytes: 0 }),
          partSize: PIECE,
          partCount: Math.max(1, Math.ceil(input.sizeBytes / PIECE)),
        },
        201
      )
    }
    const piece = /^\/api\/uploads\/([^/]+)\/parts\/(\d+)$/.exec(target.pathname)
    if (piece && method === "PUT") {
      const number = Number(piece[2])
      if (routes.part) return answer(routes.part, piece[1], number)
      return json({ partNumber: number, sizeBytes: (init?.body as Blob).size })
    }
    const done = /^\/api\/uploads\/([^/]+)\/complete$/.exec(target.pathname)
    if (done && method === "POST") {
      if (routes.complete) return answer(routes.complete, done[1])
      return json(upload({ id: done[1], originalFilename: names.get(done[1]) ?? "stock.csv" }))
    }
    if (/^\/api\/uploads\/[^/]+$/.test(target.pathname) && method === "DELETE") return new Response(null, { status: 204 })
    if (target.pathname.endsWith("/preview") && routes.preview) return reply(routes.preview(target.searchParams))
    if (target.pathname.endsWith("/check") && method === "POST" && routes.check) {
      return reply(routes.check(body as { columns: { name: string; type: string }[] }))
    }
    if (target.pathname.endsWith("/compare") && method === "POST") {
      const asked = body as { alsoUploads: string[]; columns?: { name: string; type: string }[] }
      return reply(routes.compare ? routes.compare(asked) : { mismatches: [] })
    }
    if (target.pathname.endsWith("/ingest") && method === "POST" && routes.ingest) return routes.ingest
    if (/^\/api\/uploads\/[^/]+$/.test(target.pathname) && method === "GET" && routes.get) {
      gets += 1
      return reply(routes.get(gets))
    }
    throw new Error(`unexpected fetch: ${method} ${target.pathname}${target.search}`)
  }) as unknown as typeof fetch
  return calls
}

function chooseFiles(files: File[]) {
  fireEvent.change(screen.getByLabelText("Choose files"), { target: { files } })
}

function chooseFile(file: File) {
  chooseFiles([file])
}

/** The calls of one kind, by their path. */
function sent(calls: Call[], method: string, pattern: RegExp) {
  return calls.filter((c) => c.method === method && pattern.test(c.url))
}

function next() {
  return screen.getByRole("button", { name: "Next" }) as HTMLButtonElement
}

const CSV = () => new File(["sku,qty\na1,3\nb2,5\n"], "stock.csv", { type: "text/csv" })

describe("UploadFilePage, first step", () => {
  it("has a labelled file control and refuses a file over 2 GB, sending nothing", async () => {
    const calls = stubFetch()
    render(<UploadFilePage />)
    const big = new File(["x"], "big.csv")
    Object.defineProperty(big, "size", { value: MAX_SESSION_BYTES + 1 })
    chooseFile(big)

    const alert = await screen.findByRole("alert")
    expect(alert.textContent).toContain("big.csv")
    expect(alert.textContent).toContain("2 GB limit")
    expect(next().disabled).toBe(true)
    fireEvent.click(next())
    expect(calls.filter((c) => c.method !== "GET")).toEqual([])
  })

  it("shows the API's refusal of a file in its row, word for word, and stays on the first step", async () => {
    const sentence = "This looks like a workbook. Save it as CSV and upload that."
    stubFetch({ complete: () => json({ error: sentence }, 400) })
    render(<UploadFilePage />)
    // A name the console does not refuse by itself (it refuses .xlsm before any
    // request, see below), so the API's own refusal by content is what shows.
    chooseFile(new File(["x"], "stock.dat"))

    const alert = await screen.findByRole("alert")
    expect(alert.textContent).toBe(sentence)
    // The file is still listed, and the same control can pick another.
    expect(screen.getByText("stock.dat")).toBeDefined()
    expect(screen.getByLabelText("Choose files")).toBeDefined()
    expect(next().disabled).toBe(true)
  })

  it("refuses a workbook the API does not read by its name before any request, saying what to save it as", async () => {
    const calls = stubFetch()
    render(<UploadFilePage />)
    chooseFile(new File(["x"], "stock.XLSM"))

    const alert = await screen.findByRole("alert")
    expect(alert.textContent).toContain("stock.XLSM")
    expect(alert.textContent).toContain("looks like a workbook or an archive")
    expect(alert.textContent).toContain("save the sheet as .xlsx or CSV first")
    expect(next().disabled).toBe(true)
    fireEvent.click(next())
    expect(calls.filter((c) => c.method !== "GET")).toEqual([])
  })

  it("states the limits as short facts up front, and suggests text and Excel files in the picker", () => {
    stubFetch()
    render(<UploadFilePage />)
    const facts = new Map(
      screen.getAllByRole("term").map((term) => [term.textContent, term.nextElementSibling?.textContent])
    )
    expect(facts.get("Accepted")).toBe("CSV, TSV, JSON, JSON Lines, Avro, .xls, .xlsx, .parquet, .gz and .zip")
    expect(facts.get("Files")).toContain("Up to 10 at once")
    expect(facts.get("Size")).toBe("Up to 2 GB in all; an Excel workbook up to 50 MB")
    expect(facts.get("Not accepted")).toBe(".xlsm, .xlsb, .ods, .tar, .7z and other archives")
    expect(facts.get("Workbooks")).toContain("One sheet is loaded")
    expect(facts.get("Parquet and Avro")).toContain("binary column is refused")
    expect(facts.get("JSON")).toContain("nested value is kept as JSON text")
    expect(facts.get("Compressed")).toContain("up to 10 files of one format")
    const input = screen.getByLabelText("Choose files") as HTMLInputElement
    expect(input.type).toBe("file")
    expect(input.multiple).toBe(true)
    expect(input.accept).toContain(".csv")
    expect(input.accept).toContain(".tsv")
    expect(input.accept).toContain(".xls")
    expect(input.accept).toContain(".xlsx")
    expect(input.accept).toContain(".parquet")
    expect(input.accept).toContain(".json")
    expect(input.accept).toContain(".avro")
    expect(input.accept).toContain(".gz")
    expect(input.accept).toContain(".zip")
  })

  it("is a drop area with one line, a button labelling the real file input, and the page's subtitle names Excel", () => {
    stubFetch()
    render(<UploadFilePage />)
    expect(screen.getByText("Drop files here, or choose some")).toBeDefined()
    expect(screen.getByText(/CSV, TSV, JSON, Avro, Excel or Parquet files, or a gzip or zip of them, in as raw tables/)).toBeDefined()
    const input = screen.getByLabelText("Choose files")
    const button = screen.getByText("Choose files", { selector: "label" })
    expect(button.getAttribute("for")).toBe(input.id)
  })

  it("highlights the area while files are dragged over it, and takes the dropped files as rows that start to be sent", async () => {
    const calls = stubFetch()
    render(<UploadFilePage />)
    const area = screen.getByText("Drop files here, or choose some").closest("[data-dragging]") as HTMLElement
    expect(area.dataset.dragging).toBe("false")

    fireEvent.dragOver(area)
    expect(area.dataset.dragging).toBe("true")
    expect(screen.getByText("Release to choose these files")).toBeDefined()
    fireEvent.dragLeave(area)
    expect(area.dataset.dragging).toBe("false")

    const file = CSV()
    fireEvent.dragOver(area)
    fireEvent.drop(area, { dataTransfer: { files: [file] } })
    expect(area.dataset.dragging).toBe("false")
    await waitFor(() => expect(screen.getByText("stock.csv")).toBeDefined())
    expect(screen.getByText(new RegExp(formatBytes(file.size)))).toBeDefined()
    // The drop area stays, so more files can be added.
    expect(screen.getByText("Drop files here, or choose some")).toBeDefined()
    await waitFor(() => expect(next().disabled).toBe(false))
    const session = sent(calls, "POST", /\/api\/uploads\/sessions$/)
    expect(session.length).toBe(1)
    expect(session[0].body).toEqual({ filename: "stock.csv", sizeBytes: file.size })
  })

  it("marks the chosen files by their format: green spreadsheet and XLS/XLSX, text and CSV/TSV, neutral otherwise, in any case", () => {
    stubFetch()
    render(<UploadFilePage />)
    const cases: [string, string | null, string, string][] = [
      ["stock.xls", "XLS", "workbook", "text-emerald-700"],
      ["Stock.XLSX", "XLSX", "workbook", "text-emerald-700"],
      ["orders.parquet", "PARQUET", "parquet", "text-violet-700"],
      ["ORDERS.Parquet", "PARQUET", "parquet", "text-violet-700"],
      ["stock.csv", "CSV", "delimited", "text-primary"],
      ["STOCK.TSV", "TSV", "delimited", "text-primary"],
      ["notes.txt", "TXT", "other", "text-muted-foreground"],
      ["export", null, "other", "text-muted-foreground"],
    ]
    chooseFiles(cases.map(([name]) => new File(["x"], name)))
    const marks = Array.from(document.querySelectorAll("[data-file-mark]")) as HTMLElement[]
    expect(marks.length).toBe(cases.length)
    cases.forEach(([, label, kind, tone], index) => {
      const mark = marks[index]
      expect(mark.dataset.fileMark).toBe(kind)
      expect(mark.className).toContain(tone)
      expect(mark.getAttribute("aria-hidden")).toBe("true")
      expect(mark.textContent).toBe(label ?? "")
      // Same size whatever the kind: the mark never changes the row's height.
      expect(mark.className).toContain("size-10")
    })
  })

  it("refuses a dropped file by the same checks as a chosen one, and a drop with no file changes nothing", async () => {
    stubFetch()
    render(<UploadFilePage />)
    const area = screen.getByText("Drop files here, or choose some").closest("[data-dragging]") as HTMLElement
    fireEvent.drop(area, { dataTransfer: { files: [new File(["x"], "stock.XLSM")] } })
    const alert = await screen.findByRole("alert")
    expect(alert.textContent).toContain("looks like a workbook or an archive")
    expect(next().disabled).toBe(true)

    fireEvent.drop(area, { dataTransfer: { files: [] } })
    expect(screen.getByText("stock.XLSM")).toBeDefined()
    expect(screen.getAllByRole("alert").length).toBe(1)
  })

  it("takes up to ten files at once, sends each, and says how many were left out of a larger pick", async () => {
    const calls = stubFetch()
    render(<UploadFilePage />)
    const picked = Array.from({ length: 11 }, (_, i) => new File([`a,b\n${i},1\n`], `part-${i}.csv`))
    chooseFiles(picked)

    expect(await screen.findByText(/the first ones were taken and 1 was left out/)).toBeDefined()
    expect(screen.getAllByRole("progressbar").length).toBe(10)
    expect(screen.queryByText("part-10.csv")).toBeNull()
    await waitFor(() => expect(sent(calls, "POST", /\/complete$/).length).toBe(10))
    expect(sent(calls, "POST", /\/sessions$/).length).toBe(10)
    // More files cannot be added once ten are listed.
    chooseFiles([new File(["x"], "extra.csv")])
    expect(await screen.findByText(/the first ones were taken and 1 was left out/)).toBeDefined()
    expect(screen.queryByText("extra.csv")).toBeNull()
  })

  it("shows a bar per file that moves as the API acknowledges pieces, then says Received", async () => {
    let release: (() => void) | null = null
    const held = new Promise<void>((resolve) => {
      release = resolve
    })
    stubFetch({
      // Pieces of 4 bytes: the 8-byte file is two pieces; the second is held back.
      session: ({ filename, sizeBytes }) =>
        json(
          { upload: upload({ id: "up-9", originalFilename: filename, status: "receiving", sizeBytes: 0 }), partSize: 4, partCount: Math.ceil(sizeBytes / 4) },
          201
        ),
      part: async (_id, n) => {
        if (n === 2) await held
        return json({ partNumber: n, sizeBytes: 4 })
      },
    })
    render(<UploadFilePage />)
    chooseFile(new File(["12345678"], "bars.csv"))

    const bar = await screen.findByRole("progressbar", { name: "bars.csv sent" })
    await waitFor(() => expect(bar.getAttribute("aria-valuenow")).toBe("50"))
    expect(screen.getByText(/Sending/)).toBeDefined()
    expect(next().disabled).toBe(true)

    release!()
    await waitFor(() => expect(bar.getAttribute("aria-valuenow")).toBe("100"))
    await screen.findByText(/Received/)
    await waitFor(() => expect(next().disabled).toBe(false))
  })

  it("sends an .xlsx instead of refusing it by name, and lands on a Check step with a sheet picker", async () => {
    const calls = stubFetch({
      complete: (id) => json({ ...upload({ id, originalFilename: "stock.xlsx" }), workbook: WORKBOOK }),
      preview: (q) => workbookPreview(q.get("sheet") ?? WORKBOOK.defaultSheet),
    })
    render(<UploadFilePage />)
    chooseFile(new File(["x"], "stock.xlsx"))
    expect(screen.queryByRole("alert")).toBeNull()
    await waitFor(() => expect(next().disabled).toBe(false))
    fireEvent.click(next())

    await screen.findByLabelText("Sheet")
    expect(sent(calls, "POST", /\/api\/uploads\/sessions$/).length).toBe(1)
  })

  it("sends a .parquet instead of refusing it by name, and lands on a Check step without controls", async () => {
    stubFetch({
      complete: (id) => json({ ...upload({ id, originalFilename: "orders.parquet" }), parquet: PARQUET }),
      preview: () => parquetPreview(),
    })
    render(<UploadFilePage />)
    chooseFile(new File(["x"], "orders.parquet"))
    expect(screen.queryByRole("alert")).toBeNull()
    await waitFor(() => expect(next().disabled).toBe(false))
    fireEvent.click(next())

    await screen.findByRole("columnheader", { name: /sku/ })
  })

  it("does not show a proxy's plain-text 413 as such: it says the upload did not reach the service and names the limit", async () => {
    stubFetch({ session: () => new Response("Request Entity Too Large", { status: 413 }) })
    render(<UploadFilePage />)
    chooseFile(CSV())

    const alert = await screen.findByRole("alert")
    expect(alert.textContent).toContain("did not reach the service")
    expect(alert.textContent).toContain("413")
    expect(alert.textContent).toContain("2 GB")
    expect(alert.textContent).not.toContain("Request Entity Too Large")
  })

  it("keeps trying when a proxy answers 500: the file waits for the connection, with no alert, and is not refused", async () => {
    stubFetch({ part: () => new Response("Internal Server Error", { status: 500 }) })
    render(<UploadFilePage />)
    chooseFile(CSV())

    expect(await screen.findByText(/Waiting for the connection/)).toBeDefined()
    expect(screen.queryByRole("alert")).toBeNull()
    expect(screen.queryByText(/Internal Server Error/)).toBeNull()
    expect(next().disabled).toBe(true)
  })

  it("waits as the API asks when it has no free place, then sends again by itself", async () => {
    let refused = true
    stubFetch({
      part: (_id, n) => {
        if (refused) {
          refused = false
          return new Response(JSON.stringify({ error: TOO_MANY_UPLOADS }), {
            status: 429,
            headers: { "Content-Type": "application/json", "Retry-After": "0" },
          })
        }
        return json({ partNumber: n, sizeBytes: 1 })
      },
    })
    render(<UploadFilePage />)
    chooseFile(CSV())

    await screen.findByText(/Received/)
    expect(screen.queryByRole("alert")).toBeNull()
    expect(next().disabled).toBe(false)
  })

  it("carries on with the first file that arrives and moves to the Check step with the repeated-file notice", async () => {
    const calls = stubFetch({
      complete: (id) =>
        json({ ...upload({ id }), duplicateOf: upload({ id: "up-0", originalFilename: "stock_old.csv", bronzeTable: "stock" }) }),
      preview: () => preview(),
    })
    render(<UploadFilePage />)
    chooseFile(CSV())
    await waitFor(() => expect(next().disabled).toBe(false))
    fireEvent.click(next())

    await screen.findByLabelText("Encoding")
    // The order of the requests: open, one piece, complete.
    const writes = calls.filter((c) => c.method !== "GET" && c.url.startsWith("/api/uploads/"))
    expect(writes.map((c) => `${c.method} ${c.url}`)).toEqual([
      "POST /api/uploads/sessions",
      "PUT /api/uploads/up-1/parts/1",
      "POST /api/uploads/up-1/complete",
    ])
    expect(writes[1].body).toBeInstanceOf(Blob)
    const notice = screen.getByText(/This file was uploaded before/)
    expect(notice.textContent).toContain("stock_old.csv")
    expect(notice.textContent).toMatch(/Oct.*2026|2026.*Oct/)
  })

  it("lists the other files on the Check step: being sent first, then compared with the first one", async () => {
    let release: (() => void) | null = null
    const held = new Promise<void>((resolve) => {
      release = resolve
    })
    const calls = stubFetch({
      part: async (id, n) => {
        // The second file's piece is held back.
        if (id === "up-2") await held
        return json({ partNumber: n, sizeBytes: 1 })
      },
      preview: () => preview(),
    })
    render(<UploadFilePage />)
    chooseFiles([CSV(), new File(["qty,sku\n1,a\n"], "second.csv")])
    await waitFor(() => expect(next().disabled).toBe(false))
    fireEvent.click(next())
    await screen.findByLabelText("Encoding")

    const list = screen.getByRole("list", { name: "Files in this load" })
    expect(within(list).getByText("Being sent")).toBeDefined()
    expect(screen.getByText(/Load waits until it arrives/)).toBeDefined()
    expect(next().disabled).toBe(true)
    expect(sent(calls, "POST", /compare$/)).toEqual([])
    release!()
    await within(list).findByText("Same columns")
    expect(sent(calls, "POST", /\/api\/uploads\/up-1\/compare$/)[0].body).toMatchObject({
      alsoUploads: ["up-2"],
      encoding: "utf-8",
      delimiter: ",",
      headerRow: 0,
    })
    await waitFor(() => expect(next().disabled).toBe(false))
  })

  it("takes a file out of the list, and removes the unfinished upload from the API", async () => {
    let release: (() => void) | null = null
    const held = new Promise<void>((resolve) => {
      release = resolve
    })
    const calls = stubFetch({
      part: async (_id, n) => {
        await held
        return json({ partNumber: n, sizeBytes: 1 })
      },
    })
    render(<UploadFilePage />)
    chooseFile(CSV())
    await screen.findByRole("progressbar", { name: "stock.csv sent" })
    await waitFor(() => expect(sent(calls, "PUT", /parts\/1$/).length).toBe(1))

    fireEvent.click(screen.getByRole("button", { name: "Remove stock.csv" }))
    expect(screen.queryByText("stock.csv")).toBeNull()
    await waitFor(() => expect(sent(calls, "DELETE", /\/api\/uploads\/up-1$/).length).toBe(1))
    release!()
  })

  it("shows a refusal for a missing permission in the file's row, in the API's sentence", async () => {
    stubFetch({ session: () => json({ error: "You do not have permission to do this." }, 403) })
    render(<UploadFilePage />)
    chooseFile(CSV())
    const alert = await screen.findByRole("alert")
    expect(alert.textContent).toBe("You do not have permission to do this.")
    expect(next().disabled).toBe(true)
  })
})

describe("UploadFilePage, Check step", () => {
  async function openCheck(routes: Routes) {
    url.search = "?id=up-1"
    const calls = stubFetch({ get: () => upload(), ...routes })
    render(<UploadFilePage />)
    await screen.findByLabelText("Encoding")
    return calls
  }

  it("shows what was detected and what is in force, and reads again when a setting changes", async () => {
    const calls = await openCheck({
      preview: (q) =>
        q.get("delimiter") === ";"
          ? preview({ using: { ...OPTIONS, delimiter: ";" }, columns: ["sku,qty"], rows: [["a1,3"]] })
          : preview(),
    })
    expect(screen.getAllByText("Detected: Comma").length).toBe(1)
    expect(screen.getByText("Detected: UTF-8")).toBeDefined()
    expect((screen.getByLabelText("Delimiter") as HTMLSelectElement).value).toBe(",")
    expect((screen.getByLabelText("Header row") as HTMLInputElement).value).toBe("1")
    expect(screen.getByRole("columnheader", { name: "sku" })).toBeDefined()
    // The first read sends nothing: the API detects.
    expect(calls.find((c) => c.url.includes("/preview"))?.url).toBe("/api/uploads/up-1/preview")

    fireEvent.change(screen.getByLabelText("Delimiter"), { target: { value: ";" } })
    await screen.findByRole("columnheader", { name: "sku,qty" })
    expect(calls.some((c) => c.url.includes("delimiter=%3B"))).toBe(true)
    // What was detected stays what was detected.
    expect(screen.getByText("Detected: Comma")).toBeDefined()

    fireEvent.change(screen.getByLabelText("Delimiter"), { target: { value: "\t" } })
    await waitFor(() => expect(calls.some((c) => c.url.includes("delimiter=%09"))).toBe(true))
  })

  it("shows the header row from 1 and sends it from 0, and refuses a number that is not one", async () => {
    const calls = await openCheck({ preview: () => preview() })
    const field = screen.getByLabelText("Header row") as HTMLInputElement

    fireEvent.change(field, { target: { value: "3" } })
    await waitFor(() => expect(calls.some((c) => c.url.includes("headerRow=2"))).toBe(true))

    const before = calls.length
    fireEvent.change(field, { target: { value: "0" } })
    expect(screen.getByText("Enter a whole number, 1 or more.")).toBeDefined()
    expect(next().disabled).toBe(true)
    expect(calls.length).toBe(before)
  })

  it("says so and keeps Next disabled when the header row has no columns", async () => {
    await openCheck({ preview: () => preview({ columns: [], rows: [] }) })
    expect(screen.getByRole("alert").textContent).toContain("The header row has no columns")
    expect(next().disabled).toBe(true)
  })

  it("says when the file is longer than the rows shown", async () => {
    await openCheck({ preview: () => preview({ truncated: true }) })
    expect(screen.getByText("Showing the first 2 rows. The file has more.")).toBeDefined()
    expect(next().disabled).toBe(false)
  })

  it("frames the preview: a line with the columns and the rows shown, a row-number gutter and a header that stays", async () => {
    await openCheck({ preview: () => preview({ rows: [["a1", "3"], ["b2", "5"], ["c3", "7"]] }) })
    expect(screen.getByText("2 columns")).toBeDefined()
    expect(screen.getByText("Showing all 3 rows.")).toBeDefined()
    const frame = screen.getByRole("region", { name: "Preview of the first rows" })
    expect(frame.getAttribute("tabindex")).toBe("0")
    expect(frame.parentElement?.className).toContain("isolate")
    const gutter = screen.getAllByRole("rowheader").map((cell) => cell.textContent)
    expect(gutter).toEqual(["1", "2", "3"])
    expect(screen.getByRole("columnheader", { name: "sku" }).className).toContain("sticky")
    expect(screen.getByRole("columnheader", { name: "Row number" })).toBeDefined()
  })

  it("says one column in the singular", async () => {
    await openCheck({ preview: () => preview({ columns: ["sku"], rows: [["a1"]] }) })
    expect(screen.getByText("1 column")).toBeDefined()
  })

  it("shows an empty cell in the quiet tone and a cell a short row lacks as a dash", async () => {
    await openCheck({ preview: () => preview({ rows: [["a1", ""], ["b2"]] }) })
    const empty = screen.getByText("(empty)")
    expect(empty.className).toContain("italic")
    expect(empty.className).toContain("text-muted-foreground")
    const missing = screen.getByText("—")
    expect(missing.className).toContain("italic")
    expect(missing.parentElement?.getAttribute("title")).toBe("No cell in this row")
  })

  it("marks a row with more cells than the header, names it above the frame and draws no nameless column", async () => {
    await openCheck({ preview: () => preview({ rows: [["a1", "3"], ["b2", "5", "extra"]] }) })
    const notice = screen.getByText(/Row 2 has more cells than the header \(2\)/)
    expect(notice.textContent).toContain("The extra cells are not shown here.")
    expect(notice.closest("[role=status]")).not.toBeNull()
    expect(screen.getByRole("img", { name: "This row has 3 cells; the header has 2" })).toBeDefined()
    expect(within(screen.getAllByRole("table")[0]).getAllByRole("columnheader").length).toBe(3)
    expect(screen.queryByText("extra")).toBeNull()
  })

  it("names several marked rows, and still shows the header when there are no rows below it", async () => {
    await openCheck({ preview: () => preview({ rows: [["a", "b", "c"], ["d", "e"], ["f", "g", "h"]] }) })
    expect(screen.getByText(/Rows 1, 3 have more cells than the header/)).toBeDefined()
    cleanup()
    await openCheck({ preview: () => preview({ rows: [] }) })
    expect(screen.getByText("There are no rows below the header row.")).toBeDefined()
    expect(screen.getByRole("columnheader", { name: "sku" })).toBeDefined()
  })

  it("keeps the hints under the controls to one quiet line each", async () => {
    await openCheck({ preview: () => preview() })
    expect(screen.getByText("Detected: 1. Counted from the top of the file, blank rows included.")).toBeDefined()
  })

  it("shows the API's refusal of a preview, with Retry", async () => {
    url.search = "?id=up-1"
    const sentence = "Upload storage is unavailable (timeout)."
    stubFetch({ get: () => upload(), preview: () => json({ error: sentence }, 503) })
    render(<UploadFilePage />)
    const alert = await screen.findByRole("alert")
    expect(alert.textContent).toBe(sentence)
    expect(screen.getByRole("button", { name: "Retry" })).toBeDefined()
  })
})

const WORKBOOK = {
  sheets: [
    { name: "Stock", visible: true },
    { name: "Quirks", visible: true },
    { name: "Notes", visible: false },
  ],
  defaultSheet: "Stock",
}

/** What the API answers for a workbook's preview: the fixed dialect, and the sheet read. */
function workbookPreview(sheet: string, over: Partial<UploadPreview> = {}): UploadPreview {
  return preview({
    columns: sheet === "Quirks" ? ["case", "value"] : ["sku", "qty"],
    workbook: { ...WORKBOOK, sheet },
    ...over,
  })
}

describe("UploadFilePage, Check step for a workbook", () => {
  async function openWorkbook(routes: Routes = {}) {
    url.search = "?id=up-1"
    const calls = stubFetch({
      get: () => upload({ originalFilename: "stock.xlsx" }),
      preview: (q) => workbookPreview(q.get("sheet") ?? WORKBOOK.defaultSheet),
      ...routes,
    })
    render(<UploadFilePage />)
    await screen.findByLabelText("Sheet")
    return calls
  }

  it("shows a sheet picker with the default chosen and no encoding or delimiter, and says how cells are written", async () => {
    await openWorkbook()
    expect((screen.getByLabelText("Sheet") as HTMLSelectElement).value).toBe("Stock")
    expect(screen.queryByLabelText("Encoding")).toBeNull()
    expect(screen.queryByLabelText("Delimiter")).toBeNull()
    expect(screen.getByLabelText("Header row")).toBeDefined()
    expect(screen.getByRole("columnheader", { name: "sku" })).toBeDefined()
    const note = screen.getByText(/Numbers are read as the cell holds them/)
    expect(note.textContent).toContain("2025-09-24")
    expect(screen.getByText("Notes (hidden)")).toBeDefined()
  })

  it("reads the chosen sheet and drops the header row chosen for the previous one", async () => {
    const calls = await openWorkbook()
    fireEvent.change(screen.getByLabelText("Header row"), { target: { value: "3" } })
    await waitFor(() => expect(calls.some((c) => c.url.includes("headerRow=2"))).toBe(true))

    fireEvent.change(screen.getByLabelText("Sheet"), { target: { value: "Quirks" } })
    await screen.findByRole("columnheader", { name: "case" })
    const last = calls.filter((c) => c.url.includes("/preview")).at(-1)
    expect(last?.url).toContain("sheet=Quirks")
    expect(last?.url).not.toContain("headerRow")
  })

  it("sends the sheet shown when Load is pressed, and the review names it instead of an encoding", async () => {
    const started = upload({ status: "ingesting", bronzeTable: "stock", loadMode: "replace" })
    const calls = await openWorkbook({ ingest: json({ upload: started, runId: "run-1" }) })
    fireEvent.change(screen.getByLabelText("Sheet"), { target: { value: "Quirks" } })
    await screen.findByRole("columnheader", { name: "case" })
    await waitFor(() => expect(next().disabled).toBe(false))
    fireEvent.click(next())
    await screen.findByLabelText("Table name")
    fireEvent.click(next())
    await screen.findByRole("button", { name: "Load" })
    expect(screen.getByText("Sheet")).toBeDefined()
    expect(screen.getByText("Quirks")).toBeDefined()
    expect(screen.queryByText("Encoding")).toBeNull()

    fireEvent.click(screen.getByRole("button", { name: "Load" }))
    await waitFor(() => expect(calls.some((c) => c.url.endsWith("/ingest"))).toBe(true))
    const post = calls.find((c) => c.url.endsWith("/ingest"))
    // The columns as chosen go with it (here the API's guess left them as text), and a value that
    // does not fit stops the load unless the person chose otherwise (DATA-18 decision 6).
    expect(post?.body).toMatchObject({
      encoding: "utf-8",
      delimiter: ",",
      headerRow: 0,
      sheet: "Quirks",
      bronzeTable: "stock",
      mode: "replace",
      onBadValue: "stop",
    })
    expect((post?.body as { columns: unknown[] }).columns.length).toBeGreaterThan(0)
  })
})

const PARQUET = {
  columns: [
    { name: "sku", type: "string" },
    { name: "qty", type: "int64" },
    { name: "price", type: "decimal(10,2)" },
  ],
  rows: 3000,
}

/** What the API answers for a Parquet file's preview: the fixed dialect, the header at row 1, and the declared types. */
function parquetPreview(over: Partial<UploadPreview> = {}): UploadPreview {
  return preview({
    columns: ["sku", "qty", "price"],
    rows: [
      ["a1", "3", "1.50"],
      ["b2", "5", "12.00"],
    ],
    truncated: true,
    parquet: PARQUET,
    ...over,
  })
}

describe("UploadFilePage, Check step for a Parquet file", () => {
  async function openParquet(routes: Routes = {}) {
    url.search = "?id=up-1"
    const calls = stubFetch({
      get: () => upload({ originalFilename: "orders.parquet" }),
      preview: () => parquetPreview(),
      ...routes,
    })
    render(<UploadFilePage />)
    await screen.findByRole("columnheader", { name: /sku/ })
    return calls
  }

  it("shows no sheet, encoding, delimiter or header-row control, and says each column starts with the type the file declares", async () => {
    await openParquet()
    expect(screen.queryByLabelText("Sheet")).toBeNull()
    expect(screen.queryByLabelText("Encoding")).toBeNull()
    expect(screen.queryByLabelText("Delimiter")).toBeNull()
    expect(screen.queryByLabelText("Header row")).toBeNull()
    const note = screen.getByText(/This is a Parquet file/)
    expect(note.textContent).toContain("each column starts with the type the file declares")
  })

  it("shows each column's declared type under its name, and counts the rows against the file's total", async () => {
    await openParquet()
    expect(screen.getByText("was: string")).toBeDefined()
    expect(screen.getByText("was: int64")).toBeDefined()
    expect(screen.getByText("was: decimal(10,2)")).toBeDefined()
    expect(screen.getByText("Showing the first 2 rows of 3,000.")).toBeDefined()
    await waitFor(() => expect(next().disabled).toBe(false))
  })

  it("sends the settings the preview used when Load is pressed (the API ignores them for Parquet), and the review names the format", async () => {
    const started = upload({ status: "ingesting", bronzeTable: "orders", loadMode: "replace" })
    const calls = await openParquet({ ingest: json({ upload: started, runId: "run-1" }) })
    await waitFor(() => expect(next().disabled).toBe(false))
    fireEvent.click(next())
    await screen.findByLabelText("Table name")
    fireEvent.click(next())
    await screen.findByRole("button", { name: "Load" })
    expect(screen.getByText("Format")).toBeDefined()
    expect(screen.getByText("Parquet")).toBeDefined()
    expect(screen.getByText("3,000")).toBeDefined()
    expect(screen.queryByText("Encoding")).toBeNull()
    expect(screen.queryByText("Header row")).toBeNull()
    expect(screen.queryByText("Sheet")).toBeNull()

    fireEvent.click(screen.getByRole("button", { name: "Load" }))
    await waitFor(() => expect(calls.some((c) => c.url.endsWith("/ingest"))).toBe(true))
    const post = calls.find((c) => c.url.endsWith("/ingest"))
    expect(post?.body).toMatchObject({
      encoding: "utf-8",
      delimiter: ",",
      headerRow: 0,
      bronzeTable: "orders",
      mode: "replace",
      onBadValue: "stop",
    })
  })
})

describe("UploadFilePage, Check step for JSON, Avro, gzip and zip (DATA-17 slice 4)", () => {
  async function open(name: string, over: Partial<UploadPreview>) {
    url.search = "?id=up-1"
    stubFetch({
      get: () => upload({ originalFilename: name }),
      preview: () => preview({ columns: ["id", "name", "meta"], rows: [["1", "Anna", '{"x":1}']], ...over }),
    })
    render(<UploadFilePage />)
    await screen.findByRole("columnheader", { name: /id/ })
  }

  it("shows no dialect control for a JSON file and says a nested value is kept as JSON text", async () => {
    await open("orders.json", { layout: { format: "json", compression: "none" } })
    expect(screen.queryByLabelText("Encoding")).toBeNull()
    expect(screen.queryByLabelText("Delimiter")).toBeNull()
    expect(screen.queryByLabelText("Header row")).toBeNull()
    const note = screen.getByText(/This is a JSON file/)
    expect(note.textContent).toContain("kept as JSON text in one column")
    await waitFor(() => expect(next().disabled).toBe(false))
  })

  it("says a JSON Lines file is one, and an Avro file starts each column at the type its schema declares", async () => {
    await open("orders.jsonl", { layout: { format: "jsonl", compression: "none" } })
    expect(screen.getByText(/This is a JSON Lines file/)).toBeDefined()
    cleanup()
    await open("orders.avro", { layout: { format: "avro", compression: "none" } })
    expect(screen.getByText(/This is an Avro file/).textContent).toContain("the type the schema declares")
  })

  it("keeps the dialect controls for delimited text in a gzip file and says it is gzip", async () => {
    await open("orders.csv.gz", { layout: { format: "delimited", compression: "gzip" } })
    expect(screen.getByLabelText("Encoding")).toBeDefined()
    expect(screen.getByLabelText("Delimiter")).toBeDefined()
    expect(screen.getByText(/compressed with gzip/).textContent).toContain("counts toward the 2 GB limit")
  })

  it("lists the files of a zip, and turns Next off with the reason when one has other columns", async () => {
    const file = (name: string, matchesFirst: boolean) => ({ name, sizeBytes: 2048, columns: 3, matchesFirst })
    await open("three.zip", {
      layout: { format: "delimited", compression: "zip" },
      archive: { files: [file("a.csv", true), file("sub/b.csv", false), file("c.csv", true)] },
    })
    const list = screen.getByRole("list", { name: "Files in the zip" })
    expect(list.querySelectorAll("li").length).toBe(3)
    expect(list.textContent).toContain("sub/b.csv")
    expect(list.textContent).toContain("Different columns")
    expect(list.textContent).toContain("2.0 KB · 3 columns")
    expect(screen.getByRole("alert").textContent).toContain("One file of the zip does not have the same columns as the first")
    expect(next().disabled).toBe(true)
  })

  it("lets a zip whose files agree go on, and the review names the format and the files", async () => {
    const file = (name: string) => ({ name, sizeBytes: 10, columns: 3, matchesFirst: true })
    await open("three.zip", {
      layout: { format: "delimited", compression: "zip" },
      archive: { files: [file("a.csv"), file("b.csv")] },
    })
    await waitFor(() => expect(next().disabled).toBe(false))
    fireEvent.click(next())
    await screen.findByLabelText("Table name")
    fireEvent.click(next())
    await screen.findByRole("button", { name: "Load" })
    expect(screen.getByText("CSV or TSV in a zip archive")).toBeDefined()
    expect(screen.getByText("Files in the zip")).toBeDefined()
  })
})

describe("UploadFilePage, Table step", () => {
  async function openTable(list: Upload[]) {
    url.search = "?id=up-1"
    const calls = stubFetch({ get: () => upload(), list, preview: () => preview() })
    render(<UploadFilePage />)
    await screen.findByLabelText("Encoding")
    await waitFor(() => expect(next().disabled).toBe(false))
    fireEvent.click(next())
    await screen.findByLabelText("Table name")
    return calls
  }

  it("suggests a name from the file and shows the naming rule", async () => {
    await openTable([])
    expect((screen.getByLabelText("Table name") as HTMLInputElement).value).toBe("stock")
    expect(screen.getByText(TABLE_NAME_RULE)).toBeDefined()
    expect(screen.getByText(/The columns keep the names and types chosen on the previous step/)).toBeDefined()
  })

  it("refuses a name that breaks the rule with the rule's sentence", async () => {
    await openTable([])
    fireEvent.change(screen.getByLabelText("Table name"), { target: { value: "Orders 2025" } })
    // The hint turns from the rule into the specific break, in the destructive tone.
    expect(screen.queryByText(TABLE_NAME_RULE)).toBeNull()
    expect(screen.getByText("Upper-case letters are not allowed: use lower case.").className).toContain("text-destructive")
    expect(screen.getByLabelText("Table name").getAttribute("aria-invalid")).toBe("true")
    expect(next().disabled).toBe(true)

    fireEvent.change(screen.getByLabelText("Table name"), { target: { value: "orders 2025" } })
    expect(screen.getByText("Spaces are not allowed: use an underscore.")).toBeDefined()
  })

  it("shows where the data will land, as Data Explorer names it, only for a name that passes", async () => {
    await openTable([])
    expect(screen.getByText("bronze.stock")).toBeDefined()
    fireEvent.change(screen.getByLabelText("Table name"), { target: { value: "stock_b" } })
    expect(screen.getByText("bronze.stock_b")).toBeDefined()
    fireEvent.change(screen.getByLabelText("Table name"), { target: { value: "Stock B" } })
    expect(screen.queryByText(/^bronze\./)).toBeNull()
    expect(screen.getByText("Enter a valid name to see it.")).toBeDefined()
    fireEvent.change(screen.getByLabelText("Table name"), { target: { value: "" } })
    expect(screen.getByText("Enter a valid name to see it.")).toBeDefined()
  })

  it("says in a notice that the columns keep what was chosen, and that adding keeps the table's", async () => {
    await openTable([])
    const note = screen.getByText(/The columns keep the names and types chosen on the previous step/)
    expect(note.closest("[role=status]")).not.toBeNull()
  })

  it("offers Replace or Add only for a table an earlier upload loaded", async () => {
    await openTable([upload({ id: "up-0", status: "ingested", bronzeTable: "stock", assetId: "stock" })])
    const replace = screen.getByRole("radio", { name: "Replace its rows" }) as HTMLInputElement
    const add = screen.getByRole("radio", { name: "Add to its rows" }) as HTMLInputElement
    expect(replace.checked).toBe(true)
    expect(add.checked).toBe(false)

    fireEvent.change(screen.getByLabelText("Table name"), { target: { value: "stock_b" } })
    expect(screen.queryByRole("radio", { name: "Replace its rows" })).toBeNull()
    expect(screen.queryByRole("radio", { name: "Add to its rows" })).toBeNull()
  })

  it("carries the choice to the review, and sends nothing before Load", async () => {
    const calls = await openTable([upload({ id: "up-0", status: "ingested", bronzeTable: "stock", assetId: "stock" })])
    fireEvent.click(screen.getByRole("radio", { name: "Add to its rows" }))
    fireEvent.click(next())
    expect(await screen.findByText("Add to its rows")).toBeDefined()
    expect(screen.getByText("Header row").nextSibling?.textContent).toBe("1")
    expect(screen.getByRole("button", { name: "Load" })).toBeDefined()
    expect(calls.filter((c) => c.method === "POST")).toEqual([])
  })
})

describe("UploadFilePage, Table step, folders (DATA-35)", () => {
  const FOLDERS = [folder({ id: "uf-sales", name: "Sales", childCount: 1 }), folder({ id: "uf-q1", name: "Q1", parentId: "uf-sales" })]

  async function openTable(routes: Routes) {
    url.search = "?id=up-1"
    const calls = stubFetch({ get: () => upload(), preview: () => preview(), ...routes })
    render(<UploadFilePage />)
    await screen.findByLabelText("Encoding")
    await waitFor(() => expect(next().disabled).toBe(false))
    fireEvent.click(next())
    await screen.findByLabelText("Table name")
    return calls
  }

  /** Goes on to Review and presses Load; returns the body of the ingest. */
  async function load(calls: Call[]) {
    fireEvent.click(next())
    fireEvent.click(await screen.findByRole("button", { name: "Load" }))
    await waitFor(() => expect(sent(calls, "POST", /\/ingest$/).length).toBe(1))
    return sent(calls, "POST", /\/ingest$/)[0]!.body as Record<string, unknown>
  }

  it("offers an optional folder, No folder by default, and sends no folder when it is left alone", async () => {
    const calls = await openTable({ folders: FOLDERS, ingest: json({ upload: upload({ status: "ingesting" }), runId: "run-1" }) })
    const picker = screen.getByRole("combobox", { name: "Folder (optional)" })
    expect(picker.textContent).toContain("No folder")
    const body = await load(calls)
    expect(body.folderId).toBeNull()
  })

  it("sends the folder that was chosen", async () => {
    const calls = await openTable({ folders: FOLDERS, ingest: json({ upload: upload({ status: "ingesting" }), runId: "run-1" }) })
    fireEvent.click(screen.getByRole("combobox", { name: "Folder (optional)" }))
    pickOption(await screen.findByRole("option", { name: "Sales / Q1" }))
    const body = await load(calls)
    expect(body.folderId).toBe("uf-q1")
  })

  it("makes a folder from the step, picks it, and sends it", async () => {
    const calls = await openTable({
      folders: FOLDERS,
      createFolder: (asked) => folder({ id: "uf-jan", name: asked.name }),
      ingest: json({ upload: upload({ status: "ingesting" }), runId: "run-1" }),
    })
    fireEvent.click(screen.getByRole("button", { name: "New folder" }))
    await screen.findByText("A folder groups uploaded files on the Uploaded files tab.")
    fireEvent.change(screen.getByLabelText("Folder name"), { target: { value: "January" } })
    fireEvent.click(screen.getByRole("button", { name: "Create" }))

    await waitFor(() => expect(screen.getByRole("combobox", { name: "Folder (optional)" }).textContent).toContain("January"))
    expect(sent(calls, "POST", /\/api\/uploads\/folders$/)[0]!.body).toEqual({ name: "January" })
    const body = await load(calls)
    expect(body.folderId).toBe("uf-jan")
  })

  it("shows the API's sentence when a folder is refused, and keeps the dialog open", async () => {
    await openTable({
      folders: FOLDERS,
      createFolder: () => json({ error: "A folder with that name already exists in the same place." }, 409),
    })
    fireEvent.click(screen.getByRole("button", { name: "New folder" }))
    await screen.findByLabelText("Folder name")
    fireEvent.change(screen.getByLabelText("Folder name"), { target: { value: "Sales" } })
    fireEvent.click(screen.getByRole("button", { name: "Create" }))
    expect((await screen.findByRole("alert")).textContent).toBe("A folder with that name already exists in the same place.")
  })

  it("preselects the folder of the newest upload loaded into an existing table, and sends it", async () => {
    const calls = await openTable({
      folders: FOLDERS,
      list: [upload({ id: "up-0", status: "ingested", bronzeTable: "stock", assetId: "stock", folderId: "uf-q1" })],
      ingest: json({ upload: upload({ status: "ingesting" }), runId: "run-1" }),
    })
    expect(screen.getByRole("combobox", { name: "Folder (optional)" }).textContent).toContain("Sales / Q1")
    const body = await load(calls)
    expect(body.folderId).toBe("uf-q1")
  })

  it("sends null when the person chooses No folder for a table that has one", async () => {
    const calls = await openTable({
      folders: FOLDERS,
      list: [upload({ id: "up-0", status: "ingested", bronzeTable: "stock", assetId: "stock", folderId: "uf-q1" })],
      ingest: json({ upload: upload({ status: "ingesting" }), runId: "run-1" }),
    })
    fireEvent.click(screen.getByRole("combobox", { name: "Folder (optional)" }))
    pickOption(await screen.findByRole("option", { name: "No folder" }))
    const body = await load(calls)
    expect(body.folderId).toBeNull()
  })

  it("offers no picker, and sends no folder, when the folders cannot be read", async () => {
    const calls = await openTable({
      folders: json({ error: "You do not have permission to do this." }, 403),
      ingest: json({ upload: upload({ status: "ingesting" }), runId: "run-1" }),
    })
    expect(screen.queryByRole("combobox", { name: "Folder (optional)" })).toBeNull()
    const body = await load(calls)
    expect("folderId" in body).toBe(false)
  })
})

describe("UploadFilePage, after Load", () => {
  /** Opens the page on up-1 and goes to Review; `ingest` is what the API answers to Load. */
  async function review(routes: Routes) {
    url.search = "?id=up-1"
    const calls = stubFetch({ preview: () => preview(), ...routes })
    render(<UploadFilePage pollMs={20} />)
    await screen.findByLabelText("Encoding")
    await waitFor(() => expect(next().disabled).toBe(false))
    fireEvent.click(next())
    await screen.findByLabelText("Table name")
    fireEvent.click(next())
    await screen.findByRole("button", { name: "Load" })
    return calls
  }

  const started = upload({ status: "ingesting", bronzeTable: "stock", parseOptions: OPTIONS, loadMode: "replace" })

  it("sends exactly what was confirmed, shows Loading, then Loaded with the rows, and stops polling", async () => {
    const calls = await review({
      get: (n) => (n === 1 ? upload() : n === 2 ? started : { ...started, status: "ingested", rows: 3, assetId: "stock" }),
      ingest: json({ upload: started, runId: "run-1" }),
    })
    fireEvent.click(screen.getByRole("button", { name: "Load" }))

    expect(await screen.findByText("Loading", { selector: "span" })).toBeDefined()
    const post = calls.find((c) => c.url.endsWith("/ingest"))
    expect(post?.method).toBe("POST")
    expect(post?.body).toMatchObject({ encoding: "utf-8", delimiter: ",", headerRow: 0, bronzeTable: "stock", mode: "replace", onBadValue: "stop" })
    expect((post?.body as { columns: Record<string, unknown>[] }).columns).toEqual([
      { source: "sku", name: "sku", type: "text" },
      { source: "qty", name: "qty", type: "text" },
    ])

    await screen.findByText("Loaded", { selector: "span" })
    expect(screen.getByText("3")).toBeDefined()
    const open = screen.getByText("Open in Data Explorer").closest("a")
    expect(open?.getAttribute("href")).toBe("/data/assets/stock")
    expect(screen.getByRole("button", { name: "Upload another file" })).toBeDefined()

    const settled = calls.length
    await new Promise((resolve) => setTimeout(resolve, 120))
    expect(calls.length).toBe(settled)
  })

  it("summarises the file, how it is read and the table by its full name, with one sentence saying what Load does", async () => {
    await review({ get: () => upload(), ingest: json({ upload: started, runId: "run-1" }) })
    const labels = screen.getAllByRole("term").map((term) => term.textContent)
    expect(labels).toEqual(["Name", "Size", "Uploaded", "Encoding", "Delimiter", "Header row", "Columns", "Table", "sku", "qty", "A value that does not fit"])
    const value = (label: string) =>
      screen.getAllByRole("term").find((term) => term.textContent === label)?.nextElementSibling?.textContent
    expect(value("Table")).toBe("bronze.stock")
    expect(value("Columns")).toBe("2")
    const sentence = screen.getByText(/^Load reads/)
    expect(sentence.textContent).toContain("stock.csv")
    expect(sentence.textContent).toContain("bronze.stock")
    expect(sentence.textContent).toContain("each column with the type listed above")
    expect(sentence.textContent).toContain("Any rows the table already holds are replaced by this file's.")
  })

  it("says in the Load sentence that the rows are added after the ones there when Add was chosen", async () => {
    await review({
      list: [upload({ id: "up-0", status: "ingested", bronzeTable: "stock", assetId: "stock" })],
      get: () => upload(),
      ingest: json({ upload: started, runId: "run-1" }),
    })
    fireEvent.click(screen.getByRole("button", { name: "Previous" }))
    fireEvent.click(await screen.findByRole("radio", { name: "Add to its rows" }))
    fireEvent.click(next())
    await screen.findByRole("button", { name: "Load" })
    expect(screen.getByText(/^Load reads/).textContent).toContain("this file's are added after them")
    expect(screen.getByText("Rows already in it").nextElementSibling?.textContent).toBe("Add to its rows")
  })

  it("shows the loaded view: status first, then the table and rows, then the next actions", async () => {
    await review({
      get: (n) => (n === 1 ? upload() : { ...started, status: "ingested", rows: 1234, assetId: "stock" }),
      ingest: json({ upload: started, runId: "run-1" }),
    })
    fireEvent.click(screen.getByRole("button", { name: "Load" }))
    await screen.findByText("Loaded", { selector: "span" })
    expect(screen.getByText("Load finished")).toBeDefined()
    expect(screen.getByText("bronze.stock")).toBeDefined()
    expect(screen.getByText("1,234")).toBeDefined()
    expect(screen.getByText("Replace its rows")).toBeDefined()
    expect(screen.getByText("stock.csv")).toBeDefined()
    expect(document.querySelector("[data-file-mark=delimited]")?.textContent).toBe("CSV")
    const open = screen.getByText("Open in Data Explorer").closest("a")
    expect(open?.getAttribute("href")).toBe("/data/assets/stock")
  })

  it("offers no action while loading, and Upload another file after a failure goes back to an empty first step", async () => {
    await review({
      get: (n) => (n === 1 ? upload() : n === 2 ? started : { ...started, status: "failed", error: "The load into the table failed." }),
      ingest: json({ upload: started, runId: "run-1" }),
    })
    fireEvent.click(screen.getByRole("button", { name: "Load" }))
    await screen.findByText("Load in progress")
    expect(screen.queryByRole("button", { name: "Upload another file" })).toBeNull()
    expect(screen.queryByRole("button", { name: "Try again" })).toBeNull()

    await screen.findByText("Load failed")
    expect(screen.getByText("The load into bronze.stock failed.")).toBeDefined()
    expect(screen.getByText("The load into the table failed.")).toBeDefined()
    expect(screen.getByRole("button", { name: "Try again" })).toBeDefined()
    expect(screen.getByRole("button", { name: "Change settings" })).toBeDefined()
    fireEvent.click(screen.getByRole("button", { name: "Upload another file" }))
    expect(await screen.findByText("Drop files here, or choose some")).toBeDefined()
  })

  it("sends append when Add to its rows was chosen for a table an upload created", async () => {
    const calls = await review({
      list: [upload({ id: "up-0", status: "ingested", bronzeTable: "stock", assetId: "stock" })],
      get: (n) => (n === 1 ? upload() : started),
      ingest: json({ upload: started, runId: "run-1" }),
    })
    // `review` stopped on Review; go back to the Table step to choose.
    fireEvent.click(screen.getByRole("button", { name: "Previous" }))
    fireEvent.click(await screen.findByRole("radio", { name: "Add to its rows" }))
    fireEvent.click(next())
    fireEvent.click(await screen.findByRole("button", { name: "Load" }))
    await waitFor(() => expect(calls.some((c) => c.url.endsWith("/ingest"))).toBe(true))
    expect(calls.find((c) => c.url.endsWith("/ingest"))?.body).toMatchObject({ bronzeTable: "stock", mode: "append" })
  })

  it("sends replace when Add to its rows was chosen and the name was then changed to a new table", async () => {
    const calls = await review({
      list: [upload({ id: "up-0", status: "ingested", bronzeTable: "stock", assetId: "stock" })],
      get: (n) => (n === 1 ? upload() : started),
      ingest: json({ upload: started, runId: "run-1" }),
    })
    fireEvent.click(screen.getByRole("button", { name: "Previous" }))
    fireEvent.click(await screen.findByRole("radio", { name: "Add to its rows" }))
    fireEvent.change(screen.getByLabelText("Table name"), { target: { value: "stock_b" } })
    fireEvent.click(next())
    fireEvent.click(await screen.findByRole("button", { name: "Load" }))
    await waitFor(() => expect(calls.some((c) => c.url.endsWith("/ingest"))).toBe(true))
    expect(calls.find((c) => c.url.endsWith("/ingest"))?.body).toMatchObject({ bronzeTable: "stock_b", mode: "replace" })
  })

  it("reads a loaded upload with no row count as Not measured, never 0", async () => {
    await review({
      get: (n) => (n === 1 ? upload() : { ...started, status: "ingested", assetId: "stock" }),
      ingest: json({ upload: started, runId: "run-1" }),
    })
    fireEvent.click(screen.getByRole("button", { name: "Load" }))
    await screen.findByText("Loaded", { selector: "span" })
    expect(screen.getByText("Not measured")).toBeDefined()
  })

  it("shows the API's reason when the load failed, and Try again loads with the same settings", async () => {
    const failed = { ...started, status: "failed", error: "The load into the table failed.", loadMode: "append" }
    const calls = await review({
      get: (n) => (n === 1 ? upload() : failed),
      ingest: json({ upload: started, runId: "run-1" }),
    })
    fireEvent.click(screen.getByRole("button", { name: "Load" }))

    await screen.findByText("Failed", { selector: "span" })
    expect(screen.getByText("The load into the table failed.")).toBeDefined()
    expect(screen.queryByText(/second time/)).toBeNull()

    const before = calls.filter((c) => c.url.endsWith("/ingest")).length
    fireEvent.click(screen.getByRole("button", { name: "Try again" }))
    await waitFor(() => expect(calls.filter((c) => c.url.endsWith("/ingest")).length).toBe(before + 1))
    const retry = calls.filter((c) => c.url.endsWith("/ingest")).at(-1)
    expect(retry?.body).toEqual({ ...OPTIONS, bronzeTable: "stock", mode: "append", folderId: null })
  })

  it("puts a retry in the folder the failed load used (DATA-35)", async () => {
    const failed = { ...started, status: "failed", error: "The load into the table failed.", folderId: "uf-q1" }
    const calls = await review({
      get: (n) => (n === 1 ? upload() : failed),
      ingest: json({ upload: started, runId: "run-1" }),
    })
    fireEvent.click(screen.getByRole("button", { name: "Load" }))
    await screen.findByText("Failed", { selector: "span" })

    fireEvent.click(screen.getByRole("button", { name: "Try again" }))
    await waitFor(() => expect(calls.filter((c) => c.url.endsWith("/ingest")).length).toBe(2))
    const retry = calls.filter((c) => c.url.endsWith("/ingest")).at(-1)
    expect(retry?.body).toEqual({ ...OPTIONS, bronzeTable: "stock", mode: "replace", folderId: "uf-q1" })
  })

  it("after a failure only at the catalog, says the rows are there and retries with Replace", async () => {
    const failed = { ...started, status: "failed", error: REGISTRATION_FAILED_REASON, loadMode: "append" }
    const calls = await review({
      get: (n) => (n === 1 ? upload() : failed),
      ingest: json({ upload: started, runId: "run-1" }),
    })
    fireEvent.click(screen.getByRole("button", { name: "Load" }))

    await screen.findByText(REGISTRATION_FAILED_REASON)
    const note = screen.getByText(/The rows are in the table/)
    expect(note.textContent).toContain("stock")
    expect(note.textContent).toContain('"Add to its rows" would add them a second time')
    expect(note.textContent).toContain('"Try again" loads with "Replace its rows"')

    fireEvent.click(screen.getByRole("button", { name: "Try again" }))
    await waitFor(() => expect(calls.filter((c) => c.url.endsWith("/ingest")).length).toBe(2))
    expect(calls.filter((c) => c.url.endsWith("/ingest")).at(-1)?.body).toEqual({
      ...OPTIONS,
      bronzeTable: "stock",
      mode: "replace",
      folderId: null,
    })
  })

  it("goes back to the Check step from Change settings", async () => {
    const failed = { ...started, status: "failed", error: "The header row has no columns." }
    await review({
      get: (n) => (n === 1 ? upload() : failed),
      ingest: json({ upload: started, runId: "run-1" }),
    })
    fireEvent.click(screen.getByRole("button", { name: "Load" }))
    await screen.findByText("Failed", { selector: "span" })
    fireEvent.click(screen.getByRole("button", { name: "Change settings" }))
    expect(await screen.findByLabelText("Encoding")).toBeDefined()
    expect(screen.getByText("The last load of this file failed.")).toBeDefined()
  })

  it("shows the API's refusal of the load itself, in place", async () => {
    const sentence = "A connector loads that table, so a file cannot be loaded into it."
    await review({
      get: () => upload(),
      ingest: json({ error: sentence }, 409),
    })
    fireEvent.click(screen.getByRole("button", { name: "Load" }))
    const alert = await screen.findByRole("alert")
    expect(alert.textContent).toBe(sentence)
    expect(screen.getByRole("button", { name: "Load" })).toBeDefined()
  })
})

describe("UploadFilePage, with ?id=", () => {
  it("skips the file step and shows the page's own error state for an upload that is not there", async () => {
    url.search = "?id=gone"
    stubFetch({ get: () => json({ error: "Upload not found." }, 404) })
    render(<UploadFilePage />)
    expect(await screen.findByText("Not found")).toBeDefined()
    expect(screen.getByText(/Upload not found\./)).toBeDefined()
    expect(screen.queryByLabelText("Choose files")).toBeNull()
  })

  it("opens an upload that is loading on its status", async () => {
    url.search = "?id=up-1"
    stubFetch({
      get: () => upload({ status: "ingesting", bronzeTable: "stock" }),
    })
    render(<UploadFilePage pollMs={20} />)
    expect(await screen.findByText("Loading", { selector: "span" })).toBeDefined()
    expect(screen.getByText(/Loading stock\.csv into stock/)).toBeDefined()
  })
})
// SEC-17: the sentences the API answers with for a file with too many columns
// and for too many uploads at once. The console shows them as they are.
const TOO_MANY_COLUMNS = "The file has more than 1,000 columns."
const TOO_MANY_UPLOADS = "Too many uploads are in progress. Try again in a moment."

describe("UploadFilePage, limits", () => {
  it("shows the column-cap sentence on the Check step and keeps the delimiter, encoding and header row usable", async () => {
    url.search = "?id=up-1"
    const calls = stubFetch({
      get: () => upload(),
      preview: (q) =>
        q.get("delimiter") === ";"
          ? preview({ using: { ...OPTIONS, delimiter: ";" }, columns: ["sku;qty"], rows: [["a1;3"]] })
          : json({ error: TOO_MANY_COLUMNS }, 400),
    })
    render(<UploadFilePage />)

    const alert = await screen.findByRole("alert")
    expect(alert.textContent).toBe(TOO_MANY_COLUMNS)
    // Nothing was read, so the controls have no value yet, but they are there and not disabled.
    const delimiter = screen.getByLabelText("Delimiter") as HTMLSelectElement
    expect(delimiter.disabled).toBe(false)
    expect((screen.getByLabelText("Encoding") as HTMLSelectElement).disabled).toBe(false)
    expect((screen.getByLabelText("Header row") as HTMLInputElement).disabled).toBe(false)
    expect(next().disabled).toBe(true)

    fireEvent.change(delimiter, { target: { value: ";" } })
    await screen.findByRole("columnheader", { name: "sku;qty" })
    expect(calls.some((c) => c.url.includes("delimiter=%3B"))).toBe(true)
    expect(screen.queryByText(TOO_MANY_COLUMNS)).toBeNull()
    expect(next().disabled).toBe(false)
  })

  it("keeps the shown preview and the controls when a later setting makes the file too wide", async () => {
    url.search = "?id=up-1"
    stubFetch({
      get: () => upload(),
      preview: (q) => (q.get("delimiter") === "|" ? json({ error: TOO_MANY_COLUMNS }, 400) : preview()),
    })
    render(<UploadFilePage />)
    await screen.findByRole("columnheader", { name: "sku" })

    fireEvent.change(screen.getByLabelText("Delimiter"), { target: { value: "|" } })

    const alert = await screen.findByRole("alert")
    expect(alert.textContent).toBe(TOO_MANY_COLUMNS)
    expect((screen.getByLabelText("Delimiter") as HTMLSelectElement).disabled).toBe(false)
    expect(next().disabled).toBe(true)
  })

  it("shows the too-many-uploads sentence for a load that was not started and leaves Load available", async () => {
    url.search = "?id=up-1"
    const calls = stubFetch({
      get: () => upload(),
      preview: () => preview(),
      ingest: json({ error: TOO_MANY_UPLOADS }, 429),
    })
    render(<UploadFilePage pollMs={20} />)
    await screen.findByLabelText("Encoding")
    await waitFor(() => expect(next().disabled).toBe(false))
    fireEvent.click(next())
    await screen.findByLabelText("Table name")
    fireEvent.click(next())
    fireEvent.click(await screen.findByRole("button", { name: "Load" }))

    const alert = await screen.findByRole("alert")
    expect(alert.textContent).toBe(TOO_MANY_UPLOADS)
    expect((screen.getByRole("button", { name: "Load" }) as HTMLButtonElement).disabled).toBe(false)
    fireEvent.click(screen.getByRole("button", { name: "Load" }))
    await waitFor(() =>
      expect(calls.filter((c) => c.method === "POST" && c.url.endsWith("/ingest")).length).toBe(2)
    )
  })
})


// ── Column types (DATA-18 T3.5) ──────────────────────────────────────────

function typedColumns(): UploadPreview["columnSpecs"] {
  return [
    { source: "SKU", name: "sku", type: "text", guessedType: "text", askDateOrder: false },
    { source: "Qty", name: "qty", type: "integer", guessedType: "integer", askDateOrder: false },
    { source: "Due", name: "due", type: "date", guessedType: "date", askDateOrder: false, dateFormat: "iso" },
  ]
}

describe("UploadFilePage, column types", () => {
  async function openColumns(routes: Routes = {}, specs = typedColumns()) {
    url.search = "?id=up-1"
    const calls = stubFetch({
      get: () => upload(),
      preview: () => preview({ columns: ["SKU", "Qty", "Due"], columnSpecs: specs, sampledRows: 3 }),
      check: () => ({ sampledRows: 3, count: 0, byColumn: [], rows: [] }),
      ...routes,
    })
    render(<UploadFilePage />)
    await screen.findByLabelText("Type of qty")
    return calls
  }

  it("shows each column's name and guessed type, and says how many rows the guess read", async () => {
    await openColumns()
    expect((screen.getByLabelText("Name of column 2") as HTMLInputElement).value).toBe("qty")
    expect((screen.getByLabelText("Type of qty") as HTMLSelectElement).value).toBe("integer")
    expect((screen.getByLabelText("Type of due") as HTMLSelectElement).value).toBe("date")
    expect((screen.getByLabelText("How the dates of due are read", { selector: "select" }) as HTMLSelectElement).value).toBe("iso")
    expect(screen.getByText(/Types guessed from the first 3 rows/)).toBeDefined()
    // The six types a person chooses among.
    const options = Array.from((screen.getByLabelText("Type of sku") as HTMLSelectElement).options).map((o) => o.text)
    expect(options).toEqual(["Text", "Integer", "Decimal", "Boolean", "Date", "Timestamp"])
    await waitFor(() => expect(next().disabled).toBe(false))
  })

  it("lists up to 100 failing values after a change of type, with stop pre-selected, and sends the choice with Load", async () => {
    const calls = await openColumns({
      check: (body) =>
        body.columns[0].type === "integer"
          ? { sampledRows: 3, count: 2, byColumn: [{ column: "sku", count: 2 }], rows: [{ row: 1, column: "sku", value: "A-1" }, { row: 3, column: "sku", value: "B-2" }] }
          : { sampledRows: 3, count: 0, byColumn: [], rows: [] },
      ingest: json({ upload: upload({ status: "ingesting" }), runId: "run-1" }),
    })
    fireEvent.change(screen.getByLabelText("Type of sku"), { target: { value: "integer" } })

    await screen.findByText(/2 values do not fit/, {}, { timeout: 2000 })
    const panel = screen.getByLabelText("Values that do not fit")
    expect(panel.textContent).toContain("2 values do not fit in the first 3 rows")
    expect(within(panel).getByText("A-1")).toBeDefined()
    expect(screen.getByText("Guessed: Text")).toBeDefined()
    const stop = screen.getByLabelText(/Stop the load/) as HTMLInputElement
    expect(stop.checked).toBe(true)
    const checkCall = calls.find((c) => c.url.endsWith("/check"))
    expect((checkCall?.body as { columns: { type: string }[] }).columns[0].type).toBe("integer")

    fireEvent.click(screen.getByLabelText(/Load them as empty/))
    fireEvent.click(next())
    await screen.findByLabelText("Table name")
    fireEvent.click(next())
    fireEvent.click(await screen.findByRole("button", { name: "Load" }))
    await waitFor(() => expect(calls.some((c) => c.url.endsWith("/ingest"))).toBe(true))
    const body = calls.find((c) => c.url.endsWith("/ingest"))?.body as { columns: Record<string, unknown>[]; onBadValue: string }
    expect(body.onBadValue).toBe("null")
    expect(body.columns).toEqual([
      { source: "SKU", name: "sku", type: "integer" },
      { source: "Qty", name: "qty", type: "integer" },
      { source: "Due", name: "due", type: "date", dateFormat: "iso" },
    ])
  })

  it("keeps Next off until 'day first or month first?' is answered", async () => {
    const asking = typedColumns()
    asking[2] = { source: "Due", name: "due", type: "date", guessedType: "date", askDateOrder: true }
    await openColumns({}, asking)
    expect(next().disabled).toBe(true)
    expect(screen.getByText(/Say how the dates of due are read: day first or month first/)).toBeDefined()
    const reading = screen.getByLabelText("How the dates of due are read", { selector: "select" }) as HTMLSelectElement
    expect(reading.options[0].text).toBe("Day first or month first?")
    fireEvent.change(reading, { target: { value: "dmy" } })
    await waitFor(() => expect(next().disabled).toBe(false))
    expect(screen.queryByText(/Say how the dates of due are read/)).toBeNull()
  })

  it("refuses a column name that breaks the rule while it is typed, and sends the new name when it is right", async () => {
    const calls = await openColumns({ ingest: json({ upload: upload({ status: "ingesting" }), runId: "run-1" }) })
    const name = screen.getByLabelText("Name of column 1") as HTMLInputElement
    fireEvent.change(name, { target: { value: "Item Code" } })
    expect(await screen.findByText(/Column names start with a lower-case letter/)).toBeDefined()
    expect(next().disabled).toBe(true)
    fireEvent.change(name, { target: { value: "qty" } })
    expect(next().disabled).toBe(true) // used twice
    fireEvent.change(name, { target: { value: "item_code" } })
    await waitFor(() => expect(next().disabled).toBe(false))
    fireEvent.click(next())
    await screen.findByLabelText("Table name")
    fireEvent.click(next())
    fireEvent.click(await screen.findByRole("button", { name: "Load" }))
    await waitFor(() => expect(calls.some((c) => c.url.endsWith("/ingest"))).toBe(true))
    const body = calls.find((c) => c.url.endsWith("/ingest"))?.body as { columns: { name: string }[] }
    expect(body.columns.map((c) => c.name)).toEqual(["item_code", "qty", "due"])
  })

  it("shows the names and types of a table the file is added to, and does not let them be changed", async () => {
    url.search = "?id=up-1"
    stubFetch({
      get: () => upload(),
      list: [upload({ id: "up-0", status: "ingested", bronzeTable: "stock", assetId: "stock" })],
      preview: (q) =>
        preview({
          columns: ["SKU", "Qty", "Due"],
          columnSpecs: typedColumns().map((c) => (q.get("table") ? { ...c, guessedType: "text" } : c)),
          fromTable: q.get("table") !== null,
          sampledRows: 3,
        }),
      check: () => ({ sampledRows: 3, count: 0, byColumn: [], rows: [] }),
    })
    render(<UploadFilePage />)
    await screen.findByLabelText("Type of qty")
    fireEvent.click(next())
    const name = await screen.findByLabelText("Table name")
    fireEvent.change(name, { target: { value: "stock" } })
    fireEvent.click(await screen.findByLabelText(/Add to its rows/))
    fireEvent.click(screen.getByRole("button", { name: "Previous" }))
    expect(await screen.findByText(/The names and types are the table's/)).toBeDefined()
    expect((screen.getByLabelText("Type of qty") as HTMLSelectElement).disabled).toBe(true)
    expect((screen.getByLabelText("Name of column 1") as HTMLInputElement).disabled).toBe(true)
  })

  it("shows the report of what did not fit on the run view: stopped, or loaded as empty", async () => {
    const bad = { count: 2, byColumn: [{ column: "qty", count: 2 }], rows: [{ row: 2, column: "qty", value: "abc" }] }
    const started = upload({ status: "ingesting", bronzeTable: "stock", parseOptions: OPTIONS, loadMode: "replace" })
    for (const [final, expected] of [
      [{ ...started, status: "failed", error: "Some values do not fit their column's type.", badValues: bad }, "The load stopped and wrote nothing."],
      [{ ...started, status: "ingested", rows: 5, assetId: "stock", badValues: bad }, "They were loaded as empty."],
    ] as const) {
      url.search = "?id=up-1"
      stubFetch({
        get: (n) => (n === 1 ? upload() : n === 2 ? started : final),
        preview: () => preview(),
        check: () => ({ sampledRows: 2, count: 0, byColumn: [], rows: [] }),
        ingest: json({ upload: started, runId: "run-1" }),
      })
      render(<UploadFilePage pollMs={20} />)
      await screen.findByLabelText("Encoding")
      await waitFor(() => expect(next().disabled).toBe(false))
      fireEvent.click(next())
      await screen.findByLabelText("Table name")
      fireEvent.click(next())
      fireEvent.click(await screen.findByRole("button", { name: "Load" }))
      const report = await screen.findByLabelText("Values that did not fit")
      expect(report.textContent).toContain("2 values did not fit their column's type.")
      expect(report.textContent).toContain(expected)
      expect(within(report).getByText("abc")).toBeDefined()
      cleanup()
    }
  })
})


describe("UploadFilePage, several files into one table", () => {
  const THIRD = () => new File(["sku,qty,notes\na,1,x\n"], "third.csv", { type: "text/csv" })
  const SECOND = () => new File(["qty,sku\n1,a\n"], "second.csv", { type: "text/csv" })

  /** Picks the files, goes to Check and waits for the comparison the API answers with `compare`. */
  async function openCheck(files: File[], routes: Routes = {}) {
    const calls = stubFetch({ preview: () => preview(), ...routes })
    render(<UploadFilePage pollMs={20} />)
    chooseFiles(files)
    await waitFor(() => expect(sent(calls, "POST", /complete$/).length).toBe(files.length))
    await waitFor(() => expect(next().disabled).toBe(false))
    fireEvent.click(next())
    await screen.findByLabelText("Encoding")
    return calls
  }

  const differing = (uploadId: string, file: string) => ({
    mismatches: [{ uploadId, file, missing: [], extra: ["notes"], typeConflicts: [] }],
  })

  it("marks a file whose columns differ with what is extra, and keeps Next off until it is removed", async () => {
    const calls = await openCheck([CSV(), SECOND(), THIRD()], {
      compare: (asked) => (asked.alsoUploads.includes("up-3") ? differing("up-3", "third.csv") : { mismatches: [] }),
    })
    const list = screen.getByRole("list", { name: "Files in this load" })
    await within(list).findByText("Different columns")
    expect(within(list).getByText("Same columns")).toBeDefined()
    expect(within(list).getByText("Not in the first file: notes")).toBeDefined()
    expect(screen.getByText("3 files load into one table. Their columns are matched by name, in any order.")).toBeDefined()
    expect(screen.getByText(/One file does not have the columns of the first/)).toBeDefined()
    expect(next().disabled).toBe(true)

    fireEvent.click(screen.getByRole("button", { name: "Remove third.csv" }))
    await waitFor(() => expect(next().disabled).toBe(false))
    expect(within(screen.getByRole("list", { name: "Files in this load" })).queryByText("third.csv")).toBeNull()
    // The comparison was asked again, for the file that is left.
    const asked = sent(calls, "POST", /compare$/).map((c) => (c.body as { alsoUploads: string[] }).alsoUploads)
    expect(asked[asked.length - 1]).toEqual(["up-2"])
  })

  it("lists the type conflicts of a file, and asks again with the column list when a type is changed", async () => {
    const calls = await openCheck([CSV(), SECOND()], {
      compare: (asked) =>
        asked.columns?.some((c) => c.name === "qty" && c.type === "text")
          ? { mismatches: [] }
          : {
              mismatches: [
                {
                  uploadId: "up-2",
                  file: "second.csv",
                  missing: [],
                  extra: [],
                  typeConflicts: [{ column: "qty", expected: "integer", found: "text" }],
                },
              ],
            },
      preview: () => ({
        ...preview(),
        columnSpecs: [
          { source: "sku", name: "sku", type: "text", guessedType: "text", askDateOrder: false },
          { source: "qty", name: "qty", type: "integer", guessedType: "integer", askDateOrder: false },
        ],
      }),
    })
    const list = screen.getByRole("list", { name: "Files in this load" })
    await within(list).findByText(/qty: the first file reads it as integer, here it looks like text/)
    expect(next().disabled).toBe(true)

    fireEvent.change(screen.getByLabelText("Type of qty"), { target: { value: "text" } })
    await within(list).findByText("Same columns")
    await waitFor(() => expect(next().disabled).toBe(false))
    const last = sent(calls, "POST", /compare$/).pop()
    expect((last?.body as { columns: UploadColumnSpec[] }).columns).toEqual([
      { source: "sku", name: "sku", type: "text" },
      { source: "qty", name: "qty", type: "text" },
    ])
  })

  it("does not go on when the comparison itself fails, and compares again on request", async () => {
    let failing = true
    await openCheck([CSV(), SECOND()], {
      compare: () => (failing ? json({ error: "One of the other files was not found." }, 404) : { mismatches: [] }),
    })
    expect((await screen.findByRole("alert")).textContent).toBe("One of the other files was not found.")
    expect(next().disabled).toBe(true)
    failing = false
    fireEvent.click(screen.getByRole("button", { name: "Compare again" }))
    await within(screen.getByRole("list", { name: "Files in this load" })).findByText("Same columns")
    await waitFor(() => expect(next().disabled).toBe(false))
  })

  it("reviews N files into the table, loads with the other files named, and shows one run", async () => {
    const started = upload({ status: "ingesting", bronzeTable: "stock", parseOptions: OPTIONS, loadMode: "replace" })
    const calls = await openCheck([CSV(), SECOND()], {
      get: (n) => (n < 3 ? started : { ...started, status: "ingested", rows: 6, assetId: "stock" }),
      ingest: json({ upload: started, runId: "run-1" }),
    })
    await within(screen.getByRole("list", { name: "Files in this load" })).findByText("Same columns")
    await waitFor(() => expect(next().disabled).toBe(false))
    fireEvent.click(next())
    await screen.findByLabelText("Table name")
    fireEvent.click(next())
    await screen.findByRole("button", { name: "Load" })

    const value = (label: string) =>
      screen.getAllByRole("term").find((term) => term.textContent === label)?.nextElementSibling?.textContent
    expect(value("Files")).toBe("2 files into bronze.stock")
    expect(value("File 2")).toBe("second.csv")
    expect(screen.getByText(/^Load reads/).textContent).toContain("2 files, the first of them stock.csv")
    expect(screen.getByText(/^Load reads/).textContent).toContain("Any rows the table already holds are replaced by these files'.")

    fireEvent.click(screen.getByRole("button", { name: "Load" }))
    await screen.findByText("Load finished")
    const post = calls.find((c) => c.url.endsWith("/up-1/ingest"))
    expect(post?.body).toMatchObject({ bronzeTable: "stock", alsoUploads: ["up-2"] })
    expect(screen.getByText(/2 files load into one table\. With stock\.csv:/)).toBeDefined()
    expect(screen.getByText("Rows in all the files")).toBeDefined()
    expect(screen.getByRole("button", { name: "Upload other files" })).toBeDefined()
  })

  it("tries a failed load of several files again with the same files", async () => {
    const started = upload({ status: "ingesting", bronzeTable: "stock", parseOptions: OPTIONS, loadMode: "replace" })
    const failed = { ...started, status: "failed", error: "The load into the table failed." }
    const calls = await openCheck([CSV(), SECOND()], {
      get: (n) => (n < 3 ? started : failed),
      ingest: json({ upload: started, runId: "run-1" }),
    })
    await within(screen.getByRole("list", { name: "Files in this load" })).findByText("Same columns")
    await waitFor(() => expect(next().disabled).toBe(false))
    fireEvent.click(next())
    await screen.findByLabelText("Table name")
    fireEvent.click(next())
    fireEvent.click(await screen.findByRole("button", { name: "Load" }))
    fireEvent.click(await screen.findByRole("button", { name: "Try again" }))

    await waitFor(() => expect(calls.filter((c) => c.url.endsWith("/up-1/ingest")).length).toBe(2))
    const retry = calls.filter((c) => c.url.endsWith("/up-1/ingest"))[1]
    expect(retry.body).toMatchObject({ bronzeTable: "stock", alsoUploads: ["up-2"] })
  })

  it("opened on a file that was loaded with another, does not retry it alone and points at the first file", async () => {
    url.search = "?id=up-2"
    const second = upload({
      id: "up-2",
      originalFilename: "second.csv",
      status: "ingesting",
      bronzeTable: "stock",
      parseOptions: OPTIONS,
      loadMode: "replace",
      loadedWith: "up-1",
    })
    stubFetch({
      get: (n) => (n === 1 ? second : { ...second, status: "failed", error: "The load into the table failed." }),
      list: [upload({ id: "up-1", originalFilename: "stock.csv" }), second],
    })
    render(<UploadFilePage pollMs={20} />)

    await screen.findByText("Load failed")
    expect(screen.queryByRole("button", { name: "Try again" })).toBeNull()
    expect(screen.getByText("Open the first file").closest("a")?.getAttribute("href")).toBe("/connectors/upload?id=up-1")
  })
})
