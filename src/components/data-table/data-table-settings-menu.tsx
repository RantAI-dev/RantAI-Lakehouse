"use client";

import type { Table } from "@tanstack/react-table";
import {
  ArrowUpDown,
  ChevronDown,
  ChevronUp,
  Columns3,
  GripVertical,
  Layers2,
  RefreshCw,
  RotateCcw,
  Settings2,
  X,
} from "lucide-react";
import * as React from "react";

import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuPortal,
  DropdownMenuSeparator,
  DropdownMenuSub,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
  DropdownMenuTrigger,
} from "@rantai/design-system/ui/dropdown-menu";
import {
  Sortable,
  SortableContent,
  SortableItem,
  SortableItemHandle,
} from "@/components/ui/sortable";
import { Spinner } from "@/components/ui/spinner";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@rantai/design-system/ui/tooltip";

interface DataTableSettingsMenuProps<TData> {
  table: Table<TData>;
  /** Refetches the page's own query. The menu hides the item without it. */
  onRefresh?: () => void;
  isRefreshing?: boolean;
  /**
   * Whether the rows on screen are actually laid out in columns. The list view
   * renders fixed fields, so hiding, reordering or resetting columns there
   * would change nothing the user can see — only Refresh still means something.
   */
  columnControls?: boolean;
  disabled?: boolean;
}

export function DataTableSettingsMenu<TData>({
  table,
  onRefresh,
  isRefreshing,
  columnControls = true,
  disabled,
}: DataTableSettingsMenuProps<TData>) {
  const resetLayout = table.options.meta?.resetLayout;
  const groupBy = table.options.meta?.groupBy ?? null;
  const setGroupBy = table.options.meta?.setGroupBy;

  const groupableColumns = React.useMemo(() => {
    return table
      .getAllLeafColumns()
      .filter((column) => {
        if (!column.columnDef.meta?.enableGrouping) return false;
        return column.accessorFn != null;
      })
      .map((column) => ({
        id: column.id,
        label: column.columnDef.meta?.label ?? column.id,
      }));
  }, [table]);

  const activeGroupLabel =
    groupableColumns.find((column) => column.id === groupBy)?.label ?? groupBy;

  // Leaf columns, not `getAllColumns()` — only the leaf list is run through the
  // column order, so this keeps both lists in step after a reorder.
  const hideableColumns = table
    .getAllLeafColumns()
    .filter(
      (column) =>
        typeof column.accessorFn !== "undefined" && column.getCanHide()
    );

  // Pinned columns are held against the edges and are not part of the run being
  // sorted, so this covers the same columns a header drag can move.
  const movableColumns = table.getCenterVisibleLeafColumns();

  function applyOrder(ids: string[]) {
    // Refill only the slots this menu is allowed to touch, so pinned and hidden
    // columns keep the positions they already had.
    const movable = new Set(ids);
    let cursor = 0;
    table.setColumnOrder(
      table
        .getAllLeafColumns()
        .map((column) => (movable.has(column.id) ? ids[cursor++] : column.id))
    );
  }

  function moveColumn(index: number, delta: number) {
    const ids = movableColumns.map((column) => column.id);
    const target = index + delta;
    if (target < 0 || target >= ids.length) return;

    [ids[index], ids[target]] = [ids[target], ids[index]];
    applyOrder(ids);
  }

  return (
    <DropdownMenu>
      <Tooltip>
        <TooltipTrigger asChild>
          <DropdownMenuTrigger asChild>
            <Button
              aria-label="Table settings"
              variant="outline"
              size="icon"
              disabled={disabled}
            >
              <Settings2 className="text-muted-foreground" />
            </Button>
          </DropdownMenuTrigger>
        </TooltipTrigger>
        <TooltipContent>Settings</TooltipContent>
      </Tooltip>

      <DropdownMenuContent align="end" className="w-44">
        {columnControls && (
          <>
            <DropdownMenuSub>
              <DropdownMenuSubTrigger>
                <Columns3 className="text-muted-foreground" />
                View
              </DropdownMenuSubTrigger>
              <DropdownMenuPortal>
                <DropdownMenuSubContent className="max-h-72 w-48 overflow-y-auto">
                  {hideableColumns.map((column) => (
                    <DropdownMenuCheckboxItem
                      key={column.id}
                      checked={column.getIsVisible()}
                      // Radix closes the whole menu on select; holding it open lets
                      // several columns be toggled in one visit.
                      onSelect={(event) => event.preventDefault()}
                      onCheckedChange={(checked) =>
                        column.toggleVisibility(checked)
                      }
                    >
                      <span className="truncate">
                        {column.columnDef.meta?.label ?? column.id}
                      </span>
                    </DropdownMenuCheckboxItem>
                  ))}
                </DropdownMenuSubContent>
              </DropdownMenuPortal>
            </DropdownMenuSub>

            <DropdownMenuSub>
              <DropdownMenuSubTrigger>
                <ArrowUpDown className="text-muted-foreground" />
                Reorder
              </DropdownMenuSubTrigger>
              <DropdownMenuPortal>
                <DropdownMenuSubContent className="max-h-72 w-64 overflow-y-auto">
                  <Sortable
                    value={movableColumns}
                    onValueChange={(next) =>
                      applyOrder(next.map((column) => column.id))
                    }
                    getItemValue={(column) => column.id}
                  >
                    <SortableContent asChild>
                      <div role="list" className="flex flex-col">
                        {movableColumns.map((column, index) => {
                          const label =
                            column.columnDef.meta?.label ?? column.id;

                          return (
                            // A plain row, not a DropdownMenuItem: an item would close the
                            // menu on click and swallow the buttons' own activation.
                            <SortableItem
                              key={column.id}
                              value={column.id}
                              asChild
                            >
                              <div
                                role="listitem"
                                className="flex items-center gap-1 rounded-sm py-1 pr-1 pl-1 text-sm data-[dragging=true]:opacity-80"
                              >
                                <Tooltip delayDuration={400}>
                                  <TooltipTrigger asChild>
                                    <SortableItemHandle asChild>
                                      <Button
                                        variant="ghost"
                                        size="icon-xs"
                                        aria-label={`Drag ${label} to reorder`}
                                        className="text-muted-foreground cursor-grab data-[dragging=true]:cursor-grabbing"
                                      >
                                        <GripVertical />
                                      </Button>
                                    </SortableItemHandle>
                                  </TooltipTrigger>
                                  <TooltipContent>
                                    Drag to reorder
                                  </TooltipContent>
                                </Tooltip>
                                <span className="flex-1 truncate">{label}</span>
                                <Tooltip delayDuration={400}>
                                  <TooltipTrigger asChild>
                                    <Button
                                      variant="ghost"
                                      size="icon-xs"
                                      // Specific for screen readers, where a column of
                                      // repeated "Move up" would say nothing.
                                      aria-label={`Move ${label} up`}
                                      disabled={index === 0}
                                      className="text-muted-foreground"
                                      onClick={() => moveColumn(index, -1)}
                                    >
                                      <ChevronUp />
                                    </Button>
                                  </TooltipTrigger>
                                  <TooltipContent>Move up</TooltipContent>
                                </Tooltip>
                                <Tooltip delayDuration={400}>
                                  <TooltipTrigger asChild>
                                    <Button
                                      variant="ghost"
                                      size="icon-xs"
                                      aria-label={`Move ${label} down`}
                                      disabled={
                                        index === movableColumns.length - 1
                                      }
                                      className="text-muted-foreground"
                                      onClick={() => moveColumn(index, 1)}
                                    >
                                      <ChevronDown />
                                    </Button>
                                  </TooltipTrigger>
                                  <TooltipContent>Move down</TooltipContent>
                                </Tooltip>
                              </div>
                            </SortableItem>
                          );
                        })}
                      </div>
                    </SortableContent>
                  </Sortable>
                </DropdownMenuSubContent>
              </DropdownMenuPortal>
            </DropdownMenuSub>
          </>
        )}

        {setGroupBy && groupableColumns.length > 0 && (
          <DropdownMenuSub>
            <DropdownMenuSubTrigger>
              <Layers2 className="text-muted-foreground" />
              Group
              {groupBy ? (
                <span className="text-muted-foreground ml-auto truncate text-xs">
                  {activeGroupLabel}
                </span>
              ) : null}
            </DropdownMenuSubTrigger>
            <DropdownMenuPortal>
              <DropdownMenuSubContent className="max-h-72 w-48 overflow-y-auto">
                {groupableColumns.map((column) => (
                  <DropdownMenuItem
                    key={column.id}
                    onSelect={() =>
                      setGroupBy(column.id === groupBy ? null : column.id)
                    }
                  >
                    <span className="truncate">{column.label}</span>
                    {column.id === groupBy ? (
                      <span className="text-muted-foreground ml-auto text-xs">
                        Active
                      </span>
                    ) : null}
                  </DropdownMenuItem>
                ))}
                {groupBy ? (
                  <>
                    <DropdownMenuSeparator />
                    <DropdownMenuItem onSelect={() => setGroupBy(null)}>
                      <X className="text-muted-foreground" />
                      Clear grouping
                    </DropdownMenuItem>
                  </>
                ) : null}
              </DropdownMenuSubContent>
            </DropdownMenuPortal>
          </DropdownMenuSub>
        )}

        {onRefresh && (
          <DropdownMenuItem
            onSelect={() => onRefresh()}
            disabled={isRefreshing}
          >
            {isRefreshing ? (
              <Spinner />
            ) : (
              <RefreshCw className="text-muted-foreground" />
            )}
            Refresh data
          </DropdownMenuItem>
        )}

        {columnControls && resetLayout && (
          <>
            <DropdownMenuSeparator />
            <DropdownMenuItem onSelect={resetLayout}>
              <RotateCcw className="text-muted-foreground" />
              Reset layout
            </DropdownMenuItem>
          </>
        )}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
