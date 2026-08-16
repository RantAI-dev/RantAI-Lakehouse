import { NextResponse } from "next/server";
import { chRows } from "@/services/clients/clickhouse";

export const dynamic = "force-dynamic";

/**
 * Katalog NYATA: daftar aset data (dataset) dari lakehouse kita —
 * bronze_meta.dataset_catalog (primer SDI) + bronze_meta_sec (sekunder olahan),
 * digabung total baris (dataset_sync) & jumlah kolom (dataset_column).
 * Mengembalikan { assets, namespaces } sesuai kontrak AssetService.
 */
export async function GET() {
  try {
    const cat = await chRows<{
      slug: string; title: string; description: string; tier: string;
      updated_at: string; table_name: string;
    }>(
      `SELECT slug, title, description, tier, updated_at, table_name FROM lake.\`bronze_meta.dataset_catalog\`
       UNION ALL
       SELECT slug, title, description, tier, updated_at, table_name FROM lake.\`bronze_meta_sec.dataset_catalog\``,
    );
    const syncRows = await chRows<{ slug: string; total: string; author: string; frekuensi: string }>(
      `SELECT slug, toString(total) total, author, frekuensi FROM lake.\`bronze_meta.dataset_sync\`
       UNION ALL SELECT slug, toString(total) total, author, frekuensi FROM lake.\`bronze_meta_sec.dataset_sync\``,
    );
    const colRows = await chRows<{ slug: string; n: string }>(
      `SELECT slug, toString(count()) n FROM lake.\`bronze_meta.dataset_column\` GROUP BY slug
       UNION ALL SELECT slug, toString(count()) n FROM lake.\`bronze_meta_sec.dataset_column\` GROUP BY slug`,
    );
    const totalOf = new Map(syncRows.map((s) => [s.slug, Number(s.total) || 0]));
    const authorOf = new Map(syncRows.map((s) => [s.slug, s.author]));
    const colOf = new Map(colRows.map((c) => [c.slug, Number(c.n) || 0]));

    // Landing = RAW; hanya turunan/olahan SDI terkurasi yang naik ke BRONZE.
    // (SDI primer & sekunder eksternal = raw; lihat catatan taksonomi.)
    const BRONZE_CURATED = new Set([
      "wisman-jakarta-per-bulan",
      "wisman-jakarta-per-negara",
      "wisman-jakarta-per-pintu-masuk",
      "jumlah-pengunjung-event-2026",
    ]);

    // ── Raw / Bronze: registry dataset (Iceberg landing) ───────────────────
    const assets = cat.map((c) => {
      const rows = totalOf.get(c.slug) ?? 0;
      const sekunder = c.tier === "sekunder";
      return {
        id: c.slug,
        name: c.title,
        namespace: sekunder ? "sekunder" : "sdi-primer",
        type: "iceberg-table",
        layer: BRONZE_CURATED.has(c.slug) ? "bronze" : "raw",
        tier: "warm",
        classification: "internal",
        owner: authorOf.get(c.slug) || "Dinas Pariwisata & Ekraf DKI Jakarta",
        domain: "pariwisata",
        description: c.description || "",
        format: "Apache Iceberg (Parquet)",
        engine: "hot-store",
        rows,
        sizeBytes: rows * 220,
        columnCount: colOf.get(c.slug) ?? 0,
        freshnessLagSeconds: 0,
        lastUpdated: c.updated_at || "",
        health: rows > 0 ? "healthy" : "degraded",
        residency: "id-jakarta",
      };
    });

    // ── Silver & Gold: dari ClickHouse (kurasi + mart penyaji dashboard) ────
    // Nama tabel bronze dipakai untuk membedakan Silver KURASI vs view auto
    // passthrough 1:1 (yang tak perlu tampil dobel di katalog).
    const bronzeTableNames = new Set(cat.map((c) => c.table_name));
    const prettify = (s: string) =>
      s.replace(/_/g, " ").replace(/\b\w/g, (m) => m.toUpperCase());

    const [tblRows, colCountRows, partRows] = await Promise.all([
      chRows<{ db: string; name: string; engine: string }>(
        `SELECT database db, name, engine FROM system.tables
         WHERE database IN ('silver','serving') ORDER BY name`,
      ),
      chRows<{ db: string; table: string; n: string }>(
        `SELECT database db, table, toString(count()) n FROM system.columns
         WHERE database IN ('silver','serving') GROUP BY database, table`,
      ),
      chRows<{ table: string; r: string }>(
        `SELECT table, toString(sum(rows)) r FROM system.parts
         WHERE database='serving' AND active GROUP BY table`,
      ),
    ]);
    const colCountOf = new Map(colCountRows.map((c) => [`${c.db}.${c.table}`, Number(c.n) || 0]));
    const goldRowsOf = new Map(partRows.map((p) => [p.table, Number(p.r) || 0]));

    for (const t of tblRows) {
      if (t.db === "silver") {
        // Lewati view auto-passthrough (nama identik dgn tabel bronze).
        if (bronzeTableNames.has(t.name)) continue;
        assets.push({
          id: `silver.${t.name}`,
          name: prettify(t.name),
          namespace: "silver",
          type: t.engine === "View" ? "view" : "table",
          layer: "silver",
          tier: "warm",
          classification: "internal",
          owner: "Dinas Pariwisata & Ekraf DKI Jakarta",
          domain: "pariwisata",
          description: "Model Silver terkurasi (bersih & terkonform) di ClickHouse.",
          format: t.engine === "View" ? "ClickHouse View" : `ClickHouse ${t.engine}`,
          engine: "hot-store",
          rows: 0,
          sizeBytes: 0,
          columnCount: colCountOf.get(`silver.${t.name}`) ?? 0,
          freshnessLagSeconds: 0,
          lastUpdated: "",
          health: "healthy",
          residency: "id-jakarta",
        });
      } else {
        // serving.* — mart Gold penyaji dashboard; lewati staging _baru.
        if (t.name.endsWith("_baru")) continue;
        const rows = goldRowsOf.get(t.name) ?? 0;
        assets.push({
          id: `serving.${t.name}`,
          name: prettify(t.name),
          namespace: "serving",
          type: "table",
          layer: "gold",
          tier: "hot",
          classification: "internal",
          owner: "Dinas Pariwisata & Ekraf DKI Jakarta",
          domain: "pariwisata",
          description: "Mart Gold penyaji dashboard (agregat siap pakai).",
          format: `ClickHouse ${t.engine}`,
          engine: "hot-store",
          rows,
          sizeBytes: rows * 220,
          columnCount: colCountOf.get(`serving.${t.name}`) ?? 0,
          freshnessLagSeconds: 0,
          lastUpdated: "",
          health: rows > 0 ? "healthy" : "degraded",
          residency: "id-jakarta",
        });
      }
    }

    const NS_META: Record<string, { name: string; description: string }> = {
      "sdi-primer": {
        name: "SDI Primer (Satu Data Jakarta)",
        description: "Dataset primer ditarik dari Satu Data Jakarta ke Bronze/Iceberg.",
      },
      sekunder: {
        name: "Data Sekunder (olahan)",
        description: "Dataset sekunder olahan (wisman bersih, TripAdvisor, halal, dll).",
      },
      silver: {
        name: "Silver (kurasi)",
        description: "Model bersih & terkonform di ClickHouse — dimensi, wisman, restoran, event, dst.",
      },
      serving: {
        name: "Gold (mart penyaji)",
        description: "Mart agregat penyaji dashboard — mart_wisman, mart_kuliner, mart_event, dll.",
      },
    };
    const nsCount = new Map<string, number>();
    for (const a of assets) nsCount.set(a.namespace, (nsCount.get(a.namespace) ?? 0) + 1);
    const namespaces = [...nsCount.entries()].map(([name, assetCount]) => ({
      id: name,
      name: NS_META[name]?.name ?? name,
      description: NS_META[name]?.description ?? "",
      assetCount,
      owner: "Dinas Pariwisata & Ekraf DKI Jakarta",
      residency: "id-jakarta",
      sourceEngine: "ClickHouse + Iceberg",
    }));

    return NextResponse.json({ assets, namespaces });
  } catch (e) {
    return NextResponse.json({ error: String(e), assets: [], namespaces: [] }, { status: 503 });
  }
}
