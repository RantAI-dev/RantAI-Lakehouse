"use client";

import type { Column, Table } from "@tanstack/react-table";
import { ListFilter, Plus } from "lucide-react";
import { useSearchParams } from "next/navigation";
import * as React from "react";

import { DEFAULT_QUERY_KEYS } from "@/components/data-table/data-table-query-keys";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Command,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
  CommandSeparator,
} from "@/components/ui/command";
import {
  Popover,
  PopoverAnchor,
  PopoverContent,
} from "@/components/ui/popover";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { useDataTableFilters } from "@/hooks/use-data-table-filters";
import { cn } from "@/lib/utils";

interface DataTablePropertyBarToggleProps<TData> {
  table: Table<TData>;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  disabled?: boolean;
  shallow?: boolean;
  debounceMs?: number;
  throttleMs?: number;
  /** Page-specific entries in the "Filter by..." picker. */
  menuExtras?: React.ReactNode;
  /** Called after a column filter is created from the empty-state picker. */
  onFilterCreated?: (filterId: string) => void;
  /** Called when the user chooses "Add advanced filter". */
  onAdvancedFilterStart?: () => void;
}

export function DataTablePropertyBarToggle<TData>({
  table,
  open,
  onOpenChange,
  disabled,
  shallow,
  debounceMs,
  throttleMs,
  menuExtras,
  onFilterCreated,
  onAdvancedFilterStart,
}: DataTablePropertyBarToggleProps<TData>) {
  const [pickerOpen, setPickerOpen] = React.useState(false);
  const openedBarForPickerRef = React.useRef(false);

  const { columns, filters, addColumnFilter } = useDataTableFilters(table, {
    shallow,
    debounceMs,
    throttleMs,
  });

  const hasActiveFilters = filters.length > 0;

  const availableColumns = React.useMemo(
    () =>
      columns.filter(
        (column) => !filters.some((filter) => filter.id === column.id)
      ),
    [columns, filters]
  );

  const handleColumnSelect = React.useCallback(
    (column: Column<TData>) => {
      openedBarForPickerRef.current = false;
      const next = addColumnFilter(column);
      setPickerOpen(false);
      onOpenChange(true);
      onFilterCreated?.(next.filterId);
    },
    [addColumnFilter, onFilterCreated, onOpenChange]
  );

  const handleAdvancedStart = React.useCallback(() => {
    openedBarForPickerRef.current = false;
    setPickerOpen(false);
    onOpenChange(true);
    onAdvancedFilterStart?.();
  }, [onAdvancedFilterStart, onOpenChange]);

  const handlePickerOpenChange = React.useCallback(
    (next: boolean) => {
      setPickerOpen(next);
      if (!next && openedBarForPickerRef.current && filters.length === 0) {
        openedBarForPickerRef.current = false;
        onOpenChange(false);
      }
    },
    [filters.length, onOpenChange]
  );

  const handleButtonClick = React.useCallback(() => {
    if (!hasActiveFilters) {
      if (!open) {
        openedBarForPickerRef.current = true;
        onOpenChange(true);
      } else {
        openedBarForPickerRef.current = false;
      }
      setPickerOpen(true);
      return;
    }
    onOpenChange(!open);
  }, [hasActiveFilters, onOpenChange, open]);

  const button = (
    <Button
      type="button"
      variant="outline"
      size={hasActiveFilters ? "default" : "icon"}
      disabled={disabled}
      aria-expanded={open || pickerOpen}
      aria-pressed={open}
      aria-label="Filter"
      className={cn(
        "font-normal",
        hasActiveFilters && "px-2",
        (open || hasActiveFilters || pickerOpen) && "border-border bg-muted/50"
      )}
      onClick={handleButtonClick}
    >
      <ListFilter className="text-muted-foreground" />
      {hasActiveFilters ? (
        <Badge variant="secondary" className="rounded-sm px-1 font-normal">
          {filters.length}
        </Badge>
      ) : null}
    </Button>
  );

  if (hasActiveFilters) {
    return (
      <Tooltip>
        <TooltipTrigger render={button} />
        <TooltipContent>Filter</TooltipContent>
      </Tooltip>
    );
  }

  return (
    <Popover open={pickerOpen} onOpenChange={handlePickerOpenChange}>
      <Tooltip>
        <TooltipTrigger render={<PopoverAnchor asChild>{button}</PopoverAnchor>} />
        <TooltipContent>Filter</TooltipContent>
      </Tooltip>
      <PopoverContent align="start" className="w-52 p-0">
        <Command>
          <CommandInput placeholder="Filter by..." />
          <CommandList>
            <CommandEmpty>No properties found.</CommandEmpty>
            <CommandGroup>
              {availableColumns.map((column) => {
                const Icon = column.columnDef.meta?.icon;
                const label = column.columnDef.meta?.label ?? column.id;

                return (
                  <CommandItem
                    key={column.id}
                    value={label}
                    onSelect={() => handleColumnSelect(column)}
                  >
                    {Icon ? <Icon className="size-3.5 shrink-0" /> : null}
                    {label}
                  </CommandItem>
                );
              })}
              {menuExtras ? (
                <div
                  onPointerUp={() => {
                    openedBarForPickerRef.current = false;
                    setPickerOpen(false);
                    onOpenChange(true);
                  }}
                >
                  {menuExtras}
                </div>
              ) : null}
            </CommandGroup>
            <CommandSeparator />
            <CommandGroup>
              <CommandItem
                value="Add advanced filter"
                onSelect={handleAdvancedStart}
                className="text-muted-foreground"
              >
                <Plus className="size-3.5" />
                Add advanced filter
              </CommandItem>
            </CommandGroup>
          </CommandList>
        </Command>
      </PopoverContent>
    </Popover>
  );
}

export function usePropertyBarOpenState<TData>(
  table: Table<TData>,
  /** Extra URL keys that should open the bar on load (e.g. page-specific filters). */
  openWhenKeys: string[] = []
) {
  const searchParams = useSearchParams();
  const keys = table.options.meta?.queryKeys ?? DEFAULT_QUERY_KEYS;

  const ruleSignature = React.useMemo(
    () =>
      [
        searchParams.get(keys.sort),
        searchParams.get(keys.filters),
        ...openWhenKeys.map((key) => searchParams.get(key)),
      ].join("\0"),
    [keys.filters, keys.sort, openWhenKeys, searchParams]
  );

  const hasActiveRules = ruleSignature.replace(/\0/g, "").length > 0;

  const [manualOpen, setManualOpen] = React.useState(false);
  const [dismissedFor, setDismissedFor] = React.useState<string | null>(null);

  // Opening the bar while filters/sort are active only flips `dismissedFor`, so
  // `manualOpen` can stay true from an earlier empty-bar open. When rules clear,
  // fall back to `manualOpen` — reset it so the bar collapses instead of sticking.
  React.useEffect(() => {
    if (!hasActiveRules) {
      setManualOpen(false);
    }
  }, [hasActiveRules]);

  const open = hasActiveRules
    ? dismissedFor !== ruleSignature
    : manualOpen;

  const setOpen = React.useCallback(
    (next: boolean) => {
      if (hasActiveRules) {
        setDismissedFor(next ? null : ruleSignature);
        return;
      }
      setManualOpen(next);
    },
    [hasActiveRules, ruleSignature]
  );

  return [open, setOpen] as const;
}
