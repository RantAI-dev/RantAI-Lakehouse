import type { Row } from "@tanstack/react-table";

import type { GroupSummary } from "@/services/contracts/pagination";
import {
  getGroupCount,
  getGroupLabel,
  getRowGroupKey,
  indexRowsByGroupKey,
} from "@/lib/data-table-grouping";

export type VirtualTableRow<TData> =
  | {
      kind: "group-header";
      id: string;
      groupKey: string;
      label: string;
      count?: number;
    }
  | {
      kind: "data";
      id: string;
      row: Row<TData>;
    };

export const VIRTUAL_DATA_ROW_HEIGHT = 44;
export const VIRTUAL_GROUP_HEADER_HEIGHT = 44;

export function estimateVirtualRowHeight<TData>(
  item: VirtualTableRow<TData> | undefined
): number {
  return item?.kind === "group-header"
    ? VIRTUAL_GROUP_HEADER_HEIGHT
    : VIRTUAL_DATA_ROW_HEIGHT;
}

interface BuildVirtualTableRowsOptions<TData> {
  tableRows: Row<TData>[];
  groupBy: string | null;
  groupSummaries?: GroupSummary[] | null;
  collapsedGroups: Set<string>;
}

/** Flat row model for window virtualization — group headers + data rows. */
export function buildVirtualTableRows<TData>({
  tableRows,
  groupBy,
  groupSummaries,
  collapsedGroups,
}: BuildVirtualTableRowsOptions<TData>): VirtualTableRow<TData>[] {
  const items: VirtualTableRow<TData>[] = [];

  const pushGroupHeader = (
    groupKey: string,
    label: string,
    count?: number
  ) => {
    items.push({
      kind: "group-header",
      id: `group:${groupKey}:${items.length}`,
      groupKey,
      label,
      count,
    });
  };

  if (groupBy && groupSummaries?.length) {
    const rowsByGroup = indexRowsByGroupKey(tableRows, groupBy);
    const renderedKeys = new Set<string>();

    for (const summary of groupSummaries) {
      const rows = rowsByGroup.get(summary.id) ?? [];
      // Infinite scroll loads buckets gradually — skip headers with no rows
      // yet so empty group sections do not appear ahead of loaded data.
      if (rows.length === 0) continue;

      renderedKeys.add(summary.id);
      pushGroupHeader(summary.id, summary.label, summary.count);
      if (!collapsedGroups.has(summary.id)) {
        for (const row of rows) {
          items.push({ kind: "data", id: String(row.id), row });
        }
      }
    }

    for (const [groupKey, rows] of rowsByGroup) {
      if (renderedKeys.has(groupKey) || rows.length === 0) continue;

      pushGroupHeader(
        groupKey,
        getGroupLabel(groupKey, groupSummaries),
        getGroupCount(groupKey, groupSummaries)
      );
      if (!collapsedGroups.has(groupKey)) {
        for (const row of rows) {
          items.push({ kind: "data", id: String(row.id), row });
        }
      }
    }
  } else if (groupBy) {
    let lastGroupKey: string | null = null;

    for (const row of tableRows) {
      const groupKey = getRowGroupKey(row, groupBy);

      if (groupKey !== lastGroupKey) {
        lastGroupKey = groupKey;
        pushGroupHeader(
          groupKey,
          getGroupLabel(groupKey, groupSummaries),
          getGroupCount(groupKey, groupSummaries)
        );
      }

      if (!collapsedGroups.has(groupKey)) {
        items.push({ kind: "data", id: String(row.id), row });
      }
    }
  } else {
    for (const row of tableRows) {
      items.push({ kind: "data", id: String(row.id), row });
    }
  }

  return items;
}
