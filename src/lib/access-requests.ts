/**
 * Pure logic for the "request access" flow (WS7 item E5) — request-access
 * button on the catalog asset view, and the access-request rows the
 * approvals inbox renders alongside its existing tool-call rows.
 *
 * Kept out of the pages themselves so the three-state rendering rule
 * (pending / approved-with-expiry / rejected must never be conflated) and
 * the self-approval disable condition are each testable without a DOM.
 */

import type { ApprovalStatus } from "./status"

/**
 * The UI-facing state of a `kind = "access"` approval row — distinct from
 * the bare `ApprovalStatus` because "approved" alone does not say whether
 * the grant it produced is still active.
 *
 * `"approved-active"` covers both a grant whose `expiresAt` is still in
 * the future AND the honest case where no `expiresAt` is known at all
 * (see `accessGrantState`'s doc comment) — never fabricated as
 * `"approved-expired"` for lack of data.
 */
export type AccessGrantState = "pending" | "approved-active" | "approved-expired" | "rejected"

/**
 * Classifies one access request's real-world state.
 *
 * `expiresAt` is `undefined` in two legitimate cases that must render
 * identically ("approved", no expiry claim): a request just approved with
 * no grant days configured (never happens today — `GRANT_DAYS` is fixed —
 * but the type allows it), and — the common case — a row read back from
 * `GET /api/agents/approvals`, which lists `approval_item` rows only and
 * never joins `access_grant` (WS7 item E1's migration keeps a grant's own
 * expiry off the approval row). Only the immediate response of `POST
 * .../decide` carries the grant's `expiresAt`. Fabricating an expiry
 * where none is known would violate "never fabricate" (AGENTS.md
 * principle 2); showing plain "Approved" instead is the honest choice.
 *
 * The expiry boundary matches the backend's own grant-validity check
 * (`lakehouse_auth::load_principal_for_user`: `expires_at > now()`) — an
 * expiry exactly at `nowMs` is already expired, not still active.
 */
export function accessGrantState(
  status: ApprovalStatus,
  expiresAt: string | undefined,
  nowMs: number
): AccessGrantState {
  if (status === "pending") return "pending"
  if (status === "rejected") return "rejected"
  if (expiresAt !== undefined && Date.parse(expiresAt) <= nowMs) return "approved-expired"
  return "approved-active"
}

/**
 * The fields of an `ApprovalItem` the self-approval check actually reads.
 * Kept narrow (rather than the whole contract type) so this module has no
 * dependency on `@/services/contracts/agents`.
 */
export type SelfApprovalCandidate = {
  kind: "tool_call" | "access"
  status: ApprovalStatus
  requestedByUserId?: string
}

/**
 * True exactly when deciding this row would be a self-approval the
 * backend refuses with 403 (`routes::catalog::decide_access_request`,
 * WS7 item E3) — a `kind = "access"` row, still `pending`, whose
 * `requestedByUserId` is the signed-in user. This client-side check is
 * belt-and-suspenders, never the security boundary: the server-side
 * refusal in `decide_access_request` is what actually enforces it.
 *
 * Excludes already-decided rows: the decide button is not offered at all
 * once a request is no longer pending, so the disable reason would never
 * apply there.
 */
export function isOwnAccessRequest(approval: SelfApprovalCandidate, currentUserId: string): boolean {
  return (
    approval.kind === "access" &&
    approval.status === "pending" &&
    approval.requestedByUserId === currentUserId
  )
}

/**
 * Narrows a fixed candidate list down to the permissions the principal
 * does not already hold — a permission already granted is never offered
 * for request (requesting one you already have 400s server-side, per
 * `routes::catalog::access_request`'s own check).
 */
export function requestableAccessPermissions(
  candidates: readonly string[],
  hasPermission: (permission: string) => boolean
): string[] {
  return candidates.filter((p) => !hasPermission(p))
}
