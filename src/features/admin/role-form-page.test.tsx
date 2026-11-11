// WEB-7 T2 (D2/D4): Create role and Edit role are their own pages. The
// edit page loads the role by id through the existing list read, so a
// reload works, and it keeps the WEB-3 R2 rename warning the sheet had.
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

/** Answers the session, the permission catalog and the role read; records every write. */
function stubApi(roles: unknown[] = [ROLE]) {
  const writes: { method: string; path: string; body?: unknown }[] = []
  spyOn(globalThis, "fetch").mockImplementation((async (
    input: RequestInfo | URL,
    init?: RequestInit
  ) => {
    const path = String(input)
    const method = init?.method ?? "GET"
    if (path.includes("/api/auth/me")) {
      return json({ id: "u1", name: "Admin", email: null, roles: ["Admin"], permissions: ["identity:write"], tenants: [] })
    }
    if (path.includes("/api/identity/permissions")) {
      return json([
        { name: "catalog:read", area: "Data", description: "Browse tables." },
        { name: "query:read", area: "Data", description: "Run SQL." },
      ])
    }
    if (method === "GET" && path.includes("/api/identity/roles")) return json(roles)
    if (method !== "GET") {
      writes.push({
        method,
        path,
        body: init?.body ? JSON.parse(String(init.body)) : undefined,
      })
      return json({ ...ROLE, ...(init?.body ? JSON.parse(String(init.body)) : {}) })
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
