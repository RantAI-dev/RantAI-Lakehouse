"use client";

import * as React from "react";
import { useTheme } from "next-themes";
import { RefreshCw, Trash2, Sparkles } from "lucide-react";
import { PageHeader } from "@/components/patterns/page-header";
import { SectionCard } from "@/components/patterns/section-card";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import type { ChartRenderSpec, ChartSource } from "@/lib/dashboard-specs";
import { EChart } from "./echart";
import { buildOption, fmtInt } from "./chart-option";
import { ChartBuilder } from "./chart-builder";

type Cell =
  | { columns: string[]; rows: Record<string, unknown>[] }
  | { error: string };
type KpiMeta = { id: string; title: string; caption?: string; format: string };
type Payload = { kpis: KpiMeta[]; charts: ChartRenderSpec[]; results: Record<string, Cell>; storeError?: string | null };

function hasRows(c: Cell | undefined): c is { columns: string[]; rows: Record<string, unknown>[] } {
  return !!c && "rows" in c;
}

const SOURCE_BADGE: Record<ChartSource, { label: string; cls: string } | null> = {
  builtin: null,
  ai: { label: "AI", cls: "bg-violet-500/15 text-violet-600 dark:text-violet-400" },
  ui: { label: "Manual", cls: "bg-sky-500/15 text-sky-600 dark:text-sky-400" },
};

/**
 * Dashboard visual lakehouse — "BI lakehouse". Kartu digerakkan semantic layer:
 * spec BAWAAN + spec TERSIMPAN (dibuat lewat chat AI atau builder manual, dari
 * console.bi_chart). Dua jalur menulis artefak yang sama, jadi selalu sinkron.
 */
export function DashboardPage() {
  const { resolvedTheme } = useTheme();
  const dark = resolvedTheme === "dark";
  const [data, setData] = React.useState<Payload | null>(null);
  const [loading, setLoading] = React.useState(true);
  const [error, setError] = React.useState<string | null>(null);

  const load = React.useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const res = await fetch("/api/dashboard", { cache: "no-store" });
      const json = (await res.json()) as Payload;
      if (!res.ok) throw new Error((json as { error?: string }).error ?? "Gagal memuat dashboard");
      setData(json);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }, []);

  React.useEffect(() => {
    void load();
  }, [load]);

  async function remove(id: string) {
    await fetch(`/api/dashboard/specs?id=${encodeURIComponent(id)}`, { method: "DELETE" });
    void load();
  }

  const kpis = data?.kpis ?? [];
  const charts = data?.charts ?? [];

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Dashboards"
        description="Visualisasi lakehouse dari mart Gold (serving.*). Kartu dari semantic layer — bikin manual di sini atau lewat AI Copilot (chat). Grafik oleh Apache ECharts."
        actions={
          <>
            <ChartBuilder onCreated={load} />
            <Button variant="outline" size="sm" onClick={() => void load()} disabled={loading}>
              <RefreshCw className={cn("size-4", loading && "animate-spin")} />
              Segarkan
            </Button>
          </>
        }
      />

      {error ? (
        <div className="rounded-lg border border-destructive/40 bg-destructive/5 px-4 py-3 text-sm text-destructive">
          {error}
        </div>
      ) : null}
      {data?.storeError ? (
        <div className="rounded-lg border border-amber-500/40 bg-amber-500/5 px-4 py-2 text-xs text-amber-600 dark:text-amber-400">
          Chart tersimpan tak bisa dimuat: {data.storeError}
        </div>
      ) : null}

      {/* KPI row */}
      <div className="grid grid-cols-2 gap-3 lg:grid-cols-4">
        {(kpis.length ? kpis : Array.from({ length: 4 }, (_, i) => ({ id: `s${i}`, title: "", format: "int" } as KpiMeta))).map((k) => {
          const cell = data?.results[k.id];
          const v = hasRows(cell) ? Number(cell.rows[0]?.v ?? 0) : null;
          return (
            <div key={k.id} className="rounded-xl border bg-card p-4 shadow-[0px_1px_2px_0px_rgba(0,0,0,0.05)]">
              <p className="text-xs font-medium text-muted-foreground">{k.title || "—"}</p>
              {loading && v === null ? (
                <div className="mt-2 h-7 w-24 animate-pulse rounded bg-muted" />
              ) : (
                <p className="mt-1 text-2xl font-semibold tabular-nums text-foreground">
                  {v === null ? "—" : fmtInt(v)}
                </p>
              )}
              {k.caption ? <p className="mt-1 text-[11px] text-muted-foreground">{k.caption}</p> : null}
            </div>
          );
        })}
      </div>

      {/* Charts grid */}
      <div className="grid grid-cols-1 gap-3 lg:grid-cols-2">
        {charts.map((spec) => {
          const cell = data?.results[spec.id];
          const full = spec.span === 2;
          const badge = SOURCE_BADGE[spec.source];
          return (
            <SectionCard
              key={spec.id}
              title={spec.title}
              description={spec.subtitle}
              className={cn(full && "lg:col-span-2")}
              contentClassName="pt-1"
              action={
                <div className="flex items-center gap-1.5">
                  {badge ? (
                    <span className={cn("inline-flex items-center gap-1 rounded-full px-2 py-0.5 text-[10px] font-medium", badge.cls)}>
                      {spec.source === "ai" ? <Sparkles className="size-2.5" /> : null}
                      {badge.label}
                    </span>
                  ) : null}
                  <span className="rounded-full border px-2 py-0.5 font-mono text-[10px] text-muted-foreground">
                    {spec.mart}
                  </span>
                  {spec.source !== "builtin" ? (
                    <button
                      type="button"
                      onClick={() => void remove(spec.id)}
                      aria-label="Hapus chart"
                      className="rounded p-1 text-muted-foreground hover:bg-destructive/10 hover:text-destructive"
                    >
                      <Trash2 className="size-3.5" />
                    </button>
                  ) : null}
                </div>
              }
            >
              {loading && !cell ? (
                <div className="h-[280px] animate-pulse rounded-lg bg-muted/50" />
              ) : cell && "error" in cell ? (
                <p className="py-8 text-center text-sm text-destructive">{cell.error}</p>
              ) : hasRows(cell) && cell.rows.length ? (
                <EChart option={buildOption(spec, cell.rows, dark)} height={full ? 320 : 280} />
              ) : (
                <p className="py-8 text-center text-sm text-muted-foreground">Tak ada data.</p>
              )}
            </SectionCard>
          );
        })}
      </div>
    </div>
  );
}
