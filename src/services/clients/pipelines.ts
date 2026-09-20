import type {
  PipelineService,
  Pipeline,
  PipelineDetail,
  PipelineList,
  PipelineRun,
  CreatePipelineInput,
  GeneratePipelineInput,
} from "../contracts/pipelines";
import { apiFetch } from "../http";
import { ServiceError } from "../errors";

/**
 * PipelineService NYATA — job Dagster (orkestrasi lakehouse) lewat route
 * `/api/pipelines`, plus (Task 2.5) pipeline definitions yang diauthor lewat
 * Postgres (`createPipeline`/`generatePipelineFromPrompt`) dan mutation
 * Dagster nyata untuk cancel/retry/pause/resume. Tidak ada lagi delegasi ke
 * mock — setiap method di sini memanggil backend Rust.
 */

async function getJson<T>(url: string, init?: RequestInit): Promise<T> {
  const res = await apiFetch(url, init);
  const json = await res.json();
  if (!res.ok) throw new ServiceError("unavailable", json?.error ?? `Request failed (${res.status})`);
  return json as T;
}

async function postJson<T>(url: string, body: unknown, signal?: AbortSignal): Promise<T> {
  const res = await apiFetch(url, {
    method: "POST",
    headers: body === undefined ? undefined : { "Content-Type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
    signal,
  });
  const json = await res.json();
  if (!res.ok) {
    const kind = res.status === 404 ? "not_found" : res.status >= 500 ? "unavailable" : "invalid_request";
    throw new ServiceError(kind, json?.error ?? `Request failed (${res.status})`);
  }
  return json as T;
}

export const dagsterPipelineService: PipelineService = {
  async listPipelines(signal) {
    const body = await getJson<PipelineList>("/api/pipelines", { signal });
    // The orchestrator half may be missing; the page says so rather than
    // presenting a short list as the whole one.
    return { pipelines: body.pipelines ?? [], orchestratorError: body.orchestratorError ?? null };
  },
  async listRuns(pipelineId, signal) {
    return (
      await getJson<{ runs: PipelineRun[] }>(`/api/pipelines/${encodeURIComponent(pipelineId)}/runs`, { signal })
    ).runs;
  },
  async triggerRun(id, signal) {
    return getJson<PipelineRun>(`/api/pipelines/${encodeURIComponent(id)}/trigger`, { method: "POST", signal });
  },
  // One request to one endpoint. This used to fetch the whole list, find
  // the row, fetch the runs, and then make up the description, the graph
  // and the config summary in the browser — the same three-node
  // Bronze/Silver/Gold diagram for every pipeline that exists.
  getPipeline(id, signal) {
    return getJson<PipelineDetail>(`/api/pipelines/${encodeURIComponent(id)}`, { signal });
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
