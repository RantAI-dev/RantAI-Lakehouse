"use client";

import * as React from "react";
import { useTheme } from "next-themes";
import { ChartColumn, PencilRuler } from "lucide-react";
import { Button } from "@/components/ui/button";
import { ChartBuilder, type ChartDef } from "@/features/dashboards/chart-builder";
import { TileBody } from "@/features/dashboards/tile-body";
import type { ChartKind, ChartRenderSpec } from "@/lib/dashboard-specs";
import { apiFetch } from "@/services/http";

type Preview = {
  spec: ChartRenderSpec & { text?: string; caption?: string };
  result: { columns: string[]; rows: Record<string, unknown>[] } | { error: string };
};

function str(v: unknown): string | undefined {
  return typeof v === "string" && v.trim() ? v : undefined;
}

/**
 * `create_chart` arguments as a chart definition — the manual builder's own
 * shape, so the same draft can be previewed, saved, or opened in the builder.
 * The model sometimes leaves `title` empty and puts the name in `caption`.
 */
export function chartDefFromArgs(args: Record<string, unknown>): ChartDef {
  const measures = Array.isArray(args.measures)
    ? args.measures.filter((m): m is string => typeof m === "string")
    : undefined;
  return {
    title: str(args.title) ?? str(args.caption) ?? str(args.name) ?? "",
    subtitle: str(args.subtitle),
    mart: str(args.mart),
    kind: str(args.kind) as ChartKind | undefined,
    dimension: str(args.dimension),
    measures,
    breakdown: str(args.breakdown),
    aggregate: str(args.aggregate),
    span: args.span === 2 ? 2 : 1,
    board: str(args.board),
    text: str(args.text),
    caption: str(args.caption),
    target: typeof args.target === "number" ? args.target : undefined,
    limit: typeof args.limit === "number" ? args.limit : undefined,
  };
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
  const def = React.useMemo(() => chartDefFromArgs(JSON.parse(argsKey)), [argsKey]);
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
            year="all"
          />
        ) : (
          <div className="h-full animate-pulse rounded-lg bg-muted/60" />
        )}
      </div>

      <div className="flex flex-wrap items-center gap-x-2 gap-y-1 border-t border-border px-3 py-2 text-muted-foreground">
        {def.kind ? <span className="rounded bg-muted px-1.5 py-0.5 font-mono text-[10px]">{def.kind}</span> : null}
        {def.mart ? <span className="font-mono text-[10px]">{def.mart}</span> : null}
        {def.dimension ? <span>by {def.dimension}</span> : null}
        {def.measures?.length ? (
          <span>· {def.aggregate ?? "sum"}({def.measures.join(", ")})</span>
        ) : null}
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
