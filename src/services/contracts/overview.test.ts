import { describe, expect, it } from "bun:test"
import type { AlertItem, OverviewService } from "@/services/contracts/overview"
import { clickhouseOverviewService } from "@/services/clients/overview"

describe("overview contracts (WS5 item C2)", () => {
  it("OverviewService exposes silenceAlert, mirroring acknowledgeAlert/resolveAlert's shape", () => {
    // A fixture typed as the full interface fails to compile until
    // `silenceAlert` is added to `OverviewService` — this is the type-level
    // half of Step 1's assertion.
    const svc: Pick<OverviewService, "silenceAlert"> = clickhouseOverviewService
    expect(typeof svc.silenceAlert).toBe("function")
  })


  it("AlertItem accepts a null severity — genuinely unset, not un-loaded", () => {
    const fixture: AlertItem = {
      id: "a1",
      title: "Freshness breach",
      severity: null,
      source: "dagster",
      affected: "gold.visitors",
      status: "open",
      at: "2026-09-17T00:00:00Z",
      detail: "Asset is stale",
    }
    expect(fixture.severity).toBeNull()
  })

  it("AlertItem's ruleId/firedAt/silencedUntil are absent for an alert fired before this migration landed", () => {
    const fixture: AlertItem = {
      id: "a2",
      title: "Legacy alert",
      severity: "high",
      source: "dagster",
      affected: "gold.visitors",
      status: "acknowledged",
      at: "2026-09-16T00:00:00Z",
      detail: "Pre-migration instance",
    }
    expect(fixture.ruleId).toBeUndefined()
    expect(fixture.firedAt).toBeUndefined()
    expect(fixture.silencedUntil).toBeUndefined()
  })

  it("AlertItem's ruleId/firedAt/silencedUntil round-trip when present", () => {
    const fixture: AlertItem = {
      id: "a3",
      title: "Silenced alert",
      severity: "medium",
      source: "dagster",
      affected: "gold.visitors",
      status: "open",
      at: "2026-09-17T01:00:00Z",
      detail: "Silenced for an hour",
      ruleId: "rule-1",
      firedAt: "2026-09-17T01:00:00Z",
      silencedUntil: "2026-09-17T02:00:00Z",
    }
    expect(fixture.ruleId).toBe("rule-1")
    expect(fixture.firedAt).toBe("2026-09-17T01:00:00Z")
    expect(fixture.silencedUntil).toBe("2026-09-17T02:00:00Z")
  })
})
