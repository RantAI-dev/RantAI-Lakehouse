"use client";

import * as React from "react";
import Link from "next/link";
import { useTheme } from "next-themes";

import { Skeleton } from "@/components/ui/skeleton";
import { useService } from "@/hooks/use-service";
import type { ChartRenderSpec } from "@/lib/dashboard-specs";
import type { LayoutMap } from "@/services/clients/bi-store";
import { apiFetch } from "@/services/http";
import { TileBody } from "./tile-body";

type Cell = { columns: string[]; rows: Record<string, unknown>[] } | { error: string };
type Payload = {
  charts: (ChartRenderSpec & { text?: string; caption?: string })[];
  results: Record<string, Cell>;
  layout?: LayoutMap;
};

/**
 * The first few tiles of one dashboard, read-only, for Home's "continue
 * working": the board you last had open, shown with its own data rather
 * than as a link.
 *
 * It asks the same `/api/dashboard?board=` the canvas does (through
 * `apiFetch`, like `dashboard-page.tsx`: there is no service method for a
 * board's payload), so the tiles are the governed result: masking, row
 * filters and the board's saved filters all apply exactly as on the
 * canvas. Nothing is cached or recomputed here.
 *
 * Tiles are taken in canvas order (top to bottom, left to right). Text
 * notes are skipped: a note without its neighbours says little. No drill
 * or edit: the title links to the canvas for that.
 */
export function DashboardPreview({
  boardId,
  name,
  limit = 2,
}: {
  readonly boardId: string;
  readonly name: string;
  readonly limit?: number;
}) {
  const { resolvedTheme } = useTheme();
  const state = useService(async (signal) => {
    const res = await apiFetch(`/api/dashboard?${new URLSearchParams({ board: boardId })}`, { cache: "no-store", signal });
    const json = (await res.json()) as Payload & { error?: string };
    // The upstream text is already classified by the route; only a fixed
    // message is shown here either way.
    if (!res.ok) throw new Error("Failed to load dashboard");
    return json;
  }, [boardId]);

  const href = `/dashboards/${encodeURIComponent(boardId)}`;
  const tiles = React.useMemo(() => {
    const layout = state.data?.layout ?? {};
    const order = (id: string) => {
      const box = layout[id];
      return box ? box.y * 100 + box.x : Number.MAX_SAFE_INTEGER;
    };
    return [...(state.data?.charts ?? [])]
      .filter((c) => c.kind !== "text")
      .sort((a, b) => order(a.id) - order(b.id))
      .slice(0, limit);
  }, [state.data, limit]);

  if (state.status === "success" && tiles.length === 0) return null;

  return (
    <section className="flex flex-col gap-3">
      <div className="flex items-baseline justify-between gap-3 px-1">
        <h2 className="min-w-0 truncate text-xs font-semibold tracking-[0.08em] text-muted-foreground uppercase">
          From {name}
        </h2>
        <Link href={href} className="shrink-0 text-xs text-muted-foreground underline-offset-4 hover:text-foreground hover:underline">
          Open dashboard
        </Link>
      </div>
      {state.status === "error" ? (
        <p className="rounded-xl border border-border bg-card px-4 py-3 text-xs text-muted-foreground">
          Couldn&apos;t load this dashboard&apos;s charts.
        </p>
      ) : (
        <div className="grid gap-3 sm:grid-cols-2">
          {state.status === "loading"
            ? Array.from({ length: limit }).map((_, i) => (
                <div key={i} className="flex h-72 flex-col gap-3 rounded-xl border border-border bg-card p-4">
                  <Skeleton className="h-4 w-40" />
                  <Skeleton className="flex-1" />
                </div>
              ))
            : tiles.map((spec) => (
                <div key={spec.id} className="flex h-72 min-w-0 flex-col gap-2 rounded-xl border border-border bg-card p-4">
                  <Link href={href} className="truncate text-sm font-semibold underline-offset-4 hover:underline">
                    {spec.title}
                  </Link>
                  <div className="min-h-0 flex-1">
                    <TileBody spec={spec} cell={state.data?.results[spec.id]} dark={resolvedTheme === "dark"} loading={false} year="all" />
                  </div>
                </div>
              ))}
        </div>
      )}
    </section>
  );
}
