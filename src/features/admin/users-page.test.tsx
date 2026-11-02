// ADM-3 D6: the invite MUST show the server-generated temporary password
// in a Dialog exactly once, MUST clear it when the dialog closes, MUST
// never persist it to localStorage, and the "Reset password" action MUST
// show a new one the same way.
//
// `AuthProvider` (the parent of `useAuth()`) calls `usePathname()` and
// `useRouter()` at the top of its component (auth-provider.tsx:140-141).
// Both hooks throw or expect a Next.js AppRouter context in a bare
// happy-dom environment, so stub `next/navigation` at the module boundary
// before any import resolves. `mock.module` is hoisted to the top of the
// file by bun's test runner, so the stub is in place before the `import`
// statements below pull in `AuthProvider`.
//
// `useSearchParams` is stubbed too: the Advanced Data Table's
// `useTableUrlState` reads it for page/sort/filter state, and the real
// hook returns `null` outside a Router context, which crashed every test
// in this file at `searchParams.get`.
mock.module("next/navigation", () => ({
  usePathname: () => "/admin/users",
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
import { afterEach, beforeEach, describe, expect, it, mock } from "bun:test"
import { AuthProvider } from "@/features/auth/auth-provider"
import { TableProviders } from "@/components/app-shell/table-providers"
import { UsersPage } from "./users-page"

const originalFetch = global.fetch

type FetchCall = {
  url: string
  method: string
}

let fetchCalls: FetchCall[] = []

// These stand in for the server-generated passwords the API hands out.
// They are deliberately low-entropy and obviously fake: a random-looking
// value here reads as a credential to the secret scanner (`generic-api-key`
// on the `PASSWORD` keyword), and a test fixture must never look like a
// leaked secret. PR #341 gitleaks.
const TEMPORARY_PASSWORD = "test-only-temporary-password"
const RESET_PASSWORD = "test-only-reset-password"

const ROW = {
  id: "11111111-1111-4111-8111-000000000001",
  name: "Rina Wijaya",
  email: "rina@meridian.example",
  status: "active" as const,
  roles: ["Analyst"],
  tenants: ["Meridian Retail"],
  lastActivity: null,
}

afterEach(() => {
  // Same caveat as `sessions-page.test.tsx`: testing-library v16 stopped
  // auto-cleaning between tests, so the previous `it`'s `<UsersPage />`
  // (and its `<Dialog>` portal) would otherwise still be in the DOM when
  // the next `it` rendered — `getByRole("button", { name: /user actions/i })`
  // would then find two matching buttons and throw "Found multiple
  // elements".
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
 * - GET  /api/auth/me            → AuthProvider's load() (we need a
 *                                  principal with `identity:write` so
 *                                  the invite and reset actions are
 *                                  enabled)
 * - GET  /api/identity/users     → the table's list call
 * - POST /api/identity/users     → the invite; the response carries the
 *                                  server-generated temporary password
 * - POST /api/identity/users/{id}/reset-password
 *                                → the reset; the response carries a
 *                                  NEW temporary password
 *
 * Other paths return an empty 200 so a stray fetch from a future
 * contributor surfaces in the failure message rather than a swallowed
 * `undefined`.
 */
function setupFetch(opts: { inviteStatus?: number; resetStatus?: number }): void {
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

      if (method === "POST" && url.endsWith("/reset-password")) {
        const status = opts.resetStatus ?? 200
        if (status >= 400) {
          return jsonResponse({ error: "reset failed" }, status)
        }
        return jsonResponse({ temporaryPassword: RESET_PASSWORD }, status)
      }

      if (method === "POST" && url.endsWith("/api/identity/users")) {
        const status = opts.inviteStatus ?? 200
        if (status >= 400) {
          return jsonResponse({ error: "invite failed" }, status)
        }
        return jsonResponse(
          { ...ROW, id: "22222222-2222-4222-8222-000000000002", temporaryPassword: TEMPORARY_PASSWORD },
          status
        )
      }

      if (url.includes("/api/identity/users")) {
        return jsonResponse([ROW])
      }

      return new Response("{}", { status: 200 })
    }
  ) as unknown as typeof fetch
}

function renderPage() {
  // `TableProviders` (normally mounted once in the root layout) supplies
  // the `NuqsAdapter` the Advanced Data Table's URL-backed state needs —
  // without it, `useTableMemory`'s `useQueryStates` throws "nuqs requires
  // an adapter" outside a real Next.js app tree.
  return render(
    <TableProviders>
      <AuthProvider>
        <UsersPage />
      </AuthProvider>
    </TableProviders>
  )
}

/** Fill the invite sheet and submit it. */
async function inviteUser() {
  fireEvent.click(screen.getByRole("button", { name: /invite user/i }))
  fireEvent.change(await screen.findByLabelText("Name"), {
    target: { value: "Rina Guest" },
  })
  fireEvent.change(screen.getByLabelText("Email"), {
    target: { value: "rina.guest@meridian.example" },
  })
  fireEvent.click(screen.getByRole("button", { name: "Invite" }))
}

/** Open the row actions menu and click "Reset password". */
async function resetPassword() {
  // The row menu is a Radix dropdown (`@rantai/design-system/ui/dropdown-menu`),
  // and it opens on the pointer PRESS, not on the click that follows it —
  // a bare `fireEvent.click` is dropped as a stray mouse click.
  fireEvent.pointerDown(screen.getByRole("button", { name: /user actions/i }))
  const item = await screen.findByRole("menuitem", { name: /reset password/i })
  fireEvent.click(item)
}

describe("UsersPage invite (temporary password shown once, never persisted)", () => {
  it("the invite shows the temporary password in a Dialog alongside the 'shown once' warning", async () => {
    setupFetch({})
    renderPage()
    await waitFor(() => expect(screen.getByText("Rina Wijaya")).toBeDefined())
    await inviteUser()

    // The password lands in a `<code data-testid="revealed-password">` so
    // the assertion reads like "the dialog displays the password
    // verbatim", not "the dialog has a copyable button" — the design is
    // immediate one-shot display, with the Copy button as a courtesy.
    const password = await screen.findByTestId("revealed-password")
    expect(password.textContent).toBe(TEMPORARY_PASSWORD)
    expect(screen.getByText(/shown once/i)).toBeDefined()
    expect(screen.getByText(/must change it at first sign-in/i)).toBeDefined()
  })

  it("the invite calls POST /api/identity/users and then refetches the list", async () => {
    setupFetch({})
    renderPage()
    await waitFor(() => expect(screen.getByText("Rina Wijaya")).toBeDefined())
    const getsBefore = fetchCalls.filter(
      (c) => c.method === "GET" && c.url.includes("/api/identity/users")
    ).length
    await inviteUser()
    await screen.findByTestId("revealed-password")
    const post = fetchCalls.find(
      (c) => c.method === "POST" && c.url.endsWith("/api/identity/users")
    )
    expect(post).toBeDefined()
    await waitFor(() => {
      const getsAfter = fetchCalls.filter(
        (c) => c.method === "GET" && c.url.includes("/api/identity/users")
      ).length
      expect(getsAfter).toBeGreaterThan(getsBefore)
    })
  })

  it("the password is gone once the dialog is closed", async () => {
    setupFetch({})
    renderPage()
    await waitFor(() => expect(screen.getByText("Rina Wijaya")).toBeDefined())
    await inviteUser()
    await screen.findByTestId("revealed-password")

    fireEvent.click(screen.getByRole("button", { name: "Close" }))
    await waitFor(() => {
      expect(screen.queryByTestId("revealed-password")).toBeNull()
    })
    expect(document.body.textContent).not.toContain(TEMPORARY_PASSWORD)
  })

  it("the password is never written to localStorage", async () => {
    setupFetch({})
    renderPage()
    await waitFor(() => expect(screen.getByText("Rina Wijaya")).toBeDefined())
    await inviteUser()
    await screen.findByTestId("revealed-password")
    // The password must never be persisted client-side beyond the
    // one-shot dialog prop. `JSON.stringify(localStorage)` collapses
    // every key+value into one string; if the password (or any fragment
    // long enough to be a fingerprint of it) leaked into storage, it
    // would surface here.
    const snapshot = JSON.stringify(localStorage)
    expect(snapshot.includes(TEMPORARY_PASSWORD)).toBe(false)
  })

  it("a failed invite does NOT open the dialog", async () => {
    setupFetch({ inviteStatus: 500 })
    renderPage()
    await waitFor(() => expect(screen.getByText("Rina Wijaya")).toBeDefined())
    await inviteUser()
    await waitFor(() => {
      const posts = fetchCalls.filter(
        (c) => c.method === "POST" && c.url.endsWith("/api/identity/users")
      )
      expect(posts.length).toBeGreaterThan(0)
    })
    expect(screen.queryByTestId("revealed-password")).toBeNull()
  })
})

describe("UsersPage reset password (a new password shown once)", () => {
  it("the reset action calls POST /api/identity/users/{id}/reset-password with the row id and shows the new password", async () => {
    setupFetch({})
    renderPage()
    await waitFor(() => expect(screen.getByText("Rina Wijaya")).toBeDefined())
    await resetPassword()

    const password = await screen.findByTestId("revealed-password")
    expect(password.textContent).toBe(RESET_PASSWORD)
    expect(screen.getByText(/shown once/i)).toBeDefined()
    const call = fetchCalls.find(
      (c) =>
        c.method === "POST" &&
        c.url.endsWith(`/api/identity/users/${ROW.id}/reset-password`)
    )
    expect(call).toBeDefined()
  })

  it("a failed reset does NOT open the dialog", async () => {
    setupFetch({ resetStatus: 500 })
    renderPage()
    await waitFor(() => expect(screen.getByText("Rina Wijaya")).toBeDefined())
    await resetPassword()
    await waitFor(() => {
      const posts = fetchCalls.filter(
        (c) => c.method === "POST" && c.url.endsWith("/reset-password")
      )
      expect(posts.length).toBeGreaterThan(0)
    })
    expect(screen.queryByTestId("revealed-password")).toBeNull()
  })
})
