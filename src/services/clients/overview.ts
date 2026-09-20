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
    let kind: "not_found" | "unavailable" | "invalid_request" = "invalid_request";
    if (res.status === 404) {
      kind = "not_found";
    } else if (res.status >= 500) {
      kind = "unavailable";
    }
    throw new ServiceError(kind, json?.error ?? `Gagal (${res.status})`);
  }
  return json as T;
}

export const clickhouseOverviewService: OverviewService = {
  async getSummary(signal) {
    const res = await apiFetch("/api/overview", { signal });
    const json = await res.json();
    if (!res.ok) throw new ServiceError("unavailable", json?.error ?? "Couldn't load the overview");
    return json as OverviewSummary;
  },
  async listActivity(signal) {
    // The feed is the audit trail; an empty list means nothing has happened
    // yet. It used to fall back to a hard-coded sample, which read as real.
    const res = await apiFetch("/api/overview", { method: "POST", signal });
    const json = await res.json();
    if (!res.ok) throw new ServiceError("unavailable", json?.error ?? "Couldn't load recent activity");
    return (json.activity ?? []) as ActivityItem[];
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
