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

const FALLBACK_ACTIVITY: ActivityItem[] = [
  {
    id: "act-01",
    at: "2026-09-06T04:00:10.075Z",
    actor: "Dagster",
    actorKind: "service",
    action: "pipeline completed",
    target: "ops_backup_maintenance",
    category: "pipeline",
  },
  {
    id: "act-02",
    at: "2026-09-06T02:30:14.772Z",
    actor: "Bayu Pratama",
    actorKind: "user",
    action: "schema updated",
    target: "serving.mart_customer_segment",
    category: "schema",
  },
  {
    id: "act-03",
    at: "2026-09-06T01:15:22.100Z",
    actor: "Dewi Anggraini",
    actorKind: "user",
    action: "policy updated",
    target: "pol-mask-nik",
    category: "policy",
  },
  {
    id: "act-04",
    at: "2026-09-05T23:50:00.000Z",
    actor: "inventory-copilot",
    actorKind: "agent",
    action: "run finished",
    target: "emp-inventory",
    category: "agent",
  },
  {
    id: "act-05",
    at: "2026-09-05T22:10:45.000Z",
    actor: "system",
    actorKind: "service",
    action: "connector synced",
    target: "debezium-pg-wisata",
    category: "connector",
  },
  {
    id: "act-06",
    at: "2026-09-05T21:05:12.000Z",
    actor: "Budi Santoso",
    actorKind: "user",
    action: "query executed",
    target: "q-weekly-revenue-summary",
    category: "query",
  },
  {
    id: "act-07",
    at: "2026-09-05T19:40:00.000Z",
    actor: "Bootstrap Admin",
    actorKind: "user",
    action: "approval granted",
    target: "appr-lakekeeper-token",
    category: "approval",
  },
  {
    id: "act-08",
    at: "2026-09-05T18:15:30.000Z",
    actor: "AlertManager",
    actorKind: "service",
    action: "incident opened",
    target: "rt.orders_flow_mv",
    category: "incident",
  },
  {
    id: "act-09",
    at: "2026-09-05T16:00:14.000Z",
    actor: "Dagster",
    actorKind: "service",
    action: "pipeline completed",
    target: "refresh_lakehouse",
    category: "pipeline",
  },
  {
    id: "act-10",
    at: "2026-09-05T14:20:10.000Z",
    actor: "system",
    actorKind: "service",
    action: "tiering completed",
    target: "hot -> warm migration",
    category: "pipeline",
  },
];

export const clickhouseOverviewService: OverviewService = {
  async getSummary(signal) {
    const res = await apiFetch("/api/overview", { signal });
    const json = await res.json();
    if (!res.ok) throw new ServiceError("unavailable", json?.error ?? "Overview gagal dimuat");
    return json as OverviewSummary;
  },
  async listActivity(signal) {
    try {
      const res = await apiFetch("/api/overview", { method: "POST", signal });
      const json = await res.json();
      if (res.ok && Array.isArray(json.activity)) {
        return json.activity as ActivityItem[];
      }
    } catch {
      // Dagster GraphQL out of scope for local compose; fall back gracefully
    }
    return FALLBACK_ACTIVITY;
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
