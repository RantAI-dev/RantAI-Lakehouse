mock.module("next/navigation", () => ({
  usePathname: () => "/",
  useSearchParams: () => new URLSearchParams(),
  useRouter: () => ({
    push: () => {},
    replace: () => {},
    refresh: () => {},
    back: () => {},
    forward: () => {},
    prefetch: () => {},
  }),
}))

import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, mock, spyOn } from "bun:test"
import { AuthProvider } from "@/features/auth/auth-provider"
import { CopilotProvider } from "@/features/copilot/use-copilot"
import { HomePage } from "./home-page"
import { resetHomeBackgroundMemory } from "./use-home-background"

afterEach(() => {
  cleanup()
  mock.restore()
  window.localStorage.clear()
})

const json = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } })

/** The sentences the status line uses for each check it names. */
const STATUS_SENTENCES = [
  "no failing pipelines",
  "no open alerts",
  "all sources healthy",
  "every dataset has data",
]

const me = (permissions: string[]) =>
  json({
    id: "u_viewer",
    name: "Alice Viewer",
    email: null,
    roles: ["Viewer"],
    permissions,
    tenants: [],
  })

const layout = () =>
  json({
    cards: ["dashboard-preview", "recent", "pipeline-runs", "sources", "open-alerts", "saved-queries"],
    shortcuts: ["build-dashboard", "create-pipeline", "connect-source"],
  })

const boards = () =>
  json([
    { id: "b_1", name: "Sales Dashboard", chartCount: 2, updatedAt: "2026-11-10T12:00:00Z" },
  ])

const renderHome = () =>
  render(
    <AuthProvider>
      <CopilotProvider>
        <HomePage />
      </CopilotProvider>
    </AuthProvider>,
  )

describe("HomePage per-role widgets (ADM-13 review round 1, B3)", () => {
  it("renders with no error state and sends no request to alerts, connectors, pipelines or saved queries for an assistant:ask,dashboard:read user", async () => {
    const requested: string[] = []

    spyOn(globalThis, "fetch").mockImplementation(((input: RequestInfo | URL) => {
      const path = String(input)
      requested.push(path)
      if (path.includes("/api/auth/me")) {
        return Promise.resolve(me(["assistant:ask", "dashboard:read"]))
      }
      if (path.includes("/api/home/layout")) {
        return Promise.resolve(layout())
      }
      if (path.includes("/api/dashboard")) {
        return Promise.resolve(boards())
      }
      if (path.includes("/api/overview") && !path.includes("/api/overview/alerts")) {
        return Promise.resolve(json({ staleAssets: 0 }))
      }
      if (path.includes("/api/ai/sessions")) {
        return Promise.resolve(json({ sessions: [] }))
      }
      return Promise.resolve(json({}))
    }) as unknown as typeof fetch)

    renderHome()

    // Wait for the greeting with display name to render
    expect(await screen.findByText(/Alice Viewer/)).toBeTruthy()

    // ADM-13 review round 2, R2-B1: no check was made, so no status line and
    // none of the four sentences that would claim one was.
    expect(screen.queryByLabelText("Lakehouse status")).toBeNull()
    for (const sentence of STATUS_SENTENCES) {
      expect(document.body.textContent).not.toContain(sentence)
    }

    // Assert that no requests were sent to alerts, connectors, pipelines, or saved queries
    const forbiddenPaths = [
      "/api/overview/alerts",
      "/api/alerts",
      "/api/connectors",
      "/api/pipelines",
      "/api/queries/saved",
      "/api/query",
    ]
    for (const forbidden of forbiddenPaths) {
      expect(requested.some((p) => p.includes(forbidden))).toBe(false)
    }

    // Verify allowed cards/shortcuts render and disallowed ones do not
    expect(screen.getByText("Build a dashboard")).toBeTruthy()
    expect(screen.queryByText("Create a pipeline")).toBeNull()
    expect(screen.queryByText("Connect a source")).toBeNull()
  })

  it("speaks of alerts only when the user can read alerts and nothing else", async () => {
    spyOn(globalThis, "fetch").mockImplementation(((input: RequestInfo | URL) => {
      const path = String(input)
      if (path.includes("/api/auth/me")) {
        return Promise.resolve(me(["assistant:ask", "dashboard:read", "alert:read"]))
      }
      if (path.includes("/api/home/layout")) {
        return Promise.resolve(layout())
      }
      if (path.includes("/api/overview/alerts")) {
        return Promise.resolve(json([]))
      }
      if (path.includes("/api/dashboard")) {
        return Promise.resolve(boards())
      }
      if (path.includes("/api/ai/sessions")) {
        return Promise.resolve(json({ sessions: [] }))
      }
      return Promise.resolve(json({}))
    }) as unknown as typeof fetch)

    renderHome()

    // The line is drawn, and it speaks of alerts only: the healthy phrases
    // are the tooltip of the quiet "All clear".
    const clear = await screen.findByText("All clear")
    // The healthy phrases are the link's tooltip, and they name alerts only.
    expect(clear.getAttribute("title")).toBe("no open alerts")
    expect(screen.getByLabelText("Lakehouse status")).toBeTruthy()
    expect(clear.textContent).toBe("All clear")
    expect(document.body.textContent).not.toContain("no failing pipelines")
    expect(document.body.textContent).not.toContain("all sources healthy")
    expect(document.body.textContent).not.toContain("every dataset has data")
  })
})

describe("Home background choice (WEB-17 T4)", () => {
  const stubFetch = () =>
    spyOn(globalThis, "fetch").mockImplementation(((input: RequestInfo | URL) => {
      const path = String(input)
      if (path.includes("/api/auth/me")) return Promise.resolve(me(["assistant:ask", "dashboard:read"]))
      if (path.includes("/api/home/layout")) return Promise.resolve(layout())
      if (path.includes("/api/dashboard/fields")) return Promise.resolve(json({ marts: [] }))
      if (path.includes("/api/dashboard/shortcuts")) return Promise.resolve(json({ favourites: [], recent: [] }))
      if (path.includes("/api/dashboard")) return Promise.resolve(boards())
      if (path.includes("/api/ai/sessions")) return Promise.resolve(json({ sessions: [] }))
      return Promise.resolve(json({}))
    }) as unknown as typeof fetch)

  // The page's scene, not the small previews in the open Customize menu.
  const scene = () =>
    [...document.querySelectorAll("canvas[data-slot=lake-scene]")].find((c) => !c.closest("[role=menu]")) ?? null

  it("draws the Dither scene by default", async () => {
    resetHomeBackgroundMemory()
    stubFetch()
    renderHome()
    expect(await screen.findByText(/Alice Viewer/)).toBeTruthy()
    expect(scene()?.getAttribute("data-variant")).toBe("dither")
  })

  it("Off from Customize removes the scene at once, is remembered, and still holds after a reload", async () => {
    resetHomeBackgroundMemory()
    stubFetch()
    renderHome()
    expect(await screen.findByText(/Alice Viewer/)).toBeTruthy()
    expect(scene()).not.toBeNull()

    fireEvent.click(screen.getByRole("button", { name: /Customize/ }))
    fireEvent.click(await screen.findByRole("menuitemradio", { name: /Off/ }))

    await waitFor(() => expect(scene()).toBeNull())
    expect(window.localStorage.getItem("home-background")).toBe("off")

    // A reload: a fresh page reads the choice back before drawing anything.
    cleanup()
    resetHomeBackgroundMemory()
    renderHome()
    expect(await screen.findByText(/Alice Viewer/)).toBeTruthy()
    expect(scene()).toBeNull()
  })

  it("ASCII draws the scene as glyphs", async () => {
    resetHomeBackgroundMemory()
    window.localStorage.setItem("home-background", "ascii")
    stubFetch()
    renderHome()
    expect(await screen.findByText(/Alice Viewer/)).toBeTruthy()
    expect(scene()?.getAttribute("data-variant")).toBe("ascii")
  })
})
