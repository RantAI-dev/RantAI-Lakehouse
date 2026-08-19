import { NextResponse } from "next/server";
import { chQuery, chRows } from "@/services/clients/clickhouse";

export const dynamic = "force-dynamic";

const IDENT = /^[a-zA-Z_][a-zA-Z0-9_]*$/;
const esc = (s: string) => s.replace(/\\/g, "\\\\").replace(/'/g, "''");

/**
 * Drill-down "lihat baris" — baris mentah Gold di balik satu nilai kategori.
 * mart & column DIVALIDASI terhadap skema serving.* (anti-injeksi); value
 * di-escape. Read-only, hanya Gold. GET ?mart=&column=&value=&limit=.
 */
export async function GET(req: Request) {
  const u = new URL(req.url);
  const mart = String(u.searchParams.get("mart") ?? "").replace(/^serving\./, "");
  const column = String(u.searchParams.get("column") ?? "");
  const value = String(u.searchParams.get("value") ?? "");
  const limit = Math.min(Math.max(Number(u.searchParams.get("limit") ?? 50) || 50, 1), 200);
  if (!IDENT.test(mart) || !IDENT.test(column)) {
    return NextResponse.json({ error: "mart/column tidak valid" }, { status: 400 });
  }
  try {
    const cols = new Set(
      (await chRows<{ name: string }>(`SELECT name FROM system.columns WHERE database='serving' AND table='${esc(mart)}'`)).map((c) => c.name),
    );
    if (cols.size === 0) return NextResponse.json({ error: `mart '${mart}' tidak ada` }, { status: 404 });
    if (!cols.has(column)) return NextResponse.json({ error: `kolom '${column}' tidak ada` }, { status: 400 });
    const r = await chQuery(`SELECT * FROM serving.${mart} WHERE ${column} = '${esc(value)}' LIMIT ${limit}`, req.signal);
    return NextResponse.json({ columns: r.meta.map((m) => m.name), rows: r.data, mart, column, value });
  } catch (e) {
    return NextResponse.json({ error: e instanceof Error ? e.message : String(e) }, { status: 500 });
  }
}
