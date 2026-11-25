import { afterEach, describe, expect, it, mock } from "bun:test"

import { ServiceError } from "../errors"
import { metricService } from "./metrics"

const originalFetch = global.fetch

afterEach(() => {
  global.fetch = originalFetch
})

/** The server's own refusal for a change that needs `metric:write`. */
function refuse(): void {
  global.fetch = mock(async () =>
    new Response(JSON.stringify({ error: "permission_denied: metric:write" }), {
      status: 403,
      headers: { "Content-Type": "application/json" },
    }),
  ) as unknown as typeof fetch
}

// QA LH-BI-2-05 (BI-54, plan D33): the shared mapping words both a metric's
// refusal and a segment's, so a segment's refusal no longer says "metrics".
// (Built with #497's `deniedFor`, which is in the tree.)
describe("a refused change of a metric or a segment (BI-54)", () => {
  it("a refused segment change does not say metrics", async () => {
    refuse()
    const fromSegment = (await metricService
      .createSegment({ mart: "mart_visits" }, "domestic", { label: "Domestic", filters: [] })
      .catch((e: unknown) => e)) as ServiceError
    expect(fromSegment).toBeInstanceOf(ServiceError)
    expect(fromSegment.code).toBe("permission_denied")
    expect(fromSegment.message).toBe("You don't have access. This needs the metric:write permission.")
    expect(fromSegment.message).not.toContain("metrics")

    refuse()
    const fromMetric = (await metricService
      .create({ mart: "mart_visits" }, "visitors", { label: "Visitors", description: "", formula: "sum(visitors)", filters: [] })
      .catch((e: unknown) => e)) as ServiceError
    expect(fromMetric.code).toBe("permission_denied")
    expect(fromMetric.message).toBe("You don't have access. This needs the metric:write permission.")
  })
})
