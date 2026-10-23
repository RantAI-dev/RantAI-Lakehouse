import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"
import { ConnectorIngestPanel } from "./connector-ingest-panel"

/** `SRC-3` slice 6 inside the connector page: a Workday connector names its reports by owner and name, saved into `dial.objects`. */

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
  dial: { vendor: "workday", account: "wd2.workday.com/acme", auth: { type: "integration" } },
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
    if (url.endsWith("/discovery/list") && method === "POST") return json({ kind: "list", state: "done", objects: [] })
    throw new Error(`unexpected fetch: ${method} ${url}`)
  }) as unknown as typeof fetch
  return puts
}

describe("a Workday connector's tables card", () => {
  it("offers owner and name fields for a report and no 'Add by name'", async () => {
    stub()
    render(<ConnectorIngestPanel connectorId="c1" connectorName="hr" />)
    expect(await screen.findByLabelText("Report owner")).toBeDefined()
    expect(screen.getByLabelText("Report name")).toBeDefined()
    expect(screen.queryByLabelText("Add by name")).toBeNull()
  })

  it("refuses a report name that could leave the request path, in words", async () => {
    stub()
    render(<ConnectorIngestPanel connectorId="c1" connectorName="hr" />)
    fireEvent.change(await screen.findByLabelText("Report owner"), { target: { value: "isu_owner" } })
    fireEvent.change(screen.getByLabelText("Report name"), { target: { value: "../x" } })
    expect(screen.getByText(/Write the report name as it is in its address/)).toBeDefined()
    expect((screen.getByRole("button", { name: "Add report" }) as HTMLButtonElement).disabled).toBe(true)
  })

  it("saves a named report into the dial's objects and as an object that replaces its table", async () => {
    const puts = stub()
    render(<ConnectorIngestPanel connectorId="c1" connectorName="hr" />)
    fireEvent.change(await screen.findByLabelText("Report owner"), { target: { value: "isu_owner" } })
    fireEvent.change(screen.getByLabelText("Report name"), { target: { value: "Headcount_Report" } })
    fireEvent.click(screen.getByRole("button", { name: "Add report" }))
    const save = screen.getByRole("button", { name: "Save tables and schedule" }) as HTMLButtonElement
    await waitFor(() => expect(save.disabled).toBe(false))
    fireEvent.click(save)
    await waitFor(() => expect(puts.length).toBe(1))
    const sent = puts[0] as { dial: { objects: string[] }; sourceObjects: Record<string, unknown>[] }
    expect(sent.dial.objects).toEqual(["isu_owner/Headcount_Report"])
    expect(sent.sourceObjects).toMatchObject([{ name: "isu_owner/Headcount_Report", loadMode: "replace" }])
    expect(sent.sourceObjects[0]).not.toHaveProperty("incrementalKey")
  })

  it("removing a saved report removes it from the dial and the tables", async () => {
    const puts = stub({
      ...SPEC,
      dial: { ...SPEC.dial, objects: ["isu_owner/Headcount_Report"] },
      sourceObjects: [{ name: "isu_owner/Headcount_Report", target: "hr_headcount_report", loadMode: "replace" }],
    })
    render(<ConnectorIngestPanel connectorId="c1" connectorName="hr" />)
    fireEvent.click(await screen.findByRole("button", { name: "Remove" }))
    const save = screen.getByRole("button", { name: "Save tables and schedule" }) as HTMLButtonElement
    await waitFor(() => expect(save.disabled).toBe(false))
    fireEvent.click(save)
    await waitFor(() => expect(puts.length).toBe(1))
    expect(puts[0]).toMatchObject({ dial: { objects: [] }, sourceObjects: [] })
  })
})
