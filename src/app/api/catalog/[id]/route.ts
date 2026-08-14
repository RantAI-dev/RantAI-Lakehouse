import { NextResponse } from "next/server";
import { chQuery, chRows } from "@/services/clients/clickhouse";

export const dynamic = "force-dynamic";

/** Detail satu aset (dataset) NYATA: metadata + schema + sample dari Silver. */
export async function GET(
  _req: Request,
  { params }: { params: Promise<{ id: string }> },
) {
  const { id } = await params;
  try {
    const sync = (
      await chRows<Record<string, string>>(
        `SELECT slug, title, description, tier, table_name, toString(total) total,
                author, frekuensi, satuan, klasifikasi, updated_at FROM (
           SELECT slug,title,description,'primer' tier,table_name,total,author,frekuensi,satuan,klasifikasi,'' updated_at FROM lake.\`bronze_meta.dataset_sync\` s
           UNION ALL
           SELECT slug,title,description,'sekunder' tier,table_name,total,author,frekuensi,satuan,klasifikasi,'' updated_at FROM lake.\`bronze_meta_sec.dataset_sync\`
         ) WHERE slug = '${id.replace(/'/g, "")}' LIMIT 1`,
      )
    )[0];
    if (!sync) return NextResponse.json({ error: "Aset tidak ditemukan" }, { status: 404 });

    const cols = await chRows<{ key_asli: string; tipe: string; deskripsi: string }>(
      `SELECT key_asli, tipe, deskripsi FROM lake.\`bronze_meta.dataset_column\` WHERE slug='${id.replace(/'/g, "")}'
       UNION ALL SELECT key_asli, tipe, deskripsi FROM lake.\`bronze_meta_sec.dataset_column\` WHERE slug='${id.replace(/'/g, "")}'`,
    );
    const table = sync.table_name;
    // Sample dari SILVER (processed) bila ada; audit disembunyikan.
    let sample: Record<string, string>[] = [];
    let silverSchema: { name: string; type: string }[] = [];
    try {
      const r = await chQuery(`SELECT * FROM silver.\`${table}\` LIMIT 5`);
      silverSchema = r.meta;
      sample = r.data.map((row) => {
        const o: Record<string, string> = {};
        for (const m of r.meta) if (!m.name.startsWith("_")) o[m.name] = String(row[m.name] ?? "");
        return o;
      });
    } catch {
      /* silver belum ada */
    }
    const typeOf = new Map(silverSchema.map((m) => [m.name, m.type]));
    const schema = cols.map((c) => ({
      name: c.key_asli,
      dataType: typeOf.get(c.key_asli) ?? c.tipe ?? "String",
      description: c.deskripsi || undefined,
    }));

    const rows = Number(sync.total) || 0;
    const sekunder = sync.tier === "sekunder";
    return NextResponse.json({
      id: sync.slug, name: sync.title, namespace: sekunder ? "sekunder" : "sdi-primer",
      type: "iceberg-table", layer: "bronze", tier: "warm", classification: "internal",
      owner: sync.author || "Dinas Pariwisata & Ekraf DKI Jakarta", domain: "pariwisata",
      description: sync.description || "", format: "Apache Iceberg (Parquet)", engine: "hot-store",
      rows, sizeBytes: rows * 220, columnCount: cols.length, freshnessLagSeconds: 0,
      lastUpdated: sync.updated_at || "", health: rows > 0 ? "healthy" : "degraded", residency: "id-jakarta",
      schema, sample,
      qualityChecks: [], policySummary: [], usage: { queries7d: 0, users7d: 0, avgLatencyMs: 0 },
      recentQueries: [], dependents: [], changeHistory: [], snapshots: [], schemaVersions: [],
      upstream: [], downstream: sekunder ? [] : [{ id: `silver.${table}`, name: `silver.${table}` }],
      lifecyclePolicy: "default",
      _meta: { frekuensi: sync.frekuensi, satuan: sync.satuan, klasifikasi: sync.klasifikasi },
    });
  } catch (e) {
    return NextResponse.json({ error: String(e) }, { status: 503 });
  }
}
