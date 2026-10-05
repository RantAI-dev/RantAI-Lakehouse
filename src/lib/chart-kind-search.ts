/**
 * Search for the chart-type sidebar in the chart builder.
 *
 * A type matches when every word of the query appears in its label, its
 * description, its group name, its kind id or its extra keywords, so "flow"
 * finds Sankey through its group, "dist" finds Box plot through its label and
 * "peta" (Indonesian for map) finds the map types through their keywords.
 * Groups with no matching type are dropped so the sidebar never shows an
 * empty heading.
 */
export type KindGroup<K extends string> = { group: string; items: { value: K; label: string }[] };

export function filterKindGroups<K extends string>(
  groups: KindGroup<K>[],
  descriptions: Partial<Record<K, string>>,
  query: string,
  keywords: Partial<Record<K, string>> = {},
): KindGroup<K>[] {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  if (!words.length) return groups;
  return groups
    .map((g) => ({
      group: g.group,
      items: g.items.filter((i) => {
        const hay = `${i.label} ${descriptions[i.value] ?? ""} ${g.group} ${i.value} ${keywords[i.value] ?? ""}`.toLowerCase();
        return words.every((w) => hay.includes(w));
      }),
    }))
    .filter((g) => g.items.length > 0);
}
