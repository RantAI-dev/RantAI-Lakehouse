"use client";

import { CalendarOff, Scissors } from "lucide-react";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { GRAIN_LABELS, isGrain } from "@/lib/time-grain";

/**
 * Quiet marks on a tile for a grouped chart (BI-9): the dashboard's grain
 * did not fit this chart's column, so it kept its own (like the skipped-filter
 * mark), the chart has more buckets than its limit, so only the latest
 * are shown, and — while the dashboard's control is in use — the grain does
 * not apply to this chart at all, with the reason in the tooltip (`BI-50`, QA
 * `LH-BI-9-05`).
 */
export function GrainMarkers({ skipped, truncated, limit, notApplied }: {
  readonly skipped?: string;
  readonly truncated?: boolean;
  readonly limit?: number;
  /** Why "Group dates by" does not apply here, as the one-line sentence of the server's reason (`BI-50`). */
  readonly notApplied?: string;
}) {
  const label = skipped && isGrain(skipped) ? GRAIN_LABELS[skipped].toLowerCase() : skipped;
  return (
    <>
      {skipped ? (
        <Tooltip>
          <TooltipTrigger render={<span className="inline-flex shrink-0 text-muted-foreground/70" />}>
            <CalendarOff className="size-3.5" aria-label={`Grouping by ${label} not applied to this chart`} />
          </TooltipTrigger>
          <TooltipContent>
            <p className="font-medium">Not applied to this chart</p>
            <p>Its date column cannot be grouped by {label}, so it keeps its own grouping.</p>
          </TooltipContent>
        </Tooltip>
      ) : null}
      {notApplied ? (
        <Tooltip>
          <TooltipTrigger render={<span className="inline-flex shrink-0 text-muted-foreground/70" />}>
            <CalendarOff className="size-3.5" aria-label={notApplied} />
          </TooltipTrigger>
          <TooltipContent>
            <p className="font-medium">Group dates by does not apply</p>
            <p>{notApplied}</p>
          </TooltipContent>
        </Tooltip>
      ) : null}
      {truncated ? (
        <Tooltip>
          <TooltipTrigger render={<span className="inline-flex shrink-0 text-muted-foreground/70" />}>
            <Scissors className="size-3.5" aria-label="Only the latest buckets are shown" />
          </TooltipTrigger>
          <TooltipContent>
            <p className="font-medium">Cut off</p>
            <p>{limit ? `Only the latest ${limit} are shown.` : "Only the latest buckets are shown."}</p>
          </TooltipContent>
        </Tooltip>
      ) : null}
    </>
  );
}
