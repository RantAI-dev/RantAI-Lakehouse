// `AuthProvider` (needed by the tenant picker's `useAuth()`) calls
// `usePathname()`/`useRouter()`; stub `next/navigation` before any import
// resolves (`mock.module` is hoisted by bun's test runner).
const pushed: string[] = []
mock.module("next/navigation", () => ({
  usePathname: () => "/connectors/conn-a/edit",
  useSearchParams: () => new URLSearchParams(),
  useRouter: () => ({
    push: (to: string) => {
      pushed.push(to)
    },
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
  pushed.length = 0
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
  lastRunSuccessAt: null,
  lastRunFailureAt: null,
  failureStreak: 0,
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
      return json([{ name: "PostgreSQL", adapter: "sql", supported: true, docsUrl: null, unsupportedReason: null }])
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
    // No browser suggestions from other forms' history on any field.
    expect(name.getAttribute("autocomplete")).toBe("off")
    fireEvent.change(name, { target: { value: "renamed" } })
    next(page)

    expect((page.getByLabelText("Host") as HTMLInputElement).value).toBe("192.168.18.205")
    expect(page.getByLabelText("Host").getAttribute("autocomplete")).toBe("off")
    // Blank keeps the current credential, so Next is allowed without one.
    expect((page.getByRole("button", { name: "Next" }) as HTMLButtonElement).disabled).toBe(false)
    fireEvent.change(page.getByLabelText("Password"), { target: { value: "n3w-pass" } })
    next(page)
    next(page)

    expect(page.getByText("db demo → renamed")).toBeDefined()
    expect(page.queryByText(/n3w-pass/)).toBeNull()
    fireEvent.click(page.getByRole("button", { name: "Save changes" }))
    await waitFor(() => expect(page.getByText(/Connection test passed/)).toBeDefined())
    // After saving, the way on is the connector's own page, not the list.
    expect(page.getByRole("button", { name: "Back to connector" }).getAttribute("href")).toBe("/connectors/conn-a")

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

  it("cancels back to the connector's page", async () => {
    stubFetch()
    const page = renderPage()
    await waitFor(() => page.getByLabelText("Name"))
    expect(page.getByRole("button", { name: "Cancel" }).getAttribute("href")).toBe("/connectors/conn-a")
  })

  it("will not save when nothing changed", async () => {
    stubFetch()
    const page = renderPage()
    await waitFor(() => page.getByLabelText("Name"))
    next(page)
    next(page)
    next(page)
    expect(page.getByText(/Nothing has changed yet, so there is nothing to save/)).toBeDefined()
    expect((page.getByRole("button", { name: "Save changes" }) as HTMLButtonElement).disabled).toBe(true)
  })

  /**
   * Changing a connection setting that does not move the connector (here
   * the user) re-saves the ingest spec with the tables and schedule it
   * already had (this page does not edit those) and keeps the row's `host`
   * label in step. Nothing asks for the credential and none is sent: the
   * stored one still goes to the same place.
   */
  it("keeps the ingested tables and schedule when the connection changes without moving", async () => {
    const calls = stubFetch()
    const page = renderPage()
    await waitFor(() => page.getByLabelText("Name"))
    next(page)
    fireEvent.change(page.getByLabelText("User"), { target: { value: "someone_else" } })
    expect((page.getByRole("button", { name: "Next" }) as HTMLButtonElement).disabled).toBe(false)
    next(page)
    next(page)
    fireEvent.click(page.getByRole("button", { name: "Save changes" }))
    await waitFor(() => expect(page.getByText(/Connection test passed/)).toBeDefined())

    const w = writes(calls)
    expect(w.some((c) => c.url.endsWith("/credential"))).toBe(false)
    const save = w.find((c) => c.url.endsWith("/ingest-spec"))!
    const spec = save.body as Record<string, unknown>
    expect(spec.sourceObjects).toEqual(SPEC.sourceObjects)
    expect(spec.scheduleCron).toBe("0 * * * *")
    expect((spec.dial as Record<string, unknown>).user).toBe("someone_else")
    expect("credential" in spec).toBe(false)
  })

  /**
   * SEC-14: pointing the connector at another host cannot keep the stored
   * credential, so the password is required before the form moves on, and it
   * travels WITH the connection settings in one ingest-spec request (no
   * separate credential call). The host label follows, after the settings.
   */
  it("asks for the credential again when the host changes and sends it with the settings", async () => {
    const calls = stubFetch()
    const page = renderPage()
    await waitFor(() => page.getByLabelText("Name"))
    next(page)
    fireEvent.change(page.getByLabelText("Host"), { target: { value: "10.0.0.9" } })
    // Blank no longer means "keep": the stored password is not sent to a new host.
    expect(page.getByText(/points the connector at a different place/)).toBeDefined()
    expect((page.getByRole("button", { name: "Next" }) as HTMLButtonElement).disabled).toBe(true)
    fireEvent.change(page.getByLabelText("Password"), { target: { value: "pw" } })
    expect((page.getByRole("button", { name: "Next" }) as HTMLButtonElement).disabled).toBe(false)
    next(page)
    next(page)
    expect(page.queryByText(/pw$/)).toBeNull()
    fireEvent.click(page.getByRole("button", { name: "Save changes" }))
    await waitFor(() => expect(page.getByText(/Connection test passed/)).toBeDefined())

    const w = writes(calls)
    expect(w.some((c) => c.url.endsWith("/credential"))).toBe(false)
    expect(w[0].url).toContain("/ingest-spec")
    const spec = w[0].body as Record<string, unknown>
    expect((spec.dial as Record<string, unknown>).host).toBe("10.0.0.9")
    expect(spec.credential).toEqual({ primary: { kind: "password", value: "pw" } })
    expect(spec.sourceObjects).toEqual(SPEC.sourceObjects)
    expect(spec.scheduleCron).toBe("0 * * * *")
    expect(w[1].body).toEqual({ host: "10.0.0.9" })
  })

  /**
   * When the server refuses the re-point (409, or a credential that does not
   * work at the new place) its message is shown, and nothing after it runs:
   * not the label, not the connection test.
   */
  it("shows the server's message when it refuses to point the connector elsewhere", async () => {
    const calls = stubFetch({
      "PUT /api/connectors/conn-a/ingest-spec": () =>
        json({ error: "the change was NOT saved: PostgreSQL connection failed: authentication failed" }, 422),
    })
    const page = renderPage()
    await waitFor(() => page.getByLabelText("Name"))
    next(page)
    fireEvent.change(page.getByLabelText("Host"), { target: { value: "10.0.0.9" } })
    fireEvent.change(page.getByLabelText("Password"), { target: { value: "wrong" } })
    next(page)
    next(page)
    fireEvent.click(page.getByRole("button", { name: "Save changes" }))

    await waitFor(() => expect(page.getByText("Some changes were not saved")).toBeDefined())
    expect(page.getByText(/Not saved · Connection settings and credential .*authentication failed/)).toBeDefined()
    expect(writes(calls).map((c) => c.method)).toEqual(["PUT"])
  })

  /**
   * A connector with no adapter yet dials from its `host` column, which the
   * API will not change through PATCH. Its first connection settings are not
   * a re-point (no credential is asked for), and the label is not sent.
   */
  it("does not send the host label for a connector that has no connection settings yet", async () => {
    const calls = stubFetch({
      "GET /api/connectors/conn-a/ingest-spec": () =>
        json({
          adapter: null,
          ingestMode: null,
          dial: {},
          sourceObjects: [],
          scheduleCron: null,
          secretRefs: { primary: "env:CONNECTOR_CONN_A_PASSWORD", secondary: null },
        }),
    })
    const page = renderPage()
    await waitFor(() => page.getByLabelText("Name"))
    next(page)
    fireEvent.change(await waitFor(() => page.getByLabelText("Host")), { target: { value: "10.0.0.9" } })
    expect((page.getByRole("button", { name: "Next" }) as HTMLButtonElement).disabled).toBe(false)
    next(page)
    next(page)
    fireEvent.click(page.getByRole("button", { name: "Save changes" }))
    await waitFor(() => expect(page.getByText(/Connection test passed/)).toBeDefined())

    const w = writes(calls)
    expect(w.some((c) => c.method === "PATCH")).toBe(false)
    expect(w.some((c) => c.url.endsWith("/credential"))).toBe(false)
    expect("credential" in (w[0].body as Record<string, unknown>)).toBe(false)
  })

  it("says so plainly when moving tenants is not permitted", async () => {
    stubFetch({
      "PUT /api/connectors/conn-a/tenant": () => json({ error: "permission_denied" }, 403),
    })
    const page = renderPage()
    await waitFor(() => page.getByLabelText("Name"))
    next(page)
    next(page)
    fireEvent.click(page.getByRole("radio", { name: /Other Co/ }))
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
   * all: the section names the pipelines, the button is off, and no DELETE
   * is ever sent.
   */
  it("refuses to delete a connector pipelines still use", async () => {
    const calls = stubFetch({}, {
      ...DETAIL,
      dependentPipelines: [{ id: "p-orders", name: "orders sync", kind: "pipeline" }],
    })
    const page = renderPage()
    await waitFor(() => page.getByLabelText("Name"))
    expect(page.getByText(/Still used by the pipeline/)).toBeDefined()
    expect(page.getByRole("link", { name: "orders sync" }).getAttribute("href")).toBe("/pipelines/p-orders")
    const remove = page.getByRole("button", { name: "Delete connector…" }) as HTMLButtonElement
    expect(remove.disabled).toBe(true)
    fireEvent.click(remove)
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
    fireEvent.click(page.getByRole("button", { name: "Delete connector…" }))
    fireEvent.click(await waitFor(() => screen.getByRole("button", { name: "Delete" })))
    await waitFor(() => expect(screen.getByText(/dropping slot x_slot failed/)).toBeDefined())
    fireEvent.click(screen.getByRole("button", { name: "Force delete" }))
    await waitFor(() =>
      expect(calls.filter((c) => c.method === "DELETE").map((c) => c.url.replace(/^https?:\/\/[^/]+/, ""))).toEqual([
        "/api/connectors/conn-a",
        "/api/connectors/conn-a?force=true",
      ])
    )
    // The connector is gone, so there is no page of it to go back to: the list.
    await waitFor(() => expect(pushed).toEqual(["/connectors"]))
  })

  /**
   * A sign-in method that reads a different credential (REST bearer to
   * basic auth) cannot keep the stored one: the new username and password
   * are required, go first with the new settings, and only then are the
   * settings saved.
   */
  it("requires a new credential when the sign-in method changes, and saves it first", async () => {
    const restSpec = {
      adapter: "rest",
      ingestMode: "batch",
      dial: {
        baseUrl: "https://api.example.com",
        auth: { type: "bearer" },
        pagination: { type: "none" },
        endpoints: [{ path: "/orders", recordsPath: null }],
      },
      sourceObjects: [],
      scheduleCron: null,
      secretRefs: { primary: "file:/run/secrets/connector_managed_conn_a_token", secondary: null },
    }
    const calls = stubFetch(
      {
        "GET /api/connectors/types": () => json([{ name: "REST API", adapter: "rest", supported: true, docsUrl: null, unsupportedReason: null }]),
        "GET /api/connectors/conn-a/ingest-spec": () => json(restSpec),
        "PUT /api/connectors/conn-a/ingest-spec": () => json(restSpec),
      },
      { ...DETAIL, type: "REST API", credentialManaged: true, credentialKind: "token", credentialSecondaryKind: null }
    )
    const page = renderPage()
    await waitFor(() => page.getByLabelText("Name"))
    next(page)
    fireEvent.change(page.getByLabelText("Auth type"), { target: { value: "basic" } })
    expect(page.getByText(/sign-in method changed/)).toBeDefined()
    // Blank no longer means "keep": the stored token does not fit basic auth.
    expect((page.getByRole("button", { name: "Next" }) as HTMLButtonElement).disabled).toBe(true)
    fireEvent.change(page.getByLabelText("Username"), { target: { value: "svc" } })
    fireEvent.change(page.getByLabelText("Password"), { target: { value: "pw" } })
    next(page)
    next(page)
    fireEvent.click(page.getByRole("button", { name: "Save changes" }))
    await waitFor(() => expect(page.getByText(/Connection test passed/)).toBeDefined())

    const w = writes(calls).map((c) => c.url.replace(/^https?:\/\/[^/]+/, ""))
    expect(w.indexOf("/api/connectors/conn-a/credential")).toBeGreaterThan(-1)
    expect(w.indexOf("/api/connectors/conn-a/credential")).toBeLessThan(w.indexOf("/api/connectors/conn-a/ingest-spec"))
    const credential = writes(calls).find((c) => c.url.endsWith("/credential"))!.body as Record<string, unknown>
    expect(credential.primary).toEqual({ kind: "access_key", value: "svc" })
    expect(credential.secondary).toEqual({ kind: "password", value: "pw" })
    expect((credential.dial as { auth: { type: string } }).auth.type).toBe("basic")
  })

  /** A credential the build could not test is saved, and the result says so. */
  it("says when a new credential was stored without being tested", async () => {
    stubFetch({
      "PUT /api/connectors/conn-a/credential": () =>
        json({ saved: true, slots: ["primary"], verified: false, message: "SFTP cannot be probed" }),
    })
    const page = renderPage()
    await waitFor(() => page.getByLabelText("Name"))
    next(page)
    fireEvent.change(page.getByLabelText("Password"), { target: { value: "pw" } })
    next(page)
    next(page)
    fireEvent.click(page.getByRole("button", { name: "Save changes" }))
    await waitFor(() => expect(page.getByText(/stored without a test/)).toBeDefined())
  })
})
