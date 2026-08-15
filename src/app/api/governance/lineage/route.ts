import { NextResponse } from "next/server";
import { chRows } from "@/services/clients/clickhouse";

export const dynamic = "force-dynamic";

/**
 * Lineage NYATA sebuah dataset: source → Bronze (Iceberg) → Silver (typed),
 * dengan column mapping dari audit inferensi tipe (_silver_meta.kolom_tipe).
 * Bukan digambar tangan — berubah sesuai isi lakehouse.
 */
export async function GET(req: Request) {
  const focus = new URL(req.url).searchParams.get("focus") ?? "";
  try {
    // Resolusi slug → table_name + tier + title.
    const meta = (
      await chRows<{ table_name: string; title: string; tier: string }>(
        `SELECT table_name, title, tier FROM lake.\`bronze_meta.dataset_catalog\` WHERE slug='${focus.replace(/'/g, "")}'
         UNION ALL SELECT table_name, title, tier FROM lake.\`bronze_meta_sec.dataset_catalog\` WHERE slug='${focus.replace(/'/g, "")}' LIMIT 1`,
      )
    )[0];
    if (!meta) {
      return NextResponse.json({ focus, nodes: [], edges: [], columnMappings: [] });
    }
    const table = meta.table_name;
    const bronzeNs = meta.tier === "sekunder" ? "bronze_sec" : "bronze_sdi";

    // Kolom yang di-tipe-kan (audit) → column mapping bronze→silver.
    const cols = await chRows<{ kolom: string; tipe: string }>(
      `SELECT kolom, tipe FROM _silver_meta.kolom_tipe WHERE tabel='${table.replace(/'/g, "")}' LIMIT 200`,
    );

    const srcLabel = meta.tier === "sekunder" ? "Sumber sekunder (olahan)" : "Satu Data Jakarta";
    const nodes = [
      { id: "src", label: srcLabel, kind: "source" },
      { id: `bronze.${table}`, label: `Bronze · ${table}`, kind: "iceberg-table" },
      { id: `silver.${table}`, label: `Silver · ${table}`, kind: "view" },
    ];
    const edges = [
      { id: "e1", from: "src", to: `bronze.${table}`, kind: "pipeline" as const },
      { id: "e2", from: `bronze.${table}`, to: `silver.${table}`, kind: "transform" as const },
    ];
    const columnMappings = cols.map((c) => ({
      source: `${bronzeNs}.${table}.${c.kolom}`,
      target: `silver.${table}.${c.kolom}`,
      transform: c.tipe === "teks" ? "bersih_teks (String)" : c.tipe === "angka" ? "angka_id (Decimal)" : "tanggal_id (Date)",
    }));

    return NextResponse.json({ focus, nodes, edges, columnMappings });
  } catch (e) {
    return NextResponse.json({ error: String(e), focus, nodes: [], edges: [], columnMappings: [] }, { status: 503 });
  }
}
