/**
 * What happened to a WriteHigh step's approval (AI-28 / LH-AI-18-09).
 *
 * The conversation stores the approval request, not its outcome, so a
 * reopened chat used to show every approval as still pending. The state is
 * read again from the run on open and never written back into the
 * conversation; these are the only four states a person can read, and
 * the wording lives here so the card and the test share one source.
 */

export type ApprovalState = "pending" | "ran" | "not_run" | "rejected";

export function approvalStateOf(status: string): ApprovalState {
  if (status === "succeeded") return "ran";
  if (status === "failed" || status === "budget_exhausted") return "not_run";
  if (status === "rejected") return "rejected";
  return "pending";
}

export const APPROVAL_STATE_TEXT: Record<ApprovalState, { badge: string; line: string }> = {
  pending: {
    badge: "pending approval",
    line: "Someone else with approval rights must approve it.",
  },
  ran: { badge: "approved", line: "Approved and run." },
  not_run: { badge: "approved, not run", line: "Approved, but it did not run." },
  rejected: { badge: "rejected", line: "Rejected in Approvals." },
};
