import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"
import { ConnectorIngestPanel } from "./connector-ingest-panel"
import { SharepointSitePicker } from "./sharepoint-site-picker"

/**
 * Slice 5 review BLOCKER 14: a SharePoint connector lets the person pick sites (found through discovery), shows
 * the lists as "Site / List", and names the Bronze table from the label while the source object keeps the ids.
 */

const originalFetch = global.fetch
afterEach(() => {
  global.fetch = originalFetch
  cleanup()
})

const json = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } })

const SITE = "acme.sharepoint.com,11111111-1111-4111-8111-111111111111,22222222-2222-4222-8222-222222222222"
const LIST = "55555555-5555-4555-8555-555555555555"
const SITES = [
  { name: SITE, kind: "site", label: "Team Site" },
  { name: "acme.sharepoint.com,33333333-3333-4333-8333-333333333333,44444444-4444-4444-8444-444444444444", kind: "site", label: "HR" },
]
const LISTS = [{ name: `${SITE}/${LIST}`, kind: "object", load: "incremental", label: "Team Site / Tasks" }]

const SPEC = {
  adapter: "saas",
  ingestMode: "batch",
  dial: { vendor: "sharepoint", account: "acme/00000000-0000-0000-0000-000000000001", auth: { type: "integration" } },
  sourceObjects: [] as unknown[],
  scheduleCron: null,
  secretRefs: { primary: "file:/run/secrets/connector_managed_x_password", secondary: null },
  nextRunAt: null,
}

function stub(spec: unknown = SPEC) {
  const puts: unknown[] = []
  const lists: unknown[] = []
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
      lists.push(body)
      return json({ kind: "list", state: "done", objects: body?.schema === "fields" ? SITES : LISTS })
    }
    throw new Error(`unexpected fetch: ${method} ${url}`)
  }) as unknown as typeof fetch
  return { puts, lists }
}

describe("SharepointSitePicker", () => {
  it("reads the sites through discovery and shows their titles, keeping the id as the pick", async () => {
    const { lists } = stub()
    const picks: string[] = []
    render(<SharepointSitePicker connectorId="c1" picked={[]} savedPicked={[]} onToggle={(id) => picks.push(id)} pollMs={5} />)
    expect(screen.queryByLabelText("Team Site")).toBeNull()
    fireEvent.click(screen.getByRole("button", { name: "Find sites" }))
    fireEvent.click(await screen.findByLabelText("Team Site"))
    expect(lists).toEqual([{ schema: "fields" }])
    expect(picks).toEqual([SITE])
    expect(screen.getByTestId("sites-picked").textContent).toBe("No site picked")
  })

  it("filters by title, shows a picked site that is not in the list by its id, and says picks need saving", async () => {
    stub()
    const gone = "acme.sharepoint.com,99999999-9999-4999-8999-999999999999,88888888-8888-4888-8888-888888888888"
    render(<SharepointSitePicker connectorId="c1" picked={[gone]} savedPicked={[]} onToggle={() => {}} pollMs={5} />)
    expect(screen.getByLabelText(gone)).toBeDefined()
    expect(screen.getByTestId("sites-unsaved")).toBeDefined()
    fireEvent.click(screen.getByRole("button", { name: "Find sites" }))
    await screen.findByLabelText("Team Site")
    fireEvent.change(screen.getByLabelText("Filter sites"), { target: { value: "hr" } })
    expect(screen.queryByLabelText("Team Site")).toBeNull()
    expect(screen.getByLabelText("HR")).toBeDefined()
  })
})

describe("a SharePoint connector's tables card", () => {
  it("offers the site picker and saves the picked site ids into the dial's objects", async () => {
    const { puts } = stub()
    render(<ConnectorIngestPanel connectorId="c1" connectorName="intranet" />)
    fireEvent.click(await screen.findByRole("button", { name: "Find sites" }))
    fireEvent.click(await screen.findByLabelText("Team Site"))
    const save = screen.getByRole("button", { name: "Save tables and schedule" }) as HTMLButtonElement
    await waitFor(() => expect(save.disabled).toBe(false))
    fireEvent.click(save)
    await waitFor(() => expect(puts.length).toBe(1))
    expect(puts[0]).toMatchObject({ dial: { vendor: "sharepoint", objects: [SITE] }, sourceObjects: [] })
  })

  it("shows the list as 'Site / List', names the table from the label and keeps the ids as the object's key", async () => {
    const { puts } = stub({ ...SPEC, dial: { ...SPEC.dial, objects: [SITE] } })
    render(<ConnectorIngestPanel connectorId="c1" connectorName="intranet" />)
    fireEvent.click(await screen.findByRole("button", { name: "Find objects" }))
    fireEvent.click(await screen.findByLabelText(/Team Site \/ Tasks/))
    expect(screen.getByLabelText(/Bronze table for/)).toBeDefined()
    expect((screen.getByLabelText(/Bronze table for/) as HTMLInputElement).value).toBe("intranet_team_site_tasks")
    const save = screen.getByRole("button", { name: "Save tables and schedule" }) as HTMLButtonElement
    await waitFor(() => expect(save.disabled).toBe(false))
    fireEvent.click(save)
    await waitFor(() => expect(puts.length).toBe(1))
    expect(puts[0]).toMatchObject({ sourceObjects: [{ name: `${SITE}/${LIST}`, target: "intranet_team_site_tasks" }] })
  })
})
