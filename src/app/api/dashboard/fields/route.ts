import { NextResponse } from "next/server";
import { chRows } from "@/services/clients/clickhouse";

export const dynamic = "force-dynamic";

const NUMERIC = /Int|Float|Decimal/;

/**
 * Sumber pilihan untuk UI builder chart:
 *  - tanpa ?mart  → daftar mart Gold (serving.*, non-staging).
 *  - dengan ?mart → kolom mart itu, dipisah dimensi (kategori/waktu) vs measure
 *    (numerik). Semantic layer minimal supaya orang bikin chart tanpa nulis SQL.
 */
export async function GET(req: Request) {
  const mart = new URL(req.url).searchParams.get("mart");
  try {
    if (!mart) {
      const rows = await chRows<{ name: string; total_rows: string }>(
        `SELECT name, toString(total_rows) AS total_rows FROM system.tables
          WHERE database='serving' AND name NOT LIKE '%\\_baru' ORDER BY name`,
      );
      return NextResponse.json({ marts: rows.map((r) => ({ name: r.name, rows: Number(r.total_rows) })) });
    }
    const safe = mart.replace(/[^a-zA-Z0-9_]/g, "");
    const cols = await chRows<{ name: string; type: string }>(
      `SELECT name, type FROM system.columns
        WHERE database='serving' AND table='${safe}' ORDER BY position`,
    );
    const dimensions = cols.filter((c) => !NUMERIC.test(c.type)).map((c) => c.name);
    const measures = cols.filter((c) => NUMERIC.test(c.type)).map((c) => c.name);
    return NextResponse.json({ mart: safe, dimensions, measures, columns: cols });
  } catch (e) {
    return NextResponse.json({ error: e instanceof Error ? e.message : String(e) }, { status: 500 });
  }
}
