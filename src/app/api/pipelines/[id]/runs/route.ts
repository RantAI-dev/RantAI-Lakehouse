import { NextResponse } from "next/server";
import { listRuns, mapRunStatus } from "@/services/clients/dagster";

export const dynamic = "force-dynamic";

/** Run NYATA sebuah pipeline (job Dagster). */
export async function GET(_req: Request, { params }: { params: Promise<{ id: string }> }) {
  const { id } = await params;
  try {
    const runs = await listRuns(id, 30);
    return NextResponse.json({
      runs: runs.map((r) => ({
        id: r.runId,
        pipelineId: id,
        status: mapRunStatus(r.status),
        startedAt: r.startTime ? new Date(r.startTime * 1000).toISOString() : "",
        endedAt: r.endTime ? new Date(r.endTime * 1000).toISOString() : undefined,
        processed: 0,
        accepted: 0,
        rejected: 0,
        retried: 0,
        costUnits: r.startTime && r.endTime ? Math.round(r.endTime - r.startTime) : 0,
      })),
    });
  } catch (e) {
    return NextResponse.json({ runs: [], error: String(e) }, { status: 503 });
  }
}
