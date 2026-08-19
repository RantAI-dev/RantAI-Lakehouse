import { NextResponse } from "next/server";
import { chQuery } from "@/services/clients/clickhouse";
import { KPIS, CHARTS, toRenderSpec } from "@/lib/dashboard-specs";
import { listStoredCharts, listBoards, sqlWithYear, type StoredChartSpec } from "@/services/clients/bi-store";

export const dynamic = "force-dynamic";

type CellResult =
  | { columns: string[]; rows: Record<string, unknown>[] }
  | { error: string };

async function runSpec(id: string, sql: string, signal: AbortSignal): Promise<[string, CellResult]> {
  if (!sql) return [id, { columns: [], rows: [] }]; // tile text — tak perlu query
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
 * spec PARALEL ke ClickHouse (read-only). Mendukung:
 *  - ?board=<id>  → hanya kartu board itu (default = board bawaan + seed).
 *  - ?year=2024,2025 → filter tahun untuk kartu tersimpan yang punya kolom tahun.
 */
export async function GET(req: Request) {
  const url = new URL(req.url);
  const board = url.searchParams.get("board") || "default";
  const years = (url.searchParams.get("year") || "")
    .split(",").map((s) => parseInt(s, 10)).filter((n) => Number.isFinite(n));

  let stored: StoredChartSpec[] = [];
  let boards: Awaited<ReturnType<typeof listBoards>> = [];
  let storeError: string | null = null;
  try {
    [stored, boards] = await Promise.all([listStoredCharts(), listBoards()]);
  } catch (e) {
    storeError = e instanceof Error ? e.message : String(e);
  }
  const layout = boards.find((b) => b.id === board)?.layout ?? {};

  const onDefault = board === "default" || board === "all";
  const storedForBoard = board === "all" ? stored : stored.filter((c) => (c.board || "default") === board);
  // Seed + KPI hanya tampil di board bawaan/semua.
  const builtinCharts = onDefault ? CHARTS : [];
  const kpis = onDefault ? KPIS : [];

  const jobs: Promise<[string, CellResult]>[] = [
    ...kpis.map((k) => runSpec(k.id, k.sql, req.signal)),
    ...builtinCharts.map((c) => runSpec(c.id, c.sql, req.signal)),
    ...storedForBoard.map((c) => runSpec(c.id, sqlWithYear(c, years), req.signal)),
  ];
  const settled = await Promise.all(jobs);

  return NextResponse.json({
    board,
    years,
    layout,
    boards: [{ id: "default", name: "Utama" }, ...boards.map((b) => ({ id: b.id, name: b.name }))],
    kpis: kpis.map((k) => ({ id: k.id, title: k.title, caption: k.caption, format: k.format })),
    charts: [
      ...builtinCharts.map((c) => ({ ...toRenderSpec(c, "builtin"), board: "default" })),
      ...storedForBoard.map((c) => ({ ...toRenderSpec(c, c.source), board: c.board, def: c.def })),
    ],
    results: Object.fromEntries(settled),
    storeError,
  });
}
