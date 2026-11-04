// WEB-3 S3-D8: "Teams & Roles" is "Roles", with Edit and Delete for
// someone with identity:write. A 409 from the API (the role is still held,
// or built in) is shown with the API's own sentence, and the dialog stays
// open. `AuthProvider` and the table's URL state need `next/navigation`;
// it is stubbed before any import resolves (`mock.module` is hoisted).
mock.module("next/navigation", () => ({
  usePathname: () => "/admin/roles",
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

import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react"
import { afterEach, describe, expect, it, mock, spyOn } from "bun:test"
import { TableProviders } from "@/components/app-shell/table-providers"
import { AuthProvider } from "@/features/auth/auth-provider"
import { RolesPage } from "./roles-page"

afterEach(() => {
  cleanup()
  mock.restore()
})

const json = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } })

const analyst = { id: "r1", name: "Analyst", members: 0, permissions: "catalog:read", description: "Reads" }

/** Answers the session with `permissions`; returns every write the page sent. */
function stubApi(permissions: string[], deleteAnswer: () => Response = () => new Response(null, { status: 204 })) {
  const writes: unknown[][] = []
  spyOn(globalThis, "fetch").mockImplementation((async (input: RequestInfo | URL, init?: RequestInit) => {
    const path = String(input)
    const method = init?.method ?? "GET"
    if (path.includes("/api/auth/me")) {
      return json({ id: "u1", name: "Admin", email: null, roles: ["Admin"], permissions, tenants: [] })
    }
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

  it("edits a role and says a rename leaves rules naming the old name behind", async () => {
    const writes = stubApi(["identity:read", "identity:write"])
    renderPage()
    await openMenuItem("Edit role")
    const name = await screen.findByLabelText("Name")
    expect((name as HTMLInputElement).value).toBe("Analyst")
    fireEvent.change(name, { target: { value: "Data Analyst" } })
    expect(screen.getByText(/A rule that names Analyst stops applying to this role/)).toBeTruthy()
    fireEvent.click(screen.getByRole("button", { name: "Save" }))
    await waitFor(() =>
      expect(writes).toEqual([
        ["PUT", "/api/identity/roles/r1", { name: "Data Analyst", permissions: "catalog:read", description: "Reads" }],
      ])
    )
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
