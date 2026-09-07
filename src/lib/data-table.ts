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

  // Geometry only. Upstream also sets `background` and `opacity` here, but its
  // ternary reads `isPinned ? var(--background) : var(--background)` — so every
  // cell got an opaque inline background that beat the row's hover class and
  // killed hover feedback across the whole table. Colour now lives in classes
  // in data-table.tsx, where hover and selection can still show through on the
  // pinned column. Re-check this after any tablecn registry update.
  return {
    boxShadow: withBorder
      ? isLastLeftPinnedColumn
        ? "-4px 0 4px -4px var(--border) inset"
        : isFirstRightPinnedColumn
          ? "4px 0 4px -4px var(--border) inset"
          : undefined
      : undefined,
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
