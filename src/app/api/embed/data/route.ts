import { NextResponse } from "next/server";
import { chQuery, chRows } from "@/services/clients/clickhouse";
import { toRenderSpec } from "@/lib/dashboard-specs";
import { getBoard, listStoredCharts, sqlWithFilters, type FilterDef, type StoredChartSpec } from "@/services/clients/bi-store";
import { getEmbedSecret, verifyEmbed } from "@/services/clients/embed-jwt";

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

/** params JWT { col: val | [vals] } → FilterDef[] (dikunci, viewer tak bisa ubah). */
function paramsToFilters(params: Record<string, string | string[]> | undefined): FilterDef[] {
  if (!params) return [];
  return Object.entries(params).map(([column, v]) => ({
    column, values: (Array.isArray(v) ? v : [v]).map(String),
  }));
}

/**
 * SIGNED EMBED (ala Metabase): body { jwt }. Server memverifikasi JWT dengan
 * EMBEDDING SECRET, memastikan board mengizinkan embed, lalu menyajikan data
 * dengan FILTER TERKUNCI dari klaim params (viewer tak dapat mengubahnya).
 */
export async function POST(req: Request) {
  let jwt = "";
  try { jwt = String((await req.json())?.jwt ?? ""); } catch { /* ignore */ }
  if (!jwt) return NextResponse.json({ error: "jwt wajib" }, { status: 400 });

  const secret = await getEmbedSecret();
  const claims = verifyEmbed(jwt, secret);
  if (!claims) return NextResponse.json({ error: "invalid_or_expired" }, { status: 401 });

  const boardId = claims.resource?.dashboard;
  if (!boardId) return NextResponse.json({ error: "no_resource" }, { status: 400 });

  const board = await getBoard(boardId);
  if (!board || !board.embedEnabled) return NextResponse.json({ error: "embedding_disabled" }, { status: 403 });

  const stored: StoredChartSpec[] = (await listStoredCharts()).filter((c) => (c.board || "default") === boardId);

  // Filter tersimpan board + params terkunci JWT.
  const filters = [...(board.filters ?? []), ...paramsToFilters(claims.params)];
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
    board: { id: boardId, name: board.name },
    layout: board.layout ?? {},
    charts: stored.map((c) => ({ ...toRenderSpec(c, c.source), board: c.board, def: c.def })),
    results: Object.fromEntries(settled),
  });
}
