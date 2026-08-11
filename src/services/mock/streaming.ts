import { ServiceError } from "../errors"
import { mockCall } from "../transport"
import { agoIso } from "./mock-time"
import { createStore } from "./mutable-store"
import type {
  CreateStreamingJobInput,
  StreamingJob,
  StreamingService,
} from "../contracts/streaming"

const store = createStore<StreamingJob>([
  {
    id: "sj-payments-flow",
    name: "rt.payments_flow_mv",
    status: "degraded",
    owner: "Payments Platform",
    sources: ["kafka.payments.events", "cdc.accounts"],
    sinks: ["hot.payments_flow", "kafka.agent.triggers"],
    sinkAssetIds: ["mv-payments-flow"],
    lagSeconds: 42,
    throughputPerSec: 18_400,
    stateSizeBytes: 12.4 * 1024 ** 3,
    watermarkIntervalSec: 5,
    lastBarrierAt: agoIso(1),
  },
  {
    id: "sj-fraud-windows",
    name: "rt.fraud_window_agg",
    status: "running",
    owner: "Risk",
    sources: ["kafka.auth.events"],
    sinks: ["hot.fraud_signals"],
    sinkAssetIds: ["tbl-payments-enriched"],
    lagSeconds: 3,
    throughputPerSec: 6_200,
    stateSizeBytes: 2.1 * 1024 ** 3,
    watermarkIntervalSec: 2,
    lastBarrierAt: agoIso(0),
  },
])

export const mockStreamingService: StreamingService = {
  listJobs(signal) {
    return mockCall(() => store.list(), { signal })
  },
  getJob(id, signal) {
    return mockCall(() => {
      const job = store.get(id)
      if (!job) throw new ServiceError("not_found", `Streaming job ${id} not found`)
      return {
        ...job,
        definitionSql: `CREATE MATERIALIZED VIEW ${job.name} AS\nSELECT window_start, count(*) AS events\nFROM ${job.sources[0]}\nGROUP BY tumble(event_time, INTERVAL '1' MINUTE);`,
        triggers: [
          {
            id: "tr-1",
            condition: "lag_seconds > 30",
            target: "Streaming lag triage workflow",
            targetHref: "/agents/workflows?id=wf-lag-triage",
          },
          {
            id: "tr-2",
            condition: "delinquency_score > 0.8",
            target: "collections-copilot",
            targetHref: "/agents/employees/emp-collections",
          },
        ],
        checkpoints: [
          { id: "cp-1", at: agoIso(5), sizeBytes: job.stateSizeBytes },
          { id: "cp-2", at: agoIso(20), sizeBytes: job.stateSizeBytes * 0.98 },
        ],
      }
    }, { signal })
  },
  createStreamingJob(input: CreateStreamingJobInput, signal) {
    return mockCall(
      () => {
        const job: StreamingJob = {
          id: `sj-${Date.now().toString(36)}`,
          name: input.name,
          status: "draft",
          owner: input.owner ?? "Current user",
          sources: input.sources,
          sinks: input.sinks,
          lagSeconds: 0,
          throughputPerSec: 0,
          stateSizeBytes: 0,
          watermarkIntervalSec: input.watermarkIntervalSec,
          lastBarrierAt: agoIso(0),
        }
        return store.prepend(job)
      },
      { signal, delayMs: 500 }
    )
  },
  pauseJob(id, signal) {
    return mockCall(() => {
      const updated = store.update(id, { status: "paused" })
      if (!updated) throw new ServiceError("not_found", `Streaming job ${id} not found`)
      return updated
    }, { signal })
  },
  resumeJob(id, signal) {
    return mockCall(() => {
      const updated = store.update(id, { status: "running" })
      if (!updated) throw new ServiceError("not_found", `Streaming job ${id} not found`)
      return updated
    }, { signal })
  },
}
