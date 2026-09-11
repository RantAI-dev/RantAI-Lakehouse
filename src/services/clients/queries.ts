import type {
  QueryService,
  QueryResult,
  QueryEstimate,
  SavedQuery,
  QueryHistoryItem,
} from "../contracts/queries";
import { apiFetch } from "../http";
import { ServiceError } from "../errors";

/**
 * QueryService is real — `run`/`estimate` execute SQL against ClickHouse
 * (our lakehouse) via the server route `/api/query/*`; `generateSql` goes
 * through `/api/agent/text-to-sql` (an LLM grounded on the lakehouse schema,
 * Phase 1). `listSaved`/`listHistory` are now real too, stored in Postgres
 * via the `lakehouse-store` crate (Phase 2, Task 2.4) — replacing all of
 * `mock/queries.ts`.
 *
 * `listHistory` is no longer a fixture: every successful `run` is recorded
 * by the backend (`routes::query::run` -> `lakehouse_store::queries::record_history`),
 * so the history shown is genuine execution history.
 */

/** Map an error response body onto the ServiceError code its status implies. */
function errorFor(status: number, message: string): ServiceError {
  if (status === 404) return new ServiceError("not_found", message);
  if (status === 400 || status === 409 || status === 422)
    return new ServiceError("invalid_request", message);
  if (status === 401 || status === 403)
    return new ServiceError("permission_denied", message);
  return new ServiceError("unavailable", message);
}

async function get<T>(url: string, signal?: AbortSignal): Promise<T> {
  const res = await apiFetch(url, { signal });
  const json = await res.json().catch(() => null);
  if (!res.ok) throw errorFor(res.status, json?.error ?? `Query failed (${res.status})`);
  return json as T;
}

async function postJson<T>(url: string, sql: string, signal?: AbortSignal): Promise<T> {
  const res = await apiFetch(url, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ sql }),
    signal,
  });
  const json = await res.json().catch(() => null);
  if (!res.ok) {
    throw errorFor(res.status, json?.error ?? `Query failed (${res.status})`);
  }
  return json as T;
}

export const clickhouseQueryService: QueryService = {
  // ── real (ClickHouse) ──────────────────────────────────────────────────
  run(sql, signal) {
    return postJson<QueryResult>("/api/query/run", sql, signal);
  },
  estimate(sql, signal) {
    return postJson<QueryEstimate>("/api/query/estimate", sql, signal);
  },

  // ── real (agent text-to-SQL, LLM grounded on the lakehouse schema) ─────
  async generateSql(question, signal) {
    const res = await apiFetch("/api/agent/text-to-sql", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ question }),
      signal,
    });
    const json = await res.json();
    if (!res.ok) {
      throw new ServiceError("unavailable", json?.detail ?? json?.error ?? "Agent unavailable");
    }
    return { sql: json.sql, explanation: json.explanation ?? "", assumptions: json.assumptions ?? [] };
  },

  // ── real (Postgres) ────────────────────────────────────────────────────
  listSaved(signal) {
    return get<SavedQuery[]>("/api/query/saved", signal);
  },
  listHistory(signal) {
    return get<QueryHistoryItem[]>("/api/query/history", signal);
  },
};
