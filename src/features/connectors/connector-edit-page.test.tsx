// `AuthProvider` (needed by the tenant picker's `useAuth()`) calls
// `usePathname()`/`useRouter()`; stub `next/navigation` before any import
// resolves (`mock.module` is hoisted by bun's test runner).
mock.module("next/navigation", () => ({
  usePathname: () => "/connectors/conn-a/edit",
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
import { ConnectorEditPage } from "./connector-edit-page"

const originalFetch = global.fetch

afterEach(() => {
  global.fetch = originalFetch
  cleanup()
})

const ME = {
  id: "u1",
  name: "Editor",
  email: null,
  roles: ["admin"],
  permissions: ["*:*"],
  tenants: [
    { id: "t-acme", name: "Acme Co", slug: "acme" },
    { id: "t-other", name: "Other Co", slug: "other" },
  ],
}

const DETAIL = {
  id: "conn-a",
  name: "db demo",
  type: "PostgreSQL",
  direction: "source",
  health: "unhealthy",
  environment: "production",
  tenant: "Acme Co",
  lastTestAt: null,
  lastActivityAt: null,
  capabilities: [],
  owner: "Current user",
  discoveredAssets: 0,
  discoveredSchemas: [],
  recentErrors: [],
  dependentPipelines: [],
  credentialManaged: false,
  residency: "id-jakarta",
  tenantId: "t-acme",
}

const SPEC = {
  adapter: "sql",
  ingestMode: "batch",
  dial: { driver: "postgres", host: "192.168.18.205", port: 5432, database: "insurance_db", user: "insurer", sslMode: null },
  sourceObjects: [{ name: "public.policies", target: "policies" }],
  scheduleCron: "0 * * * *",
  secretRefs: { primary: "env:CONNECTOR_CONN_A_PASSWORD", secondary: null },
}

function json(body: unknown, status = 200) {
  return new Response(body === null ? null : JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  })
}

type Call = { url: string; method: string; body: unknown }

function stubFetch(
  overrides: Record<string, () => Response> = {},
  detail: Record<string, unknown> = DETAIL
): Call[] {
  const calls: Call[] = []
  global.fetch = mock(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input)
    const method = init?.method ?? "GET"
    calls.push({ url, method, body: init?.body ? JSON.parse(init.body as string) : undefined })
    const key = `${method} ${url.replace(/^https?:\/\/[^/]+/, "")}`
    if (overrides[key]) return overrides[key]()
    if (url.includes("/api/auth/me")) return json(ME)
    if (key === "GET /api/connectors/types") {
      return json([{ name: "PostgreSQL", adapter: "sql", supported: true, docsUrl: null }])
    }
    if (key === "GET /api/connectors/conn-a") return json(detail)
    if (key === "GET /api/connectors/conn-a/ingest-spec") return json(SPEC)
    if (key === "PATCH /api/connectors/conn-a") return json({ ...DETAIL, name: "renamed" })
    if (key === "PUT /api/connectors/conn-a/ingest-spec") return json(SPEC)
    if (key === "PUT /api/connectors/conn-a/tenant") return json(null, 204)
    if (key === "PUT /api/connectors/conn-a/credential") {
      return json({ saved: true, slots: ["primary"], verified: true, message: "ok" })
    }
    if (key === "POST /api/connectors/conn-a/test") {
      return json({ ok: true, supported: true, latencyMs: 5, message: "Connected via PostgreSQL", testedAt: "2026-01-01T00:00:00.000Z" })
    }
    throw new Error(`unexpected fetch: ${key}`)
  }) as unknown as typeof fetch
  return calls
}

function renderPage() {
  const view = render(
    <AuthProvider>
      <ConnectorEditPage connectorId="conn-a" />
    </AuthProvider>
  )
  return within(view.container)
}

type Page = ReturnType<typeof renderPage>

const next = (page: Page) => fireEvent.click(page.getByRole("button", { name: "Next" }))
const writes = (calls: Call[]) => calls.filter((c) => c.method !== "GET")

describe("ConnectorEditPage", () => {
  /**
   * The current values load into the same steps create uses; a rename plus
   * a new password become a PATCH and a credential PUT (tested against the
   * source server-side), followed by a connection test. The unchanged
   * connection settings are NOT re-saved.
   */
  it("saves only what changed, in order, then re-tests the connection", async () => {
    const calls = stubFetch()
    const page = renderPage()
    const name = (await waitFor(() => page.getByLabelText("Name"))) as HTMLInputElement
    expect(name.value).toBe("db demo")
    fireEvent.change(name, { target: { value: "renamed" } })
    next(page)

    expect((page.getByLabelText("Host") as HTMLInputElement).value).toBe("192.168.18.205")
    // Blank keeps the current credential, so Next is allowed without one.
    expect((page.getByRole("button", { name: "Next" }) as HTMLButtonElement).disabled).toBe(false)
    fireEvent.change(page.getByLabelText("Password"), { target: { value: "n3w-pass" } })
    next(page)
    next(page)

    expect(page.getByText("db demo → renamed")).toBeDefined()
    expect(page.queryByText(/n3w-pass/)).toBeNull()
    fireEvent.click(page.getByRole("button", { name: "Save changes" }))
    await waitFor(() => expect(page.getByText(/Connection test passed/)).toBeDefined())

    expect(writes(calls).map((c) => `${c.method} ${c.url.replace(/^https?:\/\/[^/]+/, "")}`)).toEqual([
      "PATCH /api/connectors/conn-a",
      "PUT /api/connectors/conn-a/credential",
      "POST /api/connectors/conn-a/test",
    ])
    expect(writes(calls)[0].body).toEqual({ name: "renamed" })
    expect(writes(calls)[1].body).toEqual({ primary: { kind: "password", value: "n3w-pass" } })
  })

  /**
   * Saving stops at the first failure and says which parts are in effect:
   * the rename was saved; the rejected credential was not; no test runs.
   */
  it("stops at the first failure and reports what was and was not saved", async () => {
    const calls = stubFetch({
      "PUT /api/connectors/conn-a/credential": () =>
        json({ error: "the credential was NOT saved: PostgreSQL connection failed: authentication failed" }, 422),
    })
    const page = renderPage()
    fireEvent.change(await waitFor(() => page.getByLabelText("Name")), { target: { value: "renamed" } })
    next(page)
    fireEvent.change(page.getByLabelText("Password"), { target: { value: "wrong" } })
    next(page)
    next(page)
    fireEvent.click(page.getByRole("button", { name: "Save changes" }))

    await waitFor(() => expect(page.getByText("Some changes were not saved")).toBeDefined())
    expect(page.getByText(/Saved · Name, direction/)).toBeDefined()
    expect(page.getByText(/Not saved · Credential .*authentication failed/)).toBeDefined()
    expect(calls.some((c) => c.url.endsWith("/test"))).toBe(false)
  })

  it("will not save when nothing changed", async () => {
    stubFetch()
    const page = renderPage()
    await waitFor(() => page.getByLabelText("Name"))
    next(page)
    next(page)
    next(page)
    expect(page.getByText("Nothing has changed yet.")).toBeDefined()
    expect((page.getByRole("button", { name: "Save changes" }) as HTMLButtonElement).disabled).toBe(true)
  })

  /**
   * Changing the connection re-saves the ingest spec with the tables and
   * schedule it already had (this page does not edit those), keeps the
   * row's `host` label in step, and saves it BEFORE the credential — whose
   * server-side test dials the saved settings.
   */
  it("keeps the ingested tables and schedule when the connection changes", async () => {
    const calls = stubFetch()
    const page = renderPage()
    await waitFor(() => page.getByLabelText("Name"))
    next(page)
    fireEvent.change(page.getByLabelText("Host"), { target: { value: "10.0.0.9" } })
    fireEvent.change(page.getByLabelText("Password"), { target: { value: "pw" } })
    next(page)
    next(page)
    fireEvent.click(page.getByRole("button", { name: "Save changes" }))
    await waitFor(() => expect(page.getByText(/Connection test passed/)).toBeDefined())

    const w = writes(calls)
    expect(w[0].body).toEqual({ host: "10.0.0.9" })
    const spec = w[1].body as Record<string, unknown>
    expect(w[1].url).toContain("/ingest-spec")
    expect(spec.sourceObjects).toEqual(SPEC.sourceObjects)
    expect(spec.scheduleCron).toBe("0 * * * *")
    expect((spec.dial as Record<string, unknown>).host).toBe("10.0.0.9")
    expect(w[2].url).toContain("/credential")
  })

  it("says so plainly when moving tenants is not permitted", async () => {
    stubFetch({
      "PUT /api/connectors/conn-a/tenant": () => json({ error: "permission_denied" }, 403),
    })
    const page = renderPage()
    await waitFor(() => page.getByLabelText("Name"))
    next(page)
    next(page)
    fireEvent.change(page.getByLabelText("Tenant"), { target: { value: "t-other" } })
    next(page)
    expect(page.getByText("Acme Co → Other Co")).toBeDefined()
    fireEvent.click(page.getByRole("button", { name: "Save changes" }))
    await waitFor(() => expect(page.getByText(/Not saved · Tenant — You do not have permission/)).toBeDefined())
  })

  /**
   * The field is labelled for what the source reads (a SQL connector
   * reads a password), but replacing it keeps the kind the connector
   * already stores (read from its reference name server-side), so the
   * reference name does not change.
   */
  it("keeps the stored credential kind when replacing it", async () => {
    const calls = stubFetch({}, { ...DETAIL, credentialKind: "token", credentialSecondaryKind: null })
    const page = renderPage()
    await waitFor(() => page.getByLabelText("Name"))
    next(page)
    fireEvent.change(page.getByLabelText("Password"), { target: { value: "t0ken" } })
    next(page)
    next(page)
    fireEvent.click(page.getByRole("button", { name: "Save changes" }))
    await waitFor(() => expect(page.getByText(/Connection test passed/)).toBeDefined())
    const put = writes(calls).find((c) => c.url.endsWith("/credential"))
    expect(put?.body).toEqual({ primary: { kind: "token", value: "t0ken" } })
  })

  /**
   * A connector pipelines still read from is not offered for deletion at
   * all: the dialog names the pipelines, and no DELETE is ever sent.
   */
  it("refuses to delete a connector pipelines still use", async () => {
    const calls = stubFetch({}, {
      ...DETAIL,
      dependentPipelines: [{ id: "p-orders", name: "orders sync", kind: "pipeline" }],
    })
    const page = renderPage()
    await waitFor(() => page.getByLabelText("Name"))
    expect(page.getByText(/Used by 1 pipeline;/)).toBeDefined()
    fireEvent.click(page.getByRole("button", { name: "Delete…" }))
    await waitFor(() => expect(screen.getByText("Connector is still in use")).toBeDefined())
    expect(screen.getByRole("link", { name: "orders sync" }).getAttribute("href")).toBe("/pipelines/p-orders")
    expect(screen.queryByRole("button", { name: "Delete" })).toBeNull()
    expect(calls.some((c) => c.method === "DELETE")).toBe(false)
  })

  /**
   * The API keeps a CDC connector whose replication slot could not be
   * dropped (409). The dialog shows why and offers a forced delete, which
   * is the only request sent with `?force=true`.
   */
  it("offers a forced delete only after the server refuses a plain one", async () => {
    const calls = stubFetch({
      "DELETE /api/connectors/conn-a": () =>
        json({ error: "connector conn-a was NOT deleted: dropping slot x_slot failed" }, 409),
      "DELETE /api/connectors/conn-a?force=true": () => new Response(null, { status: 204 }),
    })
    const page = renderPage()
    await waitFor(() => page.getByLabelText("Name"))
    fireEvent.click(page.getByRole("button", { name: "Delete…" }))
    fireEvent.click(await waitFor(() => screen.getByRole("button", { name: "Delete" })))
    await waitFor(() => expect(screen.getByText(/dropping slot x_slot failed/)).toBeDefined())
    fireEvent.click(screen.getByRole("button", { name: "Force delete" }))
    await waitFor(() =>
      expect(calls.filter((c) => c.method === "DELETE").map((c) => c.url.replace(/^https?:\/\/[^/]+/, ""))).toEqual([
        "/api/connectors/conn-a",
        "/api/connectors/conn-a?force=true",
      ])
    )
  })
})
