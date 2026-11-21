import { describe, expect, it } from "bun:test"
import type { TargetLag } from "@/services/contracts/pipelines"
import { parseSchedule, triggerSummary } from "./pipeline-schedule"

describe("triggerSummary (BLD-11 F2/D3)", () => {
  it("a 10-minute target lag reads 'Within 10 min of its inputs' over a manual schedule", () => {
    const summary = triggerSummary(parseSchedule("manual"), { kind: "lag", minutes: 10 })
    expect(summary.kind).toBe("lag")
    expect(summary.label).toBe("Within 10 min of its inputs")
  })

  it("a target lag wins over a stored cron", () => {
    const summary = triggerSummary(parseSchedule("cron: 0 * * * * (RUNNING)"), {
      kind: "lag",
      minutes: 10,
    } satisfies TargetLag)
    expect(summary.kind).toBe("lag")
    expect(summary.label).toBe("Within 10 min of its inputs")
    expect(summary.label).not.toContain("cron")
  })

  it("a downstream target lag reads 'When a downstream pipeline needs it'", () => {
    const summary = triggerSummary(parseSchedule("manual"), { kind: "downstream" })
    expect(summary.kind).toBe("downstream")
    expect(summary.label).toBe("When a downstream pipeline needs it")
  })

  it("120 minutes reads as 2 h", () => {
    const summary = triggerSummary(parseSchedule("manual"), { kind: "lag", minutes: 120 })
    expect(summary.label).toBe("Within 2 h of its inputs")
  })

  it("no target lag keeps today's manual and cron labels", () => {
    expect(triggerSummary(parseSchedule("manual"), null)).toEqual({
      kind: "manual",
      label: "Manual only",
    })
    expect(triggerSummary(parseSchedule("cron: 0 * * * * (RUNNING)"), null)).toEqual({
      kind: "cron",
      label: "Hourly",
    })
  })

  it("an unknown lag kind is named rather than invented", () => {
    const summary = triggerSummary(parseSchedule("manual"), { kind: "future-kind" })
    expect(summary.kind).toBe("unknown-lag")
    expect(summary.label).toBe("Target lag (unknown kind)")
  })
})
