/**
 * What happened to a WriteHigh step's approval (AI-28 / LH-AI-18-09).
 *
 * The conversation stores the approval request, not its outcome, so a
 * reopened chat used to show every approval as still pending. The state is
 * read again from the run on open and never written back into the
 * conversation; the wording lives here so the card and the test share one
 * source. AI-28 review R1: the decide route records `failed` both when the
 * tool was never run ("Not executed") and when it ran and returned an error
 * ("Executed after approval"), so the last step tells those two apart.
 */

export type ApprovalState = "pending" | "ran" | "not_run" | "rejected" | "failed";

export function approvalStateOf(status: string, lastStepLabel?: string): ApprovalState {
  if (status === "succeeded") return "ran";
  if (status === "failed") {
    // AI-28 review R1: a failed run whose last step is the execution is not
    // "not run" — the action ran and failed. A missing label is the older
    // shape and falls back to the conservative "not run" reading.
    return lastStepLabel === "Executed after approval" ? "failed" : "not_run";
  }
  if (status === "budget_exhausted") return "not_run";
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
  // AI-28 review R1: the action was executed after approval and failed.
  failed: { badge: "approved, action failed", line: "Approved; the action ran and failed." },
};
