import type { Column } from "@tanstack/react-table";
import { dataTableConfig } from "@/config/data-table";
import type {
  ExtendedColumnFilter,
  FilterOperator,
  FilterVariant,
} from "@/types/data-table";

export function getColumnPinningStyle<TData>({
  column,
  offset,
  withBorder = false,
}: {
  column: Column<TData>;
  /**
   * Distance from the pinned edge, in pixels, measured off the rendered header
   * cells. TanStack's own `getStart`/`getAfter` add up each column's declared
   * `size`, but the table lays out `auto` — so those totals are not the widths
   * on screen, and every pinned column but the one hard against the edge lands
   * in the wrong place. Falls back to TanStack's figure before the first
   * measurement lands.
   */
  offset?: number;
  withBorder?: boolean;
}): React.CSSProperties {
  const isPinned = column.getIsPinned();
  const isLastLeftPinnedColumn =
    isPinned === "left" && column.getIsLastColumn("left");
  const isFirstRightPinnedColumn =
    isPinned === "right" && column.getIsFirstColumn("right");

  let boxShadow: string | undefined;
  if (withBorder) {
    if (isLastLeftPinnedColumn) {
      boxShadow = "-4px 0 4px -4px var(--border) inset";
    } else if (isFirstRightPinnedColumn) {
      boxShadow = "4px 0 4px -4px var(--border) inset";
    }
  }

  // Geometry only. Upstream also sets `background` and `opacity` here, but its
  // ternary reads `isPinned ? var(--background) : var(--background)` — so every
  // cell got an opaque inline background that beat the row's hover class and
  // killed hover feedback across the whole table. Colour now lives in classes
  // in data-table.tsx, where hover and selection can still show through on the
  // pinned column. Re-check this after any tablecn registry update.
  return {
    boxShadow,
    left:
      isPinned === "left"
        ? `${offset ?? column.getStart("left")}px`
        : undefined,
    right:
      isPinned === "right"
        ? `${offset ?? column.getAfter("right")}px`
        : undefined,
    position: isPinned ? "sticky" : "relative",
    // A percentage width makes an auto-layout table shrink this column to its
    // minimum content width, while the other columns absorb remaining space.
    width: column.columnDef.meta?.fitContent ? "1%" : column.getSize(),
    zIndex: isPinned ? 1 : undefined,
  };
}

export function getFilterOperators(filterVariant: FilterVariant) {
  const operatorMap: Record<
    FilterVariant,
    { label: string; value: FilterOperator }[]
  > = {
    text: dataTableConfig.textOperators,
    number: dataTableConfig.numericOperators,
    range: dataTableConfig.numericOperators,
    date: dataTableConfig.dateOperators,
    dateRange: dataTableConfig.dateOperators,
    boolean: dataTableConfig.booleanOperators,
    select: dataTableConfig.selectOperators,
    multiSelect: dataTableConfig.multiSelectOperators,
  };

  return operatorMap[filterVariant] ?? dataTableConfig.textOperators;
}

export function getDefaultFilterOperator(filterVariant: FilterVariant) {
  const operators = getFilterOperators(filterVariant);

  return operators[0]?.value ?? (filterVariant === "text" ? "iLike" : "eq");
}

export function getValidFilters<TData>(
  filters: ExtendedColumnFilter<TData>[]
): ExtendedColumnFilter<TData>[] {
  return filters.filter(
    (filter) =>
      filter.operator === "isEmpty" ||
      filter.operator === "isNotEmpty" ||
      (Array.isArray(filter.value)
        ? filter.value.length > 0
        : filter.value !== "" &&
          filter.value !== null &&
          filter.value !== undefined)
  );
}

function toSafeString(val: unknown): string {
  if (val === null || val === undefined) return "";
  if (typeof val === "string") return val;
  if (
    typeof val === "number" ||
    typeof val === "boolean" ||
    typeof val === "bigint"
  ) {
    return String(val);
  }
  return "";
}

function evaluateArrayFilter(
  rawValue: unknown,
  targetArray: string[],
  isInclude: boolean
): boolean {
  let hasMatch = false;
  if (Array.isArray(rawValue)) {
    hasMatch = rawValue.some((r) =>
      targetArray.includes(toSafeString(r).toLowerCase())
    );
  } else {
    hasMatch = targetArray.includes(toSafeString(rawValue).toLowerCase());
  }
  return isInclude ? hasMatch : !hasMatch;
}

function evaluateNumericFilter(
  rawValue: unknown,
  value: unknown,
  operator: string
): boolean {
  const num = Number(rawValue);
  if (operator === "isBetween") {
    if (Array.isArray(value) && value.length === 2) {
      return num >= Number(value[0]) && num <= Number(value[1]);
    }
    return true;
  }
  const target = Number(value);
  if (operator === "lt") return num < target;
  if (operator === "lte") return num <= target;
  if (operator === "gt") return num > target;
  if (operator === "gte") return num >= target;
  return true;
}

function evaluateTextFilter(
  strRaw: string,
  strVal: string,
  operator: string
): boolean {
  if (operator === "iLike") return strRaw.includes(strVal);
  if (operator === "notILike") return !strRaw.includes(strVal);
  if (operator === "eq") return strRaw === strVal;
  if (operator === "ne") return strRaw !== strVal;
  return true;
}

function evaluateFilter<TData>(
  item: TData,
  filter: ExtendedColumnFilter<TData>
): boolean {
  const rawValue = (item as Record<string, unknown>)[filter.id];
  const { operator, value } = filter;

  if (operator === "isEmpty") {
    return (
      rawValue === null ||
      rawValue === undefined ||
      rawValue === "" ||
      (Array.isArray(rawValue) && rawValue.length === 0)
    );
  }
  if (operator === "isNotEmpty") {
    return (
      rawValue !== null &&
      rawValue !== undefined &&
      rawValue !== "" &&
      (!Array.isArray(rawValue) || rawValue.length > 0)
    );
  }

  if (
    value === null ||
    value === undefined ||
    (Array.isArray(value) && value.length === 0) ||
    value === ""
  ) {
    return true;
  }

  if (operator === "inArray" || operator === "notInArray") {
    const targetArray = Array.isArray(value)
      ? value.map((v) => toSafeString(v).toLowerCase())
      : [toSafeString(value).toLowerCase()];
    return evaluateArrayFilter(rawValue, targetArray, operator === "inArray");
  }

  if (
    operator === "lt" ||
    operator === "lte" ||
    operator === "gt" ||
    operator === "gte" ||
    operator === "isBetween"
  ) {
    return evaluateNumericFilter(rawValue, value, operator);
  }

  if (typeof rawValue === "boolean") {
    const boolTarget = value === "true" || value === true;
    return operator === "eq" ? rawValue === boolTarget : rawValue !== boolTarget;
  }

  return evaluateTextFilter(
    toSafeString(rawValue).toLowerCase(),
    toSafeString(value).toLowerCase(),
    operator
  );
}

/**
 * Applies search query and advanced filter rules to an in-memory dataset.
 */
export function filterDataClientSide<TData>(
  data: TData[],
  options: {
    search?: string;
    searchFields?: (keyof TData | ((item: TData) => unknown))[];
    filters?: ExtendedColumnFilter<TData>[] | string | null;
    joinOperator?: "and" | "or";
  }
): TData[] {
  const { search, searchFields, filters: rawFilters, joinOperator = "and" } =
    options;

  let parsedFilters: ExtendedColumnFilter<TData>[] = [];
  if (Array.isArray(rawFilters)) {
    parsedFilters = rawFilters;
  } else if (typeof rawFilters === "string" && rawFilters.trim()) {
    try {
      const parsed = JSON.parse(rawFilters);
      if (Array.isArray(parsed)) {
        parsedFilters = parsed;
      }
    } catch {
      // ignore JSON parse error
    }
  }

  const validFilters = getValidFilters(parsedFilters);
  const q = search?.trim().toLowerCase();

  return data.filter((item) => {
    // 1. Text search
    if (q) {
      if (searchFields && searchFields.length > 0) {
        const matchesSearch = searchFields.some((field) => {
          const val =
            typeof field === "function" ? field(item) : item[field];
          return toSafeString(val)
            .toLowerCase()
            .includes(q);
        });
        if (!matchesSearch) return false;
      } else {
        const matchesSearch = Object.values(
          item as Record<string, unknown>
        ).some((val) =>
          toSafeString(val)
            .toLowerCase()
            .includes(q)
        );
        if (!matchesSearch) return false;
      }
    }

    // 2. Advanced filters
    if (validFilters.length > 0) {
      if (joinOperator === "or") {
        return validFilters.some((filter) => evaluateFilter(item, filter));
      }
      return validFilters.every((filter) => evaluateFilter(item, filter));
    }

    return true;
  });
}
