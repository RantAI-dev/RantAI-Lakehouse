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

/**
 * `DATA-66`: a card whose publication `PUT` changes what the next `GET`
 * answers, so a test can assert both the request body and the mode line
 * after the save. Every other route answers as the card expects.
 */
function mockSyncCard(initial: Record<string, unknown>) {
  const calls: { method?: string; url: string; body?: unknown }[] = []
  let current = { ...initial }
  global.fetch = mock(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = typeof input === "string" ? input : input.toString()
    if (url.includes("/consumers")) return jsonResponse(consumersRefusalBody)
    if (url.includes("/publication")) {
      if (init?.method === "PUT") {
        const body = init.body ? JSON.parse(String(init.body)) : null
        calls.push({ method: "PUT", url, body })
        current = { ...current, ...(body as Record<string, unknown>) }
        return jsonResponse(current)
      }
      return jsonResponse(current)
    }
    if (url.includes("/api/gold/exports")) return jsonResponse(emptyRunsBody)
    if (url.includes("/api/gold/export/mart_a")) return mockReadBack()
    throw new Error(`unexpected fetch: ${init?.method} ${url}`)
  }) as unknown as typeof fetch
  return calls
}

function pickOption(option: HTMLElement) {
  fireEvent.pointerDown(option, { button: 0, pointerType: "mouse" })
  fireEvent.mouseDown(option, { button: 0 })
  fireEvent.pointerUp(option, { button: 0 })
  fireEvent.mouseUp(option, { button: 0 })
  fireEvent.click(option)
}

async function chooseSyncMode(label: string) {
  fireEvent.click(screen.getByRole("combobox", { name: "Sync mode" }))
  pickOption(await screen.findByRole("option", { name: label }))
}

const publicationBody: Record<string, unknown> = {
  mart: "mart_a",
  enabled: true,
  updatedAt: "2026-10-02T04:00:00Z",
  lastChangedAt: "2026-10-02T03:30:00Z",
  lastExportedAt: "2026-10-02T04:00:00Z",
  changedSinceExport: false,
  lastFailure: null,
  canEdit: true,
  // DATA-66: the sync setting the detail route always answers.
  syncMode: "on_update",
  schedule: null,
  timeZone: null,
  nextSyncAt: null,
}

const readBackBody = {
  namespace: "gold",
  table: "mart_a",
  published: true,
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

  // QA LH-DATA-1-01 (DATA-49), review round 1 R1: the answer to a failed publish
  // is `lastFailure` on the publication route and a new run in the history, so the
  // card reads both again the moment the action settles. Before, it kept the status
  // it had — "Up to date" — until the person loaded the page again.
  it("reads the publication again after a failed Publish now and says Last publish failed", async () => {
    const reason = "This mart has 12000000 rows, exceeding GOLD_EXPORT_MAX_ROWS."
    let publications = 0
    let posts = 0
    global.fetch = mock(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = typeof input === "string" ? input : input.toString()
      if (url.includes("/consumers")) return jsonResponse(consumersRefusalBody)
      if (url.includes("/publication")) {
        publications += 1
        return jsonResponse(
          publications === 1
            ? publicationBody
            : { ...publicationBody, lastFailure: { failedAt: "2026-10-02T05:00:00Z", reason } },
        )
      }
      if (init?.method === "POST" && url.includes("/api/gold/export/mart_a")) {
        posts += 1
        return jsonResponse({ error: reason }, 422)
      }
      if (url.includes("/api/gold/export/mart_a") && !url.includes("/exports")) {
        return mockReadBack()
      }
      if (url.includes("/api/gold/exports")) {
        return jsonResponse(emptyRunsBody)
      }
      throw new Error(`unexpected fetch: ${init?.method} ${url}`)
    }) as unknown as typeof fetch

    render(<OpenFormatCard assetId="serving.mart_a" />)
    await waitFor(() =>
      expect(screen.getByTestId("publication-status").textContent).toBe("Up to date"),
    )

    fireEvent.click(screen.getByRole("button", { name: "Publish now" }))
    await waitFor(() => expect(posts).toBe(1))
    await waitFor(() =>
      expect(screen.getByTestId("publication-status").textContent).toBe("Last publish failed"),
    )
    expect(screen.getByTestId("publication-failure").textContent).toContain(reason)
  })

  // QA LH-DATA-1-01 (DATA-49): a failed newest run outranks the fingerprint
  // comparison, and the card says what failed, when, and where the last good copy is.
  it("shows Last publish failed with the time, the reason and the last successful publish", async () => {
    mockApi((url) => {
      if (url.includes("/publication")) {
        return jsonResponse({
          ...publicationBody,
          changedSinceExport: false,
          lastFailure: { failedAt: "2026-10-02T05:00:00Z", reason: "The catalog request failed. Reference: abc123" },
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
      expect(screen.getByTestId("publication-status").textContent).toBe("Last publish failed"),
    )
    expect(screen.getByTestId("publication-status").className).toContain("text-destructive")
    const failure = screen.getByTestId("publication-failure").textContent ?? ""
    expect(failure).toContain("Failed ")
    expect(failure).toContain("The catalog request failed. Reference: abc123")
    expect(screen.getByText(/Last successful publish: /)).toBeDefined()
  })

  it("says nothing has been published yet when the only run failed", async () => {
    mockApi((url) => {
      if (url.includes("/publication")) {
        return jsonResponse({
          ...publicationBody,
          lastExportedAt: null,
          changedSinceExport: null,
          lastFailure: { failedAt: null, reason: "A value in this table could not be written in open format." },
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
      expect(screen.getByTestId("publication-status").textContent).toBe("Last publish failed"),
    )
    expect(screen.getByTestId("publication-failure").textContent).toBe(
      "Failed: A value in this table could not be written in open format.",
    )
    expect(screen.getByText("Nothing has been published yet.")).toBeDefined()
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

  // QA LH-DATA-1-01 (DATA-49): a mart never published answers `published: false`,
  // and the card shows no last-published rows for it — there is nothing to name.
  //
  // Review round 1 R5: the publication here HAS a last export on record, so
  // `hasHistory` is true and the `published` read-back is the only thing that
  // closes the two rows. With `lastExportedAt: null` the test passed even with the
  // `published` check removed, which proved nothing.
  it("shows no last-published rows when the read-back says the table is not there", async () => {
    let readBacks = 0
    mockApi((url) => {
      if (url.includes("/publication")) {
        return jsonResponse({
          ...publicationBody,
          lastExportedAt: "2026-10-02T04:00:00Z",
          changedSinceExport: false,
        })
      }
      if (url.includes("/api/gold/export/mart_a") && !url.includes("/publication") && !url.includes("/exports")) {
        readBacks += 1
        // The catalog no longer has the table, so the read-back names no copy —
        // a stray snapshot in the body is still not a published one.
        return jsonResponse({
          namespace: "gold",
          table: "mart_a",
          published: false,
          formatVersion: null,
          rowsInIceberg: null,
          snapshotId: { value: 999, measured: true },
          exportedAt: "2026-10-02T04:00:00Z",
        })
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
    await waitFor(() => expect(readBacks).toBe(1))
    expect(screen.queryByText("Last published")).toBeNull()
    expect(screen.queryByText("999")).toBeNull()
    expect(screen.queryByText("Snapshot")).toBeNull()
    expect(screen.queryByText("Could not be read")).toBeNull()
  })

  // A failed read-back is a failure, not "never published": the row says the copy
  // could not be read, with the API's sentence as its tooltip.
  it("says the published copy could not be read when the read-back fails", async () => {
    mockApi((url) => {
      if (url.includes("/publication")) {
        return jsonResponse({ ...publicationBody })
      }
      if (url.includes("/api/gold/export/mart_a") && !url.includes("/publication") && !url.includes("/exports")) {
        return jsonResponse({ error: "The catalog request failed. Reference: abc123" }, 500)
      }
      if (url.includes("/api/gold/exports")) {
        return jsonResponse(emptyRunsBody)
      }
      throw new Error(`unexpected fetch: ${url}`)
    })

    render(<OpenFormatCard assetId="serving.mart_a" />)
    const cell = await screen.findByText("Could not be read")
    expect(cell.className).toContain("text-destructive")
    expect(cell.getAttribute("title")).toBe("The catalog request failed. Reference: abc123")
    expect(screen.queryByText("Never published")).toBeNull()
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

  it("makes no request for the publish history when the person may not edit", async () => {
    // QA LH-DATA-1-02 (plan D7): those routes need gold:export, and a 401
    // from them signed the person out. A read-only publication asks for the
    // publication and the readers only.
    const urls: string[] = []
    mockApi((url) => {
      urls.push(url)
      if (url.includes("/publication")) {
        return jsonResponse({ ...publicationBody, canEdit: false })
      }
      throw new Error(`unexpected fetch: ${url}`)
    })

    render(<OpenFormatCard assetId="serving.mart_a" />)
    await waitFor(() => expect(screen.getByText(/gold:export/)).toBeDefined())
    // Let any request the card would still send go out before looking.
    await new Promise((resolve) => setTimeout(resolve, 50))
    expect(urls.some((u) => u.includes("/api/gold/exports"))).toBe(false)
    expect(
      urls.some(
        (u) =>
          u.includes("/api/gold/export/mart_a") &&
          !u.includes("/publication") &&
          !u.includes("/exports"),
      ),
    ).toBe(false)
    expect(screen.queryByText("Last 5 publish runs")).toBeNull()
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
    // PR #456 CI: the history is read only after the publication loads (plan D7), so wait for it.
    expect(await screen.findByText("Last published")).toBeDefined()
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

  it("shows Copies kept at last trim with when it was counted and the oldest copy", async () => {
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
    // The label changes by D17 (DATA-49): the number is the last trim's, and a
    // publish after it is not counted, so the row says when it was counted.
    await waitFor(() => expect(screen.getByText("Copies kept at last trim")).toBeDefined())
    expect(screen.getByText(/^7 \(counted .+; oldest .+\)$/)).toBeDefined()
  })

  it("does not show a counted time when the trim recorded none", async () => {
    mockApi((url) => {
      if (url.includes("/publication")) {
        return jsonResponse({
          ...publicationBody,
          trim: {
            status: "success",
            copiesKept: 3,
            oldestKeptExportedAt: null,
            finishedAt: "",
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
    await waitFor(() => expect(screen.getByText("Copies kept at last trim")).toBeDefined())
    const value = screen.getByText("Copies kept at last trim").nextElementSibling?.textContent
    expect(value).toBe("3")
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
    expect(screen.queryByText("Copies kept at last trim")).toBeNull()
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
    expect(screen.queryByText("Copies kept at last trim")).toBeNull()
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

  // ── DATA-6 D4: the latest-copy block of the publication card ──────────

  function mockWithPublication(latestCopy: Record<string, unknown> | null) {
    mockApi((url) => {
      if (url.includes("/publication")) {
        return jsonResponse({ ...publicationBody, latestCopy })
      }
      if (url.includes("/api/gold/export/mart_a") && !url.includes("/publication") && !url.includes("/exports")) {
        return mockReadBack()
      }
      if (url.includes("/api/gold/exports")) {
        return jsonResponse(emptyRunsBody)
      }
      throw new Error(`unexpected fetch: ${url}`)
    })
  }

  it("shows the latest table's name with a copy button", async () => {
    mockWithPublication({
      table: "gold.mart_a_latest",
      holds: {
        copyExportedAt: "2026-11-20T10:00:00.123456Z",
        rows: 17,
        replacedAt: "2026-11-20T10:00:05Z",
      },
      lastRun: {
        status: "success",
        reason: null,
        cleanup: null,
        finishedAt: "2026-11-20T10:00:05Z",
      },
    })

    render(<OpenFormatCard assetId="serving.mart_a" />)
    await waitFor(() => expect(screen.getByText("gold.mart_a_latest")).toBeDefined())
    // One copy button for the table's name, reusing the shared Copyable.
    expect(screen.getByRole("button", { name: "Copy" })).toBeDefined()
    // The held copy is the one the newest publish wrote.
    expect(
      screen.getByText(/^The copy published .+ \(17 rows\)$/).textContent,
    ).toMatch(/\(17 rows\)$/)
  })

  it("says Not written yet when no copy has been written", async () => {
    mockWithPublication({
      table: "gold.mart_a_latest",
      holds: null,
      lastRun: null,
    })

    render(<OpenFormatCard assetId="serving.mart_a" />)
    await waitFor(() => expect(screen.getByText("Not written yet")).toBeDefined())
    // The name shows even before the first update: it is a fact of the
    // design, so a tool can be pointed at the table ahead of time.
    expect(screen.getByText("gold.mart_a_latest")).toBeDefined()
  })

  it("shows a failed run's fixed sentence and keeps the copy it still holds", async () => {
    mockWithPublication({
      table: "gold.mart_a_latest",
      holds: {
        copyExportedAt: "2026-11-20T10:00:00.123456Z",
        rows: 17,
        replacedAt: "2026-11-20T10:00:05Z",
      },
      lastRun: {
        status: "failed",
        reason: "copy_incomplete",
        cleanup: null,
        finishedAt: "2026-11-20T11:00:05Z",
      },
    })

    render(<OpenFormatCard assetId="serving.mart_a" />)
    await waitFor(() => expect(screen.getByTestId("latest-copy-failure")).toBeDefined())
    expect(screen.getByTestId("latest-copy-failure").textContent).toBe(
      "The latest copy was not updated: the published copy was incomplete or already removed.",
    )
    // The failure never erases the copy that is still there.
    expect(screen.getByText(/^The copy published .+ \(17 rows\)$/)).toBeDefined()
  })

  it("shows the generic sentence with an unknown reason's code", async () => {
    mockWithPublication({
      table: "gold.mart_a_latest",
      holds: null,
      lastRun: {
        status: "failed",
        reason: "a_code_this_console_does_not_know",
        cleanup: null,
        finishedAt: "2026-11-20T11:00:05Z",
      },
    })

    render(<OpenFormatCard assetId="serving.mart_a" />)
    await waitFor(() => expect(screen.getByTestId("latest-copy-failure")).toBeDefined())
    expect(screen.getByTestId("latest-copy-failure").textContent).toBe(
      "The latest copy was not updated. (a_code_this_console_does_not_know)",
    )
  })

  it("renders without the latest-copy block when the publish wrote no copy", async () => {
    // DATA-6 review R6: `latestCopy: null` is a required field with a
    // real meaning (a zero-row publish), not the absence of the field on
    // an older API. The card hides the block for null and shows it for
    // every API build this console ships with.
    mockWithPublication(null)

    render(<OpenFormatCard assetId="serving.mart_a" />)
    await waitFor(() => expect(screen.getByTestId("publication-status")).toBeDefined())
    expect(screen.queryByText("Latest copy only")).toBeNull()
    expect(screen.queryByText("Not written yet")).toBeNull()
    expect(screen.queryByTestId("latest-copy-failure")).toBeNull()
  })

  // ── DATA-66: the sync setting ─────────────────────────────────────────

  it("says how an on_update table syncs", async () => {
    mockSyncCard(publicationBody)
    render(<OpenFormatCard assetId="serving.mart_a" />)
    await waitFor(() =>
      expect(screen.getByTestId("sync-line").textContent).toBe(
        "Synced after each pipeline write, and checked every 15 seconds for other changes.",
      ),
    )
  })

  it("picking Manual only saves at once and says so", async () => {
    const calls = mockSyncCard(publicationBody)
    render(<OpenFormatCard assetId="serving.mart_a" />)
    await screen.findByTestId("sync-setting")
    await chooseSyncMode("Manual only")

    await waitFor(() => {
      const put = calls.find((c) => c.method === "PUT")
      expect(put?.body).toEqual({ enabled: true, syncMode: "manual" })
    })
    await waitFor(() =>
      expect(screen.getByTestId("sync-line").textContent).toBe(
        "Synced only when you press Publish now.",
      ),
    )
  })

  it("picking On every update saves the mode alone", async () => {
    const calls = mockSyncCard({ ...publicationBody, syncMode: "manual" })
    render(<OpenFormatCard assetId="serving.mart_a" />)
    await screen.findByTestId("sync-setting")
    await chooseSyncMode("On every update")
    await waitFor(() => {
      const put = calls.find((c) => c.method === "PUT")
      expect(put?.body).toEqual({ enabled: true, syncMode: "on_update" })
    })
  })

  it("shows the next sync and its zone for a scheduled table", async () => {
    mockSyncCard({
      ...publicationBody,
      syncMode: "scheduled",
      schedule: { kind: "daily", hour: 4, minute: 0 },
      timeZone: "Asia/Jakarta",
      nextSyncAt: "2026-11-28T21:00:00Z",
    })
    render(<OpenFormatCard assetId="serving.mart_a" />)
    await waitFor(() => {
      const line = screen.getByTestId("sync-line").textContent ?? ""
      expect(line).toContain("Next sync:")
      expect(line).toContain("Asia/Jakarta")
    })
  })

  it("saving a daily schedule sends the kind, the time and the zone", async () => {
    const calls = mockSyncCard(publicationBody)
    render(<OpenFormatCard assetId="serving.mart_a" />)
    await screen.findByTestId("sync-setting")
    await chooseSyncMode("On a schedule")

    // D4: prefilled daily 04:00 in the browser's zone.
    const time = (await screen.findByLabelText("Time of day")) as HTMLInputElement
    expect(time.value).toBe("04:00")
    fireEvent.change(time, { target: { value: "06:30" } })
    fireEvent.click(screen.getByRole("button", { name: "Save schedule" }))

    await waitFor(() => {
      const put = calls.find((c) => c.method === "PUT")
      expect(put?.body).toMatchObject({
        enabled: true,
        syncMode: "scheduled",
        schedule: { kind: "daily", hour: 6, minute: 30 },
      })
      expect(typeof (put?.body as Record<string, unknown>).timeZone).toBe("string")
      expect(((put?.body as Record<string, unknown>).timeZone as string).length).toBeGreaterThan(0)
    })
  })

  it("saving an hourly schedule sends the minute and the zone", async () => {
    const calls = mockSyncCard(publicationBody)
    render(<OpenFormatCard assetId="serving.mart_a" />)
    await screen.findByTestId("sync-setting")
    await chooseSyncMode("On a schedule")

    // The frequency picker switches the form from a time to a minute.
    fireEvent.click(screen.getByRole("combobox", { name: "Frequency" }))
    pickOption(await screen.findByRole("option", { name: "Hourly" }))
    const minute = (await screen.findByLabelText("Minute")) as HTMLInputElement
    fireEvent.change(minute, { target: { value: "30" } })
    fireEvent.click(screen.getByRole("button", { name: "Save schedule" }))

    await waitFor(() => {
      const put = calls.find((c) => c.method === "PUT")
      expect(put?.body).toMatchObject({
        syncMode: "scheduled",
        schedule: { kind: "hourly", minute: 30 },
      })
    })
  })

  it("shows the sync choice read-only without canEdit", async () => {
    mockSyncCard({ ...publicationBody, canEdit: false })
    render(<OpenFormatCard assetId="serving.mart_a" />)
    const select = await screen.findByRole("combobox", { name: "Sync mode" })
    expect(
      select.hasAttribute("disabled") ||
        select.hasAttribute("data-disabled") ||
        select.getAttribute("aria-disabled") === "true",
    ).toBe(true)
  })

})

describe("OpenFormatCard — Silver assets (DATA-65)", () => {
  const silverPublication = {
    ...publicationBody,
    zone: "silver",
    mart: "orders_clean",
    latestCopy: {
      table: "silver.orders_clean_latest",
      holds: { copyExportedAt: "2026-10-02T04:00:00Z", rows: 42, replacedAt: "2026-10-02T04:00:01Z" },
      lastRun: { status: "success", reason: null, cleanup: null, finishedAt: "2026-10-02T04:00:01Z" },
    },
  }

  it("renders the card for a silver asset with the silver Iceberg name and copy details", async () => {
    const urls: string[] = []
    global.fetch = mock(async (input: RequestInfo | URL) => {
      const url = typeof input === "string" ? input : input.toString()
      urls.push(url)
      if (url.includes("/consumers")) return jsonResponse(consumersRefusalBody)
      if (url.includes("/publication")) return jsonResponse(silverPublication)
      if (url.includes("/api/gold/export/orders_clean") && !url.includes("/exports")) {
        return jsonResponse({ ...readBackBody, zone: "silver", namespace: "silver", table: "orders_clean" })
      }
      if (url.includes("/api/gold/exports")) return jsonResponse({ mart: "orders_clean", runs: [] })
      throw new Error(`unexpected fetch: ${url}`)
    }) as unknown as typeof fetch

    render(<OpenFormatCard assetId="silver.orders_clean" />)
    await waitFor(() =>
      expect(screen.getByTestId("iceberg-table").textContent).toBe("silver.orders_clean"),
    )
    expect(screen.getByText("silver.orders_clean_latest")).toBeDefined()
    expect(screen.getByText(/The copy published/)).toBeDefined()
    expect(urls.some((u) => u.includes("/api/gold/export/orders_clean/publication?zone=silver"))).toBe(true)
    expect(urls.some((u) => u.includes("/api/gold/exports?mart=orders_clean&zone=silver"))).toBe(true)
  })

  it("toggles publication with ?zone=silver", async () => {
    const calls: { url: string; method?: string }[] = []
    global.fetch = mock(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = typeof input === "string" ? input : input.toString()
      calls.push({ url, method: init?.method })
      if (url.includes("/consumers")) return jsonResponse(consumersRefusalBody)
      if (url.includes("/publication")) {
        return jsonResponse(init?.method === "PUT" ? { ...silverPublication, enabled: true } : silverPublication)
      }
      if (url.includes("/api/gold/export/orders_clean") && !url.includes("/exports")) {
        return jsonResponse({ ...readBackBody, zone: "silver", namespace: "silver" })
      }
      if (url.includes("/api/gold/exports")) return jsonResponse({ mart: "orders_clean", runs: [] })
      throw new Error(`unexpected fetch: ${init?.method} ${url}`)
    }) as unknown as typeof fetch

    render(<OpenFormatCard assetId="silver.orders_clean" />)
    await waitFor(() => expect(screen.getByTestId("iceberg-table")).toBeDefined())
    fireEvent.click(screen.getByTestId("open-format-toggle"))
    await waitFor(() =>
      expect(calls.some((c) => c.method === "PUT" && c.url.includes("?zone=silver") && c.url.includes("/publication"))).toBe(true),
    )
  })

  it("publishes now with ?zone=silver", async () => {
    const posts: string[] = []
    global.fetch = mock(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = typeof input === "string" ? input : input.toString()
      if (url.includes("/consumers")) return jsonResponse(consumersRefusalBody)
      if (url.includes("/publication")) return jsonResponse(silverPublication)
      if (init?.method === "POST" && url.includes("/api/gold/export/orders_clean")) {
        posts.push(url)
        return jsonResponse({
          zone: "silver",
          namespace: "silver",
          table: "orders_clean",
          formatVersion: 2,
          rowsExported: 42,
          snapshotId: { value: 1, measured: true },
          exportedAt: "2026-10-02T04:00:00Z",
          copyExportedAt: null,
          latestCopy: null,
        })
      }
      if (url.includes("/api/gold/export/orders_clean") && !url.includes("/exports")) {
        return jsonResponse({ ...readBackBody, zone: "silver", namespace: "silver" })
      }
      if (url.includes("/api/gold/exports")) return jsonResponse({ mart: "orders_clean", runs: [] })
      throw new Error(`unexpected fetch: ${init?.method} ${url}`)
    }) as unknown as typeof fetch

    render(<OpenFormatCard assetId="silver.orders_clean" />)
    await waitFor(() => expect(screen.getByTestId("iceberg-table")).toBeDefined())
    fireEvent.click(screen.getByRole("button", { name: "Publish now" }))
    await waitFor(() => expect(posts.length).toBe(1))
    expect(posts[0]).toBe("/api/gold/export/orders_clean?zone=silver")
  })
})
