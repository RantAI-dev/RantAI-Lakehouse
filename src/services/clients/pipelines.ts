import type {
  PipelineService,
  Pipeline,
  PipelineRun,
  PipelineDetail,
  PipelineSource,
  PipelineRunStep,
  PipelineRunLogsPage,
  CreatePipelineInput,
} from "../contracts/pipelines";
import { apiFetch } from "../http";
import { ServiceError } from "../errors";

/**
 * PipelineService is real — Dagster jobs (lakehouse orchestration) via the
 * `/api/pipelines` route, plus (Task 2.5) pipeline definitions authored via
 * Postgres (`createPipeline`) and real Dagster mutations for
 * cancel/retry/pause/resume. There is no more delegation to mock — every
 * method here calls the Rust backend.
 */

async function getJson<T>(url: string, init?: RequestInit): Promise<T> {
  const res = await apiFetch(url, init);
  const json = await res.json();
  if (!res.ok) throw new ServiceError("unavailable", json?.error ?? `Failed (${res.status})`);
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
    throw new ServiceError(kind, json?.error ?? `Failed (${res.status})`);
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
    // WS4 item F1: `GET /api/pipelines/{id}` now returns the real detail
    // (engine, op graph, config, authored definition) directly — no more
    // reconstructing a partial detail from the list + runs endpoints, the
    // WS1 task 1.1 placeholder this replaces.
    return getJson<PipelineDetail>(`/api/pipelines/${encodeURIComponent(id)}`, { signal });
  },

  createPipeline(input: CreatePipelineInput, signal) {
    return postJson<Pipeline>("/api/pipelines", input, signal);
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
  getPipelineSource(id, op, signal) {
    return getJson<PipelineSource>(
      `/api/pipelines/${encodeURIComponent(id)}/source?op=${encodeURIComponent(op)}`,
      { signal }
    );
  },
  async getRunSteps(id, runId, signal) {
    return (
      await getJson<{ steps: PipelineRunStep[] }>(
        `/api/pipelines/${encodeURIComponent(id)}/runs/${encodeURIComponent(runId)}/steps`,
        { signal }
      )
    ).steps;
  },
  getRunLogs(id, runId, cursor, signal) {
    const query = cursor ? `?cursor=${encodeURIComponent(cursor)}` : "";
    return getJson<PipelineRunLogsPage>(
      `/api/pipelines/${encodeURIComponent(id)}/runs/${encodeURIComponent(runId)}/logs${query}`,
      { signal }
    );
  },
  setPipelineStatus(id, status, signal) {
    return postJson<Pipeline>(`/api/pipelines/${encodeURIComponent(id)}/status`, { status }, signal);
  },
};
