// `BI-40` T4 (D5, D6): the alerts page tests ONE rule, through
// `/api/alerts/test`. "Run all now" is gone — the whole tick is the
// scheduler's, and a person's session cannot call it — and a refusal is shown
// on the page, never read as a lost session (`apiFetch` only redirects on a 401,
// and this route answers 403/404).
//
// `AuthProvider` and the Advanced Data Table both need a Next.js AppRouter
// context (`next/navigation`), so stub it at the module boundary before any
// import resolves, exactly as `features/admin/users-page.test.tsx` does.

mock.module("next/navigation", () => ({
  usePathname: () => "/alerts",
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
import { afterEach, describe, expect, it, mock } from "bun:test"
import { AuthProvider } from "@/features/auth/auth-provider"
import { TableProviders } from "@/components/app-shell/table-providers"
import { AlertsPage } from "./alerts-page"

const originalFetch = global.fetch

let fetchCalls: { url: string; method: string }[] = []

afterEach(() => {
  cleanup()
  global.fetch = originalFetch
  fetchCalls = []
  localStorage.clear()
})

const RULE = {
  id: "al_1234",
  name: "Visitors in Bali",
  type: "alert" as const,
  mart: "mart_wisman",
  measure: "jumlah",
  agg: "sum",
  op: ">",
  threshold: 100,
  channel: "email" as const,
  target: "sari@meridian.example",
  enabled: true,
}

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  })
}

/** Every fetch the page issues; `testStatus` answers `POST /api/alerts/test`. */
function setupFetch(
  testStatus: number,
  permissions: string[] = ["dashboard:read", "alert:write"]
): void {
  fetchCalls = []
  global.fetch = mock(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = typeof input === "string" ? input : input instanceof URL ? input.toString() : input.url
    const method = init?.method ?? "GET"
    fetchCalls.push({ url, method })

    if (url.includes("/api/auth/me")) {
      return jsonResponse({
        id: "u1",
        name: "Sari",
        email: "sari@meridian.example",
        roles: ["Analyst"],
        permissions,
        tenants: [{ id: "t1", name: "Acme", slug: "acme" }],
      })
    }
    if (method === "POST" && url.startsWith("/api/alerts/test")) {
      if (testStatus >= 400) {
        return jsonResponse({ error: "permission_denied: alert:write" }, testStatus)
      }
      return jsonResponse({ ran: 1, results: [] })
    }
    if (url.includes("/api/alerts/status")) {
      return jsonResponse({ runEventsConfigured: true })
    }
    if (url.includes("/api/alerts")) {
      return jsonResponse({ rules: [RULE] })
    }
    if (url.includes("/api/dashboard/boards")) {
      return jsonResponse({ boards: [] })
    }
    if (url.includes("/api/dashboard/fields")) {
      return jsonResponse({ marts: [], measures: [] })
    }
    if (url.includes("/api/dashboard/metrics")) {
      return jsonResponse({ metrics: [] })
    }
    if (url.includes("/api/connectors")) {
      return jsonResponse([])
    }
    return jsonResponse({})
  }) as unknown as typeof fetch
}

function renderPage() {
  return render(
    <TableProviders>
      <AuthProvider>
        <AlertsPage />
      </AuthProvider>
    </TableProviders>
  )
}

/** A console address with a recording `assign`, so a redirect is observable. */
function withRecordedLocation(assign: (url: string) => void) {
  const original = window.location
  Object.defineProperty(window, "location", {
    configurable: true,
    writable: true,
    value: { ...original, pathname: "/alerts", search: "", assign },
  })
  return () => {
    Object.defineProperty(window, "location", {
      configurable: true,
      writable: true,
      value: original,
    })
  }
}

describe("the alerts page tests one rule", () => {
  it("has no 'Run all now' and posts the row's Test run now to /api/alerts/test", async () => {
    setupFetch(200)
    renderPage()
    await waitFor(() => expect(screen.getByText("Visitors in Bali")).toBeDefined())

    expect(screen.queryByRole("button", { name: /run all now/i })).toBeNull()

    fireEvent.click(screen.getByRole("button", { name: /test run now/i }))
    await waitFor(() =>
      expect(
        fetchCalls.some(
          (call) => call.method === "POST" && call.url === "/api/alerts/test?id=al_1234"
        )
      ).toBe(true)
    )
    expect(fetchCalls.some((call) => call.url.startsWith("/api/alerts/run"))).toBe(false)
  })

  it("shows the permission in words on a 403 and does not navigate away", async () => {
    setupFetch(403)
    const assigned: string[] = []
    const restore = withRecordedLocation((url) => assigned.push(url))
    try {
      renderPage()
      await waitFor(() => expect(screen.getByText("Visitors in Bali")).toBeDefined())

      fireEvent.click(screen.getByRole("button", { name: /test run now/i }))

      await waitFor(() =>
        expect(screen.getByText("You can test only the alerts you manage.")).toBeDefined()
      )
      expect(assigned).toEqual([])
    } finally {
      restore()
    }
  })
})

// QA TC-1696 (SRC-40 D9): "New rule" is offered only to a person who may
// create a rule. The page it opens takes `alert:write`, and the API refuses
// the save for anyone else.
describe("New rule needs alert:write", () => {
  it("offers New rule only to a person with alert:write", async () => {
    setupFetch(200, ["dashboard:read", "alert:read"])
    renderPage()
    await waitFor(() => expect(screen.getByText("Visitors in Bali")).toBeDefined())
    expect(screen.queryByRole("button", { name: "New rule" })).toBeNull()

    cleanup()
    setupFetch(200)
    renderPage()
    await waitFor(() => expect(screen.getByText("Visitors in Bali")).toBeDefined())
    expect(screen.getByRole("button", { name: "New rule" })).toBeDefined()
  })
})
