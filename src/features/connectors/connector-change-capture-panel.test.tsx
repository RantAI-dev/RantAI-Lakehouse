import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"
import { ConnectorChangeCapturePanel } from "./connector-change-capture-panel"

const originalFetch = global.fetch

afterEach(() => {
  global.fetch = originalFetch
  cleanup()
})

function json(body: unknown, status = 200) {
  return new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } })
}

type Call = { url: string; method: string; body: unknown }

const LAG = {
  tables: [
    { table: "public.orders", status: "behind", seconds: 1_200 },
    { table: "public.items", status: "up_to_date" },
    { table: "public.users", status: "not_known_yet" },
    { table: "public.logs", status: "not_known_since", since: "2026-10-30T10:00:00Z" },
  ],
  worst: { table: "public.orders", seconds: 1_200 },
  limitMinutes: 15,
  alertOpen: true,
}

function stub(capture: unknown): Call[] {
  const calls: Call[] = []
  global.fetch = mock(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input).replace(/^https?:\/\/[^/]+/, "")
    const method = init?.method ?? "GET"
    calls.push({ url, method, body: typeof init?.body === "string" ? JSON.parse(init.body) : undefined })
    if (url.endsWith("/change-capture")) return json(capture)
    if (url.endsWith("/pause")) return json({ paused: true })
    if (url.endsWith("/resume")) return json({ resumed: true })
    if (url.endsWith("/lag-limit")) return json({ minutes: 30 })
    throw new Error(`unexpected fetch: ${method} ${url}`)
  }) as unknown as typeof fetch
  return calls
}

describe("ConnectorChangeCapturePanel", () => {
  it("words the state and each table's lag as the API sent them, and says an alert is open", async () => {
    stub({ supported: true, state: "running", lag: LAG })
    render(<ConnectorChangeCapturePanel connectorId="conn-a" pausedReason={null} />)
    expect(await screen.findByText("Behind by 20m")).toBeDefined()
    expect(screen.getByText("Running")).toBeDefined()
    expect(screen.getByText("Up to date")).toBeDefined()
    expect(screen.getByText("Not known yet")).toBeDefined()
    expect(screen.getByText(/^Not known since /)).toBeDefined()
    expect(screen.getByText(/An alert is open/)).toBeDefined()
  })

  it("says so, and shows no lag, when the deployment cannot capture changes", async () => {
    stub({ supported: false, reason: "Change capture is not set up on this deployment." })
    render(<ConnectorChangeCapturePanel connectorId="conn-a" pausedReason={null} />)
    expect(await screen.findByText("Change capture is not set up on this deployment.")).toBeDefined()
    expect(screen.queryByLabelText("Alert after (minutes)")).toBeNull()
  })

  it("pauses on request", async () => {
    const calls = stub({ supported: true, state: "running", lag: LAG })
    render(<ConnectorChangeCapturePanel connectorId="conn-a" pausedReason={null} />)
    fireEvent.click(await screen.findByRole("button", { name: "Pause" }))
    await waitFor(() => expect(calls.some((c) => c.method === "POST" && c.url.endsWith("/pause"))).toBe(true))
  })

  it("offers Resume, not Pause, for a pause a person set, and for no other reason", async () => {
    const calls = stub({ supported: true, state: "paused", lag: LAG })
    const { rerender } = render(
      <ConnectorChangeCapturePanel connectorId="conn-a" pausedReason="Paused by a person" />
    )
    expect(screen.queryByRole("button", { name: "Pause" })).toBeNull()
    fireEvent.click(await screen.findByRole("button", { name: "Resume" }))
    await waitFor(() => expect(calls.some((c) => c.method === "POST" && c.url.endsWith("/resume"))).toBe(true))
    rerender(
      <ConnectorChangeCapturePanel connectorId="conn-a" pausedReason="Paused by the schema-change policy until a change is approved" />
    )
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 0))
    })
    expect(screen.queryByRole("button", { name: "Resume" })).toBeNull()
  })

  it("saves the lag limit as a whole number of minutes and refuses anything else", async () => {
    const calls = stub({ supported: true, state: "running", lag: LAG })
    render(<ConnectorChangeCapturePanel connectorId="conn-a" pausedReason={null} />)
    const input = (await screen.findByLabelText("Alert after (minutes)")) as HTMLInputElement
    const save = screen.getByRole("button", { name: "Save" }) as HTMLButtonElement
    expect(save.disabled).toBe(true)
    fireEvent.change(input, { target: { value: "0" } })
    expect(save.disabled).toBe(true)
    expect(screen.getByText("Use a whole number of minutes from 1 to 1440.")).toBeDefined()
    fireEvent.change(input, { target: { value: "30" } })
    expect(save.disabled).toBe(false)
    fireEvent.click(save)
    await waitFor(() =>
      expect(calls.some((c) => c.method === "PUT" && c.url.endsWith("/lag-limit"))).toBe(true)
    )
    expect(calls.find((c) => c.method === "PUT")?.body).toEqual({ minutes: 30 })
  })

  it("keeps Resume for a paused connector when the state cannot be read (Slice 4 review SHOULD-FIX 5)", async () => {
    const calls: Call[] = []
    global.fetch = mock(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = String(input).replace(/^https?:\/\/[^/]+/, "")
      calls.push({ url, method: init?.method ?? "GET", body: undefined })
      if (url.endsWith("/change-capture")) return json({ error: "the store is unavailable" }, 503)
      if (url.endsWith("/resume")) return json({ resumed: true })
      throw new Error(`unexpected fetch: ${url}`)
    }) as unknown as typeof fetch
    render(<ConnectorChangeCapturePanel connectorId="conn-a" pausedReason="Paused by a person" />)
    fireEvent.click(await screen.findByRole("button", { name: "Resume" }))
    await waitFor(() => expect(calls.some((c) => c.method === "POST" && c.url.endsWith("/resume"))).toBe(true))
  })
})
