import { afterEach, beforeEach, describe, expect, it, mock } from "bun:test"

// `AuthProvider` calls `usePathname()` and `useRouter()`, which need a Next.js
// AppRouter context that a bare happy-dom environment has not got. Stubbed at
// the module boundary before any import resolves, as
// `components/app-shell/tenant-switcher.test.tsx` does it.
mock.module("next/navigation", () => ({
  usePathname: () => "/",
  useRouter: () => ({
    push: () => {},
    replace: () => {},
    refresh: () => {},
    back: () => {},
    forward: () => {},
    prefetch: () => {},
  }),
}))

import * as React from "react"
import { fireEvent, render, screen, waitFor } from "@testing-library/react"
import {
  ACTIVE_TENANT_STORAGE_KEY,
  AuthProvider,
  resolveActiveTenant,
  useAuth,
} from "@/features/auth/auth-provider"
import { apiFetch } from "@/services/http"

/**
 * `ADM-15` (#483 review): the stored tenant is the signed-in person's, and
 * sign-out clears it. Every test here records what each request carried, so the
 * header is asserted where it is actually made — not through the provider's
 * state alone.
 */

type Seen = { url: string; method: string; tenant: string | null }

let seen: Seen[] = []
let sessionAnswer: () => Response;

function person(tenants: { id: string; name: string; slug: string }[]): Response {
  return new Response(
    JSON.stringify({
      id: "u1",
      name: "Test",
      email: null,
      roles: [],
      permissions: [],
      tenants,
    }),
    { status: 200, headers: { "Content-Type": "application/json" } },
  )
}

const A = [
  { id: "a1", name: "Acme", slug: "acme" },
  { id: "a2", name: "Acme Retail", slug: "acme-retail" },
]
const B = [
  { id: "b1", name: "Beta", slug: "beta" },
  { id: "b2", name: "Beta Labs", slug: "beta-labs" },
]

/** The session, and every request made after it, as the provider drives them. */
function Harness() {
  const { status, activeTenantId } = useAuth()
  React.useEffect(() => {
    // A page that loads data once the session answered — the request whose
    // `X-Tenant` the review is about.
    if (status === "authenticated") void apiFetch("/api/connectors")
  }, [status])
  return (
    <>
      <span data-testid="status">{status}</span>
      <span data-testid="active">{activeTenantId ?? "(none)"}</span>
    </>
  )
}

function Controls() {
  const { logout, refresh, setActiveTenant } = useAuth()
  return (
    <>
      <button onClick={() => void logout()}>sign out</button>
      <button onClick={() => void refresh()}>refresh</button>
      <button onClick={() => setActiveTenant("a2")}>pick a2</button>
      <button onClick={() => setActiveTenant("zz")}>pick zz</button>
    </>
  )
}

const hero = () => screen.getByTestId("active").textContent
const settled = () =>
  waitFor(() => expect(screen.getByTestId("status").textContent).not.toBe("loading"))
const carried = (fragment: string) => seen.filter((s) => s.url.includes(fragment))

describe("AuthProvider and the stored tenant (ADM-15)", () => {
  const originalFetch = globalThis.fetch

  beforeEach(() => {
    seen = []
    localStorage.clear()
    sessionAnswer = () => person(A)
    globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
      const url =
        typeof input === "string"
          ? input
          : input instanceof URL
            ? input.toString()
            : input.url
      seen.push({
        url,
        method: init?.method ?? "GET",
        tenant: new Headers(init?.headers).get("X-Tenant"),
      })
      if (url.includes("/api/auth/me")) return sessionAnswer()
      return new Response("{}", { status: 200, headers: { "Content-Type": "application/json" } })
    }) as unknown as typeof fetch
  })

  afterEach(() => {
    globalThis.fetch = originalFetch
    localStorage.clear()
  })

  it("signing out clears the stored tenant", async () => {
    localStorage.setItem(ACTIVE_TENANT_STORAGE_KEY, "a2")
    render(
      <AuthProvider>
        <Harness />
        <Controls />
      </AuthProvider>,
    )
    await settled()
    fireEvent.click(screen.getByRole("button", { name: "sign out" }))
    await waitFor(() => expect(localStorage.getItem(ACTIVE_TENANT_STORAGE_KEY)).toBeNull())
    const logout = carried("/api/auth/logout")
    expect(logout.length).toBe(1)
    expect(logout[0]!.tenant).toBeNull()
  })

  it("a second person never sends or shows the first person's tenant", async () => {
    localStorage.setItem(ACTIVE_TENANT_STORAGE_KEY, "a2")
    sessionAnswer = () => person(B)
    render(
      <AuthProvider>
        <Harness />
      </AuthProvider>,
    )
    await settled()
    await waitFor(() => expect(hero()).toBe("b1"))
    expect(localStorage.getItem(ACTIVE_TENANT_STORAGE_KEY)).toBe("b1")
    expect(carried("/api/auth/me")[0]!.tenant).toBeNull()
    await waitFor(() => expect(carried("/api/connectors").length).toBe(1))
    expect(carried("/api/connectors")[0]!.tenant).toBe("b1")
    expect(seen.filter((s) => s.tenant === "a2")).toEqual([])
  })

  it("a person signing in after another signed out starts in their own first tenant", async () => {
    localStorage.setItem(ACTIVE_TENANT_STORAGE_KEY, "a1")
    render(
      <AuthProvider>
        <Harness />
        <Controls />
      </AuthProvider>,
    )
    await settled()
    await waitFor(() => expect(hero()).toBe("a1"))
    fireEvent.click(screen.getByRole("button", { name: "pick a2" }))
    await waitFor(() => expect(hero()).toBe("a2"))
    expect(localStorage.getItem(ACTIVE_TENANT_STORAGE_KEY)).toBe("a2")
    fireEvent.click(screen.getByRole("button", { name: "sign out" }))
    await waitFor(() => expect(localStorage.getItem(ACTIVE_TENANT_STORAGE_KEY)).toBeNull())

    sessionAnswer = () => person(B)
    fireEvent.click(screen.getByRole("button", { name: "refresh" }))
    await waitFor(() => expect(hero()).toBe("b1"))
    expect(localStorage.getItem(ACTIVE_TENANT_STORAGE_KEY)).toBe("b1")
  })

  it("a person whose stored tenant is still theirs keeps it", async () => {
    // A control: this passes on the code before ADM-15 too.
    localStorage.setItem(ACTIVE_TENANT_STORAGE_KEY, "b2")
    sessionAnswer = () => person(B)
    render(
      <AuthProvider>
        <Harness />
      </AuthProvider>,
    )
    await settled()
    await waitFor(() => expect(hero()).toBe("b2"))
    expect(localStorage.getItem(ACTIVE_TENANT_STORAGE_KEY)).toBe("b2")
    await waitFor(() => expect(carried("/api/connectors").length).toBe(1))
    expect(carried("/api/connectors")[0]!.tenant).toBe("b2")
  })

  it("a person with no tenant has none stored", async () => {
    localStorage.setItem(ACTIVE_TENANT_STORAGE_KEY, "a2")
    sessionAnswer = () => person([])
    render(
      <AuthProvider>
        <Harness />
      </AuthProvider>,
    )
    await settled()
    expect(hero()).toBe("(none)")
    expect(localStorage.getItem(ACTIVE_TENANT_STORAGE_KEY)).toBeNull()
    await waitFor(() => expect(carried("/api/connectors").length).toBe(1))
    expect(carried("/api/connectors")[0]!.tenant).toBeNull()
  })

  it("a failed session check leaves the stored tenant alone", async () => {
    // A control: a session that merely ended does not clear the choice; only
    // "Sign out" does.
    localStorage.setItem(ACTIVE_TENANT_STORAGE_KEY, "a2")
    sessionAnswer = () => new Response(JSON.stringify({ error: "unauthorized" }), { status: 401 })
    render(
      <AuthProvider>
        <Harness />
      </AuthProvider>,
    )
    await settled()
    expect(screen.getByTestId("status").textContent).toBe("unauthenticated")
    expect(localStorage.getItem(ACTIVE_TENANT_STORAGE_KEY)).toBe("a2")
  })

  it("picking a tenant that is not the person's changes nothing", async () => {
    localStorage.setItem(ACTIVE_TENANT_STORAGE_KEY, "a2")
    render(
      <AuthProvider>
        <Harness />
        <Controls />
      </AuthProvider>,
    )
    await settled()
    await waitFor(() => expect(hero()).toBe("a2"))
    fireEvent.click(screen.getByRole("button", { name: "pick zz" }))
    await new Promise((resolve) => setTimeout(resolve, 0))
    expect(hero()).toBe("a2")
    expect(localStorage.getItem(ACTIVE_TENANT_STORAGE_KEY)).toBe("a2")
  })

  it("resolveActiveTenant keeps a tenant that is the person's, else takes the first, else none", () => {
    expect(resolveActiveTenant("b2", B)).toBe("b2")
    expect(resolveActiveTenant("a2", B)).toBe("b1")
    expect(resolveActiveTenant(null, B)).toBe("b1")
    expect(resolveActiveTenant("a2", [])).toBeNull()
    expect(resolveActiveTenant(null, [])).toBeNull()
  })
})
