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
import { buildCombinedOption, cutLayers, failedLayers } from "./combined-option";
import { takesLayers, type LayerDef } from "@/lib/chart-layers";
import { GeoChart } from "./geo-chart";
import { useReporting } from "./reporting-context";
import { KpiCard } from "./kpi-card";
import { PivotTable } from "./pivot-table";
import { RawTable } from "./raw-table";
import { cellClass, CellValue, columnStyle, conditionalCss } from "./table-cell";
import { domainsOf } from "@/lib/conditional-format";
import { decimalsFor } from "@/lib/chart-axis";
import type { StyleFields } from "@/lib/chart-style";
import { formatAxisValue, formatValue, withMetricFormats } from "@/lib/value-format";
import { styleOption, seriesColumns } from "./style-option";
import type { ColumnSetting, TableDefFields } from "@/lib/table-types";
import type { FilterDef } from "@/services/clients/bi-store";
import { bucketLabel, calendarFirstDay, isGrain } from "@/lib/time-grain";
import { binsOfTile, relabelBins } from "@/lib/chart-bins";
import { labelOtherRows, tileMeasureLabels, visibleColumns, withResolvedSlots, type ShapeDefFields } from "@/lib/chart-shape";
import type { Rows } from "./tile-dialogs";
import { HISTOGRAM_ROW_CAP, MAX_BULLETS, type TypeDefFields } from "@/lib/type-b";
import { ProgressCard } from "./progress-card";
import { ImageCard, WebCard } from "./content-cards";
import { bulletOption, histogramOption } from "./type-b-option";
import { resolvePalette } from "@/lib/chart-palettes";

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
  const def = withResolvedSlots(spec.def as (TableDefFields & ShapeDefFields & StyleFields & TypeDefFields) | undefined, spec.resolved);
  // BI-6: "avg of stay" for a measure with its own aggregate, or one that takes the chart's (phase 2 QA R7).
  const measureLabels = tileMeasureLabels(def?.measures, spec.y, def?.steps, hasRows(cell) ? cell.metricLabels : undefined, def?.aggregate);
  // BI-10: a binned grouping arrives as the bin's lower edge; the tile says "500 – 1,000".
  const bins = hasRows(cell) ? binsOfTile(cell.bins, def) : {};
  if (spec.kind === "text") {
    return <div className="h-full overflow-auto px-1 py-0.5 text-sm leading-relaxed"><MiniMarkdown text={spec.text ?? ""} /></div>;
  }
  // BI-16 part B: an image and a web page are static like a note: no rows, no failure of a statement.
  if (spec.kind === "image") return <ImageCard def={def} title={spec.title} />;
  if (spec.kind === "web") return <WebCard def={def} title={spec.title} />;
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
    const kpiFormat = def?.format ?? (hasRows(cell) ? withMetricFormats(undefined, cell.metricFormats)?.valueFormats?.[Object.keys(cell.metricFormats ?? {})[0] ?? ""] : undefined);
    // BI-17: the KPI's number format, and the colour a rule gives its value ("green above the goal").
    const paint = v === null ? undefined : conditionalCss(def?.conditional, null, v);
    return (
      <div className="grid h-full place-content-center px-2 text-center">
        <p className="text-4xl font-semibold tabular-nums text-foreground" style={paint && { color: paint.color, backgroundColor: paint.backgroundColor }}>{v === null ? "—" : kpiFormat ? formatValue(v, kpiFormat, reporting.numberLocale, decimalsFor([v])) : fmtInt(v)}</p>
        {spec.caption ? <p className="mt-1 text-xs text-muted-foreground">{spec.caption}</p> : null}
      </div>
    );
  }

  if (spec.kind === "progress" || spec.kind === "histogram" || spec.kind === "bullet") {
    if (!hasRows(cell) || (cell.rows.length === 0 && spec.kind !== "progress")) {
      return <p className="grid h-full place-items-center text-xs text-muted-foreground">No data.</p>;
    }
    return <TypeBBody spec={spec} cell={cell} def={def} bins={bins} measureLabels={measureLabels} dark={dark} />;
  }

  if (spec.kind === "table") {
    if (!hasRows(cell) || cell.rows.length === 0) {
      return <p className="grid h-full place-items-center text-xs text-muted-foreground">No data.</p>;
    }
    if (def?.tableMode === "rows") {
      return <RawTable spec={{ mart: spec.mart, sqlSource: spec.sqlSource, title: spec.title, def, chart: spec.source !== "builtin" && (def?.filters?.length || def?.joins?.length) ? spec.id : undefined }} cell={cell} paging={paging} onSortChange={onTableSort} />;
    }
    return <TableView columns={cell.columns} rows={relabelBins(cell.rows, bins)} settings={def?.columnSettings} labels={measureLabels} style={withMetricFormats(def, cell.metricFormats)} />;
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
    return <GeoChart spec={spec} rows={cell.rows} dark={dark} onDataClick={onDataClick} format={def?.format} locale={reporting.numberLocale} byValue={def?.colors?.byValue} />;
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
    // BI-17: formats, palette, axes, labels and notes; BI-11/BI-17: the reference lines, at the values this read gave them.
    const locale = reporting.numberLocale ?? "en-US";
    const style = withMetricFormats(def as StyleFields | undefined, cell.metricFormats);
    const yCols = Array.isArray(spec.y) ? spec.y : [spec.y];
    const decimals = decimalsFor(rows.flatMap((r) => yCols.map((c) => Number(r[c] ?? 0))));
    const built = buildOption({ ...spec, measureLabels }, rows, dark, { firstDay: calendarFirstDay(reporting.weekStart), style, locale, companyPalette: reporting.companyPalette });
    // The result column each series draws, in series order: a computed
    // reference line is drawn on the axis of the measure it describes
    // (`PR #199 review M BLOCKER 4`, decision M2).
    const cols = seriesColumns({ kind: spec.kind, y: spec.y, breakdown: !!spec.series }, (built.series as unknown[] | undefined)?.length ?? 0);
    const styled = styleOption(built, { kind: spec.kind, y: spec.y, breakdown: !!spec.series, style, locale, decimals, columns: cols, referenceLines: cell.referenceLines });
    if (styled.message) {
      return <p role="status" className="grid h-full place-items-center px-3 text-center text-xs text-muted-foreground">{styled.message}.</p>;
    }
    const fmtOf = (column?: string) => style?.valueFormats?.[column ?? yCols[0] ?? ""] ?? style?.format;
    const option = withReferenceLines(
      styled.option,
      cell.referenceLines, spec.kind === "hbar", dark,
      {
        values: (measure) => rows.map((r) => Number(r[measure ?? yCols[0] ?? ""] ?? 0)),
        format: (measure, v) => formatValue(v, fmtOf(measure), locale, decimals),
        cutOff: !!cell.truncated,
      },
      cols,
    );
    const chart = (
      <EChart option={hideLegend ? { ...option, legend: { show: false } } : option} height="100%"
        onDataClick={onDataClick ? (hit, pos) => onDataClick(hit, pos, { rows: rawRows, labelledRows: rows, grain: grain ?? undefined, otherLabels, bin: bins[spec.x] }) : undefined} />
    );
    if (!styled.hiddenNotes) return chart;
    return (
      <div className="flex h-full min-h-0 flex-col">
        <div className="min-h-0 flex-1">{chart}</div>
        <p className="px-1 pt-1 text-[11px] text-muted-foreground">
          {styled.hiddenNotes} {styled.hiddenNotes === 1 ? "note" : "notes"} not shown: {styled.hiddenNotes === 1 ? "its point is" : "their points are"} not in the result
        </p>
      </div>
    );
  }
  return <p className="grid h-full place-items-center text-xs text-muted-foreground">No data.</p>;
}

/**
 * The progress bar, the histogram and the bullet graph (`BI-16` part B). They
 * take the dashboard's filters like any chart and the number formats of
 * `BI-17`: the value's own format, else the chart's, else the metric's.
 */
function TypeBBody({
  spec, cell, def, bins, measureLabels, dark,
}: {
  spec: ChartRenderSpec & { caption?: string };
  cell: Rows;
  def: (TableDefFields & ShapeDefFields & StyleFields & TypeDefFields) | undefined;
  bins: ReturnType<typeof binsOfTile>;
  measureLabels: Record<string, string>;
  dark: boolean;
}) {
  const reporting = useReporting();
  const locale = reporting.numberLocale ?? "en-US";
  const style = withMetricFormats(def, cell.metricFormats);
  const yCols = Array.isArray(spec.y) ? spec.y : [spec.y];
  const fmtOf = (column?: string) => style?.valueFormats?.[column ?? ""] ?? style?.format;
  if (spec.kind === "progress") {
    const metricFormat = fmtOf(yCols[0]) ?? (cell.metricFormats ? withMetricFormats(undefined, cell.metricFormats)?.valueFormats?.[Object.keys(cell.metricFormats)[0] ?? ""] : undefined);
    return <ProgressCard def={def ?? {}} cell={cell} format={metricFormat} caption={spec.caption} />;
  }
  const color = resolvePalette(def?.colors, reporting.companyPalette)[0];
  if (spec.kind === "histogram") {
    const countColumn = yCols[0] ?? "";
    const counts = cell.rows.map((r) => Number(r[countColumn] ?? 0));
    const decimals = decimalsFor(counts);
    const drawn = histogramOption({
      rows: cell.rows, column: spec.x, countColumn, width: bins[spec.x]?.width, dark, color,
      format: (v) => formatValue(v, fmtOf(countColumn), locale, decimals),
      formatEdge: (v) => formatValue(v, undefined, locale, decimalsFor([v])),
      showLabels: def?.labels?.show,
    });
    if ("message" in drawn) {
      return <p role="status" className="grid h-full place-items-center px-3 text-center text-xs text-muted-foreground">{drawn.message}.</p>;
    }
    // A result as long as the server's cap may have been cut (a narrow width over a wide column); say so, as a cut-off chart does.
    return (
      <div className="flex h-full min-h-0 flex-col">
        <div className="min-h-0 flex-1"><EChart option={drawn.option} height="100%" /></div>
        {cell.rows.length >= HISTOGRAM_ROW_CAP ? (
          <p className="px-1 pt-1 text-[11px] text-muted-foreground">Only the first {HISTOGRAM_ROW_CAP} ranges are shown; choose a wider width.</p>
        ) : null}
      </div>
    );
  }
  const valueColumn = yCols[0] ?? "v";
  const goalColumn = def?.goal && typeof def.goal === "object" ? (yCols[1] ?? "g") : undefined;
  const all = cell.rows.flatMap((r) => [Number(r[valueColumn] ?? 0), ...(goalColumn ? [Number(r[goalColumn] ?? 0)] : [])]);
  const decimals = decimalsFor(all);
  const drawn = bulletOption({
    rows: cell.rows, category: spec.x || undefined, valueColumn, goalColumn,
    goal: typeof def?.goal === "number" ? def.goal : undefined, bands: def?.bands ?? [],
    singleLabel: measureLabels[valueColumn] ?? firstMeasureLabel(def?.measures) ?? spec.title,
    dark, color,
    format: (v) => formatValue(v, fmtOf(valueColumn), locale, decimals),
    axisFormat: (v) => formatAxisValue(v, fmtOf(valueColumn), locale, decimals),
  });
  if ("message" in drawn) {
    return <p role="status" className="grid h-full place-items-center px-3 text-center text-xs text-muted-foreground">{drawn.message}.</p>;
  }
  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="min-h-0 flex-1"><EChart option={drawn.option} height="100%" /></div>
      {drawn.more > 0 ? (
        <p className="px-1 pt-1 text-[11px] text-muted-foreground">{drawn.more} more not shown (the first {MAX_BULLETS} are drawn)</p>
      ) : null}
    </div>
  );
}

/** The first measure's name as a label, for the one bullet of a graph without a category. */
function firstMeasureLabel(measures: ShapeDefFields["measures"]): string | undefined {
  const first = measures?.[0];
  if (first === undefined) return undefined;
  return displayColumn(typeof first === "string" ? first : "metric" in first ? first.metric : first.column);
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
  const reporting = useReporting();
  const style = def as StyleFields | undefined;
  const locale = reporting.numberLocale ?? "en-US";
  const combined = buildCombinedOption({
    kind: spec.kind, x: spec.x, y: spec.y, measureLabels, metricLabels: cell.metricLabels, rows: cell.rows, layers,
    layerMeasures: def?.layers?.map((l) => l.measures), baseDraw: def?.draw, baseAxis: def?.axis, grain,
    style, locale, companyPalette: reporting.companyPalette,
  }, dark);
  const decimals = decimalsFor(Array.isArray(combined.option.series) ? combined.option.series.flatMap((s) => ((s as { data?: unknown[] }).data ?? []).filter((v): v is number => typeof v === "number")) : []);
  const styled = styleOption(combined.option, {
    kind: spec.kind, y: spec.y, style, locale, decimals,
    columns: Array.isArray(combined.option.series) ? combined.option.series.map((s) => (s as { name?: string }).name) : undefined,
  });
  if (styled.message) {
    return <p role="status" className="grid h-full place-items-center px-3 text-center text-xs text-muted-foreground">{styled.message}.</p>;
  }
  const option = withReferenceLines(styled.option, cell.referenceLines, false, dark);
  const failed = failedLayers(layers);
  const cut = cutLayers(layers);
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
      {styled.hiddenNotes ? (
        <p className="px-1 pt-1 text-[11px] text-muted-foreground">
          {styled.hiddenNotes} {styled.hiddenNotes === 1 ? "note" : "notes"} not shown: {styled.hiddenNotes === 1 ? "its point is" : "their points are"} not in the result
        </p>
      ) : null}
      {cut.length ? (
        <p className="px-1 pt-1 text-[11px] text-muted-foreground" aria-label="Questions that were cut off">
          {cut.map((label) => `${label}: not every value is shown`).join(" · ")}
        </p>
      ) : null}
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

function TableView({ columns, rows, settings = {}, labels = {}, style }: { columns: string[]; rows: Record<string, unknown>[]; settings?: Record<string, ColumnSetting>; labels?: Record<string, string>; style?: StyleFields }) {
  const domains = React.useMemo(() => domainsOf(style?.conditional, rows), [style?.conditional, rows]);
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
              {shown.map((c) => <td key={c} style={{ ...columnStyle(settings[c]), ...conditionalCss(style?.conditional, c, r[c], domains) }} className={cellClass(settings[c])}><CellValue value={r[c]} setting={settings[c]} format={style?.valueFormats?.[c] ?? style?.format} /></td>)}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
