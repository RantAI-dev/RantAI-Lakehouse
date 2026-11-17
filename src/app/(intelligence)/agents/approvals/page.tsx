import { redirect } from "next/navigation"

/**
 * Legacy route compatibility redirect (`WEB-3` D4): the approvals inbox
 * moved to `/governance/approvals`. Kept redirect-only, the same way
 * `/audit-logs` stands in for `/audit`.
 */
export default function LegacyRedirect() {
  redirect("/governance/approvals")
}
