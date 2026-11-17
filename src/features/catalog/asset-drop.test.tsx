// Drop table from the table's page (`DATA-55`, QA `LH-DATA-15-06`, `TC-1731`):
// the control is offered only for a table the server says may be dropped, the
// confirmation names the table and its counted rows, and a refusal keeps the
// dialog open with the server's own sentence.

const url = { search: "" }
mock.module("next/navigation", () => ({
  usePathname: () => "/data/assets/demo-orders",
  useSearchParams: () => new URLSearchParams(url.search),
  useRouter: () => ({
    push: () => {},
    replace: () => {},
    refresh: () => {},
    back: () => {},
    forward: () => {},
    prefetch: () => {},
  }),
}))

import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react"
import { afterEach, describe, expect, it, mock, spyOn } from "bun:test"
import { toast } from "sonner"
import { AuthProvider } from "@/features/auth/auth-provider"
import type { AssetDetail } from "@/services/contracts/assets"
import { DropTableAction } from "./asset-drop"

afterEach(() => {
  cleanup()
  mock.restore()
})

const ASSET: AssetDetail = {
  id: "demo-orders",
  name: "Demo Orders",
  namespace: "primer",
  type: "iceberg-table",
  layer: "raw",
  tier: "warm",
  classification: "internal",
  owner: "dagster",
  domain: "pariwisata",
  description: "Orders ingested from Postgres.",
  format: "Apache Iceberg (Parquet)",
  engine: "hot-store",
  rows: 2000,
  sizeBytes: 40287,
  columnCount: 2,
  freshnessLagSeconds: 3600,
  lastUpdated: "2026-09-23T04:57:42Z",
  health: "unknown",
  residency: "id-jakarta",
  schema: [{ name: "id", dataType: "Int64" }],
  sample: [],
  qualityChecks: [],
  policySummary: [],
  usage: null,
  recentQueries: [],
  dependents: [],
  changeHistory: [],
  snapshots: [],
  schemaVersions: [],
  upstream: [],
  downstream: [],
  tableName: "demo_orders",
  tableKey: "bronze.demo_orders",
  queryTarget: { engine: "clickhouse", table: "icecat_api.`bronze.demo_orders`" },
}

function json(body: unknown, status = 200) {
  return new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } })
}

const DROPPABLE = { droppable: true, table: "bronze.demo_orders", keptDays: 7 }

/**
 * The stood-in API: the status read without a count, the one with it (which a
 * test may hold open), and the drop itself — a success, or a refusal with the
 * status the API answers.
 */
function stubApi({
  first = DROPPABLE,
  second = { ...DROPPABLE, rowCount: 1234 },
  drop = { ok: true, table: "bronze.demo_orders", rowCount: 1234, droppedAt: "2026-11-13T08:00:00Z", expiresAt: "2026-11-20T08:00:00Z" },
  refusal,
}: {
  first?: unknown
  second?: unknown | Promise<Response>
  drop?: unknown
  /** When set, `POST .../drop` answers this status and sentence. */
  refusal?: { status: number; error: string }
} = {}) {
  const calls: { url: string; method: string }[] = []
  spyOn(globalThis, "fetch").mockImplementation((async (input: RequestInfo | URL, init?: RequestInit) => {
    const path = String(input)
    const method = init?.method ?? "GET"
    calls.push({ url: path, method })
    if (path.includes("/api/auth/me")) {
      return json({ id: "u1", name: "Reader", email: null, roles: ["Analyst"], permissions: ["governance:write"], tenants: [] })
    }
    if (path.endsWith("/api/catalog/demo-orders/drop?withRowCount=true")) {
      return second instanceof Promise ? second : json(second)
    }
    if (path.endsWith("/api/catalog/demo-orders/drop")) {
      if (method === "POST") {
        return refusal ? json({ error: refusal.error }, refusal.status) : json(drop)
      }
      return json(first)
    }
    return json({ error: "not stubbed" }, 404)
  }) as unknown as typeof fetch)
  return { calls }
}

function renderAction(onDropped: () => void = () => {}, asset: AssetDetail = ASSET) {
  return render(
    <AuthProvider>
      <DropTableAction asset={asset} onDropped={onDropped} />
    </AuthProvider>
  )
}

/** The dialog's confirm button, which shares its name with the control that opened it. */
function confirmButton() {
  return within(screen.getByRole("dialog")).getByRole("button", { name: "Drop table" }) as HTMLButtonElement
}

describe("DATA-55: Drop table", () => {
  it("names the table and its counted rows before the drop can be confirmed", async () => {
    let release: (response: Response) => void = () => {}
    const held = new Promise<Response>((resolve) => {
      release = resolve
    })
    stubApi({ second: held })
    renderAction()

    fireEvent.click(await screen.findByRole("button", { name: "Drop table" }))
    expect(await screen.findByText("Drop bronze.demo_orders?")).toBeTruthy()
    expect(await screen.findByText("Counting the rows of bronze.demo_orders…")).toBeTruthy()
    expect(confirmButton().disabled).toBe(true)

    await act(async () => {
      release(json({ ...DROPPABLE, rowCount: 1234 }))
    })
    expect(await screen.findByText("bronze.demo_orders holds 1,234 rows.")).toBeTruthy()
    await waitFor(() => expect(confirmButton().disabled).toBe(false))
  })

  it("says the rows could not be counted, and never shows 0", async () => {
    stubApi({ second: { ...DROPPABLE, rowCount: null } })
    renderAction()

    fireEvent.click(await screen.findByRole("button", { name: "Drop table" }))
    expect(await screen.findByText("The rows of bronze.demo_orders could not be counted.")).toBeTruthy()
    expect(screen.queryByText(/holds 0 rows/)).toBeNull()
    await waitFor(() => expect(confirmButton().disabled).toBe(false))
  })

  it("offers nothing for a table that was not made by an upload", async () => {
    stubApi({ first: { droppable: false, reason: "Only a table made by a file upload can be dropped here." } })
    renderAction()

    await waitFor(() => expect(screen.queryByRole("button", { name: "Drop table" })).toBeNull())
  })

  it("says how many saved queries and dashboards that the reader can open read the table", async () => {
    stubApi()
    renderAction(() => {}, {
      ...ASSET,
      dependents: [
        { id: "q1", name: "Revenue by day", kind: "saved query" },
        { id: "d1", name: "Ops board", kind: "dashboard" },
      ],
    })

    fireEvent.click(await screen.findByRole("button", { name: "Drop table" }))
    expect(
      await screen.findByText(/2 saved queries and dashboards that you can open read it\./)
    ).toBeTruthy()
  })

  it("sends the drop once, says until when the table can be restored, and reloads the table", async () => {
    const { calls } = stubApi()
    const onDropped = mock(() => {})
    const toasts = spyOn(toast, "success").mockImplementation(() => 1)
    renderAction(onDropped)

    fireEvent.click(await screen.findByRole("button", { name: "Drop table" }))
    await waitFor(() => expect(confirmButton().disabled).toBe(false))
    fireEvent.click(confirmButton())

    await waitFor(() => expect(onDropped).toHaveBeenCalledTimes(1))
    expect(calls.filter((c) => c.method === "POST")).toHaveLength(1)
    expect(toasts).toHaveBeenCalled()
    expect(toasts.mock.calls[0][0]).toBe("Table dropped")
    expect(toasts.mock.calls[0][1]?.description).toBe(
      "It can be restored from this page until November 20, 2026."
    )
  })

  it("shows the server's sentence when the drop is refused, and stays open", async () => {
    stubApi({ refusal: { status: 409, error: "This table is already dropped. It can be restored from its page." } })
    const onDropped = mock(() => {})
    renderAction(onDropped)

    fireEvent.click(await screen.findByRole("button", { name: "Drop table" }))
    await waitFor(() => expect(confirmButton().disabled).toBe(false))
    fireEvent.click(confirmButton())

    expect((await screen.findByRole("alert")).textContent).toBe(
      "This table is already dropped. It can be restored from its page."
    )
    expect(screen.getByRole("dialog")).toBeTruthy()
    expect(onDropped).not.toHaveBeenCalled()
  })
})
