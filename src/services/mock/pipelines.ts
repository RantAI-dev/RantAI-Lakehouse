import { ServiceError } from "../errors"
import { mockCall } from "../transport"
import { agoIso } from "./mock-time"
import type {
  Pipeline,
  PipelineDetail,
  PipelineRun,
  PipelineService,
} from "../contracts/pipelines"

const PIPELINES: Pipeline[] = [
  {
    id: "pl-orders-rollup",
    name: "orders_hourly_rollup",
    kind: "incremental",
    status: "ready",
    owner: "Data Platform",
    source: "core.sales.orders_events",
    target: "core.sales.orders_hourly",
    schedule: "Every hour",
    lastRunAt: agoIso(4),
    nextRunAt: agoIso(-56),
    slaOk: true,
    freshnessLagSeconds: 120,
  },
  {
    id: "pl-erp-inventory",
    name: "erp_inventory_sync",
    kind: "batch",
    status: "failed",
    owner: "Bayu Pratama",
    source: "ERP Inventory API",
    target: "bronze.inventory_snapshot",
    schedule: "Daily 02:00",
    lastRunAt: agoIso(48),
    slaOk: false,
    freshnessLagSeconds: 86_400,
  },
  {
    id: "pl-policy-docs",
    name: "credit_policy_ingest",
    kind: "document",
    status: "running",
    owner: "Risk Analytics",
    source: "s3://docs/credit-policy/",
    target: "ai.credit_policy_chunks",
    schedule: "On object create",
    lastRunAt: agoIso(2),
    slaOk: true,
    freshnessLagSeconds: 40,
  },
  {
    id: "pl-embed-faq",
    name: "faq_embedding_refresh",
    kind: "vector",
    status: "scheduled",
    owner: "Support AI",
    source: "knowledge.faq_articles",
    target: "ai.faq_vectors",
    schedule: "Every 6 hours",
    lastRunAt: agoIso(180),
    nextRunAt: agoIso(-180),
    slaOk: true,
    freshnessLagSeconds: 10_800,
  },
]

function runsFor(pipelineId: string): PipelineRun[] {
  return [
    {
      id: `${pipelineId}-run-1`,
      pipelineId,
      status: pipelineId === "pl-erp-inventory" ? "failed" : "completed",
      startedAt: agoIso(50),
      endedAt: agoIso(48),
      processed: 120_400,
      accepted: 119_196,
      rejected: 1_204,
      retried: 3,
      costUnits: 1.24,
      error:
        pipelineId === "pl-erp-inventory"
          ? "Schema mismatch on column qty_available"
          : undefined,
    },
    {
      id: `${pipelineId}-run-2`,
      pipelineId,
      status: "completed",
      startedAt: agoIso(1_500),
      endedAt: agoIso(1_480),
      processed: 98_200,
      accepted: 98_200,
      rejected: 0,
      retried: 0,
      costUnits: 0.88,
    },
  ]
}

export const mockPipelineService: PipelineService = {
  listPipelines(signal) {
    return mockCall(() => PIPELINES, { signal })
  },
  getPipeline(id, signal) {
    return mockCall(() => {
      const p = PIPELINES.find((x) => x.id === id)
      if (!p) throw new ServiceError("not_found", `Pipeline ${id} not found`)
      const detail: PipelineDetail = {
        ...p,
        description: `${p.name} moves data from ${p.source} to ${p.target}.`,
        graph: [
          { id: "n1", label: "Source", kind: "source", status: "completed" },
          { id: "n2", label: "Transform", kind: "transform", status: p.status },
          { id: "n3", label: "Target", kind: "target", status: p.status === "failed" ? "failed" : "completed" },
        ],
        runs: runsFor(id),
        configSummary: [
          { key: "Kind", value: p.kind },
          { key: "Schedule", value: p.schedule },
          { key: "Owner", value: p.owner },
        ],
      }
      return detail
    }, { signal })
  },
  listRuns(pipelineId, signal) {
    return mockCall(() => runsFor(pipelineId), { signal })
  },
}
