import type {
  AlertRule,
  AlertRuleService,
  AlertRunResult,
  SaveAlertRuleInput,
} from "../contracts/alerts";
import { apiFetch } from "../http";
import { ServiceError } from "../errors";

/**
 * AlertRuleService over `/api/alerts` (rule CRUD) and `/api/alerts/run`
 * (evaluate). WS1 task 1.15: this client (and its contract) did not exist
 * before — the feature fetched `/api/alerts` directly with no `res.ok`
 * check, so a dead API rendered as an empty rule list. Every method here
 * throws a `ServiceError` carrying the server's `{ error }` message on a
 * non-OK response, classified the same way `clickhouseOverviewService`
 * does, so `useService`/`useServiceAction` can render the failure instead
 * of silently swallowing it.
 */
async function request<T>(
  url: string,
  init: RequestInit | undefined,
  fallbackMessage: string
): Promise<T> {
  const res = await apiFetch(url, init);
  const json = await res.json();
  if (!res.ok) {
    const kind =
      res.status === 401 || res.status === 403
        ? "permission_denied"
        : res.status === 404
          ? "not_found"
          : res.status >= 500 || res.status === 503
            ? "unavailable"
            : "invalid_request";
    throw new ServiceError(kind, json?.error ?? fallbackMessage);
  }
  return json as T;
}

function saveBody(input: SaveAlertRuleInput): string {
  return JSON.stringify(input);
}

export const clickhouseAlertRuleService: AlertRuleService = {
  async listRules(signal) {
    const json = await request<{ rules: AlertRule[] }>(
      "/api/alerts",
      { cache: "no-store", signal },
      "Alert rules could not be loaded"
    );
    return json.rules;
  },
  async createRule(input, signal) {
    const json = await request<{ ok: true; rule: AlertRule }>(
      "/api/alerts",
      {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: saveBody(input),
        signal,
      },
      "Rule could not be created"
    );
    return json.rule;
  },
  async updateRule(rule, signal) {
    const json = await request<{ ok: true; rule: AlertRule }>(
      "/api/alerts",
      {
        method: "PUT",
        headers: { "Content-Type": "application/json" },
        body: saveBody(rule),
        signal,
      },
      "Rule could not be saved"
    );
    return json.rule;
  },
  async deleteRule(id, signal) {
    await request<{ ok: true }>(
      `/api/alerts?id=${encodeURIComponent(id)}`,
      { method: "DELETE", signal },
      "Rule could not be deleted"
    );
  },
  async runRules(id, signal) {
    const q = id ? `?id=${encodeURIComponent(id)}` : "";
    return request<{ ran: number; results: AlertRunResult[] }>(
      `/api/alerts/run${q}`,
      { method: "POST", signal },
      "Rules could not be run"
    );
  },
};
