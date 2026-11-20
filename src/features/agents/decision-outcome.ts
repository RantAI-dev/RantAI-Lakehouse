/**
 * What the Approvals page says after a decision (AI-28 / LH-AI-18-07).
 *
 * The decide route answers 200 for a decision it recorded even when the
 * tool could not run, so the page must say what happened rather than show
 * the toast's old "Decision recorded — not executed" for every outcome.
 * Keeping the wording in a pure function makes every branch testable and
 * keeps the four cases in one place.
 */

type DecisionOutcome = {
  readonly executed: boolean
  readonly notExecuted?: { readonly reason: string; readonly required: string }
}

export function decisionOutcomeText(
  r: DecisionOutcome,
  decision: "approved" | "rejected"
): string {
  if (r.executed) return "Approved and run."
  if (r.notExecuted?.reason === "permission") {
    return `Approved, but not run: you do not have the permission this action needs (${r.notExecuted.required}).`
  }
  if (decision === "rejected") return "Rejected. Nothing ran."
  return "Decision recorded. Nothing ran."
}
