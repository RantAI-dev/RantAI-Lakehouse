// WEB-7 T2 (D2): the create form is its own page. The outside-reader hint
// (`DATA-5` D10) moves here with the form, and the create request is
// asserted so the move cannot change what is sent.
let pushed: string[] = []
mock.module("next/navigation", () => ({
  usePathname: () => "/admin/service-identities/new",
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
import { ServiceIdentityCreatePage } from "./service-identity-create-page"

afterEach(() => {
  cleanup()
  mock.restore()
})

beforeEach(() => {
  pushed = []
})

const json = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } })

/** Answers the session and the permission catalog; records every write. */
function stubApi(permissions: string[] = ["identity:write"]) {
  const writes: { method: string; path: string; body?: unknown }[] = []
  spyOn(globalThis, "fetch").mockImplementation((async (
    input: RequestInfo | URL,
    init?: RequestInit
  ) => {
    const path = String(input)
    const method = init?.method ?? "GET"
    if (path.includes("/api/auth/me")) {
      return json({ id: "u1", name: "Admin", email: null, roles: ["Admin"], permissions, tenants: [] })
    }
    if (path.includes("/api/identity/permissions")) {
      return json([
        { name: "ingest:write", area: "Ingest", description: "Load data." },
        { name: "iceberg:read", area: "Storage", description: "Read Gold tables." },
      ])
    }
    if (method !== "GET") {
      writes.push({
        method,
        path,
        body: init?.body ? JSON.parse(String(init.body)) : undefined,
      })
      return json({
        id: "si-1",
        name: "ingestion-worker",
        scopes: ["ingest:write"],
        environment: "production",
        expiresAt: "2026-06-01T00:00:00.000Z",
        rotationStatus: "current",
        lastUsedAt: null,
      })
    }
    return json({})
  }) as unknown as typeof fetch)
  return writes
}

function renderPage() {
  return render(
    <AuthProvider>
      <ServiceIdentityCreatePage />
    </AuthProvider>
  )
}

describe("ServiceIdentityCreatePage (WEB-7 D2)", () => {
  it("creates the identity, sends the scopes split on commas, and goes back to the list", async () => {
    const writes = stubApi()
    renderPage()
    fireEvent.change(await screen.findByLabelText("Name"), {
      target: { value: " ingestion-worker " },
    })
    fireEvent.click(await screen.findByRole("checkbox", { name: "ingest:write" }))
    fireEvent.click(screen.getByRole("checkbox", { name: "iceberg:read" }))
    fireEvent.change(screen.getByLabelText("Environment"), {
      target: { value: "production" },
    })
    fireEvent.click(screen.getByRole("button", { name: "Create" }))

    await waitFor(() =>
      expect(writes).toEqual([
        {
          method: "POST",
          path: "/api/identity/service-identities",
          body: {
            name: "ingestion-worker",
            scopes: ["ingest:write", "iceberg:read"],
            environment: "production",
          },
        },
      ])
    )
    await waitFor(() => expect(pushed).toEqual(["/admin/service-identities"]))
  })

  it("keeps the page open with the API's sentence when the create is refused", async () => {
    spyOn(globalThis, "fetch").mockImplementation((async (input: RequestInfo | URL, init?: RequestInit) => {
      const path = String(input)
      if (path.includes("/api/auth/me")) {
        return json({ id: "u1", name: "Admin", email: null, roles: [], permissions: ["identity:write"], tenants: [] })
      }
      if (path.includes("/api/identity/permissions")) {
        return json([{ name: "ingest:write", area: "Ingest", description: "Load data." }])
      }
      if (init?.method === "POST") return json({ error: "Two identities cannot share a name." }, 409)
      return json({})
    }) as unknown as typeof fetch)
    renderPage()
    fireEvent.change(await screen.findByLabelText("Name"), { target: { value: "duplicate" } })
    fireEvent.click(await screen.findByRole("checkbox", { name: "ingest:write" }))
    fireEvent.change(screen.getByLabelText("Environment"), { target: { value: "production" } })
    fireEvent.click(screen.getByRole("button", { name: "Create" }))

    expect(await screen.findByRole("alert")).toBeTruthy()
    expect(pushed).toEqual([])
  })

  it("cancel goes back to the list without creating anything", async () => {
    const writes = stubApi()
    renderPage()
    await screen.findByLabelText("Name")
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }))
    expect(pushed).toEqual(["/admin/service-identities"])
    expect(writes).toEqual([])
  })

  // DATA-5 D10: the scopes field says what `iceberg:read` opens, where the
  // door is, and that the key comes from Rotate. An admin who does not
  // know the address cannot connect a tool at all.
  it("names the scope, this console's door address, and Rotate under the scopes field", async () => {
    // happy-dom's default location is `about:blank`, whose origin is
    // the string "null". Give the page a real console address, the way
    // a browser would.
    const originalLocation = window.location
    Object.defineProperty(window, "location", {
      configurable: true,
      writable: true,
      value: {
        origin: "https://console.example.com",
        pathname: "/admin/service-identities/new",
        search: "",
        assign: () => {},
      },
    })
    try {
      stubApi()
      renderPage()
      const address = await screen.findByTestId("outside-reader-address")
      expect(address.textContent).toBe("https://console.example.com/api/iceberg")
      expect(screen.getAllByText("iceberg:read").length).toBeGreaterThan(0)
      expect(screen.getByText(/Use Rotate to get its key/)).toBeDefined()
    } finally {
      Object.defineProperty(window, "location", {
        configurable: true,
        writable: true,
        value: originalLocation,
      })
    }
  })
})
