import { ServiceError } from "../errors"
import { mockCall } from "../transport"
import { agoIso } from "./mock-time"
import { createStore } from "./mutable-store"
import type {
  CreatePipelineInput,
  GeneratePipelineInput,
  Pipeline,
  PipelineDetail,
  PipelineRun,
  PipelineService,
} from "../contracts/pipelines"

const store = createStore<Pipeline>([
  {
    id: "pl-orders-rollup",
    name: "orders_hourly_rollup",
    kind: "incremental",
    status: "ready",
    owner: "Data Platform",
    source: "core.sales.orders_events",
    target: "core.sales.orders_hourly",
    connectorId: "conn-pg-core",
    sourceAssetId: "tbl-orders-events",
    targetAssetId: "tbl-payments-enriched",
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
    sourceAssetId: "ext-legacy-warehouse",
    targetAssetId: "tbl-inventory-snapshot",
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
    connectorId: "conn-s3-docs",
    targetAssetId: "kn-credit-policy",
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
    sourceAssetId: "kn-credit-policy",
    targetAssetId: "vec-support-kb",
    schedule: "Every 6 hours",
    lastRunAt: agoIso(180),
    nextRunAt: agoIso(-180),
    slaOk: true,
    freshnessLagSeconds: 10_800,
  },
])

/** In-session run overrides so cancel/retry persist until reload. */
const runOverrides = new Map<string, PipelineRun>()

function slugId(prefix: string, name: string) {
  const slug = name
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-|-$/g, "")
    .slice(0, 32)
  return `${prefix}-${slug || "new"}-${Date.now().toString(36)}`
}

function seedRuns(pipelineId: string): PipelineRun[] {
  const pipeline = store.get(pipelineId)
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
      checkpoint: `cp-${pipelineId}-48`,
      auditEventId: `aud-run-${pipelineId}-1`,
      outputAssetId: pipeline?.targetAssetId,
      error:
        pipelineId === "pl-erp-inventory"
          ? "Schema mismatch on column qty_available"
          : undefined,
    },
    {
      id: `${pipelineId}-run-2`,
      pipelineId,
      status: pipelineId === "pl-orders-rollup" ? "partial" : "completed",
      startedAt: agoIso(1_500),
      endedAt: agoIso(1_480),
      processed: 98_200,
      accepted: pipelineId === "pl-orders-rollup" ? 97_100 : 98_200,
      rejected: pipelineId === "pl-orders-rollup" ? 1_100 : 0,
      retried: 0,
      costUnits: 0.88,
      checkpoint: `cp-${pipelineId}-1480`,
      auditEventId: `aud-run-${pipelineId}-2`,
      outputAssetId: pipeline?.targetAssetId,
    },
  ]
}

function runsFor(pipelineId: string): PipelineRun[] {
  return seedRuns(pipelineId).map((r) => runOverrides.get(r.id) ?? r)
}

function findRun(runId: string): PipelineRun | undefined {
  const overridden = runOverrides.get(runId)
  if (overridden) return overridden
  const pipelineId = runId.replace(/-run-\d+$/, "").replace(/-run-[a-z0-9]+$/, "")
  // Prefer exact match against all known pipelines.
  for (const p of store.list()) {
    const found = runsFor(p.id).find((r) => r.id === runId)
    if (found) return found
  }
  // Fallback for dynamically created run ids: `${id}-run-${timestamp}`
  const match = store.list().find((p) => runId.startsWith(`${p.id}-run-`))
  if (match) {
    return (
      runOverrides.get(runId) ?? {
        id: runId,
        pipelineId: match.id,
        status: "running",
        startedAt: agoIso(0),
        processed: 0,
        accepted: 0,
        rejected: 0,
        retried: 0,
        costUnits: 0,
        outputAssetId: match.targetAssetId,
      }
    )
  }
  void pipelineId
  return undefined
}

function toDetail(p: Pipeline): PipelineDetail {
  return {
    ...p,
    description: `${p.name} moves data from ${p.source} to ${p.target}.`,
    graph: [
      { id: "n1", label: "Source", kind: "source", status: "completed" },
      { id: "n2", label: "Transform", kind: "transform", status: p.status },
      {
        id: "n3",
        label: "Target",
        kind: "target",
        status: p.status === "failed" ? "failed" : "completed",
      },
    ],
    runs: runsFor(p.id),
    configSummary: [
      { key: "Kind", value: p.kind },
      { key: "Schedule", value: p.schedule },
      { key: "Owner", value: p.owner },
      ...(p.connectorId
        ? [{ key: "Connector", value: p.connectorId }]
        : []),
    ],
  }
}

function fromCreateInput(input: CreatePipelineInput): Pipeline {
  return {
    id: slugId("pl", input.name),
    name: input.name,
    kind: input.kind,
    status: "draft",
    owner: input.owner ?? "Current user",
    source: `${input.sourceZone}.${input.sourceTable}`,
    target: `${input.targetZone}.${input.targetTable}`,
    schedule: input.schedule,
    lastRunAt: agoIso(0),
    nextRunAt: agoIso(-60),
    slaOk: true,
    freshnessLagSeconds: 0,
  }
}

export const mockPipelineService: PipelineService = {
  listPipelines(signal) {
    return mockCall(() => store.list(), { signal })
  },
  getPipeline(id, signal) {
    return mockCall(() => {
      const p = store.get(id)
      if (!p) throw new ServiceError("not_found", `Pipeline ${id} not found`)
      return toDetail(p)
    }, { signal })
  },
  listRuns(pipelineId, signal) {
    return mockCall(() => runsFor(pipelineId), { signal })
  },
  createPipeline(input, signal) {
    return mockCall(() => store.prepend(fromCreateInput(input)), {
      signal,
      delayMs: 500,
    })
  },
  generatePipelineFromPrompt(input: GeneratePipelineInput, signal) {
    return mockCall(
      () => {
        const name =
          input.instruction
            .split(/\s+/)
            .slice(0, 4)
            .join("_")
            .replace(/[^a-zA-Z0-9_]/g, "")
            .toLowerCase() || "agentic_pipeline"
        const pipeline: Pipeline = {
          id: slugId("pl", name),
          name,
          kind: "incremental",
          status: "draft",
          owner: "Agentic Builder",
          source: `${input.database}.source_table`,
          target: `${input.database}.target_table`,
          schedule: "On demand",
          lastRunAt: agoIso(0),
          slaOk: true,
          freshnessLagSeconds: 0,
        }
        return store.prepend(pipeline)
      },
      { signal, delayMs: 1600 }
    )
  },
  triggerRun(id, signal) {
    return mockCall(() => {
      const p = store.get(id)
      if (!p) throw new ServiceError("not_found", `Pipeline ${id} not found`)
      store.update(id, { status: "running", lastRunAt: agoIso(0) })
      const run: PipelineRun = {
        id: `${id}-run-${Date.now().toString(36)}`,
        pipelineId: id,
        status: "running",
        startedAt: agoIso(0),
        processed: 0,
        accepted: 0,
        rejected: 0,
        retried: 0,
        costUnits: 0,
        checkpoint: `cp-${id}-live`,
        outputAssetId: p.targetAssetId,
      }
      runOverrides.set(run.id, run)
      return run
    }, { signal, delayMs: 400 })
  },
  cancelRun(runId, signal) {
    return mockCall(() => {
      const existing = findRun(runId)
      if (!existing) throw new ServiceError("not_found", `Run ${runId} not found`)
      if (existing.status !== "running") {
        throw new ServiceError(
          "invalid_request",
          `Run ${runId} is ${existing.status} and cannot be cancelled`
        )
      }
      const cancelled: PipelineRun = {
        ...existing,
        status: "cancelled",
        endedAt: agoIso(0),
        auditEventId: existing.auditEventId ?? `aud-cancel-${runId}`,
      }
      runOverrides.set(runId, cancelled)
      return cancelled
    }, { signal, delayMs: 350 })
  },
  retryRun(runId, signal) {
    return mockCall(() => {
      const existing = findRun(runId)
      if (!existing) throw new ServiceError("not_found", `Run ${runId} not found`)
      if (existing.status !== "failed" && existing.status !== "cancelled") {
        throw new ServiceError(
          "invalid_request",
          `Run ${runId} is ${existing.status} and cannot be retried`
        )
      }
      const retry: PipelineRun = {
        ...existing,
        id: `${existing.pipelineId}-run-${Date.now().toString(36)}`,
        status: "running",
        startedAt: agoIso(0),
        endedAt: undefined,
        retried: existing.retried + 1,
        error: undefined,
        checkpoint: `cp-${existing.pipelineId}-retry`,
        auditEventId: `aud-retry-${existing.pipelineId}`,
      }
      runOverrides.set(retry.id, retry)
      store.update(existing.pipelineId, {
        status: "running",
        lastRunAt: agoIso(0),
      })
      return retry
    }, { signal, delayMs: 400 })
  },
  pausePipeline(id, signal) {
    return mockCall(() => {
      const updated = store.update(id, { status: "paused" })
      if (!updated) throw new ServiceError("not_found", `Pipeline ${id} not found`)
      return updated
    }, { signal })
  },
  resumePipeline(id, signal) {
    return mockCall(() => {
      const updated = store.update(id, { status: "ready" })
      if (!updated) throw new ServiceError("not_found", `Pipeline ${id} not found`)
      return updated
    }, { signal })
  },
}
