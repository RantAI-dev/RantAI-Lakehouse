"use client";

import { Spinner } from "@/components/ui/spinner";
import { cn } from "@/lib/utils";

export interface DataTableInfiniteState {
  onLoadMore: () => void;
  hasNextPage: boolean;
  isFetchingNextPage: boolean;
  /** Left out when the server gives no total: the footer then names no total. */
  totalItems?: number;
  loadedCount: number;
}

interface DataTableInfiniteFooterProps extends React.ComponentProps<"div"> {
  infinite: DataTableInfiniteState;
  selectedCount?: number;
}

/** Footer for infinite tables — row count, plus a spinner while the next chunk loads. */
export function DataTableInfiniteFooter({
  infinite,
  selectedCount = 0,
  className,
  ...props
}: DataTableInfiniteFooterProps) {
  const { loadedCount, totalItems, isFetchingNextPage, hasNextPage } = infinite;
  const showCount = loadedCount > 0 || (totalItems ?? 0) > 0;
  // GOV-1 review T-3: with no total the footer counts what is loaded and
  // says whether more can be loaded; it never invents a total.
  const countText =
    totalItems === undefined
      ? `${loadedCount.toLocaleString()}${hasNextPage ? " so far" : ""}`
      : `${loadedCount.toLocaleString()} of ${totalItems.toLocaleString()}`;

  if (!showCount && !isFetchingNextPage) return null;

  return (
    <div
      className={cn(
        "text-muted-foreground flex items-center justify-center gap-2 p-2 text-sm",
        className
      )}
      {...props}
    >
      {isFetchingNextPage ? <Spinner /> : null}
      {isFetchingNextPage ? (
        <span>Loading more…</span>
      ) : showCount ? (
        <span>
          Showing {countText}
          {!hasNextPage && loadedCount > 0 ? " · End of list" : null}
          {selectedCount > 0 ? ` · ${selectedCount.toLocaleString()} selected` : null}
        </span>
      ) : null}
      {isFetchingNextPage && showCount ? (
        <span className="text-muted-foreground/80">
          · {countText}
          {selectedCount > 0 ? ` · ${selectedCount.toLocaleString()} selected` : null}
        </span>
      ) : null}
    </div>
  );
}
