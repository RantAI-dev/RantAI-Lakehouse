import type { AssetDetail } from "@/services/contracts/assets"
import { lineageKey } from "./asset-lineage"

/**
 * The builders every policy made on an asset's page goes through: the
 * Access tab's Add policy dialog and the Schema tab's column "Add a mask"
 * (`WEB-3` S3-D1). One module, so both write the same enforceable condition
 * (`policy_engine::PolicyCondition`) for the same table (rule 4).
 */

/**
 * The table a policy must bind to govern this asset's rows: the one its
 * reads actually go to (`queryTarget` — a Bronze dataset with a Silver
 * model is read from Silver), named the way policies name tables.
 */
export function policyTable(a: AssetDetail): string {
  if (a.queryTarget?.policyTable) return a.queryTarget.policyTable
  const target = a.queryTarget?.table
  if (!target) return lineageKey(a)
  // An API build older than `policyTable`: icecat_api.`bronze.orders` and
  // silver.`orders` both end in zone.table.
  return target.replace(/`/g, "").split(".").slice(-2).join(".")
}

export type PolicyForm = { roles: string; mask: string[]; rowFilter: string }

/**
 * The role names a policy form's comma string holds, trimmed and without
 * blanks. One reader for the form's own checkboxes (`asset-access.tsx`,
 * `WEB-8` D8) and for the condition written from it, so the two cannot
 * disagree about what a typed "A, ,B" means.
 */
export function roleNames(roles: string): string[] {
  return roles
    .split(",")
    .map((r) => r.trim())
    .filter((r) => r.length > 0)
}

/**
 * The enforceable condition (`policy_engine::PolicyCondition`) the form
 * describes, or `null` while it describes none: a policy needs a role,
 * and at least one thing to do — a column to mask or a row filter.
 */
export function policyCondition(table: string, f: PolicyForm) {
  const roles = roleNames(f.roles)
  const rowFilter = f.rowFilter.trim()
  if (roles.length === 0 || (f.mask.length === 0 && rowFilter === "")) return null
  return { roles, table, mask: f.mask, ...(rowFilter ? { rowFilter } : {}) }
}
