// The audit drawer's "Detail" row (QA LH-DATA-15-03 F1, plan D9): a catalog
// grant's entry says who got which permission; an entry without a detail
// shows no such row.
mock.module("next/navigation", () => ({
  usePathname: () => "/audit",
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

import { cleanup, render, screen, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, mock, spyOn } from "bun:test"
import { TableProviders } from "@/components/app-shell/table-providers"
import type { AuditEvent } from "@/services/contracts/governance"
import { AuditPage, EventDetail } from "./audit-page"

afterEach(() => {
  cleanup()
  mock.restore()
})

const event = (over: Partial<AuditEvent> = {}): AuditEvent => ({
  id: "audit-1",
  at: "2026-11-06T08:00:00.000Z",
  actor: "Platform Admin",
  actorKind: "user",
  tenant: "t1",
  action: "catalog.grant",
  resource: "catalog:silver.orders",
  outcome: "success",
  policyDecision: "allow",
  obligations: [],
  ...over,
})

describe("EventDetail", () => {
  it("shows the detail sentence of a grant under a Detail label", () => {
    render(
      <EventDetail
        event={event({ detail: "Granted query:read to QA Approver <qa-approver@example.test> until 2026-12-06" })}
      />,
    )
    expect(screen.getByText("Detail")).toBeDefined()
    expect(
      screen.getByText("Granted query:read to QA Approver <qa-approver@example.test> until 2026-12-06"),
    ).toBeDefined()
  })

  it("shows no Detail row for an entry that has no detail", () => {
    render(<EventDetail event={event({ action: "catalog.annotate" })} />)
    expect(screen.queryByText("Detail")).toBeNull()
  })
})

// GOV-1 Decision 9: the API answers 403 on the audit list for a person without
// `audit:read`; the page shows the refusal state and no event rows.
describe("AuditPage", () => {
  it("shows the permission refusal, and no events, when the list answers 403", async () => {
    const answer = {
      error: "permission_denied: reading the audit trail needs the audit:read permission",
    }
    spyOn(globalThis, "fetch").mockImplementation((async () =>
      new Response(JSON.stringify(answer), {
        status: 403,
        headers: { "Content-Type": "application/json" },
      })) as unknown as typeof fetch)

    // `TableProviders` supplies the `NuqsAdapter` the data table needs.
    render(
      <TableProviders>
        <AuditPage />
      </TableProviders>,
    )

    await waitFor(() => expect(screen.getByText("You don't have access")).toBeDefined())
    expect(document.body.textContent ?? "").toContain("audit:read")
    expect(document.body.textContent ?? "").not.toMatch(/permission_denied/)
    expect(screen.queryByRole("table")).toBeNull()
    expect(screen.queryByText("catalog.grant")).toBeNull()
  })
})
