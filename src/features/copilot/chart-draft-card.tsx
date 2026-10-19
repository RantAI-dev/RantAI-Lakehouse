"use client";

import { normalizeDef } from "@/lib/params";
import * as React from "react";
import { useTheme } from "next-themes";
import { ChartColumn, PencilRuler } from "lucide-react";
import { Button } from "@/components/ui/button";
import { ChartBuilder, type ChartDef } from "@/features/dashboards/chart-builder";
import { TileBody } from "@/features/dashboards/tile-body";
import type { ChartKind, ChartRenderSpec } from "@/lib/dashboard-specs";
import type { BinDef, GridDef } from "@/lib/chart-bins";
import type { JoinDef } from "@/lib/chart-joins";
import type { LayerDef } from "@/lib/chart-layers";
import { columnRef, metricColumn, type Grouping, type MeasureEntry, type SortSpec, type StepDef } from "@/lib/chart-shape";
import type { RefLineDef } from "@/lib/params";
import type { ColumnSetting, KpiCompare, PivotValue } from "@/lib/table-types";
import type { ChartClick, FilterDef } from "@/services/clients/bi-store";
import { apiFetch } from "@/services/http";
import type { TileFailure } from "@/services/contracts/dashboards";

type Preview = {
  spec: ChartRenderSpec & { text?: string; caption?: string };
  // `metricLabels` is on the result, and only when the chart has saved metrics with labels.
  result: { columns: string[]; rows: Record<string, unknown>[]; metricLabels?: Record<string, string> } | TileFailure;
};

function isObject(v: unknown): v is Record<string, unknown> {
  return typeof v === "object" && v !== null && !Array.isArray(v);
}

function str(v: unknown): string | undefined {
  return typeof v === "string" && v.trim() ? v : undefined;
}

/**
 * `create_chart` arguments as a chart definition — the manual builder's own
 * shape, so the same draft can be previewed, saved, or opened in the builder.
 * The model sometimes leaves `title` empty and puts the name in `caption`.
 *
 * The preview must be built from the same fields Confirm sends: Confirm posts
 * the model's raw arguments, so a field this function drops makes the preview
 * and "Edit in builder" show a different chart from the one that is saved.
 * Every field of `ChartDef` the server accepts is passed through as sent.
 */
export function chartDefFromArgs(args: Record<string, unknown>): ChartDef {
  // BI-6: a measure is a column name, or a column with its own aggregate.
  const measures = Array.isArray(args.measures)
    ? args.measures.filter((m): m is MeasureEntry =>
        typeof m === "string" ||
        // BI-2: a measure may be a saved metric by name.
        (typeof m === "object" && m !== null && typeof (m as { metric?: unknown }).metric === "string") ||
        (typeof m === "object" && m !== null && typeof (m as { column?: unknown }).column === "string" && typeof (m as { aggregate?: unknown }).aggregate === "string"))
    : undefined;
  return {
    title: str(args.title) ?? str(args.caption) ?? str(args.name) ?? str(args.text) ?? "",
    subtitle: str(args.subtitle),
    mart: str(args.mart),
    // A chart on a dashboard SQL source carries this instead of `mart`;
    // without it the draft preview and "Edit in builder" saw an empty mart.
    sqlSource: str(args.sqlSource),
    kind: str(args.kind) as ChartKind | undefined,
    dimension: str(args.dimension),
    measures,
    breakdown: str(args.breakdown),
    map: str(args.map),
    lat: str(args.lat),
    lon: str(args.lon),
    aggregate: str(args.aggregate),
    span: args.span === 2 ? 2 : 1,
    board: str(args.board),
    text: str(args.text),
    caption: str(args.caption),
    target: typeof args.target === "number" ? args.target : undefined,
    // A number, or a number parameter held as { param }.
    limit: typeof args.limit === "number" ? args.limit : isObject(args.limit) && typeof args.limit.param === "string" ? (args.limit as { param: string }) : undefined,
    order: str(args.order) as ChartDef["order"],
    grain: str(args.grain),
    // BI-6: the chart's own filters, sort, "Other" and groupings travel to the builder as the model sent them.
    filters: Array.isArray(args.filters) ? (args.filters as FilterDef[]) : undefined,
    sort: typeof args.sort === "object" && args.sort !== null ? (args.sort as SortSpec) : undefined,
    other: args.other === true ? true : undefined,
    groupings: Array.isArray(args.groupings) ? (args.groupings as Grouping[]) : undefined,
    steps: Array.isArray(args.steps) ? (args.steps as StepDef[]) : undefined,
    // BI-7: the joins travel to the builder as the model sent them.
    joins: Array.isArray(args.joins) ? (args.joins as JoinDef[]) : undefined,
    // BI-10: bins and the map grid travel to the builder as the model sent them.
    bin: args.bin && typeof args.bin === "object" ? (args.bin as BinDef) : undefined,
    breakdownBin: args.breakdownBin && typeof args.breakdownBin === "object" ? (args.breakdownBin as BinDef) : undefined,
    grid: args.grid && typeof args.grid === "object" ? (args.grid as GridDef) : undefined,
    // BI-13: the chart's other questions, and how its own series is drawn, travel to the builder as the model sent them.
    layers: Array.isArray(args.layers) ? (args.layers as LayerDef[]) : undefined,
    draw: args.draw === "bar" || args.draw === "line" ? args.draw : undefined,
    axis: args.axis === "left" || args.axis === "right" ? args.axis : undefined,
    // BI-16: raw tables, pivots and the KPI comparison travel to the builder as the model sent them.
    tableMode: str(args.tableMode) as ChartDef["tableMode"],
    columns: Array.isArray(args.columns) ? (args.columns as string[]) : undefined,
    rows: Array.isArray(args.rows) ? (args.rows as string[]) : undefined,
    values: Array.isArray(args.values) ? (args.values as PivotValue[]) : undefined,
    totals: str(args.totals) as ChartDef["totals"],
    sortColumn: str(args.sortColumn),
    sortDir: str(args.sortDir) as ChartDef["sortDir"],
    columnSettings: isObject(args.columnSettings) ? (args.columnSettings as Record<string, ColumnSetting>) : undefined,
    compare: isObject(args.compare) ? (args.compare as KpiCompare) : undefined,
    goodDirection: str(args.goodDirection) as ChartDef["goodDirection"],
    // BI-11 and BI-18: the value-axis lines and what a click does.
    referenceLines: Array.isArray(args.referenceLines) ? (args.referenceLines as RefLineDef[]) : undefined,
    click: isObject(args.click) ? (args.click as ChartClick) : undefined,
  };
}

/**
 * The footer's measure text for a draft. A saved metric reads by its label
 * (its name until the preview has answered) and carries no aggregate, because
 * the metric defines its own; a measure with its own aggregate shows that one;
 * a plain column takes the chart's aggregate, `sum` when it sets none.
 */
export function draftMeasureLabel(def: ChartDef, metricLabels: Readonly<Record<string, string>> | undefined): string {
  if (!def.measures?.length) return "";
  const text = def.measures
    .map((m) => {
      if (typeof m === "string") return `${def.aggregate ?? "sum"}(${m})`;
      if ("metric" in m) return columnRef(metricColumn(m.metric), { metricLabels }).text;
      return `${m.aggregate}(${m.column})`;
    })
    .join(", ");
  return def.grain ? `${text} per ${def.grain}` : text;
}

/**
 * A `create_chart` waiting for confirmation, shown as the chart it would
 * create: rendered through the builder's preview endpoint, so what you see is
 * exactly what "Add to dashboard" saves. "Edit in builder" opens the manual
 * builder prefilled with the same draft for anything the model got wrong.
 */
export function ChartDraftCard({
  args, confirming, onConfirm, onCancel, onSavedInBuilder,
}: {
  args: Record<string, unknown>;
  confirming: boolean;
  onConfirm: () => void;
  onCancel: () => void;
  /** The draft was saved from the builder instead — it no longer needs confirming. */
  onSavedInBuilder: (def: ChartDef) => void;
}) {
  const { resolvedTheme } = useTheme();
  // `args` is rebuilt on every render; key the draft on its content so the
  // preview is fetched once per draft, not once per render.
  const argsKey = JSON.stringify(args);
  // BI-11: a parameter in a column slot arrives as { param } and is held as {name} text, as on the dashboard.
  const def = React.useMemo(() => normalizeDef(chartDefFromArgs(JSON.parse(argsKey))) ?? {}, [argsKey]);
  const [preview, setPreview] = React.useState<Preview | null>(null);
  const [previewError, setPreviewError] = React.useState<string | null>(null);
  const [loading, setLoading] = React.useState(true);
  const [builderOpen, setBuilderOpen] = React.useState(false);

  React.useEffect(() => {
    const controller = new AbortController();
    setLoading(true);
    setPreviewError(null);
    void apiFetch("/api/dashboard/specs/preview", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ ...def, title: def.title || "Chart preview" }),
      signal: controller.signal,
    })
      .then(async (res) => {
        const json = await res.json();
        if (!res.ok) throw new Error(json?.error ?? "Preview failed");
        setPreview(json as Preview);
      })
      .catch((e: unknown) => {
        if (!controller.signal.aborted) setPreviewError(e instanceof Error ? e.message : String(e));
      })
      .finally(() => {
        if (!controller.signal.aborted) setLoading(false);
      });
    return () => controller.abort();
  }, [def]);

  const metricLabels = preview && "metricLabels" in preview.result ? preview.result.metricLabels : undefined;
  const measureLabel = draftMeasureLabel(def, metricLabels);

  return (
    <div className="overflow-hidden rounded-xl border border-border bg-card text-xs shadow-xs">
      <div className="flex items-start gap-2 border-b border-border px-3 py-2">
        <span className="mt-0.5 grid size-6 shrink-0 place-items-center rounded-md bg-primary/10 text-primary">
          <ChartColumn className="size-3.5" />
        </span>
        <div className="min-w-0 flex-1">
          <p className="truncate text-sm font-semibold text-foreground">{def.title || "Untitled chart"}</p>
          {def.subtitle ? <p className="truncate text-muted-foreground">{def.subtitle}</p> : null}
        </div>
        <span className="shrink-0 rounded-full bg-amber-500/15 px-2 py-0.5 text-[10px] font-medium text-amber-700 dark:text-amber-400">
          Draft
        </span>
      </div>

      <div className="h-56 p-2">
        {previewError ? (
          <div className="grid h-full place-items-center px-4 text-center text-muted-foreground">
            Preview unavailable: {previewError}
          </div>
        ) : preview ? (
          <TileBody
            spec={preview.spec}
            cell={preview.result}
            dark={resolvedTheme === "dark"}
            loading={loading}
          />
        ) : (
          <div className="h-full animate-pulse rounded-lg bg-muted/60" />
        )}
      </div>

      <div className="flex flex-wrap items-center gap-x-2 gap-y-1 border-t border-border px-3 py-2 text-muted-foreground">
        {def.kind ? <span className="rounded bg-muted px-1.5 py-0.5 font-mono text-[10px]">{def.kind}</span> : null}
        {def.sqlSource ? (
          <span className="text-[10px]">SQL source</span>
        ) : def.mart ? (
          <span className="font-mono text-[10px]">{def.mart}</span>
        ) : null}
        {def.dimension ? <span>by {def.dimension}</span> : null}
        {measureLabel ? <span>· {measureLabel}</span> : null}
      </div>

      <div className="flex flex-wrap items-center gap-2 border-t border-border px-3 py-2">
        <Button size="sm" onClick={onConfirm} disabled={confirming}>
          {confirming ? "Adding…" : "Add to dashboard"}
        </Button>
        <Button size="sm" variant="outline" onClick={() => setBuilderOpen(true)} disabled={confirming}>
          <PencilRuler className="size-3.5" /> Edit in builder
        </Button>
        <Button size="sm" variant="ghost" className="ml-auto" onClick={onCancel} disabled={confirming}>
          Discard
        </Button>
      </div>

      {builderOpen ? (
        <ChartBuilder
          hideTrigger
          open={builderOpen}
          onOpenChange={setBuilderOpen}
          initial={def}
          board={def.board ?? "default"}
          onSaved={(saved) => {
            setBuilderOpen(false);
            onSavedInBuilder(saved);
          }}
        />
      ) : null}
    </div>
  );
}
