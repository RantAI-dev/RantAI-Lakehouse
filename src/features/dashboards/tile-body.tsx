"use client";

import * as React from "react";
import type { ChartRenderSpec } from "@/lib/dashboard-specs";
import { MiniMarkdown } from "@/features/copilot/mini-markdown";
import { EChart } from "./echart";
import { buildOption, fmtInt } from "./chart-option";
import { ensureMap } from "./echarts-maps";

type Cell = { columns: string[]; rows: Record<string, unknown>[] } | { error: string };
function hasRows(c: Cell | undefined): c is { columns: string[]; rows: Record<string, unknown>[] } {
  return !!c && "rows" in c;
}

/** Isi sebuah tile sesuai kind: text / kpi / table / chart. */
export function TileBody({
  spec, cell, dark, loading, year, onDataClick,
}: {
  spec: ChartRenderSpec & { text?: string; caption?: string };
  cell: Cell | undefined;
  dark: boolean;
  loading: boolean;
  year: string;
  /** Klik titik data (bar/irisan/titik) → drill/cross-filter. */
  onDataClick?: (name: string, pos: { x: number; y: number }) => void;
}) {
  if (spec.kind === "text") {
    return <div className="h-full overflow-auto px-1 py-0.5 text-sm leading-relaxed"><MiniMarkdown text={spec.text ?? ""} /></div>;
  }
  if (cell && "error" in cell) {
    return <p className="grid h-full place-items-center px-2 text-center text-xs text-destructive">{cell.error}</p>;
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
      return <p className="grid h-full place-items-center text-xs text-muted-foreground">Tak ada data{year !== "all" ? ` (tahun ${year})` : ""}.</p>;
    }
    return <TableView columns={cell.columns} rows={cell.rows} />;
  }

  // geomap — perlu registrasi peta dulu (GeoJSON lokal).
  if (spec.kind === "geomap") {
    if (!hasRows(cell) || cell.rows.length === 0) {
      return <p className="grid h-full place-items-center text-xs text-muted-foreground">Tak ada data{year !== "all" ? ` (tahun ${year})` : ""}.</p>;
    }
    return <GeoChart spec={spec} rows={cell.rows} dark={dark} />;
  }

  // chart
  if (hasRows(cell) && cell.rows.length) return <EChart option={buildOption(spec, cell.rows, dark)} height="100%" onDataClick={onDataClick} />;
  return <p className="grid h-full place-items-center text-xs text-muted-foreground">Tak ada data{year !== "all" ? ` (tahun ${year})` : ""}.</p>;
}

/** Choropleth — pastikan peta terdaftar sebelum render; pesan ramah bila GeoJSON belum ada. */
function GeoChart({ spec, rows, dark }: { spec: ChartRenderSpec; rows: Record<string, unknown>[]; dark: boolean }) {
  const [state, setState] = React.useState<"loading" | "ready" | "missing">("loading");
  React.useEffect(() => {
    let alive = true;
    void ensureMap().then((ok) => { if (alive) setState(ok ? "ready" : "missing"); });
    return () => { alive = false; };
  }, []);
  if (state === "loading") return <div className="h-full animate-pulse rounded bg-muted/40" />;
  if (state === "missing") {
    return (
      <div className="grid h-full place-content-center px-4 text-center text-xs text-muted-foreground">
        <p className="font-medium text-foreground">Map data not loaded</p>
        <p className="mt-1">Add <code className="rounded bg-muted px-1 font-mono">public/geo/dki-jakarta.geojson</code> to enable the map.</p>
      </div>
    );
  }
  return <EChart option={buildOption(spec, rows, dark)} height="100%" />;
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
