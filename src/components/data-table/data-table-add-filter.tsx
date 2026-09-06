"use client";

import type { Column, Table } from "@tanstack/react-table";
import { ChevronDown, Plus, Trash2 } from "lucide-react";
import * as React from "react";

import { DataTableFilterItem } from "@/components/data-table/data-table-filter-controls";
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
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import { Separator } from "@/components/ui/separator";
import { getDefaultFilterOperator } from "@/lib/data-table";
import { useDataTableFilters } from "@/hooks/use-data-table-filters";
import { cn } from "@/lib/utils";

interface DataTableAdvancedFilterPanelProps<TData>
  extends React.ComponentProps<"div"> {
  table: Table<TData>;
  debounceMs?: number;
  throttleMs?: number;
  shallow?: boolean;
}

export function DataTableAdvancedFilterPanel<TData>({
  table,
  debounceMs,
  throttleMs,
  shallow,
  className,
  ...props
}: DataTableAdvancedFilterPanelProps<TData>) {
  const id = React.useId();
  const labelId = React.useId();
  const descriptionId = React.useId();

  const {
    columns,
    filters,
    joinOperator,
    onFilterAdd,
    onFilterUpdate,
    onFilterRemove,
    onFiltersReset,
    onJoinOperatorChange,
    setFiltersAndResetPage,
  } = useDataTableFilters(table, { debounceMs, throttleMs, shallow });

  const seededRef = React.useRef(false);

  // Seed one empty rule when opening advanced filters with nothing set — Notion
  // always shows a Where row ready to edit.
  React.useEffect(() => {
    if (seededRef.current || filters.length > 0 || columns.length === 0) {
      return;
    }

    const target = columns[0];
    if (!target) return;

    seededRef.current = true;
    void setFiltersAndResetPage([
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
  }, [columns, filters.length, setFiltersAndResetPage]);

  const handleDeleteAll = React.useCallback(() => {
    seededRef.current = true;
    onFiltersReset();
  }, [onFiltersReset]);

  const handleFilterDuplicate = React.useCallback(
    (filterId: string) => {
      const source = filters.find((item) => item.filterId === filterId);
      if (!source) return;

      const index = filters.findIndex((item) => item.filterId === filterId);
      const duplicate = {
        ...source,
        filterId: crypto.randomUUID().slice(0, 8),
        value: Array.isArray(source.value) ? [...source.value] : source.value,
      };
      const next = [...filters];
      next.splice(index + 1, 0, duplicate);
      void setFiltersAndResetPage(next);
    },
    [filters, setFiltersAndResetPage]
  );

  return (
    <div
      aria-labelledby={labelId}
      aria-describedby={descriptionId}
      className={cn("flex flex-col gap-2", className)}
      {...props}
    >
      <h4 id={labelId} className="sr-only">
        Filters
      </h4>
      <p id={descriptionId} className="sr-only">
        Add and modify filter rules to refine your rows.
      </p>
      {filters.length > 0 ? (
        <div
          role="list"
          className="flex max-h-[300px] flex-col gap-1.5 overflow-x-auto overflow-y-auto"
        >
          {filters.map((filter, index) => (
            <DataTableFilterItem<TData>
              key={filter.filterId}
              filter={filter}
              index={index}
              filterItemId={`${id}-filter-${filter.filterId}`}
              joinOperator={joinOperator}
              setJoinOperator={onJoinOperatorChange}
              columns={columns}
              onFilterUpdate={onFilterUpdate}
              onFilterRemove={onFilterRemove}
              onFilterDuplicate={handleFilterDuplicate}
            />
          ))}
        </div>
      ) : null}
      <Button
        variant="ghost"
        size="sm"
        className="text-muted-foreground h-7 w-fit justify-start px-2 font-normal"
        disabled={columns.length === 0}
        onClick={() => onFilterAdd()}
      >
        <Plus className="size-3.5" />
        Add filter rule
        <ChevronDown className="size-3.5 opacity-60" />
      </Button>
      {filters.length > 0 ? (
        <>
          <Separator />
          <Button
            variant="ghost"
            size="sm"
            className="text-muted-foreground h-7 w-fit justify-start px-2 font-normal"
            onClick={handleDeleteAll}
          >
            <Trash2 className="size-3.5" />
            Delete filter
          </Button>
        </>
      ) : null}
    </div>
  );
}

interface DataTableAddFilterProps<TData> {
  table: Table<TData>;
  disabled?: boolean;
  shallow?: boolean;
  debounceMs?: number;
  throttleMs?: number;
  /** Page-specific entries in the "Filter by..." picker. */
  menuExtras?: React.ReactNode;
  columns?: Column<TData>[];
  filters?: ReturnType<typeof useDataTableFilters<TData>>["filters"];
  onFilterAdd?: (column: Column<TData>) => void;
  /** Fired when the user chooses "Add advanced filter" in the picker. */
  onAdvancedFilterStart?: () => void;
}

export function DataTableAddFilter<TData>({
  table,
  disabled,
  shallow,
  debounceMs,
  throttleMs,
  menuExtras,
  columns: columnsProp,
  filters: filtersProp,
  onFilterAdd: onFilterAddProp,
  onAdvancedFilterStart,
}: DataTableAddFilterProps<TData>) {
  const [open, setOpen] = React.useState(false);
  const hook = useDataTableFilters(table, { debounceMs, throttleMs, shallow });

  const columns = columnsProp ?? hook.columns;
  const filters = filtersProp ?? hook.filters;
  const onFilterAdd = onFilterAddProp ?? hook.addColumnFilter;

  const availableColumns = React.useMemo(
    () =>
      columns.filter(
        (column) => !filters.some((filter) => filter.id === column.id)
      ),
    [columns, filters]
  );

  const handleColumnSelect = (column: Column<TData>) => {
    onFilterAdd(column);
    setOpen(false);
  };

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <Button
          variant="ghost"
          size="sm"
          disabled={disabled}
          className="text-muted-foreground h-7 gap-1 px-2 font-normal hover:text-foreground"
        >
          <Plus className="size-3.5" />
          Filter
        </Button>
      </PopoverTrigger>
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
              {menuExtras}
            </CommandGroup>
            <CommandSeparator />
            <CommandGroup>
              <CommandItem
                value="Add advanced filter"
                onSelect={() => {
                  onAdvancedFilterStart?.();
                  setOpen(false);
                }}
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
