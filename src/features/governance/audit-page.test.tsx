// The audit drawer's "Detail" row (QA LH-DATA-15-03 F1, plan D9): a catalog
// grant's entry says who got which permission; an entry without a detail
// shows no such row.
import { render, screen } from "@testing-library/react"
import { describe, expect, it } from "bun:test"
import type { AuditEvent } from "@/services/contracts/governance"
import { EventDetail } from "./audit-page"

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
