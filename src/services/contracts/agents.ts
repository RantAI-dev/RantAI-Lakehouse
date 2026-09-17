import type { Measured } from "@/lib/measured"
import type {
  AgentRunStatus,
  ApprovalStatus,
  AutonomyLevel,
  EntityStatus,
  RunStepStatus,
} from "@/lib/status"

export type DigitalEmployee = {
  id: string
  name: string
  purpose: string
  owner: string
  autonomy: AutonomyLevel
  status: EntityStatus
  budgetLimit: number
  allowedTools: string[]
  dataScope: string
  /**
   * `budgetSpent`, `approvalRate`, `successRate` and `recentRuns` are
   * recomputed from real `agent_run`/`approval_item` rows at every run's
   * terminal transition (WS7 item G3) — real values on the DETAIL route
   * (`GET /api/agents/employees/{id}`), `null` on the LIST route (a list
   * of many employees does not pay for four extra aggregate columns
   * each; see `lakehouse_store::agents::EMPLOYEE_COLUMNS`'s doc comment)
   * and `null` for `successRate`/`approvalRate` specifically whenever an
   * employee has no qualifying terminal run/decided approval in the
   * 30-day window (an honest "no ratio to report", never a fabricated
   * `0`). `budgetReserved` stays `null` permanently — no multi-step
   * reservation/hold concept exists anywhere in this codebase.
   */
  budgetSpent: Measured
  budgetReserved: Measured
  approvalRate: Measured
  successRate: Measured
  recentRuns: Measured
  /**
   * The instruction a headless run (`POST
   * /api/agents/employees/{id}/run`) sends to the copilot. `null`/absent
   * means this employee has no prompt and cannot run without a
   * per-call override in the run request body.
   */
  prompt: string | null
  /** Cron expression for a Dagster schedule, or `null` for manual-only. */
  scheduleCron: string | null
  /** The copilot mode a run uses. */
  mode: "ask" | "build"
  /**
   * Ceiling on what this employee's runs may do, as a comma-separated
   * `resource:action` list (same format as `role.permissions`). Empty
   * string means authenticated-only, no permissioned tools — this is a
   * CEILING, not a grant: it never gives the triggering user (or the
   * schedule token) any permission they don't already have, it only
   * narrows what the run itself may call.
   */
  permissions: string
}

export type AgentRun = {
  id: string
  employeeId: string
  workflowId?: string
  status: AgentRunStatus
  trigger: string
  actor: string
  delegatedUser?: string
  startedAt: string
  endedAt?: string
  /**
   * Null until `WS7` tracks spend per tool call: `agent_run.budget_consumed`
   * exists but nothing ever updates it.
   */
  budgetConsumed: Measured
  steps: {
    id: string
    label: string
    status: RunStepStatus
    detail: string
  }[]
  approvals: { id: string; status: ApprovalStatus; at?: string }[]
  auditEventId?: string
}

/** Optional body for `POST /api/agents/employees/{id}/run`. */
export type RunEmployeeInput = {
  /** Overrides the employee's own `prompt` for this run only. */
  prompt?: string
}

export type ApprovalItem = {
  id: string
  employeeId: string
  employeeName: string
  runId?: string
  workflowId?: string
  action: string
  resource?: string
  reason?: string
  impact?: string
  evidence?: string[]
  policy?: string
  costEstimate?: number
  expiresAt?: string
  requestedAt: string
  status: ApprovalStatus
  risk: string
  decidedAt?: string
  comment?: string
  auditEventId?: string
  /**
   * `"tool_call" | "access"` (WS7 item E1, `0040_access_requests.sql`).
   * An access request (WS7 item E2) reads `permission` out of `action`
   * itself — the backend always writes `action = "access:<permission>"`
   * for a `kind = "access"` row (`lakehouse_store::agents::
   * create_access_request`) — never a separate field.
   */
  kind: "tool_call" | "access"
  /**
   * The human who requested this approval — set only for `kind =
   * "access"`, `undefined` for `kind = "tool_call"` (a digital employee
   * acted; no single human requester exists). Used to disable the decide
   * button for the requester's own pending request — see
   * `isOwnAccessRequest` in `@/lib/access-requests`. The server-side
   * refusal (`routes::catalog::decide_access_request`, WS7 item E3) is
   * what actually enforces self-approval refusal; this is belt-and-
   * suspenders only.
   */
  requestedByUserId?: string
}

export type DecideApprovalInput = {
  decision: "approved" | "rejected"
  comment?: string
}

/**
 * `POST /api/agents/approvals/{id}/decide`'s response (T0.5 of the
 * copilot-operations-handover plan): the decided approval, whether the
 * linked run's tool call actually executed, and — only when it did — the
 * tool's own result, so the UI can show what happened inline rather than
 * making the reviewer go look it up.
 *
 * `executed` is `false` for a reject, for an approval with no linked run,
 * and for an approve where the approver lacked the underlying tool's own
 * permission (the approval itself still gets decided; the tool never
 * runs).
 */
export type DecideApprovalResult = {
  approval: ApprovalItem
  executed: boolean
  result?: unknown
}

export type CreateEmployeeInput = {
  name: string
  purpose: string
  autonomy: AutonomyLevel
  allowedTools: string[]
  dataScope: string
  budgetLimit: number
  owner?: string
}

export interface AgentService {
  listEmployees(signal?: AbortSignal): Promise<DigitalEmployee[]>
  getEmployee(id: string, signal?: AbortSignal): Promise<DigitalEmployee>
  listRuns(employeeId?: string, signal?: AbortSignal): Promise<AgentRun[]>
  getRun(id: string, signal?: AbortSignal): Promise<AgentRun>
  listApprovals(employeeId?: string, signal?: AbortSignal): Promise<ApprovalItem[]>
  decideApproval(
    id: string,
    input: DecideApprovalInput,
    signal?: AbortSignal
  ): Promise<DecideApprovalResult>
  createEmployee(
    input: CreateEmployeeInput,
    signal?: AbortSignal
  ): Promise<DigitalEmployee>
  suspendEmployee(id: string, signal?: AbortSignal): Promise<DigitalEmployee>
  resumeEmployee(id: string, signal?: AbortSignal): Promise<DigitalEmployee>
  revokeEmployee(id: string, signal?: AbortSignal): Promise<DigitalEmployee>
  /**
   * `POST /api/agents/employees/{id}/run` — runs the employee headlessly
   * ("Run now"). Refuses with `invalid_request` for a suspended/revoked
   * employee (409), a missing prompt with no override (400), and the
   * reserved `emp-copilot` row (400) — the thrown `ServiceError.message`
   * is the backend's own specific reason in every case.
   */
  runEmployee(
    id: string,
    input?: RunEmployeeInput,
    signal?: AbortSignal
  ): Promise<AgentRun>
}
