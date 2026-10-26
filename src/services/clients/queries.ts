import type {
  QueryService,
  SaveQueryInput,
  QueryResult,
  QueryEstimate,
  SavedQuery,
  QueryHistoryItem,
  QuerySchedulingCapability,
  NlAnswer,
  QueryEngine,
  SqlFixOutcome,
} from "../contracts/queries";
import { historyQueryString } from "@/lib/history-query-params";
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

/**
 * One call to the chat's engine with the given messages and tool allowlist.
 * No `stream` (one JSON object back) and no `mode` (no writing tool is
 * offered); `tools` is an allowlist, so each caller gets only what it names.
 */
async function postChat(
  messages: { role: "user"; content: string }[],
  tools: string[],
  signal?: AbortSignal
): Promise<Record<string, unknown>> {
  const res = await apiFetch("/api/ai/chat", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ messages, tools }),
    signal,
  });
  const json = await res.json().catch(() => null);
  if (!res.ok) {
    throw new ServiceError(
      "unavailable",
      json?.detail ?? json?.error ?? "The assistant could not answer. Try again."
    );
  }
  // A 200 whose body is not a JSON object (a proxy's HTML page, say) would
  // otherwise read as an empty answer and show the user nothing, with no error.
  if (json === null || typeof json !== "object" || Array.isArray(json)) {
    throw new ServiceError("unavailable", "The assistant could not answer. Try again.");
  }
  return json as Record<string, unknown>;
}

async function askQuestion(question: string, signal?: AbortSignal): Promise<NlAnswer> {
  // `tools` is an allowlist, so the box can read the lakehouse and nothing else.
  const json = await postChat(
    [{ role: "user", content: question }],
    ["run_sql", "list_datasets", "describe_dataset", "lakehouse_overview"],
    signal
  );
  return chatAnswerFromBody(json);
}

/** One tool call in a chat reply's `toolTrace`. */
type TraceStep = { tool?: unknown; ok?: unknown; result?: unknown };

/** The chat's limit on a statement it is asked to fix, in code points as the server counts. */
const MAX_FIX_SQL_CHARS = 20_000;

async function explainSql(
  sql: string,
  engine: QueryEngine,
  signal?: AbortSignal
): Promise<string> {
  const json = await postChat(
    [
      {
        role: "user",
        content: `Explain what this query does.\n\nEngine: ${engine}\nSQL:\n${sql}`,
      },
    ],
    ["propose_sql"],
    signal
  );
  return typeof json.answer === "string" ? json.answer : "";
}

async function proposeFix(
  sql: string,
  engine: QueryEngine,
  error: string,
  signal?: AbortSignal
): Promise<SqlFixOutcome> {
  if ([...sql].length > MAX_FIX_SQL_CHARS) {
    throw new ServiceError("invalid_request", "This query is too long for the assistant to fix.");
  }
  const json = await postChat(
    [
      {
        role: "user",
        content: `Why does this query fail? Fix it.\n\nEngine: ${engine}\nSQL:\n${sql}\nError:\n${error}`,
      },
    ],
    ["propose_sql"],
    signal
  );
  // The last accepted proposal wins: an earlier one the check refused
  // (`ok: false`) is the model correcting itself, not the answer.
  const trace = Array.isArray(json.toolTrace) ? (json.toolTrace as TraceStep[]) : [];
  const accepted = trace.findLast((s) => s.tool === "propose_sql" && s.ok === true);
  const result = accepted?.result as { sql?: unknown; reason?: unknown } | undefined;
  if (typeof result?.sql === "string" && typeof result.reason === "string") {
    return { sql: result.sql, reason: result.reason };
  }
  return { answer: typeof json.answer === "string" ? json.answer : "" };
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
  explainSql,
  proposeFix,
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
  listHistory(filter, signal) {
    const query = historyQueryString(filter);
    return get<QueryHistoryItem[]>(`/api/query/history${query ? `?${query}` : ""}`, signal);
  },
  getSchedulingCapability(signal) {
    return get<QuerySchedulingCapability>("/api/query/scheduling", signal);
  },
};
