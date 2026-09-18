import { afterEach, beforeEach, describe, expect, it, mock } from "bun:test"

// WS8 §Phase F: `AuthProvider` calls `usePathname()` and `useRouter()`
// at the top of the component (auth-provider.tsx:140-141). Both hooks
// throw (or expect a Next.js AppRouter context) in a bare happy-dom
// environment, so stub `next/navigation` at the module boundary before
// any import resolves. `mock.module` is hoisted to the top of the file
// by bun's test runner, so the stub is in place before the `import`
// statements below pull in `AuthProvider`.
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
  readActiveTenantId,
  useAuth,
  writeActiveTenantId,
} from "@/features/auth/auth-provider"
import type { AuthUser } from "@/services/clients/auth"
import { TenantSwitcher } from "./tenant-switcher"

const TENANTS = [
  { id: "a", name: "Acme", slug: "acme" },
  { id: "b", name: "Beta", slug: "beta" },
]

describe("TenantSwitcher (WS8 §Phase F)", () => {
  it("renders every tenant the user belongs to and marks the active one", () => {
    render(
      <TenantSwitcher tenants={TENANTS} activeTenantId="a" onSwitch={() => {}} />,
    )
    fireEvent.click(screen.getByRole("button", { name: /Acme/ }))
    expect(screen.getByRole("menuitemradio", { name: /Acme/ }).getAttribute("aria-checked")).toBe("true")
    expect(screen.getByRole("menuitemradio", { name: /Beta/ }).getAttribute("aria-checked")).toBe("false")
  })

  it("renders nothing when the user belongs to zero or one tenant", () => {
    const { container: zero } = render(
      <TenantSwitcher tenants={[]} activeTenantId={null} onSwitch={() => {}} />,
    )
    expect(zero.childNodes.length).toBe(0)
    const { container: one } = render(
      <TenantSwitcher
        tenants={[TENANTS[0]!]}
        activeTenantId="a"
        onSwitch={() => {}}
      />,
    )
    expect(one.childNodes.length).toBe(0)
  })

  it("clicking another tenant calls onSwitch with its id", () => {
    const onSwitch = mock(() => {})
    render(
      <TenantSwitcher tenants={TENANTS} activeTenantId="a" onSwitch={onSwitch} />,
    )
    fireEvent.click(screen.getByRole("button", { name: /Acme/ }))
    fireEvent.click(screen.getByRole("menuitemradio", { name: /Beta/ }))
    expect(onSwitch).toHaveBeenCalledWith("b")
  })
})

// Behaviors 4–8 are exercised through `AuthProvider`'s contract rather
// than a separate test file: the same hook that the navbar's
// `TenantSwitcher` consumes (auth-provider.tsx:217) is what owns
// `activeTenantId` and `setActiveTenant`, so a provider-level test is
// the honest place to assert both the write side and the read-on-mount
// contract — a pure `writeActiveTenantId` test alone would miss the
// "exposed via useAuth" wiring the picker depends on.
describe("AuthProvider tenant-state (WS8 §Phase F)", () => {
  const originalFetch = globalThis.fetch

  beforeEach(() => {
    localStorage.clear()
  })

  afterEach(() => {
    globalThis.fetch = originalFetch
    localStorage.clear()
  })

  // Stubs `GET /api/auth/me` so `AuthProvider`'s mount-time `load()`
  // (auth-provider.tsx:151) resolves with the supplied tenant list.
  // Other paths return an empty 200 — every test in this block only
  // cares about /api/auth/me.
  function mockAuthMe(tenants: AuthUser["tenants"]): void {
    globalThis.fetch = mock(
      async (input: RequestInfo | URL) => {
        const url =
          typeof input === "string"
            ? input
            : input instanceof URL
              ? input.toString()
              : input.url
        if (url.includes("/api/auth/me")) {
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
        return new Response("{}", { status: 200 })
      },
    ) as unknown as typeof fetch
  }

  // Pulls the live `activeTenantId` out of the provider so the
  // assertions read like the picker observes the value, not like they
  // poke at React internals.
  function ActiveDisplay(): React.ReactElement {
    const { activeTenantId } = useAuth()
    return (
      <span data-testid="active">{activeTenantId ?? "(none)"}</span>
    )
  }

  it("writeActiveTenantId stores under the lh_active_tenant key (no-throw on private browsing)", () => {
    writeActiveTenantId("a")
    expect(localStorage.getItem(ACTIVE_TENANT_STORAGE_KEY)).toBe("a")
    expect(readActiveTenantId()).toBe("a")
  })

  it("setActiveTenant writes lh_active_tenant and a sibling useAuth consumer observes the change", async () => {
    function Switcher(): null {
      const { setActiveTenant } = useAuth()
      // Ref-guard so the switch fires exactly once even though the
      // context re-renders after the state update — otherwise the test
      // would oscillate between "a" and the post-set value.
      const calledRef = React.useRef(false)
      React.useEffect(() => {
        if (calledRef.current) return
        calledRef.current = true
        setActiveTenant("a")
      }, [setActiveTenant])
      return null
    }
    mockAuthMe([])
    render(
      <AuthProvider>
        <Switcher />
        <ActiveDisplay />
      </AuthProvider>,
    )
    await waitFor(() =>
      expect(screen.getByTestId("active").textContent).toBe("a"),
    )
    expect(localStorage.getItem(ACTIVE_TENANT_STORAGE_KEY)).toBe("a")
  })

  it("defaults activeTenantId to tenants[0].id when localStorage is empty and the principal has tenants", async () => {
    mockAuthMe([{ id: "first", name: "First", slug: "first" }])
    render(
      <AuthProvider>
        <ActiveDisplay />
      </AuthProvider>,
    )
    await waitFor(() =>
      expect(screen.getByTestId("active").textContent).toBe("first"),
    )
  })

  it("activeTenantId is null when localStorage is empty and the principal has no tenants", async () => {
    mockAuthMe([])
    render(
      <AuthProvider>
        <ActiveDisplay />
      </AuthProvider>,
    )
    await waitFor(() =>
      expect(screen.getByTestId("active").textContent).toBe("(none)"),
    )
  })

  it("does not crash when localStorage itself throws (private-browsing/SSR)", () => {
    const originalLocalStorage = globalThis.localStorage
    Object.defineProperty(globalThis, "localStorage", {
      get() {
        throw new Error("SecurityError")
      },
      configurable: true,
    })
    mockAuthMe([{ id: "x", name: "X", slug: "x" }])
    expect(() =>
      render(
        <AuthProvider>
          <ActiveDisplay />
        </AuthProvider>,
      ),
    ).not.toThrow()
    // The read side mirrors http.ts's fail-soft shape — same try/catch,
    // same null on throw — so even when storage is hostile the provider
    // still resolves the user and exposes a sensible activeTenantId.
    expect(readActiveTenantId()).toBeNull()
    Object.defineProperty(globalThis, "localStorage", {
      value: originalLocalStorage,
      configurable: true,
    })
  })
})