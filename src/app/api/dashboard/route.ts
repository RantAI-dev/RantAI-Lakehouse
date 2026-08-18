import { NextResponse } from "next/server";
import { chQuery } from "@/services/clients/clickhouse";
import { KPIS, CHARTS, toRenderSpec } from "@/lib/dashboard-specs";
import { listStoredCharts, type StoredChartSpec } from "@/services/clients/bi-store";

export const dynamic = "force-dynamic";

type CellResult =
  | { columns: string[]; rows: Record<string, unknown>[] }
  | { error: string };

async function runSpec(id: string, sql: string, signal: AbortSignal): Promise<[string, CellResult]> {
  try {
    const r = await chQuery(sql, signal);
    return [id, { columns: r.meta.map((m) => m.name), rows: r.data }];
  } catch (e) {
    return [id, { error: e instanceof Error ? e.message : String(e) }];
  }
}

/**
 * Data + daftar kartu dashboard. Menggabungkan spec BAWAAN (seed) dengan spec
 * TERSIMPAN (dibuat lewat chat/UI, dari console.bi_chart), menjalankan SQL tiap
 * spec PARALEL ke ClickHouse (akun read-only), lalu mengembalikan:
 *  - kpis   : metadata KPI (render)
 *  - charts : metadata chart (render, tanpa SQL) — builtin + tersimpan
 *  - results: hasil per id
 * Dengan begini chart baru dari AI/UI langsung muncul tanpa ubah kode.
 */
export async function GET(req: Request) {
  let stored: StoredChartSpec[] = [];
  let storeError: string | null = null;
  try {
    stored = await listStoredCharts();
  } catch (e) {
    storeError = e instanceof Error ? e.message : String(e);
  }

  const charts = [...CHARTS, ...stored];
  const jobs: Promise<[string, CellResult]>[] = [
    ...KPIS.map((k) => runSpec(k.id, k.sql, req.signal)),
    ...charts.map((c) => runSpec(c.id, c.sql, req.signal)),
  ];
  const settled = await Promise.all(jobs);

  return NextResponse.json({
    kpis: KPIS.map((k) => ({ id: k.id, title: k.title, caption: k.caption, format: k.format })),
    charts: [
      ...CHARTS.map((c) => toRenderSpec(c, "builtin")),
      ...stored.map((c) => toRenderSpec(c, c.source)),
    ],
    results: Object.fromEntries(settled),
    storeError,
  });
}
