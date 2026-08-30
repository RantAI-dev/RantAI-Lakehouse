import type {
  QueryService,
  QueryResult,
  QueryEstimate,
  SavedQuery,
  QueryHistoryItem,
  CollaborationProject,
  CreateCollaborationProjectInput,
} from "../contracts/queries";
import { apiFetch } from "../http";
import { ServiceError } from "../errors";

/**
 * QueryService NYATA — `run`/`estimate` mengeksekusi SQL di ClickHouse
 * (lakehouse kita) lewat route server `/api/query/*`; `generateSql` lewat
 * `/api/agent/text-to-sql` (LLM di-grounding ke skema lakehouse, Fase 1).
 * `listSaved`/`listHistory`/`listCollaboration`/`createCollaborationProject`
 * kini NYATA juga, tersimpan di Postgres lewat crate `lakehouse-store`
 * (Fase 2, Task 2.4) — menggantikan seluruh `mock/queries.ts`.
 *
 * `listHistory` bukan lagi fixture: setiap `run` yang sukses dicatat oleh
 * backend (`routes::query::run` -> `lakehouse_store::queries::record_history`),
 * jadi riwayat yang tampil adalah eksekusi sungguhan.
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
  if (!res.ok) throw errorFor(res.status, json?.error ?? `Query gagal (${res.status})`);
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
    throw errorFor(res.status, json?.error ?? `Query gagal (${res.status})`);
  }
  return json as T;
}

async function post<T>(url: string, body: unknown, signal?: AbortSignal): Promise<T> {
  const res = await apiFetch(url, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
    signal,
  });
  const json = await res.json().catch(() => null);
  if (!res.ok) {
    throw errorFor(res.status, json?.error ?? `Query gagal (${res.status})`);
  }
  return json as T;
}

export const clickhouseQueryService: QueryService = {
  // ── NYATA (ClickHouse) ─────────────────────────────────────────────────
  run(sql, signal) {
    return postJson<QueryResult>("/api/query/run", sql, signal);
  },
  estimate(sql, signal) {
    return postJson<QueryEstimate>("/api/query/estimate", sql, signal);
  },

  // ── NYATA (agent text-to-SQL, LLM di-grounding ke skema lakehouse) ──────
  async generateSql(question, signal) {
    const res = await apiFetch("/api/agent/text-to-sql", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ question }),
      signal,
    });
    const json = await res.json();
    if (!res.ok) {
      throw new ServiceError("unavailable", json?.detail ?? json?.error ?? "Agent tak tersedia");
    }
    return { sql: json.sql, explanation: json.explanation ?? "", assumptions: json.assumptions ?? [] };
  },

  // ── NYATA (Postgres) ─────────────────────────────────────────────────────
  listSaved(signal) {
    return get<SavedQuery[]>("/api/query/saved", signal);
  },
  listHistory(signal) {
    return get<QueryHistoryItem[]>("/api/query/history", signal);
  },
  listCollaboration(signal) {
    return get<CollaborationProject[]>("/api/query/collaboration", signal);
  },
  createCollaborationProject(input: CreateCollaborationProjectInput, signal) {
    return post<CollaborationProject>("/api/query/collaboration", input, signal);
  },
};
