"use client";

import { decimalsFor } from "@/lib/chart-axis";
import type { StyleFields } from "@/lib/chart-style";
import { progressView, type TypeDefFields } from "@/lib/type-b";
import { cn } from "@/lib/utils";
import { formatValue } from "@/lib/value-format";
import { useReporting } from "./reporting-context";
import type { Rows } from "./tile-dialogs";

const num = (v: unknown): number | null => {
  if (v === undefined || v === null || v === "") return null;
  const n = Number(v);
  return Number.isFinite(n) ? n : null;
};

/**
 * A value against its goal as a bar (`BI-16` part B, decisions 1 and 8): the
 * value, the goal and the percentage reached, in the value's number format.
 * Above the goal the bar is full and the label keeps the real percentage
 * ("112%"); a goal of zero or less says there is nothing to compare with. Plain
 * HTML and CSS: the bar is a `progressbar`, so the percentage is read aloud and
 * colour is never the only cue.
 */
export function ProgressCard({
  def, cell, format, caption,
}: {
  readonly def: TypeDefFields & StyleFields;
  readonly cell: Rows;
  /** The number format of the value: the chart's own, else its metric's. */
  readonly format?: StyleFields["format"];
  readonly caption?: string;
}) {
  const { numberLocale } = useReporting();
  const row = cell.rows[0];
  const value = num(row?.v);
  // A fixed goal is in the definition; a goal that is a second value came back as `g`.
  const goal = typeof def.goal === "number" ? def.goal : def.goal ? num(row?.g) : null;
  const view = progressView(value, goal);
  const fmt = format ?? def.format;
  if (view.kind === "no-goal") {
    return (
      <div className="grid h-full place-content-center gap-1 px-2 text-center">
        {value !== null ? (
          <p className="text-2xl font-semibold tabular-nums">{formatValue(value, fmt, numberLocale, decimalsFor([value]))}</p>
        ) : null}
        <p className="text-xs text-muted-foreground">No goal to compare with</p>
      </div>
    );
  }
  const dec = decimalsFor([view.value, view.goal]);
  const percent = formatValue(view.ratio, { style: "percent", decimals: view.ratio < 0.1 ? 1 : 0 }, numberLocale, 0);
  const over = view.ratio > 1;
  return (
    <div className="grid h-full content-center gap-2 px-3">
      <div className="flex items-baseline justify-between gap-2">
        <p className="text-3xl font-semibold tabular-nums">{percent}</p>
        <p className="truncate text-sm tabular-nums text-muted-foreground">
          {formatValue(view.value, fmt, numberLocale, dec)} of {formatValue(view.goal, fmt, numberLocale, dec)}
        </p>
      </div>
      <div
        role="progressbar" aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(view.fill * 100)}
        aria-valuetext={`${percent} of the goal`} aria-label="Progress towards the goal"
        className="h-3 w-full overflow-hidden rounded-full bg-muted"
      >
        <div className={cn("h-full rounded-full", over ? "bg-emerald-600 dark:bg-emerald-500" : "bg-primary")} style={{ width: `${view.fill * 100}%` }} />
      </div>
      {over ? <p className="text-xs text-muted-foreground">Above the goal</p> : null}
      {caption ? <p className="text-xs text-muted-foreground">{caption}</p> : null}
    </div>
  );
}
