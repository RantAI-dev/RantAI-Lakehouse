"use client";

import { Skeleton } from "@/components/ui/skeleton";
import { cn } from "@/lib/utils";
import { DashboardHeader } from "./dashboard-header";
import { SOFT_TILE_BODY, SOFT_TILE_FRAME, SOFT_TILE_HEADER } from "./tile-card";

/**
 * What a tile's body shows while its cell is still loading: the body box of
 * the real tile (`TileBody`), and the body of a placeholder tile below, so a
 * tile whose layout is already known and one whose layout is not yet known
 * read as the same thing.
 */
export function TileBodyPlaceholder({ className }: { className?: string }) {
  return <div data-slot="tile-body-placeholder" className={cn("h-full animate-pulse rounded-lg bg-muted/40", className)} />;
}

/**
 * The tiles while the dashboard's layout is not known yet (the session check,
 * then the first read of `/api/dashboard`, which answers the layout and the
 * cells together). Each placeholder is the real soft tile (`TileCard`'s
 * `soft` look: no border, a soft fill, a large radius) with a grey bar where
 * the title sits and the body placeholder in the body; no pill where the
 * real tile keeps its ⋯ (owner QA 2026-11-14). Half width and as tall as a
 * default grid tile (6 rows × 44 px + 5 gaps × 12 px, `dashboard-grid.tsx`).
 * Once the layout is known the real grid draws every tile in its own place
 * and size, each with `TileBodyPlaceholder` until its cell lands.
 */
export function DashboardTilesSkeleton({ tiles = 4 }: { tiles?: number }) {
  return (
    <div className="grid grid-cols-1 gap-3 md:grid-cols-2" role="status" aria-label="Loading charts">
      {Array.from({ length: tiles }).map((_, i) => (
        <div key={i} aria-hidden className={cn(SOFT_TILE_FRAME, "h-[324px]")} data-slot="tile-placeholder">
          <div className={SOFT_TILE_HEADER}>
            <Skeleton className="mt-[11px] h-3.5" style={{ width: TITLE_WIDTHS[i % TITLE_WIDTHS.length] }} />
          </div>
          <div className={SOFT_TILE_BODY}>
            <TileBodyPlaceholder />
          </div>
        </div>
      ))}
    </div>
  );
}

/** Title widths that vary like real titles do. */
const TITLE_WIDTHS = ["10rem", "12.5rem", "8.5rem", "11rem"];

/**
 * The dashboard page while the session is checked (`AppShellSkeleton`) and
 * while `/dashboards` decides which board to open (`DashboardResolver`): the
 * page's own header, drawn by `DashboardHeader` itself with the title and the
 * facts line as grey bars of their exact line height (the board's name is not
 * known yet), grey blocks where the reader's controls go (which of them a
 * person gets depends on permissions not known yet), then the tiles. The page
 * mounts into the same shapes: its header keeps the title placeholder until
 * the name is known (`DashboardHeader`, `title: null`).
 */
export function DashboardPageSkeleton() {
  return (
    <div className="flex flex-col gap-4" role="status" aria-label="Loading dashboard">
      <DashboardHeader
        folderPath={[]}
        title={null}
        facts={{}}
        loading={false}
        onStop={noop}
        actions={
          <>
            <Skeleton aria-hidden className="size-8 rounded-lg" />
            <Skeleton aria-hidden className="size-8 rounded-lg" />
            <Skeleton aria-hidden className="h-8 w-[78px] rounded-lg" />
          </>
        }
      />
      <DashboardTilesSkeleton />
    </div>
  );
}

function noop() {}
