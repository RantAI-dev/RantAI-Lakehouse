import { NextResponse } from "next/server";
import { chRows } from "@/services/clients/clickhouse";

export const dynamic = "force-dynamic";

/**
 * Nilai distinct sebuah kolom (untuk dropdown filter dashboard). Kolom bisa ada
 * di beberapa mart Gold — ambil dari mart mana pun yang punya, gabungkan.
 */
export async function GET(req: Request) {
  const column = (new URL(req.url).searchParams.get("column") || "").replace(/[^a-zA-Z0-9_]/g, "");
  if (!column) return NextResponse.json({ error: "column wajib" }, { status: 400 });
  try {
    const marts = await chRows<{ table: string }>(
      `SELECT table FROM system.columns WHERE database='serving' AND name='${column}' AND table NOT LIKE '%\\_baru'`,
    );
    if (marts.length === 0) return NextResponse.json({ column, values: [] });
    const union = marts.map((m) => `SELECT DISTINCT toString(${column}) AS v FROM serving.${m.table.replace(/[^a-zA-Z0-9_]/g, "")}`).join(" UNION DISTINCT ");
    const rows = await chRows<{ v: string }>(`SELECT v FROM (${union}) WHERE v != '' ORDER BY v LIMIT 200`);
    return NextResponse.json({ column, values: rows.map((r) => r.v) });
  } catch (e) {
    return NextResponse.json({ error: e instanceof Error ? e.message : String(e) }, { status: 500 });
  }
}
