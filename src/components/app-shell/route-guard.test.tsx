// `adm-121`, decision 6: an address no page answers shows "not found" to
// everyone, and a page that exists but is not for the person keeps "You
// don't have access". Both used to read "no access", because the guard
// treated an address missing from the access map as one the person may not
// open. Only `fetch` is stubbed; the real `AuthProvider` and guard run.
let currentPathname = "/"
mock.module("next/navigation", () => ({
  usePathname: () => currentPathname,
  useSearchParams: () => new URLSearchParams(""),
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
import { cleanup, render, screen, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"
import { RouteGuard } from "@/components/app-shell/route-guard"
import { AuthProvider } from "@/features/auth/auth-provider"

const originalFetch = global.fetch

afterEach(() => {
  cleanup()
  global.fetch = originalFetch
  currentPathname = "/"
})

function signInWith(permissions: string[]) {
  global.fetch = mock(async () =>
    new Response(
      JSON.stringify({
        id: "u1",
        name: "Test User",
        email: "test@rantai.dev",
        roles: ["Viewer"],
        permissions,
        tenants: [{ id: "t1", name: "Default Tenant", role: "Viewer" }],
      }),
      { status: 200, headers: { "Content-Type": "application/json" } },
    ),
  ) as unknown as typeof fetch
}

function renderGuard() {
  return render(
    <AuthProvider>
      <RouteGuard>
        <p>The page itself</p>
      </RouteGuard>
    </AuthProvider>,
  )
}

describe("the route guard tells a missing page from a page that is not for the person (adm-121)", () => {
  it("shows not found, for an administrator too, at an address no page answers", async () => {
    for (const permissions of [["*:*"], ["dashboard:read"]]) {
      currentPathname = "/no/such/page"
      signInWith(permissions)
      renderGuard()
      expect(await screen.findByText("Page not found")).toBeTruthy()
      expect(screen.queryByText("You don't have access")).toBeNull()
      expect(screen.queryByText("The page itself")).toBeNull()
      cleanup()
    }
  })

  it("still says the person has no access to a page that exists but needs a permission they lack", async () => {
    currentPathname = "/admin/settings"
    signInWith(["dashboard:read"])
    renderGuard()
    expect(await screen.findByText("You don't have access")).toBeTruthy()
    expect(screen.queryByText("Page not found")).toBeNull()
    expect(screen.queryByText("The page itself")).toBeNull()
  })

  it("shows a page that exists to a person who holds its permission", async () => {
    currentPathname = "/admin/settings"
    signInWith(["settings:write"])
    renderGuard()
    await waitFor(() => expect(screen.getByText("The page itself")).toBeTruthy())
  })
})
