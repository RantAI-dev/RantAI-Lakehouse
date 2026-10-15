import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"
import { ReportingSettingsForm } from "./reporting-settings-page"

const originalFetch = global.fetch

afterEach(() => {
  cleanup()
  global.fetch = originalFetch
})

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  })
}

type Call = { method: string; body: string | null }

/** `GET` answers `current`; `PUT` records its body and answers `putStatus`. */
function mockSettings(current: unknown, putStatus = 200): Call[] {
  const calls: Call[] = []
  global.fetch = mock(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = typeof input === "string" ? input : input instanceof URL ? input.toString() : input.url
    if (!url.includes("/api/settings/reporting")) return new Response("{}", { status: 200 })
    const method = (init?.method ?? "GET").toUpperCase()
    calls.push({ method, body: typeof init?.body === "string" ? init.body : null })
    if (method === "PUT") {
      return putStatus === 200
        ? jsonResponse({ timeZone: "UTC", weekStart: "sunday", saved: true })
        : jsonResponse({ error: "Unknown time zone." }, putStatus)
    }
    return jsonResponse(current)
  }) as unknown as typeof fetch
  return calls
}

// BI-9 review fix R6: a short list, so the tests do not depend on how fast
// happy-dom renders the browser's several hundred zones under load.
const zoneList = (saved?: string) =>
  ["Asia/Jakarta", "Europe/Berlin", "UTC", ...(saved && !["Asia/Jakarta", "Europe/Berlin", "UTC"].includes(saved) ? [saved] : [])]

const DEFAULTS = { timeZone: "Asia/Jakarta", weekStart: "monday", saved: false }

describe("ReportingSettingsForm", () => {
  it("shows the zone in use and the first day, and offers Save only to a writer", async () => {
    mockSettings(DEFAULTS)
    render(<ReportingSettingsForm canWrite zoneList={zoneList} />)
    await waitFor(() =>
      expect((screen.getByLabelText("Report time zone") as HTMLInputElement).value).toBe("Asia/Jakarta"),
    )
    await waitFor(() =>
      expect(screen.getByRole("combobox", { name: "First day of the week" }).textContent).toContain("Monday"),
    )
    const save = screen.getByRole("button", { name: "Save" }) as HTMLButtonElement
    expect(save.disabled).toBe(true)
  })

  it("is read-only without settings:write and says so", async () => {
    mockSettings(DEFAULTS)
    render(<ReportingSettingsForm canWrite={false} zoneList={zoneList} />)
    await waitFor(() =>
      expect((screen.getByLabelText("Report time zone") as HTMLInputElement).value).toBe("Asia/Jakarta"),
    )
    expect(screen.getByText(/Only an administrator/i)).toBeDefined()
    expect(screen.queryByRole("button", { name: "Save" })).toBeNull()
    expect((screen.getByLabelText("Report time zone") as HTMLInputElement).disabled).toBe(true)
  })

  it("sends the chosen zone and first day on Save", async () => {
    const calls = mockSettings(DEFAULTS)
    render(<ReportingSettingsForm canWrite zoneList={zoneList} />)
    const input = (await screen.findByLabelText("Report time zone")) as HTMLInputElement
    // The form fills the saved values after the load; edit only once it has.
    await waitFor(() => expect(input.value).toBe("Asia/Jakarta"))
    fireEvent.change(input, { target: { value: "Europe/Berlin" } })
    const save = screen.getByRole("button", { name: "Save" }) as HTMLButtonElement
    await waitFor(() => expect(save.disabled).toBe(false))
    fireEvent.click(save)
    await waitFor(() => expect(calls.some((c) => c.method === "PUT")).toBe(true))
    const put = calls.find((c) => c.method === "PUT")
    expect(JSON.parse(put?.body ?? "{}")).toEqual({
      timeZone: "Europe/Berlin",
      weekStart: "monday",
    })
  })
})
