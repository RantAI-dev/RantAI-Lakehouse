import type { LakehouseSchemaField, LakehouseSchemaVersion } from "@/services/contracts/lakehouse"

/** One step of a table's schema history, with what changed from the step before. */
export type SchemaChange = {
  schemaId: number
  sinceMs: number | null
  current: boolean
  /** Plain sentences, one per change; never empty. */
  changes: string[]
}

function describe(f: LakehouseSchemaField) {
  return `${f.name} (${f.type})`
}

/**
 * What changed between two schemas of one table. Fields are matched by
 * their Iceberg id, which a column keeps for life — so a rename reads as a
 * rename, not as one column dropped and another added.
 */
function diff(before: LakehouseSchemaField[], after: LakehouseSchemaField[]): string[] {
  const old = new Map(before.map((f) => [f.id, f]))
  const kept = new Set(after.map((f) => f.id))
  const out: string[] = []
  for (const f of after) {
    const was = old.get(f.id)
    if (!was) {
      out.push(`Added ${describe(f)}`)
      continue
    }
    if (was.name !== f.name) out.push(`Renamed ${was.name} to ${f.name}`)
    if (was.type !== f.type) out.push(`Changed ${f.name} from ${was.type} to ${f.type}`)
    if (was.required !== f.required) {
      out.push(`Made ${f.name} ${f.required ? "required" : "nullable"}`)
    }
  }
  for (const f of before) {
    if (!kept.has(f.id)) out.push(`Dropped ${describe(f)}`)
  }
  if (out.length > 0) return out
  // Same columns, same types: only their order can differ.
  const reordered = before.some((f, i) => after[i]?.id !== f.id)
  return [reordered ? "Reordered columns" : "No column changes"]
}

/**
 * A table's schema history as a list of changes, newest first. The oldest
 * version has nothing to be compared with, so it says what it started with.
 */
export function schemaHistory(versions: LakehouseSchemaVersion[]): SchemaChange[] {
  const ordered = [...versions].sort((a, b) => a.schemaId - b.schemaId)
  return ordered
    .map((v, i) => ({
      schemaId: v.schemaId,
      sinceMs: v.sinceMs,
      current: v.current,
      changes:
        i === 0
          ? [`Created with ${v.fields.length} column${v.fields.length === 1 ? "" : "s"}`]
          : diff(ordered[i - 1].fields, v.fields),
    }))
    .reverse()
}
