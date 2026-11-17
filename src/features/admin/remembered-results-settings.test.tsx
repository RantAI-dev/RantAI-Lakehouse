import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"
import { RememberedResultsForm } from "./remembered-results-settings"

const originalFetch = global.fetch
afterEach(() => {
  cleanup()
  global.fetch = originalFetch
})

const json = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } })
const SETTINGS = { timeZone: "UTC", weekStart: "monday", saved: true, resultCache: { mode: "minutes", minutes: 30 } }

function mockApi(putStatus = 200) {
  const puts: string[] = []
  global.fetch = mock(async (input: RequestInfo | URL, init?: RequestInit) => {
    if (!String(input).includes("/api/settings/reporting")) return json({})
    if ((init?.method ?? "GET").toUpperCase() === "PUT") {
      puts.push(String(init?.body))
      return putStatus === 200 ? json({ ...SETTINGS, resultCache: { mode: "minutes", minutes: 45 } }) : json({ error: "resultCache must be {…}" }, putStatus)
    }
    return json(SETTINGS)
  }) as unknown as typeof fetch
  return puts
}

describe("Admin > Settings > Remembered results (BI-4)", () => {
  it("reads the install's rule and keeps Save off until it changes", async () => {
    mockApi()
    render(<RememberedResultsForm canWrite />)
    expect(((await screen.findByLabelText("Remember dashboard results: minutes")) as HTMLInputElement).value).toBe("30")
    expect((screen.getByRole("button", { name: "Save" }) as HTMLButtonElement).disabled).toBe(true)
  })

  it("refuses minutes out of range in a sentence and keeps Save off", async () => {
    mockApi()
    render(<RememberedResultsForm canWrite />)
    await screen.findByLabelText("Remember dashboard results")
    const minutes = (await screen.findByLabelText("Remember dashboard results: minutes")) as HTMLInputElement
    expect(minutes.value).toBe("30")
    fireEvent.change(minutes, { target: { value: "0" } })
    expect((await screen.findByRole("alert")).textContent).toBe("Remember for between 1 minute and 24 hours (1 to 1440 minutes).")
    expect((screen.getByRole("button", { name: "Save" }) as HTMLButtonElement).disabled).toBe(true)
  })

  it("sends the zone, the first day and only the rule", async () => {
    const puts = mockApi()
    render(<RememberedResultsForm canWrite />)
    fireEvent.change(await screen.findByLabelText("Remember dashboard results: minutes"), { target: { value: "45" } })
    fireEvent.click(screen.getByRole("button", { name: "Save" }))
    await waitFor(() => expect(puts).toHaveLength(1))
    expect(JSON.parse(puts[0] ?? "{}")).toEqual({ timeZone: "UTC", weekStart: "monday", resultCache: { mode: "minutes", minutes: 45 } })
  })

  it("is read-only without settings:write", async () => {
    mockApi()
    render(<RememberedResultsForm canWrite={false} />)
    await screen.findByLabelText("Remember dashboard results")
    expect(screen.getByText("Only an administrator can change this.")).toBeTruthy()
    expect(screen.queryByRole("button", { name: "Save" })).toBeNull()
  })
})
