import { NextResponse } from "next/server";
import { listJobs, listRuns, mapRunStatus } from "@/services/clients/dagster";

export const dynamic = "force-dynamic";

/** Daftar pipeline NYATA = job Dagster, diperkaya run terakhir + jadwal. */
export async function GET() {
  try {
    const [jobs, runs] = await Promise.all([listJobs(), listRuns(undefined, 100)]);
    const lastByJob = new Map<string, (typeof runs)[number]>();
    for (const r of runs) {
      const prev = lastByJob.get(r.jobName);
      if (!prev || (r.startTime ?? 0) > (prev.startTime ?? 0)) lastByJob.set(r.jobName, r);
    }
    const pipelines = jobs.map((j) => {
      const last = lastByJob.get(j.name);
      const sched = j.schedules[0];
      const lastRunAt = last?.startTime ? new Date(last.startTime * 1000).toISOString() : "";
      return {
        id: j.name,
        name: j.name,
        kind: "batch",
        status: last ? mapRunStatus(last.status) : "unknown",
        owner: "Dinas Pariwisata & Ekraf DKI Jakarta",
        source: "Satu Data Jakarta + berkas",
        target: "serving.mart_* (Gold)",
        schedule: sched ? `cron: ${sched.cronSchedule} (${sched.scheduleState.status})` : "manual",
        lastRunAt,
        slaOk: last ? last.status === "SUCCESS" : true,
        freshnessLagSeconds: 0,
      };
    });
    return NextResponse.json({ pipelines });
  } catch (e) {
    return NextResponse.json({ pipelines: [], error: String(e) }, { status: 503 });
  }
}
