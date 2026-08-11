import { mockCall } from "../transport"
import { agoIso } from "./mock-time"
import type { GovernanceService } from "../contracts/governance"

export const mockGovernanceService: GovernanceService = {
  listPolicies(signal) {
    return mockCall(
      () => [
        {
          id: "pol-1",
          name: "tenant_row_filter_default",
          status: "ready" as const,
          kind: "Row filter",
          subjects: "All analysts",
          resources: "tenant-scoped tables",
          effect: "Permit with obligation",
          version: 3,
          owner: "Security",
          updatedAt: agoIso(200),
        },
        {
          id: "pol-2",
          name: "agent_l3_write_gate",
          status: "ready" as const,
          kind: "Agent autonomy",
          subjects: "Digital employees L3",
          resources: "Non-critical write targets",
          effect: "Require approval",
          version: 1,
          owner: "Platform Governance",
          updatedAt: agoIso(400),
        },
      ],
      { signal }
    )
  },
  listClassifications(signal) {
    return mockCall(
      () => [
        {
          id: "cl-1",
          asset: "core.customer.customer_360",
          column: "email",
          classification: "confidential" as const,
          confidence: 0.98,
          reviewStatus: "reviewed" as const,
          maskingRule: "hash_email",
        },
        {
          id: "cl-2",
          asset: "core.finance.payments_enriched",
          column: "card_last4",
          classification: "restricted" as const,
          confidence: 0.91,
          reviewStatus: "needs-review" as const,
          maskingRule: "show_last4",
        },
      ],
      { signal }
    )
  },
  listQuality(signal) {
    return mockCall(
      () => [
        {
          id: "dq-1",
          name: "email_verified_completeness",
          asset: "gold.customer_360",
          dimension: "completeness",
          threshold: ">= 95%",
          severity: "medium" as const,
          lastStatus: "warning" as const,
          lastRunAt: agoIso(60),
        },
        {
          id: "dq-2",
          name: "payments_uniqueness",
          asset: "silver.payments_enriched",
          dimension: "uniqueness",
          threshold: "payment_id unique",
          severity: "high" as const,
          lastStatus: "passed" as const,
          lastRunAt: agoIso(20),
        },
      ],
      { signal }
    )
  },
  getLineage(focusId, signal) {
    return mockCall(
      () => ({
        focus: focusId,
        nodes: [
          { id: "n1", label: "orders_events", kind: "table" },
          { id: "n2", label: "orders_hourly_rollup", kind: "pipeline" },
          { id: "n3", label: "orders_hourly", kind: "table" },
          { id: "n4", label: "collections-copilot retrieve", kind: "agent" },
        ],
        edges: [
          { id: "e1", from: "n1", to: "n2", kind: "pipeline" as const },
          { id: "e2", from: "n2", to: "n3", kind: "pipeline" as const },
          { id: "e3", from: "n3", to: "n4", kind: "agent" as const },
        ],
        columnMappings: [
          { source: "orders_events.amount", target: "orders_hourly.amount_sum", transform: "sum" },
          { source: "orders_events.region", target: "orders_hourly.region", transform: "group by" },
        ],
      }),
      { signal }
    )
  },
  listAudit(signal) {
    return mockCall(
      () => [
        {
          id: "aud-1",
          at: agoIso(9),
          actor: "Rina Wijaya",
          actorKind: "user" as const,
          tenant: "nusantara-finance",
          action: "query.run",
          resource: "gold.revenue",
          outcome: "success" as const,
          policyDecision: "permit",
          obligations: ["row filter: tenant_id"],
          engineCategory: "hot-store",
          estimatedCost: 0.02,
          actualCost: 0.014,
        },
        {
          id: "aud-2",
          at: agoIso(12),
          actor: "collections-copilot",
          actorKind: "agent" as const,
          delegatedActor: "Rina Wijaya",
          tenant: "nusantara-finance",
          action: "agent.approval.request",
          resource: "dunning_priority proposals",
          outcome: "success" as const,
          policyDecision: "permit with approval",
          obligations: ["approval gate L3"],
          approvalId: "ap-01",
          estimatedCost: 0.05,
          actualCost: 0.04,
        },
        {
          id: "aud-3",
          at: agoIso(95),
          actor: "Bayu Pratama",
          actorKind: "user" as const,
          tenant: "nusantara-finance",
          action: "query.run",
          resource: "restricted.onprem.accounts",
          outcome: "denied" as const,
          policyDecision: "deny",
          obligations: ["residency: on-prem only"],
          engineCategory: "federated-compute",
        },
      ],
      { signal }
    )
  },
  listResidency(signal) {
    return mockCall(
      () => [
        {
          id: "res-1",
          tenant: "nusantara-finance",
          classification: "restricted" as const,
          approvedSites: ["Jakarta on-prem"],
          crossSiteAllowed: false,
          allowedOutput: "Aggregates only after declassification",
          violations7d: 1,
        },
        {
          id: "res-2",
          tenant: "nusantara-finance",
          classification: "confidential" as const,
          approvedSites: ["Jakarta on-prem", "Singapore (SG)"],
          crossSiteAllowed: true,
          allowedOutput: "Masked rows may cross sites",
          violations7d: 0,
        },
      ],
      { signal }
    )
  },
}
