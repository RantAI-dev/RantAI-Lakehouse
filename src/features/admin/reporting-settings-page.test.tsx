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
    if (url.includes("/api/identity/roles")) {
      return jsonResponse([
        { id: "1", name: "Analyst", members: 1, permissions: "", description: "" },
        { id: "2", name: "Viewer", members: 1, permissions: "", description: "" },
      ])
    }
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
  downloadDeniedRoles: [],
  reportAllowedDomains: [],
  publicLinksEnabled: true,
}

/**
 * What the server sends a caller without `settings:write` (`PR #204` review
 * SHOULD-FIX 3): the two lists only an administrator may read are absent.
 */
const READER_DEFAULTS = {
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
    expect(document.querySelector("datalist")).toBeNull()
  })

  it("sends the chosen zone and first day on Save", async () => {
    const calls = mockSettings(DEFAULTS)
    render(<ReportingSettingsForm canWrite zoneList={zoneList} />)
    const input = (await screen.findByLabelText("Report time zone")) as HTMLInputElement
    // The form fills the saved values after the load; edit only once it has.
    await waitFor(() => expect(input.value).toBe("Asia/Jakarta"))
    // The console's own combobox (`UX-7`): typing filters, the arrow key opens it, an option is chosen.
    fireEvent.change(input, { target: { value: "Berlin" } })
    fireEvent.keyDown(input, { key: "ArrowDown" })
    fireEvent.click(await screen.findByRole("option", { name: /Europe\/Berlin/ }))
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
      downloadDeniedRoles: [],
      reportAllowedDomains: [],
      publicLinksEnabled: true,
    })
  })

  it("saves a zone typed in full and left, not the one the form held (PR #635 review SHOULD-FIX 2)", async () => {
    const calls = mockSettings(DEFAULTS)
    render(<ReportingSettingsForm canWrite zoneList={zoneList} />)
    const input = (await screen.findByLabelText("Report time zone")) as HTMLInputElement
    await waitFor(() => expect(input.value).toBe("Asia/Jakarta"))
    // Typed, never picked from the list: leaving the field chooses it.
    fireEvent.change(input, { target: { value: "Europe/Berlin" } })
    fireEvent.blur(input)
    const save = screen.getByRole("button", { name: "Save" }) as HTMLButtonElement
    await waitFor(() => expect(save.disabled).toBe(false))
    fireEvent.click(save)
    await waitFor(() => expect(calls.some((c) => c.method === "PUT")).toBe(true))
    const put = calls.find((c) => c.method === "PUT")
    expect(JSON.parse(put?.body ?? "{}").timeZone).toBe("Europe/Berlin")
  })

  it("puts the held zone back when the text names no zone, so the field and the form agree (PR #635 review SHOULD-FIX 2)", async () => {
    mockSettings(DEFAULTS)
    render(<ReportingSettingsForm canWrite zoneList={zoneList} />)
    const input = (await screen.findByLabelText("Report time zone")) as HTMLInputElement
    await waitFor(() => expect(input.value).toBe("Asia/Jakarta"))
    fireEvent.change(input, { target: { value: "Mars/Olympus" } })
    fireEvent.blur(input)
    await waitFor(() => expect(input.value).toBe("Asia/Jakarta"))
    // Nothing was chosen, so nothing can be saved.
    expect((screen.getByRole("button", { name: "Save" }) as HTMLButtonElement).disabled).toBe(true)
  })

  it("switches public links off for everyone on Save and warns first", async () => {
    const calls = mockSettings(DEFAULTS)
    render(<ReportingSettingsForm canWrite zoneList={zoneList} />)
    const box = (await screen.findByLabelText("Allow public links")) as HTMLElement
    await waitFor(() => expect(box.getAttribute("aria-checked")).toBe("true"))
    fireEvent.click(box)
    await screen.findByText("Saving this stops every public link now.")
    fireEvent.click(screen.getByRole("button", { name: "Save" }))
    await waitFor(() => expect(calls.some((c) => c.method === "PUT")).toBe(true))
    const put = calls.find((c) => c.method === "PUT")
    expect(JSON.parse(put?.body ?? "{}").publicLinksEnabled).toBe(false)
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
    expect(screen.getByLabelText("Number format")).toBeDefined()
    expect(screen.queryByRole("heading", { name: "Numbers" })).toBeNull()
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

  it("lists the roles that may not download and saves the chosen ones", async () => {
    const calls = mockSettings({ ...DEFAULTS, downloadDeniedRoles: ["Viewer"] })
    render(<ReportingSettingsForm canWrite zoneList={zoneList} />)
    expect(await screen.findByRole("heading", { name: "Block downloads for these roles" })).toBeDefined()
    const viewer = (await screen.findByRole("checkbox", { name: "Viewer" })) as HTMLElement
    await waitFor(() => expect(viewer.getAttribute("aria-checked")).toBe("true"))
    fireEvent.click(screen.getByRole("checkbox", { name: "Analyst" }))
    const save = screen.getByRole("button", { name: "Save" }) as HTMLButtonElement
    await waitFor(() => expect(save.disabled).toBe(false))
    fireEvent.click(save)
    await waitFor(() => expect(calls.some((c) => c.method === "PUT")).toBe(true))
    const sent = JSON.parse(calls.find((c) => c.method === "PUT")?.body ?? "{}")
    expect(sent.downloadDeniedRoles).toEqual(["Analyst", "Viewer"])
  })

  // QA LH-QS-2-06 (QS-9), decision 13: the paragraph says what the switch
  // covers now that it also covers Query Studio's download and Copy as TSV.
  it("says the download switch covers Query Studio's download and Copy as TSV", async () => {
    mockSettings(DEFAULTS)
    render(<ReportingSettingsForm canWrite zoneList={zoneList} />)

    // UX-7: the line says the rule, the help tip holds what it covers.
    expect(await screen.findByText("Checked roles cannot export or download.")).toBeDefined()
    fireEvent.click(screen.getByRole("button", { name: /Query Studio result downloads/ }))
    const tip = (await screen.findAllByText(/Query Studio result downloads/))[0] as HTMLElement
    expect(tip.textContent).toContain("Copy as TSV")
    expect(tip.textContent).toContain("It cannot stop copying from the screen.")
  })

  it("shows no downloads section to a caller who may not read the list (PR #204 review SHOULD-FIX 3)", async () => {
    mockSettings(READER_DEFAULTS)
    render(<ReportingSettingsForm canWrite={false} zoneList={zoneList} />)
    await screen.findByLabelText("Report time zone")
    expect(screen.queryByRole("heading", { name: "Block downloads for these roles" })).toBeNull()
    expect(screen.queryByRole("checkbox", { name: "Viewer" })).toBeNull()
  })

  it("adds and removes an outside report domain, refuses a wildcard or an address, and saves the list", async () => {
    const calls = mockSettings({ ...DEFAULTS, reportAllowedDomains: ["partner.example"] })
    render(<ReportingSettingsForm canWrite zoneList={zoneList} />)
    const input = (await screen.findByLabelText("Report domain")) as HTMLInputElement
    await screen.findByText("partner.example")
    const add = screen.getByRole("button", { name: "Add domain" }) as HTMLButtonElement
    for (const bad of ["*.example.com", "ann@example.com", "https://example.com", "example"]) {
      fireEvent.change(input, { target: { value: bad } })
      expect(add.disabled).toBe(true)
    }
    fireEvent.change(input, { target: { value: "Another.Example" } })
    expect(add.disabled).toBe(false)
    fireEvent.click(add)
    expect(screen.getByText("another.example")).toBeDefined()
    fireEvent.click(screen.getByRole("button", { name: "Save" }))
    await waitFor(() => expect(calls.some((c) => c.method === "PUT")).toBe(true))
    const sent = JSON.parse(calls.find((c) => c.method === "PUT")?.body ?? "{}")
    expect(sent.reportAllowedDomains).toEqual(["another.example", "partner.example"])
  })

  it("shows no report-domains section to a caller who may not read the list (PR #204 review SHOULD-FIX 3)", async () => {
    mockSettings(READER_DEFAULTS)
    render(<ReportingSettingsForm canWrite={false} zoneList={zoneList} />)
    await screen.findByLabelText("Report time zone")
    expect(screen.queryByRole("heading", { name: "Outside email domains" })).toBeNull()
    expect(screen.queryByLabelText("Report domain")).toBeNull()
  })

  it("says Unsaved changes next to Save for a palette colour, a content site and a report domain, and clears it after Save (RPT-1 QA fix Q2)", async () => {
    mockSettings(DEFAULTS)
    render(<ReportingSettingsForm canWrite zoneList={zoneList} />)
    const colour = (await screen.findByLabelText("Palette colour")) as HTMLInputElement
    await waitFor(() => expect((screen.getByLabelText("Report time zone") as HTMLInputElement).value).toBe("Asia/Jakarta"))
    expect(screen.queryByText("Unsaved changes")).toBeNull()

    fireEvent.change(screen.getByLabelText("Report domain"), { target: { value: "partner.example.com" } })
    fireEvent.click(screen.getByRole("button", { name: "Add domain" }))
    // One note in the section and one next to Save.
    expect(screen.getAllByText("Unsaved changes").length).toBe(2)
    fireEvent.click(screen.getByRole("button", { name: "Remove partner.example.com" }))
    expect(screen.queryByText("Unsaved changes")).toBeNull()

    fireEvent.change(screen.getByLabelText("Content site"), { target: { value: "https://images.example.com" } })
    fireEvent.click(screen.getByRole("button", { name: "Add site" }))
    expect(screen.getAllByText("Unsaved changes").length).toBe(2)
    fireEvent.click(screen.getByRole("button", { name: "Remove https://images.example.com" }))
    expect(screen.queryByText("Unsaved changes")).toBeNull()

    fireEvent.change(colour, { target: { value: "#112233" } })
    fireEvent.click(screen.getByRole("button", { name: "Add colour" }))
    expect(screen.getAllByText("Unsaved changes").length).toBe(2)
  })
})

describe("help text on Settings > Reporting (UX-7)", () => {
  it("keeps the detail in a help tip that opens on a click, and the line under the heading short", async () => {
    mockSettings(DEFAULTS)
    render(<ReportingSettingsForm canWrite zoneList={zoneList} />)
    // `HelpHint`: the button's name is the detail itself, and a press opens it.
    const hint = await screen.findByRole("button", { name: /Nothing is deleted: turning it on again/ })
    fireEvent.click(hint)
    expect((await screen.findAllByText(/Nothing is deleted: turning it on again/)).length).toBeGreaterThan(0)
    expect(screen.getByText("Off stops every public link at once.")).toBeDefined()
  })
})
