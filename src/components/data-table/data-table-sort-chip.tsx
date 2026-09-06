"use client";

import type { ColumnSort, SortDirection, Table } from "@tanstack/react-table";
import {
  ChevronDown,
  ChevronsUpDown,
  GripVertical,
  Plus,
  Trash2,
  X,
} from "lucide-react";
import * as React from "react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Command,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
} from "@/components/ui/command";
import {
  Popover,
  PopoverAnchor,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@rantai/design-system/ui/select";
import { Separator } from "@/components/ui/separator";
import {
  Sortable,
  SortableContent,
  SortableItem,
  SortableItemHandle,
  SortableOverlay,
} from "@/components/ui/sortable";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@rantai/design-system/ui/tooltip";
import { useDataTableSorting } from "@/hooks/use-data-table-sorting";
import { cn } from "@/lib/utils";
import type { FilterVariant } from "@/types/data-table";

const TEXT_SORT_DIRECTIONS = [
  { value: "asc" as const, label: "Sort A → Z" },
  { value: "desc" as const, label: "Sort Z → A" },
];

const DATE_SORT_DIRECTIONS = [
  { value: "asc" as const, label: "Sort old → new" },
  { value: "desc" as const, label: "Sort new → old" },
];

const NUMBER_SORT_DIRECTIONS = [
  { value: "asc" as const, label: "Sort 1 → 9" },
  { value: "desc" as const, label: "Sort 9 → 1" },
];

function getSortDirections(variant: FilterVariant | undefined) {
  switch (variant) {
    case "date":
    case "dateRange":
      return DATE_SORT_DIRECTIONS;
    case "number":
    case "range":
      return NUMBER_SORT_DIRECTIONS;
    default:
      return TEXT_SORT_DIRECTIONS;
  }
}

function getColumnIcon<TData>(table: Table<TData>, columnId: string) {
  return table.getColumn(columnId)?.columnDef.meta?.icon;
}

function getColumnVariant<TData>(table: Table<TData>, columnId: string) {
  return table.getColumn(columnId)?.columnDef.meta?.variant;
}

type SortControlsContextValue<TData> = {
  table: Table<TData>;
  disabled?: boolean;
  open: boolean;
  setOpen: (open: boolean) => void;
  sorting: ColumnSort[];
  columnLabels: Map<string, string>;
  availableColumns: { id: string; label: string }[];
  onSortAdd: (columnId?: string) => void;
  onSortUpdate: (sortId: string, updates: Partial<ColumnSort>) => void;
  onSortRemove: (sortId: string) => void;
  onSortingReset: () => void;
};

const SortControlsContext =
  React.createContext<SortControlsContextValue<unknown> | null>(null);

function useSortControls<TData>() {
  const context = React.useContext(SortControlsContext);
  if (!context) {
    throw new Error(
      "DataTable sort controls must be used within DataTableSortProvider"
    );
  }
  return context as SortControlsContextValue<TData>;
}

interface DataTableSortProviderProps<TData> {
  table: Table<TData>;
  disabled?: boolean;
  children: React.ReactNode;
}

export function DataTableSortProvider<TData>({
  table,
  disabled,
  children,
}: DataTableSortProviderProps<TData>) {
  const [open, setOpen] = React.useState(false);
  const sortingState = useDataTableSorting(table);

  return (
    <SortControlsContext.Provider
      value={
        {
          table,
          disabled,
          open,
          setOpen,
          ...sortingState,
        } as SortControlsContextValue<unknown>
      }
    >
      {children}
    </SortControlsContext.Provider>
  );
}

function SortFieldPicker<TData>({
  table,
  columns,
  onSelect,
}: {
  table: Table<TData>;
  columns: { id: string; label: string }[];
  onSelect: (columnId: string) => void;
}) {
  return (
    <Command>
      <CommandInput placeholder="Sort by..." />
      <CommandList>
        <CommandEmpty>No sortable columns.</CommandEmpty>
        <CommandGroup>
          {columns.map((column) => {
            const Icon = getColumnIcon(table, column.id);
            return (
              <CommandItem
                key={column.id}
                value={column.label}
                onSelect={() => onSelect(column.id)}
              >
                {Icon ? <Icon className="size-4" /> : null}
                <span className="truncate">{column.label}</span>
              </CommandItem>
            );
          })}
        </CommandGroup>
      </CommandList>
    </Command>
  );
}

interface SortEditorItemProps<TData> {
  table: Table<TData>;
  sort: ColumnSort;
  sorting: ColumnSort[];
  sortItemId: string;
  columnLabels: Map<string, string>;
  onSortUpdate: (sortId: string, updates: Partial<ColumnSort>) => void;
  onSortRemove: (sortId: string) => void;
  disabled?: boolean;
}

function SortEditorItem<TData>({
  table,
  sort,
  sorting,
  sortItemId,
  columnLabels,
  onSortUpdate,
  onSortRemove,
  disabled,
}: SortEditorItemProps<TData>) {
  const fieldOptions = React.useMemo(() => {
    const taken = new Set(
      sorting.filter((item) => item.id !== sort.id).map((item) => item.id)
    );

    return table
      .getAllColumns()
      .filter((column) => column.getCanSort() && !taken.has(column.id))
      .map((column) => ({
        id: column.id,
        label: column.columnDef.meta?.label ?? column.id,
        icon: column.columnDef.meta?.icon,
      }));
  }, [sort.id, sorting, table]);

  const FieldIcon = getColumnIcon(table, sort.id);
  const sortDirections = getSortDirections(getColumnVariant(table, sort.id));
  const selectedDirectionLabel =
    sortDirections.find((order) => order.value === (sort.desc ? "desc" : "asc"))
      ?.label ?? sortDirections[0]?.label;

  return (
    <SortableItem value={sort.id} asChild>
      <div role="listitem" id={sortItemId} className="flex items-center gap-1">
        <SortableItemHandle asChild>
          <Button
            variant="ghost"
            size="icon-sm"
            disabled={disabled}
            className="text-muted-foreground size-7 shrink-0"
            aria-label="Reorder sort"
          >
            <GripVertical />
          </Button>
        </SortableItemHandle>
        <Select
          value={sort.id}
          onValueChange={(value) => onSortUpdate(sort.id, { id: value })}
          disabled={disabled}
        >
          <SelectTrigger size="sm" className="h-7 min-w-28 flex-1">
            <SelectValue>
              <span className="inline-flex items-center gap-1.5 truncate">
                {/* False positive below: `FieldIcon` is not created here. It
                    is a stable component *reference* the page declared in
                    its column meta (`getColumnIcon` only reads
                    `columnDef.meta.icon`), so rendering it cannot reset
                    state the way an inline component definition would. */}
                {FieldIcon ? (
                  // eslint-disable-next-line react-hooks/static-components
                  <FieldIcon className="size-3.5 shrink-0 opacity-70" />
                ) : null}
                <span className="truncate">
                  {columnLabels.get(sort.id) ?? sort.id}
                </span>
              </span>
            </SelectValue>
          </SelectTrigger>
          <SelectContent>
            <SelectGroup>
              {fieldOptions.map((column) => {
                const Icon = column.icon;
                return (
                  <SelectItem key={column.id} value={column.id}>
                    {Icon ? <Icon className="size-4" /> : null}
                    {column.label}
                  </SelectItem>
                );
              })}
            </SelectGroup>
          </SelectContent>
        </Select>
        <Select
          value={sort.desc ? "desc" : "asc"}
          onValueChange={(value: SortDirection) =>
            onSortUpdate(sort.id, { desc: value === "desc" })
          }
          disabled={disabled}
        >
          <SelectTrigger size="sm" className="h-7 w-[9.5rem] shrink-0">
            <SelectValue>{selectedDirectionLabel}</SelectValue>
          </SelectTrigger>
          <SelectContent>
            <SelectGroup>
              {sortDirections.map((order) => (
                <SelectItem key={order.value} value={order.value}>
                  {order.label}
                </SelectItem>
              ))}
            </SelectGroup>
          </SelectContent>
        </Select>
        <Button
          variant="ghost"
          size="icon-sm"
          disabled={disabled}
          className="text-muted-foreground size-7 shrink-0"
          aria-label={`Remove sort by ${columnLabels.get(sort.id) ?? sort.id}`}
          onClick={() => onSortRemove(sort.id)}
        >
          <X />
        </Button>
      </div>
    </SortableItem>
  );
}

/** Toolbar Sort control — matches Filter: opens the property bar; picker when empty. */
export function DataTableSortButton<TData>({
  propertyBarOpen,
  onPropertyBarOpenChange,
}: {
  propertyBarOpen: boolean;
  onPropertyBarOpenChange: (open: boolean) => void;
}) {
  const [pickerOpen, setPickerOpen] = React.useState(false);
  const openedBarForPickerRef = React.useRef(false);
  const {
    table,
    disabled,
    sorting,
    availableColumns,
    onSortAdd,
  } = useSortControls<TData>();

  const hasSorting = sorting.length > 0;
  const noSortableColumns = !hasSorting && availableColumns.length === 0;

  const handlePickColumn = React.useCallback(
    (columnId: string) => {
      openedBarForPickerRef.current = false;
      onSortAdd(columnId);
      setPickerOpen(false);
      onPropertyBarOpenChange(true);
    },
    [onPropertyBarOpenChange, onSortAdd]
  );

  const handlePickerOpenChange = React.useCallback(
    (next: boolean) => {
      setPickerOpen(next);
      if (
        !next &&
        openedBarForPickerRef.current &&
        sorting.length === 0
      ) {
        openedBarForPickerRef.current = false;
        onPropertyBarOpenChange(false);
      }
    },
    [onPropertyBarOpenChange, sorting.length]
  );

  const handleButtonClick = React.useCallback(() => {
    if (!hasSorting) {
      if (!propertyBarOpen) {
        openedBarForPickerRef.current = true;
        onPropertyBarOpenChange(true);
      } else {
        openedBarForPickerRef.current = false;
      }
      setPickerOpen(true);
      return;
    }
    onPropertyBarOpenChange(!propertyBarOpen);
  }, [hasSorting, onPropertyBarOpenChange, propertyBarOpen]);

  const button = (
    <Button
      type="button"
      variant="outline"
      size={hasSorting ? "default" : "icon"}
      disabled={disabled || (!hasSorting && noSortableColumns)}
      aria-expanded={propertyBarOpen || pickerOpen}
      aria-pressed={propertyBarOpen}
      aria-label="Sort"
      className={cn(
        "font-normal",
        hasSorting && "px-2",
        (propertyBarOpen || hasSorting || pickerOpen) &&
          "border-border bg-muted/50"
      )}
      onClick={handleButtonClick}
    >
      <ChevronsUpDown className="text-muted-foreground" />
      {hasSorting ? (
        <Badge variant="secondary" className="rounded-sm px-1 font-normal">
          {sorting.length}
        </Badge>
      ) : null}
    </Button>
  );

  if (hasSorting) {
    return (
      <Tooltip>
        <TooltipTrigger asChild>{button}</TooltipTrigger>
        <TooltipContent>Sort</TooltipContent>
      </Tooltip>
    );
  }

  return (
    <Popover open={pickerOpen} onOpenChange={handlePickerOpenChange}>
      <Tooltip>
        <TooltipTrigger asChild>
          <PopoverAnchor asChild>{button}</PopoverAnchor>
        </TooltipTrigger>
        <TooltipContent>Sort</TooltipContent>
      </Tooltip>
      <PopoverContent align="start" className="w-56 p-0">
        <SortFieldPicker
          table={table}
          columns={availableColumns}
          onSelect={handlePickColumn}
        />
      </PopoverContent>
    </Popover>
  );
}

/** Property-bar summary Sort control — opens the editor popover. */
export function DataTableSortChips<TData>({
  disabled,
}: {
  disabled?: boolean;
} = {}) {
  const id = React.useId();
  const [adding, setAdding] = React.useState(false);
  const {
    table,
    sorting,
    columnLabels,
    availableColumns,
    open,
    setOpen,
    onSortAdd,
    onSortUpdate,
    onSortRemove,
    onSortingReset,
    disabled: providerDisabled,
  } = useSortControls<TData>();

  const handlePopoverOpenChange = React.useCallback(
    (next: boolean) => {
      setOpen(next);
      if (!next) setAdding(false);
    },
    [setOpen]
  );

  const handlePickColumn = React.useCallback(
    (columnId: string) => {
      onSortAdd(columnId);
      setAdding(false);
    },
    [onSortAdd]
  );

  const handleDeleteAll = React.useCallback(() => {
    onSortingReset();
    setAdding(false);
    setOpen(false);
  }, [onSortingReset, setOpen]);

  if (sorting.length === 0) return null;

  const isDisabled = disabled ?? providerDisabled;
  const sortLabel = sorting.length === 1 ? "1 sort" : `${sorting.length} sorts`;

  return (
    <>
      <Popover open={open} onOpenChange={handlePopoverOpenChange}>
        <PopoverTrigger asChild>
          <button
            type="button"
            disabled={isDisabled}
            aria-label={`Manage ${sortLabel}`}
            className={cn(
              "bg-primary/10 text-primary inline-flex h-7 items-center gap-1 rounded-md px-2 text-sm",
              "hover:bg-primary/15 disabled:pointer-events-none disabled:opacity-50"
            )}
          >
            <ChevronsUpDown className="size-3.5 shrink-0" />
            <span>{sortLabel}</span>
            <ChevronDown className="size-3.5 shrink-0 opacity-60" />
          </button>
        </PopoverTrigger>
        <PopoverContent
          align="start"
          className="flex w-[26rem] flex-col gap-1 p-2"
        >
          <Sortable
            value={sorting}
            onValueChange={table.setSorting}
            getItemValue={(item) => item.id}
          >
            <SortableContent asChild>
              <div role="list" className="flex flex-col gap-1">
                {sorting.map((sort) => (
                  <SortEditorItem
                    key={sort.id}
                    table={table}
                    sort={sort}
                    sorting={sorting}
                    sortItemId={`${id}-sort-${sort.id}`}
                    columnLabels={columnLabels}
                    onSortUpdate={onSortUpdate}
                    onSortRemove={onSortRemove}
                    disabled={isDisabled}
                  />
                ))}
              </div>
            </SortableContent>
            {adding && availableColumns.length > 0 ? (
              <div className="-mx-2 border-t">
                <SortFieldPicker
                  table={table}
                  columns={availableColumns}
                  onSelect={handlePickColumn}
                />
              </div>
            ) : (
              <div className="flex items-center justify-between pt-1">
                <Button
                  variant="ghost"
                  size="sm"
                  disabled={isDisabled || availableColumns.length === 0}
                  className="text-muted-foreground h-7 px-2 font-normal"
                  onClick={() => setAdding(true)}
                >
                  <Plus className="size-3.5" />
                  Add sort
                </Button>
                <Button
                  variant="ghost"
                  size="sm"
                  disabled={isDisabled}
                  className="text-muted-foreground h-7 px-2 font-normal"
                  onClick={handleDeleteAll}
                >
                  <Trash2 className="size-3.5" />
                  Delete sort
                </Button>
              </div>
            )}
            <SortableOverlay>
              <div className="flex items-center gap-1">
                <div className="bg-primary/10 size-7 shrink-0 rounded-sm" />
                <div className="bg-primary/10 h-7 min-w-28 flex-1 rounded-sm" />
                <div className="bg-primary/10 h-7 w-[9.5rem] shrink-0 rounded-sm" />
                <div className="bg-primary/10 size-7 shrink-0 rounded-sm" />
              </div>
            </SortableOverlay>
          </Sortable>
        </PopoverContent>
      </Popover>
      <Separator
        orientation="vertical"
        className="data-vertical:mx-0.5 data-vertical:h-4 data-vertical:self-center"
      />
    </>
  );
}
