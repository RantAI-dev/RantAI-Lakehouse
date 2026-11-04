/**
 * Which enforced policies mask one column, for the column's "Sensitivity
 * and masking" section (`WEB-3` S3-D1). The facts come from the asset's
 * `policySummary` (`routes::catalog_governance`); nothing here guesses.
 *
 * A policy that masks this column and nothing else can be removed from the
 * column (`own`). A policy that also masks other columns, or also filters
 * rows, does more than this column, so deleting it from here would take
 * away protection the reader is not looking at: it is `shared`, and the
 * section points to the Access tab instead. A draft (`status` other than
 * `ready`) masks nothing, so it is not listed as masking the column.
 */

export type MaskPolicy = {
  id: string
  name: string
  status?: string
  roles?: string[]
  mask?: string[]
  rowFilter?: string | null
}

export type ColumnMask<P extends MaskPolicy = MaskPolicy> =
  | { kind: "own"; policy: P }
  | { kind: "shared"; policy: P; otherColumns: string[]; filtersRows: boolean }

/** The enforced policies that mask `column`, each marked removable here or not. */
export function columnMasks<P extends MaskPolicy>(policies: readonly P[], column: string): ColumnMask<P>[] {
  return policies
    .filter((p) => p.status === "ready" && (p.mask ?? []).includes(column))
    .map((policy) => {
      const otherColumns = (policy.mask ?? []).filter((c) => c !== column)
      const filtersRows = Boolean(policy.rowFilter && policy.rowFilter.trim() !== "")
      return otherColumns.length === 0 && !filtersRows
        ? { kind: "own" as const, policy }
        : { kind: "shared" as const, policy, otherColumns, filtersRows }
    })
}

/** "Part of pii_mask, which also masks phone and address" — what else a shared policy does. */
export function sharedMaskSentence(mask: Extract<ColumnMask, { kind: "shared" }>): string {
  const parts: string[] = []
  if (mask.otherColumns.length > 0) parts.push(`masks ${listOf(mask.otherColumns)}`)
  if (mask.filtersRows) parts.push("filters rows")
  return `Part of ${mask.policy.name}, which also ${parts.join(" and ")}.`
}

function listOf(items: string[]): string {
  if (items.length <= 1) return items.join("")
  return `${items.slice(0, -1).join(", ")} and ${items[items.length - 1]}`
}
