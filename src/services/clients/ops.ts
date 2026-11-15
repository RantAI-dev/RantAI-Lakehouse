import type { OpsService, PlatformService } from "../contracts/ops";
import { apiFetch } from "../http";
import { ServiceError, deniedFor } from "../errors";

/**
 * OpsService is real: the service list comes from ClickHouse system.* +
 * Dagster. `WEB-3` slice 1 (D1/D2) removed the observability and workloads
 * reads with their pages — the numbers they showed were nulls and fixed
 * literals, and the one real action (cancel a running query) is Query
 * Studio's Stop (`BI-5`). The backend routes stay registered (D2) for the
 * assistant's tools; `WEB-4` retires them.
 */
async function get<T>(url: string, signal?: AbortSignal): Promise<T> {
  const res = await apiFetch(url, { signal });
  const json = await res.json();
  if (!res.ok) {
    // WEB-12: a refusal is a refusal, not the service being unavailable.
    const denied = deniedFor(res.status, json);
    if (denied) throw denied;
    throw new ServiceError("unavailable", json?.error ?? "Failed to load ops");
  }
  return json as T;
}

export const clickhouseOpsService: OpsService = {
  async listServices(s) {
    return (await get<{ services: PlatformService[] }>("/api/ops/services", s)).services;
  },
};
