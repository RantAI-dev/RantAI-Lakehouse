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
  action: string
  requestedAt: string
  status: ApprovalStatus
  risk: string
}

export interface AgentService {
  listWorkflows(signal?: AbortSignal): Promise<AgentWorkflow[]>
  listEmployees(signal?: AbortSignal): Promise<DigitalEmployee[]>
  getEmployee(id: string, signal?: AbortSignal): Promise<DigitalEmployee>
  listRuns(employeeId?: string, signal?: AbortSignal): Promise<AgentRun[]>
  getRun(id: string, signal?: AbortSignal): Promise<AgentRun>
  listTools(signal?: AbortSignal): Promise<AgentTool[]>
  listApprovals(employeeId?: string, signal?: AbortSignal): Promise<ApprovalItem[]>
}
