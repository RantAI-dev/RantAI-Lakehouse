import type {
  LakehouseCapacity,
  LakehouseMaintenance,
  LakehouseNamespace,
  LakehouseService,
  LakehouseTableDetail,
  LakehouseTableSummary,
  MaintenancePolicyInput,
  MaintenancePolicyResult,
} from "../contracts/lakehouse"
import { apiFetch } from "../http"
import { ServiceError, deniedFor } from "../errors"
import {
  lakehouseCapacityUrl,
  lakehouseMaintenanceUrl,
  lakehouseNamespacesUrl,
  lakehouseTableDetailUrl,
  lakehouseTablesUrl,
} from "@/lib/lakehouse-view"

/**
 * LakehouseService over `/api/lakehouse/*` (WS2 §4) — the read-only Iceberg
 * warehouse/namespace/table surface. Copies `clients/alerts.ts`'s private
 * `request` shape: every method throws a `ServiceError` carrying the
 * server's `{ error }` message on a non-OK response.
 */
async function request<T>(
  url: string,
  init: RequestInit | undefined,
  fallbackMessage: string
): Promise<T> {
  const res = await apiFetch(url, init)
  const json = await res.json()
  if (!res.ok) {
    // QA TC-1724 (WEB-12): a refusal goes through deniedFor.
    const denied = deniedFor(res.status, json)
    if (denied) throw denied
    const kind =
      res.status === 404
        ? "not_found"
        : res.status >= 500 || res.status === 503
          ? "unavailable"
          : "invalid_request"
    throw new ServiceError(kind, json?.error ?? fallbackMessage)
  }
  return json as T
}

export const icebergLakehouseService: LakehouseService = {
  async getCapacity(signal) {
    return request<LakehouseCapacity>(
      lakehouseCapacityUrl(),
      { cache: "no-store", signal },
      "Lakehouse capacity could not be loaded"
    )
  },
  async listNamespaces(warehouse, signal) {
    const json = await request<{ namespaces: LakehouseNamespace[] }>(
      lakehouseNamespacesUrl(warehouse),
      { cache: "no-store", signal },
      "Lakehouse namespaces could not be loaded"
    )
    return json.namespaces
  },
  async listTables(namespace, warehouse, signal) {
    const json = await request<{ tables: LakehouseTableSummary[] }>(
      lakehouseTablesUrl(namespace, warehouse),
      { cache: "no-store", signal },
      "Lakehouse tables could not be loaded"
    )
    return json.tables
  },
  async getTableDetail(namespace, table, signal) {
    return request<LakehouseTableDetail>(
      lakehouseTableDetailUrl(namespace, table),
      { cache: "no-store", signal },
      "Lakehouse table detail could not be loaded"
    )
  },
  async getMaintenance(namespace, table, signal) {
    return request<LakehouseMaintenance>(
      lakehouseMaintenanceUrl(namespace, table),
      { cache: "no-store", signal },
      "Lakehouse maintenance policy could not be loaded"
    )
  },
  async setMaintenancePolicy(namespace, table, input, signal) {
    return request<MaintenancePolicyResult>(
      lakehouseMaintenanceUrl(namespace, table),
      {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(input satisfies MaintenancePolicyInput),
        signal,
      },
      "Lakehouse maintenance policy could not be saved"
    )
  },
}
