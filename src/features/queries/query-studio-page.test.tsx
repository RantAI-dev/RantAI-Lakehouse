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

import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, mock, spyOn } from "bun:test"
import { TableProviders } from "@/components/app-shell/table-providers"
import { AuthProvider } from "@/features/auth/auth-provider"
import { CopilotProvider } from "@/features/copilot/use-copilot"
import { assetService, lakehouseService, queryService } from "@/services"
import { ServiceError } from "@/services/errors"
import { QUERY_DRAFT_KEY } from "@/lib/query-draft"
import { QueryStudioPage } from "./query-studio-page"

afterEach(() => {
  cleanup()
  mock.restore()
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
