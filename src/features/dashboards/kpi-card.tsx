"use client";

import { ArrowDown, ArrowUp, Minus } from "lucide-react";
import { readKpi, signedAmount, signedPercent, sparklinePoints, type KpiReading } from "@/lib/kpi-compare";
import { bucketLabel, isGrain } from "@/lib/time-grain";
import type { TableDefFields } from "@/lib/table-types";
import { cn } from "@/lib/utils";
import { fmtInt } from "./chart-option";
import type { Rows } from "./tile-dialogs";

const TONE: Record<KpiReading["tone"], string> = {
  good: "text-emerald-600 dark:text-emerald-400",
  bad: "text-red-600 dark:text-red-400",
  flat: "text-muted-foreground",
};

/**
 * A KPI with a comparison (`BI-16` part A): the number, how it differs from
 * the previous period or from a goal, as an amount and a percent coloured for
 * better or worse, and a trend line over the periods the server returned.
 * Colour is never the only cue: the arrow and the sign say the same.
 */
export function KpiCard({ def, cell, caption }: { readonly def: TableDefFields; readonly cell: Rows; readonly caption?: string }) {
  const r = readKpi(cell.rows, def.compare, def.goodDirection);
  const grain = isGrain(cell.grain) ? cell.grain : null;
  const label = (bucket: unknown) => (grain && bucket !== undefined ? bucketLabel(grain, bucket) : "");
  const Arrow = r.delta === null || r.delta === 0 ? Minus : r.delta > 0 ? ArrowUp : ArrowDown;
  return (
    <div className="grid h-full place-content-center px-2 text-center">
      <p className="text-4xl font-semibold tabular-nums text-foreground">{r.value === null ? "—" : fmtInt(r.value)}</p>
      {r.mode === "previous" && r.value !== null ? (
        <p className="mt-0.5 text-xs text-muted-foreground">{label(r.valueBucket)}</p>
      ) : null}
      {r.delta !== null ? (
        <p className={cn("mt-1 inline-flex items-center justify-center gap-1 text-sm font-medium tabular-nums", TONE[r.tone])}>
          <Arrow className="size-4" aria-hidden />
          {r.mode === "goal" ? (
            <span>
              {r.percent !== null ? `${r.percent.toLocaleString("id-ID", { maximumFractionDigits: 1 })}% of goal` : "of goal"}
              <span className="font-normal text-muted-foreground"> · {signedAmount(r.delta)} {r.delta < 0 ? "to go" : "over"}</span>
            </span>
          ) : (
            <span>
              {signedAmount(r.delta)}{r.percent !== null ? ` (${signedPercent(r.percent)})` : ""}
              {r.referenceBucket !== undefined ? <span className="font-normal text-muted-foreground"> vs {label(r.referenceBucket)}</span> : null}
            </span>
          )}
        </p>
      ) : r.mode === "previous" && r.value !== null ? (
        <p className="mt-1 text-xs text-muted-foreground">No earlier period to compare with.</p>
      ) : null}
      {r.series.length > 1 ? (
        <svg viewBox="0 0 120 28" role="img" aria-label="Trend over the last periods" className={cn("mx-auto mt-2 h-7 w-32", TONE[r.tone])}>
          <polyline points={sparklinePoints(r.series, 120, 28)} fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinejoin="round" strokeLinecap="round" />
        </svg>
      ) : null}
      {caption ? <p className="mt-1 text-xs text-muted-foreground">{caption}</p> : null}
    </div>
  );
}
