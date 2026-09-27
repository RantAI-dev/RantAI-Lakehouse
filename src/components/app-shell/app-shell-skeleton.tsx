"use client";

import { Skeleton } from "@/components/ui/skeleton";
import { LoadingSkeleton } from "@/components/patterns/page-states";
import { DashboardPageSkeleton } from "@/features/dashboards/dashboard-skeleton";

/**
 * What the console shows while `AuthProvider` is still checking the session:
 * the same frame as the real one (16rem sidebar, 4rem navbar, padded main)
 * with the page's own shape inside, so the first paint after a refresh does
 * not look like a different app. Only dashboards have a page-shaped
 * placeholder so far; other routes get the generic list skeleton.
 */
export function AppShellSkeleton({ pathname }: { pathname: string }) {
  return (
    <div className="flex min-h-svh w-full" role="status" aria-label="Loading">
      <aside className="hidden w-64 shrink-0 flex-col gap-4 border-r border-sidebar-border bg-sidebar p-3 md:flex">
        <div className="flex items-center gap-2 px-1 py-1">
          <Skeleton className="size-8 rounded-lg" />
          <div className="grid flex-1 gap-1.5">
            <Skeleton className="h-3.5 w-24" />
            <Skeleton className="h-2.5 w-36" />
          </div>
        </div>
        <div className="grid gap-1">
          {[28, 24, 20].map((w, i) => (
            <div key={i} className="flex h-8 items-center gap-2 px-2">
              <Skeleton className="size-4" />
              <Skeleton className="h-3" style={{ width: `${w * 4}px` }} />
            </div>
          ))}
        </div>
        <div className="grid gap-1">
          {[16, 26, 20, 22, 24, 18, 20, 22].map((w, i) => (
            <div key={i} className="flex h-8 items-center gap-2 px-2">
              <Skeleton className="size-4" />
              <Skeleton className="h-3" style={{ width: `${w * 4}px` }} />
            </div>
          ))}
        </div>
        <div className="mt-auto flex items-center gap-2 px-1">
          <Skeleton className="size-9 rounded-full" />
          <div className="grid flex-1 gap-1.5">
            <Skeleton className="h-3 w-28" />
            <Skeleton className="h-2.5 w-36" />
          </div>
        </div>
      </aside>
      <div className="flex min-w-0 flex-1 flex-col bg-muted/25">
        <div className="flex h-16 shrink-0 items-center justify-between gap-4 border-b border-border bg-background px-4 sm:px-5">
          <div className="flex items-center gap-3">
            <Skeleton className="size-9 rounded-lg" />
            <Skeleton className="hidden h-4 w-28 sm:block" />
          </div>
          <div className="flex items-center gap-3">
            <Skeleton className="hidden h-9 w-72 rounded-lg md:block" />
            <Skeleton className="size-8 rounded-full" />
            <Skeleton className="size-8 rounded-full" />
            <Skeleton className="size-8 rounded-full" />
          </div>
        </div>
        <main className="flex-1 min-w-0 p-4 sm:p-5 lg:p-6">
          {pathname.startsWith("/dashboards") ? <DashboardPageSkeleton /> : <LoadingSkeleton rows={8} />}
        </main>
      </div>
    </div>
  );
}
