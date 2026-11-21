// The page-level rule for "Fix with AI": the button is in the DOM only after a
// run has failed. The rule lives in `QueryStudioPage`'s JSX, so this renders
// the real page. Its two providers (`AuthProvider`, `CopilotProvider`) and
// its child panels reach the network only through `fetch` and the service
// objects, which are spied here, so nothing leaves the test.
// `next/navigation` is stubbed before any import resolves (`mock.module` is
// hoisted), and `TableProviders` supplies the URL state the results table
// needs, as the root layout does.
let pushed: string[] = []
mock.module("next/navigation", () => ({
  usePathname: () => "/query-studio",
  useSearchParams: () => new URLSearchParams(),
  useRouter: () => ({
    push: (href: string) => {
      pushed.push(href)
    },
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
import { AuthProvider, useAuth } from "@/features/auth/auth-provider"
import { CopilotProvider } from "@/features/copilot/use-copilot"
import { assetService, lakehouseService, queryService } from "@/services"
import { ServiceError } from "@/services/errors"
import { queryDraftKey } from "@/lib/query-draft"
import { SQL_FORMAT_SETTINGS_KEY } from "@/lib/sql-format-settings"
import { QueryStudioPage } from "./query-studio-page"
import { readSaveQueryDraft, readSaveSourceDraft } from "./save-draft"

afterEach(() => {
  cleanup()
  mock.restore()
  window.localStorage.clear()
  window.sessionStorage.clear()
  pushed = []
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
function stubNetwork(permissions: string[] = []) {
  spyOn(globalThis, "fetch").mockImplementation((async (input: RequestInfo | URL) => {
    const path = String(input)
    if (path.includes("/api/auth/me")) {
      return json({ id: "u1", name: "Analyst", email: null, roles: ["Analyst"], permissions, tenants: [] })
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
  window.localStorage.setItem(queryDraftKey("u1", null), JSON.stringify({ sql, question: "", tab: "sql" }))
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

/**
 * `UX-9`: the editor's less frequent actions are in ⋯ ("More query
 * actions"); this opens it and finds one of its items.
 */
async function moreItem(name: string): Promise<HTMLElement> {
  fireEvent.click(await screen.findByRole("button", { name: "More query actions" }))
  return screen.findByRole("menuitem", { name })
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
    const runButton = await screen.findByRole("button", { name: "Run" })
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

  // QS-11 (LH-QS-1-04 F2), D11: a permission refusal is not a broken
  // statement; changing the SQL cannot help, so no Fix with AI is offered.
  it("does not offer Fix with AI for a permission refusal", async () => {
    stubNetwork()
    spyOn(queryService, "run").mockRejectedValueOnce(
      new ServiceError("permission_denied", "You may not read this table.")
    )
    seedDraft("SELECT amont FROM t")
    renderPage()

    const runButton = await screen.findByRole("button", { name: "Run" })
    await waitFor(() => expect((runButton as HTMLButtonElement).disabled).toBe(false))
    fireEvent.click(runButton)

    expect(await screen.findByText("You don't have access")).toBeTruthy()
    expect(screen.queryByRole("button", { name: /Fix with AI/ })).toBeNull()
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

    fireEvent.click(await moreItem("Format settings…"))
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

    // The draft is read once the session has answered: wait for its text.
    await waitFor(() => expect(container.querySelector(".cm-line")?.textContent).toBe("select a from t"))
    const formatItem = await moreItem("Format")
    expect(formatItem.getAttribute("aria-disabled")).not.toBe("true")
    fireEvent.click(formatItem)

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

describe("QueryStudioPage saved-query link", () => {
  it("says the query is not available and opens no tab when the caller may not see it", async () => {
    stubNetwork()
    spyOn(queryService, "getSaved").mockRejectedValue(
      new ServiceError("not_found", "That saved query could not be found.")
    )
    seedDraft("SELECT 1")
    visit("/query-studio?saved=q1")
    renderPage()

    await screen.findByText("This query is not available to you.")
    // No tab was opened for it: the tab bar still holds the draft's one. The
    // wait is for the draft, which is read once the session has answered.
    await waitFor(() =>
      expect(screen.getAllByRole("button", { name: /^Query \d+$/ }).length).toBe(1)
    )
  })
})

describe("QueryStudioPage estimate panel", () => {
  it("says there is no estimate for a Trino tab and leaves the Estimate button with nothing to do (LH-QS-4-08)", async () => {
    stubNetwork()
    visit("/query-studio?sql=SELECT%201&engine=trino")
    const estimateSpy = queryService.estimate as unknown as { mock: { calls: unknown[] } }
    renderPage()

    // UX-9: the status bar says it in one line, and the Plan tab in full.
    expect(await screen.findByText("No pre-run estimate for Trino")).toBeTruthy()
    fireEvent.click(screen.getByRole("tab", { name: "Plan" }))
    expect(await screen.findByText("No pre-run estimate for Trino queries.")).toBeTruthy()
    expect(screen.queryByText("This query cannot be planned")).toBeNull()
    expect(screen.queryByText("Type SQL for a pre-run estimate.")).toBeNull()
    // The command menu's "Estimate again" has nothing to do either.
    fireEvent.keyDown(window, { key: "p", ctrlKey: true, shiftKey: true })
    const estimate = await screen.findByRole("option", { name: "Estimate again" })
    expect(estimate.getAttribute("aria-disabled")).toBe("true")
    expect(estimate.getAttribute("title")).toBe("No pre-run estimate for Trino queries.")
    await new Promise((resolve) => setTimeout(resolve, 700))
    expect(estimateSpy.mock.calls.length).toBe(0)
  })

  it("names the parameter the estimate waits for (QS-5-AC14)", async () => {
    stubNetwork()
    seedDraft("SELECT {{region}}")
    renderPage()

    expect(await screen.findByText("Fill in {{region}} for an estimate")).toBeTruthy()
    fireEvent.click(screen.getByRole("tab", { name: "Plan" }))
    expect(await screen.findByText("Fill in {{region}} for a pre-run estimate.")).toBeTruthy()
    expect(screen.queryByText("Type SQL for a pre-run estimate.")).toBeNull()
  })
})

describe("QueryStudioPage statements", () => {
  it("disables the actions that work on one statement when the tab holds several", async () => {
    stubNetwork()
    seedDraft("SELECT 1; SELECT 2")
    renderPage()

    // The draft is read once the session has answered: Run all says it is in.
    await screen.findByRole("button", { name: "Run all" })
    const save = screen.getByRole("button", { name: "Save" })
    expect((save as HTMLButtonElement).disabled).toBe(true)
    expect(save.getAttribute("title")).toBe("Select one statement")

    const explain = await moreItem("Explain with AI")
    expect(explain.getAttribute("aria-disabled")).toBe("true")
    expect(explain.getAttribute("title")).toBe("Select one statement")
    fireEvent.keyDown(document.activeElement ?? window, { key: "Escape" })

    fireEvent.keyDown(window, { key: "p", ctrlKey: true, shiftKey: true })
    const estimate = await screen.findByRole("option", { name: "Estimate again" })
    expect(estimate.getAttribute("aria-disabled")).toBe("true")
    expect(estimate.getAttribute("title")).toBe("Select one statement")
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

    // UX-9: the side panel is open on a laptop, on its Schema tab.
    expect(await screen.findByText(/needs catalog access/i)).toBeTruthy()

    // The refusal sits beside the editor, not in place of it.
    const runButton = await screen.findByRole("button", { name: "Run" })
    expect(container.querySelector(".cm-content")).not.toBeNull()
    await waitFor(() => expect((runButton as HTMLButtonElement).disabled).toBe(false))
    fireEvent.click(runButton)
    await waitFor(() => expect(run).toHaveBeenCalledTimes(1))
    await screen.findByText(/stopped here/)
  })

  it("hides and shows the Schema and plan panel from the toolbar toggle", async () => {
    stubNetwork()
    seedDraft("SELECT 1")
    renderPage()

    const toggle = await screen.findByRole("button", { name: "Hide schema and plan" })
    expect(toggle.getAttribute("aria-pressed")).toBe("true")
    expect(screen.getByRole("complementary", { name: "Schema and plan" })).toBeTruthy()
    fireEvent.click(toggle)

    expect(screen.queryByRole("complementary", { name: "Schema and plan" })).toBeNull()
    const shown = screen.getByRole("button", { name: "Show schema and plan" })
    expect(shown.getAttribute("aria-pressed")).toBe("false")
    fireEvent.click(shown)
    expect(screen.getByRole("complementary", { name: "Schema and plan" })).toBeTruthy()
  })
})

describe("QueryStudioPage parameters", () => {
  it("shows no field for SQL without a placeholder", async () => {
    stubNetwork()
    seedDraft("SELECT 1")
    renderPage()
    await screen.findByRole("button", { name: "Run" })
    expect(screen.queryByRole("group", { name: "Query parameters" })).toBeNull()
  })

  it("shows a field for each placeholder, between the editor and the status bar, and a field for a name typed later", async () => {
    stubNetwork()
    seedDraft("SELECT {{region}}")
    const { container } = renderPage()
    const fields = await screen.findByRole("group", { name: "Query parameters" })
    const engine = screen.getByRole("button", { name: "Engine: ClickHouse" })
    expect(within(fields).getByLabelText("region")).toBeTruthy()
    // UX-9: the row sits between the editor and the status bar.
    const editor = container.querySelector(".cm-content") as HTMLElement
    expect(editor.compareDocumentPosition(fields) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy()
    expect(fields.compareDocumentPosition(engine) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy()
  })

  it("restores the fields of a reload at the values the tab held", async () => {
    stubNetwork()
    window.localStorage.setItem(
      queryDraftKey("u1", null),
      JSON.stringify({
        tabs: [
          {
            id: "tab-1",
            name: "Query 1",
            sql: "SELECT {{region}}",
            engine: "clickhouse",
            parameters: [{ name: "region", type: "text", default: "EU" }],
            values: { region: "US" },
          },
        ],
        activeId: "tab-1",
        question: "",
        tab: "sql",
      })
    )
    renderPage()
    const fields = await screen.findByRole("group", { name: "Query parameters" })
    expect((within(fields).getByLabelText("region") as HTMLInputElement).value).toBe("US")
  })

  it("shows the server's sentence under the editor and marks the field it names", async () => {
    stubNetwork()
    spyOn(queryService, "run").mockRejectedValue(
      new ServiceError("invalid_request", "Fill in {{region}} to run this query.")
    )
    seedDraft("SELECT {{region}}, {{city}}")
    renderPage()
    const fields = await screen.findByRole("group", { name: "Query parameters" })
    const runButton = screen.getByRole("button", { name: "Run" })
    await waitFor(() => expect((runButton as HTMLButtonElement).disabled).toBe(false))

    fireEvent.click(runButton)

    // The error card adds a hint after the server's sentence.
    expect(await screen.findByText(/^Fill in \{\{region\}\} to run this query\./)).toBeTruthy()
    expect(within(fields).getByLabelText("region").getAttribute("aria-invalid")).toBe("true")
    expect(within(fields).getByLabelText("city").getAttribute("aria-invalid")).toBeNull()

    // Typing a value takes the mark off that field.
    fireEvent.change(within(fields).getByLabelText("region"), { target: { value: "EU" } })
    expect(within(fields).getByLabelText("region").getAttribute("aria-invalid")).toBeNull()
  })

  it("runs with the values typed into the fields", async () => {
    stubNetwork()
    const run = spyOn(queryService, "run").mockResolvedValue(RESULT)
    seedDraft("SELECT {{region}}")
    renderPage()
    const fields = await screen.findByRole("group", { name: "Query parameters" })
    fireEvent.change(within(fields).getByLabelText("region"), { target: { value: "EU" } })

    fireEvent.click(screen.getByRole("button", { name: "Run" }))

    await waitFor(() => expect(run).toHaveBeenCalledTimes(1))
    expect(run.mock.calls[0]?.[1]).toMatchObject({
      parameters: { region: "EU" },
      definitions: [{ name: "region", type: "text" }],
    })
  })

  it("runs with no value for a dropdown whose value was taken away", async () => {
    stubNetwork()
    const run = spyOn(queryService, "run").mockResolvedValue(RESULT)
    window.localStorage.setItem(
      queryDraftKey("u1", null),
      JSON.stringify({
        tabs: [
          {
            id: "tab-1",
            name: "Query 1",
            sql: "SELECT {{region}}",
            engine: "clickhouse",
            parameters: [
              {
                name: "region",
                type: "dropdown",
                options: [
                  { value: "eu", label: "Europe" },
                  { value: "us", label: "United States" },
                ],
              },
            ],
            values: { region: "eu" },
          },
        ],
        activeId: "tab-1",
        question: "",
        tab: "sql",
      })
    )
    renderPage()
    const fields = await screen.findByRole("group", { name: "Query parameters" })
    // The value the tab held is taken away again, the way a person does it.
    fireEvent.click(within(fields).getByLabelText("region"))
    const none = await screen.findByRole("option", { name: "No value" })
    fireEvent.focus(none)
    fireEvent.click(none)

    fireEvent.click(screen.getByRole("button", { name: "Run" }))

    await waitFor(() => expect(run).toHaveBeenCalledTimes(1))
    // The run holds no value for the parameter: the name is not sent at all.
    expect(run.mock.calls[0]?.[1]).toMatchObject({ parameters: {} })
  })
})

// `WEB-7` D5: saving is a page now, so the editor leaves the statement where
// that page reads it and goes there. The editor's own draft keeps the SQL,
// which is what makes Cancel and Save return to the same text.
describe("QueryStudioPage save handoff ('WEB-7' D5)", () => {
  it("writes the statement and the save mode where the save page reads them, and goes there", async () => {
    stubNetwork()
    seedDraft("SELECT 1")
    renderPage()

    // UX-9: a tab that did not come from a saved query has one save, "Save".
    const save = await screen.findByRole("button", { name: "Save" })
    await waitFor(() => expect((save as HTMLButtonElement).disabled).toBe(false))
    fireEvent.click(save)

    await waitFor(() => expect(pushed).toEqual(["/query-studio/save"]))
    expect(readSaveQueryDraft()).toEqual({
      sql: "SELECT 1",
      parameters: [],
      mode: "create",
      savedId: null,
      tabId: "tab-1",
    })
    // The statement travels in the browser session, never in the address.
    expect(window.location.search).toBe("")
  })

  it("writes the SQL source draft for a caller who may author sources", async () => {
    stubNetwork(["dashboard:sql"])
    seedDraft("SELECT * FROM serving.orders")
    renderPage()

    // UX-9: Save as SQL source is in Save's own menu.
    await waitFor(() =>
      expect((screen.getByRole("button", { name: "Save" }) as HTMLButtonElement).disabled).toBe(false)
    )
    fireEvent.click(screen.getByRole("button", { name: "More ways to save" }))
    fireEvent.click(await screen.findByRole("menuitem", { name: "Save as SQL source" }))

    await waitFor(() => expect(pushed).toEqual(["/query-studio/save-source"]))
    expect(readSaveSourceDraft()).toEqual({ sql: "SELECT * FROM serving.orders" })
  })

  it("shows no Save as SQL source to a caller without dashboard:sql", async () => {
    stubNetwork()
    seedDraft("SELECT * FROM serving.orders")
    renderPage()

    await screen.findByRole("button", { name: "Save" })
    // A new tab with no SQL source to save has nothing else in Save's menu,
    // so the menu is not drawn at all.
    expect(screen.queryByRole("button", { name: "More ways to save" })).toBeNull()
  })
})

// `QS-8` (`LH-QS-1-04` F1): the draft belongs to one person in one tenant.
// The session fixture here belongs to two tenants, and switching the active
// one mounts a new workspace over the other tenant's key.
describe("QueryStudioPage draft owner", () => {
  /** The session of a person who belongs to two tenants, `t1` and `t2`. */
  function stubTwoTenants() {
    spyOn(globalThis, "fetch").mockImplementation((async (input: RequestInfo | URL) => {
      const path = String(input)
      if (path.includes("/api/auth/me")) {
        return json({
          id: "u1",
          name: "Analyst",
          email: null,
          roles: ["Analyst"],
          permissions: [],
          tenants: [
            { id: "t1", name: "One", slug: "one" },
            { id: "t2", name: "Two", slug: "two" },
          ],
        })
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

  /** One stored tab under a tenant's key, with the placeholder it holds. */
  function seedTenantTab(tenantId: string, name: string) {
    window.localStorage.setItem(
      queryDraftKey("u1", tenantId),
      JSON.stringify({
        tabs: [
          {
            id: "tab-1",
            name: "Query 1",
            sql: `SELECT {{${name}}}`,
            engine: "clickhouse",
            parameters: [],
            values: {},
          },
        ],
        activeId: "tab-1",
        question: "",
        tab: "sql",
      })
    )
  }

  /** The navbar's tenant switch, rendered beside the page under one provider. */
  function TenantSwitch() {
    const { setActiveTenant } = useAuth()
    return (
      <button type="button" onClick={() => setActiveTenant("t2")}>
        Switch to Two
      </button>
    )
  }

  it("shows the other tenant's tabs, and none of the first, when the active tenant changes", async () => {
    stubTwoTenants()
    seedTenantTab("t1", "first")
    seedTenantTab("t2", "second")
    render(
      <AuthProvider>
        <TableProviders>
          <CopilotProvider>
            <QueryStudioPage />
            <TenantSwitch />
          </CopilotProvider>
        </TableProviders>
      </AuthProvider>,
    )

    // The first tenant's tab is open before the switch, and the second's is not.
    await screen.findByLabelText("first")
    expect(screen.queryByLabelText("second")).toBeNull()

    fireEvent.click(screen.getByRole("button", { name: "Switch to Two" }))

    // The new workspace reads the second tenant's key: its tab replaces the
    // first tenant's in the editor, not beside it.
    await screen.findByLabelText("second")
    expect(screen.queryByLabelText("first")).toBeNull()
    // The first tenant's tabs are still theirs to find on the next switch.
    const firstKey = window.localStorage.getItem(queryDraftKey("u1", "t1"))
    expect(JSON.parse(firstKey ?? "{}").tabs?.[0]?.sql).toBe("SELECT {{first}}")
  })
})

// QS-11 (LH-QS-1-12), D9: a batch over the limit is refused in the page,
// beside Run, with no request sent — not in a toast that vanishes.
describe("QueryStudioPage batch refusal", () => {
  it("shows the statement-limit alert and sends nothing when Run all is pressed", async () => {
    stubNetwork()
    const run = spyOn(queryService, "run").mockResolvedValue(RESULT)
    seedDraft(Array.from({ length: 21 }, (_, i) => `SELECT ${i}`).join("; "))
    renderPage()

    await waitFor(() => expect(screen.getByRole("button", { name: /Run all/ })).toBeTruthy())
    fireEvent.click(screen.getByRole("button", { name: /Run all/ }))

    const alert = await screen.findByRole("alert")
    expect(alert.textContent).toBe(
      "At most 20 statements are run at once. Split the buffer into smaller queries and run them one after another."
    )
    expect(run).not.toHaveBeenCalled()
  })
})

// UX-9 (layout A with mockup B's status bar): the frame, the status bar, the
// command menu, the version chip and Make a chart.
describe("QueryStudioPage UX-9 layout", () => {
  /** A ready ClickHouse estimate, as the API sends it. */
  const ESTIMATE = {
    supported: true,
    queryEngine: "clickhouse",
    estimatedBytes: 1024,
    estimatedCostMin: 0.01,
    estimatedCostMax: 0.02,
    error: null,
    workloadClass: "hot-analytics",
    engine: "hot-store",
    cacheEligible: null,
    freshnessLagSeconds: 3 * 3600,
    policyObligations: [],
    sources: ["serving.orders"],
    plan: null,
  } as never

  it("puts the sections on the title row as links, with no Saved or History card beside the editor", async () => {
    stubNetwork()
    seedDraft("SELECT 1")
    renderPage()

    const sections = await screen.findByRole("navigation", { name: "Query Studio sections" })
    const links = within(sections).getAllByRole("link")
    expect(links.map((link) => link.getAttribute("href"))).toEqual([
      "/query-studio",
      "/query-studio/saved",
      "/query-studio/history",
    ])
    expect(within(sections).getByRole("link", { name: "Studio" }).getAttribute("aria-current")).toBe("page")
    await screen.findByRole("button", { name: "Run" })
    expect(screen.queryByText("Your recent runs.")).toBeNull()
  })

  it("shows the engine and the estimate in the status bar, and the estimate opens the Plan tab", async () => {
    stubNetwork()
    spyOn(queryService, "estimate").mockResolvedValue(ESTIMATE)
    seedDraft("SELECT * FROM serving.orders")
    renderPage()

    expect(await screen.findByRole("button", { name: "Engine: ClickHouse" })).toBeTruthy()
    const estimate = await screen.findByRole("button", { name: /^Estimate ≈ 1\.0 KB · / })
    expect(estimate.textContent).toContain("data 3h 00m old")
    fireEvent.click(estimate)

    const plan = screen.getByRole("tab", { name: "Plan" })
    await waitFor(() => expect(plan.getAttribute("aria-selected")).toBe("true"))
    expect(await screen.findByText("Hot analytics")).toBeTruthy()
  })

  it("switches the engine from the status bar's menu", async () => {
    stubNetwork()
    seedDraft("SELECT 1")
    renderPage()

    fireEvent.click(await screen.findByRole("button", { name: "Engine: ClickHouse" }))
    fireEvent.click(await screen.findByRole("menuitemradio", { name: /^Trino/ }))

    expect(await screen.findByRole("button", { name: "Engine: Trino" })).toBeTruthy()
    expect(screen.getByText("No pre-run estimate for Trino")).toBeTruthy()
  })

  it("opens the command menu with Ctrl+Shift+P, lists the ⋯ actions, and runs the one picked", async () => {
    stubNetwork()
    seedDraft("SELECT 1")
    renderPage()
    await screen.findByRole("button", { name: "Run" })

    fireEvent.keyDown(window, { key: "P", ctrlKey: true, shiftKey: true })
    for (const name of ["Format", "Format settings…", "Explain with AI", "Time travel…", "Keyboard shortcuts"]) {
      expect(await screen.findByRole("option", { name })).toBeTruthy()
    }
    fireEvent.click(screen.getByRole("option", { name: "Keyboard shortcuts" }))

    expect(await screen.findByRole("dialog", { name: "Keyboard shortcuts" })).toBeTruthy()
  })

  it("shows a version pin as a chip and reads the current version again from its ×", async () => {
    stubNetwork()
    seedDraft("SELECT * FROM bronze.orders\nSETTINGS iceberg_snapshot_id = 42")
    const { container } = renderPage()

    expect(await screen.findByText("Version 42")).toBeTruthy()
    fireEvent.click(screen.getByRole("button", { name: "Read the current version" }))

    await waitFor(() => {
      const lines = Array.from(container.querySelectorAll(".cm-line")).map((line) => line.textContent ?? "")
      expect(lines.join("\n")).toBe("SELECT * FROM bronze.orders")
    })
    expect(screen.queryByText("Version 42")).toBeNull()
  })

  it("opens Time travel from the ⋯ menu", async () => {
    stubNetwork()
    seedDraft("SELECT 1")
    renderPage()
    await screen.findByRole("button", { name: "Run" })

    fireEvent.click(await moreItem("Time travel…"))

    expect(await screen.findByRole("dialog", { name: "Time travel" })).toBeTruthy()
    expect(screen.getByText("Query this version")).toBeTruthy()
  })

  it("makes a chart from a result through the Save as SQL source page, with the statement that ran", async () => {
    stubNetwork(["dashboard:sql", "dashboard:write"])
    spyOn(queryService, "run").mockResolvedValue(RESULT)
    seedDraft("SELECT n FROM serving.orders")
    renderPage()

    const runButton = await screen.findByRole("button", { name: "Run" })
    await waitFor(() => expect((runButton as HTMLButtonElement).disabled).toBe(false))
    fireEvent.click(runButton)
    fireEvent.click(await screen.findByRole("button", { name: "More result actions" }))
    fireEvent.click(await screen.findByRole("menuitem", { name: "Make a chart" }))

    await waitFor(() => expect(pushed).toEqual(["/query-studio/save-source"]))
    expect(readSaveSourceDraft()).toEqual({ sql: "SELECT n FROM serving.orders", next: "chart" })
  })

  it("offers no Make a chart to a caller who may not build a chart", async () => {
    stubNetwork(["dashboard:sql"])
    spyOn(queryService, "run").mockResolvedValue(RESULT)
    seedDraft("SELECT n FROM serving.orders")
    renderPage()

    const runButton = await screen.findByRole("button", { name: "Run" })
    await waitFor(() => expect((runButton as HTMLButtonElement).disabled).toBe(false))
    fireEvent.click(runButton)
    fireEvent.click(await screen.findByRole("button", { name: "More result actions" }))

    await screen.findByRole("menuitem", { name: "Column statistics" })
    expect(screen.queryAllByRole("menuitem", { name: "Make a chart" })).toHaveLength(0)
  })

  it("says where results show before anything ran", async () => {
    stubNetwork()
    seedDraft("SELECT 1")
    renderPage()

    expect(await screen.findByText("Results show here")).toBeTruthy()
  })
})
