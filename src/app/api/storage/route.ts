import { NextResponse } from "next/server";
import { chRows } from "@/services/clients/clickhouse";

export const dynamic = "force-dynamic";

/**
 * StorageOverview NYATA: Hot = MergeTree serving (system.parts), Warm = tabel
 * Iceberg @ RustFS (estimasi dari total baris katalog), Cold/AI = 0 (belum ada).
 */
export async function GET() {
  try {
    const hot = (
      await chRows<{ bytes: string; assets: string }>(
        `SELECT toString(sum(bytes_on_disk)) bytes, toString(uniqExact(table)) assets
         FROM system.parts WHERE database='serving' AND active`,
      )
    )[0];
    const warm = (
      await chRows<{ rows: string; assets: string }>(
        `SELECT toString(sum(total)) rows, toString(count()) assets FROM (
           SELECT total FROM lake.\`bronze_meta.dataset_sync\`
           UNION ALL SELECT total FROM lake.\`bronze_meta_sec.dataset_sync\`)`,
      )
    )[0];
    const hotBytes = Number(hot?.bytes) || 0;
    const warmBytes = (Number(warm?.rows) || 0) * 220;

    const byTier = {
      hot: { bytes: hotBytes, assets: Number(hot?.assets) || 0, growth7d: 0 },
      warm: { bytes: warmBytes, assets: Number(warm?.assets) || 0, growth7d: 0 },
      cold: { bytes: 0, assets: 0, growth7d: 0 },
      ai: { bytes: 0, assets: 0, growth7d: 0 },
    };
    // Penghematan vs semua-Hot: Warm/Cold di object storage jauh lebih murah.
    const savingsVsAllHot = Math.round((warmBytes / Math.max(1, hotBytes + warmBytes)) * 100);

    return NextResponse.json({ byTier, savingsVsAllHot, failedTieringOps: 0, pendingRestores: 0 });
  } catch (e) {
    return NextResponse.json({ error: String(e) }, { status: 503 });
  }
}
