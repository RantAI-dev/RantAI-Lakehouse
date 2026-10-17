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
        ? jsonResponse({ ...DEFAULTS, timeZone: "UTC", weekStart: "sunday", saved: true })
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

const DEFAULTS = {
  timeZone: "Asia/Jakarta",
  weekStart: "monday",
  saved: false,
  numberLocale: "en-US",
  companyPalette: [],
  fonts: ["inter", "ibm-plex-sans", "source-serif", "jetbrains-mono", "system"],
  contentSites: [],
}

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
      numberLocale: "en-US",
      companyPalette: [],
      fonts: DEFAULTS.fonts,
      contentSites: [],
    })
  })

  it("adds a palette colour, refuses a single colour, and saves the palette and the fonts", async () => {
    const calls = mockSettings(DEFAULTS)
    render(<ReportingSettingsForm canWrite zoneList={zoneList} />)
    const colour = (await screen.findByLabelText("Palette colour")) as HTMLInputElement
    const add = screen.getByRole("button", { name: "Add colour" }) as HTMLButtonElement
    fireEvent.change(colour, { target: { value: "blue" } })
    expect(add.disabled).toBe(true)
    fireEvent.change(colour, { target: { value: "#1A2B3C" } })
    fireEvent.click(add)
    // One colour is not a palette: Save is held back with a sentence.
    expect(screen.getByRole("alert").textContent).toContain("at least 2 colours")
    expect((screen.getByRole("button", { name: "Save" }) as HTMLButtonElement).disabled).toBe(true)
    fireEvent.change(colour, { target: { value: "#ffaa00" } })
    fireEvent.click(add)
    expect(screen.queryByRole("alert")).toBeNull()
    fireEvent.click(screen.getByRole("checkbox", { name: "System font" }))
    const save = screen.getByRole("button", { name: "Save" }) as HTMLButtonElement
    await waitFor(() => expect(save.disabled).toBe(false))
    fireEvent.click(save)
    await waitFor(() => expect(calls.some((c) => c.method === "PUT")).toBe(true))
    const sent = JSON.parse(calls.find((c) => c.method === "PUT")?.body ?? "{}")
    expect(sent.companyPalette).toEqual(["#1a2b3c", "#ffaa00"])
    expect(sent.fonts).toEqual(["inter", "ibm-plex-sans", "source-serif", "jetbrains-mono"])
  })

  it("shows the formatting sections read-only without settings:write", async () => {
    mockSettings({ ...DEFAULTS, companyPalette: ["#112233", "#445566"] })
    render(<ReportingSettingsForm canWrite={false} zoneList={zoneList} />)
    expect(await screen.findByRole("img", { name: "#112233" })).toBeDefined()
    expect(screen.queryByRole("button", { name: "Add colour" })).toBeNull()
    expect(screen.queryByRole("button", { name: "Remove #112233" })).toBeNull()
    expect(screen.getByRole("heading", { name: "Numbers" })).toBeDefined()
    expect(screen.getByRole("heading", { name: "Fonts" })).toBeDefined()
  })

  it("adds allowed content sites, refuses an address with a path or a wildcard, and saves the list", async () => {
    const calls = mockSettings({ ...DEFAULTS, contentSites: ["https://img.example"] })
    render(<ReportingSettingsForm canWrite zoneList={zoneList} />)
    const site = (await screen.findByLabelText("Content site")) as HTMLInputElement
    await screen.findByText("https://img.example")
    const add = screen.getByRole("button", { name: "Add site" }) as HTMLButtonElement
    for (const bad of ["http://a.example", "https://a.example/x", "https://*.example", "a.example"]) {
      fireEvent.change(site, { target: { value: bad } })
      expect(add.disabled).toBe(true)
    }
    fireEvent.change(site, { target: { value: "https://CDN.example:8443" } })
    expect(add.disabled).toBe(false)
    fireEvent.click(add)
    expect(screen.getByText("https://cdn.example:8443")).toBeDefined()
    const save = screen.getByRole("button", { name: "Save" }) as HTMLButtonElement
    await waitFor(() => expect(save.disabled).toBe(false))
    fireEvent.click(save)
    await waitFor(() => expect(calls.some((c) => c.method === "PUT")).toBe(true))
    const sent = JSON.parse(calls.find((c) => c.method === "PUT")?.body ?? "{}")
    expect(sent.contentSites).toEqual(["https://img.example", "https://cdn.example:8443"])
  })

  it("shows the content sites read-only without settings:write", async () => {
    mockSettings({ ...DEFAULTS, contentSites: ["https://img.example"] })
    render(<ReportingSettingsForm canWrite={false} zoneList={zoneList} />)
    expect(await screen.findByText("https://img.example")).toBeDefined()
    expect(screen.queryByRole("button", { name: "Add site" })).toBeNull()
    expect(screen.queryByRole("button", { name: "Remove https://img.example" })).toBeNull()
  })
})
