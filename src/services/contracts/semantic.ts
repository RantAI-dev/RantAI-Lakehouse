/**
 * `GET` and `PUT /api/semantic/{asset}`: the meaning and role a drafting
 * pass wrote for a table and its columns, and the person's confirmed
 * corrections. `asset` is `serving.<table>` or `silver.<table>`; the route
 * answers 404 for any other table, so it never has an entry.
 *
 * Field names mirror the API's serde names. A `null` column is the table's
 * own entry.
 */
export type SemanticRole = "measure" | "dimension" | "time" | "key" | "flag" | "non_additive" | string

export type SemanticEntry = {
  asset: string
  /** `null` for the table's own entry. */
  column: string | null
  description: string
  synonyms: string[]
  role: SemanticRole | null
  status: "draft" | "confirmed" | string
  writtenBy: string | null
  model: string | null
  updatedAt: string
  /** Present only when ClickHouse did not answer the staleness check. */
  stale?: boolean
}

export type SemanticEntries =
  | { supported: true; entries: SemanticEntry[] }
  | { supported: false; reason: string }

export type SemanticEntryInput = {
  /** `null` saves the table's own entry. */
  column: string | null
  description: string
  synonyms: string[]
  role: SemanticRole | null
}

export interface SemanticService {
  /** The entries of one asset, the table's own entry first. */
  getEntries(asset: string, signal?: AbortSignal): Promise<SemanticEntries>
  /** Saves an entry and marks it confirmed. Replaces every field of the entry. */
  saveEntry(asset: string, input: SemanticEntryInput, signal?: AbortSignal): Promise<void>
}
