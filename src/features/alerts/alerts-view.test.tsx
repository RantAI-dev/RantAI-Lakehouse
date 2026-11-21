// BI-65 (owner QA): the Alerts tab lives in the address.
let search = new URLSearchParams()
const replaced: string[] = []
mock.module("next/navigation", () => ({
  usePathname: () => "/alerts",
  useSearchParams: () => search,
  useRouter: () => ({
    push: () => {},
    replace: (href: string) => {
      replaced.push(href)
    },
    refresh: () => {},
    back: () => {},
    forward: () => {},
    prefetch: () => {},
  }),
}))
// bun's mock.module outlives the file: keep the real modules to put back in afterAll.
import * as realIncidents from "@/features/overview/alerts-page"
import * as realRules from "@/features/alerts/alerts-page"
const real = { incidents: { ...realIncidents }, rules: { ...realRules } }
mock.module("@/features/overview/alerts-page", () => ({ PlatformAlertsPage: () => <p>incidents body</p> }))
mock.module("@/features/alerts/alerts-page", () => ({ AlertRulesPage: () => <p>rules body</p> }))

import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import { afterAll, afterEach, describe, expect, it, mock } from "bun:test"
import { AlertsView } from "./alerts-view"

afterAll(() => {
  mock.module("@/features/overview/alerts-page", () => real.incidents)
  mock.module("@/features/alerts/alerts-page", () => real.rules)
})

afterEach(() => {
  cleanup()
  search = new URLSearchParams()
  replaced.length = 0
})

describe("AlertsView tab in the address", () => {
  it("opens Rules & Digests on ?tab=rules", () => {
    search = new URLSearchParams("tab=rules")
    render(<AlertsView />)
    expect(screen.getByRole("tab", { name: /Rules/ }).getAttribute("aria-selected")).toBe("true")
    expect(screen.getByText("rules body")).toBeTruthy()
  })

  it("opens Active Incidents with no tab, or any other value", () => {
    search = new URLSearchParams("tab=nonsense")
    render(<AlertsView />)
    expect(screen.getByRole("tab", { name: /Active Incidents/ }).getAttribute("aria-selected")).toBe("true")
    expect(screen.getByText("incidents body")).toBeTruthy()
  })

  it("replaces the address when the tab changes", () => {
    render(<AlertsView />)
    fireEvent.click(screen.getByRole("tab", { name: /Rules/ }))
    expect(replaced).toEqual(["/alerts?tab=rules"])
  })
})
