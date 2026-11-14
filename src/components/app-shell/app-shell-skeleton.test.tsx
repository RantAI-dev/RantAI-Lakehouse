import { afterEach, describe, expect, it, mock } from "bun:test"
import * as image from "next/image"

// The logo is a `next/image`, which needs the Next runtime's image config; the test draws its alt.
mock.module("next/image", () => ({ ...image, default: ({ alt }: { alt: string }) => <span role="img" aria-label={alt} /> }))

import { cleanup, render, screen } from "@testing-library/react"
import { SidebarProvider } from "@/components/ui/sidebar"
import { DashboardHeader } from "@/features/dashboards/dashboard-header"
import { DashboardTilesSkeleton } from "@/features/dashboards/dashboard-skeleton"
import { SOFT_TILE_FRAME } from "@/features/dashboards/tile-card"
import { HOME_BACKGROUND_SCRIPT } from "@/lib/home-background"
import { AppShellSkeleton } from "./app-shell-skeleton"

afterEach(cleanup)

function shell(pathname: string) {
  return render(
    <SidebarProvider>
      <AppShellSkeleton pathname={pathname} />
    </SidebarProvider>,
  )
}

// Owner QA 2026-11-14 ("kenapa kita punya dua skeleton page?"): the loading shell is the
// real shell's parts, and its main area is the page's own placeholder for the route.
describe("AppShellSkeleton", () => {
  it("draws the parts that do not depend on the person for real", () => {
    shell("/dashboards/browse")
    expect(screen.getByText("RantAI Lakehouse")).toBeTruthy()
    expect(screen.queryByText("Enterprise Lakehouse Console")).toBeNull()
    // The brand row is the real one: the mark first, in its own slot, then the name, left-aligned.
    const logo = screen.getByRole("img", { name: "RantAI Lakehouse", hidden: true })
    const row = screen.getByText("RantAI Lakehouse").closest("div.flex")
    expect(row?.className).toContain("justify-start")
    expect(row?.firstElementChild?.contains(logo)).toBe(true)
    expect(row?.firstElementChild?.className).toContain("size-8")
    expect(screen.getByLabelText("Open command palette")).toBeTruthy()
    expect(document.querySelector("[data-slot='sidebar-trigger']")).not.toBeNull()
  })

  it("draws the Dashboards page's placeholder on /dashboards/browse", () => {
    shell("/dashboards/browse")
    expect(screen.getByRole("heading", { level: 1, name: "Dashboards" })).toBeTruthy()
    expect(screen.getByRole("status", { name: "Loading dashboards" })).toBeTruthy()
    expect(screen.queryByRole("status", { name: "Loading dashboard" })).toBeNull()
  })

  it("draws the dashboard's placeholder on a board, with soft tiles and no stand-in title", () => {
    shell("/dashboards/b1")
    expect(screen.getByRole("status", { name: "Loading dashboard" })).toBeTruthy()
    expect(document.querySelector("[data-slot='title-placeholder']")).not.toBeNull()
    expect(screen.queryByRole("heading", { level: 1, name: "Dashboards" })).toBeNull()
    expect(document.querySelectorAll("[data-slot='tile-placeholder']").length).toBe(4)
  })

  it("draws Home's own placeholder on / (WEB-17 T5): the band's base, the composer, no generic rows", () => {
    shell("/")
    expect(screen.getByRole("status", { name: "Loading Home" })).toBeTruthy()
    expect(screen.queryByRole("status", { name: "Loading" })).toBeNull()
    // The band in its solid base colour, no drawing yet; transparent by CSS when this browser
    // chose Off, but still there, so the hero keeps its place (owner QA 2026-11-14).
    const band = document.querySelector("[data-slot='lake-band']")
    expect(band).not.toBeNull()
    expect(band?.className).toContain("in-data-[home-bg=off]:opacity-0")
    expect(band?.className).not.toContain("hidden")
    expect(document.querySelector("canvas[data-slot='lake-scene']")).toBeNull()
    // The composer's own component, inert so nothing typed before the page mounts is lost.
    const box = screen.getByLabelText("Message AI Copilot", { selector: "textarea" })
    expect(box.closest("[inert]")).not.toBeNull()
    expect(screen.getByText(/Answers only, changes nothing/)).toBeTruthy()
  })

  it("draws the generic placeholder elsewhere", () => {
    shell("/catalog")
    expect(screen.getByRole("status", { name: "Loading" })).toBeTruthy()
    expect(screen.queryByRole("status", { name: "Loading dashboard" })).toBeNull()
    expect(screen.queryByRole("status", { name: "Loading dashboards" })).toBeNull()
  })
})

describe("the dashboard's loading placeholders", () => {
  it("draws each placeholder tile in the real soft tile's box: no border, no pill", () => {
    render(<DashboardTilesSkeleton />)
    const tile = document.querySelector("[data-slot='tile-placeholder']") as HTMLElement
    // Every class of the real tile's box but its height, which the placeholder sets (a default tile's).
    for (const cls of SOFT_TILE_FRAME.split(" ").filter((c) => c !== "h-full")) expect(tile.className).toContain(cls)
    expect(tile.className).not.toMatch(/(^|\s)border(\s|$)/)
    expect(tile.querySelector(".rounded-full")).toBeNull()
  })

  it("keeps the title and facts lines at their size while the name is not known", () => {
    render(<DashboardHeader folderPath={[]} title={null} facts={{}} loading={false} onStop={() => {}} actions={null} />)
    expect(document.querySelector("[data-slot='title-placeholder']")).not.toBeNull()
    expect(document.querySelector("[data-slot='facts-placeholder']")).not.toBeNull()
    expect(screen.getByRole("heading", { level: 1 }).textContent).toBe("Loading dashboard")
  })

  it("puts the real facts in the same line once there are some, with the title still a placeholder", () => {
    render(<DashboardHeader folderPath={[]} title={null} facts={{ owner: "Ana" }} loading onStop={() => {}} actions={null} />)
    expect(document.querySelector("[data-slot='title-placeholder']")).not.toBeNull()
    expect(document.querySelector("[data-slot='facts-placeholder']")).toBeNull()
    expect(screen.getByText("Ana")).toBeTruthy()
    expect(screen.getByRole("button", { name: "Stop" })).toBeTruthy()
  })
})

describe("the Background mark before first paint (WEB-17 T5)", () => {
  afterEach(() => {
    window.localStorage.clear()
    document.documentElement.removeAttribute("data-home-bg")
  })

  it("marks <html> only when this browser chose Off, and never throws", () => {
    window.localStorage.setItem("home-background", "off")
    new Function(HOME_BACKGROUND_SCRIPT)()
    expect(document.documentElement.getAttribute("data-home-bg")).toBe("off")

    document.documentElement.removeAttribute("data-home-bg")
    window.localStorage.setItem("home-background", "ascii")
    new Function(HOME_BACKGROUND_SCRIPT)()
    expect(document.documentElement.hasAttribute("data-home-bg")).toBe(false)
  })
})
