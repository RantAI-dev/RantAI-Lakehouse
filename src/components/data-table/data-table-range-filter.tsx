"use client";

import type { Column } from "@tanstack/react-table";
import * as React from "react";

import { Input } from "@/components/ui/input";
import { cn } from "@/lib/utils";
import type { ExtendedColumnFilter } from "@/types/data-table";

interface DataTableRangeFilterProps<TData> extends React.ComponentProps<"div"> {
  filter: ExtendedColumnFilter<TData>;
  column: Column<TData>;
  inputId: string;
  onFilterUpdate: (
    filterId: string,
    updates: Partial<Omit<ExtendedColumnFilter<TData>, "filterId">>,
  ) => void;
}

function RangeNumberInput({
  id,
  ariaLabel,
  ariaValuemin,
  ariaValuemax,
  dataSlot,
  placeholder,
  min,
  max,
  className,
  value: externalValue,
  onChange,
}: {
  id: string;
  ariaLabel?: string;
  ariaValuemin?: number;
  ariaValuemax?: number;
  dataSlot?: string;
  placeholder?: string;
  min?: number;
  max?: number;
  className?: string;
  value: string;
  onChange: (value: string) => void;
}) {
  const [localValue, setLocalValue] = React.useState(externalValue);
  const pushedRef = React.useRef(externalValue);

  React.useEffect(() => {
    if (externalValue !== pushedRef.current) {
      pushedRef.current = externalValue;
      setLocalValue(externalValue);
    }
  }, [externalValue]);

  const handleChange = (event: React.ChangeEvent<HTMLInputElement>) => {
    const next = event.target.value;
    setLocalValue(next);
    pushedRef.current = next;
    onChange(next);
  };

  return (
    <Input
      id={id}
      type="number"
      aria-label={ariaLabel}
      aria-valuemin={ariaValuemin}
      aria-valuemax={ariaValuemax}
      data-slot={dataSlot}
      data-filter-value=""
      inputMode="numeric"
      placeholder={placeholder}
      min={min}
      max={max}
      className={className}
      value={localValue}
      onChange={handleChange}
    />
  );
}

export function DataTableRangeFilter<TData>({
  filter,
  column,
  inputId,
  onFilterUpdate,
  className,
  ...props
}: DataTableRangeFilterProps<TData>) {
  const meta = column.columnDef.meta;

  const [min, max] = React.useMemo(() => {
    const range = column.columnDef.meta?.range;
    if (range) return range;

    const values = column.getFacetedMinMaxValues();
    if (!values) return [0, 100];

    return [values[0], values[1]];
  }, [column]);

  const formatValue = React.useCallback(
    (value: string | number | undefined) => {
      if (value === undefined || value === "") return "";
      const numValue = Number(value);
      return Number.isNaN(numValue) ? "" : String(numValue);
    },
    [],
  );

  const value = React.useMemo(() => {
    if (Array.isArray(filter.value)) return filter.value.map(formatValue);
    return [formatValue(filter.value), ""];
  }, [filter.value, formatValue]);

  const onRangeValueChange = React.useCallback(
    (value: string, isMin?: boolean) => {
      const numValue = Number(value);
      const currentValues = Array.isArray(filter.value)
        ? filter.value
        : ["", ""];
      const otherValue = isMin
        ? (currentValues[1] ?? "")
        : (currentValues[0] ?? "");

      if (
        value === "" ||
        (!Number.isNaN(numValue) &&
          (isMin
            ? numValue >= min && numValue <= (Number(otherValue) || max)
            : numValue <= max && numValue >= (Number(otherValue) || min)))
      ) {
        onFilterUpdate(filter.filterId, {
          value: isMin ? [value, otherValue] : [otherValue, value],
        });
      }
    },
    [filter.filterId, filter.value, min, max, onFilterUpdate],
  );

  return (
    <div
      data-slot="range"
      className={cn("flex w-full items-center gap-2", className)}
      {...props}
    >
      <RangeNumberInput
        id={`${inputId}-min`}
        ariaLabel={`${meta?.label} minimum value`}
        ariaValuemin={min}
        ariaValuemax={max}
        dataSlot="range-min"
        placeholder={min.toString()}
        min={min}
        max={max}
        className="h-8 w-full rounded"
        value={value[0] ?? ""}
        onChange={(val) => onRangeValueChange(val, true)}
      />
      <span className="sr-only shrink-0 text-muted-foreground">to</span>
      <RangeNumberInput
        id={`${inputId}-max`}
        ariaLabel={`${meta?.label} maximum value`}
        ariaValuemin={min}
        ariaValuemax={max}
        dataSlot="range-max"
        placeholder={max.toString()}
        min={min}
        max={max}
        className="h-8 w-full rounded"
        value={value[1] ?? ""}
        onChange={(val) => onRangeValueChange(val)}
      />
    </div>
  );
}
