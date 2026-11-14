import { describe, expect, it } from "bun:test"
import * as fs from "node:fs"
import { shellPlaceholderFor, sidebarPlaceholderFor } from "./shell-placeholder"

describe("shellPlaceholderFor", () => {
  it("gives the Dashboards page its own list placeholder", () => {
    expect(shellPlaceholderFor("/dashboards/browse")).toBe("dashboard-list")
    expect(shellPlaceholderFor("/dashboards/browse/")).toBe("dashboard-list")
    expect(shellPlaceholderFor("/dashboards/browse?tab=reports")).toBe("dashboard-list")
  })

  it("gives one dashboard, and the resolver that forwards to one, the dashboard placeholder", () => {
    expect(shellPlaceholderFor("/dashboards/default")).toBe("dashboard")
    expect(shellPlaceholderFor("/dashboards/7f3c2a90-1b2c-4d5e-8f90-123456789abc")).toBe("dashboard")
    expect(shellPlaceholderFor("/dashboards")).toBe("dashboard")
    expect(shellPlaceholderFor("/dashboards/")).toBe("dashboard")
  })

  it("gives the dashboard routes that are not a board the generic placeholder", () => {
    expect(shellPlaceholderFor("/dashboards/metrics")).toBe("page")
    expect(shellPlaceholderFor("/dashboards/reports/new")).toBe("page")
    expect(shellPlaceholderFor("/dashboards/reports/r1")).toBe("page")
    expect(shellPlaceholderFor("/dashboards/reports")).toBe("page")
    expect(shellPlaceholderFor("/dashboards/b1/charts/new")).toBe("page")
    expect(shellPlaceholderFor("/dashboards/b1/charts/c1/edit")).toBe("page")
  })

  it("gives Home its own placeholder (WEB-17 T5)", () => {
    expect(shellPlaceholderFor("/")).toBe("home")
    expect(shellPlaceholderFor("")).toBe("home")
    expect(shellPlaceholderFor("/?from=login")).toBe("home")
    expect(shellPlaceholderFor("/#top")).toBe("home")
  })

  it("gives every other route the generic placeholder", () => {
    expect(shellPlaceholderFor("/catalog")).toBe("page")
    expect(shellPlaceholderFor("/dashboardsx")).toBe("page")
  })

  it("names every single-segment dashboards route that is a page of its own", () => {
    // A new `/dashboards/<name>/page.tsx` would otherwise be taken for a board id
    // and drawn as a dashboard while the session is checked.
    const dir = "src/app/(overview)/dashboards"
    const pages = fs
      .readdirSync(dir, { withFileTypes: true })
      .filter((e) => e.isDirectory() && !e.name.startsWith("["))
      .map((e) => e.name)
    expect(pages.length).toBeGreaterThan(0)
    for (const name of pages) {
      expect(shellPlaceholderFor(`/dashboards/${name}`)).not.toBe("dashboard")
    }
  })
})

describe("sidebarPlaceholderFor (WEB-18 T5)", () => {
  it("draws the chat list on a conversation, and the menu elsewhere", () => {
    expect(sidebarPlaceholderFor("/copilot")).toBe("chats")
    expect(sidebarPlaceholderFor("/copilot?id=c1")).toBe("chats")
    expect(sidebarPlaceholderFor("/copilot/")).toBe("chats")
    expect(sidebarPlaceholderFor("/")).toBe("menu")
    expect(sidebarPlaceholderFor("/catalog")).toBe("menu")
    expect(sidebarPlaceholderFor("/copilot/history")).toBe("menu")
  })
})
