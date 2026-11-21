import { fireEvent, render, screen, waitFor } from "@testing-library/react"
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
        lastFailure: null,
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
      // The mart has never been published: the route answers 200 with
      // `published: false` and every fact null (DATA-49, D11). The API changed,
      // not the assertion.
      return jsonResponse({
        namespace: "gold",
        table: "sales",
        published: false,
        formatVersion: null,
        rowsInIceberg: null,
        snapshotId: null,
        exportedAt: null,
      })
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

  // QA LH-DATA-1-01 (DATA-49): the route answers 200 with `published: false` for
  // a mart never published, and that is what the cell reads.
  //
  // Review round 1 R5: the body carries a stray `snapshotId`, so the snapshot cell
  // is the assertion — with `published` dropped from its condition, the stray value
  // is printed and this test fails.
  it("says Never exported, and no snapshot, for a mart the API reports as not published", async () => {
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
          lastFailure: null,
          canEdit: true,
        })
      }
      if (url.includes("/consumers")) return jsonResponse(consumersRefusalBody)
      if (url.includes("/api/gold/exports")) return jsonResponse({ mart: "sales", runs: [] })
      if (url.includes("/api/gold/export/sales")) {
        return jsonResponse({
          namespace: "gold",
          table: "sales",
          published: false,
          formatVersion: null,
          rowsInIceberg: null,
          snapshotId: { value: 999, measured: true },
          exportedAt: "2026-10-02T04:00:00Z",
        })
      }
      throw new Error(`unexpected fetch: ${url}`)
    }) as unknown as typeof fetch

    render(<GoldExportsPage />)
    const row = (await screen.findByText("sales")).closest("tr") as HTMLTableRowElement
    const cells = row.querySelectorAll("td")
    // Column order: mart, rows, enabled, last export, snapshot, history, consumers, actions.
    await waitFor(() => expect(cells[3]?.textContent).toBe("Never exported"))
    expect(cells[4]?.textContent).toBe("—")
  })

  it("shows a dash, not Never exported, when the read-back fails", async () => {
    const original = global.fetch
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
          lastChangedAt: null,
          lastExportedAt: null,
          changedSinceExport: null,
          lastFailure: null,
          canEdit: true,
        })
      }
      if (url.includes("/consumers")) return jsonResponse(consumersRefusalBody)
      if (url.includes("/api/gold/exports")) return jsonResponse({ mart: "sales", runs: [] })
      if (url.includes("/api/gold/export/sales")) {
        return jsonResponse({ error: "The catalog request failed. Reference: abc123" }, 500)
      }
      throw new Error(`unexpected fetch: ${url}`)
    }) as unknown as typeof fetch
    expect(original).toBeDefined()

    render(<GoldExportsPage />)
    await waitFor(() => expect(screen.getByText("sales")).toBeDefined())
    await waitFor(() => expect(screen.queryByText("Never exported")).toBeNull())
    const dashes = screen.getAllByText("—")
    expect(dashes.some((dash) => dash.className.includes("text-destructive"))).toBe(true)
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

  // QA LH-DATA-1-01 (DATA-49), review round 1 R1: the page's own "Export now" left
  // the label and the run count where they were after a failed publish, so the row
  // read "On · Up to date" and 0 runs until the page was loaded again.
  it("reads the publication and the run count again after a failed Export now", async () => {
    const reason = "This mart has 12000000 rows, exceeding GOLD_EXPORT_MAX_ROWS."
    let publications = 0
    let histories = 0
    let posts = 0
    global.fetch = mock(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = typeof input === "string" ? input : input.toString()
      if (url.includes("/api/dashboard/fields")) {
        return jsonResponse({ marts: [{ name: "sales", rows: 42 }] })
      }
      if (url.includes("/publication")) {
        publications += 1
        return jsonResponse({
          mart: "sales",
          enabled: true,
          updatedAt: null,
          lastChangedAt: "2026-10-02T03:30:00Z",
          lastExportedAt: "2026-10-02T04:00:00Z",
          changedSinceExport: false,
          lastFailure: publications === 1 ? null : { failedAt: "2026-10-02T05:00:00Z", reason },
          canEdit: true,
        })
      }
      if (url.includes("/consumers")) return jsonResponse(consumersRefusalBody)
      if (url.includes("/api/gold/exports")) {
        histories += 1
        return jsonResponse({
          mart: "sales",
          runs:
            histories === 1
              ? []
              : [
                  {
                    id: "run-1",
                    status: "failed",
                    rowsExported: null,
                    formatVersion: null,
                    snapshotId: null,
                    error: reason,
                    triggeredBy: "manual",
                    startedAt: "2026-10-02T05:00:00Z",
                    finishedAt: "2026-10-02T05:00:02Z",
                  },
                ],
        })
      }
      if (init?.method === "POST" && url.includes("/api/gold/export/sales")) {
        posts += 1
        return jsonResponse({ error: reason }, 422)
      }
      if (url.includes("/api/gold/export/sales")) {
        return jsonResponse({
          namespace: "gold",
          table: "sales",
          published: true,
          formatVersion: 2,
          rowsInIceberg: 42,
          snapshotId: { value: 12345, measured: true },
          exportedAt: "2026-10-02T04:00:00Z",
        })
      }
      throw new Error(`unexpected fetch: ${init?.method} ${url}`)
    }) as unknown as typeof fetch

    render(<GoldExportsPage />)
    const row = (await screen.findByText("sales")).closest("tr") as HTMLTableRowElement
    const cells = () => row.querySelectorAll("td")
    // Column order: mart, rows, enabled, last export, snapshot, history, consumers, actions.
    await waitFor(() => expect(cells()[2]?.textContent).toBe("On · Up to date"))
    expect(cells()[5]?.textContent).toBe("0")

    fireEvent.click(screen.getByRole("button", { name: "Export now" }))
    await waitFor(() => expect(posts).toBe(1))
    await waitFor(() => expect(cells()[2]?.textContent).toBe("On · Last publish failed"))
    expect(cells()[5]?.textContent).toBe("1")
  })
})
