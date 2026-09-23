import { render, screen, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"
import { SsoPage } from "./sso-page"

const originalFetch = global.fetch

afterEach(() => {
  global.fetch = originalFetch
})

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  })
}

/**
 * Stubs `GET /api/auth/providers` (the one network call `SsoPage` makes)
 * to a fixed body. Other paths return an empty 200 so any stray fetch
 * from a future contributor shows up in the failure message rather than
 * a swallowed `undefined`.
 */
function mockProviders(body: unknown): void {
  global.fetch = mock(
    async (input: RequestInfo | URL) => {
      const url =
        typeof input === "string"
          ? input
          : input instanceof URL
            ? input.toString()
            : input.url
      if (url.includes("/api/auth/providers")) {
        return jsonResponse(body)
      }
      return new Response("{}", { status: 200 })
    },
  ) as unknown as typeof fetch
}

describe("SsoPage (reads live OIDC configuration)", () => {
  it("states OIDC is not configured when providers() reports oidc:false", async () => {
    mockProviders({ oidc: false, providerName: null })
    render(<SsoPage />)
    await waitFor(() =>
      expect(screen.getByText(/OIDC is not configured/i)).toBeDefined(),
    )
  })

  it("shows the provider name and a working test sign-in link when oidc:true", async () => {
    mockProviders({ oidc: true, providerName: "okta" })
    render(<SsoPage />)
    await waitFor(() => expect(screen.getByText("okta")).toBeDefined())
    // The Button primitive renders the anchor with `role="button"` rather
    // than `role="link"` (the `nativeButton` behaviour of
    // `@base-ui/react/button`), so a role-based lookup fails even though
    // the underlying element is `<a href="/api/auth/oidc/start">`. The
    // href is the property the test really cares about (does the click
    // reach the OIDC start route?), so find the element by its visible
    // text — its only content is "Test sign-in" — and read the href.
    const link = screen.getByText(/test sign-in/i).closest("a")
    expect(link).not.toBeNull()
    expect(link?.getAttribute("href")).toBe("/api/auth/oidc/start")
  })

  it("states where OIDC_ROLE_MAP actually lives instead of fabricating a role-map API", async () => {
    mockProviders({ oidc: true, providerName: "okta" })
    render(<SsoPage />)
    await waitFor(() =>
      expect(screen.getByText(/OIDC_ROLE_MAP/)).toBeDefined(),
    )
  })
})