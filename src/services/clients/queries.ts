import type {
  QueryService,
  SaveQueryInput,
  QueryResult,
  QueryEstimate,
  SavedQuery,
  QueryHistoryItem,
  QuerySchedulingCapability,
  NlAnswer,
} from "../contracts/queries";
import { apiFetch } from "../http";
import { ServiceError } from "../errors";
import { chatAnswerFromBody } from "./chat-answer";

/**
 * QueryService is real — `run`/`estimate` execute SQL against ClickHouse
 * (our lakehouse) via the server route `/api/query/*`; `askQuestion` and
 * `generateSql` go through `/api/ai/chat`, the copilot's engine, so the SQL
 * they run is masked and row-filtered for the caller. `listSaved`/`listHistory`
 * are now real too, stored in Postgres
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

async function postJson<T>(
  url: string,
  body: Record<string, unknown>,
  signal?: AbortSignal
): Promise<T> {
  const res = await apiFetch(url, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
    signal,
  });
  const json = await res.json().catch(() => null);
  if (!res.ok) {
    throw errorFor(res.status, json?.error ?? `Query failed (${res.status})`);
  }
  return json as T;
}

async function askQuestion(question: string, signal?: AbortSignal): Promise<NlAnswer> {
  const res = await apiFetch("/api/ai/chat", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    // No `stream` (one JSON object back) and no `mode` (no writing tool
    // is offered); `tools` is an allowlist, so the box can read the
    // lakehouse and nothing else.
    body: JSON.stringify({
      messages: [{ role: "user", content: question }],
      tools: ["run_sql", "list_datasets", "describe_dataset", "lakehouse_overview"],
    }),
    signal,
  });
  const json = await res.json().catch(() => null);
  if (!res.ok) {
    throw new ServiceError(
      "unavailable",
      json?.detail ?? json?.error ?? "The assistant could not answer. Try again."
    );
  }
  return chatAnswerFromBody(json);
}

export const clickhouseQueryService: QueryService = {
  // ── real (ClickHouse / Trino) ────────────────────────────────────────────
  run(sql, options, signal) {
    return postJson<QueryResult>("/api/query/run", { sql, engine: options.engine }, signal);
  },
  estimate(sql, signal) {
    return postJson<QueryEstimate>("/api/query/estimate", { sql }, signal);
  },

  // ── real (the copilot's chat engine) ───────────────────────────────────
  askQuestion,
  async generateSql(question, signal) {
    // Same call as `askQuestion`: the query runs once on the server on the way.
    const out = await askQuestion(question, signal);
    if (out.sql === undefined) {
      throw new ServiceError("unavailable", out.answer || "The assistant ran no query.");
    }
    return { sql: out.sql, explanation: out.answer };
  },

  // ── real (Postgres) ────────────────────────────────────────────────────
  listSaved(signal) {
    return get<SavedQuery[]>("/api/query/saved", signal);
  },
  saveQuery(input: SaveQueryInput, signal) {
    return postJson<SavedQuery>("/api/query/saved", input, signal);
  },
  listHistory(signal) {
    return get<QueryHistoryItem[]>("/api/query/history", signal);
  },
  getSchedulingCapability(signal) {
    return get<QuerySchedulingCapability>("/api/query/scheduling", signal);
  },
};
