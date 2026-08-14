import { NextResponse } from "next/server";
import { launchRun, mapRunStatus } from "@/services/clients/dagster";

export const dynamic = "force-dynamic";

/** Trigger run NYATA sebuah job Dagster dari konsol. */
export async function POST(_req: Request, { params }: { params: Promise<{ id: string }> }) {
  const { id } = await params;
  try {
    const r = await launchRun(id);
    if (r.error) return NextResponse.json({ error: r.error }, { status: 422 });
    return NextResponse.json({
      id: r.runId,
      pipelineId: id,
      status: mapRunStatus("STARTED"),
      startedAt: new Date().toISOString(),
      processed: 0,
      accepted: 0,
      rejected: 0,
      retried: 0,
      costUnits: 0,
    });
  } catch (e) {
    return NextResponse.json({ error: String(e) }, { status: 503 });
  }
}
