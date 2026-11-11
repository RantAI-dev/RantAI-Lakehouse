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

import { cleanup, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it, mock, spyOn } from "bun:test"
import { AuthProvider } from "@/features/auth/auth-provider"
import { CopilotProvider } from "@/features/copilot/use-copilot"
import { HomePage } from "./home-page"

afterEach(() => {
  cleanup()
  mock.restore()
  window.localStorage.clear()
})

const json = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } })

describe("HomePage per-role widgets (ADM-13 review round 1, B3)", () => {
  it("renders with no error state and sends no request to alerts, connectors, pipelines or saved queries for an assistant:ask,dashboard:read user", async () => {
    const requested: string[] = []

    spyOn(globalThis, "fetch").mockImplementation(((input: RequestInfo | URL) => {
      const path = String(input)
      requested.push(path)
      if (path.includes("/api/auth/me")) {
        return Promise.resolve(
          json({
            id: "u_viewer",
            name: "Alice Viewer",
            email: null,
            roles: ["Viewer"],
            permissions: ["assistant:ask", "dashboard:read"],
            tenants: [],
          }),
        )
      }
      if (path.includes("/api/home/layout")) {
        return Promise.resolve(
          json({
            cards: ["dashboard-preview", "recent", "pipeline-runs", "sources", "open-alerts", "saved-queries"],
            shortcuts: ["build-dashboard", "create-pipeline", "connect-source"],
          }),
        )
      }
      if (path.includes("/api/dashboard")) {
        return Promise.resolve(
          json([
            { id: "b_1", name: "Sales Dashboard", chartCount: 2, updatedAt: "2026-11-10T12:00:00Z" },
          ]),
        )
      }
      if (path.includes("/api/overview") && !path.includes("/api/overview/alerts")) {
        return Promise.resolve(json({ staleAssets: 0 }))
      }
      if (path.includes("/api/ai/sessions")) {
        return Promise.resolve(json({ sessions: [] }))
      }
      return Promise.resolve(json({}))
    }) as unknown as typeof fetch)

    render(
      <AuthProvider>
        <CopilotProvider>
          <HomePage />
        </CopilotProvider>
      </AuthProvider>,
    )

    // Wait for the greeting with display name to render
    expect(await screen.findByText(/Alice Viewer/)).toBeTruthy()

    // Status line shows "All clear" (not an error state)
    expect(await screen.findByText("All clear")).toBeTruthy()

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
})
