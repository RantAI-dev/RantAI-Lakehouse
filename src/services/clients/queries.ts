import type {
  QueryService,
  SaveQueryInput,
  UpdateSavedQueryInput,
  CreateFolderInput,
  UpdateFolderInput,
  SavedQueryAccessInput,
  SavedQueryFilter,
  QueryResult,
  QueryEstimate,
  QueryDownload,
  QueryDownloadFormat,
  SavedQuery,
  SavedQueryAccessList,
  SavedQueryTrashEntry,
  SavedQueryVersion,
  QueryFolder,
  GranteeList,
  QueryHistoryFilter,
  QueryHistoryPage,
  QuerySchedulingCapability,
  NlAnswer,
  QueryEngine,
  SqlFixOutcome,
} from "../contracts/queries";
import { historyQueryString } from "@/lib/history-query-params";
import { readableErrorMessage } from "@/lib/notify-message";
import { apiFetch } from "../http";
import { ServiceError } from "../errors";
import { chatAnswerFromBody } from "./chat-answer";
import { RUN_ID_HEADER } from "@/lib/run-id";
import { fileNameOf, refusalOf } from "@/lib/browser-file";

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

/**
 * Map an error response body onto the ServiceError code its status implies.
 *
 * The message is the server's own sentence, with the API's own code
 * prefix (`permission_denied: `) taken off: the code is for the log,
 * the sentence is for the reader (`readableErrorMessage`, `QS-3` C6).
 */
function errorFor(status: number, message: string): ServiceError {
  const sentence = readableErrorMessage(message);
  if (status === 404) return new ServiceError("not_found", sentence);
  if (status === 400 || status === 409 || status === 422)
    return new ServiceError("invalid_request", sentence);
  if (status === 401 || status === 403)
    return new ServiceError("permission_denied", sentence);
  return new ServiceError("unavailable", sentence);
}

/**
 * One call, and the answer it carries: the JSON body when the call
 * went through, the refusal as a `ServiceError` when it did not. Every
 * method here goes through this one, so every route of this client
 * reads its refusal the same way.
 */
async function send<T>(url: string, init: RequestInit, signal?: AbortSignal): Promise<T> {
  const res = await apiFetch(url, { ...init, signal });
  const json = await res.json().catch(() => null);
  if (!res.ok) {
    throw errorFor(res.status, json?.error ?? `Query failed (${res.status})`);
  }
  return json as T;
}

async function get<T>(url: string, signal?: AbortSignal): Promise<T> {
  return send<T>(url, {}, signal);
}

async function postJson<T>(
  url: string,
  body: Record<string, unknown>,
  signal?: AbortSignal,
  extraHeaders?: Record<string, string>
): Promise<T> {
  return send<T>(
    url,
    {
      method: "POST",
      headers: { "Content-Type": "application/json", ...extraHeaders },
      body: JSON.stringify(body),
    },
    signal,
  );
}

/** A change that answers with the thing it changed (`PATCH`). */
async function patchJson<T>(
  url: string,
  body: Record<string, unknown>,
  signal?: AbortSignal
): Promise<T> {
  return send<T>(
    url,
    {
      method: "PATCH",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(body),
    },
    signal,
  );
}

/** A change that answers with the thing it changed (`PUT`). */
async function putJson<T>(
  url: string,
  body: Record<string, unknown>,
  signal?: AbortSignal
): Promise<T> {
  return send<T>(
    url,
    {
      method: "PUT",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(body),
    },
    signal,
  );
}

/**
 * A removal that answers with nothing: the server sends 204 and an empty
 * body, so there is no JSON to read and none is asked for.
 */
async function deleteEmpty(url: string, signal?: AbortSignal): Promise<void> {
  const res = await apiFetch(url, { method: "DELETE", signal });
  if (!res.ok) {
    const json = await res.json().catch(() => null);
    throw errorFor(res.status, json?.error ?? `Query failed (${res.status})`);
  }
}

/**
 * The query string the saved-query list takes (`GET /api/query/saved`):
 * the text typed in the search box and the folder asked for, each
 * dropped when it is empty (the server reads an empty value as "no
 * filter"), and no `?` at all when both are empty.
 */
function savedQueryQueryString(filter: SavedQueryFilter): string {
  const params = new URLSearchParams();
  if (filter.q) params.set("q", filter.q);
  if (filter.folder) params.set("folder", filter.folder);
  return params.toString();
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
      json?.detail ?? json?.error ?? "AI Copilot could not answer. Try again."
    );
  }
  // A 200 whose body is not a JSON object (a proxy's HTML page, say) would
  // otherwise read as an empty answer and show the user nothing, with no error.
  if (json === null || typeof json !== "object" || Array.isArray(json)) {
    throw new ServiceError("unavailable", "AI Copilot could not answer. Try again.");
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
    throw new ServiceError("invalid_request", "This query is too long for AI Copilot to fix.");
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


/**
 * Download the file of one run (`GET /api/query/run/{id}/download`).
 *
 * The server re-runs the stored statement with the caller's current masks and
 * row filters, so the file is the result as it is now, not as it was when the
 * run happened. `runId` names the run the download belongs to: the download's
 * own query on the engine carries it, so the Stop button reaches it.
 *
 * The response says what the file holds in its headers (`X-Export-Rows`,
 * `X-Export-Cut`, `X-Export-Cut-By`), so a cut is never silent. A refusal
 * carries the server's own sentence, thrown as a `ServiceError` whose code
 * the status implies.
 */
async function download(
  id: string,
  format: QueryDownloadFormat,
  runId: string,
  signal?: AbortSignal
): Promise<QueryDownload> {
  const query = new URLSearchParams({ format, runId });
  const res = await apiFetch(
    `/api/query/run/${encodeURIComponent(id)}/download?${query.toString()}`,
    { signal, cache: "no-store" }
  );
  if (!res.ok) {
    throw errorFor(res.status, await refusalOf(res, "The download failed."));
  }
  const blob = await res.blob();
  // The fallback name carries the format's own extension: a file named
  // `query.csv` that holds Parquet bytes will not open.
  const fileName = fileNameOf(res.headers.get("content-disposition"), `query.${format}`);
  const rowsHeader = res.headers.get("x-export-rows");
  const rows = rowsHeader === null ? Number.NaN : Number(rowsHeader);
  const cutBy = res.headers.get("x-export-cut-by");
  return {
    blob,
    fileName,
    // The engine-written file carries no row count: the server never counted
    // the rows it wrote, so the console does not invent one.
    rows: Number.isFinite(rows) ? rows : null,
    cut: res.headers.get("x-export-cut") === "true",
    cutBy: cutBy === "rows" || cutBy === "bytes" ? cutBy : null,
  };
}

/** `GET /api/query/history`: one numbered page with the total (QS-2/QS-4 QA of 2026-11-05). */
function getHistoryPage(filter?: QueryHistoryFilter, signal?: AbortSignal): Promise<QueryHistoryPage> {
  const query = historyQueryString(filter);
  return get<QueryHistoryPage>(`/api/query/history${query ? `?${query}` : ""}`, signal);
}

export const clickhouseQueryService: QueryService = {
  // ── real (ClickHouse / Trino) ────────────────────────────────────────────
  run(sql, options, signal) {
    return postJson<QueryResult>(
      "/api/query/run",
      {
        sql,
        engine: options.engine,
        // Left out, not sent as `null`, when absent: a statement with no
        // placeholder posts the body it posted before `QS-5`.
        ...(options.parameters !== undefined && { parameters: options.parameters }),
        ...(options.definitions !== undefined && { definitions: options.definitions }),
      },
      signal,
      options.runId ? { [RUN_ID_HEADER]: options.runId } : undefined
    );
  },
  cancel(runId, signal) {
    return postJson<{ stopped: number }>("/api/query/cancel", { runId }, signal);
  },
  download,
  estimate(sql, options, signal) {
    return postJson<QueryEstimate>(
      "/api/query/estimate",
      {
        sql,
        ...(options?.parameters !== undefined && { parameters: options.parameters }),
        ...(options?.definitions !== undefined && { definitions: options.definitions }),
      },
      signal
    );
  },

  // ── real (the copilot's chat engine) ───────────────────────────────────
  askQuestion,
  explainSql,
  proposeFix,
  async generateSql(question, signal) {
    // Same call as `askQuestion`: the query runs once on the server on the way.
    const out = await askQuestion(question, signal);
    if (out.sql === undefined) {
      throw new ServiceError("unavailable", out.answer || "AI Copilot ran no query.");
    }
    return { sql: out.sql, explanation: out.answer };
  },

  // ── real (Postgres) ────────────────────────────────────────────────────
  listSaved(signal) {
    return get<SavedQuery[]>("/api/query/saved", signal);
  },
  // ── the saved-query console (QS-3) ─────────────────────────────────────
  listSavedFiltered(filter, signal) {
    const query = savedQueryQueryString(filter);
    return get<SavedQuery[]>(`/api/query/saved${query ? `?${query}` : ""}`, signal);
  },
  getSaved(id, signal) {
    return get<SavedQuery>(`/api/query/saved/${encodeURIComponent(id)}`, signal);
  },
  saveQuery(input: SaveQueryInput, signal) {
    return postJson<SavedQuery>("/api/query/saved", input, signal);
  },
  updateSaved(id, input: UpdateSavedQueryInput, signal) {
    return patchJson<SavedQuery>(`/api/query/saved/${encodeURIComponent(id)}`, input, signal);
  },
  deleteSaved(id, signal) {
    return deleteEmpty(`/api/query/saved/${encodeURIComponent(id)}`, signal);
  },
  duplicateSaved(id, signal) {
    return send<SavedQuery>(
      `/api/query/saved/${encodeURIComponent(id)}/duplicate`,
      { method: "POST" },
      signal,
    );
  },
  restoreSaved(id, signal) {
    return send<SavedQuery>(
      `/api/query/saved/${encodeURIComponent(id)}/restore`,
      { method: "POST" },
      signal,
    );
  },
  listSavedTrash(signal) {
    return get<SavedQueryTrashEntry[]>("/api/query/saved-trash", signal);
  },
  listSavedVersions(id, signal) {
    return get<SavedQueryVersion[]>(
      `/api/query/saved/${encodeURIComponent(id)}/versions`,
      signal,
    );
  },
  revertSavedVersion(id, version, signal) {
    return send<SavedQuery>(
      `/api/query/saved/${encodeURIComponent(id)}/versions/${encodeURIComponent(String(version))}/revert`,
      { method: "POST" },
      signal,
    );
  },
  listFolders(signal) {
    return get<QueryFolder[]>("/api/query/folders", signal);
  },
  createFolder(input: CreateFolderInput, signal) {
    return postJson<QueryFolder>("/api/query/folders", input, signal);
  },
  updateFolder(id, input: UpdateFolderInput, signal) {
    return patchJson<QueryFolder>(`/api/query/folders/${encodeURIComponent(id)}`, input, signal);
  },
  deleteFolder(id, signal) {
    return deleteEmpty(`/api/query/folders/${encodeURIComponent(id)}`, signal);
  },
  getSavedAccess(id, signal) {
    return get<SavedQueryAccessList>(`/api/query/saved/${encodeURIComponent(id)}/access`, signal);
  },
  putSavedAccess(id, input: SavedQueryAccessInput, signal) {
    return putJson<SavedQueryAccessList>(
      `/api/query/saved/${encodeURIComponent(id)}/access`,
      input,
      signal,
    );
  },
  getFolderAccess(id, signal) {
    return get<SavedQueryAccessList>(`/api/query/folders/${encodeURIComponent(id)}/access`, signal);
  },
  putFolderAccess(id, input: SavedQueryAccessInput, signal) {
    return putJson<SavedQueryAccessList>(
      `/api/query/folders/${encodeURIComponent(id)}/access`,
      input,
      signal,
    );
  },
  findGrantees(query, signal) {
    const params = new URLSearchParams();
    if (query) params.set("q", query);
    const search = params.toString();
    return get<GranteeList>(`/api/query/grantees${search ? `?${search}` : ""}`, signal);
  },
  async listHistory(filter, signal) {
    return (await getHistoryPage(filter, signal)).runs;
  },
  listHistoryPage(filter, signal) {
    return getHistoryPage(filter, signal);
  },
  getSchedulingCapability(signal) {
    return get<QuerySchedulingCapability>("/api/query/scheduling", signal);
  },
};
