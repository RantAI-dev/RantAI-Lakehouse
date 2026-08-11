import { ServiceError } from "../errors"
import { mockCall } from "../transport"
import { agoIso } from "./mock-time"
import { createStore } from "./mutable-store"
import type {
  AgentRun,
  AgentService,
  AgentTool,
  AgentWorkflow,
  ApprovalItem,
  CreateEmployeeInput,
  CreateWorkflowInput,
  DecideApprovalInput,
  DigitalEmployee,
  RegisterToolInput,
} from "../contracts/agents"

const employeeStore = createStore<DigitalEmployee>([
  {
    id: "emp-collections",
    name: "collections-copilot",
    purpose: "Prioritize dunning actions and draft customer outreach.",
    owner: "Collections Ops",
    autonomy: "L3",
    status: "ready",
    budgetLimit: 1000,
    budgetSpent: 820,
    budgetReserved: 40,
    allowedTools: ["query_sql", "retrieve", "lineage", "whatif_branch"],
    dataScope: "finance + collections (confidential)",
    approvalRate: 0.72,
    successRate: 0.94,
    recentRuns: 128,
  },
  {
    id: "emp-risk",
    name: "risk-sentinel",
    purpose: "Triage fraud anomalies and explain signal provenance.",
    owner: "Risk",
    autonomy: "L2",
    status: "running",
    budgetLimit: 500,
    budgetSpent: 210,
    budgetReserved: 15,
    allowedTools: ["retrieve", "query_sql", "freshness"],
    dataScope: "risk signals + knowledge",
    approvalRate: 1,
    successRate: 0.97,
    recentRuns: 64,
  },
])

const workflowStore = createStore<AgentWorkflow>([
  {
    id: "wf-dunning",
    name: "Dunning priority workflow",
    status: "ready",
    owner: "Collections Ops",
    trigger: "Streaming event: delinquency_score",
    steps: 6,
    lastRunAt: agoIso(8),
    approvalRequired: true,
  },
  {
    id: "wf-lag-triage",
    name: "Streaming lag triage",
    status: "ready",
    owner: "Data Platform",
    trigger: "Alert: streaming lag",
    steps: 4,
    lastRunAt: agoIso(200),
    approvalRequired: false,
  },
])

const toolStore = createStore<AgentTool>([
  {
    id: "tool-query-sql",
    name: "query_sql",
    version: "1.4.0",
    publisher: "Rantai Lake",
    permission: "query:read",
    health: "healthy",
    approvalStatus: "approved",
    deprecated: false,
    rateLimit: "60/min",
    usage30d: 18420,
  },
  {
    id: "tool-retrieve",
    name: "retrieve",
    version: "1.2.1",
    publisher: "Rantai Lake",
    permission: "knowledge:read",
    health: "healthy",
    approvalStatus: "approved",
    deprecated: false,
    rateLimit: "120/min",
    usage30d: 9021,
  },
  {
    id: "tool-whatif",
    name: "whatif_branch",
    version: "0.9.0",
    publisher: "Rantai Lake",
    permission: "catalog:branch",
    health: "degraded",
    approvalStatus: "pending",
    deprecated: false,
    rateLimit: "10/min",
    usage30d: 312,
  },
])

const RUNS: AgentRun[] = [
  {
    id: "run-col-01",
    employeeId: "emp-collections",
    workflowId: "wf-dunning",
    status: "running",
    trigger: "Streaming threshold: delinquency_score > 0.8",
    actor: "collections-copilot",
    delegatedUser: "Rina Wijaya",
    startedAt: agoIso(8),
    budgetConsumed: 12.4,
    auditEventId: "aud-agent-run-col-01",
    steps: [
      {
        id: "s1",
        label: "Retrieve policy context",
        status: "completed",
        detail: "2 chunks from credit-policy-2026",
      },
      {
        id: "s2",
        label: "Query overdue accounts",
        status: "completed",
        detail: "hot-analytics · 0.02 cu",
      },
      {
        id: "s3",
        label: "Propose priority updates",
        status: "running",
        detail: "Awaiting approval gate",
      },
    ],
    approvals: [{ id: "ap-01", status: "pending" }],
  },
  {
    id: "run-risk-01",
    employeeId: "emp-risk",
    status: "completed",
    trigger: "Manual investigation",
    actor: "risk-sentinel",
    delegatedUser: "Dewi Anggraini",
    startedAt: agoIso(120),
    endedAt: agoIso(110),
    budgetConsumed: 4.1,
    auditEventId: "aud-agent-run-risk-01",
    steps: [
      {
        id: "s1",
        label: "Retrieve similar cases",
        status: "completed",
        detail: "hybrid search",
      },
      {
        id: "s2",
        label: "Explain features",
        status: "completed",
        detail: "L2 propose only",
      },
    ],
    approvals: [],
  },
]

const approvalStore = createStore<ApprovalItem>([
  {
    id: "ap-01",
    employeeId: "emp-collections",
    employeeName: "collections-copilot",
    runId: "run-col-01",
    workflowId: "wf-dunning",
    action: "Update dunning_priority proposals (24 accounts)",
    resource: "core.collections.dunning_priority",
    reason: "Delinquency score crossed approval threshold",
    impact: "Writes 24 proposal rows to a non-critical branch; no customer notifications yet.",
    evidence: [
      "Query qh-1 scanned payments_enriched (cache hit)",
      "Retrieved 2 chunks from credit-policy-2026 v12",
    ],
    policy: "pol-1 · Tenant row isolation",
    costEstimate: 0.04,
    expiresAt: agoIso(-120),
    requestedAt: agoIso(6),
    status: "pending",
    risk: "Writes to non-critical proposal branch",
    auditEventId: "aud-approval-ap-01",
  },
  {
    id: "ap-02",
    employeeId: "emp-collections",
    employeeName: "collections-copilot",
    runId: "run-col-01",
    workflowId: "wf-dunning",
    action: "Notify customers in segment B",
    resource: "notification.email",
    reason: "Playbook step after priority update",
    impact: "External email to ~180 customers in segment B.",
    evidence: ["Policy section 4.2 escalation thresholds"],
    policy: "External notification gate",
    costEstimate: 0.12,
    requestedAt: agoIso(80),
    status: "approved",
    risk: "External notification",
    decidedAt: agoIso(70),
    comment: "Approved for segment B only.",
    auditEventId: "aud-approval-ap-02",
  },
  {
    id: "ap-03",
    employeeId: "emp-risk",
    employeeName: "risk-sentinel",
    action: "Escalate fraud case FR-2291",
    resource: "risk.cases",
    reason: "Similarity search matched prior confirmed fraud",
    impact: "Creates investigator task; no automated account freeze.",
    evidence: ["Hybrid search hit score 0.91 on credit-policy corpus"],
    policy: "L2 propose-only autonomy",
    costEstimate: 0.02,
    requestedAt: agoIso(200),
    status: "pending",
    risk: "Creates investigator workload",
    auditEventId: "aud-approval-ap-03",
  },
])

export const mockAgentService: AgentService = {
  listWorkflows(signal) {
    return mockCall(() => workflowStore.list(), { signal })
  },
  listEmployees(signal) {
    return mockCall(() => employeeStore.list(), { signal })
  },
  getEmployee(id, signal) {
    return mockCall(() => {
      const e = employeeStore.get(id)
      if (!e) throw new ServiceError("not_found", `Employee ${id} not found`)
      return e
    }, { signal })
  },
  listRuns(employeeId, signal) {
    return mockCall(
      () => (employeeId ? RUNS.filter((r) => r.employeeId === employeeId) : RUNS),
      { signal }
    )
  },
  getRun(id, signal) {
    return mockCall(() => {
      const r = RUNS.find((x) => x.id === id)
      if (!r) throw new ServiceError("not_found", `Run ${id} not found`)
      return r
    }, { signal })
  },
  listTools(signal) {
    return mockCall(() => toolStore.list(), { signal })
  },
  listApprovals(employeeId, signal) {
    return mockCall(
      () =>
        employeeId
          ? approvalStore.list().filter((a) => a.employeeId === employeeId)
          : approvalStore.list(),
      { signal }
    )
  },
  decideApproval(id, input: DecideApprovalInput, signal) {
    return mockCall(() => {
      const existing = approvalStore.get(id)
      if (!existing) throw new ServiceError("not_found", `Approval ${id} not found`)
      if (existing.status !== "pending") {
        throw new ServiceError(
          "invalid_request",
          `Approval ${id} is already ${existing.status}`
        )
      }
      const updated = approvalStore.update(id, {
        status: input.decision,
        decidedAt: agoIso(0),
        comment: input.comment,
        auditEventId: `aud-approval-${id}-${input.decision}`,
      })
      if (!updated) throw new ServiceError("not_found", `Approval ${id} not found`)
      return updated
    }, { signal, delayMs: 400 })
  },
  createWorkflow(input: CreateWorkflowInput, signal) {
    return mockCall(
      () =>
        workflowStore.prepend({
          id: `wf-${Date.now().toString(36)}`,
          name: input.name,
          status: "draft",
          owner: input.owner ?? "Current user",
          trigger: input.trigger,
          steps: input.stepKinds.length,
          lastRunAt: agoIso(0),
          approvalRequired: input.approvalRequired,
        }),
      { signal, delayMs: 500 }
    )
  },
  createEmployee(input: CreateEmployeeInput, signal) {
    return mockCall(
      () =>
        employeeStore.prepend({
          id: `emp-${Date.now().toString(36)}`,
          name: input.name,
          purpose: input.purpose,
          owner: input.owner ?? "Current user",
          autonomy: input.autonomy,
          status: "draft",
          budgetLimit: input.budgetLimit,
          budgetSpent: 0,
          budgetReserved: 0,
          allowedTools: input.allowedTools,
          dataScope: input.dataScope,
          approvalRate: 0,
          successRate: 0,
          recentRuns: 0,
        }),
      { signal, delayMs: 500 }
    )
  },
  registerTool(input: RegisterToolInput, signal) {
    return mockCall(
      () =>
        toolStore.prepend({
          id: `tool-${Date.now().toString(36)}`,
          name: input.name,
          version: input.version,
          publisher: input.publisher,
          permission: input.permission,
          health: "healthy",
          approvalStatus: "pending",
          deprecated: false,
          rateLimit: input.rateLimit,
          usage30d: 0,
        }),
      { signal, delayMs: 400 }
    )
  },
  suspendEmployee(id, signal) {
    return mockCall(() => {
      const updated = employeeStore.update(id, { status: "paused" })
      if (!updated) throw new ServiceError("not_found", `Employee ${id} not found`)
      return updated
    }, { signal })
  },
  resumeEmployee(id, signal) {
    return mockCall(() => {
      const updated = employeeStore.update(id, { status: "ready" })
      if (!updated) throw new ServiceError("not_found", `Employee ${id} not found`)
      return updated
    }, { signal })
  },
  revokeEmployee(id, signal) {
    return mockCall(() => {
      const updated = employeeStore.update(id, { status: "cancelled" })
      if (!updated) throw new ServiceError("not_found", `Employee ${id} not found`)
      return updated
    }, { signal })
  },
}
