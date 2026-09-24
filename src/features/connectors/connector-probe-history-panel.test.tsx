import { render, waitFor, within } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"
import { ConnectorProbeHistoryPanel } from "./connector-probe-history-panel"

const originalFetch = global.fetch

afterEach(() => {
  global.fetch = originalFetch
})

function respondWith(body: unknown) {
  global.fetch = mock(async () =>
    new Response(JSON.stringify(body), {
      status: 200,
      headers: { "Content-Type": "application/json" },
    })
  ) as unknown as typeof fetch
}

describe("ConnectorProbeHistoryPanel", () => {
  it("renders each recorded probe, showing an unmeasured latency as a dash rather than zero", async () => {
    respondWith({
      results: [
        { testedAt: "2026-09-11T00:00:00.000Z", ok: true, latencyMs: 42, message: "connection succeeded" },
        { testedAt: "2026-09-10T00:00:00.000Z", ok: false, latencyMs: null, message: "connection refused" },
      ],
    })

    // Queries are scoped to this render: the full suite shares one DOM, and
    // a page-wide query would match other files' leftovers.
    const view = within(render(<ConnectorProbeHistoryPanel connectorId="conn-x" />).container)
    await waitFor(() => expect(view.getByText(/connection succeeded/)).toBeDefined())
    expect(view.getByText(/connection refused/)).toBeDefined()
    expect(view.getByText(/42 ms/)).toBeDefined()
    expect(view.getByText(/^— ·/)).toBeDefined()
    expect(view.queryByText(/0 ms/)).toBeNull()
  })

  it("says the connector has not been tested when no probe is recorded, never that it is healthy", async () => {
    respondWith({ results: [] })

    const view = within(render(<ConnectorProbeHistoryPanel connectorId="conn-x" />).container)
    await waitFor(() => expect(view.getByTestId("probe-history-empty")).toBeDefined())
    expect(view.queryByText(/healthy|no issues|passed/i)).toBeNull()
  })
})
