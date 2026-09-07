"use client";

import type { Table } from "@tanstack/react-table";
import * as React from "react";

import { DataTableAddFilter } from "@/components/data-table/data-table-add-filter";
import { DataTableColumnFilterChips } from "@/components/data-table/data-table-column-filter-chip";
import { DataTableResetFilters } from "@/components/data-table/data-table-reset-filters";
import { DataTableSortChips } from "@/components/data-table/data-table-sort-chip";
import { useDataTableFilters } from "@/hooks/use-data-table-filters";
import { cn } from "@/lib/utils";

interface DataTablePropertyBarProps<TData> {
  table: Table<TData>;
  disabled?: boolean;
  shallow?: boolean;
  debounceMs?: number;
  throttleMs?: number;
  className?: string;
  /** Active filter chips rendered before "+ Filter". */
  children?: React.ReactNode;
  /** Page-specific entries in the "+ Filter" picker. */
  filterMenuExtras?: React.ReactNode;
  /** filterId whose editor should open (e.g. just added from the toolbar). */
  openFilterId?: string | null;
  onOpenFilterIdChange?: (filterId: string | null) => void;
  /** Use the "N rules" advanced summary chip (e.g. after Add advanced filter). */
  advancedFilterMode?: boolean;
  onAdvancedFilterModeChange?: (advanced: boolean) => void;
}

export function DataTablePropertyBar<TData>({
  table,
  disabled,
  shallow,
  debounceMs,
  throttleMs,
  className,
  children,
  filterMenuExtras,
  openFilterId: openFilterIdProp,
  onOpenFilterIdChange,
  advancedFilterMode: advancedFilterModeProp,
  onAdvancedFilterModeChange,
}: DataTablePropertyBarProps<TData>) {
  const [uncontrolledOpenFilterId, setUncontrolledOpenFilterId] =
    React.useState<string | null>(null);
  const [uncontrolledAdvanced, setUncontrolledAdvanced] = React.useState(false);

  const openFilterId = openFilterIdProp ?? uncontrolledOpenFilterId;
  const setOpenFilterId = onOpenFilterIdChange ?? setUncontrolledOpenFilterId;
  const advancedFilterMode = advancedFilterModeProp ?? uncontrolledAdvanced;
  const setAdvancedFilterMode =
    onAdvancedFilterModeChange ?? setUncontrolledAdvanced;

  const {
    columns,
    filters,
    addColumnFilter,
    onFilterUpdate,
    onFilterRemove,
  } = useDataTableFilters(table, { shallow, debounceMs, throttleMs });

  const prevFilterCountRef = React.useRef(filters.length);
  React.useEffect(() => {
    if (prevFilterCountRef.current > 0 && filters.length === 0) {
      setAdvancedFilterMode(false);
    }
    prevFilterCountRef.current = filters.length;
  }, [filters.length, setAdvancedFilterMode]);

  // Seed only when *entering* advanced mode with an empty list — not when the
  // user clears rules while advancedFilterMode is still briefly true.
  const wasAdvancedRef = React.useRef(false);
  React.useEffect(() => {
    const entered = advancedFilterMode && !wasAdvancedRef.current;
    wasAdvancedRef.current = advancedFilterMode;
    if (!entered || filters.length > 0 || !columns[0]) return;
    addColumnFilter(columns[0]);
  }, [addColumnFilter, advancedFilterMode, columns, filters.length]);

  const handleFilterAdd = React.useCallback(
    (column: Parameters<typeof addColumnFilter>[0]) => {
      const next = addColumnFilter(column);
      // Stay in the "N rules" chip when already on the advanced path; only
      // column-picker adds outside advanced become per-property chips.
      if (advancedFilterMode) {
        setOpenFilterId("advanced");
        return;
      }
      setOpenFilterId(next.filterId);
    },
    [addColumnFilter, advancedFilterMode, setOpenFilterId]
  );

  const handleAdvancedFilterStart = React.useCallback(() => {
    setAdvancedFilterMode(true);
    setOpenFilterId("advanced");
  }, [setAdvancedFilterMode, setOpenFilterId]);

  return (
    <div
      className={cn(
        "flex w-full flex-wrap items-center gap-1.5 overflow-x-auto",
        className
      )}
    >
      <DataTableSortChips disabled={disabled} />
      {children}
      {filters.length > 0 ? (
        <DataTableColumnFilterChips
          table={table}
          columns={columns}
          filters={filters}
          onFilterUpdate={onFilterUpdate}
          onFilterRemove={onFilterRemove}
          disabled={disabled}
          shallow={shallow}
          debounceMs={debounceMs}
          throttleMs={throttleMs}
          openFilterId={openFilterId}
          forceAdvanced={advancedFilterMode}
        />
      ) : null}
      <DataTableAddFilter
        table={table}
        disabled={disabled}
        shallow={shallow}
        debounceMs={debounceMs}
        throttleMs={throttleMs}
        menuExtras={filterMenuExtras}
        columns={columns}
        filters={filters}
        onFilterAdd={handleFilterAdd}
        onAdvancedFilterStart={handleAdvancedFilterStart}
      />
      <DataTableResetFilters />
    </div>
  );
}
