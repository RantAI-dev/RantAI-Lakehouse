import { render, screen, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"
import { ConnectorCreatePage } from "./connector-create-page"

const originalFetch = global.fetch

afterEach(() => {
  global.fetch = originalFetch
})

describe("ConnectorCreatePage", () => {
  it("renders connector types from listTypes, disabling unsupported ones with their reason", async () => {
    global.fetch = mock(async () =>
      new Response(
        JSON.stringify([
          { name: "PostgreSQL", adapter: "sql", supported: true, docsUrl: null },
          { name: "Kafka", adapter: null, supported: false, docsUrl: null },
        ]),
        { status: 200, headers: { "Content-Type": "application/json" } }
      )
    ) as unknown as typeof fetch

    render(<ConnectorCreatePage />)
    await waitFor(() => expect(screen.getByText("Kafka (not yet supported)")).toBeDefined())
    const kafkaOption = screen.getByRole("option", { name: /Kafka/ }) as HTMLOptionElement
    expect(kafkaOption.disabled).toBe(true)
  })
})
