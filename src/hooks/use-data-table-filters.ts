"use client";

import type { Column, Table } from "@tanstack/react-table";
import { parseAsInteger, parseAsStringEnum, useQueryState } from "nuqs";
import * as React from "react";

import { useDebouncedCallback } from "@/hooks/use-debounced-callback";
import { getDefaultFilterOperator } from "@/lib/data-table";
import { getFiltersStateParser } from "@/lib/parsers";
import type { ExtendedColumnFilter, JoinOperator } from "@/types/data-table";

const DEFAULT_DEBOUNCE_MS = 300;
const DEFAULT_THROTTLE_MS = 50;

export function useDataTableFilters<TData>(
  table: Table<TData>,
  {
    debounceMs = DEFAULT_DEBOUNCE_MS,
    throttleMs = DEFAULT_THROTTLE_MS,
    shallow = true,
  }: {
    debounceMs?: number;
    throttleMs?: number;
    shallow?: boolean;
  } = {}
) {
  const columns = React.useMemo(() => {
    return table
      .getAllColumns()
      .filter((column) => column.columnDef.enableColumnFilter);
  }, [table]);

  const paginationMode = table.options.meta?.paginationMode ?? "pages";
  const isInfinite = paginationMode === "infinite";

  const [filters, setFilters] = useQueryState(
    table.options.meta?.queryKeys?.filters ?? "filters",
    getFiltersStateParser<TData>(columns.map((field) => field.id))
      .withDefault([])
      .withOptions({
        clearOnDefault: true,
        shallow,
        throttleMs,
      })
  );

  const [, setPageState] = useQueryState(
    table.options.meta?.queryKeys?.page ?? "page",
    parseAsInteger.withDefault(1).withOptions({
      clearOnDefault: true,
      shallow,
    })
  );

  const resetPage = React.useCallback(() => {
    if (!isInfinite) {
      void setPageState(1);
    }
  }, [isInfinite, setPageState]);

  const setFiltersAndResetPage = React.useCallback(
    (nextFilters: Parameters<typeof setFilters>[0]) => {
      resetPage();
      void setFilters(nextFilters);
    },
    [resetPage, setFilters]
  );

  const debouncedSetFilters = useDebouncedCallback(
    setFiltersAndResetPage,
    debounceMs
  );

  const [joinOperator, setJoinOperator] = useQueryState(
    table.options.meta?.queryKeys?.joinOperator ?? "joinOperator",
    parseAsStringEnum(["and", "or"]).withDefault("and").withOptions({
      clearOnDefault: true,
      shallow,
    })
  );

  const onFilterAdd = React.useCallback(
    (column?: Column<TData>) => {
      const target = column ?? columns[0];
      if (!target) return;

      debouncedSetFilters([
        ...filters,
        {
          id: target.id as Extract<keyof TData, string>,
          value: "",
          variant: target.columnDef.meta?.variant ?? "text",
          operator: getDefaultFilterOperator(
            target.columnDef.meta?.variant ?? "text"
          ),
          filterId: crypto.randomUUID().slice(0, 8),
        },
      ]);
    },
    [columns, filters, debouncedSetFilters]
  );

  const onFilterUpdate = React.useCallback(
    (
      filterId: string,
      updates: Partial<Omit<ExtendedColumnFilter<TData>, "filterId">>
    ) => {
      debouncedSetFilters((prevFilters) =>
        prevFilters.map((filter) =>
          filter.filterId === filterId
            ? ({ ...filter, ...updates } as ExtendedColumnFilter<TData>)
            : filter
        )
      );
    },
    [debouncedSetFilters]
  );

  const onFilterRemove = React.useCallback(
    (filterId: string) => {
      setFiltersAndResetPage(
        filters.filter((filter) => filter.filterId !== filterId)
      );
    },
    [filters, setFiltersAndResetPage]
  );

  const onFiltersReset = React.useCallback(() => {
    setFiltersAndResetPage(null);
    void setJoinOperator("and");
  }, [setFiltersAndResetPage, setJoinOperator]);

  const onJoinOperatorChange = React.useCallback(
    (value: JoinOperator) => {
      resetPage();
      void setJoinOperator(value);
    },
    [resetPage, setJoinOperator]
  );

  const getFilterForColumn = React.useCallback(
    (columnId: string) => filters.find((filter) => filter.id === columnId),
    [filters]
  );

  const addColumnFilter = React.useCallback(
    (column: Column<TData>) => {
      const existing = filters.find((filter) => filter.id === column.id);
      if (existing) return existing;

      const nextFilter: ExtendedColumnFilter<TData> = {
        id: column.id as Extract<keyof TData, string>,
        value: "",
        variant: column.columnDef.meta?.variant ?? "text",
        operator: getDefaultFilterOperator(
          column.columnDef.meta?.variant ?? "text"
        ),
        filterId: crypto.randomUUID().slice(0, 8),
      };

      setFiltersAndResetPage([...filters, nextFilter]);
      return nextFilter;
    },
    [filters, setFiltersAndResetPage]
  );

  const upsertColumnFilter = addColumnFilter;

  const clearColumnFilter = React.useCallback(
    (columnId: string) => {
      const remaining = filters.filter((filter) => filter.id !== columnId);
      setFiltersAndResetPage(remaining.length > 0 ? remaining : null);
    },
    [filters, setFiltersAndResetPage]
  );

  return {
    columns,
    filters,
    joinOperator,
    onFilterAdd,
    onFilterUpdate,
    onFilterRemove,
    onFiltersReset,
    onJoinOperatorChange,
    getFilterForColumn,
    upsertColumnFilter,
    addColumnFilter,
    clearColumnFilter,
    setFiltersAndResetPage,
  };
}
