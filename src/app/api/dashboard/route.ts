import { NextResponse } from "next/server";
import { chQuery, chRows } from "@/services/clients/clickhouse";
import { KPIS, CHARTS, toRenderSpec } from "@/lib/dashboard-specs";
import { listStoredCharts, listBoards, sqlWithFilters, type StoredChartSpec, type FilterDef } from "@/services/clients/bi-store";

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

/** Peta mart → set kolomnya (untuk tahu filter mana berlaku ke tile mana). */
async function martColumns(signal: AbortSignal): Promise<Map<string, Set<string>>> {
  const rows = await chRows<{ table: string; name: string }>(
    "SELECT table, name FROM system.columns WHERE database='serving'", signal,
  );
  const m = new Map<string, Set<string>>();
  for (const r of rows) {
    if (!m.has(r.table)) m.set(r.table, new Set());
    m.get(r.table)!.add(r.name);
  }
  return m;
}

/**
 * Data + daftar tile dashboard. Menggabungkan spec BAWAAN + TERSIMPAN, filter
 * TAHUN & FILTER DIMENSI (lintas-tile) disuntik ke SQL tiap tile yang punya
 * kolomnya. Mendukung ?board, ?year=2024,2025, ?filters=<json>.
 */
export async function GET(req: Request) {
  const url = new URL(req.url);
  const board = url.searchParams.get("board") || "default";
  const years = (url.searchParams.get("year") || "")
    .split(",").map((s) => parseInt(s, 10)).filter((n) => Number.isFinite(n));
  let paramFilters: FilterDef[] | null = null;
  try {
    const f = url.searchParams.get("filters");
    if (f) { const p = JSON.parse(f); if (Array.isArray(p)) paramFilters = p as FilterDef[]; }
  } catch { /* abaikan */ }

  let stored: StoredChartSpec[] = [];
  let boards: Awaited<ReturnType<typeof listBoards>> = [];
  let storeError: string | null = null;
  try {
    [stored, boards] = await Promise.all([listStoredCharts(), listBoards()]);
  } catch (e) {
    storeError = e instanceof Error ? e.message : String(e);
  }
  const boardObj = boards.find((b) => b.id === board);
  const layout = boardObj?.layout ?? {};
  const filters = paramFilters ?? boardObj?.filters ?? [];

  const onDefault = board === "default" || board === "all";
  const storedForBoard = board === "all" ? stored : stored.filter((c) => (c.board || "default") === board);
  const builtinCharts = onDefault ? CHARTS : [];
  const kpis = onDefault ? KPIS : [];

  // Kolom mart (hanya bila ada filter/tahun aktif — untuk tahu apa berlaku).
  const needCols = years.length > 0 || filters.some((f) => f.values?.length);
  const cols = needCols ? await martColumns(req.signal) : new Map<string, Set<string>>();

  const jobs: Promise<[string, CellResult]>[] = [
    ...kpis.map((k) => runSpec(k.id, k.sql, req.signal)),
    ...builtinCharts.map((c) => runSpec(c.id, c.sql, req.signal)),
    ...storedForBoard.map((c) => runSpec(c.id, sqlWithFilters(c, years, filters, cols), req.signal)),
  ];
  const settled = await Promise.all(jobs);

  // Kolom dimensi yang bisa dijadikan filter (union dari tile board).
  const filterColumns = Array.from(new Set(storedForBoard.map((c) => c.def?.dimension).filter(Boolean))) as string[];

  return NextResponse.json({
    board,
    years,
    layout,
    filters,
    filterColumns,
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
