"use client";

import * as React from "react";
import type { ChartClickHandler } from "@/lib/chart-click";
import type { ChartRenderSpec } from "@/lib/dashboard-specs";
import { ErrorWithReference } from "@/components/error-reference";
import type { TileFailure } from "@/services/contracts/dashboards";
import { MiniMarkdown } from "@/features/copilot/mini-markdown";
import { EChart } from "./echart";
import { buildOption, fmtInt } from "./chart-option";
import { GeoChart } from "./geo-chart";

type Cell = { columns: string[]; rows: Record<string, unknown>[] } | TileFailure;
function hasRows(c: Cell | undefined): c is { columns: string[]; rows: Record<string, unknown>[] } {
  return !!c && "rows" in c;
}

/** Render a tile's body per its kind: text / kpi / table / chart. */
export function TileBody({
  spec, cell, dark, loading, onDataClick, hideLegend,
}: {
  spec: ChartRenderSpec & { text?: string; caption?: string };
  cell: Cell | undefined;
  dark: boolean;
  loading: boolean;
  /** Click a data point (bar/slice/region/point/day/node/box) → drill/cross-filter. */
  onDataClick?: ChartClickHandler;
  /**
   * Draw the chart without its legend: for a small preview, where a legend
   * of many entries takes the room and cannot be read. The tooltip still
   * names each series.
   */
  hideLegend?: boolean;
}) {
  if (spec.kind === "text") {
    return <div className="h-full overflow-auto px-1 py-0.5 text-sm leading-relaxed"><MiniMarkdown text={spec.text ?? ""} /></div>;
  }
  if (cell && "error" in cell) {
    return (
      <p className="grid h-full place-items-center px-2 text-center text-xs text-destructive">
        <ErrorWithReference message={cell.error} errorId={cell.errorId} />
      </p>
    );
  }
  if (loading && !cell) return <div className="h-full animate-pulse rounded bg-muted/40" />;

  if (spec.kind === "kpi") {
    const v = hasRows(cell) ? Number(cell.rows[0]?.v ?? 0) : null;
    return (
      <div className="grid h-full place-content-center px-2 text-center">
        <p className="text-4xl font-semibold tabular-nums text-foreground">{v === null ? "—" : fmtInt(v)}</p>
        {spec.caption ? <p className="mt-1 text-xs text-muted-foreground">{spec.caption}</p> : null}
      </div>
    );
  }

  if (spec.kind === "table") {
    if (!hasRows(cell) || cell.rows.length === 0) {
      return <p className="grid h-full place-items-center text-xs text-muted-foreground">No data.</p>;
    }
    return <TableView columns={cell.columns} rows={cell.rows} />;
  }

  // Map kinds — need the map registered first (local GeoJSON).
  if (spec.kind === "geomap" || spec.kind === "pointmap" || spec.kind === "geoheat") {
    if (!hasRows(cell) || cell.rows.length === 0) {
      return <p className="grid h-full place-items-center text-xs text-muted-foreground">No data.</p>;
    }
    return <GeoChart spec={spec} rows={cell.rows} dark={dark} onDataClick={onDataClick} />;
  }

  // chart
  if (hasRows(cell) && cell.rows.length) {
    const option = buildOption(spec, cell.rows, dark);
    const rows = cell.rows;
    return (
      <EChart option={hideLegend ? { ...option, legend: { show: false } } : option} height="100%"
        onDataClick={onDataClick ? (hit, pos) => onDataClick(hit, pos, { rows }) : undefined} />
    );
  }
  return <p className="grid h-full place-items-center text-xs text-muted-foreground">No data.</p>;
}

function TableView({ columns, rows }: { columns: string[]; rows: Record<string, unknown>[] }) {
  const fmt = (v: unknown) => (typeof v === "number" ? v.toLocaleString("id-ID") : String(v ?? ""));
  return (
    <div className="h-full overflow-auto">
      <table className="w-full border-collapse text-xs">
        <thead className="sticky top-0 bg-card">
          <tr className="border-b border-border">
            {columns.map((c) => <th key={c} className="px-2 py-1.5 text-left font-medium text-muted-foreground">{c}</th>)}
          </tr>
        </thead>
        <tbody>
          {rows.map((r, i) => (
            <tr key={i} className="border-b border-border/40 last:border-0">
              {columns.map((c) => <td key={c} className="px-2 py-1 tabular-nums">{fmt(r[c])}</td>)}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
