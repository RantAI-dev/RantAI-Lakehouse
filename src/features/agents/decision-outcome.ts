/**
 * What the Approvals page says after a decision (AI-28 / LH-AI-18-07).
 *
 * The decide route answers 200 for a decision it recorded even when the
 * tool could not run, so the page must say what happened rather than show
 * the toast's old "Decision recorded — not executed" for every outcome.
 * Keeping the wording in a pure function makes every branch testable and
 * keeps the cases in one place. AI-28 review R1: an executed tool can still
 * fail (its result carries `error`), which is not the same as "ran and
 * succeeded" nor as "not run".
 */

type DecisionOutcome = {
  readonly executed: boolean
  readonly result?: unknown
  readonly notExecuted?: { readonly reason: string; readonly required: string }
}

export function decisionOutcomeText(
  r: DecisionOutcome,
  decision: "approved" | "rejected"
): string {
  if (r.executed) {
    // AI-28 review R1: the route marks an executed tool that returned an
    // error as failed in the run and the audit trail; the drawer must not
    // call that "Approved and run." The route uses the same `error` key.
    if (typeof r.result === "object" && r.result !== null && "error" in r.result) {
      return "Approved; the action ran and failed."
    }
    return "Approved and run."
  }
  if (r.notExecuted?.reason === "permission") {
    return `Approved, but not run: you do not have the permission this action needs (${r.notExecuted.required}).`
  }
  if (decision === "rejected") return "Rejected. Nothing ran."
  return "Decision recorded. Nothing ran."
}
