import { mockCall } from "../transport"
import { agoIso } from "./mock-time"
import { createStore } from "./mutable-store"
import type {
  ClassificationRule,
  CreateClassificationRuleInput,
  CreatePolicyInput,
  CreateQualityRuleInput,
  CreateResidencyRuleInput,
  GovernanceService,
  Policy,
  QualityRule,
  ResidencyRule,
} from "../contracts/governance"

const policyStore = createStore<Policy>([
  {
    id: "pol-1",
    name: "tenant_row_filter_default",
    status: "ready",
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
    status: "ready",
    kind: "Agent autonomy",
    subjects: "Digital employees L3",
    resources: "Non-critical write targets",
    effect: "Require approval",
    version: 1,
    owner: "Platform Governance",
    updatedAt: agoIso(400),
  },
])

const classificationStore = createStore<ClassificationRule>([
  {
    id: "cl-1",
    asset: "core.customer.customer_360",
    column: "email",
    classification: "confidential",
    confidence: 0.98,
    reviewStatus: "reviewed",
    maskingRule: "hash_email",
  },
  {
    id: "cl-2",
    asset: "core.finance.payments_enriched",
    column: "card_last4",
    classification: "restricted",
    confidence: 0.91,
    reviewStatus: "needs-review",
    maskingRule: "show_last4",
  },
])

const qualityStore = createStore<QualityRule>([
  {
    id: "dq-1",
    name: "email_verified_completeness",
    asset: "gold.customer_360",
    dimension: "completeness",
    threshold: ">= 95%",
    severity: "medium",
    lastStatus: "warning",
    lastRunAt: agoIso(60),
  },
  {
    id: "dq-2",
    name: "payments_uniqueness",
    asset: "silver.payments_enriched",
    dimension: "uniqueness",
    threshold: "payment_id unique",
    severity: "high",
    lastStatus: "passed",
    lastRunAt: agoIso(20),
  },
])

const residencyStore = createStore<ResidencyRule>([
  {
    id: "res-1",
    tenant: "nusantara-finance",
    classification: "restricted",
    approvedSites: ["Jakarta on-prem"],
    crossSiteAllowed: false,
    allowedOutput: "Aggregates only after declassification",
    violations7d: 1,
  },
  {
    id: "res-2",
    tenant: "nusantara-finance",
    classification: "confidential",
    approvedSites: ["Jakarta on-prem", "Singapore (SG)"],
    crossSiteAllowed: true,
    allowedOutput: "Masked rows may cross sites",
    violations7d: 0,
  },
])

export const mockGovernanceService: GovernanceService = {
  listPolicies(signal) {
    return mockCall(() => policyStore.list(), { signal })
  },
  listClassifications(signal) {
    return mockCall(() => classificationStore.list(), { signal })
  },
  listQuality(signal) {
    return mockCall(() => qualityStore.list(), { signal })
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
          {
            source: "orders_events.amount",
            target: "orders_hourly.amount_sum",
            transform: "sum",
          },
          {
            source: "orders_events.region",
            target: "orders_hourly.region",
            transform: "group by",
          },
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
          engineCategory: "hot-store" as const,
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
          engineCategory: "federated-compute" as const,
        },
      ],
      { signal }
    )
  },
  listResidency(signal) {
    return mockCall(() => residencyStore.list(), { signal })
  },
  createPolicy(input: CreatePolicyInput, signal) {
    return mockCall(
      () =>
        policyStore.prepend({
          id: `pol-${Date.now().toString(36)}`,
          name: input.name,
          status: input.activate ? "ready" : "draft",
          kind: input.kind,
          subjects: input.subjects,
          resources: input.resources,
          effect: input.effect,
          version: 1,
          owner: input.owner ?? "Current user",
          updatedAt: agoIso(0),
        }),
      { signal, delayMs: 500 }
    )
  },
  createQualityRule(input: CreateQualityRuleInput, signal) {
    return mockCall(
      () =>
        qualityStore.prepend({
          id: `dq-${Date.now().toString(36)}`,
          name: input.name,
          asset: input.asset,
          dimension: input.dimension,
          threshold: input.threshold,
          severity: input.severity,
          lastStatus: "warning",
          lastRunAt: agoIso(0),
        }),
      { signal, delayMs: 400 }
    )
  },
  createClassificationRule(input: CreateClassificationRuleInput, signal) {
    return mockCall(
      () =>
        classificationStore.prepend({
          id: `cl-${Date.now().toString(36)}`,
          asset: input.asset,
          column: input.column,
          classification: input.classification,
          confidence: 1,
          reviewStatus: "needs-review",
          maskingRule: input.maskingRule,
        }),
      { signal, delayMs: 400 }
    )
  },
  createResidencyRule(input: CreateResidencyRuleInput, signal) {
    return mockCall(
      () =>
        residencyStore.prepend({
          id: `res-${Date.now().toString(36)}`,
          tenant: input.tenant,
          classification: input.classification,
          approvedSites: input.approvedSites,
          crossSiteAllowed: input.crossSiteAllowed,
          allowedOutput: input.allowedOutput,
          violations7d: 0,
        }),
      { signal, delayMs: 400 }
    )
  },
}
