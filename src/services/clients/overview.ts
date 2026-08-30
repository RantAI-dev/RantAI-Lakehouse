import type {
  OverviewService,
  OverviewSummary,
  ActivityItem,
  AlertItem,
} from "../contracts/overview";
import { apiFetch } from "../http";
import { ServiceError } from "../errors";

/**
 * OverviewService NYATA sepenuhnya: getSummary + listActivity dari
 * ClickHouse/Dagster; alerts (list/ack/resolve) dari Postgres (Task 2.6) —
 * lihat `lakehouse_store::overview` untuk alasan instance alert disimpan di
 * Postgres, bukan ClickHouse. mock/overview.ts sudah dihapus.
 */
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
    throw new ServiceError(kind, json?.error ?? `Gagal (${res.status})`);
  }
  return json as T;
}

export const clickhouseOverviewService: OverviewService = {
  async getSummary(signal) {
    const res = await apiFetch("/api/overview", { signal });
    const json = await res.json();
    if (!res.ok) throw new ServiceError("unavailable", json?.error ?? "Overview gagal dimuat");
    return json as OverviewSummary;
  },
  async listActivity(signal) {
    const res = await apiFetch("/api/overview", { method: "POST", signal });
    const json = await res.json();
    if (!res.ok) throw new ServiceError("unavailable", json?.error ?? "Activity gagal dimuat");
    return json.activity as ActivityItem[];
  },
  async listAlerts(signal) {
    const res = await apiFetch("/api/overview/alerts", { signal });
    const json = await res.json();
    if (!res.ok) throw new ServiceError("unavailable", json?.error ?? "Alerts gagal dimuat");
    return json as AlertItem[];
  },
  acknowledgeAlert(id, signal) {
    return postJson<AlertItem>(`/api/overview/alerts/${encodeURIComponent(id)}/acknowledge`, undefined, signal);
  },
  resolveAlert(id, note, signal) {
    return postJson<AlertItem>(`/api/overview/alerts/${encodeURIComponent(id)}/resolve`, { note }, signal);
  },
};
