// `DATA-12` part 2: the buttons that define, change and remove a key show only
// with governance:write (the API refuses the three writes without it), and a
// refusal reaches the screen as the API's own sentence.
mock.module("next/navigation", () => ({
  usePathname: () => "/governance/tag-keys",
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

import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it, mock, spyOn } from "bun:test"
import { AuthProvider } from "@/features/auth/auth-provider"
import { TagKeysPage } from "./tag-keys-page"

afterEach(() => {
  cleanup()
  mock.restore()
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

/** Answers the session with `permissions` and the key list; `refuse` answers every write. */
function stubApi(permissions: string[], refuse?: { status: number; error: string }) {
  let sessionRead: () => void = () => {}
  const done = new Promise<void>((resolve) => {
    sessionRead = resolve
  })
  spyOn(globalThis, "fetch").mockImplementation((async (input: RequestInfo | URL, init?: RequestInit) => {
    const path = String(input)
    if (path.includes("/api/auth/me")) {
      const answer = json({ id: "u1", name: "Reader", email: null, roles: ["Analyst"], permissions, tenants: [] })
      sessionRead()
      return answer
    }
    if (path.includes("/api/governance/tag-keys")) {
      if (init?.method && init.method !== "GET" && refuse) return json({ error: refuse.error }, refuse.status)
      return json([KEY])
    }
    return json({ error: "not stubbed" }, 404)
  }) as unknown as typeof fetch)
  return done
}

function renderPage() {
  return render(
    <AuthProvider>
      <TagKeysPage />
    </AuthProvider>
  )
}

async function sessionSettled(done: Promise<void>) {
  await done
  await new Promise((resolve) => setTimeout(resolve, 50))
}

describe("TagKeysPage", () => {
  it("lists the keys and their values for a reader, with no way to change them", async () => {
    const done = stubApi(["catalog:read"])
    renderPage()
    expect(await screen.findByText("data_domain")).toBeTruthy()
    expect(screen.getByText("finance")).toBeTruthy()
    await sessionSettled(done)
    expect(screen.queryByRole("button", { name: "New tag key" })).toBeNull()
    expect(screen.queryByRole("button", { name: "Delete tag key data_domain" })).toBeNull()
  })

  it("shows the define, edit and delete buttons with governance:write", async () => {
    stubApi(["catalog:read", "governance:write"])
    renderPage()
    expect(await screen.findByRole("button", { name: "New tag key" })).toBeTruthy()
    expect(await screen.findByRole("button", { name: "Edit values of data_domain" })).toBeTruthy()
    expect(await screen.findByRole("button", { name: "Delete tag key data_domain" })).toBeTruthy()
  })

  it("shows the API's sentence in the form when a save is refused", async () => {
    stubApi(["catalog:read", "governance:write"], {
      status: 400,
      error: "allowedValues: \"Finance\" must be 1-64 characters matching ^[a-z0-9][a-z0-9_-]*$",
    })
    renderPage()
    fireEvent.click(await screen.findByRole("button", { name: "Edit values of data_domain" }))
    fireEvent.click(await screen.findByRole("button", { name: "Save values" }))
    expect(await screen.findByText(/allowedValues: "Finance" must be 1-64 characters/)).toBeTruthy()
  })

  it("shows the number of tables in the dialog when a delete is refused", async () => {
    stubApi(["catalog:read", "governance:write"], {
      status: 400,
      error: "\"data_domain\" is still given to 3 tables; remove it from those tables first",
    })
    renderPage()
    fireEvent.click(await screen.findByRole("button", { name: "Delete tag key data_domain" }))
    fireEvent.click(await screen.findByRole("button", { name: "Delete tag key" }))
    expect(await screen.findByText(/still given to 3 tables/)).toBeTruthy()
  })
})
