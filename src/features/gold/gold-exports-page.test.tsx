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

describe("GoldExportsPage", () => {
  it("renders a mart's last export and an honest consumers refusal, never a fabricated count", async () => {
    global.fetch = mock(async (input: RequestInfo | URL) => {
      const url = typeof input === "string" ? input : input.toString()
      if (url.includes("/api/dashboard/fields")) {
        return jsonResponse({ marts: [{ name: "sales", rows: 42 }] })
      }
      if (url.includes("/consumers")) {
        return jsonResponse({
          mart: "sales",
          consumers: null,
          supported: false,
          reason: "Trino query-history correlation for Gold marts is not implemented",
        })
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

    render(<GoldExportsPage />)

    await waitFor(() => expect(screen.getByText("sales")).toBeDefined())
    await waitFor(() => expect(screen.getByText("Never exported")).toBeDefined())
    // A mart with no measured consumer count renders the honest refusal
    // text, never "0" (which would read as a real, measured zero).
    await waitFor(() => expect(screen.getByText("Not measured")).toBeDefined())
  })
})
