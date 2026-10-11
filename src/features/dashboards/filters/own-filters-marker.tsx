"use client";

import { Filter } from "lucide-react";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { filterLabel } from "@/lib/dashboard-filter-state";
import type { FilterDef } from "@/services/clients/bi-store";

/**
 * Quiet mark on a tile for the filters the chart carries itself (`BI-6`): they
 * are part of what the numbers mean, apply whatever the dashboard's filters
 * are, and cannot be removed from the dashboard (only by editing the chart).
 * Listed in a tooltip, one line each, read like a filter chip.
 */
export function OwnFiltersMarker({ filters }: { readonly filters: readonly FilterDef[] }) {
  if (!filters.length) return null;
  const labels = filters.map((f) => filterLabel(f));
  return (
    <Tooltip>
      <TooltipTrigger render={<span className="inline-flex shrink-0 text-muted-foreground/70" />}>
        <Filter className="size-3.5" aria-label={`This chart is filtered: ${labels.join("; ")}`} />
      </TooltipTrigger>
      <TooltipContent>
        <p className="font-medium">This chart is filtered</p>
        {labels.map((label) => <p key={label}>{label}</p>)}
        <p className="text-muted-foreground">Dashboard filters add to these.</p>
      </TooltipContent>
    </Tooltip>
  );
}

/** The chart's own filters from a tile's definition, if it has any. */
export function ownFilters(spec: { def?: unknown }): FilterDef[] | undefined {
  const filters = (spec.def as { filters?: FilterDef[] } | undefined)?.filters;
  return filters?.length ? filters : undefined;
}
