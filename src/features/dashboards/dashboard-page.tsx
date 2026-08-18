"use client";

import * as React from "react";
import { useTheme } from "next-themes";
import { RefreshCw } from "lucide-react";
import { PageHeader } from "@/components/patterns/page-header";
import { SectionCard } from "@/components/patterns/section-card";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import { KPIS, CHARTS } from "@/lib/dashboard-specs";
import { EChart } from "./echart";
import { buildOption, fmtInt } from "./chart-option";

type Cell =
  | { columns: string[]; rows: Record<string, unknown>[] }
  | { error: string };
type Results = Record<string, Cell>;

function hasRows(c: Cell | undefined): c is { columns: string[]; rows: Record<string, unknown>[] } {
  return !!c && "rows" in c;
}

/**
 * Dashboard visual lakehouse — "Tableau internal". Semua kartu digerakkan
 * semantic layer (dashboard-specs); data ditarik dari /api/dashboard yang
 * query mart Gold di ClickHouse. Chart pakai Apache ECharts (Apache-2.0).
 */
export function DashboardPage() {
  const { resolvedTheme } = useTheme();
  const dark = resolvedTheme === "dark";
  const [data, setData] = React.useState<Results | null>(null);
  const [loading, setLoading] = React.useState(true);
  const [error, setError] = React.useState<string | null>(null);

  const load = React.useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const res = await fetch("/api/dashboard", { cache: "no-store" });
      const json = await res.json();
      if (!res.ok) throw new Error(json?.error ?? "Gagal memuat dashboard");
      setData(json.results as Results);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }, []);

  React.useEffect(() => {
    void load();
  }, [load]);

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Dashboards"
        description="Visualisasi lakehouse dari mart Gold (serving.*). Kartu digerakkan semantic layer metrics-as-code; grafik oleh Apache ECharts."
        actions={
          <Button variant="outline" size="sm" onClick={() => void load()} disabled={loading}>
            <RefreshCw className={cn("size-4", loading && "animate-spin")} />
            Segarkan
          </Button>
        }
      />

      {error ? (
        <div className="rounded-lg border border-destructive/40 bg-destructive/5 px-4 py-3 text-sm text-destructive">
          {error}
        </div>
      ) : null}

      {/* KPI row */}
      <div className="grid grid-cols-2 gap-3 lg:grid-cols-4">
        {KPIS.map((k) => {
          const cell = data?.[k.id];
          const v = hasRows(cell) ? Number(cell.rows[0]?.v ?? 0) : null;
          return (
            <div key={k.id} className="rounded-xl border bg-card p-4 shadow-[0px_1px_2px_0px_rgba(0,0,0,0.05)]">
              <p className="text-xs font-medium text-muted-foreground">{k.title}</p>
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
        {CHARTS.map((spec) => {
          const cell = data?.[spec.id];
          const full = spec.span === 2;
          return (
            <SectionCard
              key={spec.id}
              title={spec.title}
              description={spec.subtitle}
              className={cn(full && "lg:col-span-2")}
              contentClassName="pt-1"
              action={
                <span className="rounded-full border px-2 py-0.5 font-mono text-[10px] text-muted-foreground">
                  {spec.mart}
                </span>
              }
            >
              {loading && !cell ? (
                <div className="h-[280px] animate-pulse rounded-lg bg-muted/50" />
              ) : cell && "error" in cell ? (
                <p className="py-8 text-center text-sm text-destructive">{cell.error}</p>
              ) : hasRows(cell) && cell.rows.length ? (
                <EChart
                  option={buildOption(spec, cell.rows, dark)}
                  height={full ? 320 : 280}
                />
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
