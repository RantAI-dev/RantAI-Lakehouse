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

const publicationBody: Record<string, unknown> = {
  mart: "mart_a",
  enabled: true,
  updatedAt: "2026-10-02T04:00:00Z",
  lastChangedAt: "2026-10-02T03:30:00Z",
  lastExportedAt: "2026-10-02T04:00:00Z",
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

const emptyRunsBody = { mart: "mart_a", runs: [] satisfies GoldExportRun[] }

import type { GoldExportRun } from "@/services/contracts/gold"

describe("OpenFormatCard", () => {
  it("shows Up to date when lastChangedAt is strictly before lastExportedAt", async () => {
    global.fetch = mock(async (input: RequestInfo | URL) => {
      const url = typeof input === "string" ? input : input.toString()
      if (url.includes("/publication")) {
        return jsonResponse({
          ...publicationBody,
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
    }) as unknown as typeof fetch

    render(<OpenFormatCard assetId="serving.mart_a" />)
    await waitFor(() => expect(screen.getByText("Up to date")).toBeDefined())
  })

  it("shows Out of date when lastChangedAt is equal to lastExportedAt", async () => {
    global.fetch = mock(async (input: RequestInfo | URL) => {
      const url = typeof input === "string" ? input : input.toString()
      if (url.includes("/publication")) {
        return jsonResponse({
          ...publicationBody,
          lastChangedAt: "2026-10-02T04:00:00Z",
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
    }) as unknown as typeof fetch

    render(<OpenFormatCard assetId="serving.mart_a" />)
    await waitFor(() => expect(screen.getByText("Out of date")).toBeDefined())
  })

  it("shows Not measured when lastChangedAt is null", async () => {
    global.fetch = mock(async (input: RequestInfo | URL) => {
      const url = typeof input === "string" ? input : input.toString()
      if (url.includes("/publication")) {
        return jsonResponse({
          ...publicationBody,
          lastChangedAt: null,
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
    }) as unknown as typeof fetch

    render(<OpenFormatCard assetId="serving.mart_a" />)
    await waitFor(() => expect(screen.getByText("Not measured")).toBeDefined())
  })

  it("shows Never published when lastExportedAt is null", async () => {
    global.fetch = mock(async (input: RequestInfo | URL) => {
      const url = typeof input === "string" ? input : input.toString()
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
    }) as unknown as typeof fetch

    render(<OpenFormatCard assetId="serving.mart_a" />)
    await waitFor(() => expect(screen.getByText("Never published")).toBeDefined())
  })

  it("disables the switch when canEdit is false and shows an explanation", async () => {
    global.fetch = mock(async (input: RequestInfo | URL) => {
      const url = typeof input === "string" ? input : input.toString()
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
    }) as unknown as typeof fetch

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
    global.fetch = mock(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = typeof input === "string" ? input : input.toString()
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
    }) as unknown as typeof fetch

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
    global.fetch = mock(async (input: RequestInfo | URL) => {
      const url = typeof input === "string" ? input : input.toString()
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
    }) as unknown as typeof fetch

    render(<OpenFormatCard assetId="serving.mart_a" />)
    await waitFor(() => expect(screen.getByText("Never published")).toBeDefined())
    expect(screen.queryByText("Last published")).toBeNull()
  })

  it("shows last published time and snapshot plus off notice when switch is off and mart was previously published", async () => {
    // PR slice D review D-B3: with the switch off and a prior publish
    // on record, the card still shows last published time, snapshot,
    // and the "Publishing is off" note.
    global.fetch = mock(async (input: RequestInfo | URL) => {
      const url = typeof input === "string" ? input : input.toString()
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
    }) as unknown as typeof fetch

    render(<OpenFormatCard assetId="serving.mart_a" />)
    await waitFor(() => expect(screen.getByText("Up to date")).toBeDefined())
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
    global.fetch = mock(async (input: RequestInfo | URL) => {
      const url = typeof input === "string" ? input : input.toString()
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
    }) as unknown as typeof fetch

    render(<OpenFormatCard assetId="serving.mart_a" />)
    await waitFor(() => expect(screen.getByText("ClickHouse connection refused (111)")).toBeDefined())
  })
})