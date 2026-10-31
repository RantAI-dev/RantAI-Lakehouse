import { render, screen, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"
import { GoldExportsPage } from "./gold-exports-page"

const originalFetch = global.fetch

afterEach(() => {
  global.fetch = originalFetch
})

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  })
}

/**
 * What the mart's consumers route answers when outside reading is not
 * set up — the honest refusal of DATA-5 D4/D7.
 */
const consumersRefusalBody = {
  mart: "sales",
  supported: false,
  consumers: null,
  reason: "Outside reading is not set up: OUTSIDE_S3_ENDPOINT is not set.",
}

/**
 * A timestamp `h` hours before now, as the API sends it. The page
 * renders the age of a read relative to the moment it renders, so the
 * test asserts the age it set ("2h ago") rather than the instant.
 */
function hoursAgo(h: number): string {
  return new Date(Date.now() - h * 3_600_000).toISOString()
}

/**
 * Installs the fetch mock for one test: the mart list, its publication,
 * its export history, and the outside-reader count (DATA-5).
 */
function mockApi(consumers?: unknown): void {
  global.fetch = mock(async (input: RequestInfo | URL) => {
    const url = typeof input === "string" ? input : input.toString()
    if (url.includes("/api/dashboard/fields")) {
      return jsonResponse({ marts: [{ name: "sales", rows: 42 }] })
    }
    if (url.includes("/publication")) {
      return jsonResponse({
        mart: "sales",
        enabled: false,
        updatedAt: null,
        lastChangedAt: "2026-10-02T03:30:00Z",
        lastExportedAt: null,
        changedSinceExport: null,
        canEdit: true,
      })
    }
    if (url.includes("/consumers")) {
      return jsonResponse(consumers ?? consumersRefusalBody)
    }
    if (url.includes("/api/gold/exports")) {
      return jsonResponse({ mart: "sales", runs: [] })
    }
    if (url.includes("/api/gold/export/sales")) {
      // The mart has never been exported: read_back 500s, same as the
      // real route when the Iceberg table does not exist yet.
      return jsonResponse({ error: "table not found" }, 500)
    }
    throw new Error(`unexpected fetch: ${url}`)
  }) as unknown as typeof fetch
}

describe("GoldExportsPage", () => {
  it("renders a mart's enabled state and an honest consumers refusal, never a fabricated count", async () => {
    mockApi()

    render(<GoldExportsPage />)

    await waitFor(() => expect(screen.getByText("sales")).toBeDefined())
    await waitFor(() => expect(screen.getByText("Off")).toBeDefined())
    await waitFor(() => expect(screen.getByText("Never exported")).toBeDefined())
    // A mart with no measured consumer count renders the honest refusal
    // text, never "0" (which would read as a real, measured zero).
    await waitFor(() => expect(screen.getByText("Not measured")).toBeDefined())
    // The reason says what an operator has to set up.
    const refusal = screen.getByTitle(
      "Outside reading is not set up: OUTSIDE_S3_ENDPOINT is not set.",
    )
    expect(refusal.textContent).toBe("Not measured")
  })

  it("counts the tools that read the mart and names each one in the cell's tooltip", async () => {
    mockApi({
      mart: "sales",
      supported: true,
      windowDays: 7,
      consumers: [
        {
          identityId: "11111111-1111-1111-1111-111111111111",
          name: "Finance DuckDB",
          reads: 4,
          lastReadAt: hoursAgo(2),
        },
        {
          identityId: "22222222-2222-2222-2222-222222222222",
          name: "Analytics",
          reads: 1,
          lastReadAt: hoursAgo(24 * 3),
        },
      ],
    })

    render(<GoldExportsPage />)

    // The cell holds the number of tools, and its tooltip one line per
    // tool: name, reads and when it last read.
    const cell = await screen.findByTitle(/Finance DuckDB/)
    expect(cell.textContent).toBe("2")
    const tooltip = cell.getAttribute("title") ?? ""
    expect(tooltip).toContain("Finance DuckDB: 4 reads, last 2h ago")
    expect(tooltip).toContain("Analytics: 1 reads, last 3d ago")
  })

  it("says No reads in 7 days when the door is set up and nobody read the mart", async () => {
    mockApi({ mart: "sales", supported: true, windowDays: 7, consumers: [] })

    render(<GoldExportsPage />)

    await waitFor(() => expect(screen.getByText("No reads in 7 days")).toBeDefined())
    // The consumers cell says so in words; it never falls back to a bare
    // "0", which would read as a measured count of zero readers. (The
    // row's own history column does show a "0" — that one is a count.)
    const row = screen.getByText("sales").closest("tr")
    const cells = row?.querySelectorAll("td") ?? []
    // Column order: mart, rows, enabled, last export, snapshot,
    // history, consumers, actions.
    expect(cells[6]?.textContent).toBe("No reads in 7 days")
  })
})
