// WEB-7 T2 (D2/D3): the invite form is its own page. These are the invite
// tests that used to live beside the list, plus the D3 guarantee: the
// shown-once password crosses the redirect in memory, never in the URL or
// web storage.
let pushed: string[] = []
mock.module("next/navigation", () => ({
  usePathname: () => "/admin/users/invite",
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

import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, mock } from "bun:test"
import { AuthProvider } from "@/features/auth/auth-provider"
import { takeInvitePassword } from "./invite-password"
import { InviteUserPage } from "./invite-user-page"

const originalFetch = global.fetch

type FetchCall = { url: string; method: string; body?: string }

let fetchCalls: FetchCall[] = []

// A deliberately low-entropy stand-in for the server-generated password, so
// the secret scanner does not read it as a credential (same reason as the
// users-page test).
const TEMPORARY_PASSWORD = "test-only-temporary-password"

const ROLES = [
  { id: "r1", name: "Analyst", members: 24, permissions: "query:read", description: "Read." },
  { id: "r2", name: "Approver", members: 4, permissions: "agent:approve", description: "Approve." },
]

const TENANTS = [
  {
    id: "t1",
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
  cleanup()
  global.fetch = originalFetch
  localStorage.clear()
  sessionStorage.clear()
  fetchCalls = []
  pushed = []
  // The store is module-level and single-use; a test that never reached the
  // list would otherwise leave a password for the next one.
  takeInvitePassword("u1")
})

beforeEach(() => {
  fetchCalls = []
  pushed = []
})

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  })
}

function setupFetch(opts: { inviteStatus?: number } = {}): void {
  global.fetch = mock(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url =
      typeof input === "string" ? input : input instanceof URL ? input.toString() : input.url
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
        tenants: [],
      })
    }
    if (url.includes("/api/auth/password-rules")) {
      return jsonResponse({ minLength: 12, maxLength: 1024 })
    }
    if (method === "GET" && url.includes("/api/identity/roles")) return jsonResponse(ROLES)
    if (method === "GET" && url.includes("/api/identity/tenants")) return jsonResponse(TENANTS)
    if (method === "POST" && url.endsWith("/api/identity/users")) {
      const status = opts.inviteStatus ?? 200
      if (status >= 400) return jsonResponse({ error: "invite failed" }, status)
      const body = JSON.parse(String(init?.body ?? "{}")) as {
        name?: string
        username?: string
        password?: { mode?: string }
      }
      return jsonResponse(
        {
          id: "u2",
          name: body.name ?? "New User",
          username: body.username ?? "newuser",
          email: null,
          status: "active",
          roles: [],
          tenants: [],
          lastActivity: null,
          temporaryPassword: body.password?.mode === "set" ? undefined : TEMPORARY_PASSWORD,
        },
        status
      )
    }
    return new Response("{}", { status: 200 })
  }) as unknown as typeof fetch
}

function renderPage() {
  return render(
    <AuthProvider>
      <InviteUserPage />
    </AuthProvider>
  )
}

async function fillInvite(opts: {
  name?: string
  username?: string
  email?: string
  roles?: string[]
  tenants?: string[]
  passwordMode?: "generate" | "set"
  password?: string
  mustChangePassword?: boolean
} = {}) {
  fireEvent.change(await screen.findByLabelText("Name"), {
    target: { value: opts.name ?? "Rina Guest" },
  })
  fireEvent.change(screen.getByLabelText("Username"), {
    target: { value: opts.username ?? "rinaguest" },
  })
  fireEvent.change(screen.getByLabelText("Email (optional)"), {
    target: { value: opts.email ?? "rina.guest@meridian.example" },
  })
  for (const r of opts.roles ?? []) fireEvent.click(screen.getByLabelText(r))
  for (const t of opts.tenants ?? []) fireEvent.click(screen.getByLabelText(t))
  if (opts.passwordMode === "set") {
    fireEvent.click(screen.getByLabelText("Set it now"))
    if (opts.password) {
      fireEvent.change(screen.getByLabelText("Password"), {
        target: { value: opts.password },
      })
    }
  }
  if (opts.mustChangePassword === false) {
    fireEvent.click(screen.getByLabelText("Must change password at first sign-in"))
  }
}

describe("InviteUserPage (WEB-7 D2/D3)", () => {
  it("is the invite form as its own page, with every field it had", async () => {
    setupFetch()
    renderPage()
    expect(await screen.findByRole("heading", { name: "Invite User" })).toBeTruthy()
    expect(screen.getByLabelText("Name")).toBeTruthy()
    expect(screen.getByLabelText("Username")).toBeTruthy()
    expect(screen.getByLabelText("Email (optional)")).toBeTruthy()
    expect(await screen.findByLabelText("Analyst")).toBeTruthy()
    expect(screen.getByLabelText("Meridian Retail")).toBeTruthy()
    expect(screen.getByLabelText("Set it now")).toBeTruthy()
    expect(screen.getByLabelText("Must change password at first sign-in")).toBeTruthy()
    expect(screen.getByRole("button", { name: "Invite" })).toBeTruthy()
  })

  it("states the server's password length, not a guess", async () => {
    setupFetch()
    renderPage()
    fireEvent.click(await screen.findByLabelText("Set it now"))
    expect(await screen.findByText(/At least 12 characters/)).toBeTruthy()
  })

  it("sends the full payload with selected roles and tenants as checkboxes", async () => {
    setupFetch()
    renderPage()
    await screen.findByLabelText("Analyst")
    await fillInvite({ roles: ["Analyst"], tenants: ["Meridian Retail"] })
    fireEvent.click(screen.getByRole("button", { name: "Invite" }))

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

  it("setting a password upfront sends password mode set and remembers no password", async () => {
    setupFetch()
    renderPage()
    await screen.findByLabelText("Analyst")
    await fillInvite({
      passwordMode: "set",
      password: "customPassword123!",
      mustChangePassword: false,
    })
    fireEvent.click(screen.getByRole("button", { name: "Invite" }))

    await waitFor(() => {
      const post = fetchCalls.find(
        (c) => c.method === "POST" && c.url.endsWith("/api/identity/users")
      )
      expect(post).toBeDefined()
      const body = JSON.parse(post?.body ?? "{}")
      expect(body.password).toEqual({ mode: "set", value: "customPassword123!" })
      expect(body.mustChangePassword).toBe(false)
    })
    await waitFor(() => expect(pushed).toEqual(["/admin/users"]))
    expect(takeInvitePassword("u1")).toBeNull()
  })

  it("a failed invite stays on the page, shows the API's answer, and remembers no password", async () => {
    setupFetch({ inviteStatus: 500 })
    renderPage()
    await screen.findByLabelText("Analyst")
    await fillInvite()
    fireEvent.click(screen.getByRole("button", { name: "Invite" }))

    expect(await screen.findByRole("alert")).toBeTruthy()
    expect(pushed).toEqual([])
    expect(takeInvitePassword("u1")).toBeNull()
  })

  // WEB-7 D3: the redirect carries no password — not in the URL, not in
  // either web storage — and the one-shot store is what hands it over.
  it("hands the shown-once password over in memory, and never in the URL or web storage", async () => {
    setupFetch()
    renderPage()
    await screen.findByLabelText("Analyst")
    await fillInvite()
    fireEvent.click(screen.getByRole("button", { name: "Invite" }))

    await waitFor(() => expect(pushed.length).toBe(1))
    expect(pushed).toEqual(["/admin/users"])
    expect(pushed.join(" ")).not.toContain(TEMPORARY_PASSWORD)
    expect(JSON.stringify(localStorage)).not.toContain(TEMPORARY_PASSWORD)
    expect(JSON.stringify(sessionStorage)).not.toContain(TEMPORARY_PASSWORD)
    expect(takeInvitePassword("u1")).toEqual({
      password: TEMPORARY_PASSWORD,
      mustChangePassword: true,
    })
    // Single-use: a second read finds nothing, so the dialog cannot reopen.
    expect(takeInvitePassword("u1")).toBeNull()
  })

  it("cancel goes back to the list without inviting", async () => {
    setupFetch()
    renderPage()
    await screen.findByLabelText("Analyst")
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }))
    expect(pushed).toEqual(["/admin/users"])
    expect(fetchCalls.some((c) => c.method === "POST")).toBe(false)
  })
})
