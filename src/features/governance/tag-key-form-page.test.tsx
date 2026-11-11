// WEB-7 T2 (D2/D4): defining a tag key and editing its values are their own
// pages. The API's refusal sentence still reaches the form, and the edit page
// loads the key by name through the existing list read, so a reload works.
let pushed: string[] = []
let params: Record<string, string> = { key: "data_domain" }
mock.module("next/navigation", () => ({
  usePathname: () => "/admin/settings/tag-keys/new",
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

import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, mock, spyOn } from "bun:test"
import { AuthProvider } from "@/features/auth/auth-provider"
import { TagKeyCreatePage, TagKeyEditPage } from "./tag-key-form-page"

afterEach(() => {
  cleanup()
  mock.restore()
})

beforeEach(() => {
  pushed = []
  params = { key: "data_domain" }
})

const json = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } })

const KEY = {
  key: "data_domain",
  allowedValues: ["finance", "ops"],
  createdBy: "Rina",
  createdAt: "2026-10-14T10:00:00Z",
  updatedAt: "2026-10-14T10:00:00Z",
}

/** Answers the session and the key list; `refuse` answers every write. */
function stubApi(refuse?: { status: number; error: string }) {
  const writes: { method: string; path: string; body?: unknown }[] = []
  spyOn(globalThis, "fetch").mockImplementation((async (
    input: RequestInfo | URL,
    init?: RequestInit
  ) => {
    const path = String(input)
    const method = init?.method ?? "GET"
    if (path.includes("/api/auth/me")) {
      return json({ id: "u1", name: "Admin", email: null, roles: [], permissions: ["governance:write"], tenants: [] })
    }
    if (method !== "GET") {
      if (refuse) return json({ error: refuse.error }, refuse.status)
      writes.push({
        method,
        path,
        body: init?.body ? JSON.parse(String(init.body)) : undefined,
      })
      const body = JSON.parse(String(init?.body ?? "{}")) as { allowedValues: string[] }
      return json({ ...KEY, allowedValues: body.allowedValues })
    }
    return json([KEY])
  }) as unknown as typeof fetch)
  return writes
}

function renderPage(node: React.ReactElement) {
  return render(<AuthProvider>{node}</AuthProvider>)
}

describe("TagKeyCreatePage (WEB-7 D2)", () => {
  it("defines the key with its values split on commas and goes back to Tag keys", async () => {
    const writes = stubApi()
    renderPage(<TagKeyCreatePage />)
    fireEvent.change(await screen.findByLabelText("Key"), { target: { value: "data_domain" } })
    fireEvent.change(screen.getByLabelText("Allowed values"), {
      target: { value: "finance, ops" },
    })
    fireEvent.click(screen.getByRole("button", { name: "Define key" }))

    await waitFor(() =>
      expect(writes).toEqual([
        {
          method: "POST",
          path: "/api/governance/tag-keys",
          body: { key: "data_domain", allowedValues: ["finance", "ops"] },
        },
      ])
    )
    await waitFor(() => expect(pushed).toEqual(["/admin/settings?section=tag-keys"]))
  })
})

describe("TagKeyEditPage (WEB-7 D2/D4)", () => {
  it("loads the key's values by name and saves them, with the key fixed", async () => {
    const writes = stubApi()
    renderPage(<TagKeyEditPage />)
    const values = (await screen.findByLabelText("Allowed values")) as HTMLInputElement
    await waitFor(() => expect(values.value).toBe("finance, ops"))
    const key = screen.getByLabelText("Key") as HTMLInputElement
    expect(key.value).toBe("data_domain")
    expect(key.disabled).toBe(true)

    fireEvent.change(values, { target: { value: "finance, ops, hr" } })
    fireEvent.click(screen.getByRole("button", { name: "Save values" }))

    await waitFor(() =>
      expect(writes).toEqual([
        {
          method: "PUT",
          path: "/api/governance/tag-keys/data_domain",
          body: { allowedValues: ["finance", "ops", "hr"] },
        },
      ])
    )
    await waitFor(() => expect(pushed).toEqual(["/admin/settings?section=tag-keys"]))
  })

  it("shows the API's sentence in the form when a save is refused", async () => {
    stubApi({
      status: 400,
      error:
        'allowedValues: "Finance" must be 1-64 characters matching ^[a-z0-9][a-z0-9_-]*$',
    })
    renderPage(<TagKeyEditPage />)
    const values = await screen.findByLabelText("Allowed values")
    fireEvent.change(values, { target: { value: "Finance" } })
    fireEvent.click(screen.getByRole("button", { name: "Save values" }))
    expect(
      await screen.findByText(/allowedValues: "Finance" must be 1-64 characters/)
    ).toBeTruthy()
    expect(pushed).toEqual([])
  })

  it("shows the not-found state for a key that no longer exists", async () => {
    params = { key: "retired_key" }
    stubApi()
    renderPage(<TagKeyEditPage />)
    expect(await screen.findByText("Tag key not found")).toBeTruthy()
    expect(screen.queryByLabelText("Allowed values")).toBeNull()
  })
})
