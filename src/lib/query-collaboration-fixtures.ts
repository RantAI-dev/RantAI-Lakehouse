export type Collaborator = {
  id: string
  name: string
  role: "Owner" | "Analyst" | "Engineer" | "Viewer"
  status: "online" | "away" | "offline"
}

export type ProjectActivity = {
  id: string
  actor: string
  action: string
  when: string
}

export type ProjectQueryHistory = {
  id: string
  actor: string
  mode: "Natural Language" | "SQL"
  queryPreview: string
  status: "Success" | "Running" | "Failed"
  when: string
}

export type CollaborationProject = {
  id: string
  name: string
  memberCount: number
  members: Collaborator[]
  lastActivity: string
  description: string
  activities: ProjectActivity[]
  queryHistory: ProjectQueryHistory[]
}

export const MOCK_COLLABORATION_PROJECTS: CollaborationProject[] = [
  {
    id: "fraud-monitoring",
    name: "Fraud Monitoring Q2",
    memberCount: 5,
    lastActivity: "2m ago",
    description:
      "Cross-team investigation for suspicious transactions and anomaly trend review.",
    members: [
      { id: "u1", name: "Eri Ramadan", role: "Owner", status: "online" },
      { id: "u2", name: "Sinta Ayu", role: "Analyst", status: "online" },
      { id: "u3", name: "Bima Putra", role: "Engineer", status: "away" },
      { id: "u4", name: "Nadia Putri", role: "Analyst", status: "offline" },
      { id: "u5", name: "Kevin", role: "Viewer", status: "offline" },
    ],
    activities: [
      {
        id: "a1",
        actor: "Sinta Ayu",
        action: "edited SQL filters for risky merchant category",
        when: "2m ago",
      },
      {
        id: "a2",
        actor: "Bima Putra",
        action: "ran query on `silver.transactions_enriched`",
        when: "9m ago",
      },
      {
        id: "a3",
        actor: "Eri Ramadan",
        action: "created project and invited 4 collaborators",
        when: "45m ago",
      },
    ],
    queryHistory: [
      {
        id: "q1",
        actor: "Sinta Ayu",
        mode: "Natural Language",
        queryPreview: "Show top merchants with chargeback rate increase in last 7 days",
        status: "Success",
        when: "2m ago",
      },
      {
        id: "q2",
        actor: "Bima Putra",
        mode: "SQL",
        queryPreview:
          "SELECT merchant_id, SUM(case when chargeback = 1 then 1 end) ...",
        status: "Success",
        when: "9m ago",
      },
      {
        id: "q3",
        actor: "Nadia Putri",
        mode: "SQL",
        queryPreview: "SELECT * FROM gold.fraud_signals WHERE score > 0.85 LIMIT 100",
        status: "Running",
        when: "14m ago",
      },
    ],
  },
  {
    id: "customer-360",
    name: "Customer 360 Insights",
    memberCount: 4,
    lastActivity: "15m ago",
    description:
      "Unified customer performance dashboard and churn exploration for weekly review.",
    members: [
      { id: "u1", name: "Eri Ramadan", role: "Owner", status: "online" },
      { id: "u6", name: "Rizky", role: "Engineer", status: "away" },
      { id: "u7", name: "Nisa", role: "Analyst", status: "offline" },
      { id: "u8", name: "Dimas", role: "Viewer", status: "offline" },
    ],
    activities: [
      {
        id: "a4",
        actor: "Rizky",
        action: "updated join condition for customer dimension",
        when: "15m ago",
      },
      {
        id: "a5",
        actor: "Nisa",
        action: "ran NL query for churn segmentation",
        when: "21m ago",
      },
    ],
    queryHistory: [
      {
        id: "q4",
        actor: "Nisa",
        mode: "Natural Language",
        queryPreview: "Segment customers with declining monthly order amount",
        status: "Success",
        when: "21m ago",
      },
      {
        id: "q5",
        actor: "Rizky",
        mode: "SQL",
        queryPreview: "WITH trend AS (...) SELECT segment, avg_revenue FROM trend",
        status: "Success",
        when: "26m ago",
      },
    ],
  },
]

/**
 * Looks up a mock collaboration project by id from `MOCK_COLLABORATION_PROJECTS`.
 *
 * Returns `null` (not `undefined`) when the project is not found, so callers can
 * treat the absence as a definite "not found" state without `=== undefined` checks.
 */
export function getCollaborationProjectById(projectId: string) {
  return MOCK_COLLABORATION_PROJECTS.find((p) => p.id === projectId) ?? null
}
