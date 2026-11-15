import type {
  AlertRule,
  AlertsStatus,
  AlertRuleService,
  AlertRunResult,
  SaveAlertRuleInput,
} from "../contracts/alerts";
import { apiFetch } from "../http";
import { ServiceError, deniedFor } from "../errors";

/**
 * AlertRuleService over `/api/alerts` (rule CRUD) and `/api/alerts/test`
 * (`testRule`, one rule at a time: `BI-40` D1; the scheduler's
 * `/api/alerts/run` is not a person's door). WS1 task 1.15: this client (and
 * its contract) did not exist before — the feature fetched `/api/alerts` directly with no `res.ok`
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
    // WEB-12: a refusal goes through deniedFor (errors.ts).
    const denied = deniedFor(res.status, json);
    if (denied) throw denied;
    const kind =
      res.status === 404
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
  async getStatus(signal) {
    return request<AlertsStatus>(
      "/api/alerts/status",
      { cache: "no-store", signal },
      "Run alert status could not be loaded"
    );
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
  async testRule(id, signal) {
    try {
      return await request<{ ran: number; results: AlertRunResult[] }>(
        `/api/alerts/test?id=${encodeURIComponent(id)}`,
        { method: "POST", signal },
        "Rule could not be tested"
      );
    } catch (err) {
      // `BI-40` D2 gives this route's refusal as the `alert:write` permission
      // (`permission_denied: alert:write`); the console says it in words a
      // person can act on. A 404 keeps the server's own sentence
      // ("alert rule not found."), which is the same whether the rule is gone
      // or simply not theirs to see.
      if (err instanceof ServiceError && err.code === "permission_denied") {
        throw new ServiceError("permission_denied", "You can test only the alerts you manage.");
      }
      throw err;
    }
  },
};
