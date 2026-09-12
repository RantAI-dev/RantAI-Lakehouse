import type {
  AgentRun,
  AgentService,
  AgentTool,
  AgentWorkflow,
  ApprovalItem,
  CreateEmployeeInput,
  CreateWorkflowInput,
  DecideApprovalInput,
  DecideApprovalResult,
  DigitalEmployee,
  RegisterToolInput,
  RunEmployeeInput,
} from "../contracts/agents"
import { apiFetch } from "../http"
import { ServiceError } from "../errors"

/**
 * AgentService NYATA — definisi digital employee, tools, workflows,
 * riwayat run, dan siklus approval semuanya tersimpan di Postgres lewat
 * route `/api/agents/*` (crate `lakehouse-store`, Task 2.9).
 *
 * UPDATE (T3.2/T3.4, copilot-operations-handover plan): ada runtime
 * eksekusi headless sekarang — `runEmployee` memanggil `POST
 * /api/agents/employees/{id}/run`, yang benar-benar menjalankan loop
 * tool-calling copilot untuk satu digital employee tanpa pengawasan
 * interaktif, menulis `agent_run`/`steps`/`audit_event` yang nyata.
 * `listRuns`/`getRun` sekarang bisa menyajikan riwayat run yang benar-
 * benar terjadi, bukan cuma seed — lihat migrasi `0026_drop_seeded_
 * agent_runs.sql`, yang menghapus seed run/approval lama supaya halaman
 * ini tidak menyesatkan.
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
  listWorkflows(signal) {
    return get<AgentWorkflow[]>("/api/agents/workflows", signal, "Daftar workflow gagal dimuat")
  },
  listEmployees(signal) {
    return get<DigitalEmployee[]>(
      "/api/agents/employees",
      signal,
      "Daftar digital employee gagal dimuat"
    )
  },
  getEmployee(id, signal) {
    return get<DigitalEmployee>(
      `/api/agents/employees/${encodeURIComponent(id)}`,
      signal,
      "Detail digital employee gagal dimuat"
    )
  },
  listRuns(employeeId, signal) {
    const qs = employeeId ? `?employeeId=${encodeURIComponent(employeeId)}` : ""
    return get<AgentRun[]>(`/api/agents/runs${qs}`, signal, "Daftar run gagal dimuat")
  },
  getRun(id, signal) {
    return get<AgentRun>(`/api/agents/runs/${encodeURIComponent(id)}`, signal, "Detail run gagal dimuat")
  },
  listTools(signal) {
    return get<AgentTool[]>("/api/agents/tools", signal, "Daftar tools gagal dimuat")
  },
  listApprovals(employeeId, signal) {
    const qs = employeeId ? `?employeeId=${encodeURIComponent(employeeId)}` : ""
    return get<ApprovalItem[]>(`/api/agents/approvals${qs}`, signal, "Daftar approval gagal dimuat")
  },
  decideApproval(id, input: DecideApprovalInput, signal) {
    return post<DecideApprovalResult>(
      `/api/agents/approvals/${encodeURIComponent(id)}/decide`,
      input,
      signal,
      "Memutuskan approval gagal"
    )
  },
  createWorkflow(input: CreateWorkflowInput, signal) {
    return post<AgentWorkflow>("/api/agents/workflows", input, signal, "Membuat workflow gagal")
  },
  createEmployee(input: CreateEmployeeInput, signal) {
    return post<DigitalEmployee>(
      "/api/agents/employees",
      input,
      signal,
      "Membuat digital employee gagal"
    )
  },
  registerTool(input: RegisterToolInput, signal) {
    return post<AgentTool>("/api/agents/tools", input, signal, "Mendaftarkan tool gagal")
  },
  suspendEmployee(id, signal) {
    return post<DigitalEmployee>(
      `/api/agents/employees/${encodeURIComponent(id)}/suspend`,
      undefined,
      signal,
      "Menangguhkan digital employee gagal"
    )
  },
  resumeEmployee(id, signal) {
    return post<DigitalEmployee>(
      `/api/agents/employees/${encodeURIComponent(id)}/resume`,
      undefined,
      signal,
      "Melanjutkan digital employee gagal"
    )
  },
  revokeEmployee(id, signal) {
    return post<DigitalEmployee>(
      `/api/agents/employees/${encodeURIComponent(id)}/revoke`,
      undefined,
      signal,
      "Mencabut digital employee gagal"
    )
  },
  async runEmployee(id, input: RunEmployeeInput | undefined, signal) {
    const body = input?.prompt?.trim() ? { prompt: input.prompt.trim() } : undefined
    const { run } = await post<{ run: AgentRun }>(
      `/api/agents/employees/${encodeURIComponent(id)}/run`,
      body,
      signal,
      "Menjalankan digital employee gagal"
    )
    return run
  },
}
