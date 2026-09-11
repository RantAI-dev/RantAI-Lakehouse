import type {
  AgentRun,
  AgentService,
  ApprovalItem,
  CreateEmployeeInput,
  DecideApprovalInput,
  DecideApprovalResult,
  DigitalEmployee,
  RunEmployeeInput,
} from "../contracts/agents"
import { apiFetch } from "../http"
import { ServiceError } from "../errors"

/**
 * AgentService is real — digital employee definitions, tools, workflows,
 * run history, and the approval cycle are all stored in Postgres via the
 * `/api/agents/*` route (crate `lakehouse-store`, Task 2.9).
 *
 * UPDATE (T3.2/T3.4, copilot-operations-handover plan): there is now a
 * headless execution runtime — `runEmployee` calls `POST
 * /api/agents/employees/{id}/run`, which actually runs the copilot's
 * tool-calling loop for one digital employee without interactive
 * supervision, writing real `agent_run`/`steps`/`audit_event` rows.
 * `listRuns`/`getRun` can now surface run history that actually happened,
 * not just seed data — see migration `0026_drop_seeded_agent_runs.sql`,
 * which removed the old seed runs/approvals so this page would not
 * mislead.
 */

function errorFor(status: number, message: string): ServiceError {
  if (status === 404) return new ServiceError("not_found", message)
  if (status === 400 || status === 409 || status === 422)
    return new ServiceError("invalid_request", message)
  if (status === 401 || status === 403) return new ServiceError("permission_denied", message)
  return new ServiceError("unavailable", message)
}

async function request<T>(url: string, init: RequestInit, fallback: string): Promise<T> {
  const res = await apiFetch(url, init)
  const json = await res.json().catch(() => null)
  if (!res.ok) {
    throw errorFor(res.status, json?.error ?? fallback)
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
  listEmployees(signal) {
    return get<DigitalEmployee[]>(
      "/api/agents/employees",
      signal,
      "Failed to load digital employee list"
    )
  },
  getEmployee(id, signal) {
    return get<DigitalEmployee>(
      `/api/agents/employees/${encodeURIComponent(id)}`,
      signal,
      "Failed to load digital employee detail"
    )
  },
  listRuns(employeeId, signal) {
    const qs = employeeId ? `?employeeId=${encodeURIComponent(employeeId)}` : ""
    return get<AgentRun[]>(`/api/agents/runs${qs}`, signal, "Failed to load run list")
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
  createEmployee(input: CreateEmployeeInput, signal) {
    return post<DigitalEmployee>(
      "/api/agents/employees",
      input,
      signal,
      "Failed to create digital employee"
    )
  },
  suspendEmployee(id, signal) {
    return post<DigitalEmployee>(
      `/api/agents/employees/${encodeURIComponent(id)}/suspend`,
      undefined,
      signal,
      "Failed to suspend digital employee"
    )
  },
  resumeEmployee(id, signal) {
    return post<DigitalEmployee>(
      `/api/agents/employees/${encodeURIComponent(id)}/resume`,
      undefined,
      signal,
      "Failed to resume digital employee"
    )
  },
  revokeEmployee(id, signal) {
    return post<DigitalEmployee>(
      `/api/agents/employees/${encodeURIComponent(id)}/revoke`,
      undefined,
      signal,
      "Failed to revoke digital employee"
    )
  },
  async runEmployee(id, input: RunEmployeeInput | undefined, signal) {
    const body = input?.prompt?.trim() ? { prompt: input.prompt.trim() } : undefined
    const { run } = await post<{ run: AgentRun }>(
      `/api/agents/employees/${encodeURIComponent(id)}/run`,
      body,
      signal,
      "Failed to run digital employee"
    )
    return run
  },
}
