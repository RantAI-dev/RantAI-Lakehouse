/**
 * Search for the chart-type sidebar in the chart builder.
 *
 * A type matches when every word of the query appears in its label, its
 * description, its group name or its kind id, so "flow" finds Sankey through
 * its group and "dist" finds Box plot through its label. Groups with no
 * matching type are dropped so the sidebar never shows an empty heading.
 */
export type KindGroup<K extends string> = { group: string; items: { value: K; label: string }[] };

export function filterKindGroups<K extends string>(
  groups: KindGroup<K>[],
  descriptions: Partial<Record<K, string>>,
  query: string,
): KindGroup<K>[] {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  if (!words.length) return groups;
  return groups
    .map((g) => ({
      group: g.group,
      items: g.items.filter((i) => {
        const hay = `${i.label} ${descriptions[i.value] ?? ""} ${g.group} ${i.value}`.toLowerCase();
        return words.every((w) => hay.includes(w));
      }),
    }))
    .filter((g) => g.items.length > 0);
}
