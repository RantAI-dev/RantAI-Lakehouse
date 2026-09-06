import type { Row } from "@tanstack/react-table";

import type { GroupSummary } from "@/services/contracts/pagination";

export const EMPTY_GROUP_KEY = "__empty__";

/** Must stay in sync with `_serialize_group_key` in `api/core/pagination.py`. */
export function serializeGroupKey(value: unknown): string {
  if (value === null || value === undefined || value === "") {
    return EMPTY_GROUP_KEY;
  }
  if (value instanceof Date) {
    return value.toISOString();
  }
  return String(value);
}

export function getRowGroupKey<TData>(row: Row<TData>, groupBy: string): string {
  const original = row.original as { groupKey?: string | null };
  if (original.groupKey != null && original.groupKey !== "") {
    return serializeGroupKey(original.groupKey);
  }
  return serializeGroupKey(row.getValue(groupBy));
}

/** First role name alphabetically — must match `_primary_role_name()` on the API. */
export function getPrimaryRoleName(
  roles: { name: string }[] | undefined | null
): string | null {
  if (!roles?.length) return null;
  return [...roles].map((role) => role.name).sort()[0] ?? null;
}

export function indexRowsByGroupKey<TData>(
  rows: Row<TData>[],
  groupBy: string
): Map<string, Row<TData>[]> {
  const grouped = new Map<string, Row<TData>[]>();
  for (const row of rows) {
    const groupKey = getRowGroupKey(row, groupBy);
    const bucket = grouped.get(groupKey);
    if (bucket) bucket.push(row);
    else grouped.set(groupKey, [row]);
  }
  return grouped;
}

export function getGroupLabel(
  groupKey: string,
  summaries?: GroupSummary[] | null
): string {
  const summary = summaries?.find((group) => group.id === groupKey);
  if (summary) return summary.label;
  if (groupKey === EMPTY_GROUP_KEY) return "Empty";
  return groupKey;
}

export function getGroupCount(
  groupKey: string,
  summaries?: GroupSummary[] | null
): number | undefined {
  return summaries?.find((group) => group.id === groupKey)?.count;
}
