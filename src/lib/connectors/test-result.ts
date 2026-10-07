import type { ConnectorTestResult } from "@/services/contracts/connectors"

/**
 * What a connection test said, in the three ways it can end. `POST
 * /connectors/{id}/test` answers 200 for all three: a probe that ran and
 * failed, and a connector type this build cannot dial at all, are results,
 * not HTTP errors. So "the request succeeded" says nothing about whether the
 * connector is reachable, and the toast and the notice under the page header
 * both follow this instead.
 */
export type TestOutcome = "passed" | "failed" | "unsupported"

/**
 * `supported` is read first: the contract says `ok` is always `false` for an
 * unsupported type, but an answer that was never a probe must not read as a
 * pass or a failure whatever `ok` holds.
 */
export function testOutcome(result: Pick<ConnectorTestResult, "ok" | "supported">): TestOutcome {
  if (!result.supported) return "unsupported"
  return result.ok ? "passed" : "failed"
}

/** The toast's sentence for each outcome. */
export const TEST_OUTCOME_TOAST: Record<TestOutcome, string> = {
  passed: "Connection test passed",
  failed: "Connection test failed",
  unsupported: "This connector type cannot be tested",
}
