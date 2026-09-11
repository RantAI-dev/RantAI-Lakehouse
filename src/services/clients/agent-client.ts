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

export async function askAgentSql(question: string, signal?: AbortSignal): Promise<AgentQueryResult> {
  const res = await apiFetch("/api/agent/query", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ question }),
    signal,
  });
  const json = await res.json();
  if (!res.ok) {
    throw new ServiceError("unavailable", json?.detail ?? json?.hint ?? json?.error ?? "Agent failed");
  }
  return json as AgentQueryResult;
}
