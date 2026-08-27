import type {
  PipelineService,
  Pipeline,
  PipelineDetail,
  PipelineRun,
  CreatePipelineInput,
  GeneratePipelineInput,
} from "../contracts/pipelines";
import { ServiceError } from "../errors";

/**
 * PipelineService NYATA — job Dagster (orkestrasi lakehouse) lewat route
 * `/api/pipelines`, plus (Task 2.5) pipeline definitions yang diauthor lewat
 * Postgres (`createPipeline`/`generatePipelineFromPrompt`) dan mutation
 * Dagster nyata untuk cancel/retry/pause/resume. Tidak ada lagi delegasi ke
 * mock — setiap method di sini memanggil backend Rust.
 */

async function getJson<T>(url: string, init?: RequestInit): Promise<T> {
  const res = await fetch(url, init);
  const json = await res.json();
  if (!res.ok) throw new ServiceError("unavailable", json?.error ?? `Gagal (${res.status})`);
  return json as T;
}

async function postJson<T>(url: string, body: unknown, signal?: AbortSignal): Promise<T> {
  const res = await fetch(url, {
    method: "POST",
    headers: body === undefined ? undefined : { "Content-Type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
    signal,
  });
  const json = await res.json();
  if (!res.ok) {
    const kind = res.status === 404 ? "not_found" : res.status >= 500 ? "unavailable" : "invalid_request";
    throw new ServiceError(kind, json?.error ?? `Gagal (${res.status})`);
  }
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

  createPipeline(input: CreatePipelineInput, signal) {
    return postJson<Pipeline>("/api/pipelines", input, signal);
  },
  generatePipelineFromPrompt(input: GeneratePipelineInput, signal) {
    return postJson<Pipeline>("/api/pipelines/generate", input, signal);
  },
  cancelRun(runId, signal) {
    return postJson<PipelineRun>(`/api/pipelines/runs/${encodeURIComponent(runId)}/cancel`, undefined, signal);
  },
  retryRun(runId, signal) {
    return postJson<PipelineRun>(`/api/pipelines/runs/${encodeURIComponent(runId)}/retry`, undefined, signal);
  },
  pausePipeline(id, signal) {
    return postJson<Pipeline>(`/api/pipelines/${encodeURIComponent(id)}/pause`, undefined, signal);
  },
  resumePipeline(id, signal) {
    return postJson<Pipeline>(`/api/pipelines/${encodeURIComponent(id)}/resume`, undefined, signal);
  },
};
