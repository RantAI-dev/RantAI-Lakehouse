import { NextResponse } from "next/server";
import { chQuery } from "@/services/clients/clickhouse";

export const dynamic = "force-dynamic";
export const maxDuration = 60;

/**
 * Eksekusi SQL NYATA di ClickHouse (lakehouse kita) dan kembalikan bentuk
 * QueryResult. Dipanggil oleh services/clients/queries.ts (bukan langsung UI).
 */
export async function POST(req: Request) {
  let sql = "";
  try {
    ({ sql } = await req.json());
  } catch {
    return NextResponse.json({ error: "Body harus JSON {sql}" }, { status: 400 });
  }
  if (!sql || typeof sql !== "string") {
    return NextResponse.json({ error: "sql wajib diisi" }, { status: 400 });
  }
  // Guard read-only: Query Studio hanya untuk SELECT/WITH/SHOW/DESCRIBE/EXPLAIN.
  if (!/^\s*(with|select|show|describe|desc|explain)\b/i.test(sql) ||
      /\b(insert|alter|drop|delete|update|create|truncate|rename|attach|detach|grant|revoke)\b/i.test(sql)) {
    return NextResponse.json(
      { error: "Hanya query baca (SELECT/SHOW/DESCRIBE/EXPLAIN) yang diizinkan di Query Studio." },
      { status: 422 },
    );
  }

  const started = Date.now();
  try {
    const r = await chQuery(sql);
    const columns = r.meta.map((m) => m.name);
    const rows = r.data.map((row) => {
      const out: Record<string, string> = {};
      for (const c of columns) {
        const v = row[c];
        out[c] = v === null || v === undefined ? "" : String(v);
      }
      return out;
    });
    const scannedBytes = r.statistics?.bytes_read ?? 0;
    const durationMs = r.statistics?.elapsed
      ? Math.round(r.statistics.elapsed * 1000)
      : Date.now() - started;

    return NextResponse.json({
      id: `q-${started}`,
      columns,
      rows,
      metrics: {
        durationMs,
        scannedBytes,
        costUnits: Math.max(1, Math.round(scannedBytes / 1_000_000)), // ~1 unit / MB terbaca
        engine: "hot-store",
        workloadClass: "hot-analytics",
        cacheHit: false,
        pushdowns: [],
        policyObligations: [],
      },
      plan: [
        {
          id: "s1",
          label: "ClickHouse (Hot analytical store)",
          location: "clickhouse@lakehouse",
          operation: "scan + aggregate",
          estimatedBytes: scannedBytes,
          status: "completed",
        },
      ],
    });
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e);
    return NextResponse.json({ error: msg }, { status: 422 });
  }
}
