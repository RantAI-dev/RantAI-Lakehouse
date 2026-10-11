/**
 * Pure helpers for the meaning of a table and its columns: which catalog
 * keys have an entry, how the entries split into the table's own and one per
 * column, how many columns still await a person's confirmation, and how a
 * save body is built so no save drops a field. No React here, so each rule
 * is tested without a DOM.
 */
import type { SemanticEntry, SemanticEntryInput, SemanticRole } from "@/services/contracts/semantic"

/** The keys the semantic route serves; any other table answers 404 and never has an entry. */
const SEMANTIC_PREFIXES = ["serving.", "silver."]

export function semanticAsset(key: string): string | null {
  return SEMANTIC_PREFIXES.some((p) => key.startsWith(p)) ? key : null
}

export function splitEntries(entries: SemanticEntry[]): {
  table: SemanticEntry | null
  byColumn: Map<string, SemanticEntry>
} {
  let table: SemanticEntry | null = null
  const byColumn = new Map<string, SemanticEntry>()
  for (const e of entries) {
    if (e.column === null) table = e
    else byColumn.set(e.column, e)
  }
  return { table, byColumn }
}

/**
 * Counts only the rows whose column is in the table. An entry whose column
 * has no row is not a draft the person can see, so it is not counted.
 */
export function draftCount(
  rows: ReadonlyArray<{ column: { name: string } }>,
  byColumn: Map<string, SemanticEntry>
): number {
  return rows.filter((r) => byColumn.get(r.column.name)?.status === "draft").length
}

const MAX_SYNONYMS = 6
const MAX_SYNONYM_LENGTH = 40

/**
 * Splits on commas only, since a synonym can hold a space ("total revenue").
 * The shape copies `parseTags` in `asset-about.tsx`.
 */
export function parseSynonyms(raw: string): { synonyms: string[] } | { error: string } {
  const synonyms = [
    ...new Set(
      raw
        .split(",")
        .map((s) => s.trim())
        .filter((s) => s.length > 0)
    ),
  ]
  // The API counts code points, so a count by `length` would disagree for a character outside the BMP (plan 10 review 3).
  const long = synonyms.find((s) => [...s].length > MAX_SYNONYM_LENGTH)
  if (long) {
    return { error: `"${long}" is longer than ${MAX_SYNONYM_LENGTH} characters.` }
  }
  if (synonyms.length > MAX_SYNONYMS) return { error: `At most ${MAX_SYNONYMS} names.` }
  return { synonyms }
}

/**
 * The body of a save for this entry. Every save starts here so a field that
 * is not being changed is sent back as it was, never silently dropped.
 */
export function inputOf(entry: SemanticEntry, over: Partial<SemanticEntryInput> = {}): SemanticEntryInput {
  return {
    column: entry.column,
    description: entry.description,
    synonyms: entry.synonyms,
    role: entry.role,
    ...over,
  }
}

/**
 * What the Schema tab knows about the meanings of its table, as one value the
 * components take. `none` is a table the semantic route never serves: the
 * console makes no request and shows no meaning at all.
 */
export type MeaningState =
  | { kind: "none" }
  | { kind: "loading" }
  | { kind: "error"; message: string; retry: () => void }
  | { kind: "unsupported"; reason: string }
  | {
      kind: "ready"
      table: SemanticEntry | null
      byColumn: Map<string, SemanticEntry>
      canEdit: boolean
      /** Saves one entry. The components call nothing else to write. */
      save: (input: SemanticEntryInput, signal?: AbortSignal) => Promise<void>
      reload: () => void
    }

export type RoleOption = { value: SemanticRole; label: string; help: string }

export const ROLE_OPTIONS: readonly RoleOption[] = [
  { value: "measure", label: "Measure", help: "A number that can be added up across rows." },
  {
    value: "non_additive",
    label: "Number, not to be added up",
    help: "A count of distinct things, a rate, a share, an average or a balance.",
  },
  { value: "dimension", label: "Dimension", help: "A value to group or filter by." },
  { value: "time", label: "Time", help: "A date or a period." },
  { value: "key", label: "Key", help: "An identifier that joins this table to another." },
  { value: "flag", label: "Flag", help: "A yes or no value." },
]

/**
 * A role the options do not list is shown as its own text, so a role written
 * by a later version of the drafting pass is still readable.
 */
export function roleLabel(role: SemanticRole | null): string {
  if (role === null) return "Not set"
  return ROLE_OPTIONS.find((o) => o.value === role)?.label ?? role
}
