import { NextResponse } from "next/server";
import { chQuery, chRows } from "@/services/clients/clickhouse";

export const dynamic = "force-dynamic";

const prettify = (s: string) =>
  s.replace(/_/g, " ").replace(/\b\w/g, (m) => m.toUpperCase());

/**
 * Detail aset Silver/Gold — objek ClickHouse langsung (silver.* / serving.*),
 * bukan registry Bronze. Skema via DESCRIBE, contoh via SELECT LIMIT.
 */
async function clickhouseAssetDetail(id: string) {
  const [db, ...rest] = id.split(".");
  const table = rest.join(".").replace(/[^a-zA-Z0-9_]/g, "");
  if ((db !== "silver" && db !== "serving") || !table) {
    return NextResponse.json({ error: "Aset tidak ditemukan" }, { status: 404 });
  }
  const isGold = db === "serving";

  let schema: { name: string; dataType: string; description?: string }[] = [];
  let sample: Record<string, string>[] = [];
  try {
    const r = await chQuery(`SELECT * FROM ${db}.\`${table}\` LIMIT 5`);
    schema = r.meta
      .filter((m) => !m.name.startsWith("_"))
      .map((m) => ({ name: m.name, dataType: m.type }));
    sample = r.data.map((row) => {
      const o: Record<string, string> = {};
      for (const m of r.meta) if (!m.name.startsWith("_")) o[m.name] = String(row[m.name] ?? "");
      return o;
    });
  } catch {
    return NextResponse.json({ error: "Aset tidak ditemukan" }, { status: 404 });
  }

  let rows = 0;
  try {
    const rr = await chRows<{ r: string }>(
      `SELECT toString(sum(rows)) r FROM system.parts WHERE database='${db}' AND table='${table}' AND active`,
    );
    rows = Number(rr[0]?.r) || 0;
  } catch {
    /* view: tak ada parts */
  }

  return NextResponse.json({
    id,
    name: prettify(table),
    namespace: db,
    type: isGold ? "table" : "view",
    layer: isGold ? "gold" : "silver",
    tier: isGold ? "hot" : "warm",
    classification: "internal",
    owner: "Dinas Pariwisata & Ekraf DKI Jakarta",
    domain: "pariwisata",
    description: isGold
      ? "Mart Gold penyaji dashboard (agregat siap pakai)."
      : "Model Silver terkurasi (bersih & terkonform) di ClickHouse.",
    format: isGold ? "ClickHouse MergeTree" : "ClickHouse View",
    engine: "hot-store",
    rows,
    sizeBytes: rows * 220,
    columnCount: schema.length,
    freshnessLagSeconds: 0,
    lastUpdated: "",
    health: schema.length > 0 ? "healthy" : "degraded",
    residency: "id-jakarta",
    schema,
    sample,
    qualityChecks: [],
    policySummary: [],
    usage: { queries7d: 0, users7d: 0, avgLatencyMs: 0 },
    recentQueries: [],
    dependents: [],
    changeHistory: [],
    snapshots: [],
    schemaVersions: [],
    upstream: [],
    downstream: [],
    lifecyclePolicy: "default",
  });
}

/** Detail satu aset (dataset) NYATA: metadata + schema + sample dari Silver. */
export async function GET(
  _req: Request,
  { params }: { params: Promise<{ id: string }> },
) {
  const { id } = await params;
  try {
    // ── Silver / Gold: aset ClickHouse langsung (bukan registry Bronze) ─────
    if (id.startsWith("silver.") || id.startsWith("serving.")) {
      return await clickhouseAssetDetail(id);
    }

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
    const BRONZE_CURATED = new Set([
      "wisman-jakarta-per-bulan",
      "wisman-jakarta-per-negara",
      "wisman-jakarta-per-pintu-masuk",
      "jumlah-pengunjung-event-2026",
    ]);
    return NextResponse.json({
      id: sync.slug, name: sync.title, namespace: sekunder ? "sekunder" : "sdi-primer",
      type: "iceberg-table", layer: BRONZE_CURATED.has(sync.slug) ? "bronze" : "raw", tier: "warm", classification: "internal",
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
