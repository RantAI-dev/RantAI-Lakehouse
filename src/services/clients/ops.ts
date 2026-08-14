import type {
  OpsService,
  ObservabilitySummary,
  UsageSummary,
  WorkloadItem,
  PlatformService,
} from "../contracts/ops";
import { mockOpsService } from "../mock/ops";
import { ServiceError } from "../errors";

/**
 * OpsService NYATA: observability/usage/workloads/services dari ClickHouse
 * system.* + Dagster. cancelWorkload delegasi mock.
 */
async function get<T>(url: string, signal?: AbortSignal): Promise<T> {
  const res = await fetch(url, { signal });
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
  cancelWorkload: (id, s) => mockOpsService.cancelWorkload(id, s),
};
