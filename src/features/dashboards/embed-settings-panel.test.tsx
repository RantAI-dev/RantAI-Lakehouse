import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"
import type { EmbedInfo } from "@/services"
import { EmbedSettingsPanel } from "./embed-settings-panel"

const originalFetch = global.fetch
afterEach(() => {
  global.fetch = originalFetch
  cleanup()
})

const json = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } })

const INFO: EmbedInfo = {
  enabled: true, supported: true, sampleToken: "dash.sample.token", maxLifetimeSeconds: 3600, revokedBefore: null,
  allowedOrigins: [], filterModes: { region: "locked" }, filterColumns: ["region", "year"],
}

type Call = { url: string; method: string; body: unknown }

function stub(onWrite: () => Response = () => json({ ok: true })): Call[] {
  const calls: Call[] = []
  global.fetch = mock(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input)
    const method = init?.method ?? "GET"
    calls.push({ url, method, body: init?.body ? JSON.parse(String(init.body)) : undefined })
    if (url === "/api/dashboard/specs") return json({ charts: [{ id: "u_1", title: "Visits", board: "b_1" }, { id: "u_2", title: "Other", board: "b_2" }] })
    if (url.startsWith("/api/dashboard/embed-info")) return json({ ...INFO, sampleToken: url.includes("chart=u_1") ? "chart.sample.token" : "dash.sample.token" })
    return onWrite()
  }) as unknown as typeof fetch
  return calls
}

function open(onInfo: (i: EmbedInfo) => void = () => {}) {
  render(<EmbedSettingsPanel board="b_1" info={INFO} origin="https://lake.example" onInfo={onInfo} />)
}

const signText = () => (document.body.textContent ?? "")

describe("EmbedSettingsPanel (BI-26 D9)", () => {
  it("lists the dashboard's filters with their saved mode, hidden when none was chosen", async () => {
    stub()
    open()
    expect((screen.getByLabelText("Mode of region") as HTMLSelectElement).value).toBe("locked")
    expect((screen.getByLabelText("Mode of year") as HTMLSelectElement).value).toBe("hidden")
  })

  it("saves the modes and says unsaved changes until then", async () => {
    const calls = stub()
    const infos: EmbedInfo[] = []
    open((i) => infos.push(i))
    const save = screen.getByRole("button", { name: "Save filter modes" }) as HTMLButtonElement
    expect(save.disabled).toBe(true)
    fireEvent.change(screen.getByLabelText("Mode of year"), { target: { value: "editable" } })
    expect(screen.getByText("Unsaved changes")).toBeDefined()
    fireEvent.click(save)
    await waitFor(() => expect(calls.some((c) => c.method === "PUT")).toBe(true))
    const put = calls.find((c) => c.method === "PUT")
    expect(put?.url).toBe("/api/dashboard/embed-settings")
    expect(put?.body).toEqual({ board: "b_1", filterModes: { region: "locked", year: "editable" } })
    await waitFor(() => expect(infos[0]?.filterModes).toEqual({ region: "locked", year: "editable" }))
  })

  it("shows the server's sentence when saving is refused", async () => {
    stub(() => json({ error: "A mode can only be set for a filter the dashboard has." }, 400))
    open()
    fireEvent.change(screen.getByLabelText("Mode of year"), { target: { value: "locked" } })
    fireEvent.click(screen.getByRole("button", { name: "Save filter modes" }))
    await screen.findByText(/A mode can only be set/)
  })

  it("the signing example names every locked filter the token must set", async () => {
    stub()
    open()
    fireEvent.click(screen.getByText("How to sign a token"))
    expect(signText()).toContain('"region": "<value for region>"')
    expect(signText()).not.toContain('"year": "<value for year>"')
    expect(signText()).toContain('resource: { dashboard: "b_1" }')
  })

  it("choosing one chart changes the example to resource.chart and asks for that chart's sample", async () => {
    const calls = stub()
    open()
    fireEvent.click(screen.getByLabelText("One chart"))
    const picker = (await screen.findByLabelText("Chart")) as HTMLSelectElement
    await waitFor(() => expect(picker.options.length).toBe(2))
    expect(Array.from(picker.options).map((o) => o.text)).toEqual(["Choose a chart…", "Visits"])
    fireEvent.change(picker, { target: { value: "u_1" } })
    fireEvent.click(screen.getByText("How to sign a token"))
    expect(signText()).toContain('resource: { chart: "u_1" }')
    await waitFor(() => expect(calls.some((c) => c.url === "/api/dashboard/embed-info?board=b_1&chart=u_1")).toBe(true))
    await waitFor(() => expect((screen.getByRole("button", { name: "Show preview" }) as HTMLButtonElement).disabled).toBe(false))
    fireEvent.click(screen.getByRole("button", { name: "Show preview" }))
    await waitFor(() =>
      expect((screen.getByTitle("Embed preview") as HTMLIFrameElement).src).toContain("/embed/signed/chart.sample.token"),
    )
  })

  it("the look options go into the preview address and the snippets", async () => {
    stub()
    open()
    fireEvent.change(screen.getByLabelText("Theme"), { target: { value: "dark" } })
    fireEvent.click(screen.getByLabelText("Hide the Rantai Lake name"))
    fireEvent.click(screen.getByLabelText("Let the page behind show through"))
    fireEvent.click(screen.getByRole("button", { name: "Show preview" }))
    const frame = screen.getByTitle("Embed preview") as HTMLIFrameElement
    expect(frame.src).toContain("?theme=dark&logo=none&background=transparent")
    fireEvent.click(screen.getByText("How to sign a token"))
    fireEvent.click(screen.getByText("Embed on your page (the frame follows its content)"))
    expect(signText()).toContain('+ token + "?theme=dark&logo=none&background=transparent"')
    expect(signText()).toContain("rantai-embed:size")
  })

  it("offers only Hidden for a filter that does not pick values (BI-26 review fix R3)", async () => {
    stub()
    render(<EmbedSettingsPanel board="b_1" info={{ ...INFO, hiddenOnlyColumns: ["year"] }} origin="https://lake.example" onInfo={() => {}} />)
    const year = screen.getByLabelText("Mode of year") as HTMLSelectElement
    expect(Array.from(year.options).map((o) => o.value)).toEqual(["hidden"])
    const region = screen.getByLabelText("Mode of region") as HTMLSelectElement
    expect(Array.from(region.options).map((o) => o.value)).toEqual(["locked", "editable", "hidden"])
    expect(screen.getByText("not a list of values")).toBeDefined()
  })

  it("cannot save modes without the write right", async () => {
    stub()
    render(<EmbedSettingsPanel board="b_1" info={INFO} origin="https://lake.example" canWrite={false} onInfo={() => {}} />)
    expect((screen.getByLabelText("Mode of year") as HTMLSelectElement).disabled).toBe(true)
  })
})
