// WEB-3 S3-D4: Access rules is a read-only list. Each rule names its table
// (a link to the table's Access tab when the API sent `assetId`, plain text
// when it did not), its roles, what it does, and whether it is enforced
// (SEC-61). Enforce, Stop and Delete are on the table's Access tab only.
let search = new URLSearchParams()
mock.module("next/navigation", () => ({
  usePathname: () => "/governance/policies",
  useSearchParams: () => search,
  useRouter: () => ({
    push: () => {},
    replace: () => {},
    refresh: () => {},
    back: () => {},
    forward: () => {},
    prefetch: () => {},
  }),
}))

import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react"
import { afterEach, describe, expect, it, mock, spyOn } from "bun:test"
import { TableProviders } from "@/components/app-shell/table-providers"
import { AuthProvider } from "@/features/auth/auth-provider"
import type { Policy } from "@/services/contracts/governance"
import { accessRule, accessTabHref, clauseSentence, ruleClause } from "./access-rules"
import { AccessRulesPage } from "./policies-page"

afterEach(() => {
  cleanup()
  mock.restore()
  search = new URLSearchParams()
})

const base: Policy = {
  id: "p1",
  name: "mask_email",
  status: "ready",
  kind: "Column mask",
  subjects: "role:Analyst",
  resources: "silver.customers",
  effect: "Permit with obligation",
  version: 1,
  owner: "admin",
  updatedAt: "2026-11-04T00:00:00.000Z",
  enforceable: true,
  conditions: JSON.stringify({ roles: ["Analyst", "Viewer"], table: "silver.customers", mask: ["email", "phone"] }),
  assetId: "silver.customers",
}

describe("an access rule's facts", () => {
  it("reads the table, roles and masks of an enforceable rule", () => {
    const r = accessRule(base)
    expect(r.table).toBe("silver.customers")
    expect(r.roles).toBe("Analyst, Viewer")
    expect(r.does).toBe("Masks email and phone")
  })

  it("says a row filter filters rows, alone or after its masks", () => {
    expect(clauseSentence({ mask: [], rowFilter: "region = 'x'" })).toBe("Filters rows")
    expect(clauseSentence({ mask: ["email"], rowFilter: "region = 'x'" })).toBe("Masks email; filters rows")
  })

  it("reads nothing out of a rule the API did not call enforceable, even if its text parses", () => {
    expect(ruleClause({ ...base, enforceable: false })).toBeNull()
    const r = accessRule({ ...base, enforceable: false })
    expect(r.table).toBe("silver.customers")
    expect(r.roles).toBe("role:Analyst")
    expect(r.does).toBe("Nothing: written as text, so the engine skips it")
  })

  it("does not crash on conditions written as prose", () => {
    expect(ruleClause({ enforceable: true, conditions: "mask the emails please" })).toBeNull()
  })

  it("links to the Access tab only when the API named the asset", () => {
    expect(accessTabHref(base)).toBe("/data/assets/silver.customers?tab=access")
    expect(accessTabHref({ assetId: undefined })).toBeNull()
  })
})

const json = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } })

function stubApi(permissions: string[], policies: Policy[]) {
  const writes: string[] = []
  spyOn(globalThis, "fetch").mockImplementation((async (input: RequestInfo | URL, init?: RequestInit) => {
    const path = String(input)
    if (path.includes("/api/auth/me")) {
      return json({ id: "u1", name: "Admin", email: null, roles: ["Admin"], permissions, tenants: [] })
    }
    if ((init?.method ?? "GET") !== "GET") writes.push(`${init?.method} ${path}`)
    if (path.includes("/api/governance/policies")) return json(policies)
    return json({})
  }) as typeof fetch)
  return writes
}

function renderPage() {
  return render(
    <AuthProvider>
      <TableProviders>
        <AccessRulesPage />
      </TableProviders>
    </AuthProvider>
  )
}

describe("the Access rules page", () => {
  const bronze: Policy = {
    ...base,
    id: "p2",
    name: "filter_unknown",
    conditions: JSON.stringify({ roles: ["Analyst"], table: "bronze.no_such_table", rowFilter: "1 = 0" }),
    assetId: undefined,
  }
  const prose: Policy = {
    ...base,
    id: "p3",
    name: "old_wizard_rule",
    enforceable: false,
    conditions: "mask the emails",
    assetId: undefined,
  }

  it("lists each rule's table, roles, what it does and status", async () => {
    stubApi(["policy:read", "policy:write"], [base, bronze, prose])
    renderPage()
    expect(await screen.findByRole("heading", { name: "Access rules" })).toBeTruthy()
    const link = await screen.findByRole("link", { name: "silver.customers" })
    expect(link.getAttribute("href")).toBe("/data/assets/silver.customers?tab=access")
    // No asset id: plain text, never a guessed link.
    expect(screen.getByText("bronze.no_such_table")).toBeTruthy()
    expect(screen.queryByRole("link", { name: "bronze.no_such_table" })).toBeNull()
    expect(screen.getByText("Analyst, Viewer")).toBeTruthy()
    expect(screen.getByText("Masks email and phone")).toBeTruthy()
    expect(screen.getByText("Filters rows")).toBeTruthy()
    expect(screen.getByText("Not enforced")).toBeTruthy()
  })

  it("has no Enforce, Stop or Delete, in the list or the drawer, even with policy:write", async () => {
    const writes = stubApi(["policy:read", "policy:write"], [base])
    renderPage()
    fireEvent.click(await screen.findByRole("button", { name: "mask_email" }))
    const drawer = await screen.findByRole("dialog")
    expect(within(drawer).queryByRole("button", { name: /Enforce|Stop enforcing|Delete/ })).toBeNull()
    expect(screen.queryByRole("button", { name: /^(Enforce|Stop enforcing|Delete)/ })).toBeNull()
    const tab = within(drawer).getByRole("link", { name: "the table's Access tab" })
    expect(tab.getAttribute("href")).toBe("/data/assets/silver.customers?tab=access")
    expect(writes).toEqual([])
  })

  it("says in the drawer that a rule written as text is not enforced, and links to the Catalog", async () => {
    stubApi(["policy:read"], [prose])
    renderPage()
    fireEvent.click(await screen.findByRole("button", { name: "old_wizard_rule" }))
    const drawer = await screen.findByRole("dialog")
    await waitFor(() =>
      expect(within(drawer).getByText(/This policy is written as text, so it masks nothing/)).toBeTruthy()
    )
    expect(within(drawer).getByRole("link", { name: "Catalog" }).getAttribute("href")).toBe("/catalog")
  })
})
