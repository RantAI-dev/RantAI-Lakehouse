import { ServiceError } from "../errors"
import { mockCall } from "../transport"
import { agoIso } from "./mock-time"
import type {
  ActivityItem,
  AlertItem,
  OverviewService,
  OverviewSummary,
} from "../contracts/overview"

const SUMMARY: Omit<OverviewSummary, "incidents"> = {
  assetsTotal: 1284,
  staleAssets: 17,
  assetsByTier: {
    hot: { count: 212, bytes: 8.4 * 1024 ** 4 },
    warm: { count: 388, bytes: 22.1 * 1024 ** 4 },
    cold: { count: 611, bytes: 148.6 * 1024 ** 4 },
    ai: { count: 73, bytes: 3.2 * 1024 ** 4 },
  },
  pipelines: { active: 46, failed: 2, delayed: 3 },
  streaming: { jobs: 12, maxLagSeconds: 42, unhealthy: 1 },
  queries: {
    volume24h: 48213,
    p95Ms: 840,
    failureRate: 0.006,
    cacheAssistRate: 0.63,
    scannedBytes24h: 61.7 * 1024 ** 4,
  },
  policyViolations7d: 4,
  pendingApprovals: 5,
  agents: { activeRuns: 8, budgetUsedRate: 0.41 },
  services: { healthy: 11, degraded: 1, unhealthy: 0 },
}

function buildActivity(): ActivityItem[] {
  return [
    {
      id: "act-01",
      at: agoIso(4),
      actor: "orders_hourly_rollup",
      actorKind: "service",
      action: "Pipeline run completed",
      target: "gold.orders_hourly",
      targetHref: "/pipelines/pl-orders-rollup",
      category: "pipeline",
    },
    {
      id: "act-02",
      at: agoIso(9),
      actor: "Rina Wijaya",
      actorKind: "user",
      action: "Ran federated query",
      target: "revenue x customer_dim (hot + cold)",
      targetHref: "/query-studio",
      category: "query",
      auditEventId: "aud-1",
    },
    {
      id: "act-03",
      at: agoIso(21),
      actor: "collections-copilot",
      actorKind: "agent",
      action: "Requested approval for write action",
      target: "Update dunning_priority proposals",
      targetHref: "/agents/employees/emp-collections",
      category: "approval",
      auditEventId: "aud-2",
    },
    {
      id: "act-04",
      at: agoIso(34),
      actor: "Dewi Anggraini",
      actorKind: "user",
      action: "Activated masking policy",
      target: "customer_pii_mask_v3",
      targetHref: "/governance/classification",
      category: "policy",
    },
    {
      id: "act-05",
      at: agoIso(52),
      actor: "cdc-postgres-core",
      actorKind: "service",
      action: "Connector checkpoint advanced",
      target: "postgres core-banking CDC",
      targetHref: "/connectors",
      category: "connector",
    },
    {
      id: "act-06",
      at: agoIso(75),
      actor: "Bayu Pratama",
      actorKind: "user",
      action: "Proposed schema change",
      target: "silver.payments_enriched (+2 columns)",
      targetHref: "/catalog",
      category: "schema",
    },
    {
      id: "act-07",
      at: agoIso(96),
      actor: "risk-sentinel",
      actorKind: "agent",
      action: "Retrieved context for anomaly triage",
      target: "knowledge: credit-policy-2026",
      targetHref: "/knowledge",
      category: "agent",
    },
    {
      id: "act-08",
      at: agoIso(140),
      actor: "platform",
      actorKind: "service",
      action: "Incident resolved",
      target: "Streaming lag spike on payments topic",
      targetHref: "/alerts",
      category: "incident",
    },
  ]
}

function buildAlerts(): AlertItem[] {
  return [
    {
      id: "al-01",
      title: "Streaming lag above threshold",
      severity: "high",
      source: "Real-time plane",
      affected: "rt.payments_flow_mv",
      status: "open",
      assignee: "On-call data platform",
      at: agoIso(12),
      detail:
        "Barrier latency exceeded 30 s for 3 consecutive intervals. Sink to hot store is delayed; downstream freshness on payments dashboards is degraded.",
      href: "/streaming/sj-payments-flow",
    },
    {
      id: "al-02",
      title: "Pipeline failed after retries",
      severity: "critical",
      source: "Ingestion plane",
      affected: "erp_inventory_sync",
      status: "acknowledged",
      assignee: "Bayu Pratama",
      at: agoIso(48),
      detail:
        "Source API returned schema-mismatch errors on 3 retries. Dead-letter target captured 1,204 records. Manual mapping review required.",
      href: "/pipelines/pl-erp-inventory",
    },
    {
      id: "al-03",
      title: "Residency policy blocked a query",
      severity: "medium",
      source: "Access layer",
      affected: "tenant: nusantara-finance",
      status: "open",
      at: agoIso(95),
      detail:
        "A federated query attempted to move restricted rows out of the approved on-premise site. The plan was rejected with an explicit policy error.",
      href: "/residency",
    },
    {
      id: "al-04",
      title: "Agent budget nearing limit",
      severity: "low",
      source: "Agent operations",
      affected: "collections-copilot",
      status: "open",
      at: agoIso(130),
      detail:
        "Monthly compute budget at 82%. Reservations will start failing at 100%; consider raising the budget or lowering run frequency.",
      href: "/agents/employees/emp-collections",
    },
    {
      id: "al-05",
      title: "Quality check failing on curated table",
      severity: "medium",
      source: "Data quality",
      affected: "gold.customer_360",
      status: "resolved",
      assignee: "Dewi Anggraini",
      at: agoIso(300),
      detail:
        "Completeness on email_verified dropped to 91% after an upstream mapping change. Fixed by re-running the enrichment step.",
      resolutionNote: "Enrichment step re-run; completeness back to 99.2%.",
      href: "/governance/data-quality",
    },
  ]
}

/** Mutable in-memory store so acknowledge/resolve persist within the session. */
const ALERTS = buildAlerts()

function findAlert(id: string): AlertItem {
  const alert = ALERTS.find((a) => a.id === id)
  if (!alert) throw new ServiceError("not_found", `Alert ${id} not found`)
  return alert
}

/** Mock adapter for the Overview domain. */
export const mockOverviewService: OverviewService = {
  getSummary(signal) {
    return mockCall(
      () => ({ ...SUMMARY, incidents: ALERTS.slice(0, 2).map((a) => ({
        id: a.id,
        title: a.title,
        severity: a.severity,
        source: a.source,
        at: a.at,
      })) }),
      { signal }
    )
  },
  listActivity(signal) {
    return mockCall(buildActivity, { signal })
  },
  listAlerts(signal) {
    return mockCall(() => [...ALERTS], { signal })
  },
  acknowledgeAlert(id, signal) {
    return mockCall(() => {
      const alert = findAlert(id)
      alert.status = "acknowledged"
      return { ...alert }
    }, { signal })
  },
  resolveAlert(id, note, signal) {
    return mockCall(() => {
      const alert = findAlert(id)
      alert.status = "resolved"
      alert.resolutionNote = note
      return { ...alert }
    }, { signal })
  },
}
