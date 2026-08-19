import { NextResponse } from "next/server";
import { chQuery, chRows } from "@/services/clients/clickhouse";
import { toRenderSpec } from "@/lib/dashboard-specs";
import { getBoardByToken, listStoredCharts, sqlWithFilters, type StoredChartSpec } from "@/services/clients/bi-store";

export const dynamic = "force-dynamic";

type CellResult =
  | { columns: string[]; rows: Record<string, unknown>[] }
  | { error: string };

async function runSpec(id: string, sql: string, signal: AbortSignal): Promise<[string, CellResult]> {
  if (!sql) return [id, { columns: [], rows: [] }];
  try {
    const r = await chQuery(sql, signal);
    return [id, { columns: r.meta.map((m) => m.name), rows: r.data }];
  } catch (e) {
    return [id, { error: e instanceof Error ? e.message : String(e) }];
  }
}

/**
 * View PUBLIK read-only sebuah dashboard lewat token share. TANPA auth.
 * Hanya menyajikan tile milik board itu + hasilnya, memakai FILTER TERSIMPAN
 * board (tidak menerima override dari query publik - aman dari injeksi). Data
 * berasal dari mart Gold serving.* yang memang sudah agregat.
 */
export async function GET(req: Request, { params }: { params: Promise<{ token: string }> }) {
  const { token } = await params;
  let board: Awaited<ReturnType<typeof getBoardByToken>> = null;
  try {
    board = await getBoardByToken(token);
  } catch (e) {
    return NextResponse.json({ error: e instanceof Error ? e.message : String(e) }, { status: 500 });
  }
  if (!board) return NextResponse.json({ error: "not_found" }, { status: 404 });

  let stored: StoredChartSpec[] = [];
  try {
    stored = (await listStoredCharts()).filter((c) => (c.board || "default") === board!.id);
  } catch (e) {
    return NextResponse.json({ error: e instanceof Error ? e.message : String(e) }, { status: 500 });
  }

  const filters = board.filters ?? [];
  const needCols = filters.some((f) => f.values?.length);
  let cols = new Map<string, Set<string>>();
  if (needCols) {
    const rows = await chRows<{ table: string; name: string }>(
      "SELECT table, name FROM system.columns WHERE database='serving'", req.signal,
    );
    cols = new Map<string, Set<string>>();
    for (const r of rows) { if (!cols.has(r.table)) cols.set(r.table, new Set()); cols.get(r.table)!.add(r.name); }
  }

  const settled = await Promise.all(
    stored.map((c) => runSpec(c.id, sqlWithFilters(c, [], filters, cols), req.signal)),
  );

  return NextResponse.json({
    board: { id: board.id, name: board.name },
    layout: board.layout ?? {},
    charts: stored.map((c) => ({ ...toRenderSpec(c, c.source), board: c.board, def: c.def })),
    results: Object.fromEntries(settled),
  });
}
