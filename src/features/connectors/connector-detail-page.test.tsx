// A connector has a page of its own at `/connectors/<id>` (plan
// `docs/superpowers/plans/2026-10-05-connector-detail-page.md`, T1): the body
// of the side sheet the Sources list used to open, with a header, the three
// tabs in `?tab=`, a way back to Sources and, after a delete, the way back
// too.
const url = { search: "" }
const pushed: string[] = []
mock.module("next/navigation", () => ({
  usePathname: () => "/connectors/conn-a",
  useSearchParams: () => new URLSearchParams(url.search),
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

import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, mock, spyOn } from "bun:test"
import { toast } from "sonner"
import { ConnectorDetailPage } from "./connector-detail-page"

const originalFetch = global.fetch

beforeEach(() => {
  spyOn(window.history, "replaceState").mockImplementation(() => {})
})

afterEach(() => {
  global.fetch = originalFetch
  url.search = ""
  pushed.length = 0
  mock.restore()
  cleanup()
})

function json(body: unknown, status = 200) {
  return new Response(body === null ? null : JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  })
}

const DETAIL = {
  id: "conn-a",
  name: "db demo",
  type: "PostgreSQL",
  direction: "source",
  health: "healthy",
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
  owner: "admin",
  discoveredAssets: 0,
  discoveredSchemas: [],
  recentErrors: [],
  dependentPipelines: [] as { id: string; name: string; kind: string }[],
  auditEventId: "evt-1",
  credentialManaged: true,
  credentialSource: "managed",
  credentialKind: "password",
  credentialSecondaryKind: null,
  residency: "id-jakarta",
  tenantId: "t-acme",
}

const SPEC = {
  adapter: "sql",
  ingestMode: "batch",
  dial: { driver: "postgres", host: "192.168.18.205", port: 5432, database: "insurance_db", user: "insurer", sslMode: null },
  sourceObjects: [] as { name: string; target: string }[],
  scheduleCron: null,
  secretRefs: { primary: "file:/run/secrets/connector_managed_x_password", secondary: null },
  nextRunAt: null,
}

type Call = { url: string; method: string }
type Probe = { testedAt: string; ok: boolean; latencyMs: number | null; message: string }

/**
 * The reads the page and its three tabs make, and the two writes it can:
 * a test appends a row to the history the way `POST .../test` does server
 * side. `overrides` are keyed `"<METHOD> <path>"`.
 */
function stubFetch(
  overrides: Record<string, () => Response | Promise<Response>> = {},
  detail: Record<string, unknown> = DETAIL
): { calls: Call[]; count: (key: string) => number } {
  const calls: Call[] = []
  const probes: Probe[] = []
  global.fetch = mock(async (input: RequestInfo | URL, init?: RequestInit) => {
    const target = String(input).replace(/^https?:\/\/[^/]+/, "")
    const method = init?.method ?? "GET"
    calls.push({ url: target, method })
    const key = `${method} ${target}`
    if (overrides[key]) return overrides[key]()
    if (key === "GET /api/connectors/credential-settings") return json({ secretManager: null, typedCredentials: true })
    if (key === "GET /api/connectors/conn-a") return json(detail)
    if (key === "GET /api/connectors/conn-a/ingest-spec") return json(SPEC)
    if (key.startsWith("GET /api/connectors/conn-a/probe-history")) {
      return json({ results: key.endsWith("?limit=1") ? probes.slice(0, 1) : probes })
    }
    if (key === "GET /api/connectors/conn-a/schema-changes") return json({ pending: [], recent: [], inactiveColumns: [] })
    if (key === "GET /api/connectors/conn-a/runs") return json({ runs: [] })
    if (key === "POST /api/connectors/conn-a/test") {
      probes.unshift({ testedAt: "2026-10-05T01:00:00.000Z", ok: true, latencyMs: 5, message: "Connected via PostgreSQL" })
      return json({ ok: true, supported: true, latencyMs: 5, message: "Connected via PostgreSQL", testedAt: probes[0].testedAt })
    }
    if (key === "DELETE /api/connectors/conn-a") return new Response(null, { status: 204 })
    throw new Error(`unexpected fetch: ${key}`)
  }) as unknown as typeof fetch
  return { calls, count: (key) => calls.filter((c) => `${c.method} ${c.url}` === key).length }
}

/** Lets the reads a tab makes on mount finish inside `act`, not after the test. */
const settle = () =>
  act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 0))
  })

/** A tab by its label; a count it carries ("Ingest 2") is part of its name. */
function tab(name: string): HTMLElement {
  return screen.getByRole("tab", { name: new RegExp(`^${name}`) })
}

function selected(name: string): string | null {
  return tab(name).getAttribute("aria-selected")
}

describe("ConnectorDetailPage", () => {
  it("shows a skeleton and the way back to Sources while the connector loads", () => {
    stubFetch({ "GET /api/connectors/conn-a": () => new Promise<Response>(() => {}) })
    render(<ConnectorDetailPage connectorId="conn-a" />)
    expect(screen.getByRole("status", { name: "Loading" })).toBeDefined()
    expect(screen.getByRole("link", { name: "Sources" }).getAttribute("href")).toBe("/connectors")
  })

  it("says the connector was not found, in the API's words, with a way back", async () => {
    stubFetch({ "GET /api/connectors/conn-a": () => json({ error: "connector conn-a not found" }, 404) })
    render(<ConnectorDetailPage connectorId="conn-a" />)
    expect(await screen.findByText("Not found")).toBeDefined()
    expect(screen.getByText(/connector conn-a not found/)).toBeDefined()
    expect(screen.getByRole("link", { name: "Sources" }).getAttribute("href")).toBe("/connectors")
    expect(screen.queryByRole("tablist")).toBeNull()
  })

  it("keeps the way back when the read is refused for any other reason", async () => {
    stubFetch({ "GET /api/connectors/conn-a": () => json({ error: "tenant scope does not include this connector" }, 403) })
    render(<ConnectorDetailPage connectorId="conn-a" />)
    expect(await screen.findByText(/tenant scope does not include this connector/)).toBeDefined()
    expect(screen.getByRole("link", { name: "Sources" }).getAttribute("href")).toBe("/connectors")
    expect(screen.queryByRole("tablist")).toBeNull()
  })

  it("heads the page with the name, its type, health, direction and environment, and the actions", async () => {
    stubFetch()
    render(<ConnectorDetailPage connectorId="conn-a" />)
    const title = await screen.findByRole("heading", { level: 1, name: "db demo" })
    const badges = within(title.parentElement as HTMLElement)
    expect(badges.getByText("Healthy")).toBeDefined()
    expect(badges.getByText("Source")).toBeDefined()
    expect(badges.getByText("production")).toBeDefined()
    // The header block: the name's wrapper, the text column, then the block.
    const header = within(((title.parentElement as HTMLElement).parentElement as HTMLElement).parentElement as HTMLElement)
    expect(header.getByText("PostgreSQL")).toBeDefined()
    expect(header.getByRole("link", { name: "Sources" }).getAttribute("href")).toBe("/connectors")
    // `Button render={<Link />}` is an anchor with `role="button"`.
    expect(header.getByRole("button", { name: "Edit" }).getAttribute("href")).toBe("/connectors/conn-a/edit")
    expect(header.getByRole("button", { name: "Create pipeline" }).getAttribute("href")).toBe(
      "/pipelines/create?connectorId=conn-a"
    )
    expect(header.getByRole("button", { name: "Audit" }).getAttribute("href")).toBe("/audit?event=evt-1")
    expect(header.getByRole("button", { name: "Test connection" })).toBeDefined()
    expect((header.getByRole("button", { name: "Delete" }) as HTMLButtonElement).disabled).toBe(false)
  })

  it("opens on Overview, with the address left clean", async () => {
    stubFetch()
    render(<ConnectorDetailPage connectorId="conn-a" />)
    expect(await screen.findByText("No pipeline reads from this connector.")).toBeDefined()
    const names = screen.getAllByRole("tab").map((t) => t.textContent ?? "")
    // Ingest carries a count once the saved spec is read; the others none.
    expect(names[0]).toBe("Overview")
    expect(names[1]).toMatch(/^Ingest\d*$/)
    // `SRC-8` added the Schema changes tab between Ingest and Connection tests
    // (it carries a count only while a table waits, so none here).
    expect(names[2]).toBe("Schema changes")
    expect(names[3]).toBe("Connection tests")
    expect(selected("Overview")).toBe("true")
    expect(selected("Ingest")).toBe("false")
    expect(selected("Connection tests")).toBe("false")
    expect(window.history.replaceState).not.toHaveBeenCalled()
  })

  /** `SRC-12` part B: a reference credential, and the secret manager the installation reads from. */
  it("says the credential is a reference and names the secret manager, when one is set up", async () => {
    stubFetch(
      { "GET /api/connectors/credential-settings": () => json({ secretManager: "gcp", typedCredentials: true }) },
      { ...DETAIL, credentialSource: "reference", credentialManaged: false }
    )
    render(<ConnectorDetailPage connectorId="conn-a" />)
    await screen.findByRole("heading", { level: 1, name: "db demo" })
    expect(await screen.findByText(/Reference to a secret in the secret manager/)).toBeDefined()
    expect(await screen.findByText("Google Secret Manager")).toBeDefined()
  })

  it("leaves the secret manager fact out when none is set up", async () => {
    stubFetch()
    render(<ConnectorDetailPage connectorId="conn-a" />)
    await screen.findByRole("heading", { level: 1, name: "db demo" })
    await settle()
    expect(screen.queryByText("Secret manager")).toBeNull()
  })

  /** `SRC-12` part C: a REST connector that signs in with OAuth gets the Sign in panel; no other connector does. */
  const OAUTH_SPEC = {
    ...SPEC,
    adapter: "rest",
    dial: {
      baseUrl: "https://api.example.com",
      auth: {
        type: "oauth2_authorization_code",
        clientId: "app-1",
        authorizeUrl: "https://login.example.com/authorize",
        tokenUrl: "https://login.example.com/token",
        scopes: [],
      },
      pagination: { type: "none" },
      endpoints: [{ path: "/items", recordsPath: null }],
    },
  }

  it("shows the Sign in panel for a connector that signs in with OAuth, and asks for its status", async () => {
    const { count } = stubFetch({
      "GET /api/connectors/conn-a/ingest-spec": () => json(OAUTH_SPEC),
      "GET /api/connectors/conn-a/oauth": () =>
        json({ signedIn: false, callbackUrl: "https://console.test/api/connectors/oauth/callback", revocationConfigured: false }),
    })
    render(<ConnectorDetailPage connectorId="conn-a" />)
    expect(await screen.findByText("Not signed in")).toBeDefined()
    expect(screen.getByRole("button", { name: "Sign in" })).toBeDefined()
    expect(count("GET /api/connectors/conn-a/oauth")).toBeGreaterThan(0)
  })

  it("shows no Sign in panel, and never asks for OAuth status, for any other connector", async () => {
    const { count } = stubFetch()
    render(<ConnectorDetailPage connectorId="conn-a" />)
    await screen.findByRole("heading", { level: 1, name: "db demo" })
    await settle()
    expect(screen.queryByRole("button", { name: "Sign in" })).toBeNull()
    expect(count("GET /api/connectors/conn-a/oauth")).toBe(0)
  })

  it("tells the person, before deleting an OAuth connector, whether the provider can be told", async () => {
    stubFetch({
      "GET /api/connectors/conn-a/ingest-spec": () => json(OAUTH_SPEC),
      "GET /api/connectors/conn-a/oauth": () =>
        json({ signedIn: true, callbackUrl: null, revocationConfigured: false }),
    })
    render(<ConnectorDetailPage connectorId="conn-a" />)
    await screen.findByText("Signed in")
    fireEvent.click(await screen.findByRole("button", { name: "Delete" }))
    const dialog = await screen.findByRole("dialog")
    expect(within(dialog).getByText(/the token stays valid there until it expires/)).toBeDefined()
  })

  it("marks a paused connector in the header and holds Run now back with the reason", async () => {
    url.search = "?tab=ingest"
    stubFetch({}, {
      ...DETAIL,
      pausedReason: "A schema change at the source is waiting for approval.",
      pausedAt: "2026-10-09T01:00:00.000Z",
    })
    render(<ConnectorDetailPage connectorId="conn-a" />)
    const title = await screen.findByRole("heading", { level: 1, name: "db demo" })
    expect(
      within(title.parentElement as HTMLElement).getByText(
        "Paused: A schema change at the source is waiting for approval."
      )
    ).toBeDefined()
    // The ingest spec has no tables, so the paused reason must win over "Save at least one table".
    expect(await screen.findByText("Paused: A schema change at the source is waiting for approval.", { selector: "p" })).toBeDefined()
    expect((screen.getByRole("button", { name: "Run now" }) as HTMLButtonElement).disabled).toBe(true)
  })

  it("marks a table that waits for a schema decision in the Ingest tab, from the one list the Schema tab reads", async () => {
    url.search = "?tab=ingest"
    const { count } = stubFetch(
      {
        "GET /api/connectors/conn-a/ingest-spec": () =>
          json({ ...SPEC, sourceObjects: [{ name: "public.orders", target: "orders" }, { name: "public.items", target: "items" }] }),
        "GET /api/connectors/conn-a/schema-changes": () =>
          json({
            pending: [
              {
                id: "chg-1", objectName: "public.orders", kind: "column_removed", columnName: "note", beforeValue: "text",
                afterValue: null, breaking: true, status: "pending", runId: null, detectedAt: "2026-10-09T01:00:00.000Z",
                decidedBy: null, decidedAt: null, canApprove: true,
              },
            ],
            recent: [],
            inactiveColumns: [],
          }),
      }
    )
    render(<ConnectorDetailPage connectorId="conn-a" />)
    // SRC-10 R4 and SRC-17 (PR #124's CI): the same flake, fixed the same way.
    // "Waiting for a decision" is also the title of the always-rendered Schema
    // changes card, so awaiting that text resolved while the list was still
    // loading, before the tab's count and the Ingest mark arrived. Await the mark
    // itself (found by its title, which only the Ingest pill carries), then the
    // count, then assert.
    const marks = await screen.findAllByTitle("A change at the source waits for approval; see Schema changes")
    expect(marks).toHaveLength(1)
    expect(marks[0].textContent).toBe("Waiting for a decision")
    expect(within(marks[0].closest("li") as HTMLElement).getByText("public.orders")).toBeDefined()
    expect(within(screen.getByText("public.items").closest("li") as HTMLElement).queryByText("Waiting for a decision")).toBeNull()
    await waitFor(() => expect(tab("Schema changes").textContent).toBe("Schema changes1"))
    await settle()
    expect(screen.getAllByTitle("A change at the source waits for approval; see Schema changes")).toHaveLength(1)
    expect(count("GET /api/connectors/conn-a/schema-changes")).toBe(1)
  })

  it("opens the Ingest tab for ?tab=ingest and the Connection tests tab for ?tab=tests", async () => {
    url.search = "?tab=ingest"
    stubFetch()
    const first = render(<ConnectorDetailPage connectorId="conn-a" />)
    expect(await screen.findByText("No tables selected yet. Find tables below and tick the ones to copy.")).toBeDefined()
    expect(selected("Ingest")).toBe("true")
    first.unmount()

    url.search = "?tab=tests"
    stubFetch()
    render(<ConnectorDetailPage connectorId="conn-a" />)
    expect(await screen.findByText("Not tested yet. Only tests this build can run are recorded.")).toBeDefined()
    expect(selected("Connection tests")).toBe("true")
  })

  it("falls back to Overview for a tab it does not have", async () => {
    url.search = "?tab=nope"
    stubFetch()
    render(<ConnectorDetailPage connectorId="conn-a" />)
    expect(await screen.findByText("No pipeline reads from this connector.")).toBeDefined()
    expect(selected("Overview")).toBe("true")
  })

  it("writes the open tab to the address, and clears it for Overview", async () => {
    stubFetch()
    render(<ConnectorDetailPage connectorId="conn-a" />)
    await screen.findByText("No pipeline reads from this connector.")

    fireEvent.click(tab("Ingest"))
    expect(window.history.replaceState).toHaveBeenLastCalledWith(null, "", "/connectors/conn-a?tab=ingest")
    await screen.findByText("No tables selected yet. Find tables below and tick the ones to copy.")
    fireEvent.click(tab("Connection tests"))
    expect(window.history.replaceState).toHaveBeenLastCalledWith(null, "", "/connectors/conn-a?tab=tests")
    await screen.findByText("Not tested yet. Only tests this build can run are recorded.")
    fireEvent.click(tab("Overview"))
    expect(window.history.replaceState).toHaveBeenLastCalledWith(null, "", "/connectors/conn-a")
    await settle()
  })

  it("lets the Overview open the Ingest tab of the page", async () => {
    stubFetch()
    render(<ConnectorDetailPage connectorId="conn-a" />)
    fireEvent.click(await screen.findByRole("button", { name: /^Tables/ }))
    await waitFor(() => expect(selected("Ingest")).toBe("true"))
    expect(window.history.replaceState).toHaveBeenLastCalledWith(null, "", "/connectors/conn-a?tab=ingest")
  })

  it("keeps the Ingest tab mounted, so a table search typed there survives a look at another tab", async () => {
    stubFetch()
    render(<ConnectorDetailPage connectorId="conn-a" />)
    await screen.findByText("No pipeline reads from this connector.")
    fireEvent.click(tab("Ingest"))
    const schema = (await screen.findByLabelText("Schema")) as HTMLInputElement
    expect(schema.value).toBe("public")
    fireEvent.change(schema, { target: { value: "sales" } })

    fireEvent.click(tab("Connection tests"))
    await screen.findByText("Not tested yet. Only tests this build can run are recorded.")
    fireEvent.click(tab("Ingest"))
    expect(((await screen.findByLabelText("Schema")) as HTMLInputElement).value).toBe("sales")
  })

  it("tests the connection, shows the result line, reloads the detail and lists the test in the history", async () => {
    // The second read of the detail is held, to look at the page while the
    // reload is still out.
    let reads = 0
    let release: (r: Response) => void = () => {}
    const { count } = stubFetch({
      "GET /api/connectors/conn-a": () =>
        ++reads === 1
          ? json(DETAIL)
          : new Promise<Response>((resolve) => {
              release = resolve
            }),
    })
    render(<ConnectorDetailPage connectorId="conn-a" />)
    await screen.findByText("No pipeline reads from this connector.")

    fireEvent.click(screen.getByRole("button", { name: "Test connection" }))
    expect(await screen.findByText("Connected via PostgreSQL · 5 ms")).toBeDefined()
    await waitFor(() => expect(count("POST /api/connectors/conn-a/test")).toBe(1))
    await waitFor(() => expect(reads).toBe(2))
    // The page stays on screen for the reload: no skeleton in its place,
    // which would also unmount the Ingest tab.
    expect(screen.queryByRole("status", { name: "Loading" })).toBeNull()
    expect(screen.getByRole("heading", { level: 1, name: "db demo" })).toBeDefined()
    release(json({ ...DETAIL, health: "degraded" }))
    // The new health shows in the header and in the Overview's Health tile.
    await waitFor(() => expect(screen.getAllByText("Degraded").length).toBe(2))

    fireEvent.click(tab("Connection tests"))
    expect(await screen.findByText("Passed")).toBeDefined()
    expect(screen.getByText("Connected via PostgreSQL")).toBeDefined()
  })

  it("says plainly when the connector type cannot be tested", async () => {
    stubFetch({
      "POST /api/connectors/conn-a/test": () =>
        json({ ok: false, supported: false, latencyMs: null, message: "This build has no probe for MQTT", testedAt: null }),
    })
    render(<ConnectorDetailPage connectorId="conn-a" />)
    fireEvent.click(await screen.findByRole("button", { name: "Test connection" }))
    expect(await screen.findByText("Not testable")).toBeDefined()
    expect(screen.getByText("This build has no probe for MQTT")).toBeDefined()
  })

  it("lists the connector's facts in a row under the title, and takes them out of the Overview", async () => {
    stubFetch()
    const { container } = render(<ConnectorDetailPage connectorId="conn-a" />)
    await screen.findByRole("heading", { level: 1, name: "db demo" })
    // The first definition list of the page is the header's row of facts.
    const facts = within(container.querySelector("dl") as HTMLElement)
    const fact = (label: string) => facts.getByText(label).nextElementSibling?.textContent
    expect(fact("Tenant")).toBe("Acme Co")
    expect(fact("Residency")).toBe("id-jakarta")
    expect(fact("Owner")).toBe("admin")
    expect(fact("Credential")).toBe("Stored by lakehouse, encrypted")
    expect(fact("Last test")).toBe("Never tested")
    // The type (the description) and the environment (a pill by the name) are
    // in the header already, so the row does not say them a second time.
    expect(facts.queryByText("Type")).toBeNull()
    expect(facts.queryByText("Environment")).toBeNull()
    expect(screen.getAllByText("PostgreSQL").length).toBe(1)
    expect(screen.getAllByText("production").length).toBe(1)
    // The Overview no longer has a "Details" section repeating them.
    await screen.findByText("No pipeline reads from this connector.")
    expect(screen.queryByText("Details")).toBeNull()
    expect(screen.queryByText("Direction")).toBeNull()
  })

  it("says plainly what a fact lacks, and when the last test was, with the full time on hover", async () => {
    stubFetch(
      {},
      {
        ...DETAIL,
        tenant: "",
        environment: "",
        residency: "",
        owner: "",
        credentialManaged: false,
        credentialSource: "env",
        lastTestAt: "2026-10-04T00:00:00.000Z",
      }
    )
    const { container } = render(<ConnectorDetailPage connectorId="conn-a" />)
    await screen.findByRole("heading", { level: 1, name: "db demo" })
    const facts = within(container.querySelector("dl") as HTMLElement)
    const fact = (label: string) => facts.getByText(label).nextElementSibling
    expect(fact("Tenant")?.textContent).toBe("Unassigned")
    expect(fact("Residency")?.textContent).toBe("—")
    expect(fact("Owner")?.textContent).toBe("—")
    // SRC-12 decision 10: an environment variable says a restart is needed.
    expect(fact("Credential")?.textContent).toBe(
      "Environment variable on the server (a restart is needed after a change)"
    )
    const lastTest = fact("Last test")
    expect(lastTest?.textContent).not.toBe("Never tested")
    expect(lastTest?.querySelector("[title]")?.getAttribute("title")).toMatch(/2026/)
  })

  it("gives each tab its icon", async () => {
    stubFetch()
    render(<ConnectorDetailPage connectorId="conn-a" />)
    await screen.findByText("No pipeline reads from this connector.")
    for (const t of screen.getAllByRole("tab")) expect(t.querySelector("svg")).not.toBeNull()
  })

  it("counts the saved tables on the Ingest tab, and none on the tab with nothing to count", async () => {
    stubFetch({
      "GET /api/connectors/conn-a/ingest-spec": () =>
        json({
          ...SPEC,
          sourceObjects: [
            { name: "public.orders", target: "db_demo_orders" },
            { name: "public.customers", target: "db_demo_customers" },
          ],
        }),
    })
    render(<ConnectorDetailPage connectorId="conn-a" />)
    await waitFor(() => expect(tab("Ingest").textContent).toBe("Ingest2"))
    expect(tab("Overview").textContent).toBe("Overview")
    // No test yet: nothing to flag on Connection tests.
    expect(tab("Connection tests").textContent).toBe("Connection tests")
  })

  it("counts a zero for a connector whose connection is saved but that has no tables", async () => {
    stubFetch()
    render(<ConnectorDetailPage connectorId="conn-a" />)
    await waitFor(() => expect(tab("Ingest").textContent).toBe("Ingest0"))
  })

  it("puts no count on the Ingest tab for a connector with no connection saved, rather than a zero", async () => {
    stubFetch({
      "GET /api/connectors/conn-a/ingest-spec": () =>
        json({ ...SPEC, adapter: null, ingestMode: null, sourceObjects: [] }),
    })
    render(<ConnectorDetailPage connectorId="conn-a" />)
    // Both ingest tiles of the Overview say so, and the panel has read the
    // same spec by then.
    expect((await screen.findAllByText("Not set up yet")).length).toBe(2)
    await settle()
    expect(tab("Ingest").textContent).toBe("Ingest")
  })

  it("flags Connection tests with a red 1 while the latest test failed, and clears it after a passing one", async () => {
    let failing = true
    const probe = () => ({
      testedAt: "2026-10-05T00:00:00.000Z",
      ok: !failing,
      latencyMs: failing ? null : 5,
      message: failing ? "connection refused" : "Connected via PostgreSQL",
    })
    stubFetch({
      "GET /api/connectors/conn-a/probe-history?limit=1": () => json({ results: [probe()] }),
      "POST /api/connectors/conn-a/test": () => {
        failing = false
        return json({ ok: true, supported: true, latencyMs: 5, message: "Connected via PostgreSQL", testedAt: probe().testedAt })
      },
    })
    render(<ConnectorDetailPage connectorId="conn-a" />)
    await waitFor(() => expect(tab("Connection tests").textContent).toBe("Connection tests1"))
    // A failure reads as trouble: the count is in the danger tone.
    expect(within(tab("Connection tests")).getByText("1").className).toContain("text-destructive")

    fireEvent.click(screen.getByRole("button", { name: "Test connection" }))
    await waitFor(() => expect(tab("Connection tests").textContent).toBe("Connection tests"))
  })

  it("puts no count on Connection tests when the latest test passed, even if an older one failed", async () => {
    stubFetch({
      "GET /api/connectors/conn-a/probe-history?limit=1": () =>
        json({ results: [{ testedAt: "2026-10-05T00:00:00.000Z", ok: true, latencyMs: 5, message: "Connected via PostgreSQL" }] }),
    })
    render(<ConnectorDetailPage connectorId="conn-a" />)
    await screen.findByText("No pipeline reads from this connector.")
    await settle()
    expect(tab("Connection tests").textContent).toBe("Connection tests")
  })

  describe("the result of Test connection", () => {
    function spyToasts() {
      return {
        success: spyOn(toast, "success").mockImplementation(() => 1),
        error: spyOn(toast, "error").mockImplementation(() => 1),
        info: spyOn(toast, "info").mockImplementation(() => 1),
      }
    }
    const titleOf = (spy: { mock: { calls: unknown[][] } }) => spy.mock.calls.map((c) => c[0])

    it("announces a pass only when the probe ran and passed, and shows a notice in the success tone", async () => {
      const toasts = spyToasts()
      stubFetch()
      render(<ConnectorDetailPage connectorId="conn-a" />)
      fireEvent.click(await screen.findByRole("button", { name: "Test connection" }))
      const notice = (await screen.findByText("Connection test passed")).closest("[role=status]") as HTMLElement
      expect(notice.textContent).toContain("Connected via PostgreSQL · 5 ms")
      expect(notice.className).toContain("emerald")
      expect(titleOf(toasts.success)).toEqual(["Connection test passed"])
      expect(toasts.error).not.toHaveBeenCalled()
      expect(toasts.info).not.toHaveBeenCalled()
    })

    it("announces a failure, not a pass, when the probe ran and did not connect, though the request itself succeeded", async () => {
      const toasts = spyToasts()
      stubFetch({
        "POST /api/connectors/conn-a/test": () =>
          json({ ok: false, supported: true, latencyMs: 12, message: "connection refused", testedAt: "2026-10-05T01:00:00.000Z" }),
      })
      render(<ConnectorDetailPage connectorId="conn-a" />)
      fireEvent.click(await screen.findByRole("button", { name: "Test connection" }))
      const notice = (await screen.findByText("Connection test failed")).closest("[role=status]") as HTMLElement
      expect(notice.textContent).toContain("connection refused · 12 ms")
      expect(notice.className).toContain("destructive")
      expect(titleOf(toasts.error)).toEqual(["Connection test failed"])
      expect(toasts.success).not.toHaveBeenCalled()
      expect(toasts.info).not.toHaveBeenCalled()
    })

    it("says the type cannot be tested, as neither a pass nor a failure", async () => {
      const toasts = spyToasts()
      stubFetch({
        "POST /api/connectors/conn-a/test": () =>
          json({ ok: false, supported: false, latencyMs: null, message: "This build has no probe for MQTT", testedAt: null }),
      })
      render(<ConnectorDetailPage connectorId="conn-a" />)
      fireEvent.click(await screen.findByRole("button", { name: "Test connection" }))
      const notice = (await screen.findByText("Not testable")).closest("[role=status]") as HTMLElement
      expect(notice.textContent).toContain("This build has no probe for MQTT")
      expect(notice.textContent).not.toContain("ms")
      expect(titleOf(toasts.info)).toEqual(["This connector type cannot be tested"])
      expect(toasts.success).not.toHaveBeenCalled()
      expect(toasts.error).not.toHaveBeenCalled()
    })

    it("still reports a request that failed outright as a failed test, with the translated error, and shows no notice", async () => {
      const toasts = spyToasts()
      stubFetch({ "POST /api/connectors/conn-a/test": () => json({ error: "probe worker down" }, 500) })
      render(<ConnectorDetailPage connectorId="conn-a" />)
      fireEvent.click(await screen.findByRole("button", { name: "Test connection" }))
      await waitFor(() => expect(toasts.error).toHaveBeenCalledTimes(1))
      expect(toasts.error.mock.calls[0][0]).toBe("Connection test failed")
      expect(toasts.success).not.toHaveBeenCalled()
      // A refused request has no result to put in a notice.
      expect(screen.queryByText(/^Connection test (passed|failed)$/)).toBeNull()
    })
  })

  it("disables Delete, with the reason, while pipelines use the connector", async () => {
    stubFetch({}, { ...DETAIL, dependentPipelines: [{ id: "pl-1", name: "ingest_demo", kind: "pipeline" }] })
    render(<ConnectorDetailPage connectorId="conn-a" />)
    const del = (await screen.findByRole("button", { name: "Delete" })) as HTMLButtonElement
    expect(del.disabled).toBe(true)
    expect(del.getAttribute("title")).toBe(
      "Used by 1 pipeline (see Used by); those must be deleted or moved first"
    )
    expect(screen.getByRole("link", { name: "ingest_demo" }).getAttribute("href")).toBe("/pipelines/pl-1")
  })

  it("goes back to Sources after the connector is deleted", async () => {
    const { count } = stubFetch()
    render(<ConnectorDetailPage connectorId="conn-a" />)
    fireEvent.click(await screen.findByRole("button", { name: "Delete" }))
    const dialog = await screen.findByRole("dialog")
    expect(within(dialog).getByText("Delete db demo?")).toBeDefined()
    expect(pushed).toEqual([])

    fireEvent.click(within(dialog).getByRole("button", { name: "Delete" }))
    await waitFor(() => expect(pushed).toEqual(["/connectors"]))
    expect(count("DELETE /api/connectors/conn-a")).toBe(1)
  })
})

/**
 * `SRC-4` slice 4 (S4-g): a change-capture connector has a Change capture tab with its state, the lag of each
 * table and the lag limit; any other connector has none. A connection test of such a connector lists what the
 * source must have switched on, the missing settings first (S4-f).
 */
describe("ConnectorDetailPage change capture", () => {
  const CDC_SPEC = { ...SPEC, adapter: "cdc", ingestMode: "cdc", sourceObjects: [{ name: "public.orders", target: "orders" }] }
  const CAPTURE = {
    supported: true,
    state: "running",
    lag: {
      tables: [
        { table: "public.orders", status: "behind", seconds: 125 },
        { table: "public.items", status: "up_to_date" },
      ],
      worst: { table: "public.orders", seconds: 125 },
      limitMinutes: 15,
      alertOpen: false,
    },
  }

  it("has a Change capture tab for a change-capture connector, between Ingest and Schema changes", async () => {
    stubFetch({
      "GET /api/connectors/conn-a/ingest-spec": () => json(CDC_SPEC),
      "GET /api/connectors/conn-a/change-capture": () => json(CAPTURE),
    })
    render(<ConnectorDetailPage connectorId="conn-a" />)
    await settle()
    await waitFor(() => expect(screen.getAllByRole("tab").map((t) => t.textContent ?? "")[2]).toBe("Change capture"))
  })

  it("has no Change capture tab for any other connector, and ?tab=capture lands on the Overview", async () => {
    url.search = "tab=capture"
    stubFetch()
    render(<ConnectorDetailPage connectorId="conn-a" />)
    await settle()
    expect(screen.queryByRole("tab", { name: "Change capture" })).toBeNull()
    await waitFor(() => expect(selected("Overview")).toBe("true"))
  })

  it("opens the tab for ?tab=capture and words each table's lag", async () => {
    url.search = "tab=capture"
    stubFetch({
      "GET /api/connectors/conn-a/ingest-spec": () => json(CDC_SPEC),
      "GET /api/connectors/conn-a/change-capture": () => json(CAPTURE),
    })
    render(<ConnectorDetailPage connectorId="conn-a" />)
    expect(await screen.findByText("Behind by 2m")).toBeDefined()
    expect(screen.getByText("Up to date")).toBeDefined()
    expect(screen.getByText("Running")).toBeDefined()
    expect((screen.getByLabelText("Alert after (minutes)") as HTMLInputElement).value).toBe("15")
  })

  it("lists the setup checks after a test, the missing one first with its command", async () => {
    stubFetch({
      "GET /api/connectors/conn-a/ingest-spec": () => json(CDC_SPEC),
      "POST /api/connectors/conn-a/test": () =>
        json({
          ok: true,
          supported: true,
          latencyMs: 5,
          message: "Connected via PostgreSQL",
          testedAt: "2026-10-30T01:00:00.000Z",
          setupChecks: [
            { setting: "wal_level", status: "ok", message: "wal_level is logical." },
            {
              setting: "publication",
              status: "missing",
              message: "The publication named in the connector does not exist.",
              command: 'CREATE PUBLICATION "orders_pub" FOR TABLE "public"."orders";',
            },
          ],
        }),
    })
    render(<ConnectorDetailPage connectorId="conn-a" />)
    fireEvent.click(await screen.findByRole("button", { name: "Test connection" }))
    const section = await screen.findByRole("region", { name: "Source setup" })
    const settings = within(section).getAllByRole("listitem").map((li) => li.textContent ?? "")
    expect(settings[0]).toContain("publication")
    expect(settings[1]).toContain("wal_level")
    expect(within(section).getByText(/CREATE PUBLICATION/)).toBeDefined()
    expect(within(section).getByText(/1 setting missing/)).toBeDefined()
  })

  it("shows no setup list for a test that sent none", async () => {
    stubFetch()
    render(<ConnectorDetailPage connectorId="conn-a" />)
    fireEvent.click(await screen.findByRole("button", { name: "Test connection" }))
    await screen.findByText("Connection test passed")
    expect(screen.queryByRole("region", { name: "Source setup" })).toBeNull()
  })
})
