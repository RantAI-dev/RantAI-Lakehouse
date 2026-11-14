"use client";

import { isStaticKind } from "@/lib/dashboard-tabs";
import * as React from "react";
import Link from "next/link";
import { useTheme } from "next-themes";

import { Skeleton } from "@/components/ui/skeleton";
import { useService } from "@/hooks/use-service";
import type { ChartRenderSpec } from "@/lib/dashboard-specs";
import { cn } from "@/lib/utils";
import type { LayoutMap } from "@/services";
import { apiFetch } from "@/services/http";
import { TileBodyPlaceholder } from "./dashboard-skeleton";
import { TileBody } from "./tile-body";
import type { TileFailure } from "@/services/contracts/dashboards";

type Cell = { columns: string[]; rows: Record<string, unknown>[] } | TileFailure;
type Payload = {
  charts: (ChartRenderSpec & { text?: string; caption?: string })[];
  results: Record<string, Cell>;
  layout?: LayoutMap;
};

/**
 * The first few tiles of one dashboard, read-only, inside Home's focal
 * widget (`WEB-17`): the board drawn with its own data rather than as a
 * link. The widget owns the card and the title row; this is the tiles.
 *
 * It asks the same `/api/dashboard?board=` the canvas does (through
 * `apiFetch`, like `dashboard-page.tsx`: there is no service method for a
 * board's payload), so the tiles are the governed result: masking, row
 * filters and the board's saved filters all apply exactly as on the
 * canvas. Nothing is cached or recomputed here.
 *
 * Tiles are taken in canvas order (top to bottom, left to right), charts
 * first: text notes are never shown (a note without its neighbours says
 * little), and tables only when the board has no chart to show, since a
 * table of a row or two is mostly empty space at this size. Legends and the
 * "Data from" line are hidden: at this size they crowd out the chart. No
 * drill or edit: each title links to the canvas for that.
 *
 * Layout: two tiles a row once the widget is 480 px wide, one below that,
 * where only the first two are drawn so a phone is not a long scroll of
 * charts. Rows share the height the widget has (at least 150 px each), and
 * no tile has a box of its own: the widget is the one surface (DESIGN.md §3,
 * no box inside a box). While loading, the same grid holds grey bars in the
 * tiles' exact places, so the page does not change shape when data lands.
 */
export function DashboardPreview({
  boardId,
  limit = 4,
  className,
}: {
  readonly boardId: string;
  readonly limit?: number;
  readonly className?: string;
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
    const candidates = [...(state.data?.charts ?? [])]
      .filter((c) => !isStaticKind(c.kind))
      .sort((a, b) => order(a.id) - order(b.id));
    const visual = candidates.filter((c) => c.kind !== "table");
    return (visual.length > 0 ? visual : candidates).slice(0, limit);
  }, [state.data, limit]);

  if (state.status === "error") {
    return <p className={cn("py-1 text-xs leading-5 text-muted-foreground", className)}>Couldn&apos;t load this dashboard&apos;s charts.</p>;
  }
  if (state.status === "success" && tiles.length === 0) {
    return <p className={cn("py-1 text-xs leading-5 text-muted-foreground", className)}>This dashboard has no chart to preview yet.</p>;
  }

  const grid = cn(
    "grid min-h-0 auto-rows-[minmax(150px,1fr)] grid-cols-1 gap-x-6 gap-y-4 @[480px]/focal:grid-cols-2",
    className,
  );
  /** The third tile on is for the two-column width only. */
  const extra = (i: number) => (i >= 2 ? "hidden @[480px]/focal:flex" : "flex");

  if (state.status === "loading") {
    return (
      <div className={grid} role="status" aria-label="Loading charts">
        {Array.from({ length: limit }, (_, i) => (
          <div key={i} aria-hidden className={cn(extra(i), "min-w-0 flex-col gap-2")}>
            <div className="flex h-5 items-center">
              <Skeleton className="h-3" style={{ width: TITLE_WIDTHS[i % TITLE_WIDTHS.length] }} />
            </div>
            <TileBodyPlaceholder className="min-h-0 flex-1" />
          </div>
        ))}
      </div>
    );
  }

  return (
    <div className={grid}>
      {tiles.map((spec, i) => (
        <div key={spec.id} data-slot="preview-tile" className={cn(extra(i), "min-w-0 flex-col gap-2")}>
          <Link
            href={href}
            className="h-5 truncate text-xs leading-5 font-medium text-muted-foreground underline-offset-4 hover:text-foreground hover:underline"
          >
            {spec.title}
          </Link>
          <div className="min-h-0 flex-1">
            <TileBody spec={spec} cell={state.data?.results[spec.id]} dark={resolvedTheme === "dark"} loading={false} hideLegend hideDataFrom />
          </div>
        </div>
      ))}
    </div>
  );
}

/** Title widths that vary like real titles do. */
const TITLE_WIDTHS = ["9rem", "11rem", "7.5rem", "10rem"];
