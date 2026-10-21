// Column lineage on the Lineage tab (`DATA-13`): nothing is requested until a
// column is picked, then the upstream and downstream columns, the charts and
// dashboards that read it, and the retention line are drawn from what the
// API recorded. A mapping the statement could not fully trace says so, with
// its reason; a caller without dashboard access is told so instead of
// being shown an empty list.
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react"
import { afterEach, describe, expect, it, mock, spyOn } from "bun:test"
import type { ColumnLineage, ColumnMapping } from "@/services/contracts/governance"
import { ColumnLineageCard, columnLabel, producerLabel, retentionLine } from "./column-lineage"

mock.module("next/navigation", () => ({
  usePathname: () => "/data/assets/silver.orders",
  useSearchParams: () => new URLSearchParams(""),
  useRouter: () => ({ push: () => {}, replace: () => {}, refresh: () => {}, back: () => {}, forward: () => {}, prefetch: () => {} }),
}))

afterEach(() => {
  cleanup()
  mock.restore()
})

function json(body: unknown, status = 200) {
  return new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } })
}

const RETENTION = { days: 365, oldestRecordedAt: "2026-09-01T10:00:00Z" }

function mapping(partial: Partial<ColumnMapping> & Pick<ColumnMapping, "from" | "to">): ColumnMapping {
  return {
    transform: "copy",
    partial: false,
    reason: null,
    producer: { kind: "pipeline", ref: "pl-clean" },
    recordedAt: "2026-10-01T10:00:00Z",
    hop: 1,
    ...partial,
  }
}

const UP: ColumnLineage = {
  supported: true,
  table: "silver.orders",
  column: "amt",
  direction: "up",
  mappings: [
    mapping({ from: { table: "bronze.orders", column: "amount" }, to: { table: "silver.orders", column: "amt" } }),
  ],
  retention: RETENTION,
}

const DOWN: ColumnLineage = {
  supported: true,
  table: "silver.orders",
  column: "amt",
  direction: "down",
  mappings: [
    mapping({
      from: { table: "silver.orders", column: "amt" },
      to: { table: "serving.mart_orders", column: "total" },
      transform: "expression",
      producer: { kind: "build", ref: "serving.mart_orders" },
    }),
    mapping({
      from: { table: null, column: null },
      to: { table: "serving.mart_orders", column: "n" },
      partial: true,
      reason: "built with UNION, INTERSECT or EXCEPT; the branches are not traced",
      producer: { kind: "build", ref: "serving.mart_orders" },
      hop: 2,
    }),
  ],
  retention: RETENTION,
  charts: [{ chartId: "c1", chartTitle: "Revenue", dashboardId: "b1", dashboardTitle: "Sales" }],
}

function stubColumns(down: ColumnLineage = DOWN, up: ColumnLineage = UP) {
  return spyOn(globalThis, "fetch").mockImplementation((async (input: RequestInfo | URL) => {
    const path = String(input)
    if (path.includes("/api/governance/lineage/columns")) {
      return json(path.includes("direction=down") ? down : up)
    }
    return json({ error: "not stubbed" }, 404)
  }) as unknown as typeof fetch)
}

function renderCard() {
  return render(<ColumnLineageCard tableKey="silver.orders" columns={["id", "amt"]} />)
}

function pick(column: string) {
  fireEvent.change(screen.getByLabelText("Column"), { target: { value: column } })
}

describe("ColumnLineageCard", () => {
  it("asks for nothing until a column is picked", () => {
    const fetchSpy = stubColumns()
    renderCard()

    expect(screen.getByText("Its upstream and downstream columns appear here.")).toBeTruthy()
    expect(fetchSpy.mock.calls.length).toBe(0)
  })

  it("lists the table's columns in the picker", () => {
    stubColumns()
    renderCard()

    const options = within(screen.getByLabelText("Column")).getAllByRole("option")
    expect(options.map((o) => o.textContent)).toEqual(["Pick a column", "id", "amt"])
  })

  it("asks for both directions of the table key and column, then draws upstream, downstream, charts and retention", async () => {
    const fetchSpy = stubColumns()
    renderCard()
    pick("amt")

    expect(await screen.findByText("bronze.orders.amount")).toBeTruthy()
    const urls = fetchSpy.mock.calls.map((c) => String(c[0]))
    expect(urls.some((u) => u.includes("table=silver.orders") && u.includes("column=amt") && u.includes("direction=up"))).toBe(true)
    expect(urls.some((u) => u.includes("direction=down"))).toBe(true)

    // Downstream: the Gold column built from it, and the producer.
    expect(screen.getByText("serving.mart_orders.total")).toBeTruthy()
    // Both downstream rows were recorded by the build of that Gold table.
    expect(screen.getAllByText("build of serving.mart_orders").length).toBe(2)
    expect(screen.getByText("expression")).toBeTruthy()

    // Charts and dashboards, the dashboard linking to its board.
    expect(screen.getByText("Revenue")).toBeTruthy()
    expect(screen.getByText("Sales").closest("a")?.getAttribute("href")).toBe("/dashboards?board=b1")

    expect(screen.getByText(/Column lineage is kept for 365 days; the oldest record you can see is from/)).toBeTruthy()
  })

  it("marks a mapping it could not trace as partial, with its reason as the title and in words", async () => {
    stubColumns()
    renderCard()
    pick("amt")

    const pill = await screen.findByText("partial")
    expect(pill.getAttribute("title")).toBe("built with UNION, INTERSECT or EXCEPT; the branches are not traced")
    // Not hover-only: the reason is written out under the row.
    expect(screen.getAllByText("built with UNION, INTERSECT or EXCEPT; the branches are not traced").length).toBeGreaterThan(0)
    // The unknown source is named as unknown, not invented.
    expect(screen.getByText("unknown")).toBeTruthy()
  })

  it("says listing dashboards needs access, instead of an empty list", async () => {
    stubColumns({ ...DOWN, charts: undefined, chartsHidden: true })
    renderCard()
    pick("amt")

    expect(await screen.findByText("Listing dashboards needs dashboard access.")).toBeTruthy()
    expect(screen.queryByText("No saved chart reads this column or anything built from it.")).toBeNull()
  })

  it("says the charts could not be read when the API could not read them", async () => {
    stubColumns({ ...DOWN, charts: undefined, chartsUnavailable: true })
    renderCard()
    pick("amt")

    expect(await screen.findByText(/Charts could not be read just now/)).toBeTruthy()
  })

  it("says nothing is recorded, not that nothing exists, for a column with no mappings", async () => {
    stubColumns(
      { ...DOWN, mappings: [], charts: [], retention: { days: 365, oldestRecordedAt: null } },
      { ...UP, mappings: [], retention: { days: 365, oldestRecordedAt: null } }
    )
    renderCard()
    pick("id")

    expect(await screen.findByText("Nothing is recorded as feeding this column.")).toBeTruthy()
    expect(screen.getByText("Nothing is recorded as built from this column.")).toBeTruthy()
    expect(screen.getByText("No saved chart reads this column or anything built from it.")).toBeTruthy()
    expect(screen.getByText(/nothing is recorded yet for what you can see/)).toBeTruthy()
  })

  it("shows what the answer leaves out, once", async () => {
    const note = "lineage of the shared catalog's tables is not shown: not the catalog tenant"
    stubColumns({ ...DOWN, coverage: [note] }, { ...UP, coverage: [note] })
    renderCard()
    pick("amt")

    await waitFor(() => expect(screen.getAllByText(note).length).toBe(1))
  })

  it("shows the error, not an empty list, when the lineage cannot be read", async () => {
    spyOn(globalThis, "fetch").mockImplementation((async () =>
      json({ error: "permission_denied: lineage:read" }, 403)) as unknown as typeof fetch)
    renderCard()
    pick("amt")

    // The permission refusal is shown as a refusal, not as an empty answer.
    expect(await screen.findByText("You don't have access")).toBeTruthy()
    expect(screen.queryByText("Nothing is recorded as feeding this column.")).toBeNull()
  })
})

describe("column lineage wording", () => {
  it("names a column by table and column, and what it knows of a partial end", () => {
    expect(columnLabel({ table: "silver.orders", column: "amt" })).toBe("silver.orders.amt")
    expect(columnLabel({ table: null, column: null })).toBe("unknown")
    expect(columnLabel({ table: "silver.orders", column: null })).toBe("silver.orders")
  })

  it("names who recorded a mapping", () => {
    expect(producerLabel({ kind: "pipeline", ref: "pl-1" })).toBe("pipeline pl-1")
    expect(producerLabel({ kind: "sql_source", ref: "s1" })).toBe("SQL source s1")
    expect(producerLabel({ kind: "build", ref: "serving.m" })).toBe("build of serving.m")
  })

  it("states the retention and, when there is a record, how far back it goes", () => {
    expect(retentionLine({ days: 365, oldestRecordedAt: null })).toBe(
      "Column lineage is kept for 365 days; nothing is recorded yet for what you can see."
    )
    expect(retentionLine(RETENTION)).toContain("the oldest record you can see is from")
  })
})
