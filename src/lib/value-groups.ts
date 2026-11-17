import { MAX_GROUPS, MAX_GROUP_VALUES, type GroupDef, type ValueGroup } from "@/services/contracts/calc-fields"

/**
 * The editor of a value group (`BI-10`): chosen values of a text column
 * gathered under names. The pure part, so each rule the server also enforces
 * (`lakehouse_bi::groups`) is checked while the person edits and tested here:
 * a value belongs to one group only, at most 50 groups and 2,000 values.
 */

/** The group a value is in, or -1. */
export function groupOf(groups: readonly ValueGroup[], value: string): number {
  return groups.findIndex((g) => g.values.includes(value))
}

/** How many values all the groups hold. */
export function valueCount(groups: readonly ValueGroup[]): number {
  return groups.reduce((n, g) => n + g.values.length, 0)
}

/**
 * Put `value` in group `active`, or take it out when it is already there. A
 * value in another group is left where it is (the list shows which group
 * holds it); the total cap stops an add.
 */
export function toggleValue(groups: readonly ValueGroup[], active: number, value: string): ValueGroup[] {
  const holder = groupOf(groups, value)
  if (holder === active) {
    return groups.map((g, i) => (i === active ? { ...g, values: g.values.filter((v) => v !== value) } : g))
  }
  if (holder !== -1 || active < 0 || active >= groups.length || valueCount(groups) >= MAX_GROUP_VALUES) return [...groups]
  return groups.map((g, i) => (i === active ? { ...g, values: [...g.values, value] } : g))
}

/** A new empty group named after how many there are, and not already taken. */
export function addGroup(groups: readonly ValueGroup[]): ValueGroup[] {
  if (groups.length >= MAX_GROUPS) return [...groups]
  let n = groups.length + 1
  while (groups.some((g) => g.name === `Group ${n}`)) n += 1
  return [...groups, { name: `Group ${n}`, values: [] }]
}

/** What is still needed before the group can be saved, in the editor's words. */
export function groupProblems(name: string, column: string, groups: readonly ValueGroup[], rest: string): string[] {
  const out: string[] = []
  if (!name.trim()) out.push("a name for the field")
  if (!column) out.push("a column")
  if (!groups.length) out.push("a group")
  const names = groups.map((g) => g.name.trim())
  if (names.some((n) => !n)) out.push("a name for every group")
  if (new Set(names).size !== names.length) out.push("different names for the groups")
  if (groups.some((g) => !g.values.length)) out.push("values in every group")
  if (rest === "other" && names.includes("Other")) out.push("no group called Other while the rest is called Other")
  return out
}

/** The definition to send: names trimmed, empty groups dropped by the check above, not here. */
export function groupPayload(column: string, groups: readonly ValueGroup[], rest: "keep" | "other"): GroupDef {
  return { column, rest, groups: groups.map((g) => ({ name: g.name.trim(), values: [...g.values] })) }
}
