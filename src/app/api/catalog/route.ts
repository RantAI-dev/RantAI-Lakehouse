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
    const freqOf = new Map(syncRows.map((s) => [s.slug, s.frekuensi]));
    const colOf = new Map(colRows.map((c) => [c.slug, Number(c.n) || 0]));

    const assets = cat.map((c) => {
      const rows = totalOf.get(c.slug) ?? 0;
      const sekunder = c.tier === "sekunder";
      return {
        id: c.slug,
        name: c.title,
        namespace: sekunder ? "sekunder" : "sdi-primer",
        type: "iceberg-table",
        layer: "bronze",
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

    const nsCount = new Map<string, number>();
    for (const a of assets) nsCount.set(a.namespace, (nsCount.get(a.namespace) ?? 0) + 1);
    const namespaces = [...nsCount.entries()].map(([name, assetCount]) => ({
      id: name,
      name: name === "sdi-primer" ? "SDI Primer (Satu Data Jakarta)" : "Data Sekunder (olahan)",
      description:
        name === "sdi-primer"
          ? "Dataset primer ditarik dari Satu Data Jakarta ke Bronze/Iceberg."
          : "Dataset sekunder olahan (wisman bersih, TripAdvisor, halal, dll).",
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
