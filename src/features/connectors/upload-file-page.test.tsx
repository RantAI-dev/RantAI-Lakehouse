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

import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, mock, spyOn } from "bun:test"
import { formatBytes } from "@/lib/format"
import { MAX_UPLOAD_BYTES, REGISTRATION_FAILED_REASON, TABLE_NAME_RULE } from "@/lib/uploads"
import type { Upload, UploadPreview } from "@/services/contracts/uploads"
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

function preview(over: Partial<UploadPreview> = {}): UploadPreview {
  return {
    detected: OPTIONS,
    using: OPTIONS,
    columns: ["sku", "qty"],
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
  ingest?: Response
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
  it("has a labelled file control and refuses a file over 50 MB, sending nothing", async () => {
    const calls = stubFetch()
    render(<UploadFilePage />)
    const big = new File(["x"], "big.csv")
    Object.defineProperty(big, "size", { value: MAX_UPLOAD_BYTES + 1 })
    chooseFile(big)

    const alert = await screen.findByRole("alert")
    expect(alert.textContent).toContain("big.csv")
    expect(alert.textContent).toContain("50 MB limit")
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
    expect(alert.textContent).toContain("not an .xls or .xlsx file")
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
    expect(facts.get("Accepted")).toBe("CSV, TSV, .xls, .xlsx, .parquet")
    expect(facts.get("Files")).toContain("Up to 10 at once")
    expect(facts.get("Size")).toBe("Up to 50 MB in all")
    expect(facts.get("Not accepted")).toBe(".xlsm, .xlsb, .ods, archives")
    expect(facts.get("Workbooks")).toContain("One sheet is loaded")
    expect(facts.get("Parquet")).toContain("binary or nested column is refused")
    const input = screen.getByLabelText("Choose files") as HTMLInputElement
    expect(input.type).toBe("file")
    expect(input.multiple).toBe(true)
    expect(input.accept).toContain(".csv")
    expect(input.accept).toContain(".tsv")
    expect(input.accept).toContain(".xls")
    expect(input.accept).toContain(".xlsx")
    expect(input.accept).toContain(".parquet")
  })

  it("is a drop area with one line, a button labelling the real file input, and the page's subtitle names Excel", () => {
    stubFetch()
    render(<UploadFilePage />)
    expect(screen.getByText("Drop files here, or choose some")).toBeDefined()
    expect(screen.getByText(/CSV, TSV, Excel or Parquet files in as raw tables/)).toBeDefined()
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
    expect(alert.textContent).toContain("not an .xls or .xlsx file")
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
    expect(alert.textContent).toContain("50 MB")
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

  it("says the others are still being sent on the next steps, and where the ones that arrived are loaded from", async () => {
    let release: (() => void) | null = null
    const held = new Promise<void>((resolve) => {
      release = resolve
    })
    stubFetch({
      part: async (id, n) => {
        // The second file's piece is held back.
        if (id === "up-2") await held
        return json({ partNumber: n, sizeBytes: 1 })
      },
      preview: () => preview(),
    })
    render(<UploadFilePage />)
    chooseFiles([CSV(), new File(["a,b\n1,2\n"], "second.csv")])
    await waitFor(() => expect(next().disabled).toBe(false))
    fireEvent.click(next())
    await screen.findByLabelText("Encoding")

    expect(screen.getByText(/1 other file is still being sent/)).toBeDefined()
    release!()
    await waitFor(() => expect(screen.getByText(/1 other file has arrived/)).toBeDefined())
    expect(screen.getByText(/load the other from Uploaded files/)).toBeDefined()
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
    expect(screen.getAllByRole("columnheader").length).toBe(3)
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
    const note = screen.getByText(/Every column is loaded as text/)
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
    expect(post?.body).toEqual({
      encoding: "utf-8",
      delimiter: ",",
      headerRow: 0,
      sheet: "Quirks",
      bronzeTable: "stock",
      mode: "replace",
    })
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

  it("shows no sheet, encoding, delimiter or header-row control, and says every column is loaded as text", async () => {
    await openParquet()
    expect(screen.queryByLabelText("Sheet")).toBeNull()
    expect(screen.queryByLabelText("Encoding")).toBeNull()
    expect(screen.queryByLabelText("Delimiter")).toBeNull()
    expect(screen.queryByLabelText("Header row")).toBeNull()
    const note = screen.getByText(/This is a Parquet file/)
    expect(note.textContent).toContain("every column is loaded as text")
    expect(note.textContent).toContain("2025-09-24")
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
    expect(post?.body).toEqual({
      encoding: "utf-8",
      delimiter: ",",
      headerRow: 0,
      bronzeTable: "orders",
      mode: "replace",
    })
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
    expect(screen.getByText(/Every column is stored as text/)).toBeDefined()
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

  it("says the columns are text in a notice", async () => {
    await openTable([])
    const note = screen.getByText(/Every column is stored as text/)
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
    expect(post?.body).toEqual({ encoding: "utf-8", delimiter: ",", headerRow: 0, bronzeTable: "stock", mode: "replace" })

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
    expect(labels).toEqual(["Name", "Size", "Uploaded", "Encoding", "Delimiter", "Header row", "Columns", "Table", "Column types"])
    const value = (label: string) =>
      screen.getAllByRole("term").find((term) => term.textContent === label)?.nextElementSibling?.textContent
    expect(value("Table")).toBe("bronze.stock")
    expect(value("Columns")).toBe("2")
    const sentence = screen.getByText(/^Load reads/)
    expect(sentence.textContent).toContain("stock.csv")
    expect(sentence.textContent).toContain("bronze.stock")
    expect(sentence.textContent).toContain("every column as text")
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
    expect(retry?.body).toEqual({ ...OPTIONS, bronzeTable: "stock", mode: "append" })
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
    expect(screen.queryByLabelText("Choose a file")).toBeNull()
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
