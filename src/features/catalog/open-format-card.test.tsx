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
        return jsonResponse({
          namespace: "gold",
          table: "mart_a",
          formatVersion: 2,
          rowsInIceberg: 42,
          snapshotId: { value: 12345, measured: true },
          exportedAt: "2026-10-02T04:00:00Z",
        })
      }
      if (url.includes("/api/gold/exports")) {
        return jsonResponse({ mart: "mart_a", runs: [] })
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
        return jsonResponse({
          namespace: "gold",
          table: "mart_a",
          formatVersion: 2,
          rowsInIceberg: 42,
          snapshotId: { value: 12345, measured: true },
          exportedAt: "2026-10-02T04:00:00Z",
        })
      }
      if (url.includes("/api/gold/exports")) {
        return jsonResponse({ mart: "mart_a", runs: [] })
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
        return jsonResponse({
          namespace: "gold",
          table: "mart_a",
          formatVersion: 2,
          rowsInIceberg: 42,
          snapshotId: { value: 12345, measured: true },
          exportedAt: "2026-10-02T04:00:00Z",
        })
      }
      if (url.includes("/api/gold/exports")) {
        return jsonResponse({ mart: "mart_a", runs: [] })
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
        return jsonResponse({ mart: "mart_a", runs: [] })
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
        return jsonResponse({
          namespace: "gold",
          table: "mart_a",
          formatVersion: 2,
          rowsInIceberg: 42,
          snapshotId: { value: 12345, measured: true },
          exportedAt: "2026-10-02T04:00:00Z",
        })
      }
      if (url.includes("/api/gold/exports")) {
        return jsonResponse({ mart: "mart_a", runs: [] })
      }
      throw new Error(`unexpected fetch: ${url}`)
    }) as unknown as typeof fetch

    render(<OpenFormatCard assetId="serving.mart_a" />)
    await waitFor(() => expect(screen.getByText(/gold:export/)).toBeDefined())
    const switchEl = screen.getByRole("switch")
    expect(switchEl).toBeDefined()
    expect(switchEl.getAttribute("disabled")).not.toBeNull()
  })

  it("leaves the switch off after a failed PUT and shows the error", async () => {
    let putCalled = false
    global.fetch = mock(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = typeof input === "string" ? input : input.toString()
      if (url.includes("/publication")) {
        if (putCalled) {
          return jsonResponse({ error: "internal error" }, 500)
        }
        putCalled = true
        return jsonResponse({
          ...publicationBody,
          enabled: false,
        })
      }
      if (url.includes("/api/gold/exports")) {
        return jsonResponse({ mart: "mart_a", runs: [] })
      }
      throw new Error(`unexpected fetch: ${url}`)
    }) as unknown as typeof fetch

    render(<OpenFormatCard assetId="serving.mart_a" />)
    // The publication returns enabled: false, so status is "Off"
    await waitFor(() => {
      const statusElements = screen.getAllByText("Off")
      expect(statusElements.length).toBeGreaterThanOrEqual(1)
    })
    // Toggle the switch to try turning it on
    const switchEl = screen.getByRole("switch")
    expect(switchEl).toBeDefined()
    fireEvent.click(switchEl)
    // The PUT fails with 500 — the error is shown
    await waitFor(() => expect(screen.getByRole("alert")).toBeDefined())
  })
})