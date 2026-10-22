import { cleanup, render, screen, waitFor } from "@testing-library/react"
import { afterAll, afterEach, describe, expect, it, mock } from "bun:test"
import * as realThemes from "next-themes"
import * as realTileBody from "./tile-body"
import * as realGrid from "./dashboard-grid"
import * as realImage from "next/image"

// BI-17 part A (D6, decision 13). `mock.module` outlives this file in `bun test src`, so the real
// exports are kept and put back in `afterAll` (PR #160 CI fix, as in chart-builder.test.tsx).
const themes = { ...realThemes }
const tileBody = { ...realTileBody }
const grid = { ...realGrid }
const image = { ...realImage }
const seen: { dark: boolean }[] = []
let consoleTheme = "light"
mock.module("next-themes", () => ({ ...themes, useTheme: () => ({ resolvedTheme: consoleTheme, setTheme: () => {} }) }))
// next/image builds an absolute URL, which happy-dom cannot resolve for a relative logo path.
mock.module("next/image", () => ({ ...image, default: ({ alt }: { alt: string }) => <span role="img" aria-label={alt} /> }))
mock.module("./tile-body", () => ({
  ...tileBody,
  TileBody: (props: { dark: boolean }) => {
    seen.push({ dark: props.dark })
    return <div data-testid="tile" data-dark={String(props.dark)} />
  },
}))
mock.module("./dashboard-grid", () => ({
  ...grid,
  DashboardGrid: ({ items }: { items: { id: string; body: React.ReactNode }[] }) => <div>{items.map((i) => <div key={i.id}>{i.body}</div>)}</div>,
}))
afterAll(() => {
  mock.module("next-themes", () => themes)
  mock.module("./tile-body", () => tileBody)
  mock.module("./dashboard-grid", () => grid)
  mock.module("next/image", () => image)
})

const { PublicDashboard } = await import("./public-dashboard")
const { EmbedView } = await import("./embed-view")

const originalFetch = global.fetch
afterEach(() => {
  cleanup()
  seen.length = 0
  global.fetch = originalFetch
})

function payload(board: Record<string, unknown>, reporting: Record<string, unknown> = {}) {
  return {
    board: { id: "b1", name: "Visits", ...board },
    layout: {},
    charts: [{ id: "c1", title: "A", kind: "bar", mart: "m", x: "k", y: "v", source: "ui" }],
    results: {},
    reporting: { timeZone: "UTC", weekStart: "monday", ...reporting },
  }
}

function serve(body: unknown) {
  global.fetch = mock(async () => new Response(JSON.stringify(body), { status: 200, headers: { "Content-Type": "application/json" } })) as unknown as typeof fetch
}

describe("a dashboard with its own theme and font", () => {
  it("a public link of a dark dashboard draws dark tiles while the console is light, and hides the viewer's toggle", async () => {
    consoleTheme = "light"
    serve(payload({ theme: "dark", font: "source-serif" }))
    const { container } = render(<PublicDashboard token="t" />)
    await waitFor(() => expect(screen.getByTestId("tile").getAttribute("data-dark")).toBe("true"))
    const page = container.firstElementChild as HTMLElement
    expect(page.className).toContain("dark")
    // BI-18 part C review fix (SHOULD-FIX) R22: the scope paints its own surface and text colour from
    // its own (dark) tokens, so tile text is light on a light console and no grey box shows.
    expect(page.className).toContain("text-foreground")
    expect(page.className).toContain("bg-background")
    expect(page.style.fontFamily).toContain("--font-source-serif")
    expect(screen.queryByRole("button", { name: /^(Light|Dark|Theme)$/ })).toBeNull()
  })

  it("a public link with no theme follows the console and keeps the toggle", async () => {
    consoleTheme = "light"
    serve(payload({}))
    const { container } = render(<PublicDashboard token="t" />)
    await waitFor(() => expect(screen.getByTestId("tile").getAttribute("data-dark")).toBe("false"))
    expect((container.firstElementChild as HTMLElement).className).not.toContain("theme-light")
    expect(screen.getByRole("button", { name: /Dark|Theme/ })).toBeTruthy()
  })

  it("a light dashboard stays light while the console is dark", async () => {
    consoleTheme = "dark"
    serve(payload({ theme: "light" }))
    const { container } = render(<PublicDashboard token="t" />)
    await waitFor(() => expect(screen.getByTestId("tile").getAttribute("data-dark")).toBe("false"))
    expect((container.firstElementChild as HTMLElement).className).toContain("theme-light")
  })

  it("a font the administrator no longer allows is not applied", async () => {
    consoleTheme = "light"
    serve(payload({ font: "inter" }, { fonts: ["system"] }))
    const { container } = render(<PublicDashboard token="t" />)
    await waitFor(() => expect(screen.getByTestId("tile")).toBeTruthy())
    expect((container.firstElementChild as HTMLElement).style.fontFamily).toBe("")
  })

  it("an embed of a dark dashboard draws dark tiles while the console is light", async () => {
    consoleTheme = "light"
    serve(payload({ theme: "dark" }))
    const { container } = render(<EmbedView token="t" />)
    await waitFor(() => expect(screen.getByTestId("tile").getAttribute("data-dark")).toBe("true"))
    expect((container.firstElementChild as HTMLElement).className).toContain("dark")
  })

  it("a single-chart embed takes the theme too", async () => {
    consoleTheme = "light"
    serve(payload({ theme: "dark" }))
    render(<EmbedView token="t" chartId="c1" />)
    await waitFor(() => expect(screen.getByTestId("tile").getAttribute("data-dark")).toBe("true"))
  })
})
