// The Rotate button on the service
// identities page MUST show the freshly-minted secret in a Dialog exactly
// once, MUST trigger a list refetch so the row's new expiresAt /
// rotationStatus land without a manual reload, MUST never persist the
// secret to localStorage, and MUST stay closed on a failed rotation.

// `AuthProvider` (the parent of `useAuth()`) calls `usePathname()` and
// `useRouter()` at the top of its component (auth-provider.tsx:140-141).
// Both hooks throw or expect a Next.js AppRouter context in a bare
// happy-dom environment, so stub `next/navigation` at the module boundary
// before any import resolves. `mock.module` is hoisted to the top of the
// file by bun's test runner, so the stub is in place before the `import`
// statements below pull in `AuthProvider`.
mock.module("next/navigation", () => ({
  usePathname: () => "/admin/service-identities",
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
import { afterEach, beforeEach, describe, expect, it, mock } from "bun:test"
import { AuthProvider } from "@/features/auth/auth-provider"
import { ServiceIdentitiesPage } from "./service-identities-page"

const originalFetch = global.fetch

type FetchCall = {
  url: string
  method: string
}

let fetchCalls: FetchCall[] = []

const ROW = {
  id: "si-1",
  name: "ingestion-worker",
  scopes: ["ingest:write"],
  environment: "production",
  rotationStatus: "current" as const,
  expiresAt: "2026-06-01T00:00:00.000Z",
  lastUsedAt: null,
}

afterEach(() => {
  // Same caveat as `sessions-page.test.tsx`: testing-library v16 stopped
  // auto-cleaning between tests, so the previous `it`'s
  // `<ServiceIdentitiesPage />` (and its `<Dialog>` portal) would
  // otherwise still be in the DOM when the next `it` rendered —
  // `getByRole("button", { name: /rotate/i })` would then find two
  // matching buttons and throw "Found multiple elements".
  cleanup()
  global.fetch = originalFetch
  fetchCalls = []
  localStorage.clear()
})

beforeEach(() => {
  fetchCalls = []
  localStorage.clear()
})

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  })
}

/**
 * Stubs every fetch the page actually issues:
 * - GET  /api/auth/me             → AuthProvider's load() (we need a
 *                                   principal with `identity:write` so
 *                                   the Rotate button isn't disabled)
 * - GET  /api/identity/service-identities
 *                                 → the table's list call
 * - POST /api/identity/service-identities/{id}/rotate
 *                                 → the rotate action; controlled by
 *                                   `rotateBody` / `rotateStatus`
 *
 * Other paths return an empty 200 so a stray fetch from a future
 * contributor surfaces in the failure message rather than a swallowed
 * `undefined`. Records every call so the test can assert both the
 * POST→GET refetch pairing (behavior 4) and the failed-rotate no-refetch
 * guarantee (behavior 5).
 */
function setupFetch(opts: {
  rotateBody?: unknown
  rotateStatus?: number
}): void {
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

      if (url.includes("/api/auth/me")) {
        return jsonResponse({
          id: "u1",
          name: "Test",
          email: null,
          roles: ["admin"],
          permissions: ["identity:write", "*:*"],
          tenants: [{ id: "t1", name: "Acme", slug: "acme" }],
        })
      }

      if (
        method === "POST" &&
        url.includes("/api/identity/service-identities/si-1/rotate")
      ) {
        const status = opts.rotateStatus ?? 200
        if (status >= 400) {
          return jsonResponse(
            { error: "rotate failed" },
            status,
          )
        }
        return jsonResponse(
          opts.rotateBody ?? {
            secret: "a".repeat(64),
            identity: {
              ...ROW,
              rotationStatus: "current",
              expiresAt: "2027-06-01T00:00:00.000Z",
            },
          },
          status,
        )
      }

      if (url.includes("/api/identity/service-identities")) {
        return jsonResponse([ROW])
      }

      return new Response("{}", { status: 200 })
    },
  ) as unknown as typeof fetch
}

function renderPage() {
  return render(
    <AuthProvider>
      <ServiceIdentitiesPage />
    </AuthProvider>,
  )
}

describe("ServiceIdentitiesPage rotate (new secret shown once, never persisted)", () => {
  it("clicking Rotate calls POST /api/identity/service-identities/{id}/rotate with the row id", async () => {
    setupFetch({})
    renderPage()
    await waitFor(() => expect(screen.getByText("ingestion-worker")).toBeDefined())
    fireEvent.click(screen.getByRole("button", { name: /rotate/i }))
    await waitFor(() => {
      const call = fetchCalls.find(
        (c) =>
          c.method === "POST" &&
          c.url.endsWith("/api/identity/service-identities/si-1/rotate"),
      )
      expect(call).toBeDefined()
    })
  })

  it("the new secret appears in a Dialog exactly once, alongside the 'will not be shown again' warning", async () => {
    setupFetch({
      rotateBody: {
        secret: "a".repeat(64),
        identity: {
          ...ROW,
          rotationStatus: "current",
          expiresAt: "2027-06-01T00:00:00.000Z",
        },
      },
    })
    renderPage()
    await waitFor(() => expect(screen.getByText("ingestion-worker")).toBeDefined())
    fireEvent.click(screen.getByRole("button", { name: /rotate/i }))
    // The secret lands in a `<code data-testid="revealed-secret">` so the
    // assertion reads like "the dialog displays the secret verbatim",
    // not "the dialog has a copyable button" — the design is immediate
    // one-shot display, with the Copy button as a courtesy.
    const secretEl = await screen.findByTestId("revealed-secret")
    expect(secretEl.textContent).toBe("a".repeat(64))
    expect(screen.getByText(/will not be shown again/i)).toBeDefined()
  })

  it("the secret is never written to localStorage", async () => {
    setupFetch({
      rotateBody: {
        secret: "b".repeat(64),
        identity: ROW,
      },
    })
    renderPage()
    await waitFor(() => expect(screen.getByText("ingestion-worker")).toBeDefined())
    fireEvent.click(screen.getByRole("button", { name: /rotate/i }))
    await screen.findByTestId("revealed-secret")
    // The secret must never be persisted client-side beyond the
    // one-shot dialog prop. `JSON.stringify(localStorage)` collapses
    // every key+value into one string; if the secret (or any fragment
    // long enough to be a fingerprint of it) leaked into storage, it
    // would surface here.
    const snapshot = JSON.stringify(localStorage)
    expect(snapshot.includes("b".repeat(64))).toBe(false)
  })

  it("refetches the service identities list after a successful rotate so the row's new expiresAt/rotationStatus are visible", async () => {
    setupFetch({})
    renderPage()
    await waitFor(() => expect(screen.getByText("ingestion-worker")).toBeDefined())
    const getCallsBefore = fetchCalls.filter(
      (c) =>
        c.method === "GET" &&
        c.url.includes("/api/identity/service-identities"),
    ).length
    fireEvent.click(screen.getByRole("button", { name: /rotate/i }))
    await screen.findByTestId("revealed-secret")
    await waitFor(() => {
      const getCallsAfter = fetchCalls.filter(
        (c) =>
          c.method === "GET" &&
          c.url.includes("/api/identity/service-identities"),
      ).length
      expect(getCallsAfter).toBeGreaterThan(getCallsBefore)
    })
  })

  it("a failed rotate (5xx) does NOT open the dialog and does NOT refetch the list", async () => {
    setupFetch({ rotateStatus: 500 })
    renderPage()
    await waitFor(() => expect(screen.getByText("ingestion-worker")).toBeDefined())
    const getCallsBefore = fetchCalls.filter(
      (c) =>
        c.method === "GET" &&
        c.url.includes("/api/identity/service-identities"),
    ).length
    fireEvent.click(screen.getByRole("button", { name: /rotate/i }))
    // Let any in-flight POST + getState() resolve; the dialog should
    // never have mounted, the list should not have been refetched.
    await waitFor(() => {
      const postCalls = fetchCalls.filter(
        (c) =>
          c.method === "POST" &&
          c.url.endsWith("/api/identity/service-identities/si-1/rotate"),
      )
      expect(postCalls.length).toBeGreaterThan(0)
    })
    expect(screen.queryByTestId("revealed-secret")).toBeNull()
    const getCallsAfter = fetchCalls.filter(
      (c) =>
        c.method === "GET" &&
        c.url.includes("/api/identity/service-identities"),
    ).length
    expect(getCallsAfter).toBe(getCallsBefore)
  })
})