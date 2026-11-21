// WEB-3 S3-D8: "Teams & Roles" is "Roles", with Edit and Delete for
// someone with identity:write. A 409 from the API (the role is still held,
// or built in) is shown with the API's own sentence, and the dialog stays
// open. WEB-7 D2: creating and editing are their own pages, so the list's
// Edit action navigates there. `AuthProvider` and the table's URL state
// need `next/navigation`; it is stubbed before any import resolves
// (`mock.module` is hoisted).
let pushed: string[] = []
// Next's real router object is stable across renders; the page's column
// memo depends on it, so a new object per call would rebuild (and remount)
// the table on every render.
const router = {
  push: (href: string) => {
    pushed.push(href)
  },
  replace: () => {},
  refresh: () => {},
  back: () => {},
  forward: () => {},
  prefetch: () => {},
}
const searchParams = new URLSearchParams()
mock.module("next/navigation", () => ({
  usePathname: () => "/admin/roles",
  useSearchParams: () => searchParams,
  useRouter: () => router,
}))

import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react"
import { afterEach, describe, expect, it, mock, spyOn } from "bun:test"
import { TableProviders } from "@/components/app-shell/table-providers"
import { AuthProvider } from "@/features/auth/auth-provider"
import { RolesPage } from "./roles-page"

afterEach(() => {
  cleanup()
  mock.restore()
  pushed = []
})

const json = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } })

const analyst = { id: "r1", name: "Analyst", members: 0, permissions: "catalog:read", description: "Reads" }

/**
 * Answers the session with `permissions`; returns every write the page sent.
 * `tableAccess` is the role's read access (`GOV-7` S2's summary column).
 */
function stubApi(
  permissions: string[],
  deleteAnswer: () => Response = () => new Response(null, { status: 204 }),
  tableAccess: unknown = { scope: "all", grants: [] }
) {
  const writes: unknown[][] = []
  spyOn(globalThis, "fetch").mockImplementation((async (input: RequestInfo | URL, init?: RequestInit) => {
    const path = String(input)
    const method = init?.method ?? "GET"
    if (path.includes("/api/auth/me")) {
      return json({ id: "u1", name: "Admin", email: null, roles: ["Admin"], permissions, tenants: [] })
    }
    if (path.includes("/table-access")) return json(tableAccess)
    if (path.includes("/api/identity/roles/")) {
      writes.push([method, path, init?.body ? JSON.parse(String(init.body)) : undefined])
      if (method === "DELETE") return deleteAnswer()
      return json({ ...analyst, ...JSON.parse(String(init?.body)) })
    }
    if (path.includes("/api/identity/roles")) return json([analyst])
    return json({})
  }) as typeof fetch)
  return writes
}

function renderPage() {
  return render(
    <AuthProvider>
      <TableProviders>
        <RolesPage />
      </TableProviders>
    </AuthProvider>
  )
}

async function openMenuItem(name: string) {
  fireEvent.click(await screen.findByRole("button", { name: "Actions for role Analyst" }))
  fireEvent.click(await screen.findByRole("menuitem", { name }))
}

describe("RolesPage (WEB-3 S3-D8)", () => {
  it("is called Roles", async () => {
    stubApi(["identity:read"])
    renderPage()
    expect(await screen.findByRole("heading", { name: "Roles" })).toBeTruthy()
    expect(screen.queryByText("Teams & Roles")).toBeNull()
  })

  it("Edit opens the role's own edit page, which loads it by id (WEB-7 D2)", async () => {
    stubApi(["identity:read", "identity:write"])
    renderPage()
    await openMenuItem("Edit role")
    expect(pushed).toEqual(["/admin/roles/r1/edit"])
  })

  it("Create Role opens the new-role page", async () => {
    stubApi(["identity:read", "identity:write"])
    renderPage()
    const create = await screen.findByRole("button", { name: /create role/i })
    expect(create.getAttribute("href")).toBe("/admin/roles/new")
  })

  it("deletes a role nobody holds after confirming", async () => {
    const writes = stubApi(["identity:read", "identity:write"])
    renderPage()
    await openMenuItem("Delete role")
    const dialog = await screen.findByRole("dialog")
    expect(within(dialog).getByText("Nobody holds it. Its permissions go with it.")).toBeTruthy()
    fireEvent.click(within(dialog).getByRole("button", { name: "Delete role" }))
    await waitFor(() => expect(writes).toEqual([["DELETE", "/api/identity/roles/r1", undefined]]))
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull())
  })

  it("shows the API's 409 sentence and keeps the dialog open", async () => {
    const sentence = "This role is still given to 2 people. Take it away from them first."
    stubApi(["identity:read", "identity:write"], () => json({ error: sentence }, 409))
    renderPage()
    await openMenuItem("Delete role")
    const dialog = await screen.findByRole("dialog")
    fireEvent.click(within(dialog).getByRole("button", { name: "Delete role" }))
    expect((await within(dialog).findByRole("alert")).textContent).toBe(sentence)
  })

  it("offers no Edit or Delete without identity:write", async () => {
    stubApi(["identity:read"])
    renderPage()
    fireEvent.click(await screen.findByRole("button", { name: "Actions for role Analyst" }))
    expect(await screen.findByRole("menuitem", { name: "Copy role name" })).toBeTruthy()
    expect(screen.queryByRole("menuitem", { name: "Edit role" })).toBeNull()
    expect(screen.queryByRole("menuitem", { name: "Delete role" })).toBeNull()
  })
})

// GOV-7 S2 (D8): the list says "All tables" or "N schemas, M tables" for
// each role. The roles endpoint carries no read scope, so the page reads
// each role's access; the column is the only place that answers it.
describe("RolesPage: each role's table access (GOV-7 S2-D8)", () => {
  it("says All tables for a role nobody narrowed", async () => {
    stubApi(["identity:read"])
    renderPage()
    expect(await screen.findByText("All tables")).toBeTruthy()
  })

  it("counts the schemas and single tables of a narrowed role", async () => {
    stubApi(["identity:read"], undefined, {
      scope: "listed",
      grants: [
        { schema: "finance", table: "*" },
        { schema: "gold", table: "daily_sales" },
      ],
    })
    renderPage()
    expect(await screen.findByText("2 schemas, 1 table")).toBeTruthy()
  })

  it("says — rather than All tables when a role's access cannot be read", async () => {
    spyOn(globalThis, "fetch").mockImplementation((async (input: RequestInfo | URL) => {
      const path = String(input)
      if (path.includes("/api/auth/me")) {
        return json({ id: "u1", name: "Admin", email: null, roles: ["Admin"], permissions: ["identity:read"], tenants: [] })
      }
      if (path.includes("/table-access")) return json({ error: "database error" }, 500)
      if (path.includes("/api/identity/roles")) return json([analyst])
      return json({})
    }) as typeof fetch)
    renderPage()
    expect(await screen.findByText("—")).toBeTruthy()
    expect(screen.queryByText("All tables")).toBeNull()
  })
})
