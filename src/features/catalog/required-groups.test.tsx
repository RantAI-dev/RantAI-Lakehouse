// UX-1-R4: group requirements must be starred at the group level when an
// either/or choice satisfies them, and a pre-filled name that blocks Save is
// required, not optional. These render the real dialogs through their tabs.
mock.module("next/navigation", () => ({
  usePathname: () => "/catalog/demo",
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

import { fireEvent, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"
import type { AssetDetail } from "@/services/contracts/assets"
import { AuthProvider } from "@/features/auth/auth-provider"
import { AssetAccess } from "./asset-access"
import { AssetQuality } from "./asset-quality"

const originalFetch = global.fetch
afterEach(() => {
  global.fetch = originalFetch
})

function stubFetch() {
  global.fetch = mock(async (input: RequestInfo | URL) => {
    const url = String(input)
    if (url.includes("/api/auth/me")) {
      return new Response(
        JSON.stringify({
          id: "u1",
          name: "Owner",
          email: null,
          roles: [],
          permissions: ["*:*"],
          tenants: [{ id: "t1", name: "Acme", slug: "acme" }],
        }),
        { status: 200, headers: { "Content-Type": "application/json" } }
      )
    }
    return new Response("{}", { status: 200, headers: { "Content-Type": "application/json" } })
  }) as unknown as typeof fetch
}

const asset = (a: Partial<AssetDetail>) =>
  ({
    id: "demo-orders",
    name: "orders",
    tableKey: "bronze.demo_orders",
    classification: "internal",
    schema: [{ name: "email", typeName: "text", masked: false, classification: null }],
    classificationRules: [],
    policySummary: [],
    qualityChecks: [],
    ...a,
  }) as AssetDetail

const markerIn = (el: Element | null) => el?.querySelector('span[aria-hidden="true"][class*="before:content-"]') != null

describe("catalog group requirements (UX-1-R4)", () => {
  it("stars the masking-or-filter group but neither alternative on its own", async () => {
    stubFetch()
    render(
      <AuthProvider>
        <AssetAccess asset={asset({})} onChanged={mock()} />
      </AuthProvider>
    )
    fireEvent.click(await screen.findByText("Add policy"))

    const restriction = Array.from(document.querySelectorAll("legend")).find((l) =>
      l.textContent?.includes("Restriction")
    ) as HTMLElement
    expect(restriction).toBeDefined()
    expect(markerIn(restriction)).toBe(true)
    expect(document.getElementById("policy-restriction-help")?.textContent).toContain("At least one")
    const mask = Array.from(document.querySelectorAll("legend")).find((l) =>
      l.textContent?.includes("Columns to mask")
    ) as HTMLElement
    expect(mask).toBeDefined()
    expect(markerIn(mask)).toBe(false)
    expect(document.querySelector('label[for="policy-filter"]')?.textContent).not.toContain("optional")
  })

  it("stars the range-bounds group and marks the generated rule name required", async () => {
    stubFetch()
    render(
      <AuthProvider>
        <AssetQuality asset={asset({})} onChanged={mock()} />
      </AuthProvider>
    )
    fireEvent.click(await screen.findByText("Add rule"))
    fireEvent.change(screen.getByLabelText("Check"), { target: { value: "range" } })

    const bounds = Array.from(document.querySelectorAll("legend")).find((l) =>
      l.textContent?.includes("Range bounds")
    ) as HTMLElement
    expect(bounds).toBeDefined()
    expect(markerIn(bounds)).toBe(true)
    expect(document.getElementById("rule-range-help")?.textContent).toContain("At least one bound")
    expect(document.querySelector('label[for="rule-min"]')?.getAttribute("data-required")).toBeNull()
    expect(document.querySelector('label[for="rule-max"]')?.getAttribute("data-required")).toBeNull()

    const nameLabel = document.querySelector('label[for="rule-name"]')
    expect(nameLabel?.getAttribute("data-required")).toBe("")
    expect(nameLabel?.textContent).not.toContain("optional")
    expect(document.getElementById("rule-name")?.getAttribute("aria-required")).toBe("true")
  })
})
