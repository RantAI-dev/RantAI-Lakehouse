// SEC-22 (F4): the API refuses adding a rule without governance:write, so
// the "Add Quality Rule" button shows only with it, as the row actions beside it do.
// `AuthProvider` and the table's URL state need `next/navigation`; stub it
// before any import resolves (`mock.module` is hoisted).
mock.module("next/navigation", () => ({
  usePathname: () => "/governance/data-quality",
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

import { cleanup, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it, mock, spyOn } from "bun:test"
import { TableProviders } from "@/components/app-shell/table-providers"
import { AuthProvider } from "@/features/auth/auth-provider"
import { DataQualityPage } from "./data-quality-page"

afterEach(() => {
  cleanup()
  mock.restore()
})

const json = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } })

/** Answers the session with `permissions`; resolves `sessionRead` once it has. */
function stubApi(permissions: string[]) {
  let sessionRead: () => void = () => {}
  const done = new Promise<void>((resolve) => {
    sessionRead = resolve
  })
  spyOn(globalThis, "fetch").mockImplementation((async (input: RequestInfo | URL) => {
    const path = String(input)
    if (path.includes("/api/auth/me")) {
      const answer = json({ id: "u1", name: "Reader", email: null, roles: ["Analyst"], permissions, tenants: [] })
      sessionRead()
      return answer
    }
    if (path.includes("/api/governance/quality")) return json({ quality: [] })
    return json({ error: "not stubbed" }, 404)
  }) as unknown as typeof fetch)
  return done
}

function renderPage() {
  return render(
    <AuthProvider>
      <TableProviders>
        <DataQualityPage />
      </TableProviders>
    </AuthProvider>
  )
}

/** Lets the session answer and React apply it, so "no button" is not just "not loaded yet". */
async function sessionSettled(done: Promise<void>) {
  await done
  await new Promise((resolve) => setTimeout(resolve, 50))
}

describe("DataQualityPage: adding a rule", () => {
  it("shows the button with governance:write", async () => {
    stubApi(["governance:write"])
    renderPage()
    expect(await screen.findByRole("button", { name: "Add Quality Rule" })).toBeTruthy()
  })

  it("hides the button without governance:write", async () => {
    const done = stubApi(["catalog:read", "policy:read", "query:read"])
    renderPage()
    await sessionSettled(done)
    expect(screen.getByText("Data Quality")).toBeTruthy()
    expect(screen.queryByRole("button", { name: "Add Quality Rule" })).toBeNull()
  })
})
