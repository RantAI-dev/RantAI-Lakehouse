import { fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"
import { OpenFormatCard } from "./open-format-card"

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
 * set up — the honest refusal of DATA-5 D4/D7. It is the default for
 * every test in this file that does not care about readers.
 */
const consumersRefusalBody = {
  mart: "mart_a",
  supported: false,
  consumers: null,
  reason: "Outside reading is not set up: OUTSIDE_S3_ENDPOINT is not set.",
}

/**
 * Installs the fetch mock for one test.
 *
 * The card asks the mart's outside-reader count as well as its
 * publication (DATA-5 D10), and the count's URL also matches the "last
 * export" pattern the per-test handler uses. `mockApi` therefore
 * answers `/consumers` first — with `consumers` when the test passes
 * one, with the honest refusal otherwise — and hands every other URL to
 * the test's own handler.
 */
function mockApi(impl: (url: string) => Response, consumers?: unknown): void {
  global.fetch = mock(async (input: RequestInfo | URL) => {
    const url = typeof input === "string" ? input : input.toString()
    if (url.includes("/consumers")) {
      return jsonResponse(consumers ?? consumersRefusalBody)
    }
    return impl(url)
  }) as unknown as typeof fetch
}

const publicationBody: Record<string, unknown> = {
  mart: "mart_a",
  enabled: true,
  updatedAt: "2026-10-02T04:00:00Z",
  lastChangedAt: "2026-10-02T03:30:00Z",
  lastExportedAt: "2026-10-02T04:00:00Z",
  changedSinceExport: false,
  canEdit: true,
}

const readBackBody = {
  namespace: "gold",
  table: "mart_a",
  formatVersion: 2,
  rowsInIceberg: 42,
  snapshotId: { value: 12345, measured: true },
  exportedAt: "2026-10-02T04:00:00Z",
}

function mockReadBack() {
  return jsonResponse(readBackBody)
}

/**
 * A timestamp `h` hours before now, as the API sends it. The card
 * renders the age of a read relative to the moment it renders, so the
 * tests assert the age they set ("2h ago") rather than the instant.
 */
function hoursAgo(h: number): string {
  return new Date(Date.now() - h * 3_600_000).toISOString()
}

const emptyRunsBody = { mart: "mart_a", runs: [] satisfies GoldExportRun[] }

import type { GoldExportRun } from "@/services/contracts/gold"

describe("OpenFormatCard", () => {
  it("shows Up to date when changedSinceExport is false", async () => {
    mockApi((url) => {
      if (url.includes("/publication")) {
        return jsonResponse({
          ...publicationBody,
          changedSinceExport: false,
        })
      }
      if (url.includes("/api/gold/export/mart_a") && !url.includes("/publication") && !url.includes("/exports")) {
        return mockReadBack()
      }
      if (url.includes("/api/gold/exports")) {
        return jsonResponse(emptyRunsBody)
      }
      throw new Error(`unexpected fetch: ${url}`)
    })

    render(<OpenFormatCard assetId="serving.mart_a" />)
    await waitFor(() =>
      expect(screen.getByTestId("publication-status").textContent).toBe("Up to date"),
    )
  })

  it("shows Out of date when changedSinceExport is true", async () => {
    mockApi((url) => {
      if (url.includes("/publication")) {
        return jsonResponse({
          ...publicationBody,
          changedSinceExport: true,
        })
      }
      if (url.includes("/api/gold/export/mart_a") && !url.includes("/publication") && !url.includes("/exports")) {
        return mockReadBack()
      }
      if (url.includes("/api/gold/exports")) {
        return jsonResponse(emptyRunsBody)
      }
      throw new Error(`unexpected fetch: ${url}`)
    })

    render(<OpenFormatCard assetId="serving.mart_a" />)
    await waitFor(() =>
      expect(screen.getByTestId("publication-status").textContent).toBe("Out of date"),
    )
  })

  it("shows Not measured when changedSinceExport is null", async () => {
    mockApi((url) => {
      if (url.includes("/publication")) {
        return jsonResponse({
          ...publicationBody,
          changedSinceExport: null,
        })
      }
      if (url.includes("/api/gold/export/mart_a") && !url.includes("/publication") && !url.includes("/exports")) {
        return mockReadBack()
      }
      if (url.includes("/api/gold/exports")) {
        return jsonResponse(emptyRunsBody)
      }
      throw new Error(`unexpected fetch: ${url}`)
    })

    render(<OpenFormatCard assetId="serving.mart_a" />)
    await waitFor(() =>
      expect(screen.getByTestId("publication-status").textContent).toBe("Not measured"),
    )
  })

  it("shows Never published when lastExportedAt is null", async () => {
    mockApi((url) => {
      if (url.includes("/publication")) {
        return jsonResponse({
          ...publicationBody,
          lastChangedAt: "2026-10-02T03:30:00Z",
          lastExportedAt: null,
        })
      }
      if (url.includes("/api/gold/exports")) {
        return jsonResponse(emptyRunsBody)
      }
      throw new Error(`unexpected fetch: ${url}`)
    })

    render(<OpenFormatCard assetId="serving.mart_a" />)
    await waitFor(() => expect(screen.getByText("Never published")).toBeDefined())
  })

  it("disables the switch when canEdit is false and shows an explanation", async () => {
    mockApi((url) => {
      if (url.includes("/publication")) {
        return jsonResponse({
          ...publicationBody,
          canEdit: false,
        })
      }
      if (url.includes("/api/gold/export/mart_a") && !url.includes("/publication") && !url.includes("/exports")) {
        return mockReadBack()
      }
      if (url.includes("/api/gold/exports")) {
        return jsonResponse(emptyRunsBody)
      }
      throw new Error(`unexpected fetch: ${url}`)
    })

    render(<OpenFormatCard assetId="serving.mart_a" />)
    await waitFor(() => expect(screen.getByText(/gold:export/)).toBeDefined())
    const switchEl = screen.getByRole("switch")
    expect(switchEl).toBeDefined()
    expect(switchEl.hasAttribute("data-disabled")).toBe(true)
  })

  it("leaves the switch off after a failed PUT and shows the server's error", async () => {
    // PR slice D review D-S2: render from toggle.status === "error",
    // not from a stale closure capture of toggle.error. The returned
    // error message routes through the service action state so the test
    // can assert the server's actual message appears in the DOM.
    let putCalled = false
    mockApi((url) => {
      if (url.includes("/publication")) {
        if (putCalled) {
          return jsonResponse({ error: "Gold export is internal to this server — the previous response is still valid" }, 500)
        }
        putCalled = true
        return jsonResponse({
          ...publicationBody,
          enabled: false,
        })
      }
      if (url.includes("/api/gold/exports")) {
        return jsonResponse(emptyRunsBody)
      }
      throw new Error(`unexpected fetch: ${url}`)
    })

    render(<OpenFormatCard assetId="serving.mart_a" />)
    await waitFor(() => {
      const statusElements = screen.getAllByText("Off")
      expect(statusElements.length).toBeGreaterThanOrEqual(1)
    })
    const switchEl = screen.getByRole("switch")
    fireEvent.click(switchEl)
    await waitFor(() => expect(screen.getByRole("alert")).toBeDefined())
    // PR slice D review D-S2: assert the server's message is shown.
    expect(screen.getByText(/Gold export is internal/)).toBeDefined()
  })

  it("shows Never published and no last-export info when switch is off and mart was never published", async () => {
    // PR slice D review D-B3: with the switch off and never published,
    // the card still shows "Never published" and no last-export line.
    mockApi((url) => {
      if (url.includes("/publication")) {
        return jsonResponse({
          ...publicationBody,
          enabled: false,
          lastChangedAt: "2026-10-02T03:30:00Z",
          lastExportedAt: null,
        })
      }
      if (url.includes("/api/gold/exports")) {
        return jsonResponse(emptyRunsBody)
      }
      throw new Error(`unexpected fetch: ${url}`)
    })

    render(<OpenFormatCard assetId="serving.mart_a" />)
    await waitFor(() => expect(screen.getByText("Never published")).toBeDefined())
    expect(screen.queryByText("Last published")).toBeNull()
  })

  it("shows last published time and snapshot plus off notice when switch is off and mart was previously published", async () => {
    // PR slice D review D-B3: with the switch off and a prior publish
    // on record, the card still shows last published time, snapshot,
    // and the "Publishing is off" note.
    mockApi((url) => {
      if (url.includes("/publication")) {
        return jsonResponse({
          ...publicationBody,
          enabled: false,
          lastChangedAt: "2026-10-02T03:30:00Z",
          lastExportedAt: "2026-10-02T04:00:00Z",
        })
      }
      if (url.includes("/api/gold/export/mart_a") && !url.includes("/publication") && !url.includes("/exports")) {
        return mockReadBack()
      }
      if (url.includes("/api/gold/exports")) {
        return jsonResponse(emptyRunsBody)
      }
      throw new Error(`unexpected fetch: ${url}`)
    })

    render(<OpenFormatCard assetId="serving.mart_a" />)
    await waitFor(() =>
      expect(screen.getByTestId("publication-status").textContent).toBe("Up to date"),
    )
    expect(screen.getByText("Last published")).toBeDefined()
    expect(screen.getByText("Publishing is off; this copy is no longer updated.")).toBeDefined()
  })

  it("shows error text for failed runs in the last-5 table", async () => {
    // PR slice D review D-B2: GoldExportRun.error carries the reason
    // and must be visible for failed rows.
    const failedRun: GoldExportRun = {
      id: "run-1",
      status: "failed",
      rowsExported: null,
      formatVersion: null,
      snapshotId: null,
      error: "ClickHouse connection refused (111)",
      triggeredBy: "scheduler",
      startedAt: "2026-10-02T04:00:00Z",
      finishedAt: "2026-10-02T04:00:02Z",
    }
    mockApi((url) => {
      if (url.includes("/publication")) {
        return jsonResponse(publicationBody)
      }
      if (url.includes("/api/gold/export/mart_a") && !url.includes("/publication") && !url.includes("/exports")) {
        return mockReadBack()
      }
      if (url.includes("/api/gold/exports")) {
        return jsonResponse({ mart: "mart_a", runs: [failedRun] })
      }
      throw new Error(`unexpected fetch: ${url}`)
    })

    render(<OpenFormatCard assetId="serving.mart_a" />)
    await waitFor(() => expect(screen.getByText("ClickHouse connection refused (111)")).toBeDefined())
  })

  it("shows Copies kept and oldest timestamp when trim succeeded", async () => {
    mockApi((url) => {
      if (url.includes("/publication")) {
        return jsonResponse({
          ...publicationBody,
          trim: {
            status: "success",
            copiesKept: 7,
            oldestKeptExportedAt: "2026-10-14T10:00:00Z",
            finishedAt: "2026-10-14T10:05:00Z",
            reason: null,
          },
        })
      }
      if (url.includes("/api/gold/export/mart_a") && !url.includes("/publication") && !url.includes("/exports")) {
        return mockReadBack()
      }
      if (url.includes("/api/gold/exports")) {
        return jsonResponse(emptyRunsBody)
      }
      throw new Error(`unexpected fetch: ${url}`)
    })

    render(<OpenFormatCard assetId="serving.mart_a" />)
    await waitFor(() => expect(screen.getByText("Copies kept")).toBeDefined())
    expect(screen.getByText(/7 \(oldest/)).toBeDefined()
  })

  it("shows failed trim reason and never trimmed when trim failed", async () => {
    mockApi((url) => {
      if (url.includes("/publication")) {
        return jsonResponse({
          ...publicationBody,
          trim: {
            status: "failed",
            copiesKept: null,
            oldestKeptExportedAt: null,
            finishedAt: "2026-10-14T10:05:00Z",
            reason: "delete_failed",
          },
        })
      }
      if (url.includes("/api/gold/export/mart_a") && !url.includes("/publication") && !url.includes("/exports")) {
        return mockReadBack()
      }
      if (url.includes("/api/gold/exports")) {
        return jsonResponse(emptyRunsBody)
      }
      throw new Error(`unexpected fetch: ${url}`)
    })

    render(<OpenFormatCard assetId="serving.mart_a" />)
    await waitFor(() => expect(screen.getByText("Trim failed: delete_failed")).toBeDefined())
    expect(screen.queryByText("Copies kept")).toBeNull()
    expect(screen.queryByText(/\btrimmed\b/i)).toBeNull()
  })

  it("never shows trimmed when trim is null", async () => {
    mockApi((url) => {
      if (url.includes("/publication")) {
        return jsonResponse({
          ...publicationBody,
          trim: null,
        })
      }
      if (url.includes("/api/gold/export/mart_a") && !url.includes("/publication") && !url.includes("/exports")) {
        return mockReadBack()
      }
      if (url.includes("/api/gold/exports")) {
        return jsonResponse(emptyRunsBody)
      }
      throw new Error(`unexpected fetch: ${url}`)
    })

    render(<OpenFormatCard assetId="serving.mart_a" />)
    await waitFor(() =>
      expect(screen.getByTestId("publication-status").textContent).toBe("Up to date"),
    )
    expect(screen.queryByText("Copies kept")).toBeNull()
    expect(screen.queryByText(/\btrimmed\b/i)).toBeNull()
  })

  // ── DATA-5 D10: the "Readers (7 days)" row ──────────────────────
  //
  // Three states, and the third one is the point of the feature: a
  // mart nobody read says "No reads in 7 days", while a deployment
  // without outside reading says "Not measured" with the reason — never
  // a zero.

  it("names each tool that read the table, its reads and its last read", async () => {
    mockApi(
      (url) => {
        if (url.includes("/publication")) {
          return jsonResponse(publicationBody)
        }
        if (url.includes("/api/gold/export/mart_a") && !url.includes("/publication") && !url.includes("/exports")) {
          return mockReadBack()
        }
        if (url.includes("/api/gold/exports")) {
          return jsonResponse(emptyRunsBody)
        }
        throw new Error(`unexpected fetch: ${url}`)
      },
      {
        mart: "mart_a",
        supported: true,
        windowDays: 7,
        consumers: [
          {
            identityId: "11111111-1111-1111-1111-111111111111",
            name: "Finance DuckDB",
            reads: 4,
            lastReadAt: hoursAgo(2),
          },
        ],
      },
    )

    render(<OpenFormatCard assetId="serving.mart_a" />)
    await waitFor(() => expect(screen.getByText("Readers (7 days)")).toBeDefined())
    expect(screen.getByText(/Finance DuckDB: 4 reads, last 2h ago/)).toBeDefined()
  })

  it("names at most five tools and counts the rest", async () => {
    const readers = Array.from({ length: 7 }, (_, i) => ({
      identityId: `00000000-0000-0000-0000-00000000000${i}`,
      name: `Tool ${i + 1}`,
      reads: i + 1,
      lastReadAt: hoursAgo(1),
    }))
    mockApi(
      (url) => {
        if (url.includes("/publication")) {
          return jsonResponse(publicationBody)
        }
        if (url.includes("/api/gold/export/mart_a") && !url.includes("/publication") && !url.includes("/exports")) {
          return mockReadBack()
        }
        if (url.includes("/api/gold/exports")) {
          return jsonResponse(emptyRunsBody)
        }
        throw new Error(`unexpected fetch: ${url}`)
      },
      { mart: "mart_a", supported: true, windowDays: 7, consumers: readers },
    )

    render(<OpenFormatCard assetId="serving.mart_a" />)
    await waitFor(() => expect(screen.getByText("Readers (7 days)")).toBeDefined())
    expect(screen.getByText(/Tool 5: 5 reads/)).toBeDefined()
    expect(screen.queryByText(/Tool 6:/)).toBeNull()
    expect(screen.getByText("and 2 more")).toBeDefined()
  })

  it("says No reads in 7 days when the door is set up and nobody read the table", async () => {
    mockApi(
      (url) => {
        if (url.includes("/publication")) {
          return jsonResponse(publicationBody)
        }
        if (url.includes("/api/gold/export/mart_a") && !url.includes("/publication") && !url.includes("/exports")) {
          return mockReadBack()
        }
        if (url.includes("/api/gold/exports")) {
          return jsonResponse(emptyRunsBody)
        }
        throw new Error(`unexpected fetch: ${url}`)
      },
      { mart: "mart_a", supported: true, windowDays: 7, consumers: [] },
    )

    render(<OpenFormatCard assetId="serving.mart_a" />)
    await waitFor(() => expect(screen.getByText("No reads in 7 days")).toBeDefined())
  })

  it("says Not measured with the reason when outside reading is not set up", async () => {
    mockApi((url) => {
      if (url.includes("/publication")) {
        return jsonResponse(publicationBody)
      }
      if (url.includes("/api/gold/export/mart_a") && !url.includes("/publication") && !url.includes("/exports")) {
        return mockReadBack()
      }
      if (url.includes("/api/gold/exports")) {
        return jsonResponse(emptyRunsBody)
      }
      throw new Error(`unexpected fetch: ${url}`)
    })

    render(<OpenFormatCard assetId="serving.mart_a" />)
    await waitFor(() => expect(screen.getByText("Not measured")).toBeDefined())
    // The reason is the title of the same element: the admin must learn
    // what to set up, not only that nothing was counted.
    const refusal = screen.getByTitle(
      "Outside reading is not set up: OUTSIDE_S3_ENDPOINT is not set.",
    )
    expect(refusal.textContent).toBe("Not measured")
  })

})
