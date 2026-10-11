"use client";

import { displayColumn } from "@/lib/chart-joins";
import * as React from "react";
import type { ChartClickHandler } from "@/lib/chart-click";
import type { ChartRenderSpec } from "@/lib/dashboard-specs";
import { ErrorWithReference } from "@/components/error-reference";
import type { TileFailure } from "@/services/contracts/dashboards";
import { MiniMarkdown } from "@/features/copilot/mini-markdown";
import { EChart } from "./echart";
import { buildOption, fmtInt, withReferenceLines } from "./chart-option";
import { buildCombinedOption, failedLayers } from "./combined-option";
import { takesLayers, type LayerDef } from "@/lib/chart-layers";
import { GeoChart } from "./geo-chart";
import { useReporting } from "./reporting-context";
import { KpiCard } from "./kpi-card";
import { PivotTable } from "./pivot-table";
import { RawTable } from "./raw-table";
import { cellClass, CellValue, columnStyle } from "./table-cell";
import type { ColumnSetting, TableDefFields } from "@/lib/table-types";
import type { FilterDef } from "@/services/clients/bi-store";
import { bucketLabel, calendarFirstDay, isGrain } from "@/lib/time-grain";
import { binsOfTile, relabelBins } from "@/lib/chart-bins";
import { labelOtherRows, tileMeasureLabels, visibleColumns, type ShapeDefFields } from "@/lib/chart-shape";
import type { Rows } from "./tile-dialogs";

type Cell = Rows | TileFailure;
function hasRows(c: Cell | undefined): c is Rows {
  return !!c && "rows" in c;
}

/** Render a tile's body per its kind: text / kpi / table / chart. */
export function TileBody({
  spec, cell, dark, loading, onDataClick, hideLegend, paging, onTableSort,
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
  /** A raw table reports the sort the viewer chose (for the whole-result export). */
  onTableSort?: (sort: { column: string; dir: "asc" | "desc" }) => void;
}) {
  const reporting = useReporting();
  // `def` is the stored definition (typed `unknown` on the shared spec): the
  // fields of raw tables, pivots and KPI comparisons are read from it (BI-16 part A).
  const def = spec.def as (TableDefFields & ShapeDefFields) | undefined;
  // BI-6: "avg of stay" for a measure with its own aggregate.
  const measureLabels = tileMeasureLabels(def?.measures, spec.y, def?.steps);
  // BI-10: a binned grouping arrives as the bin's lower edge; the tile says "500 – 1,000".
  const bins = hasRows(cell) ? binsOfTile(cell.bins, def) : {};
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
      return <RawTable spec={{ mart: spec.mart, sqlSource: spec.sqlSource, title: spec.title, def, chart: spec.source !== "builtin" && (def?.filters?.length || def?.joins?.length) ? spec.id : undefined }} cell={cell} paging={paging} onSortChange={onTableSort} />;
    }
    return <TableView columns={cell.columns} rows={relabelBins(cell.rows, bins)} settings={def?.columnSettings} labels={measureLabels} />;
  }

  if (spec.kind === "pivot") {
    if (!hasRows(cell) || cell.rows.length === 0 || !def) {
      return <p className="grid h-full place-items-center text-xs text-muted-foreground">No data.</p>;
    }
    return <PivotTable def={def} cell={{ ...cell, rows: relabelBins(cell.rows, bins) }} />;
  }

  // Map kinds — need the map registered first (local GeoJSON).
  if (spec.kind === "geomap" || spec.kind === "pointmap" || spec.kind === "geoheat") {
    if (!hasRows(cell) || cell.rows.length === 0) {
      return <p className="grid h-full place-items-center text-xs text-muted-foreground">No data.</p>;
    }
    return <GeoChart spec={spec} rows={cell.rows} dark={dark} onDataClick={onDataClick} />;
  }

  // BI-13: a chart with more questions on it draws all of them on one axis.
  const layerCells = hasRows(cell) ? cell.layers : undefined;
  if (hasRows(cell) && layerCells?.length && takesLayers(spec.kind) && (cell.rows.length || layerCells.some((l) => !l.error && l.rows.length))) {
    return <CombinedBody spec={spec} cell={cell} layers={layerCells} def={def} measureLabels={measureLabels} dark={dark} hideLegend={hideLegend} onDataClick={onDataClick} />;
  }

  // chart
  if (hasRows(cell) && cell.rows.length) {
    // BI-9: a grouped chart's buckets arrive as dates or numbers; the axis,
    // tooltip and legend read them as "Mar 2026", "Q1 2026", "Mon". A click
    // maps the label back through `rawRows` (the rows as the server sent them).
    const rawRows = cell.rows;
    const grain = isGrain(cell.grain) ? cell.grain : null;
    const grained = grain ? rawRows.map((r) => ({ ...r, [spec.x]: bucketLabel(grain, r[spec.x]) })) : rawRows;
    const bucketed = relabelBins(grained, bins);
    // BI-6: the computed "Other" group is drawn last and is never a value to
    // click; a real value that is also called "Other" stays apart from it.
    const { rows, otherLabels } = labelOtherRows(bucketed, spec.x);
    // BI-11: the reference lines, at the values this read gave them.
    const option = withReferenceLines(
      buildOption({ ...spec, measureLabels }, rows, dark, { firstDay: calendarFirstDay(reporting.weekStart) }),
      cell.referenceLines, spec.kind === "hbar", dark,
    );
    return (
      <EChart option={hideLegend ? { ...option, legend: { show: false } } : option} height="100%"
        onDataClick={onDataClick ? (hit, pos) => onDataClick(hit, pos, { rows: rawRows, labelledRows: rows, grain: grain ?? undefined, otherLabels, bin: bins[spec.x] }) : undefined} />
    );
  }
  return <p className="grid h-full place-items-center text-xs text-muted-foreground">No data.</p>;
}

/**
 * A chart with more questions on it (`BI-13`). The categories are the base's
 * and those only a layer has; a click on a mark says which question it belongs
 * to, so the drill that follows lists the rows of that question's own source. A
 * question that failed is listed under the chart with its reason, and the
 * others still draw.
 */
function CombinedBody({
  spec, cell, layers, def, measureLabels, dark, hideLegend, onDataClick,
}: {
  spec: ChartRenderSpec;
  cell: Rows;
  layers: NonNullable<Rows["layers"]>;
  def: (TableDefFields & ShapeDefFields & { layers?: LayerDef[]; draw?: "bar" | "line"; axis?: "left" | "right" }) | undefined;
  measureLabels: Record<string, string>;
  dark: boolean;
  hideLegend?: boolean;
  onDataClick?: ChartClickHandler;
}) {
  const grain = isGrain(cell.grain) ? cell.grain : null;
  const combined = buildCombinedOption({
    kind: spec.kind, x: spec.x, y: spec.y, measureLabels, rows: cell.rows, layers,
    layerMeasures: def?.layers?.map((l) => l.measures), baseDraw: def?.draw, baseAxis: def?.axis, grain,
  }, dark);
  const option = withReferenceLines(combined.option, cell.referenceLines, false, dark);
  const failed = failedLayers(layers);
  const click = onDataClick
    ? (hit: Parameters<ChartClickHandler>[0], pos: { x: number; y: number }) => {
        const owner = combined.owners[hit.seriesIndex ?? -1] ?? -1;
        const own = owner >= 0 ? def?.layers?.[owner] : undefined;
        // The positions of the merged axis stand for the stored values (`rows`) and the labels drawn (`labelledRows`).
        const rows = combined.keys.map((k) => ({ [spec.x]: k }));
        const labelledRows = combined.labels.map((l) => ({ [spec.x]: l }));
        const layer = owner >= 0 && own ? { index: owner, mart: own.mart ?? "", sqlSource: own.sqlSource, dimension: own.dimension } : undefined;
        onDataClick(hit, pos, { rows, labelledRows, grain: grain ?? undefined, layer });
      }
    : undefined;
  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="min-h-0 flex-1">
        <EChart option={hideLegend ? { ...option, legend: { show: false } } : option} height="100%" onDataClick={click} />
      </div>
      {failed.length ? (
        <ul className="px-1 pt-1 text-[11px] text-destructive" aria-label="Questions that could not be drawn">
          {failed.map((f, i) => (
            <li key={i} className="truncate"><span className="font-medium">{f.label}:</span> <ErrorWithReference message={f.error} errorId={f.errorId} /></li>
          ))}
        </ul>
      ) : null}
    </div>
  );
}

function TableView({ columns, rows, settings = {}, labels = {} }: { columns: string[]; rows: Record<string, unknown>[]; settings?: Record<string, ColumnSetting>; labels?: Record<string, string> }) {
  // The flag that marks the computed "Other" group is not a column to read.
  const shown = visibleColumns(columns).filter((c) => !settings[c]?.hidden);
  return (
    <div className="h-full overflow-auto">
      <table className="w-full border-collapse text-xs">
        <thead className="sticky top-0 bg-card">
          <tr className="border-b border-border">
            {shown.map((c) => <th key={c} style={columnStyle(settings[c])} className="px-2 py-1.5 text-left font-medium text-muted-foreground">{settings[c]?.label ?? labels[c] ?? displayColumn(c)}</th>)}
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
