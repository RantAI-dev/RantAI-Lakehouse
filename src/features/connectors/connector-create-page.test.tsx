import { fireEvent, render, screen, waitFor, within } from "@testing-library/react"
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

  /**
   * ADR 0002 Addendum 3: the wizard sends `credential: { source, primary }`
   * (never a free-text `secretRef`), and after a successful create shows
   * the server-derived names verbatim from the response — the operator's
   * only chance to see them.
   */
  it("submits a credential spec (not a secretRef) and shows the derived names the response returns", async () => {
    const requests: { url: string; method: string; body: unknown }[] = []
    global.fetch = mock(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = String(input)
      const method = init?.method ?? "GET"
      const body = init?.body ? JSON.parse(init.body as string) : undefined
      requests.push({ url, method, body })

      if (url.endsWith("/api/connectors/types")) {
        return new Response(
          JSON.stringify([{ name: "PostgreSQL", adapter: "sql", supported: true, docsUrl: null }]),
          { status: 200, headers: { "Content-Type": "application/json" } }
        )
      }
      if (url.endsWith("/api/connectors") && method === "POST") {
        return new Response(
          JSON.stringify({
            id: "conn-orders-k3x9",
            name: "orders cdc",
            type: "PostgreSQL",
            direction: "source",
            health: "unknown",
            environment: "production",
            tenant: "Meridian Group",
            lastTestAt: null,
            lastActivityAt: null,
            capabilities: [],
            owner: "Current user",
            credential: {
              primary: "env:CONNECTOR_CONN_ORDERS_K3X9_PASSWORD",
              secondary: null,
            },
          }),
          { status: 201, headers: { "Content-Type": "application/json" } }
        )
      }
      if (url.includes("/test")) {
        return new Response(
          JSON.stringify({ ok: true, supported: true, latencyMs: 12, message: "ok", testedAt: "2026-01-01T00:00:00.000Z" }),
          { status: 200, headers: { "Content-Type": "application/json" } }
        )
      }
      if (url.includes("/ingest-spec")) {
        return new Response(
          JSON.stringify({
            adapter: "sql",
            ingestMode: "batch",
            dial: {},
            sourceObjects: [],
            scheduleCron: null,
            secretRefs: { primary: "env:CONNECTOR_CONN_ORDERS_K3X9_PASSWORD", secondary: null },
          }),
          { status: 200, headers: { "Content-Type": "application/json" } }
        )
      }
      throw new Error(`unexpected fetch: ${method} ${url}`);
    }) as unknown as typeof fetch

    const { container } = render(<ConnectorCreatePage />)
    const within_ = within(container)

    await waitFor(() => expect(within_.getByDisplayValue("PostgreSQL")).toBeDefined())
    fireEvent.change(within_.getByPlaceholderText("postgres core CDC"), {
      target: { value: "orders cdc" },
    })

    // Step 0 -> 1 (connection/credential).
    fireEvent.click(within_.getByRole("button", { name: "Next" }))
    // Step 1 -> 2 (scope): defaults (source=env, primary=password) already
    // satisfy `canProceed`, no interaction needed.
    fireEvent.click(within_.getByRole("button", { name: "Next" }))
    // Step 2 requires tenant/residency filled in. The `Field` wrapper does
    // not associate its `<Label>` with the input (no `htmlFor`/`id`), so
    // this selects by role/order (Environment, Tenant, Residency) rather
    // than `getByLabelText`, which would find nothing.
    const [, tenantInput, residencyInput] = within_.getAllByRole("textbox") as HTMLInputElement[]
    fireEvent.change(tenantInput, { target: { value: "Meridian Group" } })
    fireEvent.change(residencyInput, { target: { value: "in-region" } })
    fireEvent.click(within_.getByRole("button", { name: "Next" }))
    // Step 3: submit.
    fireEvent.click(within_.getByRole("button", { name: "Create connector" }))

    await waitFor(() =>
      expect(within_.getByText("env:CONNECTOR_CONN_ORDERS_K3X9_PASSWORD")).toBeDefined()
    )

    const createRequest = requests.find((r) => r.url.endsWith("/api/connectors") && r.method === "POST");
    expect(createRequest).toBeDefined()
    const createBody = createRequest?.body as Record<string, unknown>
    expect(createBody.secretRef).toBeUndefined()
    expect(createBody.credential).toEqual({ source: "env", primary: "password" })
  })
})
