"use client";

import type { Column, Table } from "@tanstack/react-table";
import { ChevronDown, ListFilter, X } from "lucide-react";
import * as React from "react";

import { DataTableAdvancedFilterPanel } from "@/components/data-table/data-table-add-filter";
import {
  DataTableColumnFilterEditor,
  formatFilterSummary,
  isFilterActive,
} from "@/components/data-table/data-table-filter-controls";
import { Button } from "@/components/ui/button";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import { cn } from "@/lib/utils";
import type { ExtendedColumnFilter } from "@/types/data-table";

interface DataTableColumnFilterChipProps<TData> {
  column: Column<TData>;
  filter: ExtendedColumnFilter<TData>;
  onFilterUpdate: (
    filterId: string,
    updates: Partial<Omit<ExtendedColumnFilter<TData>, "filterId">>
  ) => void;
  onRemove: () => void;
  disabled?: boolean;
  defaultOpen?: boolean;
}

export function DataTableColumnFilterChip<TData>({
  column,
  filter,
  onFilterUpdate,
  onRemove,
  disabled,
  defaultOpen = false,
}: DataTableColumnFilterChipProps<TData>) {
  const [open, setOpen] = React.useState(defaultOpen);
  const label = column.columnDef.meta?.label ?? column.id;
  const Icon = column.columnDef.meta?.icon;
  const active = isFilterActive(filter);
  const summary = formatFilterSummary(filter);
  const display =
    active && summary ? `${label}: ${summary}` : label;

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <div
        className={cn(
          "inline-flex h-7 items-center gap-0.5 rounded-md border border-transparent",
          active
            ? "border-border bg-background text-foreground"
            : "bg-muted/40 text-muted-foreground"
        )}
      >
        <PopoverTrigger asChild>
          <Button
            variant="ghost"
            size="sm"
            disabled={disabled}
            className={cn(
              "h-7 max-w-48 gap-1 rounded-md border-0 px-2 font-normal shadow-none",
              "bg-transparent hover:bg-transparent hover:text-inherit"
            )}
          >
            {Icon ? <Icon className="size-3.5 shrink-0" /> : null}
            <span className="truncate">{display}</span>
            <ChevronDown className="size-3.5 shrink-0 opacity-60" />
          </Button>
        </PopoverTrigger>
        <Button
          type="button"
          variant="ghost"
          size="icon-sm"
          disabled={disabled}
          aria-label={`Remove ${label} filter`}
          className="text-muted-foreground hover:text-foreground size-7 shrink-0 rounded-md"
          onClick={onRemove}
        >
          <X className="size-3.5" />
        </Button>
      </div>
      <PopoverContent
        align="start"
        className="w-auto p-0"
        onOpenAutoFocus={(event) => {
          const root = event.currentTarget as HTMLElement | null;
          const valueControl =
            root?.querySelector<HTMLElement>("[data-filter-value]");
          if (!valueControl) return;
          event.preventDefault();
          requestAnimationFrame(() => valueControl.focus());
        }}
      >
        <DataTableColumnFilterEditor
          filter={filter}
          column={column}
          onFilterUpdate={onFilterUpdate}
          onClear={() => {
            onRemove();
            setOpen(false);
          }}
        />
      </PopoverContent>
    </Popover>
  );
}

/** Notion-style summary chip for multi-rule (advanced) filters. */
function DataTableAdvancedFilterRulesChip<TData>({
  table,
  ruleCount,
  disabled,
  shallow,
  debounceMs,
  throttleMs,
  defaultOpen = false,
}: {
  table: Table<TData>;
  ruleCount: number;
  disabled?: boolean;
  shallow?: boolean;
  debounceMs?: number;
  throttleMs?: number;
  defaultOpen?: boolean;
}) {
  const [open, setOpen] = React.useState(defaultOpen);
  const label = ruleCount === 1 ? "1 rule" : `${ruleCount} rules`;

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <button
          type="button"
          disabled={disabled}
          aria-label={`Manage ${label}`}
          className={cn(
            "bg-primary/10 text-primary inline-flex h-7 items-center gap-1 rounded-md px-2 text-sm",
            "hover:bg-primary/15 disabled:pointer-events-none disabled:opacity-50"
          )}
        >
          <ListFilter className="size-3.5 shrink-0" />
          <span>{label}</span>
          <ChevronDown className="size-3.5 shrink-0 opacity-60" />
        </button>
      </PopoverTrigger>
      <PopoverContent
        align="start"
        className="w-[min(100vw-2rem,36rem)] p-3"
      >
        <DataTableAdvancedFilterPanel
          table={table}
          shallow={shallow}
          debounceMs={debounceMs}
          throttleMs={throttleMs}
        />
      </PopoverContent>
    </Popover>
  );
}

interface DataTableColumnFilterChipsProps<TData> {
  table: Table<TData>;
  columns: Column<TData>[];
  filters: ExtendedColumnFilter<TData>[];
  onFilterUpdate: (
    filterId: string,
    updates: Partial<Omit<ExtendedColumnFilter<TData>, "filterId">>
  ) => void;
  onFilterRemove: (filterId: string) => void;
  disabled?: boolean;
  shallow?: boolean;
  debounceMs?: number;
  throttleMs?: number;
  /** filterId to open editor on first render (e.g. just added from picker). */
  openFilterId?: string | null;
  /**
   * Notion-style "N rules" summary chip — only when entered via
   * "Add advanced filter", not for ordinary multi-column chips.
   */
  forceAdvanced?: boolean;
}

export function DataTableColumnFilterChips<TData>({
  table,
  columns,
  filters,
  onFilterUpdate,
  onFilterRemove,
  disabled,
  shallow,
  debounceMs,
  throttleMs,
  openFilterId,
  forceAdvanced = false,
}: DataTableColumnFilterChipsProps<TData>) {
  if (forceAdvanced) {
    return (
      <DataTableAdvancedFilterRulesChip
        table={table}
        ruleCount={filters.length}
        disabled={disabled}
        shallow={shallow}
        debounceMs={debounceMs}
        throttleMs={throttleMs}
        defaultOpen={Boolean(openFilterId)}
      />
    );
  }

  return (
    <>
      {filters.map((filter) => {
        const column = columns.find((col) => col.id === filter.id);
        if (!column) return null;

        return (
          <DataTableColumnFilterChip
            key={filter.filterId}
            column={column}
            filter={filter}
            onFilterUpdate={onFilterUpdate}
            onRemove={() => onFilterRemove(filter.filterId)}
            disabled={disabled}
            defaultOpen={openFilterId === filter.filterId}
          />
        );
      })}
    </>
  );
}
