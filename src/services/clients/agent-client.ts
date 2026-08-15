import { ServiceError } from "../errors";

/**
 * Klien agent untuk UI (client-side). Memanggil route agentic self-correcting
 * `/api/agent/query`. Bukan bagian QueryService contract (kapabilitas tambahan),
 * jadi diimpor langsung oleh halaman Query Studio.
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
  const res = await fetch("/api/agent/query", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ question }),
    signal,
  });
  const json = await res.json();
  if (!res.ok) {
    throw new ServiceError("unavailable", json?.detail ?? json?.hint ?? json?.error ?? "Agent gagal");
  }
  return json as AgentQueryResult;
}
