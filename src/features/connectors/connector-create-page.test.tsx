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

const DEFAULT_TYPES = [
  { name: "PostgreSQL", adapter: "sql", supported: true, docsUrl: null, unsupportedReason: null },
  { name: "Kafka", adapter: null, supported: false, docsUrl: null, unsupportedReason: null },
]

/** Routes every fetch the wizard issues; records them in order. */
function stubFetch(
  extra: (url: string, method: string) => Response | null = () => null,
  types: unknown[] = DEFAULT_TYPES
): Call[] {
  const calls: Call[] = []
  global.fetch = mock(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input)
    const method = init?.method ?? "GET"
    calls.push({ url, method, body: init?.body ? JSON.parse(init.body as string) : undefined })
    if (url.includes("/api/auth/me")) return json(ME)
    if (url.endsWith("/api/connectors/types")) return json(types)
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
  it("offers a link to upload a file, above the type cards, on the first step", async () => {
    stubFetch()
    renderPage()
    const link = await screen.findByRole("link", { name: "Have a file instead? Upload CSV, TSV, Excel or Parquet files" })
    expect(link.getAttribute("href")).toBe("/connectors/upload")
    const card = await screen.findByRole("radio", { name: /PostgreSQL/ })
    expect(link.compareDocumentPosition(card)).toBe(Node.DOCUMENT_POSITION_FOLLOWING)
  })

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
            lastRunSuccessAt: null,
            lastRunFailureAt: null,
            failureStreak: 0,
            schemaChangePolicy: "apply_non_breaking",
            pausedReason: null,
            pausedAt: null,
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
      if (url.endsWith("/runs")) return json({ runs: [] })
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
    expect(password.getAttribute("autocomplete")).toBe("new-password")
    fireEvent.change(password, { target: { value: "s3cret-pass" } })
    // The eye button shows the typed secret to check it, and hides it again.
    fireEvent.click(page.getByRole("button", { name: "Show password" }))
    expect(password.type).toBe("text")
    expect(password.getAttribute("autocomplete")).toBe("off")
    fireEvent.click(page.getByRole("button", { name: "Hide password" }))
    expect(password.type).toBe("password")
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
    fireEvent.change(page.getByLabelText("Residency"), { target: { value: "in-region" } })
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
    // Once created, the new connector's own page is one press away, beside
    // the way back to the list (which the header repeats).
    expect(page.getByRole("button", { name: "Open connector" }).getAttribute("href")).toBe(
      "/connectors/conn-orders-k3x9"
    )
    expect(page.getAllByRole("button", { name: "View connectors" }).map((b) => b.getAttribute("href"))).toEqual([
      "/connectors",
      "/connectors",
    ])

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
   * `SRC-12` part B: with a secret manager set up, the credential can be a
   * reference. The connector is created with nothing stored, the connection
   * settings are saved, then the reference is set through the same
   * probe-first route an edit uses, and only then is the connection tested.
   */
  it("creates a connector whose credential is a reference to a secret in the secret manager", async () => {
    const calls = stubFetch((url, method) => {
      if (url.endsWith("/api/connectors/credential-settings")) {
        return json({ secretManager: "vault", typedCredentials: true })
      }
      if (url.endsWith("/api/connectors") && method === "POST") {
        return json(
          {
            id: "conn-orders-k3x9",
            name: "orders",
            type: "PostgreSQL",
            direction: "source",
            health: "unknown",
            environment: "production",
            tenant: "Acme Co",
            lastTestAt: null,
            lastActivityAt: null,
            lastRunSuccessAt: null,
            lastRunFailureAt: null,
            failureStreak: 0,
            schemaChangePolicy: "apply_non_breaking",
            pausedReason: null,
            pausedAt: null,
            capabilities: [],
            owner: "Current user",
            credential: { primary: "file:/run/secrets/connector_managed_conn_orders_k3x9_password", secondary: null },
            credentialStored: false,
          },
          201
        )
      }
      if (url.includes("/ingest-spec")) {
        return json({ adapter: "sql", ingestMode: "batch", dial: {}, sourceObjects: [], scheduleCron: null })
      }
      if (url.endsWith("/credential") && method === "PUT") {
        return json({ saved: true, slots: ["primary"], verified: true, message: "ok" })
      }
      if (url.includes("/test")) {
        return json({ ok: true, supported: true, latencyMs: 12, message: "ok", testedAt: "2026-01-01T00:00:00.000Z" })
      }
      if (url.endsWith("/runs")) return json({ runs: [] })
      return null
    })
    const page = renderPage()
    await waitFor(() =>
      expect(page.getByRole("radio", { name: /PostgreSQL/ }).getAttribute("aria-checked")).toBe("true")
    )
    fireEvent.change(page.getByLabelText("Name"), { target: { value: "orders" } })
    fireEvent.click(page.getByRole("button", { name: "Next" }))
    fireEvent.click(await waitFor(() => page.getByRole("button", { name: "Reference a secret" })))
    expect(page.queryByLabelText("Password")).toBeNull()
    fireEvent.change(page.getByLabelText("Path in Vault"), { target: { value: "secret/connectors/orders" } })
    fireEvent.change(page.getByLabelText("Key in the secret"), { target: { value: "password" } })
    fireEvent.click(page.getByRole("button", { name: "Next" }))
    await waitFor(() => page.getByLabelText("Residency"))
    fireEvent.change(page.getByLabelText("Residency"), { target: { value: "in-region" } })
    fireEvent.click(page.getByRole("button", { name: "Next" }))
    expect(page.getByText(/reference to a secret in HashiCorp Vault, read at every use/)).toBeDefined()
    fireEvent.click(page.getByRole("button", { name: "Create connector" }))
    await waitFor(() => expect(page.getByText(/Connection test passed/)).toBeDefined())

    const create = calls.find((c) => c.url.endsWith("/api/connectors") && c.method === "POST")
    expect((create?.body as { credential: unknown }).credential).toEqual({ source: "managed", primary: "password" })
    const order = (match: (c: Call) => boolean) => calls.findIndex(match)
    const specIndex = order((c) => c.url.includes("/ingest-spec") && c.method === "PUT")
    const credentialIndex = order((c) => c.url.endsWith("/credential") && c.method === "PUT")
    const testIndex = order((c) => c.url.includes("/test"))
    expect(credentialIndex).toBeGreaterThan(specIndex)
    expect(testIndex).toBeGreaterThan(credentialIndex)
    expect(calls[credentialIndex].body).toEqual({ primary: { reference: "vault:secret/connectors/orders#password" } })
  })

  /**
   * `SRC-12` part C: a REST connector that signs in with OAuth authorization
   * code asks for the client secret only (the refresh token comes from signing
   * in). The create request carries no values (it needs both slots or none);
   * the client secret is saved right after the connection settings, probe-first,
   * and the created page offers Sign in.
   */
  it("creates an OAuth sign-in connector: the client secret is saved after the settings and Sign in is offered", async () => {
    const calls = stubFetch(
      (url, method) => {
        if (url.endsWith("/api/connectors/credential-settings")) {
          return json({
            secretManager: null,
            typedCredentials: true,
            oauthCallbackUrl: "https://console.test/api/connectors/oauth/callback",
          })
        }
        if (url.endsWith("/api/connectors") && method === "POST") {
          return json(
            {
              id: "conn-crm-k3x9",
              name: "crm",
              type: "REST API",
              direction: "source",
              health: "unknown",
              environment: "production",
              tenant: "Acme Co",
              lastTestAt: null,
              lastActivityAt: null,
              lastRunSuccessAt: null,
              lastRunFailureAt: null,
              failureStreak: 0,
              schemaChangePolicy: "apply_non_breaking",
              pausedReason: null,
              pausedAt: null,
              capabilities: [],
              owner: "Current user",
              credential: {
                primary: "file:/run/secrets/connector_managed_conn_crm_k3x9_secret_key",
                secondary: "file:/run/secrets/connector_managed_conn_crm_k3x9_token",
              },
              credentialStored: false,
            },
            201
          )
        }
        if (url.includes("/ingest-spec")) {
          return json({ adapter: "rest", ingestMode: "batch", dial: {}, sourceObjects: [], scheduleCron: null })
        }
        if (url.endsWith("/credential") && method === "PUT") {
          return json({ saved: true, slots: ["primary"], verified: false, message: "not signed in" })
        }
        if (url.endsWith("/test")) {
          return json({
            ok: false,
            supported: false,
            latencyMs: null,
            message: "Nobody has signed in to this connector yet, so the connection cannot be tested.",
            testedAt: null,
          })
        }
        if (url.endsWith("/oauth")) {
          return json({
            signedIn: false,
            callbackUrl: "https://console.test/api/connectors/oauth/callback",
            revocationConfigured: false,
          })
        }
        if (url.endsWith("/runs")) return json({ runs: [] })
        return null
      },
      [{ name: "REST API", adapter: "rest", supported: true, docsUrl: null, unsupportedReason: null }]
    )
    const page = renderPage()
    await waitFor(() =>
      expect(page.getByRole("radio", { name: /REST API/ }).getAttribute("aria-checked")).toBe("true")
    )
    fireEvent.change(page.getByLabelText("Name"), { target: { value: "crm" } })
    fireEvent.click(page.getByRole("button", { name: "Next" }))
    await waitFor(() => page.getByLabelText("Base URL"))
    fireEvent.change(page.getByLabelText("Base URL"), { target: { value: "https://api.example.com" } })
    fireEvent.change(page.getByLabelText("Auth type"), { target: { value: "oauth2_authorization_code" } })
    fireEvent.change(page.getByLabelText("Client ID"), { target: { value: "app-1" } })
    fireEvent.change(page.getByLabelText("Sign-in URL"), { target: { value: "https://login.example.com/authorize" } })
    fireEvent.change(page.getByLabelText("Token URL"), { target: { value: "https://login.example.com/token" } })
    // The address to give the provider is on the form, before anything is saved.
    await waitFor(() =>
      expect(page.getByText("https://console.test/api/connectors/oauth/callback")).toBeDefined()
    )
    // Only the client secret is asked for.
    expect(page.getByLabelText("Client secret")).toBeDefined()
    expect(page.queryByLabelText("Refresh token")).toBeNull()
    fireEvent.change(page.getByLabelText("Client secret"), { target: { value: "s3cret-value" } })
    fireEvent.click(page.getByRole("button", { name: "Next" }))
    await waitFor(() => page.getByLabelText("Residency"))
    fireEvent.change(page.getByLabelText("Residency"), { target: { value: "in-region" } })
    fireEvent.click(page.getByRole("button", { name: "Next" }))
    fireEvent.click(page.getByRole("button", { name: "Create connector" }))
    await waitFor(() => expect(page.getByRole("button", { name: "Sign in" })).toBeDefined())

    const create = calls.find((c) => c.url.endsWith("/api/connectors") && c.method === "POST")
    // No values at create: both slots or none, and the refresh token is not typed.
    expect((create?.body as { credential: unknown }).credential).toEqual({
      source: "managed",
      primary: "secret_key",
      secondary: "token",
    })
    const order = (match: (c: Call) => boolean) => calls.findIndex(match)
    const specIndex = order((c) => c.url.includes("/ingest-spec") && c.method === "PUT")
    const credentialIndex = order((c) => c.url.endsWith("/credential") && c.method === "PUT")
    const testIndex = order((c) => c.url.endsWith("/test"))
    expect(credentialIndex).toBeGreaterThan(specIndex)
    expect(testIndex).toBeGreaterThan(credentialIndex)
    expect(calls[credentialIndex].body).toEqual({ primary: { kind: "secret_key", value: "s3cret-value" } })
    const spec = calls[specIndex].body as { dial: { auth: unknown } }
    expect(spec.dial.auth).toEqual({
      type: "oauth2_authorization_code",
      clientId: "app-1",
      authorizeUrl: "https://login.example.com/authorize",
      tokenUrl: "https://login.example.com/token",
      scopes: [],
    })
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
