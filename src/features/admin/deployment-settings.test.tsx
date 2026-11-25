import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"
import { CONFIGURATION_DOC, DeploymentSettings } from "./deployment-settings"

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

/** The answer of `GET /api/settings/deployment`. */
const DEPLOYMENT = {
  features: [
    {
      feature: "pdfExport",
      on: false,
      missing: ["GOTENBERG_URL", "CONSOLE_INTERNAL_URL", "RENDER_TOKEN_SECRET"],
      where: "server",
    },
    { feature: "email", on: false, missing: ["host"], where: "console" },
    {
      feature: "consoleAddress",
      on: false,
      missing: ["consolePublicUrl"],
      where: "console",
    },
    { feature: "singleSignOn", on: false, missing: ["OIDC_ISSUER"], where: "server" },
    { feature: "assistant", on: false, missing: ["LLM_KEY"], where: "server" },
  ],
}

function mockApi(current: unknown = DEPLOYMENT) {
  global.fetch = mock(async () => jsonResponse(current)) as unknown as typeof fetch
}

describe("DeploymentSettings (ADM-6)", () => {
  it("lists every feature with whether it is set up and what is missing", async () => {
    mockApi()
    render(<DeploymentSettings />)
    await waitFor(() => expect(screen.getByText("PDF export")).toBeTruthy())
    for (const name of ["PDF export", "Email", "Console address", "Single sign-on", "Assistant"]) {
      expect(screen.getByText(name)).toBeTruthy()
    }
    expect(screen.getAllByText("Not set up").length).toBe(5)
    expect(screen.getByText(/GOTENBERG_URL, CONSOLE_INTERNAL_URL, RENDER_TOKEN_SECRET/)).toBeTruthy()
    expect(screen.getByText(/Missing: host/)).toBeTruthy()
  })

  it("offers the Email tab for the feature the console can fix", async () => {
    mockApi()
    let opened = false
    render(<DeploymentSettings onOpenEmail={() => { opened = true }} />)
    const button = await screen.findByRole("button", { name: /Set it up in the Email tab/ })
    fireEvent.click(button)
    expect(opened).toBe(true)
  })

  it("points at the configuration guide for a feature only the server owns", async () => {
    mockApi()
    render(<DeploymentSettings />)
    const links = await screen.findAllByRole("link", { name: "How to set it up" })
    expect(links.length).toBeGreaterThan(0)
    expect(links[0].getAttribute("href")).toBe(CONFIGURATION_DOC)
  })

  it("says a feature that is set up is set up", async () => {
    mockApi({
      features: [
        { feature: "email", on: true, missing: [], where: "console" },
        { feature: "pdfExport", on: false, missing: ["GOTENBERG_URL"], where: "server" },
      ],
    })
    render(<DeploymentSettings />)
    await waitFor(() => expect(screen.getByText("Set up")).toBeTruthy())
    expect(screen.getByText("Not set up")).toBeTruthy()
  })
})

describe("DeploymentSettings installation summary (BI-44 AC5)", () => {
  it("shows the full lakehouse and the ClickHouse engine when the deployment reports them", async () => {
    mockApi({ ...DEPLOYMENT, installMode: "full", queryEngine: "clickhouse" })
    render(<DeploymentSettings />)
    const summary = await screen.findByRole("region", { name: "Installation summary" })
    expect(within(summary).getByText("Installation mode")).toBeTruthy()
    expect(within(summary).getByText("Full lakehouse")).toBeTruthy()
    expect(within(summary).getByText("Query engine")).toBeTruthy()
    expect(within(summary).getByText("ClickHouse")).toBeTruthy()
    expect(within(summary).queryByText(/read in place/)).toBeNull()
  })

  it("names the in-place sources and the Trino engine of a bi-only install", async () => {
    mockApi({
      ...DEPLOYMENT,
      installMode: "bi-only",
      queryEngine: "trino",
      supportedInPlaceSources: ["postgresql", "mysql", "clickhouse"],
    })
    render(<DeploymentSettings />)
    const summary = await screen.findByRole("region", { name: "Installation summary" })
    expect(within(summary).getByText("BI & Assistant only (in-place)")).toBeTruthy()
    expect(within(summary).getByText("Trino")).toBeTruthy()
    expect(
      within(summary).getByText(/Data is read in place through the installed engine; nothing is copied into the lakehouse/)
    ).toBeTruthy()
    const sources = within(summary).getByLabelText("Supported external source types")
    for (const source of ["postgresql", "mysql", "clickhouse"]) {
      expect(within(sources).getByText(source)).toBeTruthy()
    }
  })

  it("shows ClickHouse and an empty-source note for a bi-only install with no sources registered", async () => {
    mockApi({
      features: [],
      installMode: "bi-only",
      queryEngine: "clickhouse",
      supportedInPlaceSources: [],
    })
    render(<DeploymentSettings />)
    const summary = await screen.findByRole("region", { name: "Installation summary" })
    expect(within(summary).getByText("BI & Assistant only (in-place)")).toBeTruthy()
    expect(within(summary).getByText("ClickHouse")).toBeTruthy()
    expect(within(summary).queryByText("Trino")).toBeNull()
    expect(within(summary).getByText("No external source types are registered yet.")).toBeTruthy()
  })

  it("draws no installation summary for a legacy response with no installMode or queryEngine", async () => {
    mockApi({ features: [] })
    render(<DeploymentSettings />)
    await waitFor(() => expect(screen.getByLabelText("Deployment features")).toBeTruthy())
    expect(screen.queryByRole("region", { name: "Installation summary" })).toBeNull()
  })

  it("shows an unknown mode as the server sent it instead of guessing full", async () => {
    mockApi({ features: [], installMode: "something-else", queryEngine: "other-engine" })
    render(<DeploymentSettings />)
    const summary = await screen.findByRole("region", { name: "Installation summary" })
    expect(within(summary).getByText("something-else")).toBeTruthy()
    expect(within(summary).getByText("other-engine")).toBeTruthy()
  })
})
