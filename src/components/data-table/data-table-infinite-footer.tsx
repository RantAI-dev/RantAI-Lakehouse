"use client";

import { Spinner } from "@/components/ui/spinner";
import { cn } from "@/lib/utils";

export interface DataTableInfiniteState {
  onLoadMore: () => void;
  hasNextPage: boolean;
  isFetchingNextPage: boolean;
  totalItems: number;
  loadedCount: number;
}

interface DataTableInfiniteFooterProps extends React.ComponentProps<"div"> {
  infinite: DataTableInfiniteState;
}

/** Footer for infinite tables — row count, plus a spinner while the next chunk loads. */
export function DataTableInfiniteFooter({
  infinite,
  className,
  ...props
}: DataTableInfiniteFooterProps) {
  const { loadedCount, totalItems, isFetchingNextPage, hasNextPage } = infinite;
  const showCount = loadedCount > 0 || totalItems > 0;

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
          Showing {loadedCount.toLocaleString()} of {totalItems.toLocaleString()}
          {!hasNextPage && loadedCount > 0 ? " · End of list" : null}
        </span>
      ) : null}
      {isFetchingNextPage && showCount ? (
        <span className="text-muted-foreground/80">
          · {loadedCount.toLocaleString()} of {totalItems.toLocaleString()}
        </span>
      ) : null}
    </div>
  );
}
