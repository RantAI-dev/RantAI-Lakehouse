import { apiFetch } from "../http";
import { ServiceError } from "../errors";

/**
 * Client-side agent client for the UI. Calls the self-correcting agentic
 * route `/api/agent/query`. Not part of the QueryService contract (an extra
 * capability), so it is imported directly by the Query Studio page.
 */

export type AgentStep = { step: string; detail: string };

export type AgentQueryResult = {
  question: string;
  sql: string;
  columns: string[];
  rows: Record<string, unknown>[];
  rowCount: number;
  answer: string;
  assumptions: string[];
  steps: AgentStep[];
};

/**
 * One readable sentence out of the three fields the route can send.
 *
 * `detail` alone was being shown, which for a transport failure is just
 * "error sending request for url (…)" — true, but it never says what the
 * user is looking at or what to do about it.
 */
function agentErrorMessage(json: unknown): string {
  const body = (json ?? {}) as { error?: string; detail?: string; hint?: string };
  const parts = [body.error, body.detail, body.hint].filter(
    (part): part is string => typeof part === "string" && part.trim().length > 0
  );
  return parts.length > 0 ? parts.join(" — ") : "the agent failed";
}

export async function askAgentSql(question: string, signal?: AbortSignal): Promise<AgentQueryResult> {
  const res = await apiFetch("/api/agent/query", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ question }),
    signal,
  });
  const json = await res.json();
  if (!res.ok) {
    throw new ServiceError("unavailable", agentErrorMessage(json));
  }
  return json as AgentQueryResult;
}
