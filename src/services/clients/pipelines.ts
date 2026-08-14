import type {
  PipelineService,
  Pipeline,
  PipelineDetail,
  PipelineRun,
} from "../contracts/pipelines";
import { mockPipelineService } from "../mock/pipelines";
import { ServiceError } from "../errors";

/**
 * PipelineService NYATA — job Dagster (orkestrasi lakehouse) lewat route
 * `/api/pipelines`. list/get/runs/trigger nyata; create/generate/cancel/retry/
 * pause/resume sementara delegasi mock (butuh mutation Dagster lanjutan).
 */

async function getJson<T>(url: string, init?: RequestInit): Promise<T> {
  const res = await fetch(url, init);
  const json = await res.json();
  if (!res.ok) throw new ServiceError("unavailable", json?.error ?? `Gagal (${res.status})`);
  return json as T;
}

export const dagsterPipelineService: PipelineService = {
  async listPipelines(signal) {
    return (await getJson<{ pipelines: Pipeline[] }>("/api/pipelines", { signal })).pipelines;
  },
  async listRuns(pipelineId, signal) {
    return (
      await getJson<{ runs: PipelineRun[] }>(`/api/pipelines/${encodeURIComponent(pipelineId)}/runs`, { signal })
    ).runs;
  },
  async triggerRun(id, signal) {
    return getJson<PipelineRun>(`/api/pipelines/${encodeURIComponent(id)}/trigger`, { method: "POST", signal });
  },
  async getPipeline(id, signal) {
    const [list, runs] = await Promise.all([this.listPipelines(signal), this.listRuns(id, signal)]);
    const base = list.find((p) => p.id === id);
    if (!base) throw new ServiceError("not_found", "Pipeline tidak ditemukan");
    const detail: PipelineDetail = {
      ...base,
      description: "Job Dagster: refresh lakehouse Bronze→Silver→Gold (dlt + SQLMesh).",
      graph: [
        { id: "bronze", label: "Bronze (dlt)", kind: "ingest", status: "completed" },
        { id: "silver", label: "Silver (typed)", kind: "transform", status: "completed" },
        { id: "gold", label: "Gold (mart)", kind: "publish", status: "completed" },
      ],
      runs,
      configSummary: [
        { key: "engine", value: "Dagster" },
        { key: "schedule", value: base.schedule },
      ],
    };
    return detail;
  },

  // ── Delegasi mock (mutation lanjutan) ───────────────────────────────────
  createPipeline: (i, s) => mockPipelineService.createPipeline(i, s),
  generatePipelineFromPrompt: (i, s) => mockPipelineService.generatePipelineFromPrompt(i, s),
  cancelRun: (i, s) => mockPipelineService.cancelRun(i, s),
  retryRun: (i, s) => mockPipelineService.retryRun(i, s),
  pausePipeline: (i, s) => mockPipelineService.pausePipeline(i, s),
  resumePipeline: (i, s) => mockPipelineService.resumePipeline(i, s),
};
