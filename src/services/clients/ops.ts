import type {
  OpsService,
  ObservabilitySummary,
  UsageSummary,
  WorkloadItem,
  PlatformService,
} from "../contracts/ops";
import { apiFetch } from "../http";
import { ServiceError } from "../errors";

/**
 * OpsService NYATA sepenuhnya: observability/usage/workloads/services dari
 * ClickHouse system.* + Dagster. cancelWorkload adalah `KILL QUERY` nyata
 * lewat `/api/ops/workloads/{id}/cancel`. mock/ops.ts sudah dihapus.
 */
async function get<T>(url: string, signal?: AbortSignal): Promise<T> {
  const res = await apiFetch(url, { signal });
  const json = await res.json();
  if (!res.ok) throw new ServiceError("unavailable", json?.error ?? "Ops gagal dimuat");
  return json as T;
}

export const clickhouseOpsService: OpsService = {
  getObservability: (s) => get<ObservabilitySummary>("/api/ops/observability", s),
  getUsage: (s) => get<UsageSummary>("/api/ops/usage", s),
  async listWorkloads(s) {
    return (await get<{ workloads: WorkloadItem[] }>("/api/ops/workloads", s)).workloads;
  },
  async listServices(s) {
    return (await get<{ services: PlatformService[] }>("/api/ops/services", s)).services;
  },
  async cancelWorkload(id, signal) {
    const res = await apiFetch(`/api/ops/workloads/${encodeURIComponent(id)}/cancel`, {
      method: "POST",
      signal,
    });
    const json = await res.json();
    if (!res.ok) {
      const kind = res.status === 404 ? "not_found" : res.status >= 500 ? "unavailable" : "invalid_request";
      throw new ServiceError(kind, json?.error ?? `Gagal (${res.status})`);
    }
    return json as WorkloadItem;
  },
};
