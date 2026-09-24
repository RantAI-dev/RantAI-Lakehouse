import type {
  OverviewService,
  OverviewSummary,
  ActivityItem,
  AlertItem,
} from "../contracts/overview";
import { apiFetch } from "../http";
import { ServiceError } from "../errors";

/**
 * OverviewService is now fully real: getSummary + listActivity from
 * ClickHouse/Dagster; alerts (list/ack/resolve) from Postgres (Task 2.6) —
 * see `lakehouse_store::overview` for why alert instances are stored in
 * Postgres, not ClickHouse. mock/overview.ts has been deleted.
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
    throw new ServiceError(kind, json?.error ?? `Failed (${res.status})`);
  }
  return json as T;
}

export const clickhouseOverviewService: OverviewService = {
  async getSummary(signal) {
    const res = await apiFetch("/api/overview", { signal });
    const json = await res.json();
    if (!res.ok) throw new ServiceError("unavailable", json?.error ?? "Failed to load overview");
    return json as OverviewSummary;
  },
  async listActivity(signal) {
    // The feed is the audit trail; an empty list means nothing has happened
    // yet. It used to fall back to a hard-coded sample, which read as real.
    const res = await apiFetch("/api/overview", { method: "POST", signal });
    const json = await res.json();
    if (!res.ok) throw new ServiceError("unavailable", json?.error ?? "Failed to load activity");
    return (json.activity ?? []) as ActivityItem[];
  },
  async listAlerts(signal) {
    const res = await apiFetch("/api/overview/alerts", { signal });
    const json = await res.json();
    if (!res.ok) throw new ServiceError("unavailable", json?.error ?? "Failed to load alerts");
    return json as AlertItem[];
  },
  acknowledgeAlert(id, signal) {
    return postJson<AlertItem>(`/api/overview/alerts/${encodeURIComponent(id)}/acknowledge`, undefined, signal);
  },
  resolveAlert(id, note, signal) {
    return postJson<AlertItem>(`/api/overview/alerts/${encodeURIComponent(id)}/resolve`, { note }, signal);
  },
  silenceAlert(id, untilMinutes, signal) {
    return postJson<AlertItem>(`/api/overview/alerts/${encodeURIComponent(id)}/silence`, { untilMinutes }, signal);
  },
};
