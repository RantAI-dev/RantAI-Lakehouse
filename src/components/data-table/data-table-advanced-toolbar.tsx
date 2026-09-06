"use client";

import type { Table } from "@tanstack/react-table";
import * as React from "react";

import { DataTablePropertyBar } from "@/components/data-table/data-table-property-bar";
import {
  DataTablePropertyBarToggle,
  usePropertyBarOpenState,
} from "@/components/data-table/data-table-property-bar-toggle";
import { DataTableQueryKeysProvider } from "@/components/data-table/data-table-query-keys";
import { DataTableResetFilters } from "@/components/data-table/data-table-reset-filters";
import { DataTableSettingsMenu } from "@/components/data-table/data-table-settings-menu";
import {
  DataTableSortButton,
  DataTableSortProvider,
} from "@/components/data-table/data-table-sort-chip";
import { cn } from "@/lib/utils";

interface DataTableAdvancedToolbarProps<
  TData,
> extends React.ComponentProps<"div"> {
  table: Table<TData>;
  /** Refetches the page's own query, surfaced as "Refresh data" under Settings. */
  onRefresh?: () => void;
  isRefreshing?: boolean;
  /** Off in the list view, which has no columns to hide or reorder. */
  columnControls?: boolean;
  /** Hide the Notion-style sort/filter property bar (row 2) entirely. */
  hidePropertyBar?: boolean;
  /** Passed through to the property bar filter URL state. */
  shallow?: boolean;
  debounceMs?: number;
  throttleMs?: number;
  /** Extra chips for the property bar (e.g. page-specific filters). */
  propertyBar?: React.ReactNode;
  /** Page-specific entries in the "+ Filter" picker. */
  filterMenuExtras?: React.ReactNode;
  /** Extra URL keys that auto-open the property bar on load. */
  propertyBarOpenKeys?: string[];
  /** Controls rendered on the right of row 1, after Settings. */
  trailing?: React.ReactNode;
}

export function DataTableAdvancedToolbar<TData>({
  table,
  onRefresh,
  isRefreshing,
  columnControls,
  hidePropertyBar,
  shallow,
  debounceMs,
  throttleMs,
  propertyBar,
  filterMenuExtras,
  propertyBarOpenKeys,
  trailing,
  children,
  className,
  ...props
}: DataTableAdvancedToolbarProps<TData>) {
  const [propertyBarOpen, setPropertyBarOpen] = usePropertyBarOpenState(
    table,
    propertyBarOpenKeys
  );
  const [openFilterId, setOpenFilterId] = React.useState<string | null>(null);
  const [advancedFilterMode, setAdvancedFilterMode] = React.useState(false);

  return (
    // The search box writes the table's params but never sees the table, so
    // its names reach it from here.
    <DataTableQueryKeysProvider
      keys={table.options.meta?.queryKeys}
      paginationMode={table.options.meta?.paginationMode}
      resetExtraKeys={table.options.meta?.memoryKeys}
    >
      <DataTableSortProvider table={table}>
        <div
          className={cn("flex w-full min-w-0 flex-col gap-1.5", className)}
          {...props}
        >
          <div
            role="toolbar"
            aria-orientation="horizontal"
            className="flex w-full items-start justify-between gap-2"
          >
            <div className="flex flex-1 flex-wrap items-center gap-2">
              {children}
              <DataTableSortButton
                propertyBarOpen={propertyBarOpen}
                onPropertyBarOpenChange={setPropertyBarOpen}
              />
              {!hidePropertyBar ? (
                <DataTablePropertyBarToggle
                  table={table}
                  open={propertyBarOpen}
                  onOpenChange={setPropertyBarOpen}
                  shallow={shallow}
                  debounceMs={debounceMs}
                  throttleMs={throttleMs}
                  menuExtras={filterMenuExtras}
                  onFilterCreated={(filterId) => {
                    setAdvancedFilterMode(false);
                    setOpenFilterId(filterId);
                  }}
                  onAdvancedFilterStart={() => {
                    setAdvancedFilterMode(true);
                    setOpenFilterId("advanced");
                  }}
                />
              ) : (
                <DataTableResetFilters />
              )}
            </div>
            <div className="flex items-center gap-2">
              <DataTableSettingsMenu
                table={table}
                onRefresh={onRefresh}
                isRefreshing={isRefreshing}
                columnControls={columnControls}
              />
              {trailing}
            </div>
          </div>
          {!hidePropertyBar && propertyBarOpen ? (
            <DataTablePropertyBar
              table={table}
              shallow={shallow}
              debounceMs={debounceMs}
              throttleMs={throttleMs}
              className="w-full"
              filterMenuExtras={filterMenuExtras}
              openFilterId={openFilterId}
              onOpenFilterIdChange={setOpenFilterId}
              advancedFilterMode={advancedFilterMode}
              onAdvancedFilterModeChange={setAdvancedFilterMode}
            >
              {propertyBar}
            </DataTablePropertyBar>
          ) : null}
        </div>
      </DataTableSortProvider>
    </DataTableQueryKeysProvider>
  );
}
