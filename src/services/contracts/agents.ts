import type {
  ApprovalStatus,
  AutonomyLevel,
  EntityStatus,
  Health,
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
}

export type AgentRun = {
  id: string
  employeeId: string
  workflowId?: string
  status: EntityStatus
  trigger: string
  actor: string
  delegatedUser?: string
  startedAt: string
  endedAt?: string
  budgetConsumed: number
  steps: {
    id: string
    label: string
    status: EntityStatus
    detail: string
  }[]
  approvals: { id: string; status: ApprovalStatus; at?: string }[]
  auditEventId?: string
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
  ): Promise<ApprovalItem>
  createWorkflow(input: CreateWorkflowInput, signal?: AbortSignal): Promise<AgentWorkflow>
  createEmployee(
    input: CreateEmployeeInput,
    signal?: AbortSignal
  ): Promise<DigitalEmployee>
  registerTool(input: RegisterToolInput, signal?: AbortSignal): Promise<AgentTool>
  suspendEmployee(id: string, signal?: AbortSignal): Promise<DigitalEmployee>
  resumeEmployee(id: string, signal?: AbortSignal): Promise<DigitalEmployee>
  revokeEmployee(id: string, signal?: AbortSignal): Promise<DigitalEmployee>
}
