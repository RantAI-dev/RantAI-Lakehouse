"use client";

import type { ColumnSort, Table } from "@tanstack/react-table";
import * as React from "react";

export function useDataTableSorting<TData>(table: Table<TData>) {
  const sorting = table.getState().sorting;
  const onSortingChange = table.setSorting;

  const { columnLabels, availableColumns } = React.useMemo(() => {
    const labels = new Map<string, string>();
    const sortingIds = new Set(sorting.map((sort) => sort.id));
    const columns: { id: string; label: string }[] = [];

    for (const column of table.getAllColumns()) {
      if (!column.getCanSort()) continue;

      const label = column.columnDef.meta?.label ?? column.id;
      labels.set(column.id, label);

      if (!sortingIds.has(column.id)) {
        columns.push({ id: column.id, label });
      }
    }

    return { columnLabels: labels, availableColumns: columns };
  }, [sorting, table]);

  const onSortAdd = React.useCallback(
    (columnId?: string) => {
      const id = columnId ?? availableColumns[0]?.id;
      if (!id) return;

      onSortingChange((prevSorting) => [...prevSorting, { id, desc: false }]);
    },
    [availableColumns, onSortingChange]
  );

  const onSortUpdate = React.useCallback(
    (sortId: string, updates: Partial<ColumnSort>) => {
      onSortingChange((prevSorting) =>
        prevSorting.map((sort) =>
          sort.id === sortId ? { ...sort, ...updates } : sort
        )
      );
    },
    [onSortingChange]
  );

  const onSortRemove = React.useCallback(
    (sortId: string) => {
      onSortingChange((prevSorting) =>
        prevSorting.filter((item) => item.id !== sortId)
      );
    },
    [onSortingChange]
  );

  const onSortingReset = React.useCallback(
    () => onSortingChange(table.initialState.sorting),
    [onSortingChange, table.initialState.sorting]
  );

  const toggleColumnSort = React.useCallback(
    (columnId: string) => {
      const existing = sorting.find((sort) => sort.id === columnId);
      if (!existing) {
        onSortingChange([{ id: columnId, desc: false }]);
        return;
      }
      if (!existing.desc) {
        onSortUpdate(columnId, { desc: true });
        return;
      }
      onSortRemove(columnId);
    },
    [onSortRemove, onSortUpdate, onSortingChange, sorting]
  );

  return {
    sorting,
    columnLabels,
    availableColumns,
    onSortAdd,
    onSortUpdate,
    onSortRemove,
    onSortingReset,
    toggleColumnSort,
  };
}
