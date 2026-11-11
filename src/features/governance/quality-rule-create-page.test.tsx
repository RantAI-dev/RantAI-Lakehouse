// WEB-7 T3 (D2): the add-quality-rule form is its own page. These tests pin
// what it sends — the threshold the check picker builds, and the typed
// threshold when that is chosen — and DATA-14 (AC1): the new rule runs once.
let pushed: string[] = []
mock.module("next/navigation", () => ({
  usePathname: () => "/governance/data-quality/rules/new",
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
import { QualityRuleCreatePage } from "./quality-rule-create-page"

afterEach(() => {
  cleanup()
  mock.restore()
})

beforeEach(() => {
  pushed = []
})

const json = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } })

/** Answers the session; records every write. */
function stubApi(permissions: string[] = ["governance:write", "query:read"]) {
  const writes: { method: string; path: string; body?: unknown }[] = []
  spyOn(globalThis, "fetch").mockImplementation((async (
    input: RequestInfo | URL,
    init?: RequestInit
  ) => {
    const path = String(input)
    const method = init?.method ?? "GET"
    if (path.includes("/api/auth/me")) {
      return json({ id: "u1", name: "Admin", email: null, roles: [], permissions, tenants: [] })
    }
    writes.push({
      method,
      path,
      body: init?.body ? JSON.parse(String(init.body)) : undefined,
    })
    if (method === "POST" && path.endsWith("/api/governance/quality")) {
      return json({
        id: "r1",
        name: "email_filled",
        asset: "silver.orders",
        dimension: "completeness",
        threshold: "email not null",
        severity: "medium",
        lastStatus: null,
        lastRunAt: null,
        lastValue: null,
        evaluable: true,
        hint: null,
      })
    }
    if (path.endsWith("/run")) {
      return json({
        ruleId: "r1",
        status: "passed",
        value: "100.0% not null",
        ranAt: "2026-11-11T00:00:00Z",
      })
    }
    return json({})
  }) as unknown as typeof fetch)
  return writes
}

function renderPage() {
  return render(
    <AuthProvider>
      <QualityRuleCreatePage />
    </AuthProvider>
  )
}

describe("QualityRuleCreatePage (WEB-7 D2/T3)", () => {
  it("builds the threshold from the picked check and goes back to Data Quality", async () => {
    const writes = stubApi()
    renderPage()
    fireEvent.change(await screen.findByLabelText("Name"), { target: { value: "email_filled" } })
    fireEvent.change(screen.getByLabelText("Asset"), { target: { value: "silver.orders" } })
    fireEvent.change(screen.getByLabelText("Column"), { target: { value: "email" } })
    fireEvent.click(screen.getByRole("button", { name: "Add rule" }))

    await waitFor(() =>
      expect(writes[0]).toEqual({
        method: "POST",
        path: "/api/governance/quality",
        body: {
          name: "email_filled",
          asset: "silver.orders",
          dimension: "completeness",
          threshold: "email not null",
          severity: "medium",
        },
      })
    )
    // DATA-14 (AC1): the rule is run once, right after it is created.
    await waitFor(() =>
      expect(writes.map((w) => `${w.method} ${w.path}`)).toContain(
        "POST /api/governance/quality/r1/run"
      )
    )
    await waitFor(() => expect(pushed).toEqual(["/governance/data-quality"]))
  })

  it("sends the threshold the person typed when they choose that", async () => {
    const writes = stubApi(["governance:write"])
    renderPage()
    fireEvent.change(await screen.findByLabelText("Name"), { target: { value: "custom" } })
    fireEvent.change(screen.getByLabelText("Asset"), { target: { value: "silver.orders" } })
    fireEvent.click(screen.getByRole("button", { name: "Write the threshold myself" }))
    fireEvent.change(screen.getByLabelText("Threshold"), {
      target: { value: "rows >= 10" },
    })
    fireEvent.change(screen.getByLabelText("Dimension"), {
      target: { value: "volume" },
    })
    fireEvent.click(screen.getByRole("button", { name: "Add rule" }))

    await waitFor(() =>
      expect(writes[0]).toEqual({
        method: "POST",
        path: "/api/governance/quality",
        body: {
          name: "custom",
          asset: "silver.orders",
          dimension: "volume",
          threshold: "rows >= 10",
          severity: "medium",
        },
      })
    )
    // No query:read, so the page does not try the first run.
    expect(writes.some((w) => w.path.endsWith("/run"))).toBe(false)
  })

  it("keeps the API's sentence on the page when the rule is refused", async () => {
    spyOn(globalThis, "fetch").mockImplementation((async (input: RequestInfo | URL, init?: RequestInit) => {
      const path = String(input)
      if (path.includes("/api/auth/me")) {
        return json({ id: "u1", name: "Admin", email: null, roles: [], permissions: ["governance:write"], tenants: [] })
      }
      if (init?.method === "POST") {
        return json({ error: "threshold: silver.orders is not a table this deployment has" }, 400)
      }
      return json({})
    }) as unknown as typeof fetch)
    renderPage()
    fireEvent.change(await screen.findByLabelText("Name"), { target: { value: "email_filled" } })
    fireEvent.change(screen.getByLabelText("Asset"), { target: { value: "silver.orders" } })
    fireEvent.change(screen.getByLabelText("Column"), { target: { value: "email" } })
    fireEvent.click(screen.getByRole("button", { name: "Add rule" }))

    expect(await screen.findByRole("alert")).toBeTruthy()
    expect(pushed).toEqual([])
  })

  it("cancel goes back to Data Quality without creating anything", async () => {
    const writes = stubApi()
    renderPage()
    await screen.findByLabelText("Name")
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }))
    expect(pushed).toEqual(["/governance/data-quality"])
    expect(writes).toEqual([])
  })
})
