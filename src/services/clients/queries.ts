import type {
  QueryService,
  QueryResult,
  QueryEstimate,
} from "../contracts/queries";
import { mockQueryService } from "../mock/queries";
import { ServiceError } from "../errors";

/**
 * QueryService NYATA — `run`/`estimate` mengeksekusi SQL di ClickHouse
 * (lakehouse kita) lewat route server `/api/query/*`. Sisanya (saved/history/
 * collaboration/generateSql) sementara masih memakai adapter mock sampai
 * fase berikutnya (persistensi Postgres + NL→SQL via llm-node).
 */

async function postJson<T>(url: string, sql: string, signal?: AbortSignal): Promise<T> {
  const res = await fetch(url, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ sql }),
    signal,
  });
  const json = await res.json();
  if (!res.ok) {
    throw new ServiceError("unavailable", json?.error ?? `Query gagal (${res.status})`);
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
    const res = await fetch("/api/agent/text-to-sql", {
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

  // ── Sementara delegasi ke mock (fase berikutnya dibuat nyata) ───────────
  listSaved: (signal) => mockQueryService.listSaved(signal),
  listHistory: (signal) => mockQueryService.listHistory(signal),
  listCollaboration: (signal) => mockQueryService.listCollaboration(signal),
  createCollaborationProject: (input, signal) =>
    mockQueryService.createCollaborationProject(input, signal),
};
