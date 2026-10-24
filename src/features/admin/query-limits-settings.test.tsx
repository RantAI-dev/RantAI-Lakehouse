import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"
import { QueryLimitsForm } from "./query-limits-settings"

const originalFetch = global.fetch
afterEach(() => {
  cleanup()
  global.fetch = originalFetch
})

const json = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } })

const LIMITS = {
  install: { timeoutSeconds: 60, chartRows: 2000, mapPoints: 5000, studioRows: 2000 },
  roles: [{ roleId: "r1", timeoutSeconds: 300 }],
  effective: { timeoutSeconds: 300, chartRows: 2000, mapPoints: 5000, studioRows: 2000, source: { kind: "role", name: "Analyst" } },
}

type Call = { method: string; body: string | null }
function mockApi(putStatus = 200): Call[] {
  const calls: Call[] = []
  global.fetch = mock(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input)
    if (url.includes("/api/identity/roles")) {
      return json([
        { id: "r1", name: "Analyst", members: 1, permissions: "", description: "" },
        { id: "r2", name: "Viewer", members: 1, permissions: "", description: "" },
      ])
    }
    if (!url.includes("/api/settings/limits")) return json({})
    const method = (init?.method ?? "GET").toUpperCase()
    calls.push({ method, body: typeof init?.body === "string" ? init.body : null })
    if (method === "PUT") return putStatus === 200 ? json(LIMITS) : json({ error: "Time limit out of range." }, putStatus)
    return json(LIMITS)
  }) as unknown as typeof fetch
  return calls
}

describe("Admin > Settings > Query limits (BI-5)", () => {
  it("shows the install's values, the role rows and the limits that apply to the viewer", async () => {
    mockApi()
    render(<QueryLimitsForm canWrite />)
    expect(((await screen.findByLabelText(/Time limit \(seconds\)/)) as HTMLInputElement).value).toBe("60")
    expect((screen.getByLabelText(/Rows a chart reads/) as HTMLInputElement).value).toBe("2000")
    expect((screen.getByLabelText("Time limit for Analyst (seconds)") as HTMLInputElement).value).toBe("300")
    expect(screen.getByText(/For you: 300 seconds to run/).textContent).toContain("the limit for Analyst")
  })

  it("refuses a value outside its range in a sentence and keeps Save off", async () => {
    mockApi()
    render(<QueryLimitsForm canWrite />)
    const time = (await screen.findByLabelText(/Time limit \(seconds\)/)) as HTMLInputElement
    fireEvent.change(time, { target: { value: "5" } })
    expect((await screen.findByRole("alert")).textContent).toBe("Time limit: a whole number from 10 to 1,800 seconds.")
    expect((screen.getByRole("button", { name: "Save limits" }) as HTMLButtonElement).disabled).toBe(true)
  })

  it("sends the install's values and the role rows as numbers", async () => {
    const calls = mockApi()
    render(<QueryLimitsForm canWrite />)
    const time = (await screen.findByLabelText(/Time limit \(seconds\)/)) as HTMLInputElement
    fireEvent.change(time, { target: { value: "120" } })
    fireEvent.change(screen.getByLabelText("Time limit for Analyst (seconds)"), { target: { value: "600" } })
    fireEvent.click(screen.getByRole("button", { name: "Save limits" }))
    await waitFor(() => expect(calls.some((c) => c.method === "PUT")).toBe(true))
    expect(JSON.parse(calls.find((c) => c.method === "PUT")?.body ?? "{}")).toEqual({
      install: { timeoutSeconds: 120, chartRows: 2000, mapPoints: 5000, studioRows: 2000 },
      roles: [{ roleId: "r1", timeoutSeconds: 600 }],
    })
  })

  it("lets a role be removed, and sends no row for it", async () => {
    const calls = mockApi()
    render(<QueryLimitsForm canWrite />)
    fireEvent.click(await screen.findByRole("button", { name: "Remove the limit for Analyst" }))
    fireEvent.click(screen.getByRole("button", { name: "Save limits" }))
    await waitFor(() => expect(calls.some((c) => c.method === "PUT")).toBe(true))
    expect(JSON.parse(calls.find((c) => c.method === "PUT")?.body ?? "{}").roles).toEqual([])
  })

  it("is read-only without settings:write: no Save, no inputs to type in", async () => {
    mockApi()
    render(<QueryLimitsForm canWrite={false} />)
    const time = (await screen.findByLabelText(/Time limit \(seconds\)/)) as HTMLInputElement
    expect(time.disabled).toBe(true)
    expect(screen.queryByRole("button", { name: "Save limits" })).toBeNull()
    expect(screen.getByText("Only an administrator can change these.")).toBeTruthy()
  })

  it("says it could not load when the server has no limits route", async () => {
    global.fetch = mock(async (input: RequestInfo | URL) =>
      String(input).includes("/api/identity/roles") ? json([]) : json({ error: "Not found" }, 404),
    ) as unknown as typeof fetch
    render(<QueryLimitsForm canWrite />)
    expect(await screen.findByRole("button", { name: /retry|try again/i })).toBeTruthy()
    expect(screen.queryByLabelText(/Time limit \(seconds\)/)).toBeNull()
  })

  it("shows a reader without settings:write only the limits that apply to them (BI-5 review S6)", async () => {
    global.fetch = mock(async (input: RequestInfo | URL) => {
      if (String(input).includes("/api/identity/roles")) return json([])
      return json({ effective: LIMITS.effective })
    }) as unknown as typeof fetch
    render(<QueryLimitsForm canWrite={false} />)
    expect((await screen.findByText(/For you: 300 seconds to run/)).textContent).toContain("the limit for Analyst")
    expect(screen.queryByLabelText(/Time limit \(seconds\)/)).toBeNull()
    expect(screen.queryByRole("button", { name: "Save limits" })).toBeNull()
  })
})
