// `OPS-8` part B: the Usage page shows the runs of a period summed per day,
// person or engine. The money column and its "Estimate" header appear only
// once a rate is entered, the add-rate form shows only with settings:write,
// and the tenant switch only with *:*, as the API demands each of them.
// `OPS-8` part C: the storage of every layer shows only to the platform
// operator; anybody else sees the page and a line saying who sees it.
import { mock } from "bun:test"

mock.module("next/navigation", () => ({
  usePathname: () => "/usage",
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

import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, setSystemTime, spyOn } from "bun:test"
import { chooseOption } from "@/components/patterns/select-field.testing"
import { AuthProvider } from "@/features/auth/auth-provider"
import { UsagePage } from "./usage-page"

beforeEach(() => {
  // 30 days ending today: 2026-10-18 to 2026-11-16, both included.
  setSystemTime(new Date("2026-11-16T10:00:00Z"))
})

afterEach(() => {
  cleanup()
  mock.restore()
  setSystemTime()
})

const json = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } })

const ROW = {
  key: "u1",
  label: "Rina",
  queries: 12,
  completed: 9,
  failed: 2,
  blocked: 1,
  cancelled: 0,
  scannedBytes: 2048,
  engineMs: 90000,
  estimate: null as string | null,
}

const TOTALS = { ...ROW, key: "", label: "" }

function report(overrides: Record<string, unknown> = {}) {
  return {
    from: "2026-10-18",
    to: "2026-11-16",
    group: "day",
    scope: "tenant",
    currency: null,
    rows: [ROW],
    totals: TOTALS,
    ...overrides,
  }
}

const RATE = {
  id: "r1",
  kind: "scan_tb",
  currency: "USD",
  amount: "5.000000",
  effectiveFrom: "2026-10-01",
  createdAt: "2026-10-01T08:00:00Z",
}

/** One measured layer, as the storage job wrote it. */
const STORAGE_ROW = {
  day: "2026-11-16",
  layer: "bronze",
  tableCount: 12,
  totalBytes: 1_500_000_000,
  tablesSkipped: 0,
  skippedReason: null as string | null,
  estimate: "0.322581" as string | null,
}

function storageReport(overrides: Record<string, unknown> = {}) {
  return {
    from: "2026-10-18",
    to: "2026-11-16",
    currency: "USD",
    rows: [STORAGE_ROW],
    ...overrides,
  }
}

type Stub = {
  /** What the sums answer; a function can answer by request. */
  usage?: (url: URL) => Response
  /** What the storage read answers; a function can answer by request. */
  storage?: (url: URL) => Response
  rates?: { currency: string | null; rates: (typeof RATE)[] }
  /** The URLs of every call, in order, with their method. */
  calls: { url: URL; method: string; body: string | null }[]
  sessionRead: Promise<void>
}

/** Answers the session with `permissions`, then the usage routes. */
function stubApi(
  permissions: string[],
  options: Partial<Pick<Stub, "usage" | "rates" | "storage">> = {}
): Stub {
  let sessionDone: () => void = () => {}
  const sessionRead = new Promise<void>((resolve) => {
    sessionDone = resolve
  })
  const stub: Stub = { calls: [], sessionRead, ...options }
  spyOn(globalThis, "fetch").mockImplementation((async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = new URL(String(input), "http://console.test")
    if (url.pathname.includes("/api/auth/me")) {
      const answer = json({ id: "u1", name: "Reader", email: null, roles: ["Analyst"], permissions, tenants: [] })
      sessionDone()
      return answer
    }
    stub.calls.push({
      url,
      method: init?.method ?? "GET",
      body: typeof init?.body === "string" ? init.body : null,
    })
    if (url.pathname === "/api/usage/queries/export") {
      return new Response("day,queries\n", {
        status: 200,
        headers: { "content-disposition": 'attachment; filename="usage-2026-10-18-2026-11-16.csv"' },
      })
    }
    if (url.pathname === "/api/usage/queries") return stub.usage ? stub.usage(url) : json(report())
    if (url.pathname === "/api/usage/storage") {
      return stub.storage ? stub.storage(url) : json(storageReport({ currency: null, rows: [] }))
    }
    if (url.pathname === "/api/usage/rates") {
      if (init?.method === "POST") return json(RATE, 201)
      return json(stub.rates ?? { currency: null, rates: [] })
    }
    return json({ error: "not stubbed" }, 404)
  }) as unknown as typeof fetch)
  return stub
}

function renderPage() {
  return render(
    <AuthProvider>
      <UsagePage />
    </AuthProvider>
  )
}

async function sessionSettled(stub: Stub) {
  await stub.sessionRead
  await new Promise((resolve) => setTimeout(resolve, 50))
}

const usageCalls = (stub: Stub) => stub.calls.filter((c) => c.url.pathname === "/api/usage/queries")

describe("UsagePage", () => {
  it("asks for the last 30 days by day, ending today in UTC", async () => {
    const stub = stubApi(["usage:read"])
    renderPage()
    await screen.findByText("Rina")
    const params = Object.fromEntries(usageCalls(stub)[0].url.searchParams)
    expect(params).toEqual({ from: "2026-10-18", to: "2026-11-16", group: "day" })
  })

  it("shows the totals and the row's label, counts and wall-clock time, with no money while no rate is entered", async () => {
    stubApi(["usage:read"])
    renderPage()
    expect(await screen.findByText("Rina")).toBeTruthy()
    expect(screen.getByRole("columnheader", { name: "Engine time (wall clock)" })).toBeTruthy()
    // 90,000 ms is 1m 30s, in the table and in the totals.
    expect(screen.getAllByText("1m 30s").length).toBeGreaterThan(0)
    // The money column is absent.
    expect(screen.queryByRole("columnheader", { name: /estimate/i })).toBeNull()
  })

  it("shows scanned data in decimal units, the unit the scan rate is priced in", async () => {
    // The rate is per 10^12 bytes, so 1.5e12 bytes must read as 1.5 TB (not 1.4 TB as with 1024).
    stubApi(["usage:read"], {
      usage: () =>
        json(
          report({
            rows: [{ ...ROW, scannedBytes: 1_500_000_000_000 }, { ...ROW, key: "u2", label: "Budi", scannedBytes: 2000 }],
            totals: { ...TOTALS, scannedBytes: 1_500_000_002_000 },
          })
        ),
    })
    renderPage()
    await screen.findByText("Rina")
    expect(screen.getAllByText("1.5 TB").length).toBeGreaterThan(0)
    expect(screen.getAllByText("2 KB").length).toBeGreaterThan(0)
    expect(screen.getByText(/1 TB is 10\^12 bytes/)).toBeTruthy()
  })

  it("shows an Estimate column with the currency once a rate is entered", async () => {
    stubApi(["usage:read"], {
      usage: () => json(report({ currency: "USD", rows: [{ ...ROW, estimate: "12.500000" }], totals: { ...TOTALS, estimate: "12.500000" } })),
      rates: { currency: "USD", rates: [RATE] },
    })
    renderPage()
    expect(await screen.findByRole("columnheader", { name: "Estimate (USD)" })).toBeTruthy()
    expect(screen.getAllByText("12.50").length).toBeGreaterThan(0)
  })

  it("shows a rate to all its significant decimals, and an amount of 16 digits with every digit", async () => {
    // PR review fix (SHOULD-FIX): both went through `Number`, which showed a rate of
    // 0.000012 as 0.00 and rounded 1234567890123456.5 to a neighbouring double.
    stubApi(["usage:read"], {
      usage: () =>
        json(
          report({
            currency: "USD",
            rows: [
              { ...ROW, estimate: "1234567890123456.500000" },
              // An amount of 16 digits with six places tells the two paths
              // apart: from its digits the page shows 1,234,567,890,123,456.789
              // (the digits it was given, rounded at the fourth), while a
              // float cannot hold the low digits (the nearest double is
              // 1234567890123456.75) and shows 1,234,567,890,123,456.80.
              { ...ROW, key: "u2", label: "Budi", estimate: "1234567890123456.789012" },
            ],
            // The same amount in the totals, so the card and the row are
            // both formatted from the digits.
            totals: { ...TOTALS, estimate: "1234567890123456.789012" },
          })
        ),
      rates: {
        currency: "USD",
        rates: [
          { ...RATE, id: "r2", amount: "0.000012" },
          { ...RATE, id: "r3", amount: "5.000000", effectiveFrom: "2026-09-01" },
        ],
      },
    })
    renderPage()
    expect(await screen.findByText("0.000012 USD")).toBeTruthy()
    expect(screen.getByText("5.00 USD")).toBeTruthy()
    expect(screen.getAllByText("1,234,567,890,123,456.50").length).toBeGreaterThan(0)
    expect(screen.getAllByText("1,234,567,890,123,456.789").length).toBeGreaterThan(0)
  })

  it("rounds an estimate to four decimals from its digits, not from a float", async () => {
    stubApi(["usage:read"], {
      usage: () =>
        json(
          report({
            currency: "USD",
            rows: [{ ...ROW, estimate: "0.123450" }],
            totals: { ...TOTALS, estimate: "999.999950" },
          })
        ),
    })
    renderPage()
    // 0.123450 has four significant places after trimming; 999.99995 rounds up through the carry.
    expect((await screen.findAllByText("0.1235")).length).toBeGreaterThan(0)
    expect(screen.getAllByText("1,000.00").length).toBeGreaterThan(0)
  })

  it("says a row has no rate, and not zero, when a rate exists but none covers its runs", async () => {
    stubApi(["usage:read"], { usage: () => json(report({ currency: "USD" })) })
    renderPage()
    await screen.findByRole("columnheader", { name: "Estimate (USD)" })
    expect(screen.getAllByText("No rate").length).toBeGreaterThan(0)
  })

  it("says so when the period has no recorded queries", async () => {
    stubApi(["usage:read"], { usage: () => json(report({ rows: [] })) })
    renderPage()
    expect(await screen.findByText("No queries in this period")).toBeTruthy()
  })

  it("shows the permission state when the API refuses the read", async () => {
    stubApi(["usage:read"], { usage: () => json({ error: "permission_denied: usage:read" }, 403) })
    renderPage()
    expect(await screen.findByText(/You don't have access/)).toBeTruthy()
  })

  it("asks again for the group a person picks", async () => {
    const stub = stubApi(["usage:read"])
    renderPage()
    await screen.findByText("Rina")
    fireEvent.click(screen.getByRole("button", { name: "Person" }))
    await waitFor(() => expect(usageCalls(stub).at(-1)?.url.searchParams.get("group")).toBe("person"))
    expect(await screen.findByRole("columnheader", { name: "Person" })).toBeTruthy()
  })

  it("offers the tenant group only to a caller holding *:*, and then asks for every tenant", async () => {
    const reader = stubApi(["usage:read"])
    renderPage()
    await screen.findByText("Rina")
    await sessionSettled(reader)
    expect(screen.queryByRole("button", { name: "Tenant" })).toBeNull()
    cleanup()
    mock.restore()

    const admin = stubApi(["*:*"])
    renderPage()
    fireEvent.click(await screen.findByRole("button", { name: "Tenant" }))
    await waitFor(() => {
      const last = usageCalls(admin).at(-1)?.url.searchParams
      expect(last?.get("group")).toBe("tenant")
      expect(last?.get("tenant")).toBe("all")
    })
  })

  it("does not ask when the period is longer than 366 days, and says why", async () => {
    const stub = stubApi(["usage:read"])
    renderPage()
    await screen.findByText("Rina")
    const before = usageCalls(stub).length
    fireEvent.change(screen.getByLabelText("From"), { target: { value: "2025-01-01" } })
    expect(await screen.findByText("A period is at most 366 days.")).toBeTruthy()
    expect(usageCalls(stub).length).toBe(before)
  })

  it("downloads the CSV for the period on screen", async () => {
    const stub = stubApi(["usage:read"])
    spyOn(URL, "createObjectURL").mockReturnValue("blob:usage")
    spyOn(URL, "revokeObjectURL").mockReturnValue(undefined)
    renderPage()
    await screen.findByText("Rina")
    fireEvent.click(screen.getByRole("button", { name: "Download CSV" }))
    await waitFor(() => expect(stub.calls.some((c) => c.url.pathname === "/api/usage/queries/export")).toBe(true))
    const params = Object.fromEntries(stub.calls.find((c) => c.url.pathname === "/api/usage/queries/export")!.url.searchParams)
    expect(params).toEqual({ from: "2026-10-18", to: "2026-11-16", group: "day" })
  })

  it("lists the rates with their start dates and hides the add form without settings:write", async () => {
    const stub = stubApi(["usage:read"], { rates: { currency: "USD", rates: [RATE] } })
    renderPage()
    expect(await screen.findByText("2026-10-01")).toBeTruthy()
    await sessionSettled(stub)
    expect(screen.queryByRole("button", { name: "Add rate" })).toBeNull()
  })

  it("shows the add-rate form with settings:write, and posts the rate it is given", async () => {
    const stub = stubApi(["usage:read", "settings:write"])
    renderPage()
    await screen.findByText("Rina")
    fireEvent.change(await screen.findByLabelText("Amount"), { target: { value: "5" } })
    fireEvent.change(screen.getByLabelText("Currency"), { target: { value: "usd" } })
    chooseOption(screen.getByRole("combobox", { name: "Rate" }), /Engine time/)
    fireEvent.change(screen.getByLabelText("Starts on"), { target: { value: "2026-11-01" } })
    fireEvent.click(screen.getByRole("button", { name: "Add rate" }))
    await waitFor(() => expect(stub.calls.some((c) => c.method === "POST")).toBe(true))
    const post = stub.calls.find((c) => c.method === "POST")!
    expect(post.url.pathname).toBe("/api/usage/rates")
    expect(JSON.parse(post.body ?? "{}")).toEqual({
      kind: "engine_hour",
      currency: "USD",
      amount: "5",
      effectiveFrom: "2026-11-01",
    })
  })

  it("shows the newest measurement of every layer to the platform operator", async () => {
    const stub = stubApi(["*:*"], {
      storage: () =>
        json(
          storageReport({
            rows: [
              { ...STORAGE_ROW, layer: "bronze", day: "2026-11-15", totalBytes: 100 },
              { ...STORAGE_ROW, layer: "bronze", day: "2026-11-16", totalBytes: 1_500_000_000 },
              { ...STORAGE_ROW, layer: "silver", day: "2026-11-16", totalBytes: 2_000_000_000 },
            ],
          })
        ),
    })
    renderPage()
    expect(await screen.findByRole("columnheader", { name: "Layer" })).toBeTruthy()
    // The newest measurement of each layer, not the older one beside it.
    expect(screen.getAllByText("1.5 GB").length).toBeGreaterThan(0)
    expect(screen.getAllByText("2 GB").length).toBeGreaterThan(0)
    expect(screen.queryByText("100 B")).toBeNull()
    const call = stub.calls.find((c) => c.url.pathname === "/api/usage/storage")
    expect(Object.fromEntries(call!.url.searchParams)).toEqual({
      from: "2026-10-18",
      to: "2026-11-16",
    })
  })

  it("shows a tenant administrator the page without the storage, and says who sees it", async () => {
    const stub = stubApi(["usage:read"])
    renderPage()
    await screen.findByText("Rina")
    await sessionSettled(stub)
    expect(screen.getByText("Storage is shown to the platform operator only.")).toBeTruthy()
    expect(screen.queryByRole("columnheader", { name: "Layer" })).toBeNull()
    // The page does not ask for the read the API would refuse.
    expect(stub.calls.some((c) => c.url.pathname === "/api/usage/storage")).toBe(false)
  })

  it("says how many tables of a layer could not be measured, and never shows it as a layer of zero bytes", async () => {
    stubApi(["*:*"], {
      storage: () =>
        json(
          storageReport({
            rows: [
              {
                ...STORAGE_ROW,
                layer: "gold",
                totalBytes: 0,
                tablesSkipped: 2,
                skippedReason: "the catalog's metadata could not be read",
              },
            ],
          })
        ),
    })
    renderPage()
    expect(await screen.findByText("2 not measured")).toBeTruthy()
    expect(screen.getByText(/2 of its tables could not be measured/)).toBeTruthy()
    expect(screen.getByText(/the catalog's metadata could not be read/)).toBeTruthy()
  })
})
