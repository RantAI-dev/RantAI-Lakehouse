import { cleanup, render, screen } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, mock } from "bun:test"

import { PublicDashboard } from "./public-dashboard"

const originalFetch = global.fetch

const json = (body: unknown) =>
  new Response(JSON.stringify(body), {
    status: 200,
    headers: { "Content-Type": "application/json" },
  })

/**
 * A board with no charts: the header and the footer are what this file is
 * about, and an empty grid keeps the render light.
 */
const BOARD = {
  board: { id: "b1", name: "Sales" },
  layout: {},
  charts: [],
  results: {},
}

/**
 * Serves the public branding route and the link's own route. The branding
 * route needs no session: the page reads the workspace's name and logo
 * through it, so an anonymous viewer sees the same branding as a signed-in
 * person.
 */
function serve(branding: unknown): void {
  global.fetch = mock(async (input: RequestInfo | URL) => {
    const url = String(input)
    if (url.includes("/api/branding")) return json(branding)
    if (url.includes("/api/public/dashboard/")) return json(BOARD)
    return json({})
  }) as unknown as typeof fetch
}

/** The header's two pictures, in the order they are drawn: light, then dark. */
function pictures(container: HTMLElement): HTMLImageElement[] {
  return Array.from(container.querySelectorAll("img"))
}

beforeEach(() => {
  // happy-dom starts on `about:blank`; a relative URL has no base to
  // resolve against.
  ;(window as unknown as { happyDOM: { setURL: (url: string) => void } }).happyDOM.setURL(
    "http://localhost/",
  )
})

afterEach(() => {
  cleanup()
  global.fetch = originalFetch
})

describe("the public link page shows the workspace's own branding (ADM-2)", () => {
  it("shows the workspace name and logo from settings, not a fixed product name", async () => {
    serve({
      workspaceName: "Acme Data",
      logo: { type: "image/png", version: "v1" },
      logoDark: { type: "image/png", version: "v2" },
      icon: null,
    })
    const { container } = render(<PublicDashboard token="tok" />)

    expect(await screen.findByText("Acme Data")).toBeTruthy()
    // The product's own name is nowhere in the header: a workspace that set
    // a name must not still be called RantAI Lakehouse.
    expect(screen.queryByText("RantAI Lakehouse")).toBeNull()
    expect(screen.queryByText("RantAI Lakehouse")).toBeNull()

    const shown = pictures(container)
    expect(shown.map((p) => p.getAttribute("src"))).toEqual([
      "/api/branding/images/logo?v=v1",
      "/api/branding/images/logo-dark?v=v2",
    ])
    expect(shown.map((p) => p.getAttribute("alt"))).toEqual(["Acme Data", "Acme Data"])
  })

  it("shows the product's own name and logo when the workspace set none", async () => {
    serve({ workspaceName: null, logo: null, logoDark: null, icon: null })
    const { container } = render(<PublicDashboard token="tok" />)

    expect(await screen.findByText("RantAI Lakehouse")).toBeTruthy()
    expect(pictures(container).map((p) => p.getAttribute("src"))).toEqual([
      "/logo-light.png",
      "/logo-dark.png",
    ])
  })

  it("shows a footer that reads Powered by RantAI Lakehouse", async () => {
    serve({ workspaceName: "Acme Data", logo: null, logoDark: null, icon: null })
    const { container } = render(<PublicDashboard token="tok" />)

    expect(await screen.findByText("Powered by RantAI Lakehouse")).toBeTruthy()
    // The old line named the product as if it were the workspace; it goes.
    expect(container.textContent).not.toContain("Enterprise Lakehouse Console")
  })

  it("shows a workspace name that holds markup as plain text", async () => {
    serve({ workspaceName: "<b>Acme</b>", logo: null, logoDark: null, icon: null })
    const { container } = render(<PublicDashboard token="tok" />)

    expect(await screen.findByText("<b>Acme</b>")).toBeTruthy()
    // The name is text, never HTML: a workspace that typed markup shows the
    // markup, it does not run it.
    expect(container.querySelector("b")).toBeNull()
  })
})
