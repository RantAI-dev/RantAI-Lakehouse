// The upload page (plan T11, `docs/superpowers/plans/2026-10-02-upload-file.md`;
// checklist in `docs/core/features/upload-file.md`): a file over 50 MB is
// refused before anything is sent, a refusal of the API is shown as it is, the
// preview shows what was detected and what is in force and reads again when a
// setting changes, the choice to replace or add appears only for a table an
// upload created, and a load is shown Loading, Loaded or Failed.
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

type Routes = {
  list?: Upload[] | Response
  create?: Response
  get?: (n: number) => Upload | Response
  preview?: (query: URLSearchParams) => UploadPreview | Response
  ingest?: Response
}

/** Answers every request the page makes; records them in order. A request nobody expects throws. */
function stubFetch(routes: Routes = {}): Call[] {
  const calls: Call[] = []
  let gets = 0
  global.fetch = mock(async (input: RequestInfo | URL, init?: RequestInit) => {
    const target = new URL(String(input), "http://console.test")
    const method = init?.method ?? "GET"
    const body = typeof init?.body === "string" ? JSON.parse(init.body) : init?.body
    calls.push({ url: `${target.pathname}${target.search}`, method, body })
    const reply = (value: unknown) => (value instanceof Response ? value : json(value))
    if (target.pathname === "/api/uploads" && method === "GET") return reply(routes.list ?? [])
    if (target.pathname === "/api/uploads" && method === "POST") {
      if (routes.create) return routes.create
    }
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

function chooseFile(file: File) {
  fireEvent.change(screen.getByLabelText("Choose a file"), { target: { files: [file] } })
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
    expect(calls.filter((c) => c.method === "POST")).toEqual([])
  })

  it("shows the API's refusal in place, word for word, and stays on the first step", async () => {
    const sentence = "This looks like a workbook. Save it as CSV and upload that."
    stubFetch({ create: json({ error: sentence }, 400) })
    render(<UploadFilePage />)
    // A name the console does not refuse by itself (it refuses .xlsx before any
    // request, see below), so the API's own refusal by content is what shows.
    chooseFile(new File(["x"], "stock.dat"))
    fireEvent.click(next())

    const alert = await screen.findByRole("alert")
    expect(alert.textContent).toBe(sentence)
    expect(screen.getByLabelText("Choose a file")).toBeDefined()
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
    expect(calls.filter((c) => c.method === "POST")).toEqual([])
  })

  it("states the accepted kinds and the 50 MB limit up front, and suggests text and Excel files in the picker", () => {
    stubFetch()
    render(<UploadFilePage />)
    const hint = screen.getByText(/Delimited text files \(CSV, TSV\) and Excel/)
    expect(hint.textContent).toContain("workbooks (.xls, .xlsx) up to 50 MB are accepted")
    expect(hint.textContent).toContain("(.xlsm, .xlsb, .ods, archives) are refused")
    const input = screen.getByLabelText("Choose a file") as HTMLInputElement
    expect(input.accept).toContain(".csv")
    expect(input.accept).toContain(".tsv")
    expect(input.accept).toContain(".xls")
    expect(input.accept).toContain(".xlsx")
  })

  it("sends an .xlsx instead of refusing it by name, and lands on a Check step with a sheet picker", async () => {
    const calls = stubFetch({
      create: json({ ...upload({ originalFilename: "stock.xlsx" }), workbook: WORKBOOK }, 201),
      preview: (q) => workbookPreview(q.get("sheet") ?? WORKBOOK.defaultSheet),
    })
    render(<UploadFilePage />)
    chooseFile(new File(["x"], "stock.xlsx"))
    expect(screen.queryByRole("alert")).toBeNull()
    fireEvent.click(next())

    await screen.findByLabelText("Sheet")
    expect(calls.find((c) => c.method === "POST")?.url).toBe("/api/uploads")
  })

  it("does not show a proxy's plain-text 500 or 413 as such: it says the upload did not reach the service and names the limit", async () => {
    for (const status of [500, 413]) {
      stubFetch({ create: new Response("Internal Server Error", { status }) })
      render(<UploadFilePage />)
      chooseFile(CSV())
      fireEvent.click(next())

      const alert = await screen.findByRole("alert")
      expect(alert.textContent).toContain("did not reach the service")
      expect(alert.textContent).toContain(`${status}`)
      expect(alert.textContent).toContain("50 MB")
      expect(alert.textContent).not.toContain("Internal Server Error")
      cleanup()
    }
  })

  it("sends the file as a multipart form and moves to the Check step, with the repeated-file notice", async () => {
    const calls = stubFetch({
      create: json({ ...upload(), duplicateOf: upload({ id: "up-0", originalFilename: "stock_old.csv", bronzeTable: "stock" }) }, 201),
      preview: () => preview(),
    })
    render(<UploadFilePage />)
    chooseFile(CSV())
    fireEvent.click(next())

    await screen.findByLabelText("Encoding")
    const post = calls.find((c) => c.method === "POST")
    expect(post?.url).toBe("/api/uploads")
    expect(post?.body).toBeInstanceOf(FormData)
    const notice = screen.getByText(/This file was uploaded before/)
    expect(notice.textContent).toContain("stock_old.csv")
    expect(notice.textContent).toMatch(/Oct.*2026|2026.*Oct/)
  })

  it("says a refusal for a missing permission through the existing state", async () => {
    stubFetch({ create: json({ error: "You do not have permission to do this." }, 403) })
    render(<UploadFilePage />)
    chooseFile(CSV())
    fireEvent.click(next())
    expect(await screen.findByText("You don't have access")).toBeDefined()
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
    expect(screen.getByText(TABLE_NAME_RULE).className).toContain("text-destructive")
    expect(next().disabled).toBe(true)
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
