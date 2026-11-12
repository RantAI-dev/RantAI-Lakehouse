// WEB-7 T2 (D2/D4): Create role and Edit role are their own pages. The
// edit page loads the role by id through the existing list read, so a
// reload works, and it keeps the WEB-3 R2 rename warning the sheet had.
// GOV-7 S2 (D8): the same page owns "Tables this role can read", saved by
// the same Save button after the role itself.
// GOV-8 (D2/D3): Create role owns that section too, and sends the address
// `?tables=unsaved` to the Edit page when the tables could not be saved.
let pushed: string[] = []
let params: Record<string, string> = { id: "r1" }
let search = new URLSearchParams()
/** Every request made, reads included (`GOV-9` D4: none without `policy:read`). */
let requests: string[] = []
mock.module("next/navigation", () => ({
  usePathname: () => "/admin/roles/new",
  useSearchParams: () => search,
  useParams: () => params,
  useRouter: () => ({
    push: (href: string) => {
      pushed.push(href)
    },
    replace: () => {},
    refresh: () => {},
    back: () => {},
    forward: () => {},
    prefetch: () => {},
  }),
}))

import * as React from "react"
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, mock, spyOn } from "bun:test"
import { AuthProvider } from "@/features/auth/auth-provider"
import type { Policy } from "@/services/contracts/governance"
import { RoleCreatePage, RoleEditPage } from "./role-form-page"

afterEach(() => {
  cleanup()
  mock.restore()
})

beforeEach(() => {
  pushed = []
  params = { id: "r1" }
  search = new URLSearchParams()
  requests = []
})

const json = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } })

const ROLE = {
  id: "r1",
  name: "Analyst",
  members: 0,
  permissions: "catalog:read",
  description: "Reads",
}

/** A policy row, shaped as `/api/governance/policies` answers it (`GOV-9` D4). */
const POLICY_BASE: Policy = {
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
}

const policy = (overrides: Partial<Policy>): Policy => ({ ...POLICY_BASE, ...overrides })

/** The catalog the table picker lists (`GET /api/catalog`). */
const ASSETS = [
  { id: "silver.orders", namespace: "silver" },
  { id: "silver.customers", namespace: "silver" },
  { id: "serving.daily_sales", namespace: "serving" },
]

type Write = { method: string; path: string; body?: unknown }
type Stub = {
  roles?: unknown[]
  access?: { scope: string; grants: { schema: string; table: string }[] }
  assets?: unknown[]
  /** The permission catalog the picker lists; `*:*` is only offered when named (`GOV-8` D2). */
  permissions?: unknown[]
  /** The signed-in person's grants; `policy:read` turns on the `GOV-9` section. */
  mePermissions?: string[]
  /** The policy rows the Access rules read answers (`GOV-9` D4). */
  policies?: Policy[]
  catalogStatus?: number
  accessStatus?: number
  policiesStatus?: number
  tableAccessAnswer?: () => Response
}

/**
 * Answers the session, the permission catalog, the role read, the role's
 * table access, the asset list and the policy rows; records every write in
 * order.
 */
function stubApi({
  roles = [ROLE],
  access = { scope: "all", grants: [] },
  assets = ASSETS,
  permissions = [
    { name: "catalog:read", area: "Data", description: "Browse tables." },
    { name: "query:read", area: "Data", description: "Run SQL." },
  ],
  mePermissions = ["identity:write"],
  policies = [],
  catalogStatus = 200,
  accessStatus = 200,
  policiesStatus = 200,
  tableAccessAnswer,
}: Stub = {}) {
  const writes: Write[] = []
  spyOn(globalThis, "fetch").mockImplementation((async (
    input: RequestInfo | URL,
    init?: RequestInit
  ) => {
    const path = String(input)
    const method = init?.method ?? "GET"
    const body = init?.body ? JSON.parse(String(init.body)) : undefined
    requests.push(path)
    if (path.includes("/api/auth/me")) {
      return json({ id: "u1", name: "Admin", email: null, roles: ["Admin"], permissions: mePermissions, tenants: [] })
    }
    if (path.includes("/api/identity/permissions")) {
      return json(permissions)
    }
    if (method === "GET" && path.includes("/api/governance/policies")) {
      if (policiesStatus !== 200) {
        return json({ error: "Policies unavailable" }, policiesStatus)
      }
      return json(policies)
    }
    if (path.includes("/table-access")) {
      if (method === "GET") {
        if (accessStatus !== 200) return json({ error: "Access unavailable" }, accessStatus)
        return json(access)
      }
      writes.push({ method, path, body })
      if (tableAccessAnswer) return tableAccessAnswer()
      return json({ scope: body?.scope ?? "all", grants: body?.grants ?? [] })
    }
    if (method === "GET" && path.includes("/api/catalog")) {
      if (catalogStatus !== 200) return json({ error: "Catalog unavailable" }, catalogStatus)
      return json({ assets, namespaces: [] })
    }
    if (method === "GET" && path.includes("/api/identity/roles")) return json(roles)
    if (method !== "GET") {
      writes.push({ method, path, body })
      return json({ ...ROLE, ...(body ?? {}) })
    }
    return json({})
  }) as unknown as typeof fetch)
  return writes
}

function renderPage(node: React.ReactElement) {
  return render(<AuthProvider>{node}</AuthProvider>)
}

describe("RoleCreatePage: tables this role can read (GOV-8 D2)", () => {
  it("creates the role first and then writes the tables chosen for it", async () => {
    const writes = stubApi()
    renderPage(<RoleCreatePage />)
    fireEvent.change(await screen.findByLabelText("Name"), { target: { value: "Data Analyst" } })
    fireEvent.change(screen.getByLabelText("Description"), { target: { value: "Reads data." } })
    fireEvent.click(await screen.findByRole("checkbox", { name: "catalog:read" }))
    fireEvent.click(screen.getByRole("radio", { name: "Only these" }))
    fireEvent.click(await screen.findByRole("button", { name: "Expand silver" }))
    fireEvent.click(screen.getByRole("checkbox", { name: "Whole schema silver" }))
    fireEvent.click(screen.getByRole("button", { name: "Create" }))

    await waitFor(() =>
      expect(writes).toEqual([
        {
          method: "POST",
          path: "/api/identity/roles",
          body: {
            name: "Data Analyst",
            permissions: "catalog:read",
            description: "Reads data.",
          },
        },
        // GOV-8 D2: the second request carries the id the create returned.
        {
          method: "PUT",
          path: "/api/identity/roles/r1/table-access",
          body: { scope: "listed", grants: [{ schema: "silver", table: "*" }] },
        },
      ])
    )
    await waitFor(() => expect(pushed).toEqual(["/admin/roles"]))
  })

  it("sends no table-access request when a new role is left on All tables", async () => {
    const writes = stubApi()
    renderPage(<RoleCreatePage />)
    fireEvent.change(await screen.findByLabelText("Name"), { target: { value: "Data Analyst" } })
    fireEvent.change(screen.getByLabelText("Description"), { target: { value: "Reads data." } })
    fireEvent.click(await screen.findByRole("checkbox", { name: "catalog:read" }))
    // GOV-8 D2: "All tables" is what a new role already reads, so the draft
    // does not differ and no table-access write is sent.
    expect((screen.getByRole("radio", { name: "All tables" }) as HTMLInputElement).checked).toBe(true)
    fireEvent.click(screen.getByRole("button", { name: "Create" }))

    await waitFor(() =>
      expect(writes.map((write) => write.path)).toEqual(["/api/identity/roles"])
    )
    await waitFor(() => expect(pushed).toEqual(["/admin/roles"]))
  })

  it("disables the tables section for a new role that grants *:* and writes no table access", async () => {
    // The picker offers `*:*` as a checkbox only when the catalog names it;
    // the real catalog treats `*:*` as always valid (`isKnownToken`), and a
    // role holding it shows the full-access box instead of the area list.
    const writes = stubApi({
      permissions: [{ name: "*:*", area: "Admin", description: "Unrestricted." }],
    })
    renderPage(<RoleCreatePage />)
    fireEvent.change(await screen.findByLabelText("Name"), { target: { value: "Platform Admin" } })
    fireEvent.change(screen.getByLabelText("Description"), {
      target: { value: "Administers the platform." },
    })
    fireEvent.click(await screen.findByRole("checkbox", { name: "*:*" }))
    expect(await screen.findByText("A role with full access cannot be narrowed")).toBeTruthy()
    expect(
      (screen.getByRole("radio", { name: "Only these" }) as HTMLInputElement).disabled
    ).toBe(true)
    fireEvent.click(screen.getByRole("button", { name: "Create" }))

    // GOV-8 D2: the role is created and, as on Edit role, no table-access
    // write is sent for it.
    await waitFor(() =>
      expect(writes.map((write) => write.path)).toEqual(["/api/identity/roles"])
    )
    await waitFor(() => expect(pushed).toEqual(["/admin/roles"]))
  })

  it("opens the new role's Edit page with tables=unsaved when its tables cannot be saved", async () => {
    const writes = stubApi({
      tableAccessAnswer: () => json({ error: "Tables refused" }, 500),
    })
    renderPage(<RoleCreatePage />)
    fireEvent.change(await screen.findByLabelText("Name"), { target: { value: "Data Analyst" } })
    fireEvent.change(screen.getByLabelText("Description"), { target: { value: "Reads data." } })
    fireEvent.click(await screen.findByRole("checkbox", { name: "catalog:read" }))
    fireEvent.click(screen.getByRole("radio", { name: "Only these" }))
    fireEvent.click(await screen.findByRole("button", { name: "Expand silver" }))
    fireEvent.click(screen.getByRole("checkbox", { name: "Whole schema silver" }))
    fireEvent.click(screen.getByRole("button", { name: "Create" }))

    // GOV-8 D3: no retry here (a second Create would create another role or
    // be refused for the name); the Edit page says what happened.
    await waitFor(() =>
      expect(pushed).toEqual(["/admin/roles/r1/edit?tables=unsaved"])
    )
    expect(writes.map((write) => write.path)).toEqual([
      "/api/identity/roles",
      "/api/identity/roles/r1/table-access",
    ])
  })
})

describe("RoleCreatePage (WEB-7 D2)", () => {
  it("creates the role with its three fields and goes back to the list", async () => {
    const writes = stubApi()
    renderPage(<RoleCreatePage />)
    fireEvent.change(await screen.findByLabelText("Name"), { target: { value: "Data Analyst" } })
    fireEvent.change(screen.getByLabelText("Description"), { target: { value: "Reads data." } })
    fireEvent.click(await screen.findByRole("checkbox", { name: "catalog:read" }))
    fireEvent.click(screen.getByRole("button", { name: "Create" }))

    await waitFor(() =>
      expect(writes).toEqual([
        {
          method: "POST",
          path: "/api/identity/roles",
          body: { name: "Data Analyst", permissions: "catalog:read", description: "Reads data." },
        },
      ])
    )
    await waitFor(() => expect(pushed).toEqual(["/admin/roles"]))
  })
})

describe("RoleEditPage (WEB-7 D2/D4)", () => {
  it("loads the role by id, warns about a rename, and saves the whole role", async () => {
    const writes = stubApi()
    renderPage(<RoleEditPage />)
    const name = await screen.findByLabelText("Name")
    expect((name as HTMLInputElement).value).toBe("Analyst")
    fireEvent.change(name, { target: { value: "Data Analyst" } })
    expect(screen.getByText(/A rename is refused while an access rule names Analyst/)).toBeTruthy()
    // GOV-7 S2 review 1 S2: the section writes only when it changed, so this
    // test changes it first ("Only these"); the untouched case is its own
    // test below.
    fireEvent.click(await screen.findByRole("radio", { name: "Only these" }))
    fireEvent.click(screen.getByRole("button", { name: "Save" }))

    await waitFor(() =>
      expect(writes).toEqual([
        {
          method: "PUT",
          path: "/api/identity/roles/r1",
          body: { name: "Data Analyst", permissions: "catalog:read", description: "Reads" },
        },
        // GOV-7 S2: the same Save sends the role's tables, once they changed.
        {
          method: "PUT",
          path: "/api/identity/roles/r1/table-access",
          body: { scope: "listed", grants: [] },
        },
      ])
    )
    await waitFor(() => expect(pushed).toEqual(["/admin/roles"]))
  })

  it("sends no table-access request when the section was not touched", async () => {
    const writes = stubApi()
    renderPage(<RoleEditPage />)
    await screen.findByRole("radio", { name: "Only these" })
    fireEvent.click(screen.getByRole("button", { name: "Save" }))

    await waitFor(() =>
      expect(writes.map((write) => write.path)).toEqual(["/api/identity/roles/r1"])
    )
    await waitFor(() => expect(pushed).toEqual(["/admin/roles"]))
  })

  it("saves the role and goes back when its tables could not be loaded", async () => {
    const writes = stubApi({ accessStatus: 503 })
    renderPage(<RoleEditPage />)
    expect(
      await screen.findByText(/This role's tables could not be loaded: Access unavailable/)
    ).toBeTruthy()
    fireEvent.click(screen.getByRole("button", { name: "Save" }))

    // GOV-7 S2 review 1 S2: an unloaded section is an untouched section.
    await waitFor(() =>
      expect(writes.map((write) => write.path)).toEqual(["/api/identity/roles/r1"])
    )
    await waitFor(() => expect(pushed).toEqual(["/admin/roles"]))
  })

  it("says Saving… on the edit page, never Creating…", async () => {
    stubApi()
    renderPage(<RoleEditPage />)
    await screen.findByLabelText("Name")
    expect(screen.queryByRole("button", { name: "Creating…" })).toBeNull()
    expect(screen.getByRole("button", { name: "Save" })).toBeTruthy()
  })

  it("shows the not-found state for an id that names no role", async () => {
    params = { id: "deleted-role" }
    stubApi()
    renderPage(<RoleEditPage />)
    expect(await screen.findByText("Role not found")).toBeTruthy()
    expect(screen.queryByLabelText("Name")).toBeNull()
  })

  it("keeps the API's sentence when a save is refused", async () => {
    spyOn(globalThis, "fetch").mockImplementation((async (input: RequestInfo | URL, init?: RequestInit) => {
      const path = String(input)
      if (path.includes("/api/auth/me")) {
        return json({ id: "u1", name: "Admin", email: null, roles: [], permissions: ["identity:write"], tenants: [] })
      }
      if (path.includes("/api/identity/permissions")) return json([])
      if (path.includes("/table-access")) return json({ scope: "all", grants: [] })
      if (path.includes("/api/catalog")) return json({ assets: [], namespaces: [] })
      if ((init?.method ?? "GET") === "GET") return json([ROLE])
      return json({ error: "A role with that name already exists." }, 409)
    }) as unknown as typeof fetch)
    renderPage(<RoleEditPage />)
    const name = await screen.findByLabelText("Name")
    fireEvent.change(name, { target: { value: "Auditor" } })
    fireEvent.click(screen.getByRole("button", { name: "Save" }))
    expect(await screen.findByRole("alert")).toBeTruthy()
    expect(pushed).toEqual([])
  })
})

// GOV-7 S2 (D8): "Tables this role can read", on the Edit page, saved by
// the page's one Save after the role itself.
describe("RoleEditPage: tables this role can read (GOV-7 S2-D8)", () => {
  it("saves 'Only these' with the schema chosen and the role's own save", async () => {
    const writes = stubApi()
    renderPage(<RoleEditPage />)
    fireEvent.click(await screen.findByRole("radio", { name: "Only these" }))
    // The schema opens on its own toggle; a fresh pick starts collapsed.
    fireEvent.click(await screen.findByRole("button", { name: "Expand silver" }))
    fireEvent.click(await screen.findByRole("checkbox", { name: "Whole schema silver" }))
    fireEvent.click(screen.getByRole("button", { name: "Save" }))

    await waitFor(() =>
      expect(writes.map((write) => [write.method, write.path, write.body])).toEqual([
        [
          "PUT",
          "/api/identity/roles/r1",
          { name: "Analyst", permissions: "catalog:read", description: "Reads" },
        ],
        [
          "PUT",
          "/api/identity/roles/r1/table-access",
          { scope: "listed", grants: [{ schema: "silver", table: "*" }] },
        ],
      ])
    )
    await waitFor(() => expect(pushed).toEqual(["/admin/roles"]))
  })

  it("disables the section for a *:* role with the API's own reason", async () => {
    const writes = stubApi({ roles: [{ ...ROLE, permissions: "*:*" }] })
    renderPage(<RoleEditPage />)
    expect(await screen.findByText("A role with full access cannot be narrowed")).toBeTruthy()
    expect(
      (screen.getByRole("radio", { name: "Only these" }) as HTMLInputElement).disabled
    ).toBe(true)
    // Nothing to narrow: the Save writes the role and no table access.
    fireEvent.click(screen.getByRole("button", { name: "Save" }))
    await waitFor(() =>
      expect(writes.map((write) => write.path)).toEqual(["/api/identity/roles/r1"])
    )
    await waitFor(() => expect(pushed).toEqual(["/admin/roles"]))
  })

  it("sends a table that is not created yet exactly as typed", async () => {
    const writes = stubApi()
    renderPage(<RoleEditPage />)
    fireEvent.click(await screen.findByRole("radio", { name: "Only these" }))
    fireEvent.change(await screen.findByLabelText("Add a table not yet created"), {
      target: { value: "bronze.orders_new" },
    })
    fireEvent.click(screen.getByRole("button", { name: "Add" }))
    fireEvent.click(screen.getByRole("button", { name: "Save" }))

    await waitFor(() =>
      expect(writes[writes.length - 1]).toEqual({
        method: "PUT",
        path: "/api/identity/roles/r1/table-access",
        body: {
          scope: "listed",
          grants: [{ schema: "bronze", table: "orders_new" }],
        },
      })
    )
  })

  it("shows a grant the catalog does not list and keeps it through a save", async () => {
    const writes = stubApi({
      assets: [],
      access: { scope: "listed", grants: [{ schema: "archived", table: "old_orders" }] },
    })
    renderPage(<RoleEditPage />)
    expect(await screen.findByText("old_orders")).toBeTruthy()
    expect(screen.getByText("Not in the catalog (kept until removed)")).toBeTruthy()
    // GOV-7 S2 review 1 S2: a save writes only a changed section, so this
    // test makes one change (the whole schema) and checks the unlisted grant
    // is still sent with it.
    fireEvent.click(await screen.findByRole("checkbox", { name: "Whole schema archived" }))
    fireEvent.click(screen.getByRole("button", { name: "Save" }))

    await waitFor(() =>
      expect(writes[writes.length - 1]).toEqual({
        method: "PUT",
        path: "/api/identity/roles/r1/table-access",
        body: {
          scope: "listed",
          grants: [
            { schema: "archived", table: "old_orders" },
            { schema: "archived", table: "*" },
          ],
        },
      })
    )
  })

  it("says the role was saved when only its tables were refused, and stays", async () => {
    const writes = stubApi({
      access: { scope: "listed", grants: [{ schema: "silver", table: "*" }] },
      tableAccessAnswer: () =>
        json({ error: "A role with full access cannot be narrowed" }, 400),
    })
    renderPage(<RoleEditPage />)
    await screen.findByRole("radio", { name: "Only these" })
    // GOV-7 S2 review 1 S2: make the section changed, so the refused write is
    // attempted and the page has to say the role itself was saved.
    fireEvent.click(screen.getByRole("checkbox", { name: "Whole schema silver" }))
    fireEvent.click(screen.getByRole("button", { name: "Save" }))

    const alert = await screen.findByRole("alert")
    expect(alert.textContent).toBe(
      "The role was saved, but its tables were not: A role with full access cannot be narrowed"
    )
    expect(writes.map((write) => write.path)).toEqual([
      "/api/identity/roles/r1",
      "/api/identity/roles/r1/table-access",
    ])
    expect(pushed).toEqual([])
  })

  it("says so when the catalog fails and still keeps the grant it has", async () => {
    const writes = stubApi({
      catalogStatus: 503,
      access: { scope: "listed", grants: [{ schema: "silver", table: "orders" }] },
    })
    renderPage(<RoleEditPage />)
    expect(
      await screen.findByText(/The catalog of tables could not be loaded, so only the tables this role already has are listed here/)
    ).toBeTruthy()
    expect(await screen.findByText("orders")).toBeTruthy()
    // GOV-7 S2 review 1 S2: change the section first, then the save carries
    // the grant it kept.
    fireEvent.click(screen.getByRole("checkbox", { name: "Whole schema silver" }))
    fireEvent.click(screen.getByRole("button", { name: "Save" }))
    await waitFor(() =>
      expect(writes[writes.length - 1]).toEqual({
        method: "PUT",
        path: "/api/identity/roles/r1/table-access",
        body: {
          scope: "listed",
          grants: [
            { schema: "silver", table: "orders" },
            { schema: "silver", table: "*" },
          ],
        },
      })
    )
  })

  it("offers a Bronze table by the name the list row's tableKey names", async () => {
    stubApi({
      access: { scope: "listed", grants: [] },
      assets: [{ id: "ds_commerce_orders", tableKey: "bronze.orders" }],
    })
    renderPage(<RoleEditPage />)
    await screen.findByRole("radio", { name: "Only these" })

    // `GOV-7` S2 review 1 B1: the schema is `bronze`, never the registry's
    // `namespace` (here absent, as it is a catalog grouping and no column of
    // the rule).
    expect(await screen.findByText("bronze")).toBeTruthy()
    expect(screen.queryByText("primer")).toBeNull()
    fireEvent.click(screen.getByRole("button", { name: "Expand bronze" }))
    expect(await screen.findByRole("checkbox", { name: "bronze.orders" })).toBeTruthy()
  })

  it("counts a row the registry names no table for under bronze, never under primer", async () => {
    stubApi({
      access: { scope: "listed", grants: [] },
      assets: [{ id: "ds_anonymous" }],
    })
    renderPage(<RoleEditPage />)
    await screen.findByRole("radio", { name: "Only these" })

    expect(await screen.findByText("bronze")).toBeTruthy()
    expect(screen.queryByText("primer")).toBeNull()
    expect(
      screen.getByText(
        "The registry records no table name for these assets, so no grant here can cover them: they stay hidden from a role limited to these tables."
      )
    ).toBeTruthy()
  })

  it("ticks the catalog's table for a stored grant that differs only by case, and unticking removes it", async () => {
    const writes = stubApi({
      access: { scope: "listed", grants: [{ schema: "silver", table: "Orders" }] },
      assets: [{ id: "silver.orders" }],
    })
    renderPage(<RoleEditPage />)

    // `GOV-7` S2 review 1 S1: the rule compares tables without regard to
    // case, so the stored `silver.Orders` is the catalog's `orders`.
    const box = await screen.findByRole("checkbox", { name: "silver.orders" })
    expect(box.getAttribute("aria-checked")).toBe("true")
    fireEvent.click(box)
    fireEvent.click(screen.getByRole("button", { name: "Save" }))

    await waitFor(() =>
      expect(writes[writes.length - 1]).toEqual({
        method: "PUT",
        path: "/api/identity/roles/r1/table-access",
        body: { scope: "listed", grants: [] },
      })
    )
  })
})

// GOV-8 D3: Create role sends the administrator here when the role exists
// but its tables were not saved; the notice must not appear otherwise.
describe("RoleEditPage: the unsaved-tables notice (GOV-8 D3)", () => {
  it("shows the notice when the address carries tables=unsaved, and not otherwise", async () => {
    stubApi()
    search = new URLSearchParams("tables=unsaved")
    renderPage(<RoleEditPage />)
    expect(
      await screen.findByText(
        "This role was created, but its tables were not saved. It reads every table until you save them here."
      )
    ).toBeTruthy()

    cleanup()
    search = new URLSearchParams()
    renderPage(<RoleEditPage />)
    await screen.findByLabelText("Name")
    expect(
      screen.queryByText(/This role was created, but its tables were not saved/)
    ).toBeNull()
  })
})

// GOV-9 D4: Edit role lists the masks and row filters that name the role
// (`role.name`), read-only, below the tables section.
describe("RoleEditPage: masks and row filters on this role (GOV-9 D4)", () => {
  const policyRead = ["identity:write", "policy:read"]

  it("lists only the rules that name the role exactly, as the engine matches", async () => {
    stubApi({
      mePermissions: policyRead,
      policies: [
        policy({
          id: "p1",
          name: "mask_analyst",
          conditions: JSON.stringify({
            roles: ["Analyst"],
            table: "silver.customers",
            mask: ["email"],
          }),
          assetId: "silver.customers",
        }),
        // `condition_names_role` matches case-sensitively: neither of these
        // names the `Analyst` role.
        policy({
          id: "p2",
          name: "mask_lower",
          conditions: JSON.stringify({
            roles: ["analyst"],
            table: "silver.orders",
            mask: ["email"],
          }),
          assetId: "silver.orders",
        }),
        policy({
          id: "p3",
          name: "mask_senior",
          conditions: JSON.stringify({
            roles: ["Senior Analyst"],
            table: "bronze.orders",
            mask: ["email"],
          }),
          assetId: "bronze.orders",
        }),
      ],
    })
    renderPage(<RoleEditPage />)

    expect(await screen.findByRole("link", { name: "silver.customers" })).toBeTruthy()
    expect(screen.queryByText("silver.orders")).toBeNull()
    expect(screen.queryByText("bronze.orders")).toBeNull()
  })

  it("links a rule's table to its Access tab when the API named the asset, and shows it as text otherwise", async () => {
    stubApi({
      mePermissions: policyRead,
      policies: [
        policy({
          id: "p1",
          name: "mask_email",
          conditions: JSON.stringify({
            roles: ["Analyst"],
            table: "silver.customers",
            mask: ["email"],
          }),
          assetId: "silver.customers",
        }),
        policy({
          id: "p2",
          name: "filter_rows",
          conditions: JSON.stringify({
            roles: ["Analyst"],
            table: "bronze.no_such_table",
            rowFilter: "1 = 0",
          }),
          assetId: undefined,
        }),
      ],
    })
    renderPage(<RoleEditPage />)

    const link = await screen.findByRole("link", { name: "silver.customers" })
    expect(link.getAttribute("href")).toBe("/data/assets/silver.customers?tab=access")
    // No asset id: plain text, never a guessed link.
    expect(screen.getByText("bronze.no_such_table")).toBeTruthy()
    expect(screen.queryByRole("link", { name: "bronze.no_such_table" })).toBeNull()
  })

  it("says a rule both masks and filters, and shows a draft rule's draft status", async () => {
    stubApi({
      mePermissions: policyRead,
      policies: [
        policy({
          id: "p1",
          name: "mask_and_filter",
          conditions: JSON.stringify({
            roles: ["Analyst"],
            table: "silver.customers",
            mask: ["email"],
            rowFilter: "region = 'x'",
          }),
          assetId: "silver.customers",
        }),
        policy({
          id: "p2",
          name: "draft_mask",
          status: "draft",
          conditions: JSON.stringify({
            roles: ["Analyst"],
            table: "silver.orders",
            mask: ["phone"],
          }),
          assetId: "silver.orders",
        }),
      ],
    })
    renderPage(<RoleEditPage />)

    expect(await screen.findByText("Masks email; filters rows")).toBeTruthy()
    // The draft reads as a draft, never as an enforced rule.
    expect(screen.getByText("Draft")).toBeTruthy()
  })

  it("sends no policies request without policy:read and names the permission it needs", async () => {
    stubApi()
    renderPage(<RoleEditPage />)

    expect(
      await screen.findByText(
        "Seeing the masks and row filters on a role needs the permission policy:read."
      )
    ).toBeTruthy()
    await screen.findByLabelText("Name")
    expect(requests.some((path) => path.includes("/api/governance/policies"))).toBe(false)
  })

  it("shows the policies error with Retry and still saves the role", async () => {
    const writes = stubApi({ mePermissions: policyRead, policiesStatus: 503 })
    renderPage(<RoleEditPage />)

    expect(
      await screen.findByText(
        /The masks and row filters on this role could not be loaded: Policies unavailable/
      )
    ).toBeTruthy()
    expect(screen.getByRole("button", { name: "Retry" })).toBeTruthy()

    fireEvent.click(screen.getByRole("button", { name: "Save" }))
    // The role save is independent of the unread list.
    await waitFor(() =>
      expect(writes.map((write) => write.path)).toEqual(["/api/identity/roles/r1"])
    )
  })

  it("says no mask or row filter names the role when none does", async () => {
    stubApi({
      mePermissions: policyRead,
      policies: [
        policy({
          id: "p1",
          name: "mask_viewer",
          conditions: JSON.stringify({
            roles: ["Viewer"],
            table: "silver.customers",
            mask: ["email"],
          }),
          assetId: "silver.customers",
        }),
      ],
    })
    renderPage(<RoleEditPage />)

    expect(
      await screen.findByText(
        "No mask or row filter names this role. They are added on a table's Access tab."
      )
    ).toBeTruthy()
  })
})
