import { afterEach, beforeEach, describe, expect, it, spyOn } from "bun:test"
import { apiFetch, __resetApiFetchRedirectStateForTests, clearActiveTenantId } from "./http"

// apiFetch attaches X-Tenant from the locally-persisted
// active tenant, and degrades silently (no header, never a throw) when the
// tenant is unset or localStorage itself is unavailable.
describe("apiFetch X-Tenant header", () => {
  beforeEach(() => {
    localStorage.clear()
    __resetApiFetchRedirectStateForTests()
  })

  afterEach(() => {
    localStorage.clear()
  })

  it("attaches X-Tenant when an active tenant is set in localStorage", async () => {
    localStorage.setItem("lh_active_tenant", "tenant-a-id")
    const fetchSpy = spyOn(globalThis, "fetch").mockResolvedValue(new Response("{}"))
    await apiFetch("/api/connectors")
    const [, init] = fetchSpy.mock.calls[0]!
    expect((init?.headers as Headers).get("X-Tenant")).toBe("tenant-a-id")
    fetchSpy.mockRestore()
  })

  it("sends no X-Tenant header when none is set", async () => {
    const fetchSpy = spyOn(globalThis, "fetch").mockResolvedValue(new Response("{}"))
    await apiFetch("/api/connectors")
    const [, init] = fetchSpy.mock.calls[0]!
    expect((init?.headers as Headers).has("X-Tenant")).toBe(false)
    fetchSpy.mockRestore()
  })

  it("never throws when localStorage itself throws (private-browsing/SSR)", async () => {
    const original = globalThis.localStorage
    Object.defineProperty(globalThis, "localStorage", {
      get() {
        throw new Error("SecurityError")
      },
      configurable: true,
    })
    const fetchSpy = spyOn(globalThis, "fetch").mockResolvedValue(new Response("{}"))
    await expect(apiFetch("/api/connectors")).resolves.toBeDefined()
    expect((fetchSpy.mock.calls[0]![1]?.headers as Headers).has("X-Tenant")).toBe(false)
    fetchSpy.mockRestore()
    Object.defineProperty(globalThis, "localStorage", { value: original, configurable: true })
  })
})

// ADM-15 (#483 review): the moment the stored tenant is about to be replaced
// or cleared is the moment it must not travel.
describe("apiFetch leaves /api/auth alone", () => {
  beforeEach(() => {
    localStorage.clear()
    __resetApiFetchRedirectStateForTests()
  })

  afterEach(() => {
    localStorage.clear()
  })

  it("leaves X-Tenant off a call to /api/auth even when a tenant is stored", async () => {
    localStorage.setItem("lh_active_tenant", "tenant-a-id")
    const fetchSpy = spyOn(globalThis, "fetch").mockResolvedValue(new Response("{}"))
    await apiFetch("/api/auth/me")
    await apiFetch("/api/auth/login", { method: "POST" })
    await apiFetch("/api/connectors")
    const headers = (i: number) => fetchSpy.mock.calls[i]![1]?.headers as Headers
    expect(headers(0).has("X-Tenant")).toBe(false)
    expect(headers(1).has("X-Tenant")).toBe(false)
    expect(headers(2).get("X-Tenant")).toBe("tenant-a-id")
    fetchSpy.mockRestore()
  })

  it("clearActiveTenantId removes the stored tenant and never throws when storage does", () => {
    localStorage.setItem("lh_active_tenant", "tenant-a-id")
    clearActiveTenantId()
    expect(localStorage.getItem("lh_active_tenant")).toBeNull()

    const original = globalThis.localStorage
    Object.defineProperty(globalThis, "localStorage", {
      get() {
        throw new Error("SecurityError")
      },
      configurable: true,
    })
    expect(() => clearActiveTenantId()).not.toThrow()
    Object.defineProperty(globalThis, "localStorage", { value: original, configurable: true })
  })
})
