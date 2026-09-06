"use client";

import {
  closestCenter,
  DndContext,
  type DragEndEvent,
  KeyboardSensor,
  PointerSensor,
  useSensor,
  useSensors,
} from "@dnd-kit/core";
import { restrictToHorizontalAxis } from "@dnd-kit/modifiers";
import {
  arrayMove,
  horizontalListSortingStrategy,
  SortableContext,
  sortableKeyboardCoordinates,
  useSortable,
} from "@dnd-kit/sortable";
import { CSS, type Transform } from "@dnd-kit/utilities";
import {
  type Cell,
  flexRender,
  type Header,
  type Row,
  type Table as TanstackTable,
} from "@tanstack/react-table";
import { ChevronDown } from "lucide-react";
import * as React from "react";
import { useWindowVirtualizer } from "@tanstack/react-virtual";

import {
  DataTableInfiniteFooter,
  type DataTableInfiniteState,
} from "@/components/data-table/data-table-infinite-footer";
import { DataTablePagination } from "@/components/data-table/data-table-pagination";
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuTrigger,
} from "@/components/ui/context-menu";
import {
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import {
  buildVirtualTableRows,
  estimateVirtualRowHeight,
  type VirtualTableRow,
} from "@/lib/data-table-virtual-rows";
import { getColumnPinningStyle } from "@/lib/data-table";
import type { GroupSummary } from "@/services/contracts/pagination";
import { cn } from "@/lib/utils";

/** Clears below the sticky dashboard header (`h-14` = 3.5rem). */
const STICKY_HEADER_TOP_PX = 56;

const stickyHeadClass = "bg-background";

function getDocumentTop(element: HTMLElement) {
  return element.getBoundingClientRect().top + window.scrollY;
}

function VirtualSpacerRow({
  colSpan,
  height,
}: {
  colSpan: number;
  height: number;
}) {
  return (
    <TableRow aria-hidden="true" className="border-0 hover:bg-transparent">
      <TableCell colSpan={colSpan} className="p-0 border-0">
        <div style={{ height }} aria-hidden="true" />
      </TableCell>
    </TableRow>
  );
}

function renderVirtualTableRow<TData>({
  item,
  visibleColumnCount,
  collapsedGroups,
  pinnedOffsets,
  sortableColumnIds,
  onRowClick,
  renderRowContextMenu,
  onToggleGroup,
  virtualIndex,
  measureElement,
}: {
  item: VirtualTableRow<TData>;
  visibleColumnCount: number;
  collapsedGroups: Set<string>;
  pinnedOffsets: Record<string, number>;
  sortableColumnIds: string[];
  onRowClick?: (row: TData) => void;
  renderRowContextMenu?: (row: TData) => React.ReactNode;
  onToggleGroup: (groupKey: string) => void;
  virtualIndex?: number;
  measureElement?: (node: Element | null) => void;
}) {
  const measureProps =
    virtualIndex !== undefined && measureElement
      ? ({
          "data-index": virtualIndex,
          ref: measureElement,
        } as const)
      : {};

  if (item.kind === "group-header") {
    return (
      <TableRow {...measureProps} className="bg-muted/40 hover:bg-muted/40">
        <TableCell colSpan={visibleColumnCount} className="py-2">
          <button
            type="button"
            className="flex w-full items-center gap-2 text-left font-medium"
            onClick={() => onToggleGroup(item.groupKey)}
          >
            <ChevronDown
              className={cn(
                "text-muted-foreground size-4 shrink-0 transition-transform",
                collapsedGroups.has(item.groupKey) && "-rotate-90"
              )}
            />
            <span className="truncate">{item.label}</span>
            {item.count !== undefined ? (
              <span className="text-muted-foreground text-xs font-normal">
                {item.count}
              </span>
            ) : null}
          </button>
        </TableCell>
      </TableRow>
    );
  }

  return (
    <DataTableDataRow
      row={item.row}
      pinnedOffsets={pinnedOffsets}
      sortableColumnIds={sortableColumnIds}
      onRowClick={onRowClick}
      renderRowContextMenu={renderRowContextMenu}
      measureProps={measureProps}
    />
  );
}

interface DataTableProps<TData> extends React.ComponentProps<"div"> {
  table: TanstackTable<TData>;
  actionBar?: React.ReactNode;
  /** Group counts from the server when `meta.groupBy` is active. */
  groupSummaries?: GroupSummary[] | null;
  /** Infinite scroll state — omit for page-button pagination. */
  infinite?: DataTableInfiniteState;
  /**
   * Makes each row open its record. Interactive cells (action menus, buttons)
   * have to stop propagation themselves, or they fire this too.
   */
  onRowClick?: (row: TData) => void;
  /** Right-click menu content for a data row. Omit to disable. */
  renderRowContextMenu?: (row: TData) => React.ReactNode;
}

/**
 * A pinned cell floats above the columns scrolling under it, so it can never be
 * even slightly transparent. The row's own hover and open-menu tints are
 * `bg-muted/50`, which would do exactly that, so they are pre-mixed against the
 * page background here — same resulting colour, no see-through.
 */
const pinnedCellClass = cn(
  "bg-background",
  // Spelled out rather than built from a shared constant: Tailwind only sees
  // class names that appear literally in the source.
  "group-hover/row:bg-[color-mix(in_srgb,var(--muted)_50%,var(--background))]",
  "group-has-aria-expanded/row:bg-[color-mix(in_srgb,var(--muted)_50%,var(--background))]",
  "group-data-[state=selected]/row:bg-muted"
);

/** Geometry the dragged column carries while it is in flight. */
function getDragStyle(
  transform: Transform | null,
  transition: string | undefined,
  isDragging: boolean
): React.CSSProperties {
  // Never leave a transform on idle headers — any transform creates a
  // containing block and kills `position: sticky` on the same element.
  if (!isDragging) {
    return { transition };
  }

  return {
    transform: CSS.Translate.toString(transform),
    transition,
    opacity: 0.8,
    zIndex: 40,
  };
}

function DraggableTableHead<TData>({
  header,
}: {
  header: Header<TData, unknown>;
}) {
  const { attributes, isDragging, listeners, setNodeRef, transform, transition } =
    useSortable({
      id: header.column.id,
      // Keep the cell announcing itself as a column header — dnd-kit would
      // otherwise relabel it a button and lose the table semantics.
      attributes: { role: "columnheader" },
    });

  const onKeyDown = (event: React.KeyboardEvent<HTMLTableCellElement>) => {
    // Enter and Space start a keyboard drag, and they are also how the sort
    // menu inside the header opens. Only the header itself may start the drag.
    if (event.target !== event.currentTarget) return;
    listeners?.onKeyDown?.(event);
  };

  return (
    <TableHead
      ref={setNodeRef}
      colSpan={header.colSpan}
      // `touch-none` is what lets a touch drag begin at all; the cost is that
      // the table can only be scrolled sideways by touching its body.
      className={cn(
        stickyHeadClass,
        "sticky border-b touch-none select-none",
        isDragging ? "cursor-grabbing" : "cursor-grab"
      )}
      style={{
        ...getColumnPinningStyle({ column: header.column }),
        ...getDragStyle(transform, transition, isDragging),
        // Pinning helper uses `relative` when unpinned — force sticky for headers.
        position: "sticky",
        top: STICKY_HEADER_TOP_PX,
        zIndex: isDragging ? 40 : 20,
      }}
      {...attributes}
      {...listeners}
      onKeyDown={onKeyDown}
    >
      {header.isPlaceholder
        ? null
        : flexRender(header.column.columnDef.header, header.getContext())}
    </TableHead>
  );
}

function DraggableTableCell<TData>({ cell }: { cell: Cell<TData, unknown> }) {
  const { isDragging, setNodeRef, transform, transition } = useSortable({
    id: cell.column.id,
  });

  return (
    <TableCell
      ref={setNodeRef}
      style={{
        ...getColumnPinningStyle({ column: cell.column }),
        ...getDragStyle(transform, transition, isDragging),
      }}
    >
      {flexRender(cell.column.columnDef.cell, cell.getContext())}
    </TableCell>
  );
}

function DataTableRowCells<TData>({
  row,
  pinnedOffsets,
  sortableColumnIds,
}: {
  row: Row<TData>;
  pinnedOffsets: Record<string, number>;
  sortableColumnIds: string[];
}) {
  return (
    <SortableContext
      items={sortableColumnIds}
      strategy={horizontalListSortingStrategy}
    >
      {row.getVisibleCells().map((cell) =>
        cell.column.getIsPinned() ? (
          <TableCell
            key={cell.id}
            className={pinnedCellClass}
            style={{
              ...getColumnPinningStyle({
                column: cell.column,
                offset: pinnedOffsets[cell.column.id],
                withBorder: true,
              }),
            }}
          >
            {flexRender(cell.column.columnDef.cell, cell.getContext())}
          </TableCell>
        ) : (
          <DraggableTableCell key={cell.id} cell={cell} />
        )
      )}
    </SortableContext>
  );
}

function DataTableDataRow<TData>({
  row,
  pinnedOffsets,
  sortableColumnIds,
  onRowClick,
  renderRowContextMenu,
  measureProps,
}: {
  row: Row<TData>;
  pinnedOffsets: Record<string, number>;
  sortableColumnIds: string[];
  onRowClick?: (row: TData) => void;
  renderRowContextMenu?: (row: TData) => React.ReactNode;
  measureProps?: {
    "data-index"?: number;
    ref?: (node: Element | null) => void;
  };
}) {
  const rowNode = (
    <TableRow
      {...measureProps}
      data-state={
        renderRowContextMenu
          ? undefined
          : row.getIsSelected()
            ? "selected"
            : undefined
      }
      className={cn(
        "group/row",
        onRowClick && "cursor-pointer",
        renderRowContextMenu && row.getIsSelected() && "bg-muted",
        renderRowContextMenu && "data-[state=open]:bg-muted/50"
      )}
      onClick={onRowClick ? () => onRowClick(row.original) : undefined}
    >
      <DataTableRowCells
        row={row}
        pinnedOffsets={pinnedOffsets}
        sortableColumnIds={sortableColumnIds}
      />
    </TableRow>
  );

  if (!renderRowContextMenu) return rowNode;

  return (
    <ContextMenu>
      <ContextMenuTrigger asChild className="select-text">
        {rowNode}
      </ContextMenuTrigger>
      <ContextMenuContent>
        {renderRowContextMenu(row.original)}
      </ContextMenuContent>
    </ContextMenu>
  );
}

export function DataTable<TData>({
  table,
  actionBar,
  groupSummaries,
  infinite,
  children,
  className,
  onRowClick,
  renderRowContextMenu,
  ...props
}: DataTableProps<TData>) {
  const sensors = useSensors(
    // Anything shorter than the threshold stays a click, so the sort/hide
    // dropdown inside a header still opens on tap.
    useSensor(PointerSensor, { activationConstraint: { distance: 8 } }),
    useSensor(KeyboardSensor, {
      coordinateGetter: sortableKeyboardCoordinates,
    })
  );

  // Only the columns that scroll can be reordered. A pinned column is placed by
  // `left`/`right` offsets that a drag transform would fight with, and it sits
  // outside the run of columns being sorted anyway.
  const centerColumns = table.getCenterVisibleLeafColumns();
  const sortableColumnIds = React.useMemo(
    () => centerColumns.map((column) => column.id),
    [centerColumns]
  );

  const leftPinned = table.getLeftVisibleLeafColumns();
  const rightPinned = table.getRightVisibleLeafColumns();
  const headerRef = React.useRef<HTMLTableSectionElement>(null);
  const bodyRef = React.useRef<HTMLTableSectionElement>(null);
  const tableContainerRef = React.useRef<HTMLDivElement>(null);
  const loadMoreRef = React.useRef<HTMLDivElement>(null);
  const [scrollMargin, setScrollMargin] = React.useState<number | null>(null);
  const [pinnedWidths, setPinnedWidths] = React.useState<
    Record<string, number>
  >({});

  // Re-measure whenever the set of pinned columns changes. Their ids are the
  // dependency, not the arrays — TanStack hands back a new array whenever any
  // part of the table state moves.
  const pinnedIds = [...leftPinned, ...rightPinned].map((c) => c.id).join(",");

  React.useLayoutEffect(() => {
    const header = headerRef.current;
    if (!header) return;

    const cells = Array.from(
      header.querySelectorAll<HTMLTableCellElement>("th[data-pinned-column]")
    );
    if (cells.length === 0) return;

    const measure = () => {
      setPinnedWidths((previous) => {
        const next: Record<string, number> = {};
        let changed = Object.keys(previous).length !== cells.length;
        for (const cell of cells) {
          const id = cell.dataset.pinnedColumn as string;
          // offsetWidth over getBoundingClientRect: the rect picks up the drag
          // transform, which would feed a moving number back into the layout.
          next[id] = cell.offsetWidth;
          if (previous[id] !== next[id]) changed = true;
        }
        return changed ? next : previous;
      });
    };

    measure();
    const observer = new ResizeObserver(measure);
    for (const cell of cells) observer.observe(cell);
    return () => observer.disconnect();
  }, [pinnedIds]);

  const pinnedOffsets = React.useMemo(() => {
    const offsets: Record<string, number> = {};

    let fromLeft = 0;
    for (const column of leftPinned) {
      offsets[column.id] = fromLeft;
      fromLeft += pinnedWidths[column.id] ?? column.getSize();
    }

    let fromRight = 0;
    for (let i = rightPinned.length - 1; i >= 0; i--) {
      const column = rightPinned[i];
      offsets[column.id] = fromRight;
      fromRight += pinnedWidths[column.id] ?? column.getSize();
    }

    return offsets;
  }, [leftPinned, rightPinned, pinnedWidths]);

  const onDragEnd = React.useCallback(
    ({ active, over }: DragEndEvent) => {
      if (!over || active.id === over.id) return;

      // The order state can be empty (meaning "as defined"), so take the live
      // order off the table instead — it already includes hidden columns, which
      // have to keep their place for when they come back.
      const order = table.getAllLeafColumns().map((column) => column.id);
      const from = order.indexOf(String(active.id));
      const to = order.indexOf(String(over.id));
      if (from === -1 || to === -1) return;

      table.setColumnOrder(arrayMove(order, from, to));
    },
    [table]
  );

  const groupBy = table.options.meta?.groupBy ?? null;
  const [collapsedByGroup, setCollapsedByGroup] = React.useState<
    Record<string, Set<string>>
  >({});

  const collapsedGroups = React.useMemo(
    () =>
      groupBy
        ? (collapsedByGroup[groupBy] ?? new Set<string>())
        : new Set<string>(),
    [collapsedByGroup, groupBy]
  );

  const toggleGroup = React.useCallback(
    (groupKey: string) => {
      if (!groupBy) return;
      setCollapsedByGroup((previous) => {
        const current = new Set<string>(previous[groupBy] ?? []);
        if (current.has(groupKey)) current.delete(groupKey);
        else current.add(groupKey);
        return { ...previous, [groupBy]: current };
      });
    },
    [groupBy]
  );

  const visibleColumnCount = table.getVisibleLeafColumns().length;
  const tableRows = table.getRowModel().rows;

  const virtualItems = React.useMemo(() => {
    if (!infinite) return [];
    return buildVirtualTableRows({
      tableRows,
      groupBy,
      groupSummaries,
      collapsedGroups,
    });
  }, [
    collapsedGroups,
    groupBy,
    groupSummaries,
    infinite,
    tableRows,
  ]);

  // `setScrollMargin` is listed even though `useState` setters are stable:
  // the React Compiler infers it as a dependency and refuses to optimize
  // the component when the manual list disagrees. Naming it costs nothing
  // (the identity never changes, so the callback is still created once)
  // and keeps this file on the compiler's fast path. Same below.
  const updateScrollMargin = React.useCallback(() => {
    const table = bodyRef.current?.closest("table");
    const thead = headerRef.current;
    if (!table || !thead) return;
    setScrollMargin(getDocumentTop(table) + thead.offsetHeight);
  }, [setScrollMargin]);

  const assignBodyRef = React.useCallback(
    (node: HTMLTableSectionElement | null) => {
      bodyRef.current = node;
      if (!node || !infinite) return;

      const table = node.closest("table");
      const thead = headerRef.current ?? table?.querySelector("thead");
      if (!table || !thead) return;

      setScrollMargin(getDocumentTop(table) + thead.offsetHeight);
    },
    [infinite, setScrollMargin]
  );

  React.useLayoutEffect(() => {
    if (!infinite) {
      setScrollMargin(null);
      return;
    }

    updateScrollMargin();

    const container = tableContainerRef.current;
    if (!container) return;

    const observer = new ResizeObserver(updateScrollMargin);
    observer.observe(container);
    window.addEventListener("resize", updateScrollMargin);
    window.addEventListener("scroll", updateScrollMargin, { passive: true });

    return () => {
      observer.disconnect();
      window.removeEventListener("resize", updateScrollMargin);
      window.removeEventListener("scroll", updateScrollMargin);
    };
  }, [infinite, updateScrollMargin, virtualItems.length]);

  const scrollMarginReady = scrollMargin !== null;
  const resolvedScrollMargin = scrollMargin ?? 0;

  const rowVirtualizer = useWindowVirtualizer({
    count: infinite && scrollMarginReady ? virtualItems.length : 0,
    estimateSize: (index) =>
      estimateVirtualRowHeight(virtualItems[index]),
    scrollMargin: resolvedScrollMargin,
    overscan: 12,
    getItemKey: (index) => virtualItems[index]?.id ?? index,
  });

  const virtualRows =
    infinite && scrollMarginReady ? rowVirtualizer.getVirtualItems() : [];
  const virtualPaddingTop =
    infinite && scrollMarginReady && virtualRows.length > 0
      ? Math.max(
          0,
          virtualRows[0].start - rowVirtualizer.options.scrollMargin
        )
      : 0;
  const virtualPaddingBottom =
    infinite && scrollMarginReady && virtualRows.length > 0
      ? Math.max(
          0,
          rowVirtualizer.getTotalSize() -
            virtualRows[virtualRows.length - 1].end
        )
      : 0;

  React.useEffect(() => {
    if (!infinite?.hasNextPage || infinite.isFetchingNextPage) return;

    const lastItem = virtualRows[virtualRows.length - 1];
    if (lastItem && lastItem.index >= Math.max(0, virtualItems.length - 8)) {
      infinite.onLoadMore();
      return;
    }

    const target = loadMoreRef.current;
    if (!target) return;

    const observer = new IntersectionObserver(
      (entries) => {
        if (entries.some((entry) => entry.isIntersecting)) {
          infinite.onLoadMore();
        }
      },
      { root: null, rootMargin: "600px" }
    );

    observer.observe(target);
    return () => observer.disconnect();
  }, [infinite, virtualItems.length, virtualRows]);

  const virtualBodyRows =
    infinite &&
    scrollMarginReady &&
    (virtualItems.length > 0 || infinite.hasNextPage) ? (
      <>
        {virtualPaddingTop > 0 ? (
          <VirtualSpacerRow
            colSpan={visibleColumnCount}
            height={virtualPaddingTop}
          />
        ) : null}
        {virtualRows.map((virtualRow) => {
          const item = virtualItems[virtualRow.index];
          if (!item) return null;

          return (
            <React.Fragment key={item.id}>
              {renderVirtualTableRow({
                item,
                visibleColumnCount,
                collapsedGroups,
                pinnedOffsets,
                sortableColumnIds,
                onRowClick,
                renderRowContextMenu,
                onToggleGroup: toggleGroup,
                virtualIndex: virtualRow.index,
                measureElement: rowVirtualizer.measureElement,
              })}
            </React.Fragment>
          );
        })}
        {virtualPaddingBottom > 0 ? (
          <VirtualSpacerRow
            colSpan={visibleColumnCount}
            height={virtualPaddingBottom}
          />
        ) : null}
      </>
    ) : null;

  const bodyRows = React.useMemo(() => {
    if (infinite) return null;
    if (!tableRows.length) return null;

    const items = buildVirtualTableRows({
      tableRows,
      groupBy,
      groupSummaries,
      collapsedGroups,
    });

    return items.map((item) => (
      <React.Fragment key={item.id}>
        {renderVirtualTableRow({
          item,
          visibleColumnCount,
          collapsedGroups,
          pinnedOffsets,
          sortableColumnIds,
          onRowClick,
          renderRowContextMenu,
          onToggleGroup: toggleGroup,
        })}
      </React.Fragment>
    ));
  }, [
    collapsedGroups,
    groupBy,
    groupSummaries,
    infinite,
    onRowClick,
    pinnedOffsets,
    renderRowContextMenu,
    sortableColumnIds,
    tableRows,
    toggleGroup,
    visibleColumnCount,
  ]);

  const tableBodyContent =
    virtualBodyRows ??
    (infinite && !scrollMarginReady ? (
      <TableRow aria-hidden="true" className="border-0 hover:bg-transparent">
        <TableCell
          colSpan={visibleColumnCount}
          className="h-0 p-0 border-0"
        />
      </TableRow>
    ) : null) ??
    bodyRows ?? (
      <TableRow>
        <TableCell
          colSpan={table.getAllColumns().length}
          className="h-24 text-center"
        >
          No results.
        </TableCell>
      </TableRow>
    );

  return (
    <div
      className={cn("flex w-full min-w-0 flex-col gap-2", className)}
      {...props}
    >
      {children}
      <DndContext
        sensors={sensors}
        collisionDetection={closestCenter}
        modifiers={[restrictToHorizontalAxis]}
        onDragEnd={onDragEnd}
      >
        <div ref={tableContainerRef} className="min-w-0">
          {/*
            No overflow-x wrapper here — any overflow ancestor traps sticky
            headers so they scroll away with the page. Horizontal overflow is
            rare on these tables; pinned columns still use sticky left/right
            against the viewport.
          */}
          <table
            data-slot="table"
            className="w-full caption-bottom border-separate border-spacing-0 text-sm"
          >
            <TableHeader ref={headerRef} className="[&_tr]:border-b-0">
              {table.getHeaderGroups().map((headerGroup) => (
                <TableRow
                  key={headerGroup.id}
                  className="group/row hover:bg-transparent"
                >
                  <SortableContext
                    items={sortableColumnIds}
                    strategy={horizontalListSortingStrategy}
                  >
                    {headerGroup.headers.map((header) =>
                      header.column.getIsPinned() ? (
                        <TableHead
                          key={header.id}
                          colSpan={header.colSpan}
                          data-pinned-column={header.column.id}
                          className={cn(
                            // Opaque cover for horizontal scroll only — do not
                            // reuse body `pinnedCellClass` hover/selection mixes;
                            // those tint the header while siblings stay plain.
                            stickyHeadClass,
                            "sticky border-b"
                          )}
                          style={{
                            ...getColumnPinningStyle({
                              column: header.column,
                              offset: pinnedOffsets[header.column.id],
                              withBorder: true,
                            }),
                            top: STICKY_HEADER_TOP_PX,
                            zIndex: 30,
                          }}
                        >
                          {header.isPlaceholder
                            ? null
                            : flexRender(
                                header.column.columnDef.header,
                                header.getContext()
                              )}
                        </TableHead>
                      ) : (
                        <DraggableTableHead
                          key={header.id}
                          header={header}
                        />
                      )
                    )}
                  </SortableContext>
                </TableRow>
              ))}
            </TableHeader>
            <TableBody
              ref={assignBodyRef}
              className="[&_td]:border-b [&_tr:last-child_td]:border-b-0"
            >
              {tableBodyContent}
            </TableBody>
          </table>
        </div>
      </DndContext>
      <div className="flex flex-col gap-2.5">
        {infinite ? (
          <>
            {(infinite.hasNextPage || infinite.isFetchingNextPage) && (
              <div
                ref={loadMoreRef}
                className="h-1 w-full"
                aria-hidden="true"
              />
            )}
            <DataTableInfiniteFooter infinite={infinite} />
          </>
        ) : (
          <DataTablePagination table={table} />
        )}
        {actionBar &&
          table.getFilteredSelectedRowModel().rows.length > 0 &&
          actionBar}
      </div>
    </div>
  );
}
