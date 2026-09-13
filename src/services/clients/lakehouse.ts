import type {
  LakehouseMaintenance,
  LakehouseNamespace,
  LakehouseService,
  LakehouseTableDetail,
  LakehouseTableSummary,
  LakehouseWarehouse,
  MaintenancePolicyInput,
  MaintenancePolicyResult,
} from "../contracts/lakehouse"
import { apiFetch } from "../http"
import { ServiceError } from "../errors"
import {
  lakehouseMaintenanceUrl,
  lakehouseNamespacesUrl,
  lakehouseTableDetailUrl,
  lakehouseTablesUrl,
  lakehouseWarehousesUrl,
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
    const kind =
      res.status === 401 || res.status === 403
        ? "permission_denied"
        : res.status === 404
          ? "not_found"
          : res.status >= 500 || res.status === 503
            ? "unavailable"
            : "invalid_request"
    throw new ServiceError(kind, json?.error ?? fallbackMessage)
  }
  return json as T
}

export const icebergLakehouseService: LakehouseService = {
  async listWarehouses(signal) {
    const json = await request<{ warehouses: LakehouseWarehouse[] }>(
      lakehouseWarehousesUrl(),
      { cache: "no-store", signal },
      "Lakehouse warehouses could not be loaded"
    )
    return json.warehouses
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
