import { listOf } from "@/lib/column-masks"
import type { Policy } from "@/services/contracts/governance"

/**
 * What one access rule does, for the read-only Access rules list
 * (`WEB-3` S3-D4): the table it governs, the roles it applies to, and
 * whether it masks columns or filters rows.
 *
 * Whether a rule is enforced is the API's `enforceable` (`SEC-61`,
 * computed by the engine's own parser), never decided here. This module
 * only reads the structured fields out of a rule the API already called
 * enforceable; for any other rule the stored `subjects` and `resources`
 * text is shown as it is, and "does" says it does nothing, because the
 * engine skips it.
 */
export type AccessRule = Policy & {
  /** The governed table (`silver.orders`), or the stored resources text. */
  table: string
  /** The roles it applies to, or the stored subjects text. */
  roles: string
  /** "Masks email and phone", "Filters rows", or why it does nothing. */
  does: string
}

type Clause = { roles: string[]; table: string; mask: string[]; rowFilter: string | null }

/** The structured clause of an enforceable rule, or `null` when there is none to read. */
export function ruleClause(p: Pick<Policy, "enforceable" | "conditions">): Clause | null {
  if (!p.enforceable || !p.conditions) return null
  let raw: unknown
  try {
    raw = JSON.parse(p.conditions)
  } catch {
    return null
  }
  if (!raw || typeof raw !== "object") return null
  const o = raw as Record<string, unknown>
  if (typeof o.table !== "string" || !Array.isArray(o.roles)) return null
  const strings = (v: unknown) => (Array.isArray(v) ? v.filter((x): x is string => typeof x === "string") : [])
  const rowFilter = typeof o.rowFilter === "string" && o.rowFilter.trim() !== "" ? o.rowFilter : null
  return { roles: strings(o.roles), table: o.table, mask: strings(o.mask), rowFilter }
}

/** "Masks email and phone; filters rows" — what an enforceable clause does. */
export function clauseSentence(c: Pick<Clause, "mask" | "rowFilter">): string {
  const parts: string[] = []
  if (c.mask.length > 0) parts.push(`masks ${listOf(c.mask)}`)
  if (c.rowFilter) parts.push("filters rows")
  const s = parts.join("; ")
  return s.charAt(0).toUpperCase() + s.slice(1)
}

/** The row the Access rules list shows for one policy. */
export function accessRule(p: Policy): AccessRule {
  const clause = ruleClause(p)
  if (!clause) {
    return {
      ...p,
      table: p.resources,
      roles: p.subjects,
      does: "Nothing: written as text, so the engine skips it",
    }
  }
  return { ...p, table: clause.table, roles: clause.roles.join(", "), does: clauseSentence(clause) }
}

/**
 * The table's Access tab, where a rule is changed, or `null` when the API
 * named no asset for it (unparseable conditions, or a table the catalog
 * does not address): then the table is plain text, never a guessed link.
 */
export function accessTabHref(p: Pick<Policy, "assetId">): string | null {
  return p.assetId ? `/data/assets/${encodeURIComponent(p.assetId)}?tab=access` : null
}
