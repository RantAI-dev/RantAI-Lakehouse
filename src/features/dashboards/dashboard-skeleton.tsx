"use client";

import { Skeleton } from "@/components/ui/skeleton";

/**
 * Loading placeholders shaped like the dashboard itself: a title row with
 * the three header actions, the filter bar, and half-width tiles as tall as
 * a default grid tile (6 rows × 44px + 5 gaps × 12px, dashboard-grid.tsx),
 * so the page does not jump when the real tiles arrive.
 */
export function DashboardTilesSkeleton({ tiles = 4 }: { tiles?: number }) {
  return (
    <div className="grid grid-cols-1 gap-3 md:grid-cols-2" role="status" aria-label="Loading charts">
      {Array.from({ length: tiles }).map((_, i) => (
        <div key={i} className="flex h-[324px] flex-col gap-3 rounded-xl border border-border bg-card p-4">
          <div className="flex items-center justify-between gap-3">
            <Skeleton className="h-4 w-40" />
            <Skeleton className="h-5 w-14 rounded-full" />
          </div>
          <Skeleton className="flex-1" />
        </div>
      ))}
    </div>
  );
}

export function DashboardPageSkeleton() {
  return (
    <div className="flex flex-col gap-4" role="status" aria-label="Loading dashboard">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <Skeleton className="h-8 w-56" />
        <div className="flex items-center gap-2">
          <Skeleton className="h-8 w-28" />
          <Skeleton className="size-8" />
          <Skeleton className="h-8 w-28" />
        </div>
      </div>
      <Skeleton className="h-11 w-full rounded-lg" />
      <DashboardTilesSkeleton />
    </div>
  );
}
