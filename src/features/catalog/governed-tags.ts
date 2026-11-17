/**
 * Governed tags (`DATA-12` part 2) as the asset pages show and send them.
 * Pure, so the rules the About editor and the Data Explorer share live in one
 * place and are tested without a DOM.
 */

/** `["data_domain: finance", ...]`, sorted by key: how a pair reads as a chip and in the Tags column. */
export function governedPairLabels(governed: Record<string, string> | undefined | null): string[] {
  return Object.entries(governed ?? {})
    .sort(([a], [b]) => a.localeCompare(b))
    .map(([key, value]) => `${key}: ${value}`)
}

/**
 * The set to send on a save: the keys the editor chose a value for. A key left
 * on "none" is not in it, so saving removes that key from the table, which is
 * what "none" means.
 */
export function chosenGovernedTags(chosen: Record<string, string>): Record<string, string> {
  return Object.fromEntries(Object.entries(chosen).filter(([, value]) => value !== ""))
}
