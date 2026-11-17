// WEB-7 T2 (D2): the create-tenant form is its own page. The test pins the
// request it sends and that Cancel and a save both return to the list.
let pushed: string[] = []
mock.module("next/navigation", () => ({
  usePathname: () => "/admin/tenants/new",
  useSearchParams: () => new URLSearchParams(),
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

import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, mock, spyOn } from "bun:test"
import { AuthProvider } from "@/features/auth/auth-provider"
import { TenantCreatePage } from "./tenant-create-page"

afterEach(() => {
  cleanup()
  mock.restore()
})

beforeEach(() => {
  pushed = []
})

const json = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } })

function stubApi() {
  const writes: { method: string; path: string; body?: unknown }[] = []
  spyOn(globalThis, "fetch").mockImplementation((async (
    input: RequestInfo | URL,
    init?: RequestInit
  ) => {
    const path = String(input)
    const method = init?.method ?? "GET"
    if (path.includes("/api/auth/me")) {
      return json({ id: "u1", name: "Admin", email: null, roles: [], permissions: ["identity:write"], tenants: [] })
    }
    if (method === "POST") {
      writes.push({
        method,
        path,
        body: init?.body ? JSON.parse(String(init.body)) : undefined,
      })
      return json({
        id: "t2",
        name: "Meridian Retail",
        slug: "meridian-retail",
        plan: "Enterprise",
        residency: "Jakarta (ID)",
        users: 0,
        agents: 0,
        storageBytes: 0,
        quotaCompute: 0,
        usedCompute: 0,
      })
    }
    return json({})
  }) as unknown as typeof fetch)
  return writes
}

function renderPage() {
  return render(
    <AuthProvider>
      <TenantCreatePage />
    </AuthProvider>
  )
}

describe("TenantCreatePage (WEB-7 D2)", () => {
  it("creates the tenant with all four fields and goes back to the list", async () => {
    const writes = stubApi()
    renderPage()
    fireEvent.change(await screen.findByLabelText("Name"), { target: { value: "Meridian Retail" } })
    fireEvent.change(screen.getByLabelText("Slug"), { target: { value: "meridian-retail" } })
    fireEvent.change(screen.getByLabelText("Plan"), { target: { value: "Enterprise" } })
    fireEvent.change(screen.getByLabelText("Residency"), { target: { value: "Jakarta (ID)" } })
    fireEvent.click(screen.getByRole("button", { name: "Create" }))

    await waitFor(() =>
      expect(writes).toEqual([
        {
          method: "POST",
          path: "/api/identity/tenants",
          body: {
            name: "Meridian Retail",
            slug: "meridian-retail",
            plan: "Enterprise",
            residency: "Jakarta (ID)",
          },
        },
      ])
    )
    await waitFor(() => expect(pushed).toEqual(["/admin/tenants"]))
  })

  it("keeps Create disabled until every field is filled", async () => {
    stubApi()
    renderPage()
    const create = (await screen.findByRole("button", { name: "Create" })) as HTMLButtonElement
    expect(create.disabled).toBe(true)
    fireEvent.change(screen.getByLabelText("Name"), { target: { value: "Meridian" } })
    expect(create.disabled).toBe(true)
    fireEvent.change(screen.getByLabelText("Slug"), { target: { value: "meridian" } })
    fireEvent.change(screen.getByLabelText("Plan"), { target: { value: "Enterprise" } })
    fireEvent.change(screen.getByLabelText("Residency"), { target: { value: "Jakarta (ID)" } })
    await waitFor(() =>
      expect((screen.getByRole("button", { name: "Create" }) as HTMLButtonElement).disabled).toBe(
        false
      )
    )
  })

  it("cancel goes back to the list without creating anything", async () => {
    const writes = stubApi()
    renderPage()
    await screen.findByLabelText("Name")
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }))
    expect(pushed).toEqual(["/admin/tenants"])
    expect(writes).toEqual([])
  })
})
