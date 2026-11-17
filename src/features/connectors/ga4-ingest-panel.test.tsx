import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"
import { ConnectorIngestPanel } from "./connector-ingest-panel"

/** Slice 3 review BLOCKER 9 inside the connector page: a GA4 connector gets the report editor, saves `dial.objects`. */

const originalFetch = global.fetch
afterEach(() => {
  global.fetch = originalFetch
  cleanup()
})

const json = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } })

const SPEC = {
  adapter: "saas",
  ingestMode: "batch",
  dial: { vendor: "ga4", account: "123", auth: { type: "integration" } },
  sourceObjects: [] as unknown[],
  scheduleCron: null,
  secretRefs: { primary: "file:/run/secrets/connector_managed_x_password", secondary: null },
  nextRunAt: null,
}

function stub(spec: unknown = SPEC) {
  const puts: unknown[] = []
  global.fetch = mock(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input)
    const method = init?.method ?? "GET"
    const body = init?.body ? JSON.parse(init.body as string) : undefined
    if (url.includes("/ingest-spec") && method === "GET") return json(spec)
    if (url.includes("/ingest-spec") && method === "PUT") {
      puts.push(body)
      return json({ ...(spec as object), ...body, nextRunAt: null })
    }
    if (url.endsWith("/runs")) return json({ runs: [] })
    if (url.endsWith("/discovery/list") && method === "POST") {
      return json({
        kind: "list",
        state: "done",
        objects: body?.schema === "fields"
          ? [{ name: "country", kind: "dimension" }, { name: "sessions", kind: "metric" }]
          : [],
      })
    }
    throw new Error(`unexpected fetch: ${method} ${url}`)
  }) as unknown as typeof fetch
  return puts
}

describe("a Google Analytics 4 connector's tables card", () => {
  it("offers the report editor and no 'Add by name'", async () => {
    stub()
    render(<ConnectorIngestPanel connectorId="c1" connectorName="analytics" />)
    expect(await screen.findByRole("button", { name: "Load dimensions and metrics" })).toBeDefined()
    expect(screen.queryByLabelText("Add by name")).toBeNull()
  })

  it("saves a defined report into the dial's objects and as an incremental object keyed by its dimensions", async () => {
    const puts = stub()
    render(<ConnectorIngestPanel connectorId="c1" connectorName="analytics" />)
    fireEvent.click(await screen.findByRole("button", { name: "Load dimensions and metrics" }))
    fireEvent.click(await screen.findByLabelText("country"))
    fireEvent.click(screen.getByLabelText("sessions"))
    fireEvent.change(screen.getByLabelText("Report name"), { target: { value: "traffic" } })
    fireEvent.click(screen.getByRole("button", { name: "Add report" }))
    const save = screen.getByRole("button", { name: "Save tables and schedule" }) as HTMLButtonElement
    await waitFor(() => expect(save.disabled).toBe(false))
    fireEvent.click(save)
    await waitFor(() => expect(puts.length).toBe(1))
    const sent = puts[0] as { dial: { objects: string[] }; sourceObjects: Record<string, unknown>[] }
    expect(sent.dial.objects).toEqual(["traffic = country, date | sessions"])
    expect(sent.sourceObjects).toMatchObject([
      { name: "traffic = country, date | sessions", loadMode: "incremental_dedup", incrementalKey: "date", primaryKey: ["country", "date"], target: "analytics_traffic" },
    ])
  })

  it("removing a saved report removes it from the dial and the tables, and can be saved", async () => {
    const report = "traffic = country, date | sessions"
    const puts = stub({
      ...SPEC,
      dial: { ...SPEC.dial, objects: [report] },
      sourceObjects: [{ name: report, target: "analytics_traffic", loadMode: "incremental_dedup", incrementalKey: "date", primaryKey: ["country", "date"] }],
    })
    render(<ConnectorIngestPanel connectorId="c1" connectorName="analytics" />)
    fireEvent.click(await screen.findByRole("button", { name: "Remove" }))
    const save = screen.getByRole("button", { name: "Save tables and schedule" }) as HTMLButtonElement
    await waitFor(() => expect(save.disabled).toBe(false))
    fireEvent.click(save)
    await waitFor(() => expect(puts.length).toBe(1))
    expect(puts[0]).toMatchObject({ dial: { objects: [] }, sourceObjects: [] })
  })
})
