import { NextResponse } from "next/server";
import { chQuery } from "@/services/clients/clickhouse";

export const dynamic = "force-dynamic";

/**
 * Estimasi biaya/plan sebuah SQL sebelum dijalankan, memakai EXPLAIN ESTIMATE
 * ClickHouse. Kembalikan bentuk QueryEstimate.
 */
export async function POST(req: Request) {
  let sql = "";
  try {
    ({ sql } = await req.json());
  } catch {
    return NextResponse.json({ error: "Body harus JSON {sql}" }, { status: 400 });
  }
  if (!sql?.trim()) {
    return NextResponse.json({ error: "sql wajib diisi" }, { status: 400 });
  }

  let estimatedBytes = 0;
  const sources: string[] = [];
  try {
    // EXPLAIN ESTIMATE memberi baris/marks/parts per tabel yang tersentuh.
    const r = await chQuery(`EXPLAIN ESTIMATE ${sql.replace(/;\s*$/, "")}`);
    for (const row of r.data) {
      const db = String(row["database"] ?? "");
      const tbl = String(row["table"] ?? "");
      if (tbl) sources.push(db ? `${db}.${tbl}` : tbl);
      estimatedBytes += Number(row["rows"] ?? 0) * 64; // taksiran kasar byte/baris
    }
  } catch {
    // EXPLAIN gagal (mis. bukan SELECT) — estimasi nol, biarkan UI jalan.
  }

  return NextResponse.json({
    estimatedBytes,
    estimatedCostMin: Math.max(1, Math.round(estimatedBytes / 2_000_000)),
    estimatedCostMax: Math.max(1, Math.round(estimatedBytes / 1_000_000)),
    workloadClass: "hot-analytics",
    engine: "hot-store",
    cacheEligible: true,
    freshnessLagSeconds: 0,
    policyObligations: [],
    sources: sources.length ? sources : ["clickhouse@lakehouse"],
    plan: sources.map((s, i) => ({
      id: `p${i}`,
      label: s,
      location: "clickhouse@lakehouse",
      operation: "scan",
      estimatedBytes: 0,
      status: "completed" as const,
    })),
  });
}
