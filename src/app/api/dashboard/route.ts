import { NextResponse } from "next/server";
import { chQuery } from "@/services/clients/clickhouse";
import { SPEC_SQL } from "@/lib/dashboard-specs";

export const dynamic = "force-dynamic";

type CellResult =
  | { columns: string[]; rows: Record<string, unknown>[] }
  | { error: string };

/**
 * Data untuk semua kartu dashboard. Menjalankan SQL tiap spec (dari semantic
 * layer `dashboard-specs`) PARALEL ke ClickHouse dan mengembalikan hasil
 * ter-key per id. SQL bukan dari input pengguna — hanya spec tetap kita —
 * dan dieksekusi oleh akun read-only.
 */
export async function GET(req: Request) {
  const entries = Object.entries(SPEC_SQL);
  const settled = await Promise.all(
    entries.map(async ([id, sql]): Promise<[string, CellResult]> => {
      try {
        const r = await chQuery(sql, req.signal);
        return [
          id,
          { columns: r.meta.map((m) => m.name), rows: r.data },
        ];
      } catch (e) {
        return [id, { error: e instanceof Error ? e.message : String(e) }];
      }
    }),
  );
  return NextResponse.json({ results: Object.fromEntries(settled) });
}
