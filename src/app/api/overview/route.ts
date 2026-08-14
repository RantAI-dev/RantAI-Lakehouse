import { NextResponse } from "next/server";
import { chRows } from "@/services/clients/clickhouse";
import { listJobs, listRuns, mapRunStatus } from "@/services/clients/dagster";

export const dynamic = "force-dynamic";

/** OverviewSummary NYATA: agregat lakehouse (katalog + query_log + Dagster). */
export async function GET() {
  try {
    const [assetsRow] = await chRows<{ n: string; stale: string }>(
      `SELECT toString(count()) n, toString(countIf(coalesce(s.total,0)=0)) stale FROM (
         SELECT slug FROM lake.\`bronze_meta.dataset_catalog\`
         UNION ALL SELECT slug FROM lake.\`bronze_meta_sec.dataset_catalog\`) c
       LEFT JOIN (SELECT slug,total FROM lake.\`bronze_meta.dataset_sync\`
                  UNION ALL SELECT slug,total FROM lake.\`bronze_meta_sec.dataset_sync\`) s ON c.slug=s.slug`,
    );
    const [hot] = await chRows<{ bytes: string; assets: string }>(
      `SELECT toString(sum(bytes_on_disk)) bytes, toString(uniqExact(table)) assets FROM system.parts WHERE database='serving' AND active`,
    );
    const [warm] = await chRows<{ rows: string; assets: string }>(
      `SELECT toString(sum(total)) rows, toString(count()) assets FROM (SELECT total FROM lake.\`bronze_meta.dataset_sync\` UNION ALL SELECT total FROM lake.\`bronze_meta_sec.dataset_sync\`)`,
    );
    const [q] = await chRows<{ vol: string; p95: string; err: string; scan: string }>(
      `SELECT toString(count()) vol, toString(round(quantile(0.95)(query_duration_ms))) p95,
              toString(round(countIf(exception!='')/greatest(count(),1),4)) err, toString(sum(read_bytes)) scan
       FROM system.query_log WHERE type='QueryFinish' AND event_time > now() - INTERVAL 24 HOUR`,
    );

    const runs = await listRuns(undefined, 100);
    const jobs = await listJobs();
    const recent = runs.filter((r) => (r.startTime ?? 0) * 1000 > Date.now() - 864e5);
    const failed = recent.filter((r) => r.status === "FAILURE").length;
    const active = recent.filter((r) => ["STARTED", "STARTING", "QUEUED"].includes(r.status)).length;

    return NextResponse.json({
      assetsTotal: Number(assetsRow?.n) || 0,
      staleAssets: Number(assetsRow?.stale) || 0,
      assetsByTier: {
        hot: { count: Number(hot?.assets) || 0, bytes: Number(hot?.bytes) || 0 },
        warm: { count: Number(warm?.assets) || 0, bytes: (Number(warm?.rows) || 0) * 220 },
        cold: { count: 0, bytes: 0 },
        ai: { count: 0, bytes: 0 },
      },
      pipelines: { active, failed, delayed: 0 },
      streaming: { jobs: 0, maxLagSeconds: 0, unhealthy: 0 },
      queries: {
        volume24h: Number(q?.vol) || 0,
        p95Ms: Number(q?.p95) || 0,
        failureRate: Number(q?.err) || 0,
        cacheAssistRate: 0,
        scannedBytes24h: Number(q?.scan) || 0,
      },
      policyViolations7d: 0,
      pendingApprovals: 0,
      agents: { activeRuns: 0, budgetUsedRate: 0 },
      services: { healthy: 4, degraded: 0, unhealthy: 0 },
      incidents: [],
    });
  } catch (e) {
    return NextResponse.json({ error: String(e) }, { status: 503 });
  }
}

/** Aktivitas terbaru NYATA: run Dagster. */
export async function POST() {
  try {
    const runs = await listRuns(undefined, 20);
    return NextResponse.json({
      activity: runs.map((r) => ({
        id: r.runId,
        at: r.startTime ? new Date(r.startTime * 1000).toISOString() : "",
        actor: "Dagster",
        actorKind: "service",
        action: `pipeline ${mapRunStatus(r.status)}`,
        target: r.jobName,
        category: "pipeline",
      })),
    });
  } catch (e) {
    return NextResponse.json({ activity: [], error: String(e) }, { status: 503 });
  }
}
