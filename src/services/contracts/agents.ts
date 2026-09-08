import type {
  AgentRunStatus,
  ApprovalStatus,
  AutonomyLevel,
  EntityStatus,
  Health,
  RunStepStatus,
} from "@/lib/status"

export type AgentWorkflow = {
  id: string
  name: string
  status: EntityStatus
  owner: string
  trigger: string
  steps: number
  lastRunAt: string
  approvalRequired: boolean
}

export type DigitalEmployee = {
  id: string
  name: string
  purpose: string
  owner: string
  autonomy: AutonomyLevel
  status: EntityStatus
  budgetLimit: number
  budgetSpent: number
  budgetReserved: number
  allowedTools: string[]
  dataScope: string
  approvalRate: number
  successRate: number
  recentRuns: number
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
  budgetConsumed: number
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

export type AgentTool = {
  id: string
  name: string
  version: string
  publisher: string
  permission: string
  health: Health
  approvalStatus: ApprovalStatus
  deprecated: boolean
  rateLimit: string
  usage30d: number
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

export type CreateWorkflowInput = {
  name: string
  trigger: string
  stepKinds: string[]
  approvalRequired: boolean
  owner?: string
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

export type RegisterToolInput = {
  name: string
  version: string
  publisher: string
  permission: string
  rateLimit: string
}

export interface AgentService {
  listWorkflows(signal?: AbortSignal): Promise<AgentWorkflow[]>
  listEmployees(signal?: AbortSignal): Promise<DigitalEmployee[]>
  getEmployee(id: string, signal?: AbortSignal): Promise<DigitalEmployee>
  listRuns(employeeId?: string, signal?: AbortSignal): Promise<AgentRun[]>
  getRun(id: string, signal?: AbortSignal): Promise<AgentRun>
  listTools(signal?: AbortSignal): Promise<AgentTool[]>
  listApprovals(employeeId?: string, signal?: AbortSignal): Promise<ApprovalItem[]>
  decideApproval(
    id: string,
    input: DecideApprovalInput,
    signal?: AbortSignal
  ): Promise<DecideApprovalResult>
  createWorkflow(input: CreateWorkflowInput, signal?: AbortSignal): Promise<AgentWorkflow>
  createEmployee(
    input: CreateEmployeeInput,
    signal?: AbortSignal
  ): Promise<DigitalEmployee>
  registerTool(input: RegisterToolInput, signal?: AbortSignal): Promise<AgentTool>
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
