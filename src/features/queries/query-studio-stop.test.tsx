// BI-5 (plan D5, feature decision 4): Stop in Query Studio. This renders the real page. Its two providers (`AuthProvider`, `CopilotProvider`) and
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


describe("QueryStudioPage Stop (BI-5)", () => {
  it("sends the run an id, and Stop asks the server to cancel that run, then aborts the request", async () => {
    stubNetwork()
    let seen: { runId?: string; signal?: AbortSignal } = {}
    spyOn(queryService, "run").mockImplementation(((_sql: string, options: { runId?: string }, signal?: AbortSignal) => {
      seen = { runId: options.runId, signal }
      return new Promise(() => {})
    }) as never)
    const cancel = spyOn(queryService, "cancel").mockResolvedValue({ stopped: 1 })
    seedDraft("SELECT sleep(100)")
    renderPage()

    const runButton = await screen.findByRole("button", { name: "Run query" })
    await waitFor(() => expect((runButton as HTMLButtonElement).disabled).toBe(false))
    fireEvent.click(runButton)
    const stop = await screen.findByRole("button", { name: "Stop" })
    expect(seen.runId).toMatch(/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/)
    expect(seen.signal?.aborted).toBe(false)

    fireEvent.click(stop)
    expect(cancel).toHaveBeenCalledTimes(1)
    expect(cancel.mock.calls[0][0]).toBe(seen.runId as string)
    expect(seen.signal?.aborted).toBe(true)
    // The page is usable again: Run is back, Stop is gone.
    await waitFor(() => expect(screen.queryByRole("button", { name: "Stop" })).toBeNull())
  })

  it("still aborts when the server has no cancel route yet", async () => {
    stubNetwork()
    let signal: AbortSignal | undefined
    spyOn(queryService, "run").mockImplementation(((_s: string, _o: unknown, sig?: AbortSignal) => {
      signal = sig
      return new Promise(() => {})
    }) as never)
    spyOn(queryService, "cancel").mockRejectedValue(new ServiceError("not_found", "Not found"))
    seedDraft("SELECT 1")
    renderPage()
    const runButton = await screen.findByRole("button", { name: "Run query" })
    await waitFor(() => expect((runButton as HTMLButtonElement).disabled).toBe(false))
    fireEvent.click(runButton)
    fireEvent.click(await screen.findByRole("button", { name: "Stop" }))
    expect(signal?.aborted).toBe(true)
  })
})
