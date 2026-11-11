let search = new URLSearchParams()
import { mock } from "bun:test"

mock.module("next/navigation", () => ({
  usePathname: () => "/admin/settings",
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
import { afterEach, describe, expect, it } from "bun:test"
import { SsoPageForm } from "./sso-page"

const originalFetch = global.fetch

afterEach(() => {
  cleanup()
  global.fetch = originalFetch
  search = new URLSearchParams()
})

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  })
}

type Call = { method: string; path: string; body: string | null }

const DEFAULT_SSO = {
  issuer: { value: "https://auth.example.com", source: "console" },
  clientId: { value: "lakehouse-client", source: "console" },
  clientSecret: { set: true, source: "console" },
  providerName: { value: "Okta", source: "console" },
  groupsClaim: { value: "groups", source: "console" },
  roleMap: {
    value: [{ group: "engineering", role: "developer" }],
    source: "console",
  },
  jitProvisioning: { value: true, source: "console" },
  canEdit: true,
}

const ROLES = [
  { id: "r1", name: "admin" },
  { id: "r2", name: "developer" },
  { id: "r3", name: "viewer" },
]

function mockApi(
  ssoSettings: unknown = DEFAULT_SSO,
  roles: unknown = ROLES,
  testResult: unknown = {
    ok: true,
    subject: "user-sub-123",
    email: "test@example.com",
    groups: ["engineering", "admins"],
    mappedRoles: ["developer", "admin"],
  }
) {
  const calls: Call[] = []
  global.fetch = mock(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url =
      typeof input === "string"
        ? input
        : input instanceof URL
          ? input.toString()
          : input.url
    const method = (init?.method ?? "GET").toUpperCase()
    calls.push({
      method,
      path: url,
      body: typeof init?.body === "string" ? init.body : null,
    })

    if (url.includes("/api/identity/roles")) {
      return jsonResponse(roles)
    }
    if (url.includes("/api/settings/sso/test/")) {
      return jsonResponse(testResult)
    }
    if (url.includes("/api/settings/sso/test")) {
      return jsonResponse({
        id: "test-run-uuid-456",
        authorizeUrl: "https://auth.example.com/authorize?response_type=code",
      })
    }
    if (url.includes("/api/settings/sso")) {
      return jsonResponse(ssoSettings)
    }
    return jsonResponse({})
  }) as unknown as typeof fetch
  return calls
}

describe("SsoPageForm (ADM-12, D9)", () => {
  it("server-owned fields are read-only", async () => {
    mockApi({
      ...DEFAULT_SSO,
      issuer: { value: "https://auth.server.com", source: "server" },
      clientId: { value: "server-client", source: "server" },
      clientSecret: { set: true, source: "server" },
      roleMap: {
        value: [{ group: "ops", role: "admin" }],
        source: "server",
      },
    })

    render(<SsoPageForm canWrite />)

    const issuerInput = (await screen.findByLabelText("Issuer URL")) as HTMLInputElement
    await waitFor(() => expect(issuerInput.value).toBe("https://auth.server.com"))
    expect(issuerInput.disabled).toBe(true)

    const clientIdInput = (await screen.findByLabelText("Client ID")) as HTMLInputElement
    expect(clientIdInput.disabled).toBe(true)

    const groupInput = (await screen.findByLabelText("IdP group row 1")) as HTMLInputElement
    expect(groupInput.disabled).toBe(true)

    expect(screen.getAllByText("Set by the server").length).toBeGreaterThan(0)
  })

  it("the secret is never in a GET payload", async () => {
    mockApi({
      ...DEFAULT_SSO,
      clientSecret: { set: true, source: "console" },
    })

    render(<SsoPageForm canWrite />)

    await screen.findByLabelText("Issuer URL")
    // Client secret only indicates "Set" or "Not set", never exposes a plaintext secret
    expect(screen.getByText("Set")).toBeDefined()
    expect(screen.queryByDisplayValue(/secret/i)).toBeNull()
    expect(screen.getByRole("button", { name: /replace/i })).toBeDefined()
    expect(screen.getByRole("button", { name: /clear/i })).toBeDefined()
  })

  it("the role map rows send {group, role}", async () => {
    const calls = mockApi()

    render(<SsoPageForm canWrite />)

    await screen.findByLabelText("Issuer URL")
    const saveButton = screen.getByRole("button", { name: "Save" })
    fireEvent.click(saveButton)

    await waitFor(() => {
      const putCall = calls.find(
        (c) => c.method === "PUT" && c.path.includes("/api/settings/sso")
      )
      expect(putCall).toBeDefined()
      const body = JSON.parse(putCall?.body ?? "{}")
      expect(body.roleMap).toEqual([{ group: "engineering", role: "developer" }])
    })
  })

  it("the test result card renders", async () => {
    search = new URLSearchParams("ssoTest=test-run-uuid-456")
    mockApi(DEFAULT_SSO, ROLES, {
      ok: true,
      subject: "oidc-sub-999",
      email: "engineer@company.com",
      groups: ["engineering"],
      mappedRoles: ["developer"],
    })

    render(<SsoPageForm canWrite />)

    await waitFor(() => {
      expect(screen.getByRole("region", { name: "Test sign-in result" })).toBeDefined()
      expect(screen.getByText("oidc-sub-999")).toBeDefined()
      expect(screen.getByText("engineer@company.com")).toBeDefined()
    })
  })

  it("the test result card renders error message when test fails", async () => {
    search = new URLSearchParams("ssoTest=test-run-uuid-456")
    mockApi(DEFAULT_SSO, ROLES, {
      ok: false,
      error: "The provider refused the client ID or secret.",
    })

    render(<SsoPageForm canWrite />)

    await waitFor(() => {
      expect(screen.getByRole("region", { name: "Test sign-in result" })).toBeDefined()
      expect(
        screen.getByText("The provider refused the client ID or secret.")
      ).toBeDefined()
    })
  })

  it("the page is read-only without canEdit", async () => {
    mockApi({
      ...DEFAULT_SSO,
      canEdit: false,
    })

    render(<SsoPageForm canWrite />)

    const issuerInput = (await screen.findByLabelText("Issuer URL")) as HTMLInputElement
    await waitFor(() => expect(issuerInput.value).toBe("https://auth.example.com"))
    expect(issuerInput.disabled).toBe(true)

    const clientIdInput = (await screen.findByLabelText("Client ID")) as HTMLInputElement
    expect(clientIdInput.disabled).toBe(true)

    // Save button should not be present
    expect(screen.queryByRole("button", { name: "Save" })).toBeNull()

    // Platform Admin notice should be visible
    expect(
      screen.getByText(/Only a Platform Admin can change single sign-on settings/i)
    ).toBeDefined()
  })
})