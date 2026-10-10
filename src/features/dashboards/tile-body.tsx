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
import { useReporting } from "./reporting-context";
import { KpiCard } from "./kpi-card";
import { PivotTable } from "./pivot-table";
import { RawTable } from "./raw-table";
import { cellClass, CellValue, columnStyle } from "./table-cell";
import type { ColumnSetting, TableDefFields } from "@/lib/table-types";
import type { FilterDef } from "@/services/clients/bi-store";
import { bucketLabel, calendarFirstDay, isGrain } from "@/lib/time-grain";
import type { Rows } from "./tile-dialogs";

type Cell = Rows | TileFailure;
function hasRows(c: Cell | undefined): c is Rows {
  return !!c && "rows" in c;
}

/** Render a tile's body per its kind: text / kpi / table / chart. */
export function TileBody({
  spec, cell, dark, loading, onDataClick, hideLegend, paging,
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
  /**
   * A raw table can page and sort through the records route with these
   * filters. Absent (an embed, a public link, a preview): it shows its first
   * page only (BI-16 part A).
   */
  paging?: { filters: FilterDef[] };
}) {
  const reporting = useReporting();
  // `def` is the stored definition (typed `unknown` on the shared spec): the
  // fields of raw tables, pivots and KPI comparisons are read from it (BI-16 part A).
  const def = spec.def as TableDefFields | undefined;
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

  if (spec.kind === "kpi" && hasRows(cell) && def?.compare) {
    return <KpiCard def={def} cell={cell} caption={spec.caption} />;
  }
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
    if (def?.tableMode === "rows") {
      return <RawTable spec={{ mart: spec.mart, sqlSource: spec.sqlSource, title: spec.title, def }} cell={cell} paging={paging} />;
    }
    return <TableView columns={cell.columns} rows={cell.rows} settings={def?.columnSettings} />;
  }

  if (spec.kind === "pivot") {
    if (!hasRows(cell) || cell.rows.length === 0 || !def) {
      return <p className="grid h-full place-items-center text-xs text-muted-foreground">No data.</p>;
    }
    return <PivotTable def={def} cell={cell} />;
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
    // BI-9: a grouped chart's buckets arrive as dates or numbers; the axis,
    // tooltip and legend read them as "Mar 2026", "Q1 2026", "Mon". A click
    // maps the label back through `rawRows` (the rows as the server sent them).
    const rawRows = cell.rows;
    const grain = isGrain(cell.grain) ? cell.grain : null;
    const rows = grain ? rawRows.map((r) => ({ ...r, [spec.x]: bucketLabel(grain, r[spec.x]) })) : rawRows;
    const option = buildOption(spec, rows, dark, { firstDay: calendarFirstDay(reporting.weekStart) });
    return (
      <EChart option={hideLegend ? { ...option, legend: { show: false } } : option} height="100%"
        onDataClick={onDataClick ? (hit, pos) => onDataClick(hit, pos, { rows: rawRows, labelledRows: rows, grain: grain ?? undefined }) : undefined} />
    );
  }
  return <p className="grid h-full place-items-center text-xs text-muted-foreground">No data.</p>;
}

function TableView({ columns, rows, settings = {} }: { columns: string[]; rows: Record<string, unknown>[]; settings?: Record<string, ColumnSetting> }) {
  const shown = columns.filter((c) => !settings[c]?.hidden);
  return (
    <div className="h-full overflow-auto">
      <table className="w-full border-collapse text-xs">
        <thead className="sticky top-0 bg-card">
          <tr className="border-b border-border">
            {shown.map((c) => <th key={c} style={columnStyle(settings[c])} className="px-2 py-1.5 text-left font-medium text-muted-foreground">{settings[c]?.label ?? c}</th>)}
          </tr>
        </thead>
        <tbody>
          {rows.map((r, i) => (
            <tr key={i} className="border-b border-border/40 last:border-0">
              {shown.map((c) => <td key={c} style={columnStyle(settings[c])} className={cellClass(settings[c])}><CellValue value={r[c]} setting={settings[c]} /></td>)}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
