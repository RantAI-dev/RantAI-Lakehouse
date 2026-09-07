"use client";

import type { Column, ColumnMeta } from "@tanstack/react-table";
import {
  CalendarIcon,
  ChevronsUpDown,
  Copy,
  MoreHorizontal,
  Trash2,
} from "lucide-react";
import * as React from "react";

import { DataTableRangeFilter } from "@/components/data-table/data-table-range-filter";
import { Button } from "@/components/ui/button";
import { Calendar } from "@rantai/design-system/ui/calendar";
import {
  Command,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
} from "@/components/ui/command";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@rantai/design-system/ui/dropdown-menu";
import {
  Faceted,
  FacetedBadgeList,
  FacetedContent,
  FacetedEmpty,
  FacetedGroup,
  FacetedInput,
  FacetedItem,
  FacetedList,
  FacetedTrigger,
} from "@/components/ui/faceted";
import { Input } from "@/components/ui/input";
import {
  Popover,
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
import { dataTableConfig } from "@/config/data-table";
import {
  getDefaultFilterOperator,
  getFilterOperators,
} from "@/lib/data-table";
import { formatDate } from "@/lib/format";
import { cn } from "@/lib/utils";
import type {
  ExtendedColumnFilter,
  FilterOperator,
  JoinOperator,
} from "@/types/data-table";

const REMOVE_FILTER_SHORTCUTS = ["backspace", "delete"];

const JOIN_OPERATOR_LABELS: Record<JoinOperator, string> = {
  and: "And",
  or: "Or",
};

export function formatFilterSummary<TData = unknown>(
  filter: ExtendedColumnFilter<TData>
): string | null {
  if (filter.operator === "isEmpty") return "is empty";
  if (filter.operator === "isNotEmpty") return "is not empty";

  const hasValue = Array.isArray(filter.value)
    ? filter.value.length > 0
    : filter.value !== "" && filter.value != null;

  if (!hasValue) return null;

  if (Array.isArray(filter.value)) {
    return filter.value.join(", ");
  }

  if (filter.variant === "date" || filter.variant === "dateRange") {
    const date = new Date(Number(filter.value));
    if (!Number.isNaN(date.getTime())) {
      return formatDate(date, { month: "short", day: "numeric" });
    }
  }

  return String(filter.value);
}

export function isFilterActive<TData = unknown>(
  filter: ExtendedColumnFilter<TData>
): boolean {
  if (filter.operator === "isEmpty" || filter.operator === "isNotEmpty") {
    return true;
  }
  if (Array.isArray(filter.value)) return filter.value.length > 0;
  return filter.value !== "" && filter.value != null;
}

export interface DataTableFilterItemProps<TData> {
  filter: ExtendedColumnFilter<TData>;
  index: number;
  filterItemId: string;
  joinOperator: JoinOperator;
  setJoinOperator: (value: JoinOperator) => void;
  columns: Column<TData>[];
  onFilterUpdate: (
    filterId: string,
    updates: Partial<Omit<ExtendedColumnFilter<TData>, "filterId">>
  ) => void;
  onFilterRemove: (filterId: string) => void;
  onFilterDuplicate: (filterId: string) => void;
}

export function DataTableFilterItem<TData>({
  filter,
  index,
  filterItemId,
  joinOperator,
  setJoinOperator,
  columns,
  onFilterUpdate,
  onFilterRemove,
  onFilterDuplicate,
}: DataTableFilterItemProps<TData>) {
  const [showFieldSelector, setShowFieldSelector] = React.useState(false);
  const [showOperatorSelector, setShowOperatorSelector] = React.useState(false);
  const [showValueSelector, setShowValueSelector] = React.useState(false);

  const column = columns.find((column) => column.id === filter.id);

  const joinOperatorListboxId = `${filterItemId}-join-operator-listbox`;
  const fieldListboxId = `${filterItemId}-field-listbox`;
  const operatorListboxId = `${filterItemId}-operator-listbox`;
  const inputId = `${filterItemId}-input`;

  const columnMeta = column?.columnDef.meta;
  const filterOperators = getFilterOperators(filter.variant);

  const onItemKeyDown = React.useCallback(
    (event: React.KeyboardEvent<HTMLDivElement>) => {
      if (
        event.target instanceof HTMLInputElement ||
        event.target instanceof HTMLTextAreaElement
      ) {
        return;
      }

      if (showFieldSelector || showOperatorSelector || showValueSelector) {
        return;
      }

      if (REMOVE_FILTER_SHORTCUTS.includes(event.key.toLowerCase())) {
        event.preventDefault();
        onFilterRemove(filter.filterId);
      }
    },
    [
      filter.filterId,
      showFieldSelector,
      showOperatorSelector,
      showValueSelector,
      onFilterRemove,
    ]
  );

  if (!column) return null;

  const FieldIcon = columnMeta?.icon;
  const fieldLabel = columnMeta?.label ?? column.id;

  return (
    <div
      role="listitem"
      id={filterItemId}
      tabIndex={-1}
      className="flex min-w-max items-center gap-1.5"
      onKeyDown={onItemKeyDown}
    >
      <div className="flex w-[4.75rem] shrink-0 items-center justify-start">
        {index === 0 ? (
          <span className="text-muted-foreground px-1 text-sm">Where</span>
        ) : (
          <Select
            value={joinOperator}
            onValueChange={(value: JoinOperator) => setJoinOperator(value)}
          >
            <SelectTrigger
              size="sm"
              aria-label="Select join operator"
              aria-controls={joinOperatorListboxId}
              className="h-7 w-[4.75rem] px-2 font-normal"
            >
              <SelectValue>
                {JOIN_OPERATOR_LABELS[joinOperator]}
              </SelectValue>
            </SelectTrigger>
            <SelectContent
              id={joinOperatorListboxId}
              position="popper"
              className="min-w-(--radix-select-trigger-width)"
            >
              <SelectGroup>
                {dataTableConfig.joinOperators.map((op) => (
                  <SelectItem key={op} value={op}>
                    {JOIN_OPERATOR_LABELS[op]}
                  </SelectItem>
                ))}
              </SelectGroup>
            </SelectContent>
          </Select>
        )}
      </div>
      <Popover open={showFieldSelector} onOpenChange={setShowFieldSelector}>
        <PopoverTrigger asChild>
          <Button
            aria-controls={fieldListboxId}
            variant="outline"
            size="sm"
            className="h-7 min-w-28 justify-between gap-1.5 px-2 font-normal"
          >
            <span className="inline-flex min-w-0 items-center gap-1.5 truncate">
              {FieldIcon ? (
                <FieldIcon className="size-3.5 shrink-0 opacity-70" />
              ) : null}
              <span className="truncate">{fieldLabel}</span>
            </span>
            <ChevronsUpDown className="size-3.5 shrink-0 opacity-50" />
          </Button>
        </PopoverTrigger>
        <PopoverContent
          id={fieldListboxId}
          align="start"
          className="w-48 p-0"
        >
          <Command>
            <CommandInput placeholder="Search fields..." />
            <CommandList>
              <CommandEmpty>No fields found.</CommandEmpty>
              <CommandGroup>
                {columns.map((columnOption) => {
                  const Icon = columnOption.columnDef.meta?.icon;
                  return (
                    <CommandItem
                      key={columnOption.id}
                      value={columnOption.id}
                      data-checked={columnOption.id === filter.id}
                      onSelect={(value) => {
                        onFilterUpdate(filter.filterId, {
                          id: value as Extract<keyof TData, string>,
                          variant:
                            columnOption.columnDef.meta?.variant ?? "text",
                          operator: getDefaultFilterOperator(
                            columnOption.columnDef.meta?.variant ?? "text"
                          ),
                          value: "",
                        });

                        setShowFieldSelector(false);
                      }}
                    >
                      {Icon ? <Icon className="size-3.5 shrink-0" /> : null}
                      <span className="truncate">
                        {columnOption.columnDef.meta?.label}
                      </span>
                    </CommandItem>
                  );
                })}
              </CommandGroup>
            </CommandList>
          </Command>
        </PopoverContent>
      </Popover>
      <Select
        open={showOperatorSelector}
        onOpenChange={setShowOperatorSelector}
        value={filter.operator}
        onValueChange={(value: FilterOperator) =>
          onFilterUpdate(filter.filterId, {
            operator: value,
            value:
              value === "isEmpty" || value === "isNotEmpty" ? "" : filter.value,
          })
        }
      >
        <SelectTrigger
          size="sm"
          aria-controls={operatorListboxId}
          className="h-7 w-28 px-2 font-normal"
        >
          <div className="truncate">
            <SelectValue placeholder={filter.operator} />
          </div>
        </SelectTrigger>
        <SelectContent id={operatorListboxId}>
          <SelectGroup>
            {filterOperators.map((operator) => (
              <SelectItem key={operator.value} value={operator.value}>
                {operator.label}
              </SelectItem>
            ))}
          </SelectGroup>
        </SelectContent>
      </Select>
      <div className="max-w-56 min-w-28 flex-1">
        <DataTableFilterValueInput
          filter={filter}
          inputId={inputId}
          column={column}
          columnMeta={columnMeta}
          onFilterUpdate={onFilterUpdate}
          showValueSelector={showValueSelector}
          setShowValueSelector={setShowValueSelector}
        />
      </div>
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button
            variant="ghost"
            size="icon-sm"
            className="text-muted-foreground size-7 shrink-0"
            aria-label="Filter rule actions"
          >
            <MoreHorizontal />
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end">
          <DropdownMenuItem onClick={() => onFilterDuplicate(filter.filterId)}>
            <Copy />
            Duplicate
          </DropdownMenuItem>
          <DropdownMenuItem
            variant="destructive"
            onClick={() => onFilterRemove(filter.filterId)}
          >
            <Trash2 />
            Delete
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
    </div>
  );
}

export function DataTableFilterValueInput<TData>({
  filter,
  inputId,
  column,
  columnMeta,
  onFilterUpdate,
  showValueSelector,
  setShowValueSelector,
}: {
  filter: ExtendedColumnFilter<TData>;
  inputId: string;
  column: Column<TData>;
  columnMeta?: ColumnMeta<TData, unknown>;
  onFilterUpdate: (
    filterId: string,
    updates: Partial<Omit<ExtendedColumnFilter<TData>, "filterId">>
  ) => void;
  showValueSelector: boolean;
  setShowValueSelector: (value: boolean) => void;
}) {
  if (filter.operator === "isEmpty" || filter.operator === "isNotEmpty") {
    return (
      <div
        id={inputId}
        role="status"
        aria-label={`${columnMeta?.label} filter is ${
          filter.operator === "isEmpty" ? "empty" : "not empty"
        }`}
        aria-live="polite"
        className="dark:bg-input/30 h-8 w-full rounded border bg-transparent"
      />
    );
  }

  switch (filter.variant) {
    case "text":
    case "number":
    case "range": {
      if (
        (filter.variant === "range" && filter.operator === "isBetween") ||
        filter.operator === "isBetween"
      ) {
        return (
          <DataTableRangeFilter
            filter={filter}
            column={column}
            inputId={inputId}
            onFilterUpdate={onFilterUpdate}
          />
        );
      }

      const isNumber =
        filter.variant === "number" || filter.variant === "range";

      return (
        <Input
          id={inputId}
          type={isNumber ? "number" : filter.variant}
          aria-label={`${columnMeta?.label} filter value`}
          aria-describedby={`${inputId}-description`}
          inputMode={isNumber ? "numeric" : undefined}
          placeholder={columnMeta?.placeholder ?? "Enter a value..."}
          className="h-7 w-full rounded-md"
          data-filter-value=""
          defaultValue={
            typeof filter.value === "string" ? filter.value : undefined
          }
          onChange={(event) =>
            onFilterUpdate(filter.filterId, {
              value: event.target.value,
            })
          }
        />
      );
    }

    case "boolean": {
      if (Array.isArray(filter.value)) return null;

      const inputListboxId = `${inputId}-listbox`;

      return (
        <Select
          open={showValueSelector}
          onOpenChange={setShowValueSelector}
          value={filter.value}
          onValueChange={(value) =>
            onFilterUpdate(filter.filterId, {
              value,
            })
          }
        >
          <SelectTrigger
            id={inputId}
            aria-controls={inputListboxId}
            aria-label={`${columnMeta?.label} boolean filter`}
            className="w-full rounded"
            data-filter-value=""
          >
            <SelectValue placeholder={filter.value ? "True" : "False"} />
          </SelectTrigger>
          <SelectContent id={inputListboxId}>
            <SelectGroup>
              <SelectItem value="true">True</SelectItem>
              <SelectItem value="false">False</SelectItem>
            </SelectGroup>
          </SelectContent>
        </Select>
      );
    }

    case "select":
    case "multiSelect": {
      const inputListboxId = `${inputId}-listbox`;

      const multiple = filter.variant === "multiSelect";
      const selectedValues = multiple
        ? Array.isArray(filter.value)
          ? filter.value
          : []
        : typeof filter.value === "string"
          ? filter.value
          : undefined;

      return (
        <Faceted
          open={showValueSelector}
          onOpenChange={setShowValueSelector}
          value={selectedValues}
          onValueChange={(value) => {
            onFilterUpdate(filter.filterId, {
              value,
            });
          }}
          multiple={multiple}
        >
          <FacetedTrigger asChild>
            <Button
              id={inputId}
              aria-controls={inputListboxId}
              aria-label={`${columnMeta?.label} filter value${multiple ? "s" : ""}`}
              variant="outline"
              className="w-full rounded font-normal"
              data-filter-value=""
            >
              <FacetedBadgeList
                options={columnMeta?.options}
                placeholder={
                  columnMeta?.placeholder ??
                  `Select option${multiple ? "s" : ""}...`
                }
              />
            </Button>
          </FacetedTrigger>
          <FacetedContent id={inputListboxId} className="w-[200px]">
            <FacetedInput
              aria-label={`Search ${columnMeta?.label} options`}
              placeholder={columnMeta?.placeholder ?? "Search options..."}
            />
            <FacetedList>
              <FacetedEmpty>No options found.</FacetedEmpty>
              <FacetedGroup>
                {columnMeta?.options?.map((option) => (
                  <FacetedItem key={option.value} value={option.value}>
                    {option.icon && <option.icon />}
                    <span>{option.label}</span>
                    {option.count && (
                      <span className="ml-auto font-mono text-xs">
                        {option.count}
                      </span>
                    )}
                  </FacetedItem>
                ))}
              </FacetedGroup>
            </FacetedList>
          </FacetedContent>
        </Faceted>
      );
    }

    case "date":
    case "dateRange": {
      const inputListboxId = `${inputId}-listbox`;

      const dateValue = Array.isArray(filter.value)
        ? filter.value.filter(Boolean)
        : [filter.value, filter.value].filter(Boolean);

      const startDate = dateValue[0]
        ? new Date(Number(dateValue[0]))
        : undefined;
      const endDate = dateValue[1] ? new Date(Number(dateValue[1])) : undefined;

      const isSameDate =
        startDate &&
        endDate &&
        startDate.toDateString() === endDate.toDateString();

      const displayValue =
        filter.operator === "isBetween" && dateValue.length === 2 && !isSameDate
          ? `${formatDate(startDate, { month: "short" })} - ${formatDate(endDate, { month: "short" })}`
          : startDate
            ? formatDate(startDate, { month: "short" })
            : "Pick a date";

      return (
        <Popover open={showValueSelector} onOpenChange={setShowValueSelector}>
          <PopoverTrigger asChild>
            <Button
              id={inputId}
              aria-controls={inputListboxId}
              aria-label={`${columnMeta?.label} date filter`}
              variant="outline"
              className={cn(
                "w-full justify-start rounded text-left font-normal",
                !filter.value && "text-muted-foreground"
              )}
              data-filter-value=""
            >
              <CalendarIcon />
              <span className="truncate">{displayValue}</span>
            </Button>
          </PopoverTrigger>
          <PopoverContent
            id={inputListboxId}
            align="start"
            className="w-auto p-0"
          >
            {filter.operator === "isBetween" ? (
              <Calendar
                aria-label={`Select ${columnMeta?.label} date range`}
                autoFocus
                captionLayout="dropdown"
                mode="range"
                selected={
                  dateValue.length === 2
                    ? {
                        from: new Date(Number(dateValue[0])),
                        to: new Date(Number(dateValue[1])),
                      }
                    : {
                        from: new Date(),
                        to: new Date(),
                      }
                }
                onSelect={(date) => {
                  onFilterUpdate(filter.filterId, {
                    value: date
                      ? [
                          (date.from?.getTime() ?? "").toString(),
                          (date.to?.getTime() ?? "").toString(),
                        ]
                      : [],
                  });
                }}
              />
            ) : (
              <Calendar
                aria-label={`Select ${columnMeta?.label} date`}
                autoFocus
                captionLayout="dropdown"
                mode="single"
                selected={
                  dateValue[0] ? new Date(Number(dateValue[0])) : undefined
                }
                onSelect={(date) => {
                  onFilterUpdate(filter.filterId, {
                    value: (date?.getTime() ?? "").toString(),
                  });
                  setShowValueSelector(false);
                }}
              />
            )}
          </PopoverContent>
        </Popover>
      );
    }

    default:
      return null;
  }
}

interface DataTableColumnFilterEditorProps<TData> {
  filter: ExtendedColumnFilter<TData>;
  column: Column<TData>;
  onFilterUpdate: (
    filterId: string,
    updates: Partial<Omit<ExtendedColumnFilter<TData>, "filterId">>
  ) => void;
  onClear: () => void;
}

/** Compact operator + value editor for the Notion-style column chip popover. */
export function DataTableColumnFilterEditor<TData>({
  filter,
  column,
  onFilterUpdate,
  onClear,
}: DataTableColumnFilterEditorProps<TData>) {
  const inputId = React.useId();
  const operatorListboxId = `${inputId}-operator-listbox`;
  const [showValueSelector, setShowValueSelector] = React.useState(false);
  const columnMeta = column.columnDef.meta;
  const filterOperators = getFilterOperators(filter.variant);

  return (
    <div className="flex w-64 flex-col gap-3 p-3">
      <Select
        value={filter.operator}
        onValueChange={(value: FilterOperator) =>
          onFilterUpdate(filter.filterId, {
            operator: value,
            value:
              value === "isEmpty" || value === "isNotEmpty" ? "" : filter.value,
          })
        }
      >
        <SelectTrigger
          aria-controls={operatorListboxId}
          className="w-full rounded lowercase"
        >
          <SelectValue placeholder={filter.operator} />
        </SelectTrigger>
        <SelectContent id={operatorListboxId}>
          <SelectGroup>
            {filterOperators.map((operator) => (
              <SelectItem
                key={operator.value}
                value={operator.value}
                className="lowercase"
              >
                {operator.label}
              </SelectItem>
            ))}
          </SelectGroup>
        </SelectContent>
      </Select>
      <DataTableFilterValueInput
        filter={filter}
        inputId={inputId}
        column={column}
        columnMeta={columnMeta}
        onFilterUpdate={onFilterUpdate}
        showValueSelector={showValueSelector}
        setShowValueSelector={setShowValueSelector}
      />
      {isFilterActive(filter) ? (
        <Button variant="outline" size="sm" className="rounded" onClick={onClear}>
          Clear filter
        </Button>
      ) : null}
    </div>
  );
}
