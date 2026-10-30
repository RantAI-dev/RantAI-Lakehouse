// The download of a Query Studio result: the menu that asks for the file,
// the file that reaches the browser, the refusal that is shown in the
// server's own words, the cut that is named, and the Stop that ends a
// download in flight.
//
// The real page is rendered here: the menu, the results section and the Stop
// button are wired in the page, and only the real page proves that they are.
// Its two providers (`AuthProvider`, `CopilotProvider`) and its child panels
// reach the network only through `fetch` and the service objects, which are
// spied here, so nothing leaves the test.
// `next/navigation` is stubbed before any import resolves (`mock.module` is
// hoisted), and `TableProviders` supplies the URL state the results table
// needs, as the root layout does.
mock.module("next/navigation", () => ({
  usePathname: () => "/query-studio",
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

import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, jest, mock, spyOn } from "bun:test"
import { TableProviders } from "@/components/app-shell/table-providers"
import { AuthProvider } from "@/features/auth/auth-provider"
import { CopilotProvider } from "@/features/copilot/use-copilot"
import * as browserFile from "@/lib/browser-file"
import * as notify from "@/lib/notify"
import { QUERY_DRAFT_KEY } from "@/lib/query-draft"
import { assetService, lakehouseService, queryService } from "@/services"
import { ServiceError } from "@/services/errors"
import { QueryStudioPage } from "./query-studio-page"

afterEach(() => {
  cleanup()
  mock.restore()
  // The clock is faked in one test only; the next one starts from the real
  // one again.
  jest.useRealTimers()
  window.localStorage.clear()
})

const json = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } })

/** Answers the session and the Copilot sessions list; everything else is an empty success. */
function stubNetwork() {
  spyOn(globalThis, "fetch").mockImplementation((async (input: RequestInfo | URL) => {
    const path = String(input)
    if (path.includes("/api/auth/me")) {
      return json({ id: "u1", name: "Analyst", email: null, roles: ["Analyst"], permissions: [], tenants: [] })
    }
    if (path.includes("/api/ai/sessions")) return json({ sessions: [] })
    return json({})
  }) as unknown as typeof fetch)
  spyOn(queryService, "listSaved").mockResolvedValue([])
  spyOn(queryService, "listHistory").mockResolvedValue([])
  spyOn(queryService, "estimate").mockRejectedValue(new ServiceError("unavailable", "no estimate"))
  spyOn(assetService, "listAssets").mockResolvedValue([])
  spyOn(lakehouseService, "listNamespaces").mockResolvedValue([])
}

/** Seeds the editor with a statement and opens the SQL tab, the way a saved draft would. */
function seedDraft(sql: string) {
  window.localStorage.setItem(QUERY_DRAFT_KEY, JSON.stringify({ sql, question: "", tab: "sql" }))
}

function renderPage() {
  return render(
    <AuthProvider>
      <TableProviders>
        <CopilotProvider>
          <QueryStudioPage />
        </CopilotProvider>
      </TableProviders>
    </AuthProvider>,
  )
}

/** One row of a result, as the API sends it. */
const RESULT = {
  id: "q-1",
  columns: ["n"],
  rows: [{ n: 1 }],
  engine: "clickhouse",
  rowCount: 1,
  truncated: false,
  rowLimit: 2000,
  metrics: {
    durationMs: 1,
    scannedBytes: 0,
    costUnits: 0,
    engine: "hot",
    workloadClass: "interactive",
    cacheHit: null,
    pushdowns: null,
    policyObligations: null,
  },
  plan: null,
}

/** The file a finished download holds. */
const FILE = {
  blob: new Blob(["n\r\n1\r\n"]),
  fileName: "q-1.csv",
  rows: 1,
  cut: false,
  cutBy: null,
}

/** Runs the statement of the draft and waits for the result to be on screen. */
async function runAndWait() {
  const runButton = await screen.findByRole("button", { name: "Run query" })
  await waitFor(() => expect((runButton as HTMLButtonElement).disabled).toBe(false))
  fireEvent.click(runButton)
  return screen.findByRole("button", { name: "Download" })
}

/** Opens the download menu and picks one file kind. */
async function downloadAs(label: string) {
  fireEvent.click(await screen.findByRole("button", { name: "Download" }))
  fireEvent.click(await screen.findByRole("menuitem", { name: label }))
}

describe("the download menu of a result", () => {
  it("offers the four file kinds, and asks for the chosen one under the run's own id and run id", async () => {
    stubNetwork()
    let runId = ""
    spyOn(queryService, "run").mockImplementation(((_sql: string, options: { runId?: string }) => {
      runId = options.runId ?? ""
      return Promise.resolve(RESULT as never)
    }) as never)
    const download = spyOn(queryService, "download").mockResolvedValue(FILE as never)
    seedDraft("SELECT 1")
    renderPage()

    await runAndWait()
    fireEvent.click(await screen.findByRole("button", { name: "Download" }))
    for (const label of ["CSV", "TSV", "Excel", "Parquet"]) {
      expect(await screen.findByRole("menuitem", { name: label })).toBeTruthy()
    }
    fireEvent.click(screen.getByRole("menuitem", { name: "CSV" }))

    await waitFor(() => expect(download).toHaveBeenCalledTimes(1))
    expect(download.mock.calls[0]?.[0]).toBe("q-1")
    expect(download.mock.calls[0]?.[1]).toBe("csv")
    expect(download.mock.calls[0]?.[2]).toBe(runId)
  })
})

describe("a finished download", () => {
  it("saves the file and says it did", async () => {
    stubNetwork()
    spyOn(queryService, "run").mockResolvedValue(RESULT as never)
    spyOn(queryService, "download").mockResolvedValue(FILE as never)
    const saved = spyOn(browserFile, "saveBlob")
    const success = spyOn(notify, "notifySuccess")
    seedDraft("SELECT 1")
    renderPage()

    await runAndWait()
    await downloadAs("CSV")

    await waitFor(() => expect(success).toHaveBeenCalledTimes(1))
    expect(saved).toHaveBeenCalledTimes(1)
    expect(saved.mock.calls[0]?.[1]).toBe("q-1.csv")
    expect(success).toHaveBeenCalledWith("Downloaded q-1.csv", "The file holds 1 rows.")
  })

  it("names the row limit that cut the file", async () => {
    stubNetwork()
    spyOn(queryService, "run").mockResolvedValue(RESULT as never)
    spyOn(queryService, "download").mockResolvedValue({
      ...FILE,
      rows: 100,
      cut: true,
      cutBy: "rows",
    } as never)
    const success = spyOn(notify, "notifySuccess")
    seedDraft("SELECT 1")
    renderPage()

    await runAndWait()
    await downloadAs("CSV")

    await waitFor(() => expect(success).toHaveBeenCalledTimes(1))
    expect(success).toHaveBeenCalledWith(
      "Downloaded q-1.csv",
      "The file was cut at 100 rows, which is the limit of this format.",
    )
  })

  it("names the byte limit that cut the file", async () => {
    stubNetwork()
    spyOn(queryService, "run").mockResolvedValue(RESULT as never)
    spyOn(queryService, "download").mockResolvedValue({
      ...FILE,
      rows: null,
      cut: true,
      cutBy: "bytes",
    } as never)
    const success = spyOn(notify, "notifySuccess")
    seedDraft("SELECT 1")
    renderPage()

    await runAndWait()
    await downloadAs("CSV")

    await waitFor(() => expect(success).toHaveBeenCalledTimes(1))
    expect(success).toHaveBeenCalledWith("Downloaded q-1.csv", "The file was cut at the 5 GB limit.")
  })

  it("shows the refusal in the server's own words", async () => {
    stubNetwork()
    spyOn(queryService, "run").mockResolvedValue(RESULT as never)
    spyOn(queryService, "download").mockRejectedValue(
      new Error("Downloads are switched off for your role."),
    )
    const failure = spyOn(notify, "notifyFailure")
    seedDraft("SELECT 1")
    renderPage()

    await runAndWait()
    await downloadAs("CSV")

    await waitFor(() => expect(failure).toHaveBeenCalledTimes(1))
    expect(failure).toHaveBeenCalledWith("Downloads are switched off for your role.")
  })
})

describe("Stop during a download", () => {
  it("aborts the request, asks the server to end the download's query, and takes the Stop button away", async () => {
    stubNetwork()
    let runId = ""
    spyOn(queryService, "run").mockImplementation(((_sql: string, options: { runId?: string }) => {
      runId = options.runId ?? ""
      return Promise.resolve(RESULT as never)
    }) as never)
    let signal: AbortSignal | undefined
    spyOn(queryService, "download").mockImplementation(((
      _id: string,
      _format: string,
      _runId: string,
      sig?: AbortSignal,
    ) => {
      signal = sig
      return new Promise(() => {})
    }) as never)
    const cancel = spyOn(queryService, "cancel").mockResolvedValue({ stopped: 1 })
    seedDraft("SELECT 1")
    renderPage()

    await runAndWait()
    await downloadAs("CSV")
    const stop = await screen.findByRole("button", { name: "Stop" })
    fireEvent.click(stop)

    await waitFor(() => expect(cancel).toHaveBeenCalledTimes(1))
    expect(cancel.mock.calls[0]?.[0]).toBe(runId)
    expect(signal?.aborted).toBe(true)
    // `queryAllBy…` and a length, not `queryBy…` and `toBeNull()`: a failed
    // assertion on a DOM node makes the runner print the whole document.
    await waitFor(() =>
      expect(screen.queryAllByRole("button", { name: "Stop" })).toHaveLength(0)
    )
  })
})

describe("the elapsed time of a run", () => {
  it("is on screen while the run is in flight", async () => {
    stubNetwork()
    spyOn(queryService, "run").mockImplementation((() => new Promise(() => {})) as never)
    seedDraft("SELECT 1")
    renderPage()

    const runButton = await screen.findByRole("button", { name: "Run query" })
    await waitFor(() => expect((runButton as HTMLButtonElement).disabled).toBe(false))
    expect(screen.queryAllByRole("timer", { name: "Elapsed time" })).toHaveLength(0)

    fireEvent.click(runButton)
    const timer = await screen.findByRole("timer", { name: "Elapsed time" })
    expect(timer.textContent).toContain("ms")
  })

  it("ticks every second while the run is in flight", async () => {
    // The clock is faked, so the test drives the seconds itself: the page's
    // own interval is the only thing that can move the elapsed time on.
    jest.useFakeTimers()
    stubNetwork()
    spyOn(queryService, "run").mockImplementation((() => new Promise(() => {})) as never)
    seedDraft("SELECT 1")
    renderPage()

    const runButton = await screen.findByRole("button", { name: "Run query" })
    await waitFor(() => expect((runButton as HTMLButtonElement).disabled).toBe(false))

    fireEvent.click(runButton)
    const timer = await screen.findByRole("timer", { name: "Elapsed time" })
    expect(timer.textContent).toBe("0 ms")

    await act(async () => {
      jest.advanceTimersByTime(1000)
    })
    // One second later the run has been going for a second, and the page says
    // so: the clock ticks every second, not every other one.
    expect(timer.textContent).toBe("1.0 s")

    await act(async () => {
      jest.advanceTimersByTime(1000)
    })
    expect(timer.textContent).toBe("2.0 s")
  })
})
