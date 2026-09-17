import { describe, expect, it } from "bun:test"
import type { Policy } from "@/services/contracts/governance"

// WS7 item A2: `Policy.conditions` is new on the read path. The plan text
// assumed an existing governance contract test file to extend; none
// existed (only `connectors.test.ts`/`overview.test.ts` did) — this file
// is new, following that same fixture-round-trip pattern.
describe("governance contracts", () => {
  it("Policy round-trips with conditions undefined for a legacy policy authored before WS7", () => {
    const fixture: Policy = {
      id: "p1",
      name: "n",
      status: "ready",
      kind: "Row filter",
      subjects: "s",
      resources: "r",
      effect: "Permit with obligation",
      version: 1,
      owner: "o",
      updatedAt: "2026-01-01T00:00:00.000Z",
    }
    expect(fixture.conditions).toBeUndefined()
  })

  it("Policy carries a structured conditions blob as a raw, unparsed string", () => {
    const fixture: Policy = {
      id: "p2",
      name: "n",
      status: "ready",
      kind: "Row filter",
      subjects: "s",
      resources: "r",
      effect: "Permit with obligation",
      version: 1,
      owner: "o",
      updatedAt: "2026-01-01T00:00:00.000Z",
      conditions: '{"roles":["Analyst"],"table":"serving.mart_x","mask":["email"]}',
    }
    expect(fixture.conditions).toContain("Analyst")
  })
})
