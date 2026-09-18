import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"
import { SessionsPage } from "./sessions-page"

const originalFetch = global.fetch

type FetchCall = {
  url: string
  method: string
}

let fetchCalls: FetchCall[] = []

afterEach(() => {
  // `@testing-library/react` v16 stopped auto-cleaning between tests, so
  // the previous `it`'s `<SessionsPage />` would otherwise still be in
  // the DOM when the next `it` rendered — `getByRole("button", { name:
  // /revoke/i })` would then find two matching buttons and throw
  // "Found multiple elements". The sso and connector tests don't hit
  // this because their assertions are text-based and never exact-match
  // duplicates; the Revoke button is the first role-and-name query in
  // the suite.
  cleanup()
  global.fetch = originalFetch
  fetchCalls = []
})

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  })
}

/**
 * Stubs `GET /api/auth/sessions` to `sessionsBody` and
 * `DELETE /api/auth/sessions/<id>` to an empty 204 — the two routes
 * `SessionsPage` actually exercises. Every other path returns an empty
 * 200 so a stray fetch surfaces in the failure message rather than a
 * swallowed `undefined`. Records every call so the test can assert the
 * page refetches the list after a successful revoke (the "no stale row"
 * guarantee from `useService`'s `reload()`).
 */
function mockSessions(sessionsBody: unknown): void {
  fetchCalls = []
  global.fetch = mock(
    async (input: RequestInfo | URL, init?: RequestInit) => {
      const url =
        typeof input === "string"
          ? input
          : input instanceof URL
            ? input.toString()
            : input.url
      const method = init?.method ?? "GET"
      fetchCalls.push({ url, method })
      if (url.includes("/api/auth/sessions") && method === "DELETE") {
        return new Response(null, { status: 204 })
      }
      if (url.includes("/api/auth/sessions") && method === "GET") {
        return jsonResponse(sessionsBody)
      }
      return new Response("{}", { status: 200 })
    },
  ) as unknown as typeof fetch
}

describe("SessionsPage (WS8 §Phase F)", () => {
  it("lists sessions and revokes one on click", async () => {
    mockSessions([
      {
        id: "s1",
        userId: "u1",
        userName: "Rina Wijaya",
        createdAt: "2026-01-01T00:00:00.000Z",
        expiresAt: "2026-01-02T00:00:00.000Z",
        createdIp: null,
        userAgent: null,
      },
    ])
    render(<SessionsPage />)
    await waitFor(() => expect(screen.getByText("Rina Wijaya")).toBeDefined())
    fireEvent.click(screen.getByRole("button", { name: /revoke/i }))
    await waitFor(() => {
      const deleteCall = fetchCalls.find(
        (c) => c.method === "DELETE" && c.url.endsWith("/api/auth/sessions/s1"),
      )
      expect(deleteCall).toBeDefined()
    })
  })

  it("refreshes the list after a successful revoke so a stale row does not linger", async () => {
    mockSessions([
      {
        id: "s1",
        userId: "u1",
        userName: "Rina Wijaya",
        createdAt: "2026-01-01T00:00:00.000Z",
        expiresAt: "2026-01-02T00:00:00.000Z",
        createdIp: null,
        userAgent: null,
      },
    ])
    render(<SessionsPage />)
    await waitFor(() => expect(screen.getByText("Rina Wijaya")).toBeDefined())
    const callsBeforeRevoke = fetchCalls.filter(
      (c) => c.url.includes("/api/auth/sessions") && c.method === "GET",
    ).length
    fireEvent.click(screen.getByRole("button", { name: /revoke/i }))
    await waitFor(() => {
      const callsAfterRevoke = fetchCalls.filter(
        (c) => c.url.includes("/api/auth/sessions") && c.method === "GET",
      ).length
      expect(callsAfterRevoke).toBeGreaterThan(callsBeforeRevoke)
    })
  })

  it("shows an ErrorState when the list call fails", async () => {
    global.fetch = mock(async () => {
      throw new Error("boom")
    }) as unknown as typeof fetch
    render(<SessionsPage />)
    // `<ErrorState>` renders a `role="status"` shell (see
    // `components/patterns/page-states.tsx`), not `role="alert"`; this is
    // what the test asserts on so it stays in sync with the shared
    // pattern rather than a one-off variant.
    await waitFor(() => expect(screen.getByRole("status")).toBeDefined())
  })
})