import { ServiceError } from "../errors"
import { mockCall } from "../transport"
import { agoIso } from "./mock-time"
import type {
  AgentRun,
  AgentService,
  DigitalEmployee,
} from "../contracts/agents"

const EMPLOYEES: DigitalEmployee[] = [
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
]

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
    steps: [
      { id: "s1", label: "Retrieve policy context", status: "completed", detail: "2 chunks from credit-policy-2026" },
      { id: "s2", label: "Query overdue accounts", status: "completed", detail: "hot-analytics · 0.02 cu" },
      { id: "s3", label: "Propose priority updates", status: "running", detail: "Awaiting approval gate" },
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
    steps: [
      { id: "s1", label: "Retrieve similar cases", status: "completed", detail: "hybrid search" },
      { id: "s2", label: "Explain features", status: "completed", detail: "L2 propose only" },
    ],
    approvals: [],
  },
]

export const mockAgentService: AgentService = {
  listWorkflows(signal) {
    return mockCall(
      () => [
        {
          id: "wf-dunning",
          name: "Dunning priority workflow",
          status: "ready" as const,
          owner: "Collections Ops",
          trigger: "Streaming event: delinquency_score",
          steps: 6,
          lastRunAt: agoIso(8),
          approvalRequired: true,
        },
        {
          id: "wf-lag-triage",
          name: "Streaming lag triage",
          status: "ready" as const,
          owner: "Data Platform",
          trigger: "Alert: streaming lag",
          steps: 4,
          lastRunAt: agoIso(200),
          approvalRequired: false,
        },
      ],
      { signal }
    )
  },
  listEmployees(signal) {
    return mockCall(() => EMPLOYEES, { signal })
  },
  getEmployee(id, signal) {
    return mockCall(() => {
      const e = EMPLOYEES.find((x) => x.id === id)
      if (!e) throw new ServiceError("not_found", `Employee ${id} not found`)
      return e
    }, { signal })
  },
  listRuns(employeeId, signal) {
    return mockCall(
      () =>
        employeeId
          ? RUNS.filter((r) => r.employeeId === employeeId)
          : RUNS,
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
    return mockCall(
      () => [
        {
          id: "tool-query-sql",
          name: "query_sql",
          version: "1.4.0",
          publisher: "Rantai Lake",
          permission: "query:read",
          health: "healthy" as const,
          approvalStatus: "approved" as const,
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
          health: "healthy" as const,
          approvalStatus: "approved" as const,
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
          health: "degraded" as const,
          approvalStatus: "pending" as const,
          deprecated: false,
          rateLimit: "10/min",
          usage30d: 312,
        },
      ],
      { signal }
    )
  },
  listApprovals(employeeId, signal) {
    return mockCall(
      () => {
        const approvals = [
          {
            id: "ap-01",
            employeeId: "emp-collections",
            employeeName: "collections-copilot",
            action: "Update dunning_priority proposals (24 accounts)",
            requestedAt: agoIso(6),
            status: "pending" as const,
            risk: "Writes to non-critical proposal branch",
          },
          {
            id: "ap-02",
            employeeId: "emp-collections",
            employeeName: "collections-copilot",
            action: "Notify customers in segment B",
            requestedAt: agoIso(80),
            status: "approved" as const,
            risk: "External notification",
          },
        ]
        return employeeId
          ? approvals.filter((a) => a.employeeId === employeeId)
          : approvals
      },
      { signal }
    )
  },
}
