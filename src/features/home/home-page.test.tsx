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

  // The page's band and the box under it that holds the greeting, composer and chips.
  const band = () => scene()?.closest("[data-slot=lake-band]") ?? null
  const heroTop = () => screen.getByRole("heading", { level: 1 }).closest("section")?.parentElement?.className

  it("Off from Customize hides the art in place, keeps the hero where it was, is remembered, and holds after a reload", async () => {
    resetHomeBackgroundMemory()
    stubFetch()
    renderHome()
    expect(await screen.findByText(/Alice Viewer/)).toBeTruthy()
    expect(scene()).not.toBeNull()
    const before = heroTop()

    fireEvent.click(screen.getByRole("button", { name: /Customize/ }))
    fireEvent.click(await screen.findByRole("menuitemradio", { name: /Off/ }))

    // Owner QA 2026-11-14: Off must not move the composer up; the art fades out, nothing moves.
    await waitFor(() => expect(band()?.getAttribute("data-state")).toBe("off"))
    expect(band()?.className).toContain("opacity-0")
    expect(heroTop()).toBe(before)
    expect(window.localStorage.getItem("home-background")).toBe("off")

    // A reload: a fresh page reads the choice back and never mounts the scene, at the same place.
    cleanup()
    resetHomeBackgroundMemory()
    renderHome()
    expect(await screen.findByText(/Alice Viewer/)).toBeTruthy()
    expect(scene()).toBeNull()
    expect(heroTop()).toBe(before)
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

describe("Home's states (WEB-17 T5)", () => {
  type Route = [string, () => Response]
  const stub = (permissions: string[], routes: Route[]) => {
    const requested: { path: string; method: string }[] = []
    spyOn(globalThis, "fetch").mockImplementation(((input: RequestInfo | URL, init?: RequestInit) => {
      const path = String(input)
      requested.push({ path, method: init?.method ?? "GET" })
      if (path.includes("/api/auth/me")) return Promise.resolve(me(permissions))
      if (path.includes("/api/home/layout")) return Promise.resolve(layout())
      for (const [prefix, answer] of routes) if (path.includes(prefix)) return Promise.resolve(answer())
      return Promise.resolve(json({}))
    }) as unknown as typeof fetch)
    return requested
  }

  it("a new user gets a next step in every widget instead of empty boxes", async () => {
    resetHomeBackgroundMemory()
    const requested = stub(["assistant:ask", "dashboard:read", "pipeline:read"], [
      ["/api/dashboard/fields", () => json({ marts: [] })],
      ["/api/dashboard/shortcuts", () => json({ favourites: [], recent: [] })],
      ["/api/dashboard", () => json([])],
      ["/api/ai/sessions", () => json({ sessions: [] })],
      ["/api/pipelines", () => json({ pipelines: [] })],
    ])
    renderHome()

    expect(await screen.findByText("No dashboards yet")).toBeTruthy()
    expect(await screen.findByText("No chats yet. Try one:")).toBeTruthy()
    expect(await screen.findByText(/No pipelines yet/)).toBeTruthy()

    // The dashboard's next step fills the composer in Build mode; nothing is sent.
    fireEvent.click(screen.getByRole("button", { name: /Build one with AI Copilot/ }))
    const box = screen.getByLabelText("Message AI Copilot", { selector: "textarea" }) as HTMLTextAreaElement
    await waitFor(() => expect(box.value).toBe("Suggest a dashboard for the data we have"))
    expect(screen.getByRole("button", { name: "Build", pressed: true })).toBeTruthy()
    expect(requested.some((r) => r.path.includes("/api/ai/chat"))).toBe(false)
  })

  it("a widget whose read failed says so in one line while the others still render", async () => {
    resetHomeBackgroundMemory()
    stub(["assistant:ask", "dashboard:read", "pipeline:read"], [
      ["/api/dashboard/fields", () => json({ marts: [] })],
      ["/api/dashboard/shortcuts", () => json({ favourites: [], recent: [] })],
      ["/api/dashboard", () => json({ error: "database error" }, 500)],
      ["/api/ai/sessions", () => json({ error: "database error" }, 500)],
      [
        "/api/pipelines",
        () => json({ pipelines: [{ id: "p1", name: "orders_daily", status: "completed", lastRunAt: "2026-11-14T01:00:00Z" }] }),
      ],
    ])
    renderHome()

    expect(await screen.findByText("Couldn't load your dashboards.")).toBeTruthy()
    expect(await screen.findByText("Couldn't load your chats.")).toBeTruthy()
    expect(await screen.findByText("orders_daily")).toBeTruthy()
  })

  it("a pipeline list the route could not read is a failed read, not 'no pipelines yet'", async () => {
    resetHomeBackgroundMemory()
    stub(["assistant:ask", "pipeline:read"], [
      ["/api/dashboard/fields", () => json({ marts: [] })],
      ["/api/ai/sessions", () => json({ sessions: [] })],
      ["/api/pipelines", () => json({ pipelines: [], error: "Dagster unavailable" })],
    ])
    renderHome()

    expect(await screen.findByText("Couldn't reach the orchestrator to list pipelines.")).toBeTruthy()
    expect(screen.queryByText(/No pipelines yet/)).toBeNull()
  })
})

describe("Ask why on a failed pipeline (WEB-17 T6)", () => {
  it("fills the composer with a question naming the pipeline, in Ask mode, focused, and sends nothing", async () => {
    resetHomeBackgroundMemory()
    const requested: string[] = []
    spyOn(globalThis, "fetch").mockImplementation(((input: RequestInfo | URL) => {
      const path = String(input)
      requested.push(path)
      if (path.includes("/api/auth/me")) return Promise.resolve(me(["assistant:ask", "pipeline:read"]))
      if (path.includes("/api/home/layout")) return Promise.resolve(layout())
      if (path.includes("/api/dashboard/fields")) return Promise.resolve(json({ marts: [] }))
      if (path.includes("/api/ai/sessions")) return Promise.resolve(json({ sessions: [] }))
      if (path.includes("/api/pipelines")) {
        return Promise.resolve(
          json({
            pipelines: [
              { id: "p1", name: "orders_daily", status: "failed", lastRunAt: "2026-11-14T01:00:00Z" },
              { id: "p2", name: "visits_hourly", status: "completed", lastRunAt: "2026-11-14T02:00:00Z" },
            ],
          }),
        )
      }
      return Promise.resolve(json({}))
    }) as unknown as typeof fetch)
    renderHome()

    // Only the failed pipeline offers it.
    const askWhy = await screen.findByRole("button", { name: "Ask AI Copilot why orders_daily failed" })
    expect(screen.queryByRole("button", { name: "Ask AI Copilot why visits_hourly failed" })).toBeNull()

    // Start from Build, to see Ask why switch the mode back to Ask.
    fireEvent.click(screen.getByRole("button", { name: "Build" }))
    fireEvent.click(askWhy)

    const box = screen.getByLabelText("Message AI Copilot", { selector: "textarea" }) as HTMLTextAreaElement
    await waitFor(() => expect(box.value).toBe("Why did orders_daily fail?"))
    expect(document.activeElement).toBe(box)
    expect(screen.getByRole("button", { name: "Ask", pressed: true })).toBeTruthy()
    expect(requested.some((p) => p.includes("/api/ai/chat"))).toBe(false)
  })
})
