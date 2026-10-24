// The page-level rule for "Fix with AI": the button is in the DOM only after a
// run has failed. The rule lives in `QueryStudioPage`'s JSX, so this renders
// the real page. Its two providers (`AuthProvider`, `CopilotProvider`) and
// its child panels reach the network only through `fetch` and the service
// objects, which are spied here, so nothing leaves the test.
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

import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react"
import { afterEach, describe, expect, it, mock, spyOn } from "bun:test"
import { TableProviders } from "@/components/app-shell/table-providers"
import { AuthProvider } from "@/features/auth/auth-provider"
import { CopilotProvider } from "@/features/copilot/use-copilot"
import { assetService, lakehouseService, queryService } from "@/services"
import { ServiceError } from "@/services/errors"
import { QUERY_DRAFT_KEY } from "@/lib/query-draft"
import { SQL_FORMAT_SETTINGS_KEY } from "@/lib/sql-format-settings"
import { QueryStudioPage } from "./query-studio-page"

afterEach(() => {
  cleanup()
  mock.restore()
  window.localStorage.clear()
  visit("/query-studio")
})

const json = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } })

/** Points the browser at a Query Studio address, the way an href does. */
function visit(path: string) {
  const url = new URL(path, "http://localhost:3000").href
  ;(window as unknown as { happyDOM: { setURL: (url: string) => void } }).happyDOM.setURL(url)
}

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
  id: "r1",
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
} as never

describe("QueryStudioPage Fix with AI", () => {
  it("shows the Fix with AI button only after a run has failed, not before one and not after one that succeeded", async () => {
    stubNetwork()
    const run = spyOn(queryService, "run")
      .mockResolvedValueOnce({
        id: "r1",
        columns: [],
        rows: [],
        engine: "clickhouse",
        rowCount: 0,
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
      } as never)
      .mockRejectedValueOnce(new ServiceError("invalid_request", "Unknown identifier: amont"))
    seedDraft("SELECT amont FROM t")
    renderPage()

    // Before any run: the button is absent.
    const runButton = await screen.findByRole("button", { name: "Run query" })
    await waitFor(() => expect((runButton as HTMLButtonElement).disabled).toBe(false))
    expect(screen.queryByRole("button", { name: /Fix with AI/ })).toBeNull()

    // After a run that succeeded: still absent.
    fireEvent.click(runButton)
    await waitFor(() => expect(run).toHaveBeenCalledTimes(1))
    await new Promise((resolve) => setTimeout(resolve, 0))
    expect(screen.queryByRole("button", { name: /Fix with AI/ })).toBeNull()

    // After a run that failed: the button is there.
    fireEvent.click(runButton)
    await waitFor(() => expect(run).toHaveBeenCalledTimes(2))
    expect(await screen.findByRole("button", { name: /Fix with AI/ })).toBeTruthy()
  })
})

describe("QueryStudioPage format settings", () => {
  it("menampilkan pengaturan format yang tersimpan di browser ini", async () => {
    stubNetwork()
    window.localStorage.setItem(
      SQL_FORMAT_SETTINGS_KEY,
      JSON.stringify({ keywordCase: "lower", indent: 4 })
    )
    seedDraft("select a from t")
    renderPage()

    fireEvent.click(await screen.findByRole("button", { name: "Format settings" }))
    await waitFor(() =>
      expect(screen.getByRole("combobox", { name: "Keyword case" }).textContent).toContain(
        "Lower case"
      )
    )
    expect(screen.getByRole("combobox", { name: "Indent" }).textContent).toContain("4 spaces")
  })

  it("memformat dengan pengaturan yang tersimpan", async () => {
    stubNetwork()
    window.localStorage.setItem(
      SQL_FORMAT_SETTINGS_KEY,
      JSON.stringify({ keywordCase: "lower", indent: 2 })
    )
    seedDraft("select a from t")
    const { container } = renderPage()

    const formatButton = await screen.findByRole("button", { name: "Format" })
    await waitFor(() => expect((formatButton as HTMLButtonElement).disabled).toBe(false))
    fireEvent.click(formatButton)

    await waitFor(() => {
      const lines = Array.from(container.querySelectorAll(".cm-line")).map(
        (line) => line.textContent ?? ""
      )
      expect(lines.join("\n")).toBe("select\n  a\nfrom\n  t")
    })
  })
})

describe("QueryStudioPage query tabs", () => {
  it("opens a ?sql= link in a tab of its own and leaves the draft where it was", async () => {
    stubNetwork()
    seedDraft("SELECT 1")
    visit("/query-studio?sql=SELECT%202")
    const { container } = renderPage()

    // The link's query opens in a tab of its own, beside the draft.
    await waitFor(() =>
      expect(screen.getAllByRole("button", { name: /^Query \d+$/ }).length).toBe(2)
    )
    const editorText = () =>
      Array.from(container.querySelectorAll(".cm-line"))
        .map((line) => line.textContent ?? "")
        .join("\n")
    await waitFor(() => expect(editorText()).toBe("SELECT 2"))

    // The draft is still there, in the tab that was open before.
    fireEvent.click(screen.getByRole("button", { name: "Query 1" }))
    await waitFor(() => expect(editorText()).toBe("SELECT 1"))
  })
})

describe("QueryStudioPage statements", () => {
  it("disables the actions that work on one statement when the tab holds several", async () => {
    stubNetwork()
    seedDraft("SELECT 1; SELECT 2")
    renderPage()

    const explain = await screen.findByRole("button", { name: "Explain" })
    await waitFor(() => expect((explain as HTMLButtonElement).disabled).toBe(true))
    expect(explain.getAttribute("title")).toBe("Select one statement")

    const estimate = screen.getByRole("button", { name: "Estimate" })
    expect((estimate as HTMLButtonElement).disabled).toBe(true)
    expect(estimate.getAttribute("title")).toBe("Select one statement")

    const save = screen.getByRole("button", { name: "Save query" })
    expect((save as HTMLButtonElement).disabled).toBe(true)
    expect(save.getAttribute("title")).toBe("Select one statement")
  })

  it("proposes a fix for the statement that failed, not for the whole buffer", async () => {
    stubNetwork()
    const proposeFix = spyOn(queryService, "proposeFix").mockResolvedValue({
      sql: "SELECT amount FROM t",
      reason: "The column is named amount.",
    })
    spyOn(queryService, "run").mockRejectedValue(
      new ServiceError("invalid_request", "Unknown identifier: amont")
    )
    seedDraft("SELECT amont FROM t; SELECT 2")
    renderPage()

    const runButton = await screen.findByRole("button", { name: "Run all" })
    await waitFor(() => expect((runButton as HTMLButtonElement).disabled).toBe(false))
    fireEvent.click(runButton)

    fireEvent.click(await screen.findByRole("button", { name: /Fix with AI/ }))

    await waitFor(() => expect(proposeFix).toHaveBeenCalledTimes(1))
    expect(proposeFix.mock.calls[0]?.[0]).toBe("SELECT amont FROM t")
  })

  it("puts the accepted fix back in the statement it was made for and nowhere else", async () => {
    stubNetwork()
    spyOn(queryService, "run")
      .mockResolvedValueOnce(RESULT)
      .mockRejectedValueOnce(new ServiceError("invalid_request", "Unknown identifier: amont"))
    spyOn(queryService, "proposeFix").mockResolvedValue({
      sql: "SELECT amount FROM t",
      reason: "The column is named amount.",
    })
    seedDraft("SELECT 1;\nSELECT amont FROM t;\nSELECT 3")
    const { container } = renderPage()

    const runButton = await screen.findByRole("button", { name: "Run all" })
    await waitFor(() => expect((runButton as HTMLButtonElement).disabled).toBe(false))
    fireEvent.click(runButton)

    // The failure is on the second statement, so its result tab is the one
    // that carries the error.
    const bar = await screen.findByRole("group", { name: "Statement results" })
    fireEvent.click(within(bar).getByRole("button", { name: "SELECT amont FROM t" }))
    fireEvent.click(await screen.findByRole("button", { name: /Fix with AI/ }))
    fireEvent.click(await screen.findByRole("button", { name: "Accept" }))

    await waitFor(() => {
      const lines = Array.from(container.querySelectorAll(".cm-line")).map(
        (line) => line.textContent ?? ""
      )
      expect(lines.join("\n")).toBe("SELECT 1;\nSELECT amount FROM t;\nSELECT 3")
    })
  })
})

describe("QueryStudioPage schema browser", () => {
  it("keeps the editor and the run working when the catalog refuses the schema browser", async () => {
    stubNetwork()
    spyOn(assetService, "listAssets").mockRejectedValue(
      new ServiceError("permission_denied", "catalog:read", 403)
    )
    const run = spyOn(queryService, "run").mockRejectedValue(
      new ServiceError("invalid_request", "stopped here")
    )
    seedDraft("SELECT 1")
    const { container } = renderPage()

    fireEvent.click(await screen.findByRole("button", { name: "Schema" }))
    expect(await screen.findByText(/needs catalog access/i)).toBeTruthy()

    // The refusal sits beside the editor, not in place of it.
    expect(container.querySelector(".cm-content")).not.toBeNull()
    const runButton = await screen.findByRole("button", { name: "Run query" })
    await waitFor(() => expect((runButton as HTMLButtonElement).disabled).toBe(false))
    fireEvent.click(runButton)
    await waitFor(() => expect(run).toHaveBeenCalledTimes(1))
    await screen.findByText(/stopped here/)
  })

  it("closes the browser from its own button and from the toolbar toggle", async () => {
    stubNetwork()
    seedDraft("SELECT 1")
    renderPage()

    const toggle = await screen.findByRole("button", { name: "Schema" })
    expect(toggle.getAttribute("aria-pressed")).toBe("false")
    fireEvent.click(toggle)
    expect(toggle.getAttribute("aria-pressed")).toBe("true")
    fireEvent.click(await screen.findByRole("button", { name: "Close schema browser" }))

    expect(screen.queryByRole("complementary", { name: "Schema browser" })).toBeNull()
    expect(toggle.getAttribute("aria-pressed")).toBe("false")
  })
})
