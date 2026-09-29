// `AuthProvider` (which `useAuth()` in the tenant picker needs) calls
// `usePathname()`/`useRouter()`; stub `next/navigation` before any import
// resolves (`mock.module` is hoisted by bun's test runner).
mock.module("next/navigation", () => ({
  usePathname: () => "/connectors/create",
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

import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"
import { AuthProvider } from "@/features/auth/auth-provider"
import { ConnectorCreatePage } from "./connector-create-page"

const originalFetch = global.fetch

afterEach(() => {
  global.fetch = originalFetch
  cleanup()
})

const ME = {
  id: "u1",
  name: "Creator",
  email: null,
  roles: ["admin"],
  permissions: ["*:*"],
  tenants: [
    { id: "t-acme", name: "Acme Co", slug: "acme" },
    { id: "t-other", name: "Other Co", slug: "other" },
  ],
}

function json(body: unknown, status = 200) {
  return new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } })
}

type Call = { url: string; method: string; body: unknown }

/** Routes every fetch the wizard issues; records them in order. */
function stubFetch(extra: (url: string, method: string) => Response | null = () => null): Call[] {
  const calls: Call[] = []
  global.fetch = mock(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input)
    const method = init?.method ?? "GET"
    calls.push({ url, method, body: init?.body ? JSON.parse(init.body as string) : undefined })
    if (url.includes("/api/auth/me")) return json(ME)
    if (url.endsWith("/api/connectors/types")) {
      return json([
        { name: "PostgreSQL", adapter: "sql", supported: true, docsUrl: null },
        { name: "Kafka", adapter: null, supported: false, docsUrl: null },
      ])
    }
    const handled = extra(url, method)
    if (handled) return handled
    throw new Error(`unexpected fetch: ${method} ${url}`)
  }) as unknown as typeof fetch
  return calls
}

function renderPage() {
  const view = render(
    <AuthProvider>
      <ConnectorCreatePage />
    </AuthProvider>
  )
  return within(view.container)
}

describe("ConnectorCreatePage", () => {
  it("renders connector types from listTypes as cards, disabling unsupported ones", async () => {
    stubFetch()
    renderPage()
    const kafka = (await screen.findByRole("radio", { name: /Kafka/ })) as HTMLButtonElement
    expect(kafka.disabled).toBe(true)
    expect(within(kafka).getByText("Soon")).toBeDefined()
    // The first supported type is preselected so the step never opens on a
    // disabled option.
    await waitFor(() =>
      expect(screen.getByRole("radio", { name: /PostgreSQL/ }).getAttribute("aria-checked")).toBe("true")
    )
  })

  it("filters connector types by search and says so when nothing matches", async () => {
    stubFetch()
    renderPage()
    await screen.findByRole("radio", { name: /PostgreSQL/ })
    const search = screen.getByLabelText("Search connector types")
    fireEvent.change(search, { target: { value: "kaf" } })
    expect(screen.queryByRole("radio", { name: /PostgreSQL/ })).toBeNull()
    expect(screen.getByRole("radio", { name: /Kafka/ })).toBeDefined()
    fireEvent.change(search, { target: { value: "zzz" } })
    expect(screen.getByText(/No connector type matches/)).toBeDefined()
  })

  /**
   * ADR 0002 Addendum 4: the typed credential is sent once as a `managed`
   * credential's write-only `values`, never a `secretRef`; nothing is left
   * to provision, so no reference name is shown.
   *
   * The connector's tenant is picked from the user's real tenants
   * (defaulting to the active one) and sent as `tenantId`, so the new
   * connector is visible to its creator.
   *
   * And the ordering fix: the ingest spec (host/port/user/database) is
   * saved BEFORE the connection test runs.
   */
  it("creates a managed-credential connector in the chosen tenant and saves the connection before testing it", async () => {
    const calls = stubFetch((url, method) => {
      if (url.endsWith("/api/connectors") && method === "POST") {
        return json(
          {
            id: "conn-orders-k3x9",
            name: "orders",
            type: "PostgreSQL",
            direction: "source",
            health: "unknown",
            environment: "production",
            tenant: "Other Co",
            lastTestAt: null,
            lastActivityAt: null,
            capabilities: [],
            owner: "Current user",
            credential: {
              primary: "file:/run/secrets/connector_managed_conn_orders_k3x9_password",
              secondary: null,
            },
            credentialStored: true,
          },
          201
        )
      }
      if (url.includes("/ingest-spec")) {
        return json({ adapter: "sql", ingestMode: "batch", dial: {}, sourceObjects: [], scheduleCron: null })
      }
      if (url.includes("/test")) {
        return json({ ok: true, supported: true, latencyMs: 12, message: "ok", testedAt: "2026-01-01T00:00:00.000Z" })
      }
      if (url.includes("/api/governance/ingest-runs")) return json([])
      if (url.includes("/ingest/runs")) return json([])
      return null
    })
    const page = renderPage()

    await waitFor(() =>
      expect(page.getByRole("radio", { name: /PostgreSQL/ }).getAttribute("aria-checked")).toBe("true")
    )
    fireEvent.change(page.getByLabelText("Name"), { target: { value: "orders" } })
    fireEvent.click(page.getByRole("button", { name: "Next" }))

    // The password is required: Next stays disabled until it is typed.
    expect((page.getByRole("button", { name: "Next" }) as HTMLButtonElement).disabled).toBe(true)
    const password = page.getByLabelText("Password") as HTMLInputElement
    expect(password.type).toBe("password")
    fireEvent.change(password, { target: { value: "s3cret-pass" } })
    fireEvent.click(page.getByRole("button", { name: "Next" }))

    // Tenant: one card per tenant the user belongs to, defaulting to the
    // active (first) one.
    await waitFor(() =>
      expect(page.getByRole("radio", { name: /Acme Co/ }).getAttribute("aria-checked")).toBe("true")
    )
    fireEvent.click(page.getByRole("radio", { name: /Other Co/ }))
    expect(page.getByRole("radio", { name: /Other Co/ }).getAttribute("aria-checked")).toBe("true")
    // Environment presets fill the free-text field.
    fireEvent.click(page.getByRole("button", { name: "staging" }))
    expect((page.getByLabelText("Environment") as HTMLInputElement).value).toBe("staging")
    fireEvent.change(page.getByLabelText("Residency label"), { target: { value: "in-region" } })
    fireEvent.click(page.getByRole("button", { name: "Next" }))

    // Review: every step has its own section with a way back to it, and the
    // typed credential is described, never echoed.
    expect(page.getByRole("button", { name: "Edit tenant and residency" })).toBeDefined()
    expect(page.getAllByText("Other Co").length).toBeGreaterThan(0)
    expect(page.getByText("Source")).toBeDefined()
    expect(page.getByText(/Password · entered, stored by lakehouse/)).toBeDefined()
    fireEvent.click(page.getByRole("button", { name: "Edit connection" }))
    expect(page.getByLabelText("Password")).toBeDefined()
    fireEvent.click(page.getByRole("button", { name: "Next" }))
    fireEvent.click(page.getByRole("button", { name: "Next" }))
    expect(page.queryByText(/s3cret-pass/)).toBeNull()
    fireEvent.click(page.getByRole("button", { name: "Create connector" }))
    await waitFor(() => expect(page.getByText(/Connection test passed/)).toBeDefined())

    const create = calls.find((c) => c.url.endsWith("/api/connectors") && c.method === "POST")
    const body = create?.body as Record<string, unknown>
    expect(body.secretRef).toBeUndefined()
    expect(body.credential).toEqual({ source: "managed", primary: "password", values: { primary: "s3cret-pass" } })
    expect(body.tenantId).toBe("t-other")
    expect(body.tenant).toBe("Other Co")
    expect(body.environment).toBe("staging")
    expect(body.residency).toBe("in-region")

    expect(page.queryByText(/Provision these credentials/)).toBeNull()
    expect(page.queryByText(/connector_managed_conn_orders_k3x9_password/)).toBeNull()

    const specIndex = calls.findIndex((c) => c.url.includes("/ingest-spec") && c.method === "PUT")
    const testIndex = calls.findIndex((c) => c.url.includes("/test"))
    expect(specIndex).toBeGreaterThan(-1)
    expect(testIndex).toBeGreaterThan(specIndex)
  })

  /**
   * Both resolvers read a stored credential back trimmed, so a password
   * with a leading/trailing space would be saved and then never match. The
   * wizard says so and will not continue.
   */
  it("refuses a password that starts or ends with a space", async () => {
    stubFetch()
    const page = renderPage()
    await waitFor(() =>
      expect(page.getByRole("radio", { name: /PostgreSQL/ }).getAttribute("aria-checked")).toBe("true")
    )
    fireEvent.change(page.getByLabelText("Name"), { target: { value: "orders" } })
    fireEvent.click(page.getByRole("button", { name: "Next" }))
    fireEvent.change(page.getByLabelText("Password"), { target: { value: "padded " } })

    expect(page.getByText("Must not start or end with a space.")).toBeDefined()
    expect((page.getByRole("button", { name: "Next" }) as HTMLButtonElement).disabled).toBe(true)
  })
})
