// WEB-7 T2 (D2/D4): Create role and Edit role are their own pages. The
// edit page loads the role by id through the existing list read, so a
// reload works, and it keeps the WEB-3 R2 rename warning the sheet had.
// GOV-7 S2 (D8): the same page owns "Tables this role can read", saved by
// the same Save button after the role itself.
let pushed: string[] = []
let params: Record<string, string> = { id: "r1" }
mock.module("next/navigation", () => ({
  usePathname: () => "/admin/roles/new",
  useSearchParams: () => new URLSearchParams(),
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
import { RoleCreatePage, RoleEditPage } from "./role-form-page"

afterEach(() => {
  cleanup()
  mock.restore()
})

beforeEach(() => {
  pushed = []
  params = { id: "r1" }
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
  catalogStatus?: number
  tableAccessAnswer?: () => Response
}

/**
 * Answers the session, the permission catalog, the role read, the role's
 * table access and the asset list; records every write in order.
 */
function stubApi({
  roles = [ROLE],
  access = { scope: "all", grants: [] },
  assets = ASSETS,
  catalogStatus = 200,
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
    if (path.includes("/api/auth/me")) {
      return json({ id: "u1", name: "Admin", email: null, roles: ["Admin"], permissions: ["identity:write"], tenants: [] })
    }
    if (path.includes("/api/identity/permissions")) {
      return json([
        { name: "catalog:read", area: "Data", description: "Browse tables." },
        { name: "query:read", area: "Data", description: "Run SQL." },
      ])
    }
    if (path.includes("/table-access")) {
      if (method === "GET") return json(access)
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
    fireEvent.click(screen.getByRole("button", { name: "Save" }))

    await waitFor(() =>
      expect(writes).toEqual([
        {
          method: "PUT",
          path: "/api/identity/roles/r1",
          body: { name: "Data Analyst", permissions: "catalog:read", description: "Reads" },
        },
        // GOV-7 S2: the same Save sends the role's tables, which were
        // "all" when the page loaded.
        {
          method: "PUT",
          path: "/api/identity/roles/r1/table-access",
          body: { scope: "all", grants: [] },
        },
      ])
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
    fireEvent.click(screen.getByRole("button", { name: "Save" }))

    await waitFor(() =>
      expect(writes[writes.length - 1]).toEqual({
        method: "PUT",
        path: "/api/identity/roles/r1/table-access",
        body: {
          scope: "listed",
          grants: [{ schema: "archived", table: "old_orders" }],
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
    fireEvent.click(screen.getByRole("button", { name: "Save" }))
    await waitFor(() =>
      expect(writes[writes.length - 1]).toEqual({
        method: "PUT",
        path: "/api/identity/roles/r1/table-access",
        body: { scope: "listed", grants: [{ schema: "silver", table: "orders" }] },
      })
    )
  })
})
