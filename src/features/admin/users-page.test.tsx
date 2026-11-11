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
// WEB-3 S3-D7: `search` is the address's query, so a test can open `?tab=sessions`.
let search = new URLSearchParams()
mock.module("next/navigation", () => ({
  usePathname: () => "/admin/users",
  useSearchParams: () => search,
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
import type { User } from "@/services"
import { UsersPage } from "./users-page"

const originalFetch = global.fetch

type FetchCall = {
  url: string
  method: string
  /** The request body, when there was one (ADM-4 D1's role change). */
  body?: string
}

let fetchCalls: FetchCall[] = []

// These stand in for the server-generated passwords the API hands out.
// They are deliberately low-entropy and obviously fake: a random-looking
// value here reads as a credential to the secret scanner (`generic-api-key`
// on the `PASSWORD` keyword), and a test fixture must never look like a
// leaked secret. PR #341 gitleaks.
const TEMPORARY_PASSWORD = "test-only-temporary-password"
const RESET_PASSWORD = "test-only-reset-password"

// The row every test starts from. Typed as the contract's `User` so a test
// can flip `status` (ADM-4 D3's inactive user) without widening a literal.
const ROW: User = {
  id: "11111111-1111-4111-8111-000000000001",
  name: "Rina Wijaya",
  username: "rina",
  email: "rina@meridian.example",
  status: "active",
  roles: ["Analyst"],
  tenants: ["Meridian Retail"],
  lastActivity: null,
}

/**
 * The roles that exist (ADM-4 D7: the role editor offers every role the
 * deployment has, not only the ones this user holds).
 */
const ROLES = [
  {
    id: "22222222-2222-4222-8222-000000000001",
    name: "Analyst",
    members: 24,
    permissions: "query:read",
    description: "Read governed data.",
  },
  {
    id: "22222222-2222-4222-8222-000000000002",
    name: "Approver",
    members: 4,
    permissions: "agent:approve",
    description: "Approve agent actions.",
  },
]

/**
 * The tenants that exist (ADM-4 D2/D7: the user carries tenant NAMES,
 * and the route needs the id behind each one).
 */
const TENANTS = [
  {
    id: "11111111-1111-4111-8111-000000000002",
    name: "Meridian Retail",
    slug: "meridian-retail",
    plan: "Enterprise",
    residency: "Jakarta (ID)",
    users: 3,
    agents: 0,
    storageBytes: 0,
    quotaCompute: 0,
    usedCompute: 0,
  },
]

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
  search = new URLSearchParams()
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
 * - GET  /api/identity/roles     → the role editor's list (ADM-4 D7)
 * - GET  /api/identity/tenants   → the tenant remover's list (ADM-4 D2)
 * - POST /api/identity/users     → the invite; the response carries the
 *                                  server-generated temporary password
 * - POST /api/identity/users/{id}/reset-password
 *                                → the reset; the response carries a
 *                                  NEW temporary password
 * - PUT  /api/identity/users/{id}/roles
 *                                → the role change (ADM-4 D1)
 * - DELETE /api/identity/users/{id}/tenants/{tenant_id}
 *                                → the tenant removal (ADM-4 D2)
 * - POST /api/identity/users/{id}/deactivate and /reactivate
 *                                → the status switch (ADM-4 D3)
 *
 * Other paths return an empty 200 so a stray fetch from a future
 * contributor surfaces in the failure message rather than a swallowed
 * `undefined`.
 */
function setupFetch(opts: {
  inviteStatus?: number
  resetStatus?: number
  rolesStatus?: number
  tenantsStatus?: number
  setRolesStatus?: number
  deactivateStatus?: number
  row?: typeof ROW
} = {}): void {
  const row = opts.row ?? ROW
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
      fetchCalls.push({
        url,
        method,
        body: init?.body === undefined ? undefined : String(init.body),
      })

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

      // ADM-4 D1: the role change. The body carries the user's COMPLETE
      // role set, so the answer echoes it back.
      if (method === "PUT" && url.includes("/roles")) {
        const status = opts.setRolesStatus ?? 200
        if (status >= 400) {
          return jsonResponse({ error: "role change failed" }, status)
        }
        const body = JSON.parse(String(init?.body ?? "{}")) as { roles?: string[] }
        return jsonResponse({ ...row, roles: body.roles ?? row.roles }, status)
      }

      // ADM-4 D2: the tenant removal. The answer no longer lists the
      // tenant.
      if (method === "DELETE" && url.includes("/tenants/")) {
        return jsonResponse({ ...row, tenants: [] }, 200)
      }

      // ADM-4 D3: the status switch.
      if (method === "POST" && url.endsWith("/deactivate")) {
        const status = opts.deactivateStatus ?? 200
        if (status >= 400) {
          return jsonResponse({ error: "deactivate failed" }, status)
        }
        return jsonResponse({ ...row, status: "inactive" }, status)
      }
      if (method === "POST" && url.endsWith("/reactivate")) {
        return jsonResponse({ ...row, status: "active" }, 200)
      }

      if (method === "GET" && url.includes("/api/identity/roles")) {
        const status = opts.rolesStatus ?? 200
        if (status >= 400) {
          return jsonResponse({ error: "roles failed" }, status)
        }
        return jsonResponse(ROLES, status)
      }
      if (method === "GET" && url.includes("/api/identity/tenants")) {
        const status = opts.tenantsStatus ?? 200
        if (status >= 400) {
          return jsonResponse({ error: "tenants failed" }, status)
        }
        return jsonResponse(TENANTS, status)
      }

      if (url.includes("/api/auth/password-rules")) {
        return jsonResponse({ minLength: 8, maxLength: 1024 })
      }

      if (method === "PATCH" && url.includes("/api/identity/users/")) {
        const body = JSON.parse(String(init?.body ?? "{}")) as {
          username?: string
          email?: string | null
        }
        return jsonResponse({ ...row, ...body }, 200)
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
        const body = JSON.parse(String(init?.body ?? "{}")) as {
          name?: string
          username?: string
          email?: string | null
          roles?: string[]
          tenants?: string[]
          password?: { mode?: string; value?: string }
          mustChangePassword?: boolean
        }
        const tempPassword =
          body.password?.mode === "set" ? null : TEMPORARY_PASSWORD
        return jsonResponse(
          {
            ...ROW,
            id: "22222222-2222-4222-8222-000000000002",
            name: body.name ?? "New User",
            username: body.username ?? "newuser",
            email: body.email ?? null,
            temporaryPassword: tempPassword,
          },
          status
        )
      }

      if (url.includes("/api/identity/users")) {
        return jsonResponse([row])
      }

      // WEB-3 S3-D7: the Sessions tab's list.
      if (method === "GET" && url.includes("/api/auth/sessions")) {
        return jsonResponse([
          {
            id: "s1",
            userId: "u1",
            userName: "Sesi Pengguna",
            createdAt: "2026-01-01T00:00:00.000Z",
            expiresAt: "2026-01-02T00:00:00.000Z",
            createdIp: null,
            userAgent: null,
          },
        ])
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
async function inviteUser(opts: {
  name?: string
  username?: string
  email?: string
  roles?: string[]
  tenants?: string[]
  passwordMode?: "generate" | "set"
  password?: string
  mustChangePassword?: boolean
} = {}) {
  fireEvent.click(screen.getByRole("button", { name: /invite user/i }))
  fireEvent.change(await screen.findByLabelText("Name"), {
    target: { value: opts.name ?? "Rina Guest" },
  })
  fireEvent.change(screen.getByLabelText("Username"), {
    target: { value: opts.username ?? "rinaguest" },
  })
  if (opts.email !== undefined) {
    fireEvent.change(screen.getByLabelText("Email (optional)"), {
      target: { value: opts.email },
    })
  } else {
    fireEvent.change(screen.getByLabelText("Email (optional)"), {
      target: { value: "rina.guest@meridian.example" },
    })
  }
  if (opts.roles) {
    for (const r of opts.roles) {
      fireEvent.click(screen.getByLabelText(r))
    }
  }
  if (opts.tenants) {
    for (const t of opts.tenants) {
      fireEvent.click(screen.getByLabelText(t))
    }
  }
  if (opts.passwordMode === "set") {
    fireEvent.click(screen.getByLabelText("Set it now"))
    if (opts.password) {
      fireEvent.change(screen.getByLabelText("Password"), {
        target: { value: opts.password },
      })
    }
  }
  if (opts.mustChangePassword === false) {
    fireEvent.click(
      screen.getByLabelText("Must change password at first sign-in")
    )
  }
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

/** Open the row actions menu and click the item whose name matches. */
async function openRowAction(name: RegExp) {
  fireEvent.pointerDown(screen.getByRole("button", { name: /user actions/i }))
  const item = await screen.findByRole("menuitem", { name })
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
    expect(screen.getAllByText(/shown once/i).length).toBeGreaterThan(0)
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

  it("setting password upfront sends password mode set and does NOT open the revealed-password dialog", async () => {
    setupFetch({})
    renderPage()
    await waitFor(() => expect(screen.getByText("Rina Wijaya")).toBeDefined())
    await inviteUser({
      name: "User With Password",
      username: "userpass",
      passwordMode: "set",
      password: "customPassword123!",
      mustChangePassword: false,
    })

    await waitFor(() => {
      const post = fetchCalls.find(
        (c) => c.method === "POST" && c.url.endsWith("/api/identity/users")
      )
      expect(post).toBeDefined()
      const body = JSON.parse(post?.body ?? "{}")
      expect(body.password).toEqual({ mode: "set", value: "customPassword123!" })
      expect(body.mustChangePassword).toBe(false)
    })
    expect(screen.queryByTestId("revealed-password")).toBeNull()
  })

  it("invite sends full payload with selected roles and tenants as checkboxes", async () => {
    setupFetch({})
    renderPage()
    await waitFor(() => expect(screen.getByText("Rina Wijaya")).toBeDefined())
    await inviteUser({
      name: "Role User",
      username: "roleuser",
      roles: ["Analyst"],
      tenants: ["Meridian Retail"],
    })

    await waitFor(() => {
      const post = fetchCalls.find(
        (c) => c.method === "POST" && c.url.endsWith("/api/identity/users")
      )
      expect(post).toBeDefined()
      const body = JSON.parse(post?.body ?? "{}")
      expect(body.roles).toEqual(["Analyst"])
      expect(body.tenants).toEqual(["Meridian Retail"])
      expect(body.password).toEqual({ mode: "generate" })
      expect(body.mustChangePassword).toBe(true)
    })
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

describe("UsersPage edit roles (ADM-4 D1/D7)", () => {
  it("the role editor lists every role, ticks the ones the user holds, and saves the whole set", async () => {
    setupFetch({})
    renderPage()
    await waitFor(() => expect(screen.getByText("Rina Wijaya")).toBeDefined())
    await openRowAction(/edit roles/i)

    // The editor offers every role the deployment has...
    const analyst = await screen.findByLabelText("Analyst")
    const approver = await screen.findByLabelText("Approver")
    // ...with the ones the user already holds ticked.
    expect(analyst.getAttribute("aria-checked")).toBe("true")
    expect(approver.getAttribute("aria-checked")).toBe("false")

    fireEvent.click(approver)
    fireEvent.click(screen.getByRole("button", { name: /save roles/i }))

    await waitFor(() => {
      const put = fetchCalls.find(
        (c) => c.method === "PUT" && c.url.endsWith(`/api/identity/users/${ROW.id}/roles`)
      )
      expect(put).toBeDefined()
      // The body carries the user's COMPLETE role set, not a delta.
      expect(JSON.parse(put?.body ?? "{}")).toEqual({ roles: ["Analyst", "Approver"] })
    })
  })

  it("a failed role change leaves the dialog open", async () => {
    setupFetch({ setRolesStatus: 500 })
    renderPage()
    await waitFor(() => expect(screen.getByText("Rina Wijaya")).toBeDefined())
    await openRowAction(/edit roles/i)
    await screen.findByLabelText("Analyst")
    fireEvent.click(screen.getByRole("button", { name: /save roles/i }))

    await waitFor(() => {
      const puts = fetchCalls.filter(
        (c) => c.method === "PUT" && c.url.includes("/roles")
      )
      expect(puts.length).toBeGreaterThan(0)
    })
    expect(screen.getByRole("button", { name: /save roles/i })).toBeDefined()
  })
})

describe("UsersPage remove from tenant (ADM-4 D2/D7)", () => {
  it("the tenant remover names the user's tenants and removes the one pressed", async () => {
    setupFetch({})
    renderPage()
    await waitFor(() => expect(screen.getByText("Rina Wijaya")).toBeDefined())
    await openRowAction(/remove from tenant/i)

    const remove = await screen.findByRole("button", { name: "Remove" })
    fireEvent.click(remove)

    await waitFor(() => {
      const del = fetchCalls.find(
        (c) =>
          c.method === "DELETE" &&
          c.url.endsWith(
            `/api/identity/users/${ROW.id}/tenants/${TENANTS[0].id}`
          )
      )
      expect(del).toBeDefined()
    })
  })
})

describe("UsersPage deactivate and reactivate (ADM-4 D3/D7)", () => {
  it("deactivating asks for confirmation first, and then switches the user off", async () => {
    setupFetch({})
    renderPage()
    await waitFor(() => expect(screen.getByText("Rina Wijaya")).toBeDefined())
    await openRowAction(/deactivate/i)

    // The confirmation comes first: nothing is called before it.
    const confirm = await screen.findByRole("button", { name: "Deactivate" })
    expect(
      fetchCalls.some((c) => c.method === "POST" && c.url.endsWith("/deactivate"))
    ).toBe(false)

    fireEvent.click(confirm)
    await waitFor(() => {
      const post = fetchCalls.find(
        (c) => c.method === "POST" && c.url.endsWith(`/api/identity/users/${ROW.id}/deactivate`)
      )
      expect(post).toBeDefined()
    })
  })

  it("an inactive user is offered Reactivate instead", async () => {
    setupFetch({ row: { ...ROW, status: "inactive" } })
    renderPage()
    await waitFor(() => expect(screen.getByText("Rina Wijaya")).toBeDefined())
    await openRowAction(/reactivate/i)

    const confirm = await screen.findByRole("button", { name: "Reactivate" })
    fireEvent.click(confirm)
    await waitFor(() => {
      const post = fetchCalls.find(
        (c) => c.method === "POST" && c.url.endsWith(`/api/identity/users/${ROW.id}/reactivate`)
      )
      expect(post).toBeDefined()
    })
  })
})

describe("UsersPage tabs (WEB-3 S3-D7)", () => {
  it("opens on People, offers Invite, and does not read the sessions", async () => {
    setupFetch()
    renderPage()
    expect(await screen.findByText("Rina Wijaya")).toBeTruthy()
    expect(screen.getByRole("tab", { name: "People" }).getAttribute("aria-selected")).toBe("true")
    expect(screen.getByRole("button", { name: /invite user/i })).toBeTruthy()
    expect(fetchCalls.some((c) => c.url.includes("/api/auth/sessions"))).toBe(false)
  })

  it("opens on Sessions for ?tab=sessions, lists them, and offers no Invite there", async () => {
    search = new URLSearchParams("tab=sessions")
    setupFetch()
    renderPage()
    expect(await screen.findByText("Sesi Pengguna")).toBeTruthy()
    expect(screen.getByRole("tab", { name: "Sessions" }).getAttribute("aria-selected")).toBe("true")
    expect(screen.queryByRole("button", { name: /invite user/i })).toBeNull()
    expect(screen.getByRole("button", { name: "Revoke" })).toBeTruthy()
  })
})


describe("UsersPage edit username and email (ADM-11 D8/D11)", () => {
  it("editing username and email calls PATCH /api/identity/users/{id} and reloads", async () => {
    setupFetch({})
    renderPage()
    await waitFor(() => expect(screen.getByText("Rina Wijaya")).toBeDefined())
    await openRowAction(/edit username and email/i)

    const usernameInput = await screen.findByLabelText("Username")
    const emailInput = screen.getByLabelText("Email")
    fireEvent.change(usernameInput, { target: { value: "rinanew" } })
    fireEvent.change(emailInput, { target: { value: "rinanew@meridian.example" } })

    fireEvent.click(screen.getByRole("button", { name: "Save" }))

    await waitFor(() => {
      const patch = fetchCalls.find(
        (c) => c.method === "PATCH" && c.url.endsWith(`/api/identity/users/${ROW.id}`)
      )
      expect(patch).toBeDefined()
      expect(JSON.parse(patch?.body ?? "{}")).toEqual({
        username: "rinanew",
        email: "rinanew@meridian.example",
      })
    })
  })
})

describe("UsersPage email display (ADM-11 D2/D11)", () => {
  it("a user with null email displays an em dash in the email column", async () => {
    setupFetch({ row: { ...ROW, email: null } })
    renderPage()
    await waitFor(() => expect(screen.getByText("Rina Wijaya")).toBeDefined())
    expect(screen.getAllByText("—").length).toBeGreaterThanOrEqual(1)
  })
})
