import { ServiceError } from "../errors"
import { mockCall } from "../transport"
import { agoIso } from "./mock-time"
import type { OpsService, WorkloadItem } from "../contracts/ops"

/** Mutable in-memory store so cancellation persists within the session. */
const WORKLOADS: WorkloadItem[] = [
  {
    id: "wl-1",
    principal: "Rina Wijaya",
    tenant: "nusantara-finance",
    class: "hot-analytics",
    engine: "hot-store",
    status: "running",
    elapsedMs: 820,
    estimatedCost: 0.02,
    startedAt: agoIso(1),
  },
  {
    id: "wl-2",
    principal: "collections-copilot",
    tenant: "nusantara-finance",
    class: "agent-tool",
    engine: "ai-store",
    status: "queued",
    elapsedMs: 0,
    estimatedCost: 0.08,
    queueReason: "Tenant concurrency limit",
    startedAt: agoIso(0),
  },
  {
    id: "wl-3",
    principal: "Bayu Pratama",
    tenant: "nusantara-finance",
    class: "federated",
    engine: "federated-compute",
    status: "completed",
    elapsedMs: 3200,
    estimatedCost: 0.42,
    startedAt: agoIso(40),
  },
]

export const mockOpsService: OpsService = {
  listWorkloads(signal) {
    return mockCall(() => [...WORKLOADS], { signal })
  },
  cancelWorkload(id, signal) {
    return mockCall(() => {
      const workload = WORKLOADS.find((w) => w.id === id)
      if (!workload) throw new ServiceError("not_found", `Workload ${id} not found`)
      if (workload.status === "queued" || workload.status === "running") {
        workload.status = "cancelled"
      }
      return { ...workload }
    }, { signal })
  },
  getObservability(signal) {
    return mockCall(
      () => ({
        queryP95Ms: 840,
        queryErrorRate: 0.006,
        ingestLagSeconds: 8,
        streamingLagSeconds: 42,
        cacheHitRate: 0.63,
        policyDecisionP95Ms: 12,
        agentSuccessRate: 0.95,
        activeIncidents: 2,
        slos: [
          { name: "Hot analytics p95", target: "< 1s", current: "840 ms", ok: true },
          { name: "Cache assist rate", target: ">= 60%", current: "63%", ok: true },
          { name: "Streaming lag", target: "< 10s", current: "42 s", ok: false },
          { name: "Ingest freshness", target: "< 10s", current: "8 s", ok: true },
        ],
      }),
      { signal }
    )
  },
  listServices(signal) {
    return mockCall(
      () => [
        {
          id: "svc-access",
          name: "Access & routing layer",
          health: "healthy" as const,
          version: "2.4.1",
          site: "Jakarta",
          replicas: 6,
          errorRate: 0.001,
          latencyMs: 18,
          dependencies: ["Identity", "Policy engine", "Metadata store"],
        },
        {
          id: "svc-hot",
          name: "Hot analytical store",
          health: "healthy" as const,
          version: "24.8",
          site: "Jakarta",
          replicas: 12,
          errorRate: 0.0004,
          latencyMs: 42,
          dependencies: ["Object storage"],
        },
        {
          id: "svc-stream",
          name: "Real-time streaming plane",
          health: "degraded" as const,
          version: "1.9.0",
          site: "Jakarta",
          replicas: 4,
          errorRate: 0.02,
          latencyMs: 120,
          dependencies: ["Kafka", "Hot analytical store"],
        },
        {
          id: "svc-ai",
          name: "AI retrieval store",
          health: "healthy" as const,
          version: "0.22",
          site: "Singapore",
          replicas: 3,
          errorRate: 0.003,
          latencyMs: 65,
          dependencies: ["Object storage", "Ingestion plane"],
        },
      ],
      { signal }
    )
  },
  getUsage(signal) {
    return mockCall(
      () => ({
        computeUnits7d: 18420,
        scannedBytes7d: 420 * 1024 ** 4,
        storageByTier: {
          hot: 8.4 * 1024 ** 4,
          warm: 22.1 * 1024 ** 4,
          cold: 148.6 * 1024 ** 4,
          ai: 3.2 * 1024 ** 4,
        },
        pipelineRuns7d: 6120,
        agentBudgetUsedRate: 0.41,
        tenants: [
          {
            id: "t-nusantara",
            name: "Nusantara Finance",
            computeUnits: 12400,
            budgetLimit: 20000,
            budgetSpent: 12400,
          },
          {
            id: "t-retail",
            name: "Retail Analytics",
            computeUnits: 4200,
            budgetLimit: 8000,
            budgetSpent: 4200,
          },
        ],
      }),
      { signal }
    )
  },
}
