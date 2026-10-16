// SEC-23 (F4): the API refuses adding a rule without governance:write, so
// the "Add Quality Rule" button shows only with it, as the row actions beside it do.
// `AuthProvider` and the table's URL state need `next/navigation`; stub it
// before any import resolves (`mock.module` is hoisted).
mock.module("next/navigation", () => ({
  usePathname: () => "/governance/data-quality",
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
import { afterEach, describe, expect, it, mock, spyOn } from "bun:test"
import { TableProviders } from "@/components/app-shell/table-providers"
import { AuthProvider } from "@/features/auth/auth-provider"
import { DataQualityPage } from "./data-quality-page"

afterEach(() => {
  cleanup()
  mock.restore()
})

const json = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } })

/** The `[method, path, body]` of every write the page sent to the stub. */
let writesSeen: unknown[][] = []

/** Answers the session with `permissions`; resolves `sessionRead` once it has. */
function stubApi(
  permissions: string[],
  {
    quality = [],
    incidents = [],
    runs = [],
  }: { quality?: unknown[]; incidents?: unknown[]; runs?: unknown[] } = {}
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
    expect(await screen.findByRole("button", { name: "Add Quality Rule" })).toBeTruthy()
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
    await waitFor(() => expect(screen.queryByLabelText("Why is it resolved?")).toBeNull())
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
