import type {
  AgentRun,
  AgentService,
  ApprovalItem,
  DecideApprovalInput,
  DecideApprovalResult,
  DigitalEmployee,
} from "../contracts/agents"
import { apiFetch } from "../http"
import { ServiceError, deniedFor } from "../errors"

/**
 * AgentService is real: employee definitions, run history and the approval
 * cycle are stored in Postgres via the `/api/agents/*` route (crate
 * `lakehouse-store`, Task 2.9).
 *
 * `WEB-3` slice 1 (D1/D2/D3) removed the employees list, employee detail and
 * runs list — and with them the employee create/suspend/resume/revoke/run
 * actions and the two list reads. What remains is what live pages read: a
 * run's own trace (`/agents/runs/[runId]`, reachable from an approval, D3)
 * and the approvals inbox. The backend routes stay registered (D2): the
 * assistant's tools still use them, and `WEB-4` retires them.
 */

function errorFor(status: number, body: { error?: string } | null, fallback: string): ServiceError {
  // WEB-12: a refusal goes through deniedFor (errors.ts).
  const denied = deniedFor(status, body)
  if (denied) return denied
  const message = body?.error ?? fallback
  if (status === 404) return new ServiceError("not_found", message)
  if (status === 400 || status === 409 || status === 422)
    return new ServiceError("invalid_request", message)
  return new ServiceError("unavailable", message)
}

async function request<T>(url: string, init: RequestInit, fallback: string): Promise<T> {
  const res = await apiFetch(url, init)
  const json = await res.json().catch(() => null)
  if (!res.ok) {
    throw errorFor(res.status, json, fallback)
  }
  return json as T
}

function get<T>(url: string, signal: AbortSignal | undefined, fallback: string): Promise<T> {
  return request<T>(url, { signal }, fallback)
}

function post<T>(
  url: string,
  body: unknown,
  signal: AbortSignal | undefined,
  fallback: string
): Promise<T> {
  return request<T>(
    url,
    {
      method: "POST",
      headers: body === undefined ? undefined : { "content-type": "application/json" },
      body: body === undefined ? undefined : JSON.stringify(body),
      signal,
    },
    fallback
  )
}

export const postgresAgentService: AgentService = {
  getEmployee(id, signal) {
    return get<DigitalEmployee>(
      `/api/agents/employees/${encodeURIComponent(id)}`,
      signal,
      "Failed to load digital employee detail"
    )
  },
  getRun(id, signal) {
    return get<AgentRun>(`/api/agents/runs/${encodeURIComponent(id)}`, signal, "Failed to load run detail")
  },
  listApprovals(employeeId, signal) {
    const qs = employeeId ? `?employeeId=${encodeURIComponent(employeeId)}` : ""
    return get<ApprovalItem[]>(`/api/agents/approvals${qs}`, signal, "Failed to load approval list")
  },
  decideApproval(id, input: DecideApprovalInput, signal) {
    return post<DecideApprovalResult>(
      `/api/agents/approvals/${encodeURIComponent(id)}/decide`,
      input,
      signal,
      "Failed to decide approval"
    )
  },
}
