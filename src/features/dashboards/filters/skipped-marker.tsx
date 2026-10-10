"use client";

import { FilterX } from "lucide-react";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import type { FilterSkip } from "@/services/clients/bi-store";

const WHY: Record<string, string> = {
  no_column: "this tile's data has no such column",
  wrong_type: "the column's type does not fit the filter",
};

/**
 * Quiet marker on a tile for active filters its data could not honour. The
 * tile is drawn unfiltered by them, so without this the page would claim a
 * filter that is not in effect.
 */
export function SkippedFiltersMarker({ skipped }: { skipped: FilterSkip[] }) {
  if (!skipped.length) return null;
  const names = skipped.map((s) => s.column).join(", ");
  return (
    <Tooltip>
      <TooltipTrigger render={<span className="inline-flex shrink-0 text-muted-foreground/70" />}>
        <FilterX className="size-3.5" aria-label={`Filters not applied to this tile: ${names}`} />
      </TooltipTrigger>
      <TooltipContent>
        <p className="font-medium">Not applied to this tile</p>
        {skipped.map((s) => (
          <p key={s.column}>{s.column}: {WHY[s.reason] ?? "does not apply"}</p>
        ))}
      </TooltipContent>
    </Tooltip>
  );
}
