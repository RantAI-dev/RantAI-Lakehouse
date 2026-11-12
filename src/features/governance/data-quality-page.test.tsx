// SEC-23 (F4): the API refuses adding a rule without governance:write, so
// the "Add Quality Rule" button shows only with it, as the row actions beside it do.
// `AuthProvider` and the table's URL state need `next/navigation`; stub it
// before any import resolves (`mock.module` is hoisted).
// WEB-3 S3-D5: `search` is the address's query, so a test can open a tab.
let search = new URLSearchParams()
mock.module("next/navigation", () => ({
  usePathname: () => "/governance/data-quality",
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

import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react"
import { afterEach, describe, expect, it, mock, spyOn } from "bun:test"
import { TableProviders } from "@/components/app-shell/table-providers"
import { chooseOption } from "@/components/patterns/select-field.testing"
import { AuthProvider } from "@/features/auth/auth-provider"
import { DataQualityPage } from "./data-quality-page"

afterEach(() => {
  cleanup()
  mock.restore()
  search = new URLSearchParams()
  resetAnswer = () => new Response(null, { status: 204 })
})

const json = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } })

/** The `[method, path, body]` of every write the page sent to the stub. */
let writesSeen: unknown[][] = []
/** How the stub answers a schedule reset; a test may swap it for a failure. */
let resetAnswer: () => Response = () => new Response(null, { status: 204 })

/** Answers the session with `permissions`; resolves `sessionRead` once it has. */
function stubApi(
  permissions: string[],
  {
    quality = [],
    incidents = [],
    runs = [],
    schedules = [],
    targets = [],
    catalog,
  }: {
    quality?: unknown[]
    incidents?: unknown[]
    runs?: unknown[]
    schedules?: unknown[]
    targets?: unknown[]
    /** WEB-8 D8: the catalog's tables, for the freshness dialog's picker. */
    catalog?: unknown[]
  } = {}
) {
  writesSeen = []
  let sessionRead: () => void = () => {}
  const done = new Promise<void>((resolve) => {
    sessionRead = resolve
  })
  spyOn(globalThis, "fetch").mockImplementation((async (input: RequestInfo | URL, init?: RequestInit) => {
    const path = String(input)
    if (path.includes("/api/auth/me")) {
      const answer = json({ id: "u1", name: "Reader", email: null, roles: ["Analyst"], permissions, tenants: [] })
      sessionRead()
      return answer
    }
    // DATA-14 (F7): the incidents list leads the page and a rule's drawer
    // lists its runs; the stub used to answer every quality path with the
    // rule list.
    if (path.includes("/api/governance/quality/incidents")) {
      if ((init?.method ?? "GET") === "POST") {
        writesSeen.push(["POST", path, JSON.parse(String(init?.body))])
        return json({ ...(incidents[0] as object), status: "resolved", resolution: JSON.parse(String(init?.body)).resolution })
      }
      return json({ incidents })
    }
    if (path.includes("/api/governance/quality/") && path.endsWith("/runs")) return json({ runs })
    // WEB-8 D8: the catalog the freshness dialog's table picker reads. When
    // the test gives none the read fails, which is the free-text fallback.
    if (catalog !== undefined && path.startsWith("/api/catalog")) {
      return json({ assets: catalog, namespaces: [] })
    }
    // WEB-3 S3-D5: the Schedules and Freshness targets tabs.
    if (path.includes("/api/governance/quality/schedules/")) {
      // S3a-D3: Reset is a DELETE with no body, answered 204.
      if (init?.method === "DELETE") {
        writesSeen.push(["DELETE", path])
        return resetAnswer()
      }
      const body = JSON.parse(String(init?.body))
      writesSeen.push([init?.method, path, body])
      return json({ asset: "silver.orders", updatedBy: "u1", updatedAt: "2026-11-04T00:00:00.000Z", ...body })
    }
    if (path.includes("/api/governance/quality/schedules")) return json({ schedules, defaultIntervalMinutes: 60 })
    if (path.includes("/api/governance/sla")) {
      const method = init?.method ?? "GET"
      if (method === "GET") return json(targets)
      writesSeen.push([method, path, init?.body ? JSON.parse(String(init.body)) : null])
      return method === "DELETE" ? new Response(null, { status: 204 }) : json(JSON.parse(String(init?.body)))
    }
    if (path.includes("/api/governance/quality")) return json({ quality })
    return json({ error: "not stubbed" }, 404)
  }) as unknown as typeof fetch)
  return done
}

function renderPage() {
  return render(
    <AuthProvider>
      <TableProviders>
        <DataQualityPage />
      </TableProviders>
    </AuthProvider>
  )
}

/** Lets the session answer and React apply it, so "no button" is not just "not loaded yet". */
async function sessionSettled(done: Promise<void>) {
  await done
  await new Promise((resolve) => setTimeout(resolve, 50))
}

describe("DataQualityPage: adding a rule", () => {
  it("shows the button with governance:write", async () => {
    stubApi(["governance:write"])
    renderPage()
    // WEB-7 T3: the form is its own page.
    expect(
      (await screen.findByRole("button", { name: "Add Quality Rule" })).getAttribute("href")
    ).toBe("/governance/data-quality/rules/new")
  })

  it("hides the button without governance:write", async () => {
    const done = stubApi(["catalog:read", "policy:read", "query:read"])
    renderPage()
    await sessionSettled(done)
    expect(screen.getByText("Data Quality")).toBeTruthy()
    expect(screen.queryByRole("button", { name: "Add Quality Rule" })).toBeNull()
  })
})

const incident = (over: Record<string, unknown>) => ({
  id: "i1",
  asset: "silver.orders",
  ruleId: "r1",
  ruleName: "email_filled",
  kind: "check",
  openedAt: "2026-10-15T09:00:00.000Z",
  lastSeenAt: "2026-10-16T09:00:00.000Z",
  detail: "email_filled — 75.0% not null (threshold: email not null >= 90%)",
  status: "open",
  resolvedAt: null,
  resolvedBy: null,
  resolution: null,
  ...over,
})

describe("DataQualityPage: incidents (DATA-14)", () => {
  it("leads with the open incidents, grouped by table, the oldest table first", async () => {
    stubApi(["governance:write"], {
      incidents: [
        incident({ id: "i1", asset: "silver.orders", openedAt: "2026-10-15T09:00:00.000Z" }),
        incident({ id: "i2", asset: "serving.mart", ruleId: null, ruleName: null, kind: "volume_anomaly", detail: "The latest load added 100 rows, 10% of the usual", openedAt: "2026-10-10T09:00:00.000Z" }),
        incident({ id: "i3", asset: "silver.orders", ruleId: "r2", ruleName: "id_unique", openedAt: "2026-10-14T09:00:00.000Z", detail: "id_unique — 3 repeated values" }),
      ],
    })
    renderPage()

    expect(await screen.findByText("3 open across 2 tables.")).toBeTruthy()
    const tables = screen.getAllByRole("link").map((l) => l.textContent).filter((t) => t === "serving.mart" || t === "silver.orders")
    // serving.mart has the oldest incident, so it is read first.
    expect(tables).toEqual(["serving.mart", "silver.orders"])
    expect(screen.getByText("Volume anomaly")).toBeTruthy()
    expect(screen.getByText(/The latest load added 100 rows, 10% of the usual/)).toBeTruthy()
    expect(screen.getByText(/2 open · oldest opened/)).toBeTruthy()
  })

  it("says nothing is wrong when no incident is open", async () => {
    stubApi(["governance:write"])
    renderPage()
    expect(await screen.findByText("Nothing is wrong right now.")).toBeTruthy()
    expect(screen.getByText("No open incidents")).toBeTruthy()
  })

  it("resolves an incident only with a reason", async () => {
    stubApi(["governance:write"], { incidents: [incident({})] })
    renderPage()

    fireEvent.click(await screen.findByRole("button", { name: "Resolve" }))
    const confirm = screen.getAllByRole("button", { name: "Resolve" }).at(-1) as HTMLButtonElement
    expect(confirm.disabled).toBe(true)
    fireEvent.change(screen.getByLabelText("Why is it resolved?"), { target: { value: "Known outage" } })
    fireEvent.click(confirm)
    // Poll on a timer, not with `waitFor`: started while the resolve is in
    // flight, its DOM observer made Bun's happy-dom grow to GBs and crash the
    // whole run in CI (run 38881397576). The page sends no extra request: the
    // stub saw one POST and one reload.
    for (let i = 0; i < 100 && screen.queryByLabelText("Why is it resolved?"); i++) {
      await new Promise((resolve) => setTimeout(resolve, 20))
    }
    expect(screen.queryByLabelText("Why is it resolved?")).toBeNull()
    expect(writesSeen).toEqual([
      ["POST", "/api/governance/quality/incidents/i1/resolve", { resolution: "Known outage" }],
    ])
  })

  it("offers no Resolve without governance:write", async () => {
    stubApi(["catalog:read", "query:read"], { incidents: [incident({})] })
    renderPage()
    await screen.findByText(/75.0% not null/)
    expect(screen.queryByRole("button", { name: "Resolve" })).toBeNull()
  })

  it("shows the resolved ones as history, with who resolved them and why", async () => {
    stubApi(["governance:write"], {
      incidents: [
        incident({ status: "resolved", resolvedAt: "2026-10-16T08:00:00.000Z", resolvedBy: "Rina", resolution: "Source corrected" }),
      ],
    })
    renderPage()
    fireEvent.click(await screen.findByRole("button", { name: "Resolved" }))
    expect(await screen.findByText(/by Rina: Source corrected/)).toBeTruthy()
    expect(screen.queryByRole("button", { name: "Resolve" })).toBeNull()
  })
})

describe("DataQualityPage: a rule's run history (DATA-14)", () => {
  const rule = {
    id: "r1",
    name: "email_filled",
    asset: "silver.orders",
    dimension: "completeness",
    threshold: "email not null >= 90%",
    severity: "high",
    lastStatus: "failed",
    lastRunAt: "2026-10-16T09:00:00.000Z",
    lastValue: "75.0% not null",
    evaluable: true,
    hint: null,
  }

  it("lists the runs with the threshold each ran with, marking an earlier one", async () => {
    stubApi(["governance:write"], {
      quality: [rule],
      runs: [
        { status: "failed", value: "75.0% not null", runBy: "schedule", at: "2026-10-16T09:00:00Z", threshold: "email not null >= 90%" },
        { status: "passed", value: "75.0% not null", runBy: "Ayu", at: "2026-10-15T09:00:00Z", threshold: "email not null >= 70%" },
        { status: "passed", value: "80.0% not null", runBy: "load", at: "2026-10-01T09:00:00Z", threshold: null },
      ],
    })
    renderPage()
    fireEvent.click(await screen.findByText("email_filled"))

    const history = (await screen.findByText("Run history")).closest("section") as HTMLElement
    await within(history).findByText("email not null >= 70%")
    expect(within(history).getByText("email not null >= 90%")).toBeTruthy()
    // Only the older threshold is marked.
    expect(within(history).getAllByText("(earlier)")).toHaveLength(1)
    expect(within(history).getByText("not recorded")).toBeTruthy()
    expect(within(history).getByText("Ayu")).toBeTruthy()
  })

  it("says so when the history cannot be loaded", async () => {
    spyOn(globalThis, "fetch").mockImplementation((async (input: RequestInfo | URL) => {
      const path = String(input)
      if (path.includes("/api/auth/me")) {
        return json({ id: "u1", name: "Reader", email: null, roles: [], permissions: ["governance:write"], tenants: [] })
      }
      if (path.endsWith("/runs")) return json({ error: "no" }, 503)
      if (path.includes("/quality/incidents")) return json({ incidents: [] })
      if (path.includes("/api/governance/quality")) return json({ quality: [rule] })
      return json({ error: "not stubbed" }, 404)
    }) as unknown as typeof fetch)
    renderPage()
    fireEvent.click(await screen.findByText("email_filled"))
    expect(await screen.findByText(/The run history could not be loaded/)).toBeTruthy()
  })
})

// QA LH-DATA-14-04 (GOV-10, DEC-17): running a rule by hand is a governance
// action — the API takes `governance:write` for it, so the drawer's button
// follows.
describe("DataQualityPage: running a rule by hand (GOV-10)", () => {
  const rule = {
    id: "r1",
    name: "email_filled",
    asset: "silver.orders",
    dimension: "completeness",
    threshold: "email not null >= 90%",
    severity: "high",
    lastStatus: "failed",
    lastRunAt: "2026-10-16T09:00:00.000Z",
    lastValue: "75.0% not null",
    evaluable: true,
    hint: null,
  }

  it("hides Run check for a person who may only read governance", async () => {
    const done = stubApi(["governance:read", "query:read"], { quality: [rule] })
    renderPage()
    fireEvent.click(await screen.findByText("email_filled"))
    await sessionSettled(done)
    expect(screen.queryByRole("button", { name: "Run check" })).toBeNull()
  })

  it("shows Run check with governance:write", async () => {
    stubApi(["governance:write"], { quality: [rule] })
    renderPage()
    fireEvent.click(await screen.findByText("email_filled"))
    expect(await screen.findByRole("button", { name: "Run check" })).toBeTruthy()
  })
})

const schedule = (over: Record<string, unknown>) => ({
  asset: "silver.orders",
  mode: "interval",
  intervalMinutes: 180,
  updatedBy: "rina",
  updatedAt: "2026-11-01T00:00:00.000Z",
  ...over,
})

describe("DataQualityPage: Schedules tab (WEB-3 S3-D5)", () => {
  it("lists every saved schedule, links a Silver table and leaves a Bronze key as plain text", async () => {
    search = new URLSearchParams("tab=schedules")
    stubApi(["governance:write"], {
      schedules: [schedule({}), schedule({ asset: "bronze.raw_orders", mode: "on_load", intervalMinutes: 1440 })],
    })
    renderPage()
    expect(await screen.findByText("Every 3 hours.")).toBeTruthy()
    expect(screen.getByText("After every load of the table, and every 1 day as a backstop.")).toBeTruthy()
    expect(screen.getByText(/A table not listed here runs its checks every 1 hour/)).toBeTruthy()
    expect(screen.getByRole("link", { name: "silver.orders" }).getAttribute("href")).toBe("/data/assets/silver.orders?tab=quality")
    expect(screen.queryByRole("link", { name: "bronze.raw_orders" })).toBeNull()
    expect(screen.getByText("bronze.raw_orders")).toBeTruthy()
    // Adding a rule belongs to the rules tab.
    expect(screen.queryByRole("button", { name: "Add Quality Rule" })).toBeNull()
  })

  it("changes a schedule with the asset page's control", async () => {
    search = new URLSearchParams("tab=schedules")
    stubApi(["governance:write"], { schedules: [schedule({})] })
    renderPage()
    fireEvent.click(await screen.findByRole("button", { name: "Edit the schedule of silver.orders" }))
    chooseOption(screen.getByRole("combobox", { name: "Run the checks" }), "On every load")
    fireEvent.click(screen.getByRole("button", { name: "Save schedule" }))
    await waitFor(() => expect(writesSeen.length).toBe(1))
    expect(writesSeen[0]).toEqual([
      "PUT",
      "/api/governance/quality/schedules/silver.orders",
      { mode: "on_load", intervalMinutes: 180 },
    ])
  })

  it("offers no Edit or Reset without governance:write", async () => {
    search = new URLSearchParams("tab=schedules")
    const done = stubApi(["catalog:read"], { schedules: [schedule({})] })
    renderPage()
    await sessionSettled(done)
    expect(await screen.findByText("Every 3 hours.")).toBeTruthy()
    expect(screen.queryByRole("button", { name: /Edit the schedule/ })).toBeNull()
    expect(screen.queryByRole("button", { name: /Reset the schedule/ })).toBeNull()
  })

  it("resets a schedule to the default after confirming (S3a-D3)", async () => {
    search = new URLSearchParams("tab=schedules")
    stubApi(["governance:write"], { schedules: [schedule({})] })
    renderPage()
    fireEvent.click(await screen.findByRole("button", { name: "Reset the schedule of silver.orders" }))
    const dialog = await screen.findByRole("dialog")
    expect(within(dialog).getByText(/Its checks then run every 1 hour/)).toBeTruthy()
    expect(writesSeen).toEqual([])
    fireEvent.click(within(dialog).getByRole("button", { name: "Reset" }))
    await waitFor(() => expect(writesSeen).toEqual([["DELETE", "/api/governance/quality/schedules/silver.orders"]]))
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull())
  })

  it("keeps the dialog open with the API's sentence when a reset fails", async () => {
    search = new URLSearchParams("tab=schedules")
    resetAnswer = () => json({ error: "You need governance:write to change a schedule." }, 403)
    stubApi(["governance:write"], { schedules: [schedule({})] })
    renderPage()
    fireEvent.click(await screen.findByRole("button", { name: "Reset the schedule of silver.orders" }))
    const dialog = await screen.findByRole("dialog")
    fireEvent.click(within(dialog).getByRole("button", { name: "Reset" }))
    expect(await within(dialog).findByRole("alert")).toBeTruthy()
    expect(within(dialog).getByText("You need governance:write to change a schedule.")).toBeTruthy()
  })
})

describe("DataQualityPage: Freshness targets tab (WEB-3 S3-D5)", () => {
  const target = { tableName: "silver.orders", expectedIntervalMinutes: 1440 }

  it("lists every target and removes one after confirming", async () => {
    search = new URLSearchParams("tab=freshness")
    stubApi(["governance:write"], { targets: [target, { tableName: "bronze.raw_orders", expectedIntervalMinutes: 90, owner: "ops" }] })
    renderPage()
    expect(await screen.findByText("1 day")).toBeTruthy()
    expect(screen.getByText("90 minutes")).toBeTruthy()
    expect(screen.getByRole("link", { name: "silver.orders" }).getAttribute("href")).toBe("/data/assets/silver.orders?tab=activity")
    expect(screen.queryByRole("link", { name: "bronze.raw_orders" })).toBeNull()
    fireEvent.click(screen.getByRole("button", { name: "Remove the target of silver.orders" }))
    fireEvent.click(await screen.findByRole("button", { name: "Remove" }))
    await waitFor(() => expect(writesSeen.length).toBe(1))
    expect(writesSeen[0]).toEqual(["DELETE", "/api/governance/sla/silver.orders", null])
  })

  it("adds a target for a table it is told", async () => {
    search = new URLSearchParams("tab=freshness")
    stubApi(["governance:write"])
    renderPage()
    expect(await screen.findByText("No freshness target")).toBeTruthy()
    fireEvent.click(screen.getByRole("button", { name: "Add target" }))
    const save = await screen.findByRole("button", { name: "Save target" })
    expect((save as HTMLButtonElement).disabled).toBe(true)
    fireEvent.change(screen.getByLabelText("Table"), { target: { value: "silver.customers" } })
    fireEvent.change(screen.getByLabelText("New data at least every"), { target: { value: "6" } })
    chooseOption(screen.getByRole("combobox", { name: "Unit" }), "hours")
    fireEvent.click(save)
    await waitFor(() => expect(writesSeen.length).toBe(1))
    expect(writesSeen[0]).toEqual(["PUT", "/api/governance/sla", { tableName: "silver.customers", expectedIntervalMinutes: 360 }])
  })

  it("changes a target, starting from the one in force", async () => {
    search = new URLSearchParams("tab=freshness")
    stubApi(["governance:write"], { targets: [target] })
    renderPage()
    fireEvent.click(await screen.findByRole("button", { name: "Edit the target of silver.orders" }))
    expect((screen.getByLabelText("New data at least every") as HTMLInputElement).value).toBe("1")
    // WEB-9 D2: the trigger shows the chosen option's label.
    expect(
      screen.getByRole("combobox", { name: "Unit" }).querySelector('[data-slot="select-value"]')
        ?.textContent
    ).toBe("days")
    expect(screen.getByRole("button", { name: "Remove target" })).toBeTruthy()
  })

  it("picks the table to add a target for from the catalog (WEB-8 D8)", async () => {
    search = new URLSearchParams("tab=freshness")
    stubApi(["governance:write"], {
      catalog: [
        // Shaped like the API's list rows (`DATA-54`): a Silver row's `id`
        // is its table key; a Bronze row's `id` is a registry slug and its
        // table is named by `tableKey`.
        { id: "silver.orders", name: "Orders", namespace: "silver" },
        {
          id: "raw-orders",
          name: "Raw Orders",
          namespace: "primer",
          tableKey: "bronze.raw_orders",
        },
      ],
    })
    renderPage()
    fireEvent.click(await screen.findByRole("button", { name: "Add target" }))
    // The table is a combobox over the catalog's tables, shown as
    // namespace.table — the key the SLA stores.
    fireEvent.click(await screen.findByRole("button", { name: "Table" }))
    const list = await screen.findByRole("listbox")
    expect(within(list).getAllByRole("option").map((o) => o.textContent)).toEqual([
      "bronze.raw_orders",
      "silver.orders",
    ])
    fireEvent.click(within(list).getByText("silver.orders"))
    fireEvent.change(screen.getByLabelText("New data at least every"), { target: { value: "6" } })
    chooseOption(screen.getByRole("combobox", { name: "Unit" }), "hours")
    fireEvent.click(screen.getByRole("button", { name: "Save target" }))
    await waitFor(() => expect(writesSeen.length).toBe(1))
    expect(writesSeen[0]).toEqual([
      "PUT",
      "/api/governance/sla",
      { tableName: "silver.orders", expectedIntervalMinutes: 360 },
    ])
  })

  it("offers no Add, Edit or Remove without governance:write", async () => {
    search = new URLSearchParams("tab=freshness")
    const done = stubApi(["catalog:read"], { targets: [target] })
    renderPage()
    await sessionSettled(done)
    expect(await screen.findByText("1 day")).toBeTruthy()
    expect(screen.queryByRole("button", { name: "Add target" })).toBeNull()
    expect(screen.queryByRole("button", { name: /Remove the target/ })).toBeNull()
  })
})
