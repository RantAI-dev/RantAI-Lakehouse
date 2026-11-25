"use client";

import { LoadingSkeleton } from "@/components/patterns/page-states";
import { DashboardPageSkeleton } from "@/features/dashboards/dashboard-skeleton";
import { DashboardListPageSkeleton } from "@/features/dashboards/dashboard-list-skeleton";
import { shellPlaceholderFor } from "./shell-placeholder";

/**
 * The page-shaped placeholder for a route's main area. One source for the auth
 * check (`AppShellSkeleton`) and the route fallback (`app/loading.tsx`), so a
 * refresh, a navigation and the page's own loading all draw the same shapes
 * (owner QA 2026-11-15). It draws no padding: the caller's main area supplies it.
 */
export function ShellMainPlaceholder({ pathname }: { pathname: string }) {
  const placeholder = shellPlaceholderFor(pathname);
  if (placeholder === "dashboard-list") return <DashboardListPageSkeleton />;
  if (placeholder === "dashboard") return <DashboardPageSkeleton />;
  return <LoadingSkeleton rows={8} />;
}
