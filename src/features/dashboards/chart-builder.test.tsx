// UX-1-R3: every kind-dependent required control in the chart builder must
// have an id linked to a visible Label that carries the shared required prop,
// not just aria-required on an unlabelled trigger.
mock.module("next/navigation", () => ({
  usePathname: () => "/dashboards",
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

import { render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"
import { AuthProvider } from "@/features/auth/auth-provider"
import { ChartBuilder } from "./chart-builder"

const originalFetch = global.fetch
afterEach(() => {
  global.fetch = originalFetch
})

function stubFetch() {
  global.fetch = mock(async (input: RequestInfo | URL) => {
    const url = String(input)
    const json = (body: unknown) =>
      new Response(JSON.stringify(body), { status: 200, headers: { "Content-Type": "application/json" } })
    if (url.includes("/api/auth/me")) {
      return json({
        id: "u1",
        name: "Builder",
        email: null,
        roles: [],
        permissions: ["*:*"],
        tenants: [{ id: "t1", name: "Acme", slug: "acme" }],
      })
    }
    if (url.includes("/api/dashboard/fields")) return json({ marts: [] })
    if (url.includes("/api/dashboard/sql-sources")) return json([])
    return json({})
  }) as unknown as typeof fetch
}

/** The label must be visible, associated by htmlFor, and carry data-required. */
function expectAssociatedRequiredLabel(id: string) {
  const control = document.getElementById(id)
  expect(control).not.toBeNull()
  const label = document.querySelector(`label[for="${id}"]`)
  expect(label).not.toBeNull()
  expect(label?.getAttribute("data-required")).toBe("")
  expect(control?.getAttribute("aria-required")).toBe("true")
}

describe("ChartBuilder required control associations (UX-1-R3)", () => {
  it("associates every required kind-dependent select with a visible Label", () => {
    stubFetch()
    render(
      <AuthProvider>
        <ChartBuilder open hideTrigger onOpenChange={mock()} onSaved={mock()} />
      </AuthProvider>
    )

    // hbar (the default kind) renders dimension + measure + data source.
    expectAssociatedRequiredLabel("ch-source")
    expectAssociatedRequiredLabel("ch-dimension")
    expectAssociatedRequiredLabel("ch-measure")
    expect(screen.getByText(/Required/)).toBeDefined()
  })

  it("uses the shared required prop for the text-chart title and content", () => {
    stubFetch()
    render(
      <AuthProvider>
        <ChartBuilder open hideTrigger onOpenChange={mock()} onSaved={mock()} initial={{ kind: "text" }} />
      </AuthProvider>
    )

    const titleLabel = document.querySelector('label[for="ch-title"]')
    const contentLabel = document.querySelector('label[for="ch-content"]')
    expect(titleLabel?.getAttribute("data-required")).toBe("")
    expect(contentLabel?.getAttribute("data-required")).toBe("")
    expect(document.getElementById("ch-content")?.getAttribute("aria-required")).toBe("true")
  })
})
